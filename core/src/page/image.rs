//! Drawing an image file on a page (ISO 32000-2:2020, 8.9).
//!
//! Numbers are fixed point, [`SCALE`] units per point, so placement is exact to that unit.

use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use core::iter;

use super::Page;
use crate::drawing::DrawImageOptions;
use crate::input::Source;
use crate::objects::ObjectId;
use crate::overlay::{Data, Dictionary, Object, Overlay, Produce};
use crate::pull::Pull;
use crate::raster::{self, Filter, Image, Profile};
use crate::store::Objects;
use crate::{ErrorCode, PdfError};

/// Fraction digits of a fixed-point number.
const DIGITS: usize = 12;

/// The operation errors name.
const DRAW: &str = "Page::draw_image";

/// The media box of a page without one: US Letter, as [`Page::boundary`] gives it.
const LETTER: [i128; 4] = [0, 0, 612 * SCALE, 792 * SCALE];

/// The end of a content stream drawn here.
const PAINT: &[u8] = b" Do Q\n";

/// Fixed-point units per point: 10 to the [`DIGITS`].
const SCALE: i128 = 1_000_000_000_000;

/// A stream of an image file.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// The samples.
    Data,
    /// The soft mask's samples.
    Mask,
    /// The ICC profile.
    Profile,
}

/// One stream of an image file, read from the file as it is written.
#[derive(Debug)]
struct Part {
    /// The image.
    image: Arc<Image>,
    /// Which stream.
    kind: Kind,
}

/// An affine map in fixed point: x' = a·x + c·y + e, y' = b·x + d·y + f, as `cm` operands.
type Affine = [i128; 6];

impl Produce for Part {
    fn length(&self) -> u64 {
        match self.kind {
            Kind::Data => self.image.length(),
            Kind::Mask => self.image.mask().map_or(0, |mask| mask.1),
            Kind::Profile => self.image.profile().map_or(0, Profile::length),
        }
    }

    fn pull(&self) -> Option<Box<dyn Pull + '_>> {
        match self.kind {
            Kind::Data => self.image.data(),
            Kind::Mask => None,
            Kind::Profile => self.image.profile_data(),
        }
    }

    fn write(&self, out: &mut dyn FnMut(&[u8]) -> Result<(), PdfError>) -> Result<(), PdfError> {
        match self.kind {
            Kind::Data => self.image.write_data(out),
            Kind::Mask => self.image.write_mask(out),
            Kind::Profile => self.image.write_profile(out),
        }
    }
}

/// `units` as the shortest decimal in points.
fn decimal(units: i128) -> String {
    let magnitude = units.unsigned_abs();
    let scale = SCALE.unsigned_abs();
    let whole = magnitude.checked_div(scale).unwrap_or_default();
    let fraction = magnitude.checked_rem(scale).unwrap_or_default();
    let mut text = if units < 0 {
        format!("-{whole}")
    } else {
        whole.to_string()
    };
    if fraction != 0 {
        text.push('.');
        text.push_str(format!("{fraction:0DIGITS$}").trim_end_matches('0'));
    }
    text
}

