//! Redaction marks: their areas and interior colours (ISO 32000-2:2020, 12.5.2, 12.5.6.23).

use crate::PdfError;
use crate::overlay::{Object, Overlay};
use crate::render::fixed::Fixed;
use crate::render::geometry::Point;
use crate::store::Objects;

/// Units of a fraction digit written: ten of them.
const DIGITS: u128 = 10_000_000_000;

/// A redaction mark.
pub(super) struct Mark {
    /// Its areas in default user space: convex polygons, counterclockwise.
    pub areas: Vec<Vec<Point>>,
    /// `IC`; empty leaves the area empty.
    pub color: Vec<Fixed>,
}

/// Twice the signed area of the triangle `origin`, `first`, `second`, in squared bits.
fn cross(origin: Point, first: Point, second: Point) -> i128 {
    let along = |to: Point| {
        (
            i128::from(to.across.bits()).saturating_sub(i128::from(origin.across.bits())),
            i128::from(to.down.bits()).saturating_sub(i128::from(origin.down.bits())),
        )
    };
    let ((ax, ay), (bx, by)) = (along(first), along(second));
    ax.saturating_mul(by).saturating_sub(ay.saturating_mul(bx))
}

/// Appends `value` in decimal, to ten fraction digits.
pub(super) fn decimal(out: &mut String, value: Fixed) {
    let bits = i128::from(value.bits());
    let scaled = bits
        .unsigned_abs()
        .saturating_mul(DIGITS)
        .saturating_add(1 << 31_u32)
        >> 32_u32;
    let whole = scaled.checked_div(DIGITS).unwrap_or_default();
    let fraction = scaled.checked_rem(DIGITS).unwrap_or_default();
    if bits < 0 && scaled > 0 {
        out.push('-');
    }
    out.push_str(&whole.to_string());
    if fraction > 0 {
        let digits = format!("{fraction:010}");
        out.push('.');
        out.push_str(digits.trim_end_matches('0'));
    }
}

/// Content that fills each mark's area with its colour (8.5.3.3).
pub(super) fn fill(marks: &[Mark]) -> String {
    let mut out = String::new();
    for mark in marks {
        let operator = match mark.color.len() {
            1 => "g",
            3 => "rg",
            4 => "k",
            _ => continue,
        };
        out.push('q');
        for value in &mark.color {
            out.push(' ');
            decimal(&mut out, *value);
        }
        out.push(' ');
        out.push_str(operator);
        for area in mark.areas.iter().filter(|area| area.len() >= 3) {
            for (at, point) in area.iter().enumerate() {
                for value in [point.across, point.down] {
                    out.push(' ');
                    decimal(&mut out, value);
                }
                out.push_str(if at == 0 { " m" } else { " l" });
            }
            out.push_str(" h");
        }
        out.push_str(" f Q\n");
    }
    out
}

/// The convex hull of `points`, counterclockwise, the leftmost lowest first.
fn hull(mut points: Vec<Point>) -> Vec<Point> {
    points.sort_by_key(|point| (point.across, point.down));
    points.dedup_by(|first, second| first.across == second.across && first.down == second.down);
    if points.len() < 3 {
        return points;
    }
    let half = |ordered: &mut dyn Iterator<Item = Point>| {
        let mut chain: Vec<Point> = Vec::new();
        for point in ordered {
            while let [.., before, last] = *chain.as_slice()
                && cross(before, last, point) <= 0
            {
                let _last: Option<Point> = chain.pop();
            }
            chain.push(point);
        }
        let _last: Option<Point> = chain.pop();
        chain
    };
    let mut lower = half(&mut points.iter().copied());
    lower.extend(half(&mut points.iter().rev().copied()));
    lower
}

/// Whether the polygon `shape` meets the area of a mark: their insides overlap.
pub(super) fn meets(marks: &[Mark], shape: &[Point]) -> bool {
    marks
        .iter()
        .flat_map(|mark| mark.areas.iter())
        .any(|area| overlap(area, shape))
}

