//! Signed-area coverage (ISO 32000-2:2020, 8.5.3.3).
//!
//! A pixel's value is the same whichever area around it is gathered, so bands do not show.

use crate::render::OPERATION;
use crate::render::fixed::{Fixed, quotient, saturated};
use crate::render::geometry::{Area, Point};
use crate::render::job::Job;
use crate::render::plane::{Plane, Spans};
use crate::{ErrorCode, PdfError};

/// The area of a whole pixel in a cell, once per winding.
const FULL: u32 = 0x2_0000;

/// Work of an edge, beside the rows and columns it touches.
const EDGE_WORK: u64 = 8;

/// The most cells between runs of a row that keep them apart.
const GAP: usize = 32;

/// The most runs of a row; past them it is taken whole.
const RUNS: usize = 16;

/// The fraction bits of a slope.
const SLOPE: u32 = 48;

/// What is inside a path.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Rule {
    /// An odd winding number.
    EvenOdd,
    /// A winding number other than 0.
    NonZero,
}

/// The coverage of an area, as it is gathered.
pub(crate) struct Coverage {
    /// Counts the bytes a row of all cells would take, which those touched never pass by more
    /// than a row.
    _counted: Plane<i64>,
    /// The area.
    area: Area,
    /// Each row's runs of cells that edges touched, apart and in order.
    rows: Vec<Vec<Run>>,
}

/// Cells of a row that edges touched, from the column of the first: signed area per cell, as
/// differences along the row, wide enough for any winding the work limit lets a path reach.
type Run = (usize, Vec<i64>);

/// A mask row as it is made from runs of cells.
#[derive(Default)]
struct Scan {
    /// The columns between runs, filled alike.
    gaps: usize,
    /// The next column.
    place: usize,
    /// The cells read.
    scanned: usize,
    /// Where values stopped being 0, while they are not.
    since: Option<usize>,
    /// The signed area so far.
    sum: i64,
    /// The share of the column at hand.
    value: u8,
}

/// An edge, pointing down, in 24.8.
struct Edge {
    /// Where it ends.
    bottom: i32,
    /// Its end across.
    last: i64,
    /// Its upper end, unrounded.
    origin: Point,
    /// Across per down, with [`SLOPE`] fraction bits; none: upright.
    slope: Option<i128>,
    /// Its start across.
    start: i64,
    /// Where it starts.
    top: i32,
}

impl Coverage {
    /// The area.
    pub(crate) const fn area(&self) -> Area {
        self.area
    }

    /// Adds the edge `from` to `to`.
    pub(crate) fn edge(&mut self, job: &Job<'_>, from: Point, to: Point) -> Result<(), PdfError> {
        job.spend(EDGE_WORK)?;
        if self.idle(job) {
            return Ok(());
        }
        let (low, high, sign) = if from.down < to.down {
            (from, to, 1_i32)
        } else {
            (to, from, -1_i32)
        };
        let (top, bottom) = (low.down.pixel(), high.down.pixel());
        let first = (top >> 8_u32).max(self.area.top);
        let end = (bottom.saturating_add(255) >> 8_u32).min(self.area.bottom);
        if top == bottom || first >= end {
            return Ok(());
        }
        let [left, right] =
            [self.area.left, self.area.right].map(|column| i64::from(column).saturating_mul(256));
        let (mut start, mut last) = (low.across.subpixel(), high.across.subpixel());
        if start.min(last) >= right {
            return Ok(());
        }
        // Left of the area only the winding counts.
        let beside = start.max(last) <= left;
        if beside {
            (start, last) = (left, left);
        }
        let rows = u64::try_from(end.saturating_sub(first)).unwrap_or(0);
        let columns = start.clamp(left, right).abs_diff(last.clamp(left, right)) >> 8_u32;
        // A row's cells are found among its runs: two units a row.
        job.spend(rows.saturating_mul(2).saturating_add(columns))?;
        let across = i128::from(high.across.bits()).saturating_sub(i128::from(low.across.bits()));
        let down = i128::from(high.down.bits()).saturating_sub(i128::from(low.down.bits()));
        let crosses = top >> 8_u32 != bottom.saturating_sub(1) >> 8_u32;
        let slope = if crosses && !beside && across != 0 {
            quotient(across.saturating_mul(1_i128 << SLOPE), down)
        } else {
            None
        };
        let edge = Edge {
            bottom,
            last,
            origin: low,
            slope,
            start,
            top,
        };
        let mut upper = first.saturating_mul(256).max(top);
        let mut upper_across = edge.across(upper);
        for row in first..end {
            let lower = row.saturating_add(1).saturating_mul(256).min(bottom);
            let lower_across = edge.across(lower);
            let drop = lower.saturating_sub(upper).saturating_mul(sign);
            let ends = [
                upper_across.min(lower_across).saturating_sub(left),
                upper_across.max(lower_across).saturating_sub(left),
            ];
            let (column, cells, moved) = self.touch(row, ends)?;
            let shift = i64::try_from(column).unwrap_or(0).saturating_mul(256);
            deposit(cells, ends.map(|place| place.saturating_sub(shift)), drop);
            // What runs were made, grown or moved for it.
            job.spend(moved)?;
            (upper, upper_across) = (lower, lower_across);
        }
        Ok(())
    }