/// Draws the image file in `source` on `page` as `options` say: an image `XObject` named in the
/// page's resources, painted by a content stream appended after the page's content, which is
/// wrapped in `q` … `Q` so its graphics state does not reach the image (ISO 32000-2:2020, 7.8.2,
/// 8.4.2, 8.8, 8.9.5).
pub(super) fn draw(
    page: &Page,
    source: &Source,
    options: &DrawImageOptions,
) -> Result<(), PdfError> {
    let invalid = || PdfError::new(ErrorCode::InvalidArgument, DRAW);
    let length = |given: Option<f64>| {
        given
            .map(|value| fixed(value).filter(|units| *units > 0).ok_or_else(invalid))
            .transpose()
    };
    let size = (length(options.width)?, length(options.height)?);
    let corner = [options.x, options.y].map(fixed);
    let [Some(left), Some(top)] = corner else {
        return Err(invalid());
    };
    if options.frame != 0 {
        return Err(invalid());
    }
    let objects = page.store.objects(DRAW);
    let overlay = &*page.overlay;
    let mut dictionary = overlay.resolve(objects, page.id)?;
    if !matches!(dictionary, Object::Dictionary(_)) {
        return Err(objects.malformed());
    }
    let image = raster::read(source, options.format, objects.decoded_limit())?;
    if image.dropped_profile() {
        objects.deviation(
            "8.6.5.5",
            "an image's ICC profile is not of its colour space; the device space applies",
            None,
        )?;
    }
    let orientation = if options.ignore_orientation {
        1
    } else {
        image.orientation()
    };
    let placed = place(
        &image,
        orientation,
        size,
        (left, top),
        view(objects, overlay, &dictionary)?,
    )
    .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, DRAW))?;
    let mut resources = dictionary_entries(
        objects,
        overlay,
        inherited(objects, overlay, &dictionary, b"Resources")?,
    )?;
    let held = resources
        .iter()
        .find(|entry| entry.0 == b"XObject")
        .map_or(Object::Null, |entry| entry.1.clone());
    let mut xobjects = dictionary_entries(objects, overlay, held)?;
    let label = {
        let taken: BTreeSet<&[u8]> = xobjects.iter().map(|entry| entry.0.as_slice()).collect();
        (1..=xobjects.len().saturating_add(1))
            .map(|number| format!("Im{number}").into_bytes())
            .find(|label| !taken.contains(label.as_slice()))
            .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, DRAW))?
    };
    let existing = streams(objects, overlay, dictionary.get(b"Contents"))?;
    // Content ending in a stream drawn here leaves the graphics state as it found it.
    let drawn = match existing.last() {
        Some(&Object::Reference(id)) => matches!(
            overlay.get(id.number)?,
            Some((_, Object::Stream(_, Data::Bytes(bytes)))) if bytes.ends_with(PAINT)
        ),
        Some(_) | None => false,
    };
    let wrap = !existing.is_empty() && !drawn;
    let mut operators = Vec::from(if wrap { "Q\nq" } else { "q" });
    for number in placed {
        operators.push(b' ');
        operators.extend_from_slice(decimal(number).as_bytes());
    }
    operators.extend_from_slice(b" cm /");
    operators.extend_from_slice(&label);
    operators.extend_from_slice(PAINT);
    let content = |bytes: Vec<u8>| overlay.add(Object::Stream(Vec::new(), Data::Bytes(bytes)));
    let contents = if existing.is_empty() {
        Object::Reference(content(operators)?)
    } else {
        let mut items = Vec::new();
        if wrap {
            items.push(Object::Reference(content(b"q\n".to_vec())?));
        }
        items.extend(existing);
        items.push(Object::Reference(content(operators)?));
        Object::Array(items)
    };
    xobjects.push((label, Object::Reference(xobject(overlay, image)?)));
    resources.retain(|entry| entry.0 != b"XObject");
    resources.push((b"XObject".to_vec(), Object::Dictionary(xobjects)));
    dictionary.set(b"Resources", Some(Object::Dictionary(resources)));
    dictionary.set(b"Contents", Some(contents));
    overlay.set(page.id, dictionary);
    Ok(())
}

/// The entries of the dictionary `value`; none when null.
fn dictionary_entries(
    objects: Objects<'_>,
    overlay: &Overlay,
    value: Object,
) -> Result<Dictionary, PdfError> {
    match overlay.direct(objects, value)? {
        Object::Null => Ok(Vec::new()),
        Object::Dictionary(entries) => Ok(entries),
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => Err(objects.malformed()),
    }
}

