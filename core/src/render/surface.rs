//! The surface: layers, groups, knockout and clips (ISO 32000-2:2020, 11.3, 11.4).
//!
//! A layer holds only its group's own pixels, so leaving a group removes no backdrop; blending
//! reads the backdrop off the layers below.

use alloc::rc::Rc;

use crate::render::geometry::Area;
use crate::render::job::Job;
use crate::render::plane::Plane;
use crate::render::{OPERATION, blend, fixed};
use crate::{ErrorCode, PdfError};

/// Work of a pixel composited by a blend mode other than `Normal`.
const BLEND_WORK: u64 = 4;

/// Work of a pixel composited.
const MARK_WORK: u64 = 1;

/// Work of starting a run of pixels to composite.
const RUN_WORK: u64 = 8;

/// Where painting is let through.
#[derive(Clone)]
pub(crate) struct Clip {
    /// The pixels.
    pub area: Area,
    /// The share of each, the product of every nested clip; none: all.
    pub mask: Option<Rc<Plane<u8>>>,
}

/// How a source joins what is painted.
pub(crate) struct Composite {
    /// The constant alpha.
    pub alpha: u16,
    /// Alpha and soft mask are shape, not opacity.
    pub alpha_is_shape: bool,
    /// The blend mode.
    pub blend: blend::Mode,
    /// The clip.
    pub clip: Clip,
    /// The soft mask.
    pub soft: Option<Soft>,
}

/// A transparency group to begin.
#[derive(Clone, Copy)]
pub(crate) struct Group {
    /// Its pixels.
    pub area: Area,
    /// It starts transparent.
    pub isolated: bool,
    /// Each object replaces those before it.
    pub knockout: bool,
}

/// A group being painted, or the page.
pub(crate) struct Layer {
    /// How far it grows to what is painted in it; none: it stays as begun.
    bound: Option<Area>,
    /// It starts transparent.
    isolated: bool,
    /// Each object replaces those before it.
    knockout: bool,
    /// Its pixels without the backdrop, the fourth its alpha.
    pixels: Plane<[u16; 4]>,
    /// Its shape, where a group around it knocks out.
    shape: Option<Plane<u16>>,
}

/// What joins one row of a layer.
struct Marks<'row> {
    /// The clip.
    clip: Track<'row>,
    /// The coverage.
    cover: Track<'row>,
    /// The first column.
    left: i32,
    /// The row.
    row: i32,
    /// The source.
    samples: &'row [Sample],
    /// The soft mask.
    soft: Track<'row>,
    /// What the layer started over; empty: nothing.
    under: &'row [[u16; 4]],
}

/// One pixel of a source.
#[derive(Clone, Copy, Default)]
pub(crate) struct Sample {
    /// The colour, not premultiplied.
    pub color: [u16; 3],
    /// The opacity.
    pub opacity: u16,
    /// The shape.
    pub shape: u16,
}

/// The pixels a source paints.
#[derive(Clone, Copy)]
pub(crate) struct Shape<'mask> {
    /// The pixels.
    pub area: Area,
    /// The share of each; none: all.
    pub cover: Option<&'mask Plane<u8>>,
}

/// A soft mask.
#[derive(Clone)]
pub(crate) struct Soft {
    /// Its value outside its plane.
    pub outside: u8,
    /// Its values.
    pub values: Rc<Plane<u8>>,
}

/// One colour everywhere.
pub(crate) struct Solid {
    /// The colour.
    pub color: [u16; 3],
}

/// Layers over one area.
pub(crate) struct Surface {
    /// The area.
    area: Area,
    /// The layers, the group being painted last.
    layers: Vec<Layer>,
    /// What every pass shares, which holds the area: the page's image, or a cell's grid.
    whole: Area,
}

/// One row of a mask.
#[derive(Clone, Copy)]
struct Track<'plane> {
    /// Its first column.
    left: i32,
    /// Its value elsewhere.
    outside: u8,
    /// Its values.
    values: &'plane [u8],
}

/// A source of colour, asked by row, rows rising within one draw.
pub(crate) trait Paint {
    /// Fills `out`, which starts unpainted, from column `left` of row `row`.
    ///
    /// # Errors
    ///
    /// Whatever making the colours fails with.
    fn row(
        &mut self,
        job: &Job<'_>,
        row: i32,
        left: i32,
        out: &mut [Sample],
    ) -> Result<(), PdfError>;
}

