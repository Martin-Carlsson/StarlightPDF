//! Redaction marks: their areas and interior colours (ISO 32000-2:2020, 12.5.2, 12.5.6.23).

use crate::PdfError;
use crate::overlay::{Object, Overlay};
use crate::render::fixed::Fixed;
use crate::render::geometry::Point;
use crate::render::job::Job;
use crate::render::redact::walk;
use crate::store::Objects;

/// Units of a fraction digit written: ten of them.
const DIGITS: u128 = 10_000_000_000;

/// The most areas a leaf of [`Marks`]' tree holds.
const LEAF: usize = 4;

/// Work of testing a box of [`Marks`]' tree, or an area.
const TEST_WORK: u64 = 8;

/// An area of [`Marks`]: its box, its mark and its place there.
type Entry = ([Fixed; 4], usize, usize);

/// The marks of a page, their areas in a tree of boxes halving them by turns across and up, so
/// that finding those a shape meets tests few.
pub(super) struct Marks {
    /// The areas, in the tree's order.
    areas: Vec<Entry>,
    /// The marks.
    list: Vec<Mark>,
    /// The box of each node: the first holds all, node `n` the halves `2n + 1` and `2n + 2`.
    nodes: Vec<[Fixed; 4]>,
}

/// A redaction mark.
pub(super) struct Mark {
    /// Its areas in default user space: convex polygons, counterclockwise.
    pub areas: Vec<Vec<Point>>,
    /// `IC`; empty leaves the area empty.
    pub color: Vec<Fixed>,
}

impl Marks {
    /// The areas of `list` that `shape` meets, by mark and place, in order; only the first unless
    /// `all`. Each box tested is counted.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorCode::LimitExceeded`] past the work limit.
    pub(super) fn find(
        &self,
        job: &Job<'_>,
        shape: &[Point],
        all: bool,
    ) -> Result<Vec<(usize, usize)>, PdfError> {
        let mut found = Vec::new();
        if let Some(bounds) = extent(shape) {
            self.visit(
                job,
                (shape, bounds),
                (0, 0, self.areas.len()),
                (&mut found, all),
            )?;
        }
        found.sort_unstable();
        Ok(found)
    }

    /// The marks.
    pub(super) fn list(&self) -> &[Mark] {
        &self.list
    }

    /// Whether `shape` meets a mark's area, as [`Self::find`] counts.
    ///
    /// # Errors
    ///
    /// As [`Self::find`].
    pub(super) fn meets(&self, job: &Job<'_>, shape: &[Point]) -> Result<bool, PdfError> {
        Ok(!self.find(job, shape, false)?.is_empty())
    }

    /// `list` with its areas in a tree.
    pub(super) fn new(list: Vec<Mark>) -> Self {
        let mut areas: Vec<Entry> = list
            .iter()
            .enumerate()
            .flat_map(|(at, mark)| {
                mark.areas
                    .iter()
                    .enumerate()
                    .filter_map(move |(place, area)| extent(area).map(|found| (found, at, place)))
            })
            .collect();
        let mut nodes = Vec::new();
        built(&mut areas, &mut nodes, 0, true);
        Self { areas, list, nodes }
    }

    /// Adds to `found` the areas from `low` to `high` under `node` that `shape`, within
    /// `bounds`, meets; stops at the first unless `all`.
    fn visit(
        &self,
        job: &Job<'_>,
        (shape, bounds): (&[Point], [Fixed; 4]),
        (node, low, high): (usize, usize, usize),
        (found, all): (&mut Vec<(usize, usize)>, bool),
    ) -> Result<(), PdfError> {
        job.spend(TEST_WORK)?;
        if !self
            .nodes
            .get(node)
            .is_some_and(|held| touches(*held, bounds))
        {
            return Ok(());
        }
        let count = high.saturating_sub(low);
        if count > LEAF {
            let middle = low.saturating_add(count >> 1_u32);
            let first = node.saturating_mul(2).saturating_add(1);
            self.visit(
                job,
                (shape, bounds),
                (first, low, middle),
                (&mut *found, all),
            )?;
            if !all && !found.is_empty() {
                return Ok(());
            }
            let second = first.saturating_add(1);
            return self.visit(job, (shape, bounds), (second, middle, high), (found, all));
        }
        for &(held, mark, place) in self.areas.get(low..high).unwrap_or_default() {
            job.spend(TEST_WORK)?;
            let area = self.list.get(mark).and_then(|owner| owner.areas.get(place));
            if touches(held, bounds) && area.is_some_and(|inside| overlap(inside, shape)) {
                found.push((mark, place));
                if !all {
                    break;
                }
            }
        }
        Ok(())
    }
}