/// `value` in fixed point, its shortest decimal cut to [`DIGITS`] fraction digits; `None` when
/// not finite or too large.
fn fixed(value: f64) -> Option<i128> {
    let text = value.to_string();
    let magnitude = text.trim_start_matches('-');
    let (whole, fraction) = magnitude.split_once('.').unwrap_or((magnitude, ""));
    let digits: String = whole
        .chars()
        .chain(fraction.chars().chain(iter::repeat('0')).take(DIGITS))
        .collect();
    let units: i128 = digits.parse().ok()?;
    if text.starts_with('-') {
        units.checked_neg()
    } else {
        Some(units)
    }
}

/// The value of `key` in the page tree node `node` or the nearest ancestor that has it,
/// resolved; null when none has it (ISO 32000-2:2020, 7.7.3.4).
fn inherited(
    objects: Objects<'_>,
    overlay: &Overlay,
    node: &Object,
    key: &[u8],
) -> Result<Object, PdfError> {
    let mut current = node.clone();
    for _ in 0..=objects.depth_limit() {
        let found = overlay.entry(objects, &current, key)?;
        if !matches!(found, Object::Null) {
            return Ok(found);
        }
        current = overlay.entry(objects, &current, b"Parent")?;
        if !matches!(current, Object::Dictionary(_)) {
            return Ok(Object::Null);
        }
    }
    Err(PdfError::new(ErrorCode::LimitExceeded, DRAW))
}

/// The `cm` operands that paint the unit square as `image` shown in Exif `orientation`
/// (CIPA DC-008-2019, 4.6.4 A), its top-left corner at `corner` and `size` points on the page as
/// displayed, which `display` maps to user space; `None` past the fixed-point range.
fn place(
    image: &Image,
    orientation: u8,
    size: (Option<i128>, Option<i128>),
    corner: (i128, i128),
    display: Affine,
) -> Option<Affine> {
    let (columns, rows) = (i128::from(image.width()), i128::from(image.height()));
    // The displayed fraction across and down, constant, u and v terms, of unit square u, v,
    // whose top row v = 1 holds the stored first row (8.9.4).
    let (across, down): ([i128; 3], [i128; 3]) = match orientation {
        2 => ([1, -1, 0], [1, 0, -1]),
        3 => ([1, -1, 0], [0, 0, 1]),
        4 => ([0, 1, 0], [0, 0, 1]),
        5 => ([1, 0, -1], [0, 1, 0]),
        6 => ([0, 0, 1], [0, 1, 0]),
        7 => ([0, 0, 1], [1, -1, 0]),
        8 => ([1, 0, -1], [1, -1, 0]),
        _ => ([0, 1, 0], [1, 0, -1]),
    };
    let (wide, high) = if orientation >= 5 {
        (rows, columns)
    } else {
        (columns, rows)
    };
    let ratio = |given: i128, by: i128, over: i128| given.checked_mul(by)?.checked_div(over);
    let (width, height) = match size {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) => (width, ratio(width, high, wide)?),
        (None, Some(height)) => (ratio(height, wide, high)?, height),
        (None, None) => (wide.checked_mul(SCALE)?, high.checked_mul(SCALE)?),
    };
    let terms = |start: i128, side: i128, fractions: [i128; 3]| -> Option<[i128; 3]> {
        let [constant, along_u, along_v] = fractions;
        Some([
            start.checked_add(side.checked_mul(constant)?)?,
            side.checked_mul(along_u)?,
            side.checked_mul(along_v)?,
        ])
    };
    let [x0, xu, xv] = terms(corner.0, width, across)?;
    let [y0, yu, yv] = terms(corner.1, height, down)?;
    let [m11, m12, m21, m22, dx, dy] = display;
    let apply = |row: i128, column: i128, horizontal: i128, vertical: i128| {
        product(row, horizontal)?.checked_add(product(column, vertical)?)
    };
    Some([
        apply(m11, m21, xu, yu)?,
        apply(m12, m22, xu, yu)?,
        apply(m11, m21, xv, yv)?,
        apply(m12, m22, xv, yv)?,
        dx.checked_add(apply(m11, m21, x0, y0)?)?,
        dy.checked_add(apply(m12, m22, x0, y0)?)?,
    ])
}

