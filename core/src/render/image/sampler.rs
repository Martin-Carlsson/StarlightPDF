//! Where an image's samples land on the pixels of a band (ISO 32000-2:2020, 8.9.4).
//!
//! Coordinates are exact numerators over the map's determinant, so pixels do not depend on bands
//! and abutting images leave no seam.

use alloc::collections::BTreeSet;

use crate::PdfError;
use crate::render::fixed::{self, Fixed};
use crate::render::function::room;
use crate::render::geometry::{Area, Matrix, Point};
use crate::render::image::samples::Cell;
use crate::render::job::Job;
use crate::render::math;
use crate::render::plane::Plane;

/// The fewest texels across counted a byte at a time.
const BITS_AT_ONCE: u32 = 4;

/// The bits a determinant keeps, so that times a side it fits.
const KEPT: u32 = 94;

/// Two to the 32.
const ONE: i128 = 0x1_0000_0000;

/// Work of adding to a block's sum, beside its texels.
const SUM_WORK: u64 = 2;

/// Work of a line's visit to a block of rows: taking it from the waiting and putting it back.
const VISIT_WORK: u64 = 2;

/// A coordinate in the image times the determinant, linear in the pixel.
#[derive(Clone, Copy)]
struct Axis {
    /// Its change per column.
    across: i128,
    /// Its change per row.
    down: i128,
    /// Its value at the centre of pixel 0, 0.
    start: i128,
}

/// One pixel's worth of a block of texels, in bytes.
#[derive(Clone, Copy, Default)]
struct Mark {
    /// The colour and the opacity.
    pixel: [u8; 4],
    /// The shape.
    shape: u8,
}

/// An image's unit square on the pixels.
pub(super) struct Placed {
    /// The axes, across and down the image.
    axes: [Axis; 2],
    /// The pixels it may touch.
    bounds: Area,
    /// The square's sides, in pixels.
    sides: [Fixed; 2],
    /// The determinant.
    whole: i128,
}

/// A block's number and what is left over it, also a step between two.
#[derive(Clone, Copy)]
struct Place {
    /// The number.
    index: i64,
    /// The rest, over the scale's divisor.
    part: i128,
}

/// How an axis falls into the blocks of texels that are averaged.
struct Scale {
    /// The blocks.
    blocks: i64,
    /// The determinant times the span.
    divisor: i128,
    /// The texels.
    size: i128,
    /// The texels of a block.
    span: u32,
    /// The change per pixel of a walk.
    step: Place,
}

/// The texels of a block, summed.
#[derive(Clone, Copy, Default)]
struct Sum {
    /// The colour times the weight.
    color: [u64; 3],
    /// The texels there.
    count: u64,
    /// The shape.
    shape: u64,
    /// The shape times the opacity.
    weight: u64,
}

/// What one pass writes.
pub(super) struct Written<'plane> {
    /// Opacity is shape (11.6.4.3).
    pub alpha_is_shape: bool,
    /// The shape.
    pub cover: &'plane mut Plane<u8>,
    /// Whether the pass is the first, which writes; a later one multiplies.
    pub first: bool,
    /// The colour and the opacity; none for a stencil.
    pub pixels: Option<&'plane mut Plane<[u8; 4]>>,
}

/// The pixels left of one row, walked so that the image's rows rise.
#[derive(Clone, Copy)]
struct Walk {
    /// The pixels left.
    left: i32,
    /// The block across.
    over: Place,
    /// The block down.
    rank: Place,
    /// The row.
    row: i32,
    /// The column.
    x_at: i32,
}

/// The pixels of an image of one size, taken in the order of its rows.
pub(super) struct Walks {
    /// Columns per step.
    direction: i32,
    /// The blocks across.
    horizontal: Scale,
    /// The rows.
    lines: Vec<Walk>,
    /// The blocks down.
    vertical: Scale,
    /// The block each row waits for, and the row's walk.
    waiting: BTreeSet<(i64, usize)>,
}

