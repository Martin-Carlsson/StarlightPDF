//! Shadings of types 4 to 7, meshes of triangles and patches (ISO 32000-2:2020, 8.7.4.5.5 to
//! 8.7.4.5.8).
//!
//! A vertex is a list: across and down in 1/256 pixel, then
//! its values with 32 fraction bits.

use core::mem::swap;

use crate::overlay::Object;
use crate::pull::{Buffered, Pull};
use crate::render::OPERATION;
use crate::render::colour::Converter;
use crate::render::data::{self, Streams};
use crate::render::fixed::{Fixed, byte, quotient, saturated};
use crate::render::function::VALUES;
use crate::render::geometry::{Area, Matrix, Point};
use crate::render::job::Job;
use crate::render::plane::Plane;
use crate::render::shading::{Common, Ramp};
use crate::render::surface::{Paint, Sample};
use crate::{ErrorCode, PdfError};

/// The points of each edge of a patch, by its place in the stream: at v 0 and 1, then at u 0 and 1.
const EDGES: [[usize; 4]; 4] = [[0, 11, 10, 9], [3, 4, 5, 6], [0, 1, 2, 3], [9, 8, 7, 6]];

/// Where each point of a patch is in its net, across u and across v, in the stream's order.
const NET: [(usize, usize); 16] = [
    (0, 0),
    (0, 1),
    (0, 2),
    (0, 3),
    (1, 3),
    (2, 3),
    (3, 3),
    (3, 2),
    (3, 1),
    (3, 0),
    (2, 0),
    (1, 0),
    (1, 1),
    (1, 2),
    (2, 2),
    (2, 1),
];

/// The inner lines of a patch's net: two along u, then two along v.
const NET_LINES: [[usize; 4]; 4] = [
    [1, 12, 15, 8],
    [2, 13, 14, 7],
    [11, 12, 13, 4],
    [10, 15, 14, 5],
];

/// Fraction bits of a vertex's place.
const PLACE_BITS: u32 = 8;

/// The most lines across a patch.
const PIECES: i64 = 64;

/// Four pixels of a patch's net make a line, for its curves.
const PIECE_BITS: u32 = 34;

/// Up to this many lines a pixel makes one, for its colours, which are bilinear.
const PIECES_SMALL: i64 = 12;

/// The points on a patch's edges.
const ROUND: usize = 12;

/// The most numbers held of a row of a lattice mesh.
const ROW: usize = 0x8_0000;

/// Work of a pixel of a mesh: its values are mixed, then converted.
const PIXEL_WORK: u64 = 3;

/// Work of a step of geometry for each number it takes, as it multiplies and divides widely.
const STEP_WORK: u64 = 3;

/// Work of a triangle, even one that is not seen.
const TRIANGLE_WORK: u64 = 8;

/// Work of a number decoded.
const VALUE_WORK: u64 = 4;

/// The message for damaged data.
const DAMAGED: &str = "a mesh shading's data is damaged";

/// Data read by bits.
struct Bits<'data> {
    /// The bits read and not taken.
    count: u32,
    /// The data.
    data: Buffered<Box<dyn Pull + 'data>>,
    /// Their value, in its lowest bits.
    held: u64,
}

/// What triangles are painted into.
struct Canvas {
    /// Converts.
    converter: Converter,
    /// Where a row enters a triangle.
    entering: Vec<i64>,
    /// Where it leaves.
    leaving: Vec<i64>,
    /// The colours; the fourth: painted.
    pixels: Plane<[u8; 4]>,
    /// With a function, its colours.
    ramp: Option<Ramp>,
    /// The values of one pixel.
    values: Vec<Fixed>,
}

/// What the dictionary says of the data.
struct Layout {
    /// The bits of a component.
    component: u32,
    /// The bits of a coordinate.
    coordinate: u32,
    /// What each number decodes to: across, down, then each value.
    decode: Vec<[Fixed; 2]>,
    /// The bits of a flag; none in type 5.
    flag: u32,
    /// The map to pixels.
    matrix: Matrix,
    /// The vertices of a row in type 5.
    per_row: usize,
}

