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

/// The columns, then the rows, a box of samples spans, past the last excluded, and the area of a
/// mark it holds.
type Span<'marks> = ([u32; 4], &'marks [Point]);

/// How the samples a mark meets become transparent.
enum Veil {
    /// A stencil mask of the image's size is added, its samples here (8.9.6.3).
    Added(Vec<u8>),
    /// Its explicit mask is copied (8.9.6.3).
    Explicit(Resolved),
    /// They take the low ends of its colour key (8.9.6.4).
    Key(Vec<u16>),
    /// The channel of its JPX data that is opacity becomes a soft mask, its samples here (7.4.9).
    Opacity(Vec<u8>),
    /// Its soft mask is copied (11.6.5.3).
    Soft(Resolved),
    /// It is an image mask: they take this sample, which leaves the page as it is (8.9.6.2).
    Stencil(u16),
}

/// The samples of the image `found` at a column and row within `size` whose area meets a mark,
/// given to `each`; how many.
fn covered(
    job: &Job<'_>,
    (marks, matrix): (&[Mark], &Matrix),
    size: [u32; 2],
    each: &mut dyn FnMut(u32, u32),
) -> Result<u64, PdfError> {
    let spans = spanned(marks, matrix, size);
    let first = spans.iter().map(|span| span.0[2]).min().unwrap_or(0);
    let last = spans.iter().map(|span| span.0[3]).max().unwrap_or(0);
    let mut count = 0_u64;
    for row in first..last {
        let near = at(&spans, row);
        for (from, to) in across(&near) {
            for column in from..to {
                job.spend(CELL_WORK)?;
                if meets(&near, column, &cell(matrix, size, column, row)) {
                    each(column, row);
                    count = count.saturating_add(1);
                }
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

/// A copy of the image `owner` holding `samples`, Flate-coded, with `set` set.
fn copy(
    job: &Job<'_>,
    stage: &mut Stage,
    owner: &Object,
    samples: &[u8],
    set: Vec<Entry>,
) -> Result<ObjectId, PdfError> {
    let entries = data::pairs(owner).map_or_else(Vec::new, |listed| {
        listed
            .iter()
            .filter(|pair| !LEFT_OUT.contains(&pair.0.as_slice()))
            .cloned()
            .collect()
    });
    job.spend(u64::try_from(samples.len()).unwrap_or(u64::MAX))?;
    let mut coded = Vec::new();
    let mut deflate = Deflate::new(&mut coded);
    deflate.write(samples, &mut coded);
    deflate.finish(&mut coded);
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
    let packing = (found.bits, found.components, found.stride);
    let samples = &mut found.data;
    let size = [found.columns, found.height];
    let _count: u64 = covered(job, (marks, matrix), size, &mut |column, row| {
        for component in 0..packing.1 {
            put(samples, packing, (column, row, component), value);
        }
    })?;
    let set = sizes(&found);
    copy(job, stage, mask.value(), &found.data, set)
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
    let opacity = split(&mut base);
    let mut veil = veil(job, owner, (stencil, &base), opacity)?;
    let blank = match veil {
        Veil::Stencil(value) => vec![value],
        Veil::Key(ref lows) => lows.clone(),
        Veil::Added(_) | Veil::Explicit(_) | Veil::Opacity(_) | Veil::Soft(_) => {
            vec![0; base.components]
        }
    };
    let size = [base.columns, base.height];
    let packing = (base.bits, base.components, base.stride);
    let (bits, columns) = (base.bits, base.columns);
    let mut target = match veil {
        Veil::Added(ref mut mask) => Some((mask, (1, 1, stride(columns, 1, 1)), 1)),
        Veil::Opacity(ref mut plane) => Some((plane, (bits, 1, stride(columns, 1, bits)), 0)),
        Veil::Explicit(_) | Veil::Key(_) | Veil::Soft(_) | Veil::Stencil(_) => None,
    };
    let samples = &mut base.data;
    let count = covered(job, (marks, &matrix), size, &mut |column, row| {
        for (component, value) in blank.iter().enumerate() {
            put(samples, packing, (column, row, component), *value);
        }
        if let Some(&mut (ref mut plane, plane_packing, value)) = target.as_mut() {
            put(plane, plane_packing, (column, row, 0), value);
        }
    })?;
    if count >= u64::from(base.columns).saturating_mul(u64::from(base.height)) {
        return Ok(None);
    }
    let mut set = sizes(&base);
    set.extend(veiled(run, (marks, stage, &matrix), &base, veil)?);
    set.extend(spelled(job, stage, &mut base)?);
    if base.jpx && !stencil {
        set.push((b"Decode", None));
    }
    copy(job, stage, owner, &base.data, set).map(Some)
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
fn sizes(found: &Unpacked) -> Vec<Entry> {
    vec![
        (b"Width", Some(Object::Integer(i64::from(found.columns)))),
        (b"Height", Some(Object::Integer(i64::from(found.height)))),
        (
            b"BitsPerComponent",
            Some(Object::Integer(i64::from(found.bits))),
        ),
    ]
}

/// The colour space `found`'s copy names in place of its own: the ICC profile or device space of
/// JPX data, or the device space of samples whose dictionary names none (8.9.5, 7.4.9).
fn spelled(
    job: &Job<'_>,
    stage: &mut Stage,
    found: &mut Unpacked,
) -> Result<Option<Entry>, PdfError> {
    if let Some(profile) = found.profile.take() {
        let components = i64::try_from(found.components).unwrap_or(i64::MAX);
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
    let space: &[u8] = match found.components {
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

/// The opacity channel of `found` taken out of its samples, which keep their colours: the
/// samples of a soft mask where it is opacity (7.4.9).
fn split(found: &mut Unpacked) -> Option<Vec<u8>> {
    let channel = found.alpha.take()?;
    let (count, bits) = (found.components, found.bits);
    let colours = count.saturating_sub(1);
    let (kept_stride, alpha_stride) = (
        stride(found.columns, colours, bits),
        stride(found.columns, 1, bits),
    );
    let rows = usize::try_from(found.height).unwrap_or(0);
    let mut kept = vec![0_u8; kept_stride.saturating_mul(rows)];
    let mut alpha = vec![0_u8; alpha_stride.saturating_mul(rows)];
    for row in 0..found.height {
        for column in 0..found.columns {
            for component in 0..count {
                let value = sample(
                    &found.data,
                    (bits, count, found.stride),
                    (column, row, component),
                );
                if component == channel {
                    put(&mut alpha, (bits, 1, alpha_stride), (column, row, 0), value);
                } else {
                    let at = component.saturating_sub(usize::from(component > channel));
                    put(
                        &mut kept,
                        (bits, colours, kept_stride),
                        (column, row, at),
                        value,
                    );
                }
            }
        }
    }
    found.data = kept;
    found.components = colours;
    found.stride = kept_stride;
    found.opacity.then_some(alpha)
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

/// How the samples of the image `owner`, `base` decoded, that a mark meets become transparent;
/// `opacity` is the channel of its JPX data that is opacity (7.4.9, 11.6.4.3).
fn veil(
    job: &Job<'_>,
    owner: &Object,
    (stencil, base): (bool, &Unpacked),
    opacity: Option<Vec<u8>>,
) -> Result<Veil, PdfError> {
    if stencil {
        return Ok(Veil::Stencil(unpainted(job, owner)?));
    }
    let soft = data::entry(job, owner, b"SMask")?;
    if matches!(*soft.value(), Object::Stream(..)) {
        return Ok(Veil::Soft(soft));
    }
    if let Some(plane) = opacity {
        return Ok(Veil::Opacity(plane));
    }
    let hard = data::entry(job, owner, b"Mask")?;
    if matches!(*hard.value(), Object::Stream(..)) {
        return Ok(Veil::Explicit(hard));
    }
    let lows = keyed(hard.value(), base.components);
    if !lows.is_empty() {
        return Ok(Veil::Key(lows));
    }
    let rows = usize::try_from(base.height).unwrap_or(usize::MAX);
    let bytes = stride(base.columns, 1, 1).saturating_mul(rows);
    job.fits(u64::try_from(bytes).unwrap_or(u64::MAX))?;
    Ok(Veil::Added(vec![0; bytes]))
}

/// The entry that gives `base`'s copy the mask `veil` makes, where one does.
fn veiled(
    run: &mut Run<'_, '_>,
    (marks, stage, matrix): (&[Mark], &mut Stage, &Matrix),
    base: &Unpacked,
    veil: Veil,
) -> Result<Option<Entry>, PdfError> {
    let job = run.job;
    let (key, made): (&[u8], _) = match veil {
        Veil::Key(_) | Veil::Stencil(_) => return Ok(None),
        Veil::Soft(found) => (b"SMask", masked(run, (marks, stage, matrix), &found, true)?),
        Veil::Explicit(found) => (b"Mask", masked(run, (marks, stage, matrix), &found, false)?),
        Veil::Opacity(plane) => {
            let mut entries = sizes(base);
            entries.push((b"ColorSpace", Some(name(b"DeviceGray"))));
            if !base.matte.is_empty() {
                let values = base.matte.iter().map(|low| Object::Integer(*low)).collect();
                entries.push((b"Matte", Some(Object::Array(values))));
            }
            (
                b"SMask",
                copy(job, stage, &plain(entries), &plane, Vec::new())?,
            )
        }
        Veil::Added(mask) => {
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
                copy(job, stage, &plain(entries), &mask, Vec::new())?,
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
