//! How a page is displayed: its boxes, `Rotate` and `UserUnit` (ISO 32000-2:2020, 7.7.3.3,
//! 14.11.2).
//!
//! Numbers are fixed point, [`SCALE`] units per point, exact to that unit.

use alloc::collections::BTreeSet;
use core::iter;

use crate::overlay::{Object, Overlay};
use crate::store::Objects;
use crate::{ErrorCode, PdfError};

/// The deviation of a page tree node met again among its own ancestors.
pub(crate) const CYCLE: &str = "a page tree node is its own ancestor";

/// Fraction digits of a fixed-point number.
pub(crate) const DIGITS: usize = 12;

/// The media box of a page without one: US Letter, as [`Page::boundary`](super::Page::boundary)
/// gives it.
const LETTER: [i128; 4] = [0, 0, 612 * SCALE, 792 * SCALE];

/// Fixed-point units per point: 10 to the [`DIGITS`].
pub(crate) const SCALE: i128 = 1_000_000_000_000;

/// An affine map in fixed point: x' = a·x + c·y + e, y' = b·x + d·y + f, as `cm` operands.
pub(crate) type Affine = [i128; 6];

/// A page as displayed.
pub(crate) struct Display {
    /// `Rotate` is not a multiple of 90 and counts as 0.
    #[cfg(feature = "rendering")]
    pub bad_rotate: bool,
    /// `UserUnit` is not a positive number and counts as 1.
    #[cfg(feature = "rendering")]
    pub bad_unit: bool,
    /// The map from the page as displayed, in points from its top-left corner, to default user
    /// space.
    pub matrix: Affine,
    /// `MediaBox` is missing and counts as US Letter.
    #[cfg(feature = "rendering")]
    pub missing_media: bool,
    /// The displayed width and height, in units of default user space.
    #[cfg(feature = "rendering")]
    pub size: [i128; 2],
    /// Points per unit of default user space.
    #[cfg(feature = "rendering")]
    pub unit: i128,
}

/// `units` as the shortest decimal in points.
pub(crate) fn decimal(units: i128) -> String {
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

/// The page `dictionary` as displayed: through `Rotate`, the crop box clipped to the media box,
/// and `UserUnit` (ISO 32000-2:2020, 7.7.3.3, 14.11.2).
pub(crate) fn display(
    objects: Objects<'_>,
    overlay: &Overlay,
    dictionary: &Object,
) -> Result<Display, PdfError> {
    let malformed = || objects.malformed();
    let boxed = |key: &[u8]| {
        rectangle(
            objects,
            overlay,
            inherited(objects, overlay, dictionary, key)?,
        )
    };
    let media = boxed(b"MediaBox")?;
    let [media_left, media_bottom, media_right, media_top] = media.unwrap_or(LETTER);
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
    let turned = rotation(objects, overlay, dictionary)?;
    let listed = overlay.entry(objects, dictionary, b"UserUnit")?;
    let given = units(&listed).filter(|unit| *unit > 0);
    let unit = given.unwrap_or(SCALE);
    // A displayed point is 1 / UserUnit units, along an axis of user space.
    let step = SCALE
        .checked_mul(SCALE)
        .and_then(|square| square.checked_div(unit))
        .ok_or_else(malformed)?;
    let back = step.checked_neg().ok_or_else(malformed)?;
    Ok(Display {
        #[cfg(feature = "rendering")]
        bad_rotate: turned.1,
        #[cfg(feature = "rendering")]
        bad_unit: given.is_none() && !matches!(listed, Object::Null),
        matrix: match turned.0 {
            90 => [0, step, step, 0, left, bottom],
            180 => [back, 0, 0, step, right, bottom],
            270 => [0, back, back, 0, right, top],
            _ => [step, 0, 0, back, left, top],
        },
        #[cfg(feature = "rendering")]
        missing_media: media.is_none(),
        #[cfg(feature = "rendering")]
        size: {
            // Saturating: a box too large to render still takes an image.
            let (across, down) = (right.saturating_sub(left), top.saturating_sub(bottom));
            if matches!(turned.0, 90 | 270) {
                [down, across]
            } else {
                [across, down]
            }
        },
        #[cfg(feature = "rendering")]
        unit,
    })
}

/// The `Rotate` of the page `dictionary`, inherited, as 0, 90, 180 or 270, and whether it is
/// wrong and counts as 0 (ISO 32000-2:2020, 7.7.3.3).
pub(crate) fn rotation(
    objects: Objects<'_>,
    overlay: &Overlay,
    dictionary: &Object,
) -> Result<(i64, bool), PdfError> {
    Ok(match inherited(objects, overlay, dictionary, b"Rotate")? {
        Object::Integer(degrees) if degrees.checked_rem(90) == Some(0) => {
            (degrees.checked_rem_euclid(360).unwrap_or_default(), false)
        }
        Object::Null => (0, false),
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => (0, true),
    })
}

/// `value` in fixed point, its shortest decimal cut to [`DIGITS`] fraction digits; `None` when
/// not finite or too large.
pub(crate) fn fixed(value: f64) -> Option<i128> {
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
/// resolved; null when none has it, a node not a dictionary malformed (ISO 32000-2:2020, 7.3.10,
/// 7.7.3.4).
pub(crate) fn inherited(
    objects: Objects<'_>,
    overlay: &Overlay,
    node: &Object,
    key: &[u8],
) -> Result<Object, PdfError> {
    let mut current = node.clone();
    let mut visited = BTreeSet::new();
    for _ in 0..=objects.depth_limit() {
        let found = overlay.entry(objects, &current, key)?;
        if !matches!(found, Object::Null) {
            return Ok(found);
        }
        let link = match current {
            Object::Dictionary(ref entries) => entries.iter().find(|entry| entry.0 == b"Parent"),
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => None,
        };
        if let Some(&(_, Object::Reference(id))) = link {
            // A tree has no cycle (7.7.3.4): an ancestor met again ends the walk.
            if !visited.insert((id.number, id.generation)) {
                objects.deviation("7.7.3.4", CYCLE, None)?;
                return Ok(Object::Null);
            }
        }
        current = match overlay.entry(objects, &current, b"Parent")? {
            Object::Null => return Ok(Object::Null),
            parent @ Object::Dictionary(_) => parent,
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => return Err(objects.malformed()),
        };
    }
    Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()))
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
pub(crate) fn streams(
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
pub(crate) fn units(value: &Object) -> Option<i128> {
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