/// A patch.
struct Patch {
    /// The colours at its corners, in the stream's order, one after the other.
    colours: Vec<i64>,
    /// Its points in pixels, 32 fraction bits, in the stream's order.
    points: [[i64; 2]; 16],
}

/// The painted pixels of a mesh.
struct Picture {
    /// The colours; the fourth: painted.
    pixels: Plane<[u8; 4]>,
}

/// A place along a patch: a numerator over a denominator.
#[derive(Clone, Copy)]
struct Place {
    /// The numerator.
    above: i64,
    /// The denominator.
    below: i64,
}

/// The reading of one mesh.
struct Reader<'run, 'data> {
    /// The data.
    bits: Bits<'data>,
    /// Where it is painted.
    canvas: &'run mut Canvas,
    /// The job.
    job: &'run Job<'data>,
    /// The layout.
    layout: &'run Layout,
}

impl Bits<'_> {
    /// Passes over the rest of the byte.
    const fn align(&mut self) {
        self.count = 0;
    }

    /// Whether the data ends here, at the start of an item.
    fn ended(&mut self) -> Result<bool, PdfError> {
        let Some(next) = self.data.byte()? else {
            return Ok(true);
        };
        (self.count, self.held) = (8, u64::from(next));
        Ok(false)
    }

    /// The next number of `width` bits, at most 32; none past the end.
    fn field(&mut self, width: u32) -> Result<Option<u32>, PdfError> {
        while self.count < width {
            let Some(next) = self.data.byte()? else {
                return Ok(None);
            };
            self.held = self.held.wrapping_shl(8) | u64::from(next);
            self.count = self.count.saturating_add(8);
        }
        self.count = self.count.saturating_sub(width);
        let mask = u64::from(u32::MAX >> 32_u32.saturating_sub(width));
        Ok(u32::try_from(self.held.wrapping_shr(self.count) & mask).ok())
    }
}

impl Canvas {
    /// Paints row `row` between where it enters and leaves.
    fn span(&mut self, job: &Job<'_>, row: i32) -> Result<(), PdfError> {
        if self.leaving.first() < self.entering.first() {
            swap(&mut self.entering, &mut self.leaving);
        }
        let (Some((&start, starting)), Some((&end, ending))) =
            (self.entering.split_first(), self.leaving.split_first())
        else {
            return Ok(());
        };
        let area = self.pixels.area;
        let (first, past) = (first_centre(start), first_centre(end));
        let left = first.max(i64::from(area.left));
        let right = past.min(i64::from(area.right));
        let Ok(count) = u64::try_from(right.saturating_sub(left)) else {
            return Ok(());
        };
        if count == 0 {
            return Ok(());
        }
        // Each pixel interpolates every value.
        let values = u64::try_from(self.values.len() >> 1_u32).unwrap_or(u64::MAX);
        job.spend(count.saturating_mul(PIXEL_WORK.saturating_add(values)))?;
        let width = i128::from(end).saturating_sub(i128::from(start));
        let share = |length: i128| quotient(length.saturating_mul(1 << 32_u32), width).unwrap_or(0);
        let rise = share(1 << PLACE_BITS);
        // From the span's own first column, so that a clip changes no pixel.
        let head = share(i128::from(centre(first)).saturating_sub(i128::from(start)));
        let mut along =
            head.saturating_add(rise.saturating_mul(i128::from(left.saturating_sub(first))));
        let from = usize::try_from(left.saturating_sub(i64::from(area.left))).unwrap_or(usize::MAX);
        let taken = usize::try_from(count).unwrap_or(usize::MAX);
        let pixels = self.pixels.row_mut(row).get_mut(from..).unwrap_or_default();
        for pixel in pixels.iter_mut().take(taken) {
            let weight = along.clamp(0, 1 << 32_u32);
            along = along.saturating_add(rise);
            for ((value, &low), &high) in self.values.iter_mut().zip(starting).zip(ending) {
                *value = Fixed::from_bits(between(low, high, weight));
            }
            let color = match self.ramp.as_mut() {
                Some(ramp) => {
                    let place = self.values.first().copied().unwrap_or_default();
                    let [from_value, to_value] = ramp.ends;
                    let way = place.minus(from_value).over(to_value.minus(from_value));
                    ramp.colour(job, way.unwrap_or_default())?
                }
                None => self.converter.rgb(job, &self.values)?,
            };
            let [red, green, blue] = color.map(byte);
            *pixel = [red, green, blue, u8::MAX];
        }
        Ok(())
    }

