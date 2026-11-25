//! Images a mark meets: removed where the marks cover them, else copied with each sample a mark
//! meets transparent, stored losslessly (ISO 32000-2:2020, 8.9).

use alloc::sync::Arc;

use crate::PdfError;
use crate::objects::ObjectId;
use crate::overlay::{Data, Object};
use crate::render::data::{self, Resolved};
use crate::render::fixed::Fixed;
use crate::render::geometry::{Matrix, Point};
use crate::render::image::{self as images, Unpacked};
use crate::render::interpret::{Run, content, inline};
use crate::render::job::Job;
use crate::render::output::deflate::Deflate;
use crate::render::redact::marks::{self, Mark};
use crate::render::redact::stage::Stage;

/// Work of a sample tested against the marks.
const CELL_WORK: u64 = 8;

/// Keys a copy leaves out: its data is new, and no alternate keeps the samples (7.3.8.2, 8.9.5.4).
const LEFT_OUT: [&[u8]; 9] = [
    b"Alternates",
    b"DL",
    b"DecodeParms",
    b"F",
    b"FDecodeParms",
    b"FFilter",
    b"Filter",
    b"Length",
    b"SMaskInData",
];

/// The message for an image that cannot be drawn.
const UNUSABLE: &str = "an image cannot be drawn";

/// A key of a dictionary and its value; none removes it.
type Entry = (&'static [u8], Option<Object>);

/// Where samples lie: bits per sample, samples per pixel and bytes per row.
type Packing = (u32, usize, usize);

/// Samples coded, those of a plane beside them, and how many a mark meets.
type Streamed = (Vec<u8>, Option<Vec<u8>>, u64);

/// The columns, then the rows, a box of samples spans, past the last excluded, and the area of a
/// mark it holds.
type Span<'marks> = ([u32; 4], &'marks [Point]);

/// Samples Flate-coded as they come.
struct Coder {
    /// The coder.
    deflate: Deflate,
    /// What it wrote.
    out: Vec<u8>,
}

/// How the samples a mark meets become transparent.
enum Veil {
    /// A stencil mask of the image's size is added (8.9.6.3).
    Added,
    /// Its explicit mask is copied (8.9.6.3).
    Explicit(Resolved),
    /// They take the low ends of its colour key (8.9.6.4).
    Key(Vec<u16>),
    /// The channel of its JPX data that is opacity becomes a soft mask (7.4.9).
    Opacity,
    /// Its soft mask is copied (11.6.5.3).
    Soft(Resolved),
    /// It is an image mask: they take this sample, which leaves the page as it is (8.9.6.2).
    Stencil(u16),
}

impl Coder {
    /// What it wrote, ended.
    fn finish(mut self) -> Vec<u8> {
        self.deflate.finish(&mut self.out);
        self.out
    }

    /// One that wrote nothing.
    fn new() -> Self {
        let mut out = Vec::new();
        let deflate = Deflate::new(&mut out);
        Self { deflate, out }
    }

    /// Codes `bytes`.
    fn write(&mut self, bytes: &[u8]) {
        self.deflate.write(bytes, &mut self.out);
    }
}