    /// The covered share of each pixel by `rule`, 255 being all.
    pub(crate) fn finish(self, job: &Job<'_>, rule: Rule) -> Result<Plane<u8>, PdfError> {
        self.mask(job, rule, self.area)
    }

    /// Whether nothing is to be gathered: measuring, or no pixels.
    pub(crate) const fn idle(&self, job: &Job<'_>) -> bool {
        job.planes.measuring() || self.area.is_empty()
    }

    /// The covered share by `rule` of each pixel of `area` that is in this one's.
    pub(crate) fn mask(
        &self,
        job: &Job<'_>,
        rule: Rule,
        area: Area,
    ) -> Result<Plane<u8>, PdfError> {
        let within = area.intersect(self.area);
        let mut mask = Plane::new(&job.planes, within, 0_u8)?;
        // Its rows' runs count beside it.
        mask.counts(Spans::bytes(within.width()));
        if job.planes.measuring() || within.is_empty() {
            return Ok(mask);
        }
        let skipped = usize::try_from(within.left.saturating_sub(self.area.left)).unwrap_or(0);
        let width = within.width();
        let mut spans = Spans::default();
        spans.reserve(within.height(), width)?;
        for row in within.top..within.bottom {
            let runs = usize::try_from(row.saturating_sub(self.area.top))
                .ok()
                .and_then(|index| self.rows.get(index))
                .map_or(&[][..], Vec::as_slice);
            let out = mask.row_mut(row);
            let mut state = Scan::default();
            for &(first, ref cells) in runs {
                // Cells before the mask's columns only add up.
                let ahead = skipped.saturating_sub(first).min(cells.len());
                let (before, rest) = cells.split_at_checked(ahead).unwrap_or_default();
                let start = first
                    .saturating_add(ahead)
                    .saturating_sub(skipped)
                    .min(width);
                state.gap(out, start);
                if !before.is_empty() {
                    let sum = before
                        .iter()
                        .fold(state.sum, |total, delta| total.wrapping_add(*delta));
                    state.change(sum, rule, &mut spans);
                    state.scanned = state.scanned.saturating_add(before.len());
                }
                let slots = out.get_mut(start..).unwrap_or_default();
                let taken = rest.len().min(slots.len());
                for (offset, (slot, delta)) in slots.iter_mut().zip(rest).enumerate() {
                    if *delta != 0_i64 {
                        state.place = start.saturating_add(offset);
                        state.change(state.sum.wrapping_add(*delta), rule, &mut spans);
                    }
                    *slot = state.value;
                }
                state.place = start.saturating_add(taken);
                state.scanned = state.scanned.saturating_add(taken);
            }
            state.gap(out, width);
            if let Some(since) = state.since {
                spans.push((since, width));
            }
            spans.end_row(within.left);
            let gaps = u64::try_from(state.gaps).unwrap_or(u64::MAX) >> 4_u32;
            job.spend(
                u64::try_from(state.scanned)
                    .unwrap_or(u64::MAX)
                    .saturating_add(gaps)
                    .saturating_add(1),
            )?;
        }
        mask.spans = spans;
        Ok(mask)
    }

    /// Whether `points` are all on one side outside the area widened by `margin`, so what
    /// joins the first to the last inside their hull counts as the line between them.
    pub(crate) fn misses(&self, points: &[Point], margin: Fixed) -> bool {
        let [left, right, top, bottom] = [
            self.area.left,
            self.area.right,
            self.area.top,
            self.area.bottom,
        ]
        .map(|side| Fixed::from_i64(i64::from(side)));
        let (low, high) = (left.minus(margin), right.plus(margin));
        let (above, below) = (top.minus(margin), bottom.plus(margin));
        points.iter().all(|point| point.across <= low)
            || points.iter().all(|point| point.across >= high)
            || points.iter().all(|point| point.down <= above)
            || points.iter().all(|point| point.down >= below)
    }