/// The numbers of the array `value`, through references; `None` when another object.
fn numbers(
    objects: Objects<'_>,
    overlay: &Overlay,
    value: Object,
) -> Result<Option<Vec<Fixed>>, PdfError> {
    let Object::Array(items) = overlay.direct(objects, value)? else {
        return Ok(None);
    };
    let mut found = Vec::new();
    for item in items {
        let Some(number) = Fixed::from_object(&overlay.direct(objects, item)?) else {
            return Ok(None);
        };
        found.push(number);
    }
    Ok(Some(found))
}

/// Whether the insides of the convex polygons `first` and `second` overlap: no axis of theirs
/// separates them.
fn overlap(first: &[Point], second: &[Point]) -> bool {
    if first.is_empty() || second.is_empty() {
        return false;
    }
    let one = Fixed::from_i64(1).bits();
    let mut axes = vec![(one, 0_i64), (0_i64, one)];
    for polygon in [first, second] {
        for (at, start) in polygon.iter().enumerate() {
            let Some(end) = polygon
                .get(at.saturating_add(1))
                .or_else(|| polygon.first())
            else {
                continue;
            };
            let normal = (
                start.down.bits().saturating_sub(end.down.bits()),
                end.across.bits().saturating_sub(start.across.bits()),
            );
            if normal != (0, 0) {
                axes.push(normal);
            }
        }
    }
    axes.iter().all(|axis| {
        let (low, high) = span(first, *axis);
        let (other_low, other_high) = span(second, *axis);
        high > other_low && other_high > low
    })
}

/// The mark `annotation` is, when it is one: over its `QuadPoints`, else its `Rect`.
pub(super) fn read(
    objects: Objects<'_>,
    overlay: &Overlay,
    annotation: &Object,
) -> Result<Option<Mark>, PdfError> {
    let kind = overlay.entry(objects, annotation, b"Subtype")?;
    if !matches!(kind, Object::Name(ref name) if name == b"Redact") {
        return Ok(None);
    }
    let at = |pair: &[Fixed]| match *pair {
        [across, down] => Some(Point { across, down }),
        _ => None,
    };
    let quads = numbers(
        objects,
        overlay,
        annotation.get(b"QuadPoints").unwrap_or(Object::Null),
    )?
    .filter(|found| !found.is_empty() && found.len().is_multiple_of(8));
    let areas = if let Some(found) = quads {
        found
            .chunks(8)
            .map(|quad| hull(quad.chunks(2).filter_map(at).collect()))
            .collect()
    } else {
        let rect = numbers(
            objects,
            overlay,
            annotation.get(b"Rect").unwrap_or(Object::Null),
        )?;
        match rect.as_deref() {
            Some(&[left, bottom, right, top]) => vec![rectangle([
                left.min(right),
                bottom.min(top),
                left.max(right),
                bottom.max(top),
            ])],
            _ => Vec::new(),
        }
    };
    let color = numbers(
        objects,
        overlay,
        annotation.get(b"IC").unwrap_or(Object::Null),
    )?;
    Ok(Some(Mark {
        areas,
        color: color.unwrap_or_default(),
    }))
}

/// The corners of the rectangle `corners`: left, bottom, right and top.
pub(super) fn rectangle(corners: [Fixed; 4]) -> Vec<Point> {
    let [left, bottom, right, top] = corners;
    [(left, bottom), (right, bottom), (right, top), (left, top)]
        .map(|(across, down)| Point { across, down })
        .to_vec()
}

/// The least and most of `polygon` along `axis`.
fn span(polygon: &[Point], axis: (i64, i64)) -> (i128, i128) {
    let (ax, ay) = (i128::from(axis.0), i128::from(axis.1));
    let mut found = (i128::MAX, i128::MIN);
    for point in polygon {
        let along = ax
            .saturating_mul(i128::from(point.across.bits()))
            .saturating_add(ay.saturating_mul(i128::from(point.down.bits())));
        found = (found.0.min(along), found.1.max(along));
    }
    found
}