/// The fixed-point product of `left` and `right`.
fn product(left: i128, right: i128) -> Option<i128> {
    left.checked_mul(right)?.checked_div(SCALE)
}

/// The rectangle `value`, normalized to `[left, bottom, right, top]`; `None` when null
/// (ISO 32000-2:2020, 7.9.5).
fn rectangle(
    objects: Objects<'_>,
    overlay: &Overlay,
    value: Object,
) -> Result<Option<[i128; 4]>, PdfError> {
    let items = match overlay.direct(objects, value)? {
        Object::Null => return Ok(None),
        Object::Array(items) if items.len() == 4 => items,
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => return Err(objects.malformed()),
    };
    let mut numbers = [0_i128; 4];
    for (slot, item) in numbers.iter_mut().zip(items) {
        *slot = units(&overlay.direct(objects, item)?).ok_or_else(|| objects.malformed())?;
    }
    let [x0, y0, x1, y1] = numbers;
    Ok(Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]))
}

/// The content streams `contents` names, in order (ISO 32000-2:2020, 7.7.3.3).
fn streams(
    objects: Objects<'_>,
    overlay: &Overlay,
    contents: Option<Object>,
) -> Result<Vec<Object>, PdfError> {
    let Some(value) = contents else {
        return Ok(Vec::new());
    };
    match overlay.direct(objects, value.clone())? {
        Object::Null => Ok(Vec::new()),
        Object::Array(items) => Ok(items),
        Object::Stream(..) => Ok(vec![value]),
        Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Real(_)
        | Object::Reference(_)
        | Object::String(_) => Err(objects.malformed()),
    }
}

/// The number `value` in fixed point.
fn units(value: &Object) -> Option<i128> {
    match *value {
        Object::Integer(integer) => i128::from(integer).checked_mul(SCALE),
        Object::Real(real) => fixed(real),
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Name(_)
        | Object::Null
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => None,
    }
}

/// The map from the page `dictionary` as displayed, in points from its top-left corner, to its
/// default user space: through `Rotate`, the crop box clipped to the media box, and `UserUnit`
/// (ISO 32000-2:2020, 7.7.3.3, 14.11.2).
fn view(objects: Objects<'_>, overlay: &Overlay, dictionary: &Object) -> Result<Affine, PdfError> {
    let malformed = || objects.malformed();
    let boxed = |key: &[u8]| {
        rectangle(
            objects,
            overlay,
            inherited(objects, overlay, dictionary, key)?,
        )
    };
    let [media_left, media_bottom, media_right, media_top] = boxed(b"MediaBox")?.unwrap_or(LETTER);
    let [left, bottom, right, top] = boxed(b"CropBox")?.map_or(
        [media_left, media_bottom, media_right, media_top],
        |[crop_left, crop_bottom, crop_right, crop_top]| {
            let low_x = crop_left.max(media_left);
            let low_y = crop_bottom.max(media_bottom);
            [
                low_x,
                low_y,
                crop_right.min(media_right).max(low_x),
                crop_top.min(media_top).max(low_y),
            ]
        },
    );
    let rotation = match inherited(objects, overlay, dictionary, b"Rotate")? {
        Object::Integer(degrees) if degrees.checked_rem(90) == Some(0) => {
            degrees.checked_rem_euclid(360).unwrap_or_default()
        }
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => 0,
    };
    let unit = units(&overlay.entry(objects, dictionary, b"UserUnit")?)
        .filter(|unit| *unit > 0)
        .unwrap_or(SCALE);
    // A displayed point is 1 / UserUnit units, along an axis of user space.
    let step = SCALE
        .checked_mul(SCALE)
        .and_then(|square| square.checked_div(unit))
        .ok_or_else(malformed)?;
    let back = step.checked_neg().ok_or_else(malformed)?;
    Ok(match rotation {
        90 => [0, step, step, 0, left, bottom],
        180 => [back, 0, 0, step, right, bottom],
        270 => [0, back, back, 0, right, top],
        _ => [step, 0, 0, back, left, top],
    })
}