/// What gives texels, row after row.
pub(super) trait Texels {
    /// The texels `from` to `to` of the row read last, as how many there are of each cell, when
    /// that is quicker than one by one; `None` otherwise.
    ///
    /// # Errors
    ///
    /// Whatever converting them fails with.
    fn bits(
        &mut self,
        job: &Job<'_>,
        span: (u32, u32),
    ) -> Result<Option<[(Cell, u64); 2]>, PdfError>;

    /// The texel at `column` of the row read last; `None` past the data.
    ///
    /// # Errors
    ///
    /// Whatever converting it fails with.
    fn cell(&mut self, job: &Job<'_>, column: u32) -> Result<Option<Cell>, PdfError>;

    /// Reads row `row`, not above the last.
    ///
    /// # Errors
    ///
    /// Whatever reading it fails with.
    fn read(&mut self, job: &Job<'_>, row: u32) -> Result<(), PdfError>;

    /// The work of reading a texel to average.
    fn work(&self) -> u64;
}

impl Axis {
    /// Its value at the pixel in `column` and `row`.
    fn at(self, column: i32, row: i32) -> i128 {
        self.start
            .saturating_add(self.across.saturating_mul(i128::from(column)))
            .saturating_add(self.down.saturating_mul(i128::from(row)))
    }

    /// The columns of `row`, within `from` to `to`, where it is from 0 to below `whole`.
    fn inside(self, row: i32, (from, to): (i32, i32), whole: i128) -> (i32, i32) {
        let value = self.at(0, row);
        if self.across == 0 {
            let within = (0..whole).contains(&value);
            return if within { (from, to) } else { (from, from) };
        }
        let least = value.saturating_neg();
        let most = whole.saturating_sub(value);
        let floor = |above: i128, below: i128| above.checked_div_euclid(below).unwrap_or(0);
        let (start, end) = if self.across > 0 {
            let ceiling = |above: i128| floor(above.saturating_neg(), self.across).saturating_neg();
            (ceiling(least), ceiling(most))
        } else {
            let after = |above: i128| floor(above, self.across.saturating_neg()).saturating_add(1);
            (after(most.saturating_neg()), after(value))
        };
        let held = |bound: i128| {
            i32::try_from(bound.clamp(i128::from(from), i128::from(to))).unwrap_or(from)
        };
        (held(start), held(end))
    }
}

impl Mark {
    /// `cell` in bytes, its opacity joined to its shape where that is what it is.
    fn of(cell: Cell, alpha_is_shape: bool) -> Self {
        let (shape, opacity) = if alpha_is_shape {
            (fixed::unit(cell.shape, cell.opacity), u16::MAX)
        } else {
            (cell.shape, cell.opacity)
        };
        let [red, green, blue] = cell.color.map(fixed::byte);
        Self {
            pixel: [red, green, blue, fixed::byte(opacity)],
            shape: fixed::byte(shape),
        }
    }
}

impl Place {
    /// This and `step` after it, over `divisor`.
    const fn plus(self, step: Self, divisor: i128) -> Self {
        let index = self.index.saturating_add(step.index);
        let part = self.part.saturating_add(step.part);
        if part >= divisor {
            Self {
                index: index.saturating_add(1),
                part: part.saturating_sub(divisor),
            }
        } else {
            Self { index, part }
        }
    }
}

impl Placed {
    /// The pixels it may touch.
    pub(super) const fn bounds(&self) -> Area {
        self.bounds
    }

    /// The unit square under `matrix`; `None` where that is singular. An upright one thinner
    /// than a pixel keeps one.
    pub(super) fn new(matrix: &Matrix) -> Option<Self> {
        let determinant = |terms: [Fixed; 6]| {
            let [aa, bb, cc, dd, _, _] = terms.map(|term| i128::from(term.bits()));
            bb.saturating_mul(cc).saturating_sub(aa.saturating_mul(dd))
        };
        if determinant(matrix.terms) == 0 {
            return None;
        }
        let terms = widened(matrix.terms);
        let mut whole = determinant(terms);
        let [aa, bb, cc, dd, ee, ff] = terms.map(|term| i128::from(term.bits()));
        // From the first sample's corner, the square's (0, 1), to the centre of pixel 0, 0.
        let half = ONE >> 1_u32;
        let right = half.saturating_sub(cc.saturating_add(ee));
        let below = half.saturating_sub(dd.saturating_add(ff));
        let mut axes = [
            Axis {
                across: dd.saturating_neg().saturating_mul(ONE),
                down: cc.saturating_mul(ONE),
                start: cc
                    .saturating_mul(below)
                    .saturating_sub(dd.saturating_mul(right)),
            },
            Axis {
                across: bb.saturating_neg().saturating_mul(ONE),
                down: aa.saturating_mul(ONE),
                start: aa
                    .saturating_mul(below)
                    .saturating_sub(bb.saturating_mul(right)),
            },
        ];
        if whole < 0 {
            whole = whole.saturating_neg();
            axes = axes.map(|axis| Axis {
                across: axis.across.saturating_neg(),
                down: axis.down.saturating_neg(),
                start: axis.start.saturating_neg(),
            });
        }
        let used = i128::BITS.saturating_sub(whole.leading_zeros());
        let shift = used.saturating_sub(KEPT);
        for axis in &mut axes {
            axis.across = axis.across.wrapping_shr(shift);
            axis.down = axis.down.wrapping_shr(shift);
            axis.start = axis.start.wrapping_shr(shift);
            // The left and top edges are the image's whichever way it turns.
            if axis.across < 0 || (axis.across == 0 && axis.down < 0) {
                axis.start = axis.start.saturating_sub(1);
            }
        }
        let side = |across, down| {
            math::hypotenuse(Point {
                across: Fixed::from_bits(fixed::saturated(across)),
                down: Fixed::from_bits(fixed::saturated(down)),
            })
        };
        let (nought, one) = (Fixed::default(), Fixed::from_i64(1));
        Some(Self {
            axes,
            bounds: Matrix { terms }.bounds([nought, nought, one, one]),
            sides: [side(aa, bb), side(cc, dd)],
            whole: whole.wrapping_shr(shift),
        })
    }

    /// Whether the image's rows only rise as the page's do, so a decoder can stay open.
    pub(super) const fn rising(&self) -> bool {
        let [_, down] = self.axes;
        down.across == 0 && down.down > 0
    }

    /// The columns of `row` within `area` whose centres are in the square.
    fn span(&self, row: i32, area: Area) -> (i32, i32) {
        let [first, second] = self.axes;
        let span = first.inside(row, (area.left, area.right), self.whole);
        second.inside(row, span, self.whole)
    }

    /// The pixels its sides take, up to `size`: what is worth decoding.
    pub(super) fn target(&self, size: [u32; 2]) -> [u32; 2] {
        let [across, down] = self.sides;
        let [width, height] = size;
        let pixels = |side: Fixed, most: u32| {
            let rounded = side.bits().saturating_add(0xFFFF_FFFF);
            let count = Fixed::from_bits(rounded)
                .floor()
                .clamp(1, i64::from(most.max(1)));
            u32::try_from(count).unwrap_or(most)
        };
        [pixels(across, width), pixels(down, height)]
    }
}

impl Scale {
    /// The block of `place`, within the image.
    fn block(&self, place: Place) -> i64 {
        place.index.min(self.blocks.saturating_sub(1)).max(0)
    }

    /// How `size` texels fall on `axis`, over `whole`, stepping by `direction`.
    fn new(axis: Axis, whole: i128, size: u32, direction: i32) -> Self {
        let texels = i128::from(size);
        // Averaged by the whole texels of a pixel's footprint: never past it, so upright rows
        // land in blocks of their own.
        let reach = axis.across.saturating_abs().max(axis.down.saturating_abs());
        let footprint = reach.saturating_mul(texels).checked_div(whole).unwrap_or(1);
        let span = u32::try_from(footprint.clamp(1, texels.max(1))).unwrap_or(1);
        let divisor = whole.saturating_mul(i128::from(span));
        let mut scale = Self {
            blocks: i64::from(size.div_ceil(span)),
            divisor,
            size: texels,
            span,
            step: Place { index: 0, part: 0 },
        };
        scale.step = scale.place(axis.across.saturating_mul(i128::from(direction)));
        scale
    }

    /// Where the coordinate `value` falls.
    fn place(&self, value: i128) -> Place {
        let scaled = value.saturating_mul(self.size);
        let index = scaled.checked_div_euclid(self.divisor).unwrap_or(0);
        Place {
            index: fixed::saturated(index),
            part: scaled.checked_rem_euclid(self.divisor).unwrap_or(0),
        }
    }

    /// The texels of block `block`.
    fn texels(&self, block: i64) -> (u32, u32) {
        let span = i64::from(self.span);
        let from = block.saturating_mul(span);
        let held = |at: i64| u32::try_from(i128::from(at).min(self.size)).unwrap_or(0);
        (held(from), held(from.saturating_add(span)))
    }
}

impl Sum {
    /// Adds `cell`, `times` times.
    fn add(&mut self, cell: Cell, times: u64) {
        let weight = u64::from(fixed::unit(cell.shape, cell.opacity));
        for (total, part) in self.color.iter_mut().zip(cell.color) {
            let product = u64::from(part).saturating_mul(weight).saturating_mul(times);
            *total = total.saturating_add(product);
        }
        self.count = self.count.saturating_add(times);
        let shape = u64::from(cell.shape).saturating_mul(times);
        self.shape = self.shape.saturating_add(shape);
        self.weight = self.weight.saturating_add(weight.saturating_mul(times));
    }

    /// The mean, colour weighed by alpha.
    fn mean(&self) -> Cell {
        let part = |total: u64, over: u64| {
            let rounded = total.saturating_add(over >> 1_u32).checked_div(over);
            u16::try_from(rounded.unwrap_or(0)).unwrap_or(u16::MAX)
        };
        Cell {
            color: self.color.map(|total| part(total, self.weight)),
            opacity: part(self.weight.saturating_mul(0xFFFF), self.shape),
            shape: part(self.shape, self.count),
        }
    }
}

impl Walks {
    /// Writes to `target` what is left of `walk` in block `block`, `marks` being its blocks
    /// from `low`.
    fn emit(
        &self,
        walk: &mut Walk,
        block: i64,
        (marks, low): (&[Mark], i64),
        target: &mut Written<'_>,
    ) {
        let origin = target.cover.area.left;
        let covers = target.cover.row_mut(walk.row);
        let mut pixels = target.pixels.as_mut().map(|plane| plane.row_mut(walk.row));
        while walk.left > 0_i32 && self.vertical.block(walk.rank) == block {
            let from = self.horizontal.block(walk.over).saturating_sub(low);
            let mark = usize::try_from(from)
                .ok()
                .and_then(|index| marks.get(index))
                .copied()
                .unwrap_or_default();
            let at = usize::try_from(walk.x_at.saturating_sub(origin)).unwrap_or(usize::MAX);
            if let Some(cover) = covers.get_mut(at) {
                *cover = if target.first {
                    mark.shape
                } else {
                    product(*cover, mark.shape)
                };
            }
            if let Some(pixel) = pixels.as_mut().and_then(|row| row.get_mut(at)) {
                *pixel = if target.first {
                    mark.pixel
                } else {
                    let ([red, green, blue, own], [_, _, _, other]) = (*pixel, mark.pixel);
                    [red, green, blue, product(own, other)]
                };
            }
            self.step(walk);
        }
    }

    /// The first row of the image that a pixel needs.
    pub(super) fn first(&self) -> u32 {
        self.waiting
            .first()
            .map_or(0, |waits| self.vertical.texels(waits.0).0)
    }

    /// Fills `marks` with the blocks `low` to `high` across of block `block` down.
    fn gather(
        &self,
        job: &Job<'_>,
        texels: &mut dyn Texels,
        (block, low, high): (i64, i64, i64),
        (marks, sums): (&mut Vec<Mark>, &mut Vec<Sum>),
        alpha_is_shape: bool,
    ) -> Result<(), PdfError> {
        let count = usize::try_from(high.saturating_sub(low)).map_err(|_error| job.limit())?;
        marks.clear();
        if count > marks.capacity() {
            *marks = room(count)?;
        }
        let (top, bottom) = self.vertical.texels(block);
        let (left, _) = self.horizontal.texels(low);
        let (_, right) = self.horizontal.texels(high.saturating_sub(1));
        let read = u64::from(right.saturating_sub(left)).saturating_add(1);
        if self.horizontal.span == 1 && self.vertical.span == 1 {
            job.spend(read.saturating_mul(texels.work()))?;
            texels.read(job, top)?;
            for column in left..right {
                let mark = texels
                    .cell(job, column)?
                    .map(|cell| Mark::of(cell, alpha_is_shape));
                marks.push(mark.unwrap_or_default());
            }
            return Ok(());
        }
        sums.clear();
        if count > sums.capacity() {
            *sums = room(count)?;
        }
        sums.resize(count, Sum::default());
        for row in top..bottom {
            // Beside the texels, each block's sum, which only narrow blocks do not cover.
            let narrow = self.horizontal.span < BITS_AT_ONCE;
            let sums_work = u64::try_from(count)
                .unwrap_or(u64::MAX)
                .saturating_mul(SUM_WORK)
                .saturating_mul(u64::from(narrow));
            // An eighth more for walking each texel into its block's sum.
            let walked = read
                .saturating_mul(texels.work())
                .saturating_add(read >> 3_u32);
            job.spend(walked.saturating_add(sums_work))?;
            texels.read(job, row)?;
            for (sum, across) in sums.iter_mut().zip(low..high) {
                let (from, to) = self.horizontal.texels(across);
                // Counted by bytes only where that saves more than it costs.
                if to.saturating_sub(from) >= BITS_AT_ONCE
                    && let Some(counted) = texels.bits(job, (from, to))?
                {
                    for (cell, times) in counted {
                        sum.add(cell, times);
                    }
                    continue;
                }
                for column in from..to {
                    if let Some(cell) = texels.cell(job, column)? {
                        sum.add(cell, 1);
                    }
                }
            }
        }
        marks.extend(sums.iter().map(|sum| Mark::of(sum.mean(), alpha_is_shape)));
        Ok(())
    }

    /// The walks over `area` of an image `placed` of `size`; `None` where it has no pixel.
    pub(super) fn new(
        job: &Job<'_>,
        placed: &Placed,
        area: Area,
        size: [u32; 2],
    ) -> Result<Option<Self>, PdfError> {
        let ([first, second], [width, height]) = (placed.axes, size);
        let direction = if second.across < 0 { -1_i32 } else { 1_i32 };
        let mut found = Self {
            direction,
            horizontal: Scale::new(first, placed.whole, width, direction),
            vertical: Scale::new(second, placed.whole, height, direction),
            waiting: BTreeSet::new(),
            lines: room(area.height())?,
        };
        for row in area.top..area.bottom {
            job.spend(1)?;
            let (from, to) = placed.span(row, area);
            if from >= to {
                continue;
            }
            let start = if direction > 0_i32 {
                from
            } else {
                to.saturating_sub(1)
            };
            let walk = Walk {
                left: to.saturating_sub(from),
                over: found.horizontal.place(first.at(start, row)),
                rank: found.vertical.place(second.at(start, row)),
                row,
                x_at: start,
            };
            let waits = (found.vertical.block(walk.rank), found.lines.len());
            let _new: bool = found.waiting.insert(waits);
            found.lines.push(walk);
        }
        Ok((!found.lines.is_empty()).then_some(found))
    }

    /// The blocks across that `walk` takes in block `block` down, the last excluded.
    fn reached(&self, mut walk: Walk, block: i64) -> (i64, i64) {
        let start = self.horizontal.block(walk.over);
        let mut end = start;
        while walk.left > 0_i32 && self.vertical.block(walk.rank) == block {
            end = self.horizontal.block(walk.over);
            self.step(&mut walk);
        }
        (start.min(end), start.max(end).saturating_add(1))
    }

    /// Writes every pixel to `target` from `texels`, whose rows it reads once, in order.
    pub(super) fn sample(
        mut self,
        job: &Job<'_>,
        texels: &mut dyn Texels,
        target: &mut Written<'_>,
    ) -> Result<(), PdfError> {
        let (mut due, mut marks, mut sums) = (Vec::new(), Vec::new(), Vec::new());
        while let Some(&(block, _)) = self.waiting.first() {
            due.clear();
            while self.waiting.first().is_some_and(|waits| waits.0 == block) {
                due.extend(self.waiting.pop_first().map(|waits| waits.1));
            }
            let visits = u64::try_from(due.len()).unwrap_or(u64::MAX);
            job.spend(visits.saturating_mul(VISIT_WORK))?;
            let (mut low, mut high) = (i64::MAX, 0);
            for &index in &due {
                if let Some(walk) = self.lines.get(index).copied() {
                    let (from, to) = self.reached(walk, block);
                    low = low.min(from);
                    high = high.max(to);
                }
            }
            let wanted = (block, low, high);
            let held = (&mut marks, &mut sums);
            self.gather(job, texels, wanted, held, target.alpha_is_shape)?;
            for &index in &due {
                let Some(mut walk) = self.lines.get(index).copied() else {
                    continue;
                };
                self.emit(&mut walk, block, (&marks, low), target);
                if walk.left > 0_i32 {
                    let _new: bool = self.waiting.insert((self.vertical.block(walk.rank), index));
                }
                if let Some(slot) = self.lines.get_mut(index) {
                    *slot = walk;
                }
            }
        }
        Ok(())
    }

    /// The next pixel of `walk`.
    const fn step(&self, walk: &mut Walk) {
        walk.left = walk.left.saturating_sub(1);
        walk.x_at = walk.x_at.saturating_add(self.direction);
        walk.over = walk
            .over
            .plus(self.horizontal.step, self.horizontal.divisor);
        walk.rank = walk.rank.plus(self.vertical.step, self.vertical.divisor);
    }

    /// The texels, as columns and rows, that no pixel of `area` reaches past: the map is linear,
    /// so its corners reach furthest.
    pub(super) fn window(&self, placed: &Placed, area: Area) -> Area {
        let (end, last) = (area.right.saturating_sub(1), area.bottom.saturating_sub(1));
        let range = |axis: Axis, scale: &Scale| {
            let corners = [
                axis.at(area.left, area.top),
                axis.at(end, area.top),
                axis.at(area.left, last),
                axis.at(end, last),
            ];
            let block = |value: i128| scale.texels(scale.block(scale.place(value)));
            let (from, _) = block(corners.into_iter().fold(i128::MAX, i128::min));
            let (_, to) = block(corners.into_iter().fold(i128::MIN, i128::max));
            [from, to].map(|texel| i32::try_from(texel).unwrap_or(i32::MAX))
        };
        let [first, second] = placed.axes;
        let [left, right] = range(first, &self.horizontal);
        let [top, bottom] = range(second, &self.vertical);
        Area {
            bottom,
            left,
            right,
            top,
        }
    }
}

/// The product of two shares in bytes.
fn product(left: u8, right: u8) -> u8 {
    let wide = |share: u8| u16::from(share).wrapping_mul(0x0101);
    fixed::byte(fixed::unit(wide(left), wide(right)))
}

/// A side less than a pixel, from `start`, as the whole pixel its middle is in.
fn pixel_wide(side: Fixed, start: Fixed) -> (Fixed, Fixed) {
    let one = Fixed::from_i64(1);
    let backward = side < Fixed::default();
    let length = if backward {
        Fixed::default().minus(side)
    } else {
        side
    };
    if length >= one {
        return (side, start);
    }
    let middle = start.plus(Fixed::from_bits(side.bits() >> 1_u32));
    let first = Fixed::from_i64(middle.floor());
    if backward {
        (Fixed::default().minus(one), first.plus(one))
    } else {
        (one, first)
    }
}

/// `terms`, an upright side thinner than a pixel made one.
fn widened(terms: [Fixed; 6]) -> [Fixed; 6] {
    let [aa, bb, cc, dd, ee, ff] = terms;
    let none = Fixed::default();
    if bb == none && cc == none {
        let ((wide, left), (high, top)) = (pixel_wide(aa, ee), pixel_wide(dd, ff));
        [wide, none, none, high, left, top]
    } else if aa == none && dd == none {
        let ((wide, left), (high, top)) = (pixel_wide(cc, ee), pixel_wide(bb, ff));
        [none, high, wide, none, left, top]
    } else {
        terms
    }
}
