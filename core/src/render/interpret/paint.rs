//! Path, colour and clipping operators, and what turns a colour into paint
//! (ISO 32000-2:2020, 8.5, 8.6.8).

use alloc::rc::Rc;
use core::mem::take;
use core::slice;

use crate::overlay::Object;
use crate::render::coverage::Rule;
use crate::render::data::{self, Category};
use crate::render::fixed::Fixed;
use crate::render::geometry::{Area, Point};
use crate::render::interpret::state::{Fill, Patterned, Selected, Tint};
use crate::render::interpret::{Run, name_operand, numbers, pixel_edged};
use crate::render::job::Job;
use crate::render::path::Path;
use crate::render::surface::{Clip, Composite, Group, Shape, Solid};
use crate::render::{blend, colour, shading, tiling};
use crate::{ErrorCode, PdfError};

/// The path being built.
#[derive(Default)]
pub(crate) struct Drawing {
    /// It is one rectangle on pixel edges, whose clip needs no mask.
    boxed: bool,
    /// The clip pending until it is painted.
    clip: Option<Rule>,
    /// The current point, in user space.
    current: Point,
    /// The path.
    path: Option<Path>,
    /// Where the subpath started.
    start: Point,
}

/// Filling or stroking.
#[derive(Clone, Copy)]
pub(crate) enum Which {
    /// Filling.
    Fill,
    /// Stroking.
    Stroke,
}

impl Drawing {
    /// Whether a path is being built.
    pub(crate) const fn building(&self) -> bool {
        self.path.is_some()
    }
}

/// Runs a colour operator (8.6.8).
pub(crate) fn color(
    run: &mut Run<'_, '_>,
    name: &[u8],
    operands: &[Object],
) -> Result<(), PdfError> {
    if run.locked_color {
        return Ok(());
    }
    let job = run.job;
    let which = if name.first().is_some_and(u8::is_ascii_uppercase) {
        Which::Stroke
    } else {
        Which::Fill
    };
    let (paint, space) = match name {
        b"cs" | b"CS" => {
            // Until it is read, what it selects paints nothing.
            put(run, which, Fill::None, Selected::Lost);
            let key = name_operand(job, operands, 0)?;
            selected(run, key)?
        }
        b"g" | b"G" | b"k" | b"K" | b"rg" | b"RG" => {
            let space = colour::device(job, &mut run.caches.spaces, &run.scope, operands.len())?;
            let values = components(job, &space, operands)?;
            tinted(run, Tint { space, values })?
        }
        _ => {
            let current = match which {
                Which::Fill => run.state.fill_space.clone(),
                Which::Stroke => run.state.stroke_space.clone(),
            };
            recolored(run, current, operands)?
        }
    };
    put(run, which, paint, space);
    Ok(())
}

/// The values `operands` give in `space`.
fn components(
    job: &Job<'_>,
    space: &colour::Space,
    operands: &[Object],
) -> Result<Vec<Fixed>, PdfError> {
    let mut values = operands
        .iter()
        .map(|operand| Fixed::from_object(operand).ok_or_else(|| job.objects.malformed()))
        .collect::<Result<Vec<_>, _>>()?;
    let Some(surplus) = values.len().checked_sub(space.components()) else {
        return Err(job.objects.malformed());
    };
    if surplus > 0 {
        job.warn("7.8.2", "an operator has too many operands")?;
        drop(values.drain(..surplus));
    }
    Ok(values)
}

/// How `which` joins what is painted.
pub(crate) fn composite(run: &Run<'_, '_>, which: Which) -> Composite {
    Composite {
        alpha: match which {
            Which::Fill => run.state.fill_alpha,
            Which::Stroke => run.state.stroke_alpha,
        },
        alpha_is_shape: run.state.alpha_is_shape,
        blend: run.state.blend,
        clip: run.state.clip.clone(),
        soft: run.state.soft.clone(),
    }
}

/// Ends the group begun last, after `result`, unless that ends the render.
pub(crate) fn ended(
    run: &mut Run<'_, '_>,
    result: Result<(), PdfError>,
    how: &Composite,
) -> Result<(), PdfError> {
    if let Err(error) = result.as_ref()
        && error.code() != ErrorCode::Malformed
    {
        return result;
    }
    let joined = run.surface.end(run.job, how);
    result.and(joined)
}