    /// Paints the triangle `corners` at the pixel centres in it, its top and left sides included.
    fn triangle(&mut self, job: &Job<'_>, corners: [&[i64]; 3]) -> Result<(), PdfError> {
        job.spend(TRIANGLE_WORK)?;
        if job.planes.measuring() {
            return Ok(());
        }
        let [first, second, third] = corners;
        let (Some(one), Some(two), Some(three)) = (spot(first), spot(second), spot(third)) else {
            return Ok(());
        };
        let area = self.pixels.area;
        let ([one_across, one_down], [two_across, two_down]) = (one, two);
        let [three_across, three_down] = three;
        let top = first_centre(one_down.min(two_down).min(three_down)).max(i64::from(area.top));
        let bottom =
            first_centre(one_down.max(two_down).max(three_down)).min(i64::from(area.bottom));
        if top >= bottom
            || first_centre(one_across.min(two_across).min(three_across)) >= i64::from(area.right)
            || first_centre(one_across.max(two_across).max(three_across)) <= i64::from(area.left)
        {
            return Ok(());
        }
        let work = u64::try_from(first.len().saturating_add(8))
            .unwrap_or(u64::MAX)
            .saturating_mul(STEP_WORK);
        for row in top..bottom {
            job.spend(work)?;
            let down = centre(row);
            let mut found = 0_u8;
            for (start, end) in [(first, second), (second, third), (third, first)] {
                let out = if found == 0 {
                    &mut self.entering
                } else {
                    &mut self.leaving
                };
                if crossed(start, end, down, out) {
                    found = found.saturating_add(1);
                }
            }
            if found == 2
                && let Ok(line) = i32::try_from(row)
            {
                self.span(job, line)?;
            }
        }
        Ok(())
    }
}

impl Layout {
    /// The layout the dictionary of `common` gives type `kind`, with `values` per vertex.
    fn read(
        job: &Job<'_>,
        common: &Common<'_>,
        kind: i64,
        values: usize,
    ) -> Result<Self, PdfError> {
        let owner = common.owner.value();
        let broken = || job.objects.malformed();
        if !matches!(*owner, Object::Stream(..)) || values > VALUES {
            return Err(broken());
        }
        let whole = |key: &[u8]| -> Result<i64, PdfError> {
            Fixed::from_object(data::entry(job, owner, key)?.value())
                .map(Fixed::floor)
                .ok_or_else(broken)
        };
        let coordinate = u32::try_from(whole(b"BitsPerCoordinate")?)
            .ok()
            .filter(|found| matches!(*found, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32))
            .ok_or_else(broken)?;
        let component = u32::try_from(whole(b"BitsPerComponent")?)
            .ok()
            .filter(|found| matches!(*found, 1 | 2 | 4 | 8 | 12 | 16))
            .ok_or_else(broken)?;
        let (mut flag, mut per_row) = (0, 0);
        if kind == 5 {
            per_row = usize::try_from(whole(b"VerticesPerRow")?)
                .ok()
                .filter(|found| *found >= 2)
                .ok_or_else(broken)?;
            let held = per_row.saturating_mul(values.saturating_add(2));
            if held > ROW {
                return Err(job.capped());
            }
        } else {
            flag = u32::try_from(whole(b"BitsPerFlag")?)
                .ok()
                .filter(|found| matches!(*found, 2 | 4 | 8))
                .ok_or_else(broken)?;
        }
        let listed = data::entry(job, owner, b"Decode")?;
        let wanted = values.saturating_add(2);
        let items = data::items(listed.value()).ok_or_else(broken)?;
        let (pairs, odd) = items.as_chunks::<2>();
        if pairs.len() < wanted {
            return Err(broken());
        }
        if pairs.len() > wanted || !odd.is_empty() {
            job.warn("8.7.4.5", "a mesh's Decode has numbers past those needed")?;
        }
        let mut decode = Vec::with_capacity(wanted);
        for pair in pairs.iter().take(wanted) {
            let number = |item: &Object| {
                let found = data::resolve(job, item)?;
                Fixed::from_object(found.value()).ok_or_else(broken)
            };
            let [low, high] = pair.each_ref();
            decode.push([number(low)?, number(high)?]);
        }
        Ok(Self {
            component,
            coordinate,
            decode,
            flag,
            matrix: common.matrix,
            per_row,
        })
    }
}

