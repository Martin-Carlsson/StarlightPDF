//! Shadings, `sh` and shading patterns (ISO 32000-2:2020, 8.7.4).

mod gradient;
mod mesh;

use alloc::rc::Rc;

use crate::PdfError;
use crate::overlay::Object;
use crate::render::blend;
use crate::render::colour::{self, Intent, Space};
use crate::render::data::{self, Resolved};
use crate::render::fixed::{self, Fixed, quotient, saturated};
use crate::render::function::Function;
use crate::render::geometry::{Area, Matrix, Point};
use crate::render::interpret::{Run, paint, rectangle, state};
use crate::render::job::Job;
use crate::render::plane::Plane;
use crate::render::surface::{Clip, Composite, Group, Paint, Sample, Shape};

/// Half a pixel.
const HALF: Fixed = Fixed::from_bits(0x8000_0000);

/// The entries of a ramp.
const RAMP: usize = 4096;

/// The last entry of a ramp.
const RAMP_LAST: i64 = 4095;

/// Work of a ramp made.
const RAMP_WORK: u64 = 256;

/// The message for a shading that cannot be used.
const UNUSABLE: &str = "a shading cannot be used";

/// What the kinds of shading share.
struct Common<'shading> {
    /// The pixels it may paint.
    area: Area,
    /// The intent.
    intent: Intent,
    /// The map from its space to pixels.
    matrix: Matrix,
    /// Its dictionary or stream.
    owner: &'shading Resolved,
    /// Its colour space.
    space: Rc<Space>,
}

/// A shading within its box, over its background.
struct Framed {
    /// The background, which the box clips too (8.7.4.3).
    background: Option<[u16; 3]>,
    /// The box's share of each pixel; none: all.
    boxed: Option<Plane<u8>>,
    /// The shading.
    inner: Box<dyn Paint>,
}

/// The colours of one variable, worked out as they are asked for.
struct Ramp {
    /// The colours.
    colours: Vec<Option<[u16; 3]>>,
    /// The variable at either end.
    ends: [Fixed; 2],
    /// What gives the values.
    function: Rc<Function>,
    /// The intent.
    intent: Intent,
    /// The space.
    space: Rc<Space>,
    /// The values of one colour.
    values: Vec<Fixed>,
}

/// A point that follows the pixel centres along a row.
pub(crate) struct Steps {
    /// The point.
    at: Point,
    /// Its way per column.
    step: Point,
}

impl Common<'_> {
    /// Its function of `inputs`, which gives a value per component; none without `Function`.
    fn function(&self, job: &Job<'_>, inputs: usize) -> Result<Option<Rc<Function>>, PdfError> {
        let value = data::entry(job, self.owner.value(), b"Function")?;
        if matches!(*value.value(), Object::Null) {
            return Ok(None);
        }
        let function = Function::read(job, &value)?;
        if function.inputs() != inputs || function.outputs() != self.space.components() {
            return Err(job.objects.malformed());
        }
        // Counted while the shading draws.
        job.lend(&function, function.bytes())?;
        Ok(Some(function))
    }
}

impl Paint for Framed {
    fn row(
        &mut self,
        job: &Job<'_>,
        row: i32,
        left: i32,
        out: &mut [Sample],
    ) -> Result<(), PdfError> {
        self.inner.row(job, row, left, out)?;
        if self.background.is_none() && self.boxed.is_none() {
            return Ok(());
        }
        for (column, sample) in (left..).zip(out.iter_mut()) {
            if let Some([red, green, blue]) = self.background {
                let (above, below) = (sample.shape, u16::MAX.saturating_sub(sample.shape));
                let part = |over, under| {
                    fixed::unit(over, above).saturating_add(fixed::unit(under, below))
                };
                let [over_red, over_green, over_blue] = sample.color;
                *sample = Sample {
                    color: [
                        part(over_red, red),
                        part(over_green, green),
                        part(over_blue, blue),
                    ],
                    opacity: u16::MAX,
                    shape: u16::MAX,
                };
            }
            if let Some(boxed) = self.boxed.as_ref() {
                let index = usize::try_from(column.saturating_sub(boxed.area.left)).ok();
                let share = index
                    .and_then(|place| boxed.row(row).get(place))
                    .copied()
                    .unwrap_or_default();
                sample.shape = fixed::unit(sample.shape, u16::from(share).wrapping_mul(0x0101));
            }
        }
        Ok(())
    }
}