/// The samples of row `row` of an image of `size` under `matrix`, before column `present`, whose
/// area meets a span of `spans`, given to `each`; how many.
fn covered(
    job: &Job<'_>,
    (spans, matrix): (&[Span<'_>], &Matrix),
    (size, row, present): ([u32; 2], u32, u32),
    each: &mut dyn FnMut(u32),
) -> Result<u64, PdfError> {
    let near = at(spans, row);
    let mut count = 0_u64;
    for (from, to) in across(&near) {
        for column in from..to.min(present) {
            job.spend(CELL_WORK)?;
            if meets(&near, column, &cell(matrix, size, column, row)) {
                each(column);
                count = count.saturating_add(1);
            }
        }
    }
    Ok(count)
}

/// The columns of `spans`, joined where they overlap.
fn across(spans: &[&Span<'_>]) -> Vec<(u32, u32)> {
    let mut found: Vec<(u32, u32)> = spans.iter().map(|span| (span.0[0], span.0[1])).collect();
    found.sort_unstable();
    let mut joined: Vec<(u32, u32)> = Vec::new();
    for (from, to) in found {
        match joined.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            Some(_) | None => joined.push((from, to)),
        }
    }
    joined
}

/// The spans of `spans` at `row`.
fn at<'spans, 'marks>(spans: &'spans [Span<'marks>], row: u32) -> Vec<&'spans Span<'marks>> {
    spans
        .iter()
        .filter(|span| span.0[2] <= row && row < span.0[3])
        .collect()
}

/// The area of the sample at `column`, `row` of an image of `size` under `matrix`; row 0 is at
/// the top (8.9.4).
fn cell(matrix: &Matrix, [columns, rows]: [u32; 2], column: u32, row: u32) -> [Point; 4] {
    let part = |count: u32, of: u32| {
        Fixed::from_i64(i64::from(count))
            .over(Fixed::from_i64(i64::from(of)))
            .unwrap_or_default()
    };
    let one = Fixed::from_i64(1);
    let (left, right) = (
        part(column, columns),
        part(column.saturating_add(1), columns),
    );
    let (top, bottom) = (
        one.minus(part(row, rows)),
        one.minus(part(row.saturating_add(1), rows)),
    );
    [(left, bottom), (right, bottom), (right, top), (left, top)]
        .map(|(across, down)| matrix.apply(Point { across, down }))
}

/// A copy of the image `owner` holding `coded`, Flate-coded samples, with `set` set.
fn copy(
    job: &Job<'_>,
    stage: &mut Stage,
    owner: &Object,
    coded: Vec<u8>,
    set: Vec<Entry>,
) -> Result<ObjectId, PdfError> {
    let entries = data::pairs(owner).map_or_else(Vec::new, |listed| {
        listed
            .iter()
            .filter(|pair| !LEFT_OUT.contains(&pair.0.as_slice()))
            .cloned()
            .collect()
    });
    let mut made = Object::Stream(entries, Data::Bytes(Arc::from(coded)));
    made.set(b"Filter", Some(name(b"FlateDecode")));
    for (key, value) in set {
        made.set(key, value);
    }
    stage.add(job, made)
}

/// Whether `owner` is an image mask (8.9.6.2).
fn flagged(job: &Job<'_>, owner: &Object) -> Result<bool, PdfError> {
    data::entry(job, owner, b"ImageMask")
        .map(|found| matches!(*found.value(), Object::Boolean(true)))
}

/// An inline image a mark meets, read past: none where it is removed, else its copy as an image
/// `XObject` (8.9.7).
pub(super) fn inline(
    run: &mut Run<'_, '_>,
    (marks, stage): (&[Mark], &mut Stage),
    operations: &mut content::Operations<'_>,
) -> Result<Option<ObjectId>, PdfError> {
    let (dictionary, stored) = inline::taken(run, operations)?;
    let entries = data::pairs(&dictionary).cloned().unwrap_or_default();
    let mut image = Object::Stream(entries, Data::Bytes(Arc::from(stored)));
    image.set(b"Type", Some(name(b"XObject")));
    image.set(b"Subtype", Some(name(b"Image")));
    let found = data::made(run.job, image, None)?;
    painted(run, (marks, stage), &found)
}

/// The low end of each colour key range of `mask`, when it is one of `components` (8.9.6.4).
fn keyed(mask: &Object, components: usize) -> Vec<u16> {
    let numbers: Vec<Fixed> = data::items(mask)
        .unwrap_or_default()
        .iter()
        .filter_map(Fixed::from_object)
        .collect();
    if numbers.len() != components.saturating_mul(2) {
        return Vec::new();
    }
    numbers
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[low, _]| u16::try_from(low.floor().max(0)).unwrap_or(u16::MAX))
        .collect()
}

