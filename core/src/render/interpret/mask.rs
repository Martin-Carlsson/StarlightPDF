//! Soft masks in the graphics state (ISO 32000-2:2020, 11.5, 11.6.5.2).

use alloc::rc::Rc;

use crate::PdfError;
use crate::overlay::Object;
use crate::render::data::{self, Resolved};
use crate::render::fixed::{self, Fixed};
use crate::render::function::{self, Function};
use crate::render::interpret::{Run, xobject};
use crate::render::job::Job;
use crate::render::surface::{Clip, Soft, Surface};
use crate::render::{blend, colour};

/// The colour `BC` gives the backdrop, in the colour space of `group`; `None`: black.
fn backdrop(
    run: &mut Run<'_, '_>,
    owner: &Object,
    group: &Resolved,
) -> Result<Option<[u16; 3]>, PdfError> {
    let job = run.job;
    let items = data::array(job, owner, b"BC", function::VALUES)?;
    if items.is_empty() {
        return Ok(None);
    }
    let attributes = data::entry(job, group.value(), b"Group")?;
    let family = data::entry(job, attributes.value(), b"CS")?;
    let space = if matches!(*family.value(), Object::Null) {
        colour::device(job, &mut run.caches.spaces, &run.scope, items.len())?
    } else {
        colour::resolve(job, &mut run.caches.spaces, &run.scope, &family)?
    };
    let values = items
        .iter()
        .map(|item| Fixed::from_object(item.value()).ok_or_else(|| job.objects.malformed()))
        .collect::<Result<Vec<_>, _>>()?;
    space.rgb(job, &values, run.state.intent).map(Some)
}

/// The soft mask `value` sets: its group drawn now, under the current matrix, in either pass.
pub(crate) fn set(run: &mut Run<'_, '_>, value: &Resolved) -> Result<Option<Soft>, PdfError> {
    let job = run.job;
    let owner = value.value();
    if matches!(owner, Object::Name(name) if name == b"None") {
        return Ok(None);
    }
    let kind = data::entry(job, owner, b"S")?;
    let alpha = matches!(kind.value(), Object::Name(name) if name == b"Alpha");
    if !alpha && !matches!(kind.value(), Object::Name(name) if name == b"Luminosity") {
        return Err(job.objects.malformed());
    }
    let group = data::entry(job, owner, b"G")?;
    let bounds =
        data::numbers(job, group.value(), b"BBox")?.ok_or_else(|| job.objects.malformed())?;
    let area = data::matrix(job, group.value(), b"Matrix")?
        .then(&run.state.matrix)
        .bounds(bounds)
        .intersect(run.surface.area())
        .intersect(run.state.clip.area);
    let table = job
        .tolerate(
            transfer(job, owner),
            "11.6.5.2",
            "a soft mask's transfer function cannot be used",
        )?
        .flatten();
    let color = if alpha {
        None
    } else {
        backdrop(run, owner, &group)?
    };
    let mut drawn = Surface::new(job, area, color)?.spanning(run.surface.whole());
    let mut state = run.state.clone();
    state.alpha_is_shape = false;
    state.blend = blend::Mode::Normal;
    state.clip = Clip { area, mask: None };
    (state.fill_alpha, state.stroke_alpha) = (u16::MAX, u16::MAX);
    state.soft = None;
    let mut inner = Run {
        active: run.active.clone(),
        caches: &mut *run.caches,
        // Counted by its stack: see job::NESTING.
        depth: run.depth.saturating_add(1),
        hidden: 0,
        job,
        locked_color: false,
        scope: Rc::clone(&run.scope),
        state,
        surface: &mut drawn,
    };
    xobject::form(&mut inner, group, "11.6.5.2")?;
    let lit = color.map_or(0, |backed| fixed::byte(blend::luminosity(backed)));
    let outside = table
        .as_deref()
        .and_then(|entries| entries.get(usize::from(lit)))
        .copied()
        .unwrap_or(lit);
    let plane = if alpha {
        drawn.alpha_mask(job, table.as_deref())?
    } else {
        drawn.luminosity_mask(job, table.as_deref())?
    };
    Ok(Some(Soft {
        outside,
        values: Rc::new(plane),
    }))
}

/// The `TR` of `owner` as a table; `None`: identity.
fn transfer(job: &Job<'_>, owner: &Object) -> Result<Option<Box<[u8; 256]>>, PdfError> {
    let value = data::entry(job, owner, b"TR")?;
    let identity = matches!(value.value(), Object::Name(name) if name == b"Identity");
    if identity || matches!(*value.value(), Object::Null) {
        return Ok(None);
    }
    let function = Function::read(job, &value)?;
    if function.inputs() != 1 || function.outputs() != 1 {
        return Err(job.objects.malformed());
    }
    let most = Fixed::from_i64(255);
    let mut table = Box::new([0; 256]);
    for (level, slot) in (0_i64..).zip(table.iter_mut()) {
        let input = Fixed::from_i64(level).over(most).unwrap_or_default();
        let mut found = [Fixed::default()];
        function.evaluate(job, &[input], &mut found)?;
        let [output] = found;
        *slot = fixed::byte(output.unit());
    }
    Ok(Some(table))
}