impl Paint for Layer {
    fn row(
        &mut self,
        _job: &Job<'_>,
        row: i32,
        left: i32,
        out: &mut [Sample],
    ) -> Result<(), PdfError> {
        let from = offset(self.pixels.area.left, left);
        let pixels = self.pixels.row(row).get(from..).unwrap_or_default();
        let shapes = self
            .shape
            .as_ref()
            .and_then(|plane| plane.row(row).get(from..))
            .unwrap_or_default();
        for (index, (sample, pixel)) in out.iter_mut().zip(pixels).enumerate() {
            let [_, _, _, alpha] = *pixel;
            let shape = shapes.get(index).copied().unwrap_or(u16::MAX);
            *sample = Sample {
                color: straight(*pixel),
                opacity: if shape == u16::MAX {
                    alpha
                } else {
                    blend::ratio(alpha, shape)
                },
                shape,
            };
        }
        Ok(())
    }
}

impl Paint for Solid {
    fn row(
        &mut self,
        _job: &Job<'_>,
        _row: i32,
        _left: i32,
        out: &mut [Sample],
    ) -> Result<(), PdfError> {
        out.fill(Sample {
            color: self.color,
            opacity: u16::MAX,
            shape: u16::MAX,
        });
        Ok(())
    }
}

impl Clip {
    /// This clip within `shape` too.
    pub(crate) fn within(&self, job: &Job<'_>, shape: Shape<'_>) -> Result<Self, PdfError> {
        let area = self.area.intersect(shape.area);
        let Some(cover) = shape.cover else {
            return Ok(Self {
                area,
                mask: self.mask.clone(),
            });
        };
        let within = area.intersect(cover.area);
        let mut mask = Plane::new(&job.planes, within, 0_u8)?;
        if !job.planes.measuring() {
            for row in within.top..within.bottom {
                job.spend(visited(within))?;
                let covered = track(Some(cover), row, 0);
                let held = track(self.mask.as_deref(), row, 0);
                // The values first: past the last the column may overflow.
                for (value, column) in mask.row_mut(row).iter_mut().zip(within.left..) {
                    *value = fixed::byte(fixed::unit(covered.at(column), held.at(column)));
                }
            }
        }
        Ok(Self {
            area: within,
            mask: Some(Rc::new(mask)),
        })
    }
}

impl Layer {
    /// Composites one row (11.3.3, 11.4.6).
    fn land(&mut self, marks: &Marks<'_>, how: &Composite) {
        let from = offset(self.pixels.area.left, marks.left);
        let shapes = self
            .shape
            .as_mut()
            .and_then(|plane| plane.row_mut(marks.row).get_mut(from..))
            .unwrap_or_default();
        let pixels = self.pixels.row_mut(marks.row).get_mut(from..);
        let painted = pixels.unwrap_or_default().iter_mut().zip(marks.samples);
        for ((index, column), (pixel, sample)) in (0_usize..).zip(marks.left..).zip(painted) {
            let covered = fixed::unit(marks.cover.at(column), marks.clip.at(column));
            let mut shape = fixed::unit(covered, sample.shape);
            let mut opacity = sample.opacity;
            let veil = fixed::unit(how.alpha, marks.soft.at(column));
            if how.alpha_is_shape {
                shape = fixed::unit(shape, veil);
            } else {
                opacity = fixed::unit(opacity, veil);
            }
            if shape == 0 {
                continue;
            }
            if let Some(union) = shapes.get_mut(index) {
                *union = fixed::unit(*union, u16::MAX.saturating_sub(shape)).saturating_add(shape);
            }
            let alpha = fixed::unit(shape, opacity);
            let replaced = if self.knockout { shape } else { alpha };
            if replaced == 0 {
                continue;
            }
            let color = if how.blend == blend::Mode::Normal {
                sample.color
            } else {
                let start = marks.under.get(index).copied().unwrap_or_default();
                let beneath = if self.knockout {
                    start
                } else {
                    let [_, _, _, own] = *pixel;
                    join(start, own, *pixel)
                };
                blended(how.blend, beneath, sample.color)
            };
            *pixel = join(*pixel, replaced, premultiplied(color, alpha));
        }
    }