/// The mask image `mask`, soft when `soft`, with each sample a mark meets transparent.
fn masked(
    run: &mut Run<'_, '_>,
    (marks, stage, matrix): (&[Mark], &mut Stage, &Matrix),
    mask: &Resolved,
    soft: bool,
) -> Result<ObjectId, PdfError> {
    let job = run.job;
    let mut found = images::unpacked(run, mask, soft)?;
    let value = if soft {
        let decode = numbers(job, mask.value(), b"Decode")?;
        match *decode.as_slice() {
            [low, high] if low > high => most(found.bits),
            _ => 0,
        }
    } else {
        unpainted(job, mask.value())?
    };
    let blank = vec![value; found.components];
    let room = stage.room(job);
    let (coded, _plane, _count) = streamed(job, (marks, matrix), &mut found, (&blank, None), room)?;
    let set = sizes(&found);
    copy(job, stage, mask.value(), coded, set)
}

/// Whether the area of a span of `spans` holding `column` meets `shape`.
fn meets(spans: &[&Span<'_>], column: u32, shape: &[Point]) -> bool {
    spans
        .iter()
        .any(|&&(span, area)| span[0] <= column && column < span[1] && marks::overlap(area, shape))
}

/// The largest sample of `bits`.
const fn most(bits: u32) -> u16 {
    u16::MAX.wrapping_shr(16_u32.saturating_sub(bits))
}

/// The name `bytes`.
fn name(bytes: &[u8]) -> Object {
    Object::Name(bytes.to_vec())
}

/// The numbers of the array `key` of `owner`; empty when none.
fn numbers(job: &Job<'_>, owner: &Object, key: &[u8]) -> Result<Vec<Fixed>, PdfError> {
    let found = data::entry(job, owner, key)?;
    Ok(data::items(found.value())
        .unwrap_or_default()
        .iter()
        .filter_map(Fixed::from_object)
        .collect())
}

/// The image `XObject` `found` a mark meets: none where the marks cover it or it paints nothing,
/// else its copy, each sample a mark meets transparent through a mask (8.9.5, 8.9.6, 11.6.5.3).
pub(super) fn painted(
    run: &mut Run<'_, '_>,
    (marks, stage): (&[Mark], &mut Stage),
    found: &Resolved,
) -> Result<Option<ObjectId>, PdfError> {
    let job = run.job;
    let owner = found.value();
    let stencil = flagged(job, owner)?;
    let matrix = run.state.matrix;
    // It paints nothing where colour may not be given or it has no area (8.7.3.3).
    if (run.locked_color && !stencil) || matrix.inverse().is_none() || within(marks, &matrix) {
        return Ok(None);
    }
    // A JPEG's frame gives the size it lacks.
    let declared = [side(job, owner, b"Width")?, side(job, owner, b"Height")?];
    if !declared.contains(&0) && whole(job, (marks, &matrix), declared)? {
        return Ok(None);
    }
    let Some(mut base) = job.tolerate(images::unpacked(run, found, false), "8.9.5", UNUSABLE)?
    else {
        return Ok(None);
    };
    let colours = base
        .components
        .saturating_sub(usize::from(base.alpha.is_some()));
    let veil = veil(
        job,
        owner,
        (stencil, colours),
        base.alpha.is_some() && base.opacity,
    )?;
    let blank = match veil {
        Veil::Stencil(value) => vec![value],
        Veil::Key(ref lows) => lows.clone(),
        Veil::Added | Veil::Explicit(_) | Veil::Opacity | Veil::Soft(_) => vec![0; colours],
    };
    let (bits, columns) = (base.bits, base.columns);
    let plane = match veil {
        Veil::Added => Some(((1, 1, stride(columns, 1, 1)), 1)),
        Veil::Opacity => Some(((bits, 1, stride(columns, 1, bits)), 0)),
        Veil::Explicit(_) | Veil::Key(_) | Veil::Soft(_) | Veil::Stencil(_) => None,
    };
    let room = stage.room(job);
    let read = streamed(job, (marks, &matrix), &mut base, (&blank, plane), room);
    let Some((coded, beside, count)) = job.tolerate(read, "8.9.5", UNUSABLE)? else {
        return Ok(None);
    };
    if count >= u64::from(base.columns).saturating_mul(u64::from(base.height)) {
        return Ok(None);
    }
    let mut set = sizes(&base);
    set.extend(veiled(run, (marks, stage, &matrix), &base, (veil, beside))?);
    set.extend(spelled(job, stage, &mut base, colours)?);
    if base.jpx && !stencil {
        set.push((b"Decode", None));
    }
    copy(job, stage, owner, coded, set).map(Some)
}

/// An image dictionary with `entries`.
fn plain(entries: Vec<Entry>) -> Object {
    let mut made = Object::Dictionary(vec![
        (b"Type".to_vec(), name(b"XObject")),
        (b"Subtype".to_vec(), name(b"Image")),
    ]);
    for (key, value) in entries {
        made.set(key, value);
    }
    made
}

/// Sets sample `component` at `column` of `row` of `samples`, packed as `packing`, to `value`.
fn put(
    samples: &mut [u8],
    (bits, components, stride): Packing,
    place: (u32, u32, usize),
    value: u16,
) {
    let (column, row, component) = place;
    let index = u64::from(column)
        .saturating_mul(u64::try_from(components).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(component).unwrap_or(u64::MAX));
    let bit = index.saturating_mul(u64::from(bits));
    let start = usize::try_from(row)
        .unwrap_or(usize::MAX)
        .saturating_mul(stride)
        .saturating_add(usize::try_from(bit >> 3_u32).unwrap_or(usize::MAX));
    let (high, low) = (
        u8::try_from(value.wrapping_shr(8)).unwrap_or(0),
        u8::try_from(value & 0xFF).unwrap_or(0),
    );
    if bits == 16 {
        for (at, byte) in [(start, high), (start.saturating_add(1), low)] {
            if let Some(slot) = samples.get_mut(at) {
                *slot = byte;
            }
        }
        return;
    }
    let Some(slot) = samples.get_mut(start) else {
        return;
    };
    let shift = 8_u32
        .saturating_sub(bits)
        .saturating_sub(u32::try_from(bit & 7).unwrap_or(0));
    let held = u8::try_from(most(bits))
        .unwrap_or(u8::MAX)
        .wrapping_shl(shift);
    *slot = (*slot & !held) | (low.wrapping_shl(shift) & held);
}

/// The side `key` of `owner`; 0 when it has none.
fn side(job: &Job<'_>, owner: &Object, key: &[u8]) -> Result<u32, PdfError> {
    let found = data::entry(job, owner, key)?;
    Ok(Fixed::from_object(found.value())
        .and_then(|value| u32::try_from(value.floor()).ok())
        .unwrap_or(0))
}

/// The entries that give `found`'s size and depth.
fn sizes(found: &Unpacked<'_>) -> Vec<Entry> {
    vec![
        (b"Width", Some(Object::Integer(i64::from(found.columns)))),
        (b"Height", Some(Object::Integer(i64::from(found.height)))),
        (
            b"BitsPerComponent",
            Some(Object::Integer(i64::from(found.bits))),
        ),
    ]
}

/// The colour space `found`'s copy, of `colours` samples a pixel, names in place of its own: the
/// ICC profile or device space of JPX data, or the device space of samples whose dictionary names
/// none (8.9.5, 7.4.9).
fn spelled(
    job: &Job<'_>,
    stage: &mut Stage,
    found: &mut Unpacked<'_>,
    colours: usize,
) -> Result<Option<Entry>, PdfError> {
    if let Some(profile) = found.profile.take() {
        let components = i64::try_from(colours).unwrap_or(i64::MAX);
        let icc = Object::Stream(
            vec![(b"N".to_vec(), Object::Integer(components))],
            Data::Bytes(Arc::from(profile)),
        );
        let made = stage.add(job, icc)?;
        let space = Object::Array(vec![name(b"ICCBased"), Object::Reference(made)]);
        return Ok(Some((b"ColorSpace", Some(space))));
    }
    if !found.device {
        return Ok(None);
    }
    let space: &[u8] = match colours {
        1 => b"DeviceGray",
        4 => b"DeviceCMYK",
        _ => b"DeviceRGB",
    };
    Ok(Some((b"ColorSpace", Some(name(space)))))
}

/// The columns and rows of samples each mark's area may meet in an image of `size` under
/// `matrix`, a sample wider each way.
fn spanned<'marks>(
    marks: &'marks [Mark],
    matrix: &Matrix,
    [columns, rows]: [u32; 2],
) -> Vec<Span<'marks>> {
    let Some(inverse) = matrix.inverse() else {
        return Vec::new();
    };
    let (wide, high) = (
        Fixed::from_i64(i64::from(columns)),
        Fixed::from_i64(i64::from(rows)),
    );
    let one = Fixed::from_i64(1);
    let clamp = |value: i64, most: u32| u32::try_from(value.max(0)).unwrap_or(u32::MAX).min(most);
    let mut found = Vec::new();
    for area in marks.iter().flat_map(|mark| mark.areas.iter()) {
        let mut bounds: Option<[i64; 4]> = None;
        for point in area {
            let unit = inverse.apply(*point);
            let across = unit.across.times(wide).floor();
            let down = one.minus(unit.down).times(high).floor();
            bounds = Some(bounds.map_or(
                [across, across, down, down],
                |[left, right, top, bottom]| {
                    [
                        left.min(across),
                        right.max(across),
                        top.min(down),
                        bottom.max(down),
                    ]
                },
            ));
        }
        if let Some([left, right, top, bottom]) = bounds {
            found.push((
                [
                    clamp(left.saturating_sub(1), columns),
                    clamp(right.saturating_add(2), columns),
                    clamp(top.saturating_sub(1), rows),
                    clamp(bottom.saturating_add(2), rows),
                ],
                area.as_slice(),
            ));
        }
    }
    found
}