/// Orders `areas` as the tree of node `node` takes them, halved `across` or up, and sets the
/// boxes of its nodes in `nodes`.
fn built(areas: &mut [Entry], nodes: &mut Vec<[Fixed; 4]>, node: usize, across: bool) {
    if areas.len() > LEAF {
        areas.sort_unstable_by_key(|entry| {
            let [left, bottom, right, top] = entry.0;
            if across {
                (left.plus(right), bottom.plus(top))
            } else {
                (bottom.plus(top), left.plus(right))
            }
        });
        let (low, high) = areas.split_at_mut(areas.len() >> 1_u32);
        let first = node.saturating_mul(2).saturating_add(1);
        built(low, nodes, first, !across);
        built(high, nodes, first.saturating_add(1), !across);
    }
    let held = areas
        .iter()
        .fold(None, |held, entry| Some(walk::joined(held, entry.0)));
    let Some(found) = held else {
        return;
    };
    if nodes.len() <= node {
        nodes.resize(node.saturating_add(1), [Fixed::default(); 4]);
    }
    if let Some(slot) = nodes.get_mut(node) {
        *slot = found;
    }
}

/// Twice the signed area of the triangle `origin`, `first`, `second`, in squared bits.
pub(super) fn cross(origin: Point, first: Point, second: Point) -> i128 {
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
pub(super) fn hull(mut points: Vec<Point>) -> Vec<Point> {
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
pub(super) fn overlap(first: &[Point], second: &[Point]) -> bool {
    if first.is_empty() || second.is_empty() {
        return false;
    }
    let one = Fixed::from_i64(1).bits();
    let normals = [first, second].into_iter().flat_map(|polygon| {
        polygon
            .iter()
            .zip(polygon.iter().skip(1).chain(polygon.first()))
            .map(|(start, end)| {
                (
                    start.down.bits().saturating_sub(end.down.bits()),
                    end.across.bits().saturating_sub(start.across.bits()),
                )
            })
    });
    [(one, 0_i64), (0_i64, one)]
        .into_iter()
        .chain(normals)
        .filter(|normal| *normal != (0, 0))
        .all(|axis| {
            let (low, high) = span(first, axis);
            let (other_low, other_high) = span(second, axis);
            high > other_low && other_high > low
        })
}

/// The mark `annotation` is, when it is one: over its `QuadPoints`, else, or where they cover no
/// area, its `Rect`; none, with a warning, where it has neither.
pub(super) fn read(job: &Job<'_>, annotation: &Object) -> Result<Option<Mark>, PdfError> {
    let (objects, overlay) = (job.objects, &*job.overlay);
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
    )?;
    let mut areas: Vec<Vec<Point>> = quads
        .iter()
        .filter(|found| found.len().is_multiple_of(8))
        .flat_map(|found| found.chunks(8))
        .map(|quad| hull(quad.chunks(2).filter_map(at).collect()))
        .filter(|area| area.len() >= 3)
        .collect();
    if quads.is_some() && areas.is_empty() {
        job.warn("12.5.6.23", "a mark's QuadPoints cover no area")?;
    }
    if areas.is_empty() {
        let rect = numbers(
            objects,
            overlay,
            annotation.get(b"Rect").unwrap_or(Object::Null),
        )?;
        let Some(&[left, bottom, right, top]) = rect.as_deref() else {
            job.warn("12.5.2", "a mark has neither a valid Rect nor QuadPoints")?;
            return Ok(None);
        };
        areas.push(rectangle([
            left.min(right),
            bottom.min(top),
            left.max(right),
            bottom.max(top),
        ]));
    }
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

/// The box of `points`: left, bottom, right and top; none where there are none.
fn extent(points: &[Point]) -> Option<[Fixed; 4]> {
    points.iter().fold(None, |held, point| {
        Some(walk::joined(
            held,
            [point.across, point.down, point.across, point.down],
        ))
    })
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

/// Whether the insides of the boxes `first` and `second` may overlap: neither ends before the
/// other starts.
fn touches(first: [Fixed; 4], second: [Fixed; 4]) -> bool {
    let [left, bottom, right, top] = first;
    let [other_left, other_bottom, other_right, other_top] = second;
    right > other_left && other_right > left && top > other_bottom && other_top > bottom
}