    /// The box of what it painted: the pixels of any shape, else of any alpha.
    fn painted(&self, job: &Job<'_>) -> Result<Area, PdfError> {
        let whole = self.pixels.area;
        let mut found = Area {
            bottom: whole.top,
            left: whole.right,
            right: whole.left,
            top: whole.bottom,
        };
        if job.planes.measuring() {
            return Ok(whole);
        }
        for row in whole.top..whole.bottom {
            job.spend((visited(whole) >> 3_u32).saturating_add(1))?;
            let ends = self.shape.as_ref().map_or_else(
                || ends(self.pixels.row(row), |&[_, _, _, alpha]| alpha != 0),
                |plane| ends(plane.row(row), |shape| *shape != 0),
            );
            let Some((first, last)) = ends else {
                continue;
            };
            let column = |index: usize| {
                whole
                    .left
                    .saturating_add(i32::try_from(index).unwrap_or(i32::MAX))
            };
            found.top = found.top.min(row);
            found.bottom = row.saturating_add(1);
            found.left = found.left.min(column(first));
            found.right = found.right.max(column(last).saturating_add(1));
        }
        Ok(found)
    }
}

impl Surface {
    /// Its alpha as a soft mask, through `transfer` (11.5.2).
    pub(crate) fn alpha_mask(
        self,
        job: &Job<'_>,
        transfer: Option<&[u8; 256]>,
    ) -> Result<Plane<u8>, PdfError> {
        masked(&self, job, transfer, |[_, _, _, alpha]| alpha)
    }

    /// Its area.
    pub(crate) const fn area(&self) -> Area {
        self.area
    }

    /// Begins a group (11.4).
    pub(crate) fn begin(&mut self, job: &Job<'_>, group: Group) -> Result<(), PdfError> {
        self.reach(job, group.area)?;
        let parent = self.layers.last();
        let around = parent.map_or(self.area, |layer| layer.pixels.area);
        let area = group.area.intersect(around);
        let shaped = parent.is_some_and(|layer| layer.knockout || layer.shape.is_some());
        self.layers.push(Layer {
            bound: None,
            isolated: group.isolated,
            knockout: group.knockout,
            pixels: Plane::new(&job.planes, area, [0; 4])?,
            shape: if shaped {
                Some(Plane::new(&job.planes, area, 0)?)
            } else {
                None
            },
        });
        Ok(())
    }

    /// Lets the group begun last grow within `area` to what is painted in it.
    pub(crate) fn bound(&mut self, area: Area) {
        if self.layers.len() > 1
            && let Some(top) = self.layers.last_mut()
        {
            top.bound = Some(area);
        }
    }