impl Ramp {
    /// The colour `part` of the way along.
    fn colour(&mut self, job: &Job<'_>, part: Fixed) -> Result<[u16; 3], PdfError> {
        let clamped = part.clamp(Fixed::default(), Fixed::from_i64(1));
        let step = clamped
            .bits()
            .saturating_mul(RAMP_LAST)
            .saturating_add(HALF.bits())
            >> 32_u32;
        let index = usize::try_from(step).unwrap_or(0);
        if let Some(found) = self.colours.get(index).copied().flatten() {
            return Ok(found);
        }
        let [from, to] = self.ends.map(|end| i128::from(end.bits()));
        let way = to.saturating_sub(from).saturating_mul(i128::from(step));
        let moved = quotient(way, i128::from(RAMP_LAST)).unwrap_or_default();
        let input = Fixed::from_bits(saturated(from.saturating_add(moved)));
        self.function.evaluate(job, &[input], &mut self.values)?;
        let found = self.space.rgb(job, &self.values, self.intent)?;
        if let Some(slot) = self.colours.get_mut(index) {
            *slot = Some(found);
        }
        Ok(found)
    }

    /// The colours of `function` from one of `ends` to the other.
    fn new(
        job: &Job<'_>,
        common: &Common<'_>,
        function: Rc<Function>,
        ends: [Fixed; 2],
    ) -> Result<Self, PdfError> {
        job.spend(RAMP_WORK)?;
        Ok(Self {
            colours: vec![None; RAMP],
            ends,
            values: vec![Fixed::default(); function.outputs()],
            function,
            intent: common.intent,
            space: Rc::clone(&common.space),
        })
    }
}

impl Steps {
    /// The point of this column; it moves to the next.
    pub(crate) const fn advance(&mut self) -> Point {
        let found = self.at;
        self.at = Point {
            across: found.across.plus(self.step.across),
            down: found.down.plus(self.step.down),
        };
        found
    }

    /// The points `lookup` gives for the centres of row `row` from column `left`: the same
    /// whatever column a span starts at.
    pub(crate) fn new(lookup: &Matrix, row: i32, left: i32) -> Self {
        let centre = |pixel: i32| Fixed::from_i64(i64::from(pixel)).plus(HALF);
        let [across, down, _, _, _, _] = lookup.terms;
        Self {
            at: lookup.apply(Point {
                across: centre(left),
                down: centre(row),
            }),
            step: Point { across, down },
        }
    }
}

/// The background of a shading in `common` (8.7.4.3); none without one.
fn background(job: &Job<'_>, common: &Common<'_>) -> Result<Option<[u16; 3]>, PdfError> {
    let items = data::array(
        job,
        common.owner.value(),
        b"Background",
        common.space.components(),
    )?;
    if items.is_empty() {
        return Ok(None);
    }
    if items.len() != common.space.components() {
        return Err(job.objects.malformed());
    }
    let values = items
        .iter()
        .map(|item| Fixed::from_object(item.value()).ok_or_else(|| job.objects.malformed()))
        .collect::<Result<Vec<_>, _>>()?;
    common.space.rgb(job, &values, common.intent).map(Some)
}

/// Paints the clip with `shading`, for `sh`.
pub(crate) fn operator(run: &mut Run<'_, '_>, shading: &Resolved) -> Result<(), PdfError> {
    let job = run.job;
    if run.locked_color {
        return job.warn("8.7.3.3", "sh where colour may not be given");
    }
    let how = paint::composite(run, paint::Which::Fill);
    let shape = Shape {
        area: run.surface.area().intersect(how.clip.area),
        cover: None,
    };
    if !job.planes.measuring() && shape.area.is_empty() {
        return Ok(());
    }
    let matrix = run.state.matrix;
    let result = painted(run, shading, (matrix, false), shape, &how);
    let _painted: Option<()> = job.tolerate(result, "8.7.4.3", UNUSABLE)?;
    Ok(())
}