/// Adds `image` as an image `XObject`, with its soft mask and ICC profile, its data read from the
/// file at save (ISO 32000-2:2020, 7.4.4.4, 7.4.8, 8.6.5.5, 8.6.6.3, 8.9.5, 11.6.5.3).
fn xobject(overlay: &Overlay, image: Image) -> Result<ObjectId, PdfError> {
    let shared = Arc::new(image);
    let part = |kind| {
        Data::Produced(Arc::new(Part {
            image: Arc::clone(&shared),
            kind,
        }))
    };
    let name = |bytes: &[u8]| Object::Name(bytes.to_vec());
    let entry = |key: &[u8], value| (key.to_vec(), value);
    let components = shared.components();
    let base = match shared.profile() {
        Some(profile) => {
            let mut entries = vec![entry(b"N", Object::Integer(i64::from(components)))];
            if profile.compressed() {
                entries.push(entry(b"Filter", name(b"FlateDecode")));
            }
            let stream = overlay.add(Object::Stream(entries, part(Kind::Profile)))?;
            Object::Array(vec![name(b"ICCBased"), Object::Reference(stream)])
        }
        None => name(match components {
            1 => b"DeviceGray",
            3 => b"DeviceRGB",
            _ => b"DeviceCMYK",
        }),
    };
    let space = match shared.palette() {
        Some(table) => {
            let high = table
                .len()
                .checked_div(3)
                .and_then(|entries| entries.checked_sub(1))
                .and_then(|high| i64::try_from(high).ok())
                .ok_or_else(|| PdfError::new(ErrorCode::Malformed, DRAW))?;
            Object::Array(vec![
                name(b"Indexed"),
                base,
                Object::Integer(high),
                Object::String(table.to_vec()),
            ])
        }
        None => base,
    };
    let (width, height) = (
        Object::Integer(i64::from(shared.width())),
        Object::Integer(i64::from(shared.height())),
    );
    let bits = Object::Integer(i64::from(shared.bits()));
    let head = |family: Object, depth: Object| {
        vec![
            entry(b"Subtype", name(b"Image")),
            entry(b"Width", width.clone()),
            entry(b"Height", height.clone()),
            entry(b"ColorSpace", family),
            entry(b"BitsPerComponent", depth),
        ]
    };
    let mut entries: Dictionary = head(space, bits.clone());
    if shared.decode_inverted() {
        let pairs = iter::repeat([Object::Integer(1), Object::Integer(0)]);
        entries.push(entry(
            b"Decode",
            Object::Array(pairs.take(usize::from(components)).flatten().collect()),
        ));
    }
    if let Some((mask_bits, _)) = shared.mask() {
        let mask = head(name(b"DeviceGray"), Object::Integer(i64::from(mask_bits)));
        let stream = overlay.add(Object::Stream(mask, part(Kind::Mask)))?;
        entries.push(entry(b"SMask", Object::Reference(stream)));
    }
    match shared.filter() {
        Some(Filter::Dct) => entries.push(entry(b"Filter", name(b"DCTDecode"))),
        Some(Filter::Png { colors }) => {
            entries.push(entry(b"Filter", name(b"FlateDecode")));
            entries.push(entry(
                b"DecodeParms",
                Object::Dictionary(vec![
                    entry(b"Predictor", Object::Integer(15)),
                    entry(b"Colors", Object::Integer(i64::from(colors))),
                    entry(b"BitsPerComponent", bits),
                    entry(b"Columns", width),
                ]),
            ));
        }
        None => {}
    }
    overlay.add(Object::Stream(entries, part(Kind::Data)))
}