/// Paints `shape` with what `which` paints: the one place where a colour becomes paint.
pub(crate) fn fill(run: &mut Run<'_, '_>, shape: Shape<'_>, which: Which) -> Result<(), PdfError> {
    let job = run.job;
    let unseen = || shape.area.intersect(run.state.clip.area).is_empty();
    if run.hidden > 0 || (!job.planes.measuring() && unseen()) {
        return Ok(());
    }
    let how = composite(run, which);
    let chosen = match which {
        Which::Fill => run.state.fill.clone(),
        Which::Stroke => run.state.stroke.clone(),
    };
    match chosen {
        Fill::None => Ok(()),
        Fill::Solid(color) => run.surface.draw(job, shape, &mut Solid { color }, &how),
        Fill::Pattern(found) => {
            // Its space and state are of the stream that selected it (8.7.2, 8.7.3.1).
            let parent = run.state.parent.clone();
            run.state.parent.clone_from(&found.parent);
            let result = patterned(run, &found, shape, (&how, which));
            run.state.parent = parent;
            // Else what follows would depend on whether a band culled it.
            let _painted: Option<()> = job.tolerate(result, "8.7", "a pattern cannot be used")?;
            Ok(())
        }
    }
}

/// Fills as `rule` says, then strokes, both at once as a knockout group where transparency
/// shows it (11.7.4.4).
fn marks(
    run: &mut Run<'_, '_>,
    path: &Path,
    rule: Option<Rule>,
    stroked: bool,
) -> Result<(), PdfError> {
    let job = run.job;
    let area = run.surface.area().intersect(run.state.clip.area);
    let inside = path.bounds().intersect(area);
    let state = &run.state;
    let grouped = stroked
        && rule.is_some()
        && (state.fill_alpha < u16::MAX
            || state.stroke_alpha < u16::MAX
            || state.blend != blend::Mode::Normal
            || state.soft.is_some());
    let outline = if grouped {
        let cover = path.stroke(job, &run.state.style, &run.state.matrix, area)?;
        let group = Group {
            area: union(inside, cover.area),
            isolated: false,
            knockout: true,
        };
        run.surface.begin(job, group)?;
        Some(cover)
    } else {
        None
    };
    let filled = rule.map_or(Ok(()), |chosen| {
        let cover = path.fill(job, chosen, inside)?;
        let shape = Shape {
            area: cover.area,
            cover: Some(&cover),
        };
        fill(run, shape, Which::Fill)
    });
    let result = filled.and_then(|()| {
        if !stroked {
            return Ok(());
        }
        let cover = match outline {
            Some(found) => found,
            None => path.stroke(job, &run.state.style, &run.state.matrix, area)?,
        };
        let shape = Shape {
            area: cover.area,
            cover: Some(&cover),
        };
        fill(run, shape, Which::Stroke)
    });
    if grouped {
        let how = plain(area);
        ended(run, result, &how)
    } else {
        result
    }
}

/// Runs a path construction or clipping operator (8.5.2, 8.5.4).
pub(crate) fn path(
    run: &Run<'_, '_>,
    drawing: &mut Drawing,
    name: &[u8],
    operands: &[Object],
) -> Result<(), PdfError> {
    let job = run.job;
    if name == b"W" || name == b"W*" {
        drawing.clip = Some(if name == b"W" {
            Rule::NonZero
        } else {
            Rule::EvenOdd
        });
        return Ok(());
    }
    let at = |across, down| Point { across, down };
    let device = |point: Point| run.state.matrix.apply(point);
    if name == b"m" || name == b"re" {
        // Only a lone re stays boxed.
        let lone = drawing.path.is_none();
        drawing.boxed = false;
        let path = drawing.path.get_or_insert_with(|| {
            Path::new(
                run.surface.area().intersect(run.state.clip.area),
                run.state.matrix,
                run.state.style.clone(),
            )
        });
        if name == b"m" {
            let [across, down] = numbers(job, operands)?;
            drawing.current = at(across, down);
            path.move_to(job, device(drawing.current))?;
        } else {
            let [left, bottom, width, height] = numbers(job, operands)?;
            let (right, top) = (left.plus(width), bottom.plus(height));
            drawing.current = at(left, bottom);
            drawing.boxed = lone && pixel_edged(&run.state.matrix, [left, bottom, right, top]);
            path.move_to(job, device(drawing.current))?;
            for corner in [at(right, bottom), at(right, top), at(left, top)] {
                path.line_to(job, device(corner))?;
            }
            path.close(job)?;
        }
        drawing.start = drawing.current;
        return Ok(());
    }
    let Some(path) = drawing.path.as_mut() else {
        return job.warn("8.5.2", "a path has no starting point");
    };
    // Closing a rectangle again changes nothing.
    drawing.boxed &= name == b"h";
    let current = drawing.current;
    let mut curve = |first: Point, second: Point, end: Point| {
        drawing.current = end;
        path.curve_to(job, [first, second, end].map(device))
    };
    match name {
        b"c" => {
            let [x1, y1, x2, y2, x3, y3] = numbers(job, operands)?;
            curve(at(x1, y1), at(x2, y2), at(x3, y3))?;
        }
        b"v" | b"y" => {
            let [x1, y1, x3, y3] = numbers(job, operands)?;
            let (middle, end) = (at(x1, y1), at(x3, y3));
            if name == b"v" {
                curve(current, middle, end)?;
            } else {
                curve(middle, end, end)?;
            }
        }
        b"l" => {
            let [across, down] = numbers(job, operands)?;
            drawing.current = at(across, down);
            path.line_to(job, device(drawing.current))?;
        }
        _ => {
            drawing.current = drawing.start;
            path.close(job)?;
        }
    }
    Ok(())
}