    /// Paints `paint` where `shape` covers; at once done while measuring.
    pub(crate) fn draw(
        &mut self,
        job: &Job<'_>,
        shape: Shape<'_>,
        paint: &mut dyn Paint,
        how: &Composite,
    ) -> Result<(), PdfError> {
        if job.planes.measuring() {
            return Ok(());
        }
        self.reach(job, shape.area.intersect(how.clip.area))?;
        let Some((top, below)) = self.layers.split_last_mut() else {
            return Ok(());
        };
        let mask = how.clip.mask.as_deref();
        let mut area = shape
            .area
            .intersect(how.clip.area)
            .intersect(top.pixels.area);
        for plane in [shape.cover, mask].into_iter().flatten() {
            area = area.intersect(plane.area);
        }
        if area.is_empty() {
            return Ok(());
        }
        let blending = how.blend != blend::Mode::Normal && !top.isolated;
        let (mut samples, mut under, mut runs) = (Vec::new(), Vec::new(), Vec::new());
        let each = if how.blend == blend::Mode::Normal {
            MARK_WORK
        } else {
            BLEND_WORK
        };
        for row in area.top..area.bottom {
            let cover = track(shape.cover, row, 0);
            let clip = track(mask, row, 0);
            // A cover's known runs narrow without a scan; what is scanned is charged.
            let known = shape.cover.and_then(|plane| {
                let index = usize::try_from(row.saturating_sub(plane.area.top)).ok()?;
                plane.spans.row(index)
            });
            runs.clear();
            runs.try_reserve(known.map_or(1, <[(i32, i32)]>::len))
                .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))?;
            if let Some(found) = known {
                job.spend(1)?;
                for &(first, last) in found {
                    let (start, end) = (area.left.max(first), area.right.min(last));
                    if start < end {
                        runs.push((start, end));
                    }
                }
            } else {
                job.spend((visited(area) >> 4_u32).saturating_add(1))?;
                runs.push(cover.narrowed((area.left, area.right)));
            }
            for &run in &runs {
                // A clip's values are looked at over the run, though it may leave none of it.
                if mask.is_some() {
                    let (start, end) = run;
                    let width = u64::try_from(end.saturating_sub(start)).unwrap_or(0);
                    job.spend(width >> 4_u32)?;
                }
                let (left, right) = clip.narrowed(run);
                let span = Area {
                    left,
                    right,
                    ..area
                };
                if span.is_empty() {
                    continue;
                }
                // Beside its pixels, a run's start and what it looks at.
                let looked = visited(span) >> 4_u32;
                job.spend(
                    visited(span)
                        .saturating_mul(each)
                        .saturating_add(looked)
                        .saturating_add(RUN_WORK),
                )?;
                refill(&mut samples, span.width(), Sample::default())?;
                paint.row(job, row, left, &mut samples)?;
                if blending {
                    refill(&mut under, span.width(), [0; 4])?;
                    backdrop(job, below, row, left, &mut under)?;
                }
                let marks = Marks {
                    clip,
                    cover,
                    left,
                    row,
                    samples: &samples,
                    soft: how.soft.as_ref().map_or_else(
                        || track(None, row, 0),
                        |soft| track(Some(&*soft.values), row, soft.outside),
                    ),
                    under: &under,
                };
                top.land(&marks, how);
            }
        }
        Ok(())
    }

    /// Composites the group begun last into what holds it.
    pub(crate) fn end(&mut self, job: &Job<'_>, how: &Composite) -> Result<(), PdfError> {
        if self.layers.len() > 1
            && let Some(mut group) = self.layers.pop()
        {
            // Nothing lands outside what it painted, often little of its area.
            let shape = Shape {
                area: group.painted(job)?,
                cover: None,
            };
            self.draw(job, shape, &mut group, how)?;
        }
        Ok(())
    }

    /// While measuring, lets the group begun last grow to `area`, its old planes and new counted at
    /// once, as a band may grow it.
    pub(crate) fn grow(&mut self, job: &Job<'_>, area: Area) -> Result<(), PdfError> {
        if job.planes.measuring() {
            self.widen(job, area)?;
        }
        Ok(())
    }

    /// Its luminosity as a soft mask, through `transfer` (11.5.3).
    pub(crate) fn luminosity_mask(
        self,
        job: &Job<'_>,
        transfer: Option<&[u8; 256]>,
    ) -> Result<Plane<u8>, PdfError> {
        // Premultiplied is composited over black.
        masked(&self, job, transfer, |[red, green, blue, _]| {
            blend::luminosity([red, green, blue])
        })
    }

    /// A surface over `area`, transparent or of the colour `backdrop`.
    pub(crate) fn new(
        job: &Job<'_>,
        area: Area,
        backdrop: Option<[u16; 3]>,
    ) -> Result<Self, PdfError> {
        let fill = backdrop.map_or([0; 4], |color| premultiplied(color, u16::MAX));
        Ok(Self {
            area,
            layers: vec![Layer {
                bound: None,
                isolated: true,
                knockout: false,
                pixels: Plane::new(&job.planes, area, fill)?,
                shape: None,
            }],
            whole: job.page,
        })
    }

    /// The premultiplied pixels of row `row`.
    pub(crate) fn pixels(&self, row: i32) -> &[[u16; 4]] {
        self.layers
            .first()
            .map_or(&[], |layer| layer.pixels.row(row))
    }

    /// Widens the group begun last, where it grows, to what `area` paints within its bound.
    fn reach(&mut self, job: &Job<'_>, area: Area) -> Result<(), PdfError> {
        let Some(top) = self.layers.last() else {
            return Ok(());
        };
        let Some(bound) = top.bound else {
            return Ok(());
        };
        let open = top.pixels.area;
        let wanted = area.intersect(bound);
        if wanted.is_empty() || open.intersect(wanted) == wanted {
            return Ok(());
        }
        let wider = open.joined(wanted);
        // Growing fills every pixel.
        let pixels = wider.width().saturating_mul(wider.height());
        job.spend(u64::try_from(pixels >> 2_u32).unwrap_or(u64::MAX))?;
        self.widen(job, wider)
    }

    /// The same over `whole`, which every pass shares: a cell's grid.
    pub(crate) const fn spanning(mut self, whole: Area) -> Self {
        self.whole = whole;
        self
    }

    /// What every pass shares, which holds the area.
    pub(crate) const fn whole(&self) -> Area {
        self.whole
    }

    /// Widens the group begun last to `area`, which holds its own, within what holds it: as if it
    /// had begun so wide, as what lies outside its pixels is transparent.
    fn widen(&mut self, job: &Job<'_>, area: Area) -> Result<(), PdfError> {
        let count = self.layers.len();
        let below = count
            .checked_sub(2)
            .and_then(|index| self.layers.get(index));
        let Some(around) = below.map(|layer| layer.pixels.area) else {
            return Ok(());
        };
        let Some(top) = self.layers.last_mut() else {
            return Ok(());
        };
        let wider = area.intersect(around);
        let mut pixels = Plane::new(&job.planes, wider, [0; 4])?;
        moved(&top.pixels, &mut pixels);
        let shape = top
            .shape
            .as_ref()
            .map(|old| -> Result<Plane<u16>, PdfError> {
                let mut grown = Plane::new(&job.planes, wider, 0)?;
                moved(old, &mut grown);
                Ok(grown)
            })
            .transpose()?;
        top.pixels = pixels;
        top.shape = shape;
        Ok(())
    }
}