impl Paint for Picture {
    fn row(
        &mut self,
        _job: &Job<'_>,
        row: i32,
        left: i32,
        out: &mut [Sample],
    ) -> Result<(), PdfError> {
        let area = self.pixels.area;
        let skipped = usize::try_from(area.left.saturating_sub(left)).unwrap_or(0);
        let from = usize::try_from(left.saturating_sub(area.left)).unwrap_or(0);
        let values = self.pixels.row(row).get(from..).unwrap_or_default();
        let targets = out.get_mut(skipped..).unwrap_or_default();
        for (sample, value) in targets.iter_mut().zip(values) {
            let [red, green, blue, shape] = value.map(|part| u16::from(part).wrapping_mul(0x0101));
            *sample = Sample {
                color: [red, green, blue],
                opacity: u16::MAX,
                shape,
            };
        }
        Ok(())
    }
}

impl Patch {
    /// Whether no pixel of `area` is within the hull of its points.
    fn misses(&self, area: Area) -> bool {
        let pixel = |bits: i64| bits >> 32_u32;
        let all = |test: &dyn Fn(i64, i64) -> bool| {
            self.points
                .iter()
                .all(|&[across, down]| test(pixel(across), pixel(down)))
        };
        all(&|across, _| across >= i64::from(area.right))
            || all(&|across, _| across < i64::from(area.left).saturating_sub(1))
            || all(&|_, down| down >= i64::from(area.bottom))
            || all(&|_, down| down < i64::from(area.top).saturating_sub(1))
    }

    /// The lines that follow the line of its net through `line`, the same from either end.
    fn pieces(&self, line: [usize; 4]) -> i64 {
        let points = line.map(|index| self.points.get(index).copied().unwrap_or_default());
        let mut length = 0_u64;
        for (from, to) in points.iter().zip(points.iter().skip(1)) {
            for (&start, &end) in from.iter().zip(to) {
                length = length.saturating_add(start.abs_diff(end));
            }
        }
        let lines_of = |bits: u32| {
            let rounded = length.saturating_add((1_u64 << bits).saturating_sub(1)) >> bits;
            i64::try_from(rounded).unwrap_or(PIECES)
        };
        lines_of(PIECE_BITS)
            .max(lines_of(32).min(PIECES_SMALL))
            .clamp(1, PIECES)
    }

    /// Takes the four points of a Coons patch that its edges imply (8.7.4.5.8).
    fn raise(&mut self) {
        let edge = self.points;
        for (corner, slot) in (0..ROUND)
            .step_by(3)
            .zip(self.points.iter_mut().skip(ROUND))
        {
            let at = |offset: usize| {
                let index = wrapped(corner.saturating_add(offset), ROUND);
                edge.get(index).copied().unwrap_or_default().map(i128::from)
            };
            let terms = [
                (-4_i128, at(0)),
                (6, at(1)),
                (6, at(11)),
                (-2, at(3)),
                (-2, at(9)),
                (3, at(4)),
                (3, at(8)),
                (-1, at(6)),
            ];
            let mut sums = [0_i128; 2];
            for (weight, point) in terms {
                for (sum, value) in sums.iter_mut().zip(point) {
                    *sum = sum.saturating_add(weight.saturating_mul(value));
                }
            }
            *slot = sums.map(|sum| saturated(quotient(sum, 9).unwrap_or_default()));
        }
    }