/// Runs a path-painting operator, then the pending clip (8.5.3, 8.5.4).
pub(crate) fn painted(
    run: &mut Run<'_, '_>,
    drawing: &mut Drawing,
    name: &[u8],
) -> Result<(), PdfError> {
    let job = run.job;
    let clip = drawing.clip.take();
    let boxed = take(&mut drawing.boxed);
    let Some(mut path) = drawing.path.take() else {
        return Ok(());
    };
    if matches!(name, b"b" | b"b*" | b"s") {
        path.close(job)?;
    }
    let rule = match name {
        b"B" | b"b" | b"f" | b"F" => Some(Rule::NonZero),
        b"B*" | b"b*" | b"f*" => Some(Rule::EvenOdd),
        _ => None,
    };
    let stroked = matches!(name, b"B" | b"B*" | b"b" | b"b*" | b"s" | b"S");
    let result = if run.hidden > 0 {
        Ok(())
    } else {
        marks(run, &path, rule, stroked)
    };
    if let Some(clipping) = clip {
        let inside = path
            .bounds()
            .intersect(run.surface.area())
            .intersect(run.state.clip.area);
        let cover = if boxed {
            None
        } else {
            Some(path.fill(job, clipping, inside)?)
        };
        let shape = Shape {
            area: inside,
            cover: cover.as_ref(),
        };
        run.state.clip = run.state.clip.within(job, shape)?;
    }
    result
}

/// Paints `shape` with the pattern `found`, as `which` paints (8.7).
fn patterned(
    run: &mut Run<'_, '_>,
    found: &Patterned,
    shape: Shape<'_>,
    (how, which): (&Composite, Which),
) -> Result<(), PdfError> {
    let job = run.job;
    let pattern = data::followed(job, &found.pattern)?;
    let kind = data::entry(job, pattern.value(), b"PatternType")?;
    match *kind.value() {
        Object::Integer(1) => tiling::pattern(run, &pattern, found.color, shape, how),
        Object::Integer(2) => shading::pattern(run, &pattern, shape, (how, which)),
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => Err(job.objects.malformed()),
    }
}

/// How a group joins that is there only to knock out: plainly, within `area`.
pub(crate) const fn plain(area: Area) -> Composite {
    Composite {
        alpha: u16::MAX,
        alpha_is_shape: false,
        blend: blend::Mode::Normal,
        clip: Clip { area, mask: None },
        soft: None,
    }
}

/// Makes `which` paint `paint`, in `space`.
fn put(run: &mut Run<'_, '_>, which: Which, paint: Fill, space: Selected) {
    match which {
        Which::Fill => (run.state.fill, run.state.fill_space) = (paint, space),
        Which::Stroke => (run.state.stroke, run.state.stroke_space) = (paint, space),
    }
}

/// What `sc` or `scn` with `operands` selects in the space `current`.
fn recolored(
    run: &mut Run<'_, '_>,
    current: Selected,
    operands: &[Object],
) -> Result<(Fill, Selected), PdfError> {
    let job = run.job;
    let space = match current {
        Selected::Lost => return Ok((Fill::None, Selected::Lost)),
        Selected::Pattern(below) => return repatterned(run, below, operands),
        Selected::Initial => colour::device(job, &mut run.caches.spaces, &run.scope, 1)?,
        Selected::Space(tint) => Rc::clone(&tint.space),
    };
    let values = components(job, &space, operands)?;
    tinted(run, Tint { space, values })
}

/// What `scn` with `operands` selects in `Pattern` over the space of `below` (8.6.6.2).
fn repatterned(
    run: &Run<'_, '_>,
    below: Option<Rc<Tint>>,
    operands: &[Object],
) -> Result<(Fill, Selected), PdfError> {
    let job = run.job;
    let last = operands.len().saturating_sub(1);
    let key = name_operand(job, operands, last)?;
    let Some(pattern) = data::named(job, &run.scope, Category::Pattern, key)? else {
        job.warn("7.8.3", "a pattern is missing")?;
        return Ok((Fill::None, Selected::Pattern(below)));
    };
    let given = operands.get(..last).unwrap_or_default();
    let (mut tint, mut color) = (below, None);
    if !given.is_empty()
        && let Some(held) = tint.as_ref()
    {
        let made = Rc::new(Tint {
            space: Rc::clone(&held.space),
            values: components(job, &held.space, given)?,
        });
        color = shade(run, &made)?;
        tint = Some(made);
        if color.is_none() {
            return Ok((Fill::None, Selected::Pattern(tint)));
        }
    }
    // Kept by its number where it has one, so that saved states hold none of it.
    let kept = pattern.id().map_or(pattern, data::reference);
    let paint = Patterned {
        color,
        parent: run.state.parent.clone(),
        pattern: kept,
    };
    Ok((Fill::Pattern(Rc::new(paint)), Selected::Pattern(tint)))
}