/// The samples of `base` row by row, each sample a mark meets under `matrix` `blank`, without
/// its channel that is no colour, and beside them a plane where one is given: its packing, and
/// the sample those a mark meets take; its rows are that channel where it is opacity, else 0.
/// Both Flate-coded within `room` bytes, the samples ending where the data does, and how many
/// samples a mark meets or the data lacks (7.4.9, 8.9.5).
fn streamed(
    job: &Job<'_>,
    (marks, matrix): (&[Mark], &Matrix),
    base: &mut Unpacked<'_>,
    (blank, plane): (&[u16], Option<(Packing, u16)>),
    room: u64,
) -> Result<Streamed, PdfError> {
    let size = [base.columns, base.height];
    let spans = spanned(marks, matrix, size);
    let (bits, count, channel) = (base.bits, base.components, base.alpha);
    let colours = count.saturating_sub(usize::from(channel.is_some()));
    let packing = (bits, colours, stride(base.columns, colours, bits));
    let (mut main, mut beside) = (Coder::new(), plane.map(|_given| Coder::new()));
    let (mut raw, mut kept, mut extra) = (Vec::new(), Vec::new(), Vec::new());
    let (mut total, mut ended) = (0_u64, false);
    for row in 0..base.height {
        let present = base.rows.row(job, row, &mut raw)?;
        job.spend(u64::try_from(raw.len()).unwrap_or(u64::MAX))?;
        let samples = if let Some(skipped) = channel {
            kept.clear();
            kept.resize(packing.2, 0);
            let wide = stride(base.columns, 1, bits);
            extra.clear();
            extra.resize(wide, 0);
            for column in 0..base.columns {
                for component in 0..count {
                    let value = sample(&raw, (bits, count, base.stride), (column, 0, component));
                    if component == skipped {
                        put(&mut extra, (bits, 1, wide), (column, 0, 0), value);
                    } else {
                        let at = component.saturating_sub(usize::from(component > skipped));
                        put(&mut kept, packing, (column, 0, at), value);
                    }
                }
            }
            &mut kept
        } else {
            &mut raw
        };
        let mut veil = plane.map(|(veiled, value)| {
            let mut found = if base.opacity {
                extra.clone()
            } else {
                Vec::new()
            };
            found.resize(veiled.2, 0);
            (found, veiled, value)
        });
        total = total.saturating_add(covered(
            job,
            (&spans, matrix),
            (size, row, present),
            &mut |column| {
                for (component, value) in blank.iter().enumerate() {
                    put(samples, packing, (column, 0, component), *value);
                }
                if let Some(&mut (ref mut found, veiled, value)) = veil.as_mut() {
                    put(found, veiled, (column, 0, 0), value);
                }
            },
        )?);
        // What the data lacks stays unpainted: the copy's data lacks it too.
        let lacking = base.columns.saturating_sub(present);
        total = total.saturating_add(u64::from(lacking));
        if !ended {
            let length = if lacking > 0 {
                stride(present, colours, bits)
            } else {
                packing.2
            };
            main.write(samples.get(..length).unwrap_or_default());
            ended = lacking > 0;
        }
        let mut held = main.out.len();
        if let (Some(coder), Some(found)) = (beside.as_mut(), veil) {
            coder.write(&found.0);
            held = held.saturating_add(coder.out.len());
        }
        if u64::try_from(held).unwrap_or(u64::MAX) > room {
            return Err(job.limit());
        }
    }
    Ok((main.finish(), beside.map(Coder::finish), total))
}