    /// Puts the vertex at `along` u and v into `out`.
    fn vertex(&self, along: [Place; 2], out: &mut Vec<i64>) {
        let [first, second] = along;
        let (weights, others) = (bernstein(first), bernstein(second));
        let mut sums = [0_i128; 2];
        for (&(column, line), &point) in NET.iter().zip(&self.points) {
            let weight = weights
                .get(column)
                .copied()
                .unwrap_or_default()
                .saturating_mul(others.get(line).copied().unwrap_or_default());
            if weight == 0 {
                continue;
            }
            for (sum, value) in sums.iter_mut().zip(point) {
                *sum = sum.saturating_add(i128::from(value).saturating_mul(weight));
            }
        }
        let cube = |place: Place| i128::from(place.below).saturating_pow(3);
        // One rounding, so that patches meet exactly at the edge they share.
        let scale = cube(first)
            .saturating_mul(cube(second))
            .saturating_mul(1 << 24_u32);
        out.extend(sums.map(|sum| saturated(quotient(sum, scale).unwrap_or_default())));
        let (across, down) = (i128::from(first.above), i128::from(second.above));
        let (wide, high) = (i128::from(first.below), i128::from(second.below));
        let (back, up) = (wide.saturating_sub(across), high.saturating_sub(down));
        let shares = [
            back.saturating_mul(up),
            back.saturating_mul(down),
            across.saturating_mul(down),
            across.saturating_mul(up),
        ];
        let total = wide.saturating_mul(high);
        let count = self.colours.len() >> 2_u32;
        for index in 0..count {
            let mut sum = 0_i128;
            let mut place = index;
            for share in shares {
                let value = self.colours.get(place).copied().unwrap_or_default();
                sum = sum.saturating_add(i128::from(value).saturating_mul(share));
                place = place.saturating_add(count);
            }
            out.push(saturated(quotient(sum, total).unwrap_or_default()));
        }
    }
}

impl Reader<'_, '_> {
    /// Triangles that share edges (8.7.4.5.5).
    fn free(&mut self) -> Result<(), PdfError> {
        let (mut first, mut second, mut third) = (Vec::new(), Vec::new(), Vec::new());
        let mut next = Vec::new();
        let mut started = false;
        while !self.bits.ended()? {
            let flag = self.number(self.layout.flag)?;
            if flag > 2 {
                return Err(self.job.objects.malformed());
            }
            self.vertex(&mut next)?;
            match flag {
                0 => {
                    swap(&mut first, &mut next);
                    let _second: u32 = self.number(self.layout.flag)?;
                    self.vertex(&mut second)?;
                    let _third: u32 = self.number(self.layout.flag)?;
                    self.vertex(&mut third)?;
                    started = true;
                }
                1 if started => {
                    swap(&mut first, &mut second);
                    swap(&mut second, &mut third);
                    swap(&mut third, &mut next);
                }
                2 if started => {
                    swap(&mut second, &mut third);
                    swap(&mut third, &mut next);
                }
                _ => return Err(self.job.objects.malformed()),
            }
            self.canvas.triangle(self.job, [&first, &second, &third])?;
        }
        Ok(())
    }