    /// Nothing covered in `area`.
    pub(crate) fn new(job: &Job<'_>, area: Area) -> Result<Self, PdfError> {
        let counted = Plane::counted(&job.planes, area)?;
        let mut rows = Vec::new();
        if !job.planes.measuring() {
            rows.try_reserve_exact(area.height())
                .map_err(|_error| job.limit())?;
            rows.resize_with(area.height(), Vec::new);
        }
        Ok(Self {
            area,
            _counted: counted,
            rows,
        })
    }

    /// The cells of `row` that a piece of an edge between `ends` touches, at least, and the
    /// column of the first: a run of them, which joins those near.
    fn touch(&mut self, row: i32, ends: [i64; 2]) -> Result<(usize, &mut [i64], u64), PdfError> {
        let width = self.area.width();
        let [from, until] = ends;
        let column = |at: i64| usize::try_from(at.max(0) >> 8_u32).unwrap_or(usize::MAX);
        // A cell's share reaches the next one too.
        let (low, high) = if until <= 0 {
            (0, 2.min(width))
        } else {
            (
                column(from).min(width),
                column(until).saturating_add(2).min(width),
            )
        };
        let index = usize::try_from(row.saturating_sub(self.area.top)).unwrap_or(usize::MAX);
        let Some(runs) = self.rows.get_mut(index) else {
            return Ok((0, &mut [], 0));
        };
        let end_of = |run: &Run| run.0.saturating_add(run.1.len());
        // Most often one run holds the row, and the cells.
        if let [ref only] = **runs
            && only.0 <= low
            && high <= end_of(only)
        {
            return Ok(runs
                .get_mut(0)
                .map_or((0, &mut [][..], 0), |run| (run.0, run.1.as_mut_slice(), 0)));
        }
        let mut first = runs.partition_point(|run| end_of(run).saturating_add(GAP) < low);
        let near = runs.get(first..).unwrap_or_default();
        let mut last =
            first.saturating_add(near.partition_point(|run| run.0 <= high.saturating_add(GAP)));
        let within = |run: &Run| run.0 <= low && high <= end_of(run);
        if last == first.saturating_add(1) && runs.get(first).is_some_and(within) {
            let found = runs
                .get_mut(first)
                .map_or((0, &mut [][..], 0), |run| (run.0, run.1.as_mut_slice(), 0));
            return Ok(found);
        }
        let joined = runs.get(first..last).unwrap_or_default();
        let lowest = joined.first().map_or(low, |run| run.0.min(low));
        let highest = joined.last().map_or(high, |run| end_of(run).max(high));
        let length = highest.saturating_sub(lowest);
        // Grown, it takes as much again about what it needs between its neighbours, so that
        // growing is seldom; past a quarter of the row, or past the most runs, it takes the row
        // whole.
        let crowded = first == last && runs.len() >= RUNS;
        let (start, stop) = if crowded || length.saturating_mul(4) > width {
            (first, last) = (0, runs.len());
            (0, width)
        } else if first == last {
            (low, high)
        } else {
            let slack = length.max(8) >> 1_u32;
            let before = first
                .checked_sub(1)
                .and_then(|at| runs.get(at))
                .map_or(0, end_of);
            let after = runs.get(last).map_or(width, |run| run.0);
            (
                lowest.saturating_sub(slack).max(before),
                highest.saturating_add(slack).min(after),
            )
        };
        let size = stop.saturating_sub(start);
        // What is copied, at a sixteenth a cell: the cells of the runs it joins, or of the run it
        // grows past its room; and the runs after it in the row's list.
        let mut moved = 0_usize;
        let mut shifted = 0_usize;
        if last == first.saturating_add(1)
            && let Some(&mut (begins, ref mut cells)) = runs.get_mut(first)
            && begins == start
        {
            if size > cells.capacity() {
                moved = cells.len();
            }
            cells
                .try_reserve_exact(size.saturating_sub(cells.len()))
                .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))?;
            cells.resize(size, 0);
        } else {
            let mut grown = Vec::new();
            grown
                .try_reserve_exact(size)
                .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))?;
            grown.resize(size, 0);
            for (begins, cells) in runs.drain(first..last) {
                moved = moved.saturating_add(cells.len());
                let at = begins.saturating_sub(start);
                if let Some(slots) = grown.get_mut(at..at.saturating_add(cells.len())) {
                    slots.copy_from_slice(&cells);
                }
            }
            runs.insert(first.min(runs.len()), (start, grown));
            if runs.len() > 1 {
                shifted = runs.len().saturating_sub(first);
            }
        }
        let copied = u64::try_from((moved >> 4_u32).saturating_add(shifted)).unwrap_or(u64::MAX);
        Ok(runs.get_mut(first).map_or((0, &mut [][..], copied), |run| {
            (run.0, run.1.as_mut_slice(), copied)
        }))
    }
}