/// Converts both colours again: content began, whose resources give the initial `DeviceGray`
/// (8.6.5.6), or the intent changed (8.6.5.8).
pub(crate) fn retint(run: &mut Run<'_, '_>) -> Result<(), PdfError> {
    if run.locked_color {
        return Ok(());
    }
    let (filling, below) = (run.state.fill.clone(), run.state.fill_space.clone());
    run.state.fill = retinted(run, filling, below)?;
    let (stroking, beside) = (run.state.stroke.clone(), run.state.stroke_space.clone());
    run.state.stroke = retinted(run, stroking, beside)?;
    Ok(())
}

/// What paints `paint`, selected as `space`, now.
fn retinted(run: &mut Run<'_, '_>, paint: Fill, space: Selected) -> Result<Fill, PdfError> {
    let job = run.job;
    match space {
        Selected::Initial => {
            let gray = Tint {
                space: colour::device(job, &mut run.caches.spaces, &run.scope, 1)?,
                values: vec![Fixed::default()],
            };
            Ok(shade(run, &gray)?.map_or(Fill::None, Fill::Solid))
        }
        Selected::Lost | Selected::Pattern(None) => Ok(paint),
        Selected::Pattern(Some(tint)) => {
            if let Fill::Pattern(found) = paint.clone()
                && found.color.is_some()
            {
                return Ok(Fill::Pattern(Rc::new(Patterned {
                    color: shade(run, &tint)?,
                    parent: found.parent.clone(),
                    pattern: found.pattern.clone(),
                })));
            }
            Ok(paint)
        }
        Selected::Space(tint) => Ok(shade(run, &tint)?.map_or(Fill::None, Fill::Solid)),
    }
}

/// The space `key` that `cs` selects and its initial colour.
fn selected(run: &mut Run<'_, '_>, key: &[u8]) -> Result<(Fill, Selected), PdfError> {
    let job = run.job;
    let direct = matches!(
        key,
        b"DeviceCMYK" | b"DeviceGray" | b"DeviceRGB" | b"Pattern"
    );
    let value = if direct {
        // One of these short names, copied.
        data::made(job, Object::Name(key.to_vec()), None)?
    } else if let Some(found) = data::named(job, &run.scope, Category::ColorSpace, key)? {
        found
    } else {
        job.warn("7.8.3", "a colour space is missing")?;
        return Ok((Fill::None, Selected::Lost));
    };
    let parts = data::items(value.value()).unwrap_or_else(|| slice::from_ref(value.value()));
    let family = parts.first().ok_or_else(|| job.objects.malformed())?;
    let first = data::resolve(job, family)?;
    let pattern = data::name(first.value()) == Some(b"Pattern".as_slice());
    let named = if pattern {
        let Some(base) = parts.get(1) else {
            return Ok((Fill::None, Selected::Pattern(None)));
        };
        data::resolve(job, base)?
    } else {
        value
    };
    let space = colour::resolve(job, &mut run.caches.spaces, &run.scope, &named)?;
    let mut values = Vec::new();
    if pattern {
        let below = Rc::new(Tint { space, values });
        return Ok((Fill::None, Selected::Pattern(Some(below))));
    }
    values.resize(space.components(), Fixed::default());
    space.initial(&mut values);
    tinted(run, Tint { space, values })
}

/// `tint` converted; none where it marks nothing (8.6.6.4).
fn shade(run: &Run<'_, '_>, tint: &Tint) -> Result<Option<[u16; 3]>, PdfError> {
    if !tint.space.marks() {
        return Ok(None);
    }
    tint.space
        .rgb(run.job, &tint.values, run.state.intent)
        .map(Some)
}

/// What `tint` paints, and it selected.
fn tinted(run: &Run<'_, '_>, tint: Tint) -> Result<(Fill, Selected), PdfError> {
    let paint = shade(run, &tint)?.map_or(Fill::None, Fill::Solid);
    Ok((paint, Selected::Space(Rc::new(tint))))
}

/// The pixels of both and between.
fn union(first: Area, second: Area) -> Area {
    if first.is_empty() {
        return second;
    }
    if second.is_empty() {
        return first;
    }
    Area {
        bottom: first.bottom.max(second.bottom),
        left: first.left.min(second.left),
        right: first.right.max(second.right),
        top: first.top.min(second.top),
    }
}