    /// Rows of vertices (8.7.4.5.6).
    fn lattice(&mut self) -> Result<(), PdfError> {
        let (job, per_row) = (self.job, self.layout.per_row);
        let stride = self.layout.decode.len();
        let (mut held, mut before, mut next) = (Vec::<i64>::new(), Vec::new(), Vec::new());
        // The row whole, counted again beside the mesh's data now held, so that growing it never
        // doubles past.
        let row = per_row.saturating_mul(stride);
        job.fits(u64::try_from(row.saturating_mul(size_of::<i64>())).unwrap_or(u64::MAX))?;
        held.try_reserve_exact(row)
            .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))?;
        let (mut column, mut later) = (0_usize, false);
        while !self.bits.ended()? {
            self.vertex(&mut next)?;
            let start = column.saturating_mul(stride);
            let end = start.saturating_add(stride);
            if later {
                let above = held.get(start..end).unwrap_or_default();
                if let Some(beside) = start
                    .checked_sub(stride)
                    .and_then(|from| held.get(from..start))
                {
                    self.canvas.triangle(job, [&before, above, beside])?;
                    self.canvas.triangle(job, [above, beside, &next])?;
                }
                before.clear();
                before.extend_from_slice(above);
                if let Some(slot) = held.get_mut(start..end) {
                    slot.copy_from_slice(&next);
                }
            } else {
                held.try_reserve(stride)
                    .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))?;
                held.extend_from_slice(&next);
            }
            column = column.saturating_add(1);
            if column >= per_row {
                (column, later) = (0, true);
            }
        }
        if column > 0 {
            return Err(job.objects.malformed());
        }
        Ok(())
    }

    /// The next number of `width` bits; past the end the data is damaged.
    fn number(&mut self, width: u32) -> Result<u32, PdfError> {
        self.job.spend(VALUE_WORK)?;
        self.bits
            .field(width)?
            .ok_or_else(|| self.job.objects.malformed())
    }

    /// Coons or tensor-product patches (8.7.4.5.7, 8.7.4.5.8).
    fn patches(&mut self, coons: bool) -> Result<(), PdfError> {
        let stride = self.layout.decode.len().saturating_sub(2);
        let mut patch = Patch {
            colours: vec![0; stride.saturating_mul(4)],
            points: [[0; 2]; 16],
        };
        let mut lines = [Vec::new(), Vec::new()];
        let mut started = false;
        while !self.bits.ended()? {
            let flag = usize::try_from(self.number(self.layout.flag)?).unwrap_or(usize::MAX);
            if flag > 3 || (flag > 0 && !started) {
                return Err(self.job.objects.malformed());
            }
            started = true;
            let (mut known, mut coloured) = (0_usize, 0_usize);
            if flag > 0 {
                let (edge, colours) = (patch.points, patch.colours.clone());
                // The edge it shares becomes its first.
                (known, coloured) = (4, 2);
                let from = flag.saturating_mul(3);
                for (offset, slot) in patch.points.iter_mut().take(known).enumerate() {
                    let index = wrapped(from.saturating_add(offset), ROUND);
                    *slot = edge.get(index).copied().unwrap_or_default();
                }
                for (offset, slot) in patch
                    .colours
                    .iter_mut()
                    .take(coloured.saturating_mul(stride))
                    .enumerate()
                {
                    let index = flag.saturating_mul(stride).saturating_add(offset);
                    let place = wrapped(index, colours.len());
                    *slot = colours.get(place).copied().unwrap_or_default();
                }
            }
            let given = if coons { ROUND } else { NET.len() };
            for slot in patch.points.iter_mut().take(given).skip(known) {
                let point = self.point()?;
                *slot = [point.across.bits(), point.down.bits()];
            }
            let fresh = patch
                .colours
                .iter_mut()
                .skip(coloured.saturating_mul(stride));
            for (slot, index) in fresh.zip((0..stride).cycle()) {
                *slot = self
                    .value(self.layout.component, index.saturating_add(2))?
                    .bits();
            }
            self.bits.align();
            if coons {
                patch.raise();
            }
            self.tessellated(&patch, &mut lines)?;
        }
        Ok(())
    }

    /// The next point, in pixels.
    fn point(&mut self) -> Result<Point, PdfError> {
        let width = self.layout.coordinate;
        let point = Point {
            across: self.value(width, 0)?,
            down: self.value(width, 1)?,
        };
        Ok(self.layout.matrix.apply(point))
    }

    /// Paints `patch` as triangles, larger v and then larger u on top (8.7.4.5.7).
    fn tessellated(&mut self, patch: &Patch, lines: &mut [Vec<i64>; 2]) -> Result<(), PdfError> {
        let job = self.job;
        job.spend(16)?;
        if patch.misses(self.canvas.pixels.area) {
            return Ok(());
        }
        let edges = EDGES.map(|line| patch.pieces(line));
        let [low, high, start, end] = edges;
        let [low_inner, high_inner, start_inner, end_inner] =
            NET_LINES.map(|line| patch.pieces(line));
        let wide = low.max(high).max(low_inner).max(high_inner);
        let tall = start.max(end).max(start_inner).max(end_inner);
        // An edge has its own places only, so a neighbour's are the same.
        let place = |at: i64, total: i64, own: i64| Place {
            above: if own == total {
                at
            } else {
                quotient(i128::from(at.saturating_mul(own)), i128::from(total)).map_or(0, saturated)
            },
            below: own,
        };
        let stride = self.layout.decode.len();
        let each = u64::try_from(stride.saturating_add(16))
            .unwrap_or(u64::MAX)
            .saturating_mul(STEP_WORK);
        let count = u64::try_from(wide.saturating_add(1)).unwrap_or(u64::MAX);
        let [above, below] = lines.each_mut();
        above.clear();
        for line in 0..=tall {
            job.spend(count.saturating_mul(each))?;
            below.clear();
            for column in 0..=wide {
                let own_wide = match line {
                    0 => low,
                    _ if line == tall => high,
                    _ => wide,
                };
                let own_tall = match column {
                    0 => start,
                    _ if column == wide => end,
                    _ => tall,
                };
                let along = [place(column, wide, own_wide), place(line, tall, own_tall)];
                patch.vertex(along, below);
            }
            let upper = above.chunks_exact(stride);
            let lower = below.chunks_exact(stride);
            let pairs = upper.clone().zip(upper.skip(1));
            for ((one, two), (three, four)) in pairs.zip(lower.clone().zip(lower.skip(1))) {
                self.canvas.triangle(job, [one, two, three])?;
                self.canvas.triangle(job, [two, three, four])?;
            }
            swap(above, below);
        }
        Ok(())
    }

    /// The next number, decoded as `index` of `Decode`.
    fn value(&mut self, width: u32, index: usize) -> Result<Fixed, PdfError> {
        let raw = self.number(width)?;
        let [low, high] = self.layout.decode.get(index).copied().unwrap_or_default();
        let (from, to) = (i128::from(low.bits()), i128::from(high.bits()));
        let way = to.saturating_sub(from).saturating_mul(i128::from(raw));
        let most = i128::from(u32::MAX >> 32_u32.saturating_sub(width));
        let moved = quotient(way, most).unwrap_or_default();
        Ok(Fixed::from_bits(saturated(from.saturating_add(moved))))
    }

    /// The next vertex, into `out`; each starts a byte.
    fn vertex(&mut self, out: &mut Vec<i64>) -> Result<(), PdfError> {
        out.clear();
        let point = self.point()?;
        out.extend([point.across.subpixel(), point.down.subpixel()]);
        for index in 2..self.layout.decode.len() {
            out.push(self.value(self.layout.component, index)?.bits());
        }
        self.bits.align();
        Ok(())
    }
}