/// The bytes of a row of `columns` pixels of `components` samples of `bits`.
fn stride(columns: u32, components: usize, bits: u32) -> usize {
    let total = u64::from(columns)
        .saturating_mul(u64::try_from(components).unwrap_or(u64::MAX))
        .saturating_mul(u64::from(bits));
    usize::try_from(total.div_ceil(8)).unwrap_or(usize::MAX)
}

/// Sample `component` at `column` of `row` of `samples`, packed as `packing`.
fn sample(samples: &[u8], (bits, components, stride): Packing, place: (u32, u32, usize)) -> u16 {
    let (column, row, component) = place;
    let index = u64::from(column)
        .saturating_mul(u64::try_from(components).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(component).unwrap_or(u64::MAX));
    let bit = index.saturating_mul(u64::from(bits));
    let start = usize::try_from(row)
        .unwrap_or(usize::MAX)
        .saturating_mul(stride)
        .saturating_add(usize::try_from(bit >> 3_u32).unwrap_or(usize::MAX));
    let byte = |at: usize| samples.get(at).copied().unwrap_or(0);
    if bits == 16 {
        return u16::from(byte(start)).wrapping_shl(8) | u16::from(byte(start.saturating_add(1)));
    }
    let shift = 8_u32
        .saturating_sub(bits)
        .saturating_sub(u32::try_from(bit & 7).unwrap_or(0));
    u16::from(byte(start).wrapping_shr(shift)) & most(bits)
}