impl Track<'_> {
    /// Its value at `column`.
    fn at(&self, column: i32) -> u16 {
        let index = usize::try_from(column.saturating_sub(self.left)).ok();
        let value = index.and_then(|place| self.values.get(place));
        u16::from(*value.unwrap_or(&self.outside)).wrapping_mul(0x0101)
    }

    /// The columns of `span`, all within it, from the first value not 0 to the last.
    fn narrowed(&self, span: (i32, i32)) -> (i32, i32) {
        let (left, right) = span;
        if self.values.is_empty() {
            return span;
        }
        let from = offset(self.left, left);
        let to = offset(self.left, right);
        let values = self.values.get(from..to).unwrap_or_default();
        let column = |index: usize| left.saturating_add(i32::try_from(index).unwrap_or(i32::MAX));
        let first = values.iter().position(|value| *value != 0);
        let last = values.iter().rposition(|value| *value != 0);
        match (first, last) {
            (Some(start), Some(end)) => (column(start), column(end.saturating_add(1))),
            (None, _) | (_, None) => (left, left),
        }
    }
}

/// Copies the rows of `from` into `into`, whose area holds its own.
fn moved<T>(from: &Plane<T>, into: &mut Plane<T>)
where
    T: Copy,
{
    let start = offset(into.area.left, from.area.left);
    for row in from.area.top..from.area.bottom {
        let source = from.row(row);
        let end = start.saturating_add(source.len());
        if let Some(slots) = into.row_mut(row).get_mut(start..end) {
            slots.copy_from_slice(source);
        }
    }
}

/// Lays into `out` what the top layer started over, off the layers `below` it (11.4.4).
fn backdrop(
    job: &Job<'_>,
    below: &[Layer],
    row: i32,
    left: i32,
    out: &mut [[u16; 4]],
) -> Result<(), PdfError> {
    let start = below.iter().rposition(|layer| layer.isolated).unwrap_or(0);
    for layer in below.get(start..).unwrap_or_default() {
        // An element of a knockout group starts over what the group started over.
        if layer.knockout {
            continue;
        }
        job.spend(u64::try_from(out.len()).unwrap_or(u64::MAX))?;
        let from = offset(layer.pixels.area.left, left);
        let pixels = layer.pixels.row(row).get(from..).unwrap_or_default();
        for (under, pixel) in out.iter_mut().zip(pixels) {
            let [_, _, _, alpha] = *pixel;
            *under = join(*under, alpha, *pixel);
        }
    }
    Ok(())
}