/// The weights of a cubic at `place`, times its denominator cubed.
fn bernstein(place: Place) -> [i128; 4] {
    let along = i128::from(place.above);
    let back = i128::from(place.below).saturating_sub(along);
    [
        back.saturating_pow(3),
        back.saturating_pow(2)
            .saturating_mul(along)
            .saturating_mul(3),
        along
            .saturating_pow(2)
            .saturating_mul(back)
            .saturating_mul(3),
        along.saturating_pow(3),
    ]
}

/// The value `part` of the way from `low` to `high`.
fn between(low: i64, high: i64, part: i128) -> i64 {
    let way = i128::from(high).saturating_sub(i128::from(low));
    let moved = way.saturating_mul(part).saturating_add(1 << 31_u32) >> 32_u32;
    saturated(i128::from(low).saturating_add(moved))
}

/// The centre of pixel `pixel`, in 1/256 pixel.
const fn centre(pixel: i64) -> i64 {
    pixel.saturating_mul(1 << PLACE_BITS).saturating_add(128)
}

/// Puts where the edge from `one` to `other` crosses `down` into `out`: across, then the values;
/// whether it does.
fn crossed(one: &[i64], other: &[i64], down: i64, out: &mut Vec<i64>) -> bool {
    let (Some((&first, first_values)), Some((&second, second_values))) =
        (one.split_first_chunk::<2>(), other.split_first_chunk::<2>())
    else {
        return false;
    };
    let ([_, first_down], [_, second_down]) = (first, second);
    let ([from, top], starting, [to, bottom], ending) = if first_down <= second_down {
        (first, first_values, second, second_values)
    } else {
        (second, second_values, first, first_values)
    };
    if down < top || down >= bottom {
        return false;
    }
    let passed = i128::from(down).saturating_sub(i128::from(top));
    let tall = i128::from(bottom).saturating_sub(i128::from(top));
    let way = i128::from(to).saturating_sub(i128::from(from));
    let moved = quotient(passed.saturating_mul(way), tall).unwrap_or_default();
    let part = quotient(passed.saturating_mul(1 << 32_u32), tall).unwrap_or_default();
    out.clear();
    out.push(saturated(i128::from(from).saturating_add(moved)));
    let values = starting.iter().zip(ending);
    out.extend(values.map(|(&low, &high)| between(low, high, part)));
    true
}