/// How the samples of the image `owner`, of `colours` samples a pixel, that a mark meets become
/// transparent; `opacity`: a channel of its JPX data is opacity (7.4.9, 11.6.4.3).
fn veil(
    job: &Job<'_>,
    owner: &Object,
    (stencil, colours): (bool, usize),
    opacity: bool,
) -> Result<Veil, PdfError> {
    if stencil {
        return Ok(Veil::Stencil(unpainted(job, owner)?));
    }
    let soft = data::entry(job, owner, b"SMask")?;
    if matches!(*soft.value(), Object::Stream(..)) {
        return Ok(Veil::Soft(soft));
    }
    if opacity {
        return Ok(Veil::Opacity);
    }
    let hard = data::entry(job, owner, b"Mask")?;
    if matches!(*hard.value(), Object::Stream(..)) {
        return Ok(Veil::Explicit(hard));
    }
    let lows = keyed(hard.value(), colours);
    if !lows.is_empty() {
        return Ok(Veil::Key(lows));
    }
    Ok(Veil::Added)
}

/// The entry that gives `base`'s copy the mask `veil` makes, where one does, of `plane`, the
/// samples coded for it where it adds them.
fn veiled(
    run: &mut Run<'_, '_>,
    (marks, stage, matrix): (&[Mark], &mut Stage, &Matrix),
    base: &Unpacked<'_>,
    (veil, plane): (Veil, Option<Vec<u8>>),
) -> Result<Option<Entry>, PdfError> {
    let job = run.job;
    let (key, made): (&[u8], _) = match veil {
        Veil::Key(_) | Veil::Stencil(_) => return Ok(None),
        Veil::Soft(found) => (b"SMask", masked(run, (marks, stage, matrix), &found, true)?),
        Veil::Explicit(found) => (b"Mask", masked(run, (marks, stage, matrix), &found, false)?),
        Veil::Opacity => {
            let mut entries = sizes(base);
            entries.push((b"ColorSpace", Some(name(b"DeviceGray"))));
            if !base.matte.is_empty() {
                let values = base.matte.iter().map(|low| Object::Integer(*low)).collect();
                entries.push((b"Matte", Some(Object::Array(values))));
            }
            (
                b"SMask",
                copy(
                    job,
                    stage,
                    &plain(entries),
                    plane.unwrap_or_default(),
                    Vec::new(),
                )?,
            )
        }
        Veil::Added => {
            let entries = vec![
                (
                    &b"Width"[..],
                    Some(Object::Integer(i64::from(base.columns))),
                ),
                (b"Height", Some(Object::Integer(i64::from(base.height)))),
                (b"ImageMask", Some(Object::Boolean(true))),
                (b"BitsPerComponent", Some(Object::Integer(1))),
            ];
            (
                b"Mask",
                copy(
                    job,
                    stage,
                    &plain(entries),
                    plane.unwrap_or_default(),
                    Vec::new(),
                )?,
            )
        }
    };
    Ok(Some((key, Some(Object::Reference(made)))))
}