impl Scan {
    /// The sum turned to `sum` at the column at hand, giving its share by `rule`.
    fn change(&mut self, sum: i64, rule: Rule, spans: &mut Spans) {
        self.sum = sum;
        let next = share(sum, rule);
        match (self.value == 0, next == 0) {
            (true, false) => self.since = Some(self.place),
            (false, true) => {
                if let Some(since) = self.since.take() {
                    spans.push((since, self.place));
                }
            }
            (true, true) | (false, false) => {}
        }
        self.value = next;
    }

    /// The columns up to `until` take the share at hand, as nothing touches them.
    fn gap(&mut self, out: &mut [u8], until: usize) {
        if until > self.place {
            if self.value != 0 {
                out.get_mut(self.place..until)
                    .unwrap_or_default()
                    .fill(self.value);
            }
            self.gaps = self.gaps.saturating_add(until.saturating_sub(self.place));
            self.place = until;
        }
    }
}

impl Edge {
    /// Where it is across at `down`, from its unrounded ends whatever the area.
    fn across(&self, down: i32) -> i64 {
        if down <= self.top {
            return self.start;
        }
        if down >= self.bottom {
            return self.last;
        }
        let Some(slope) = self.slope else {
            return self.start;
        };
        let past = (i128::from(down) << 24_u32).saturating_sub(i128::from(self.origin.down.bits()));
        let moved = slope
            .saturating_mul(past)
            .saturating_add(1_i128 << SLOPE.saturating_sub(1))
            >> SLOPE;
        let found = i128::from(self.origin.across.bits()).saturating_add(moved);
        Fixed::from_bits(saturated(found))
            .subpixel()
            .clamp(self.start.min(self.last), self.start.max(self.last))
    }
}

/// Adds the piece of an edge within a row: `drop` tall, between `ends` in 24.8 from the first
/// cell.
fn deposit(cells: &mut [i64], ends: [i64; 2], drop: i32) {
    let [from, until] = ends;
    if until <= 0 {
        split(cells, 0, drop, 0);
        return;
    }
    let mut at = from.max(0);
    let Ok(mut column) = usize::try_from(at >> 8_u32) else {
        return;
    };
    let wide = until.saturating_sub(from);
    let mut given = 0_i32;
    if from < 0 {
        given = part(drop, at.saturating_sub(from), wide);
        split(cells, 0, given, 0);
    }
    while column < cells.len() {
        let start = at >> 8_u32 << 8_u32;
        let stop = until.min(start.saturating_add(256));
        let total = if stop == until {
            drop
        } else {
            part(drop, stop.saturating_sub(from), wide)
        };
        let ends_in = at
            .saturating_add(stop)
            .saturating_sub(start.saturating_mul(2));
        let share = total.saturating_sub(given);
        split(
            cells,
            column,
            share,
            i32::try_from(ends_in).unwrap_or(0_i32),
        );
        if stop == until {
            break;
        }
        (at, given) = (stop, total);
        column = column.saturating_add(1);
    }
}

/// How much of `drop` lies in the first `past` of `wide`, rounded to the nearest.
fn part(drop: i32, past: i64, wide: i64) -> i32 {
    let found = i64::from(drop.unsigned_abs())
        .saturating_mul(past)
        .saturating_add(wide >> 1_u32)
        .checked_div(wide)
        .and_then(|found| i32::try_from(found).ok())
        .unwrap_or(0_i32);
    if drop < 0 {
        found.saturating_neg()
    } else {
        found
    }
}

/// The share a pixel of the signed area `sum` has by `rule`, 255 being all.
fn share(sum: i64, rule: Rule) -> u8 {
    let area = sum.unsigned_abs();
    let full = u64::from(FULL);
    let covered = match rule {
        Rule::EvenOdd => {
            let cycle = area & 0x3_FFFF;
            cycle.min(full.saturating_mul(2).saturating_sub(cycle))
        }
        Rule::NonZero => area.min(full),
    };
    u8::try_from(covered.saturating_mul(255).saturating_add(0x1_0000) >> 17_u32).unwrap_or(u8::MAX)
}

/// Adds `drop` at `inset`, twice the mean distance from the left of cell `column`.
fn split(cells: &mut [i64], column: usize, drop: i32, inset: i32) {
    let (height, mean) = (i64::from(drop), i64::from(inset));
    let after = height.wrapping_mul(mean);
    if let Some(cell) = cells.get_mut(column) {
        *cell = cell.wrapping_add(height.wrapping_mul(512).wrapping_sub(after));
    }
    if let Some(cell) = cells.get_mut(column.saturating_add(1)) {
        *cell = cell.wrapping_add(after);
    }
}