/// The first pixel whose centre is not before `place`.
const fn first_centre(place: i64) -> i64 {
    place.saturating_add(127) >> PLACE_BITS
}

/// The mesh of type `kind`, 4 to 7, of `common`, painted over its area; in the pass without pixels
/// its data is read, not painted.
pub(super) fn read(
    job: &Job<'_>,
    streams: &mut Streams,
    common: &Common<'_>,
    kind: i64,
) -> Result<Box<dyn Paint>, PdfError> {
    let function = common.function(job, 1)?;
    let values = match function.as_ref() {
        Some(_) => 1,
        None => common.space.components(),
    };
    let layout = Layout::read(job, common, kind, values)?;
    let pixels = Plane::new(&job.planes, common.area, [0; 4])?;
    // The pass without pixels reads it whole too, so that each band is refunded its reading.
    if common.area.is_empty() && !job.planes.measuring() {
        return Ok(Box::new(Picture { pixels }));
    }
    let ends = layout.decode.get(2).copied().unwrap_or_default();
    let mut canvas = Canvas {
        converter: Converter::new(job, &common.space, common.intent)?,
        entering: Vec::new(),
        leaving: Vec::new(),
        pixels,
        ramp: function
            .map(|found| Ramp::new(job, common, found, ends))
            .transpose()?,
        values: vec![Fixed::default(); values],
    };
    // A lattice's row is held beside the data.
    let row = layout
        .per_row
        .saturating_mul(layout.decode.len())
        .saturating_mul(size_of::<i64>());
    let opened = data::kept(
        job,
        streams,
        common.owner,
        u64::try_from(row).unwrap_or(u64::MAX),
    );
    let result = opened.and_then(|stream| {
        let mut reader = Reader {
            bits: Bits {
                count: 0,
                data: Buffered::new(stream),
                held: 0,
            },
            canvas: &mut canvas,
            job,
            layout: &layout,
        };
        match kind {
            4 => reader.free(),
            5 => reader.lattice(),
            _ => reader.patches(kind == 6),
        }
    });
    let _kept: Option<()> = job.tolerate(result, "8.7.4.5", DAMAGED)?;
    Ok(Box::new(Picture {
        pixels: canvas.pixels,
    }))
}

/// Where `vertex` is.
const fn spot(vertex: &[i64]) -> Option<[i64; 2]> {
    vertex.first_chunk::<2>().copied()
}

/// `index` around a ring of `length`.
fn wrapped(index: usize, length: usize) -> usize {
    index.checked_rem(length).unwrap_or_default()
}