/// The colour `source` becomes by `mode` over the premultiplied `beneath`.
fn blended(mode: blend::Mode, beneath: [u16; 4], source: [u16; 3]) -> [u16; 3] {
    let [_, _, _, backed] = beneath;
    if backed == 0 {
        return source;
    }
    let mixed = blend::mix(mode, straight(beneath), source);
    if backed == u16::MAX {
        return mixed;
    }
    let bare = u16::MAX.saturating_sub(backed);
    let part = |own, met| fixed::unit(bare, own).saturating_add(fixed::unit(backed, met));
    let [red, green, blue] = source;
    let [mixed_red, mixed_green, mixed_blue] = mixed;
    [
        part(red, mixed_red),
        part(green, mixed_green),
        part(blue, mixed_blue),
    ]
}

/// The first and the last of `values` that are `marked`.
fn ends<T>(values: &[T], marked: impl Fn(&T) -> bool) -> Option<(usize, usize)> {
    let first = values.iter().position(&marked)?;
    let last = values.iter().rposition(&marked)?;
    Some((first, last))
}

/// `below` less the share `share`, plus the premultiplied `above`.
fn join(below: [u16; 4], share: u16, above: [u16; 4]) -> [u16; 4] {
    if share == u16::MAX {
        return above;
    }
    let kept = u16::MAX.saturating_sub(share);
    let part = |old, new| fixed::unit(old, kept).saturating_add(new);
    let [red, green, blue, alpha] = below;
    let [new_red, new_green, new_blue, new_alpha] = above;
    [
        part(red, new_red),
        part(green, new_green),
        part(blue, new_blue),
        part(alpha, new_alpha),
    ]
}

/// The root of `surface` as a soft mask of `value` through `transfer`.
fn masked(
    surface: &Surface,
    job: &Job<'_>,
    transfer: Option<&[u8; 256]>,
    value: fn([u16; 4]) -> u16,
) -> Result<Plane<u8>, PdfError> {
    let area = surface.area;
    let mut mask = Plane::new(&job.planes, area, 0_u8)?;
    if job.planes.measuring() {
        return Ok(mask);
    }
    for row in area.top..area.bottom {
        job.spend(visited(area))?;
        for (out, pixel) in mask.row_mut(row).iter_mut().zip(surface.pixels(row)) {
            let found = fixed::byte(value(*pixel));
            *out = transfer
                .and_then(|table| table.get(usize::from(found)))
                .copied()
                .unwrap_or(found);
        }
    }
    Ok(mask)
}

/// Where column `left` is in a row that starts at `origin`; past it when before that.
fn offset(origin: i32, left: i32) -> usize {
    usize::try_from(left.saturating_sub(origin)).unwrap_or(usize::MAX)
}

/// `color` times `alpha`, with it.
fn premultiplied(color: [u16; 3], alpha: u16) -> [u16; 4] {
    let [red, green, blue] = color;
    if alpha == u16::MAX {
        return [red, green, blue, alpha];
    }
    let [red_part, green_part, blue_part] = color.map(|part| fixed::unit(part, alpha));
    [red_part, green_part, blue_part, alpha]
}

/// Makes `values` `count` times `fill`, failing where memory does.
fn refill<T>(values: &mut Vec<T>, count: usize, fill: T) -> Result<(), PdfError>
where
    T: Copy,
{
    values.clear();
    values
        .try_reserve_exact(count)
        .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))?;
    values.resize(count, fill);
    Ok(())
}

/// The colour of a premultiplied pixel.
fn straight(pixel: [u16; 4]) -> [u16; 3] {
    let [red, green, blue, alpha] = pixel;
    if alpha == u16::MAX {
        [red, green, blue]
    } else {
        [red, green, blue].map(|part| blend::ratio(part, alpha))
    }
}

/// Row `row` of `plane`, `outside` beside it; none: all.
fn track(plane: Option<&Plane<u8>>, row: i32, outside: u8) -> Track<'_> {
    plane.map_or(
        Track {
            left: 0,
            outside: u8::MAX,
            values: &[],
        },
        |found| Track {
            left: found.area.left,
            outside,
            values: found.row(row),
        },
    )
}

/// The work of a row across `area`.
fn visited(area: Area) -> u64 {
    u64::try_from(area.width())
        .unwrap_or(u64::MAX)
        .saturating_add(1)
}