/// The sample of an image mask `owner` that leaves the page as it is: 1, or 0 where its
/// `Decode` is [1 0] (8.9.6.2).
fn unpainted(job: &Job<'_>, owner: &Object) -> Result<u16, PdfError> {
    let decode = numbers(job, owner, b"Decode")?;
    Ok(u16::from(
        decode.first().is_none_or(|low| *low < Fixed::from_i64(1)),
    ))
}

/// Whether every sample of an image of `size` under `matrix` meets a mark.
fn whole(
    job: &Job<'_>,
    (marks, matrix): (&[Mark], &Matrix),
    size: [u32; 2],
) -> Result<bool, PdfError> {
    let spans = spanned(marks, matrix, size);
    let [columns, rows] = size;
    for row in 0..rows {
        let near = at(&spans, row);
        if across(&near) != [(0, columns)] {
            return Ok(false);
        }
        for column in 0..columns {
            job.spend(CELL_WORK)?;
            if !meets(&near, column, &cell(matrix, size, column, row)) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Whether the image under `matrix` lies within one area of a mark: each corner on the inner side
/// of each of its edges, counterclockwise.
fn within(marks: &[Mark], matrix: &Matrix) -> bool {
    let (none, one) = (Fixed::default(), Fixed::from_i64(1));
    let corners = [(none, none), (one, none), (one, one), (none, one)]
        .map(|(across, down)| matrix.apply(Point { across, down }));
    marks.iter().flat_map(|mark| mark.areas.iter()).any(|area| {
        area.len() >= 3
            && area.iter().enumerate().all(|(at, start)| {
                let end = area.get(at.saturating_add(1)).or_else(|| area.first());
                end.is_some_and(|stop| {
                    corners
                        .iter()
                        .all(|corner| marks::cross(*start, *stop, *corner) >= 0)
                })
            })
    })
}