/// Paints `shape` with `shading` under `matrix`, over its background where it is a pattern's.
fn painted(
    run: &mut Run<'_, '_>,
    shading: &Resolved,
    (matrix, patterned): (Matrix, bool),
    shape: Shape<'_>,
    how: &Composite,
) -> Result<(), PdfError> {
    let job = run.job;
    let owner = shading.value();
    let kind = match *data::entry(job, owner, b"ShadingType")?.value() {
        Object::Integer(kind @ 1..=7) => kind,
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => return Err(job.objects.malformed()),
    };
    let family = data::entry(job, owner, b"ColorSpace")?;
    let space = colour::resolve(job, &mut run.caches.spaces, &run.scope, &family)?;
    if !space.marks() || matrix.inverse().is_none() {
        return Ok(());
    }
    let mut common = Common {
        area: shape
            .area
            .intersect(how.clip.area)
            .intersect(run.surface.area()),
        intent: run.state.intent,
        matrix,
        owner: shading,
        space,
    };
    let behind = if patterned {
        background(job, &common)?
    } else {
        None
    };
    let boxed = if let Some(bounds) = data::numbers(job, owner, b"BBox")? {
        common.area = common.area.intersect(matrix.bounds(bounds));
        Some(rectangle(run, &matrix, bounds, common.area)?)
    } else {
        None
    };
    let inner: Box<dyn Paint> = if kind < 4 {
        gradient::read(job, &common, kind)?
    } else {
        mesh::read(job, &mut run.caches.streams, &common, kind)?
    };
    let visited = Shape {
        // A background fills the shape, within the box where there is one.
        area: if behind.is_some() && boxed.is_none() {
            shape.area
        } else {
            common.area
        },
        cover: shape.cover,
    };
    let mut framed = Framed {
        background: behind,
        boxed,
        inner,
    };
    run.surface.draw(job, visited, &mut framed, how)
}

/// Paints `shape` with the shading pattern `pattern`, as `which` paints (8.7.4.2).
pub(crate) fn pattern(
    run: &mut Run<'_, '_>,
    pattern: &Resolved,
    shape: Shape<'_>,
    (how, which): (&Composite, paint::Which),
) -> Result<(), PdfError> {
    let job = run.job;
    let result = shaded(run, pattern, shape, (how, which));
    let _painted: Option<()> = job.tolerate(result, "8.7.4.2", UNUSABLE)?;
    Ok(())
}

/// Reads the pattern and paints with its shading, as `which` paints.
fn shaded(
    run: &mut Run<'_, '_>,
    pattern: &Resolved,
    shape: Shape<'_>,
    (how, which): (&Composite, paint::Which),
) -> Result<(), PdfError> {
    let job = run.job;
    let owner = pattern.value();
    let parent = run
        .state
        .parent
        .clone()
        .ok_or_else(|| job.objects.malformed())?;
    let matrix = data::matrix(job, owner, b"Matrix")?.then(&parent.matrix);
    let shading = data::entry(job, owner, b"Shading")?;
    let parameters = data::entry(job, owner, b"ExtGState")?;
    let saved = Box::new(run.state.clone());
    let result = if matches!(*parameters.value(), Object::Null) {
        run.state.intent = parent.intent;
        painted(run, &shading, (matrix, true), shape, how)
    } else {
        // What ExtGState leaves out, and a soft mask there is drawn with, is the parent's.
        parent.hand_down(&mut run.state);
        let inner = run.active.split_off(parent.active.min(run.active.len()));
        let depth = run.depth;
        // Counted by its stack: see job::NESTING.
        run.depth = depth.saturating_add(1);
        let drawn = stated(run, (&shading, &parameters, matrix), shape, (how, which));
        run.depth = depth;
        run.active.extend(inner);
        drawn
    };
    run.state = *saved;
    result
}

/// Paints with a pattern's `parameters` in force, their alpha `which` paints: inside, they join
/// its shading to a group, which joins as `how` says (11.6.7).
fn stated(
    run: &mut Run<'_, '_>,
    (shading, parameters, matrix): (&Resolved, &Resolved, Matrix),
    shape: Shape<'_>,
    (how, which): (&Composite, paint::Which),
) -> Result<(), PdfError> {
    let job = run.job;
    let state = &mut run.state;
    (state.alpha_is_shape, state.blend) = (false, blend::Mode::Normal);
    (state.fill_alpha, state.stroke_alpha) = (u16::MAX, u16::MAX);
    (state.soft, state.soft_source) = (None, None);
    state.matrix = matrix;
    // Set from what is held and counted, not copied.
    state::applied(run, parameters)?;
    let inner = paint::composite(run, which);
    if inner.alpha == u16::MAX && inner.blend == blend::Mode::Normal && inner.soft.is_none() {
        return painted(run, shading, (matrix, true), shape, how);
    }
    let area = shape
        .area
        .intersect(how.clip.area)
        .intersect(run.surface.area());
    let within = Composite {
        clip: Clip { area, mask: None },
        ..inner
    };
    let group = Group {
        area,
        isolated: false,
        knockout: false,
    };
    run.surface.begin(job, group)?;
    let drawn = painted(run, shading, (matrix, true), shape, &within);
    paint::ended(run, drawn, how)
}
