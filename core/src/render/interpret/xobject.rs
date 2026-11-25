//! `Do`: images and forms (ISO 32000-2:2020, 8.8, 8.10, 11.6.6).

use crate::PdfError;
use crate::overlay::Object;
use crate::render::data::{self, Category, Resolved};
use crate::render::interpret::{Content, Run, layers, nested, paint};
use crate::render::surface::{Clip, Composite, Group};
use crate::render::{blend, image};

/// Paints the `XObject` resource `key`.
pub(crate) fn draw(run: &mut Run<'_, '_>, key: &[u8]) -> Result<(), PdfError> {
    let job = run.job;
    let Some(found) = data::named(job, &run.scope, Category::XObject, key)? else {
        return job.warn("7.8.3", "an XObject is missing");
    };
    if run.hidden > 0 || !layers::allows(run, found.value())? {
        return Ok(());
    }
    let kind = data::entry(job, found.value(), b"Subtype")?;
    if matches!(kind.value(), Object::Name(name) if name == b"Image") {
        return image::draw(run, &found);
    }
    if !matches!(kind.value(), Object::Name(name) if name == b"Form") {
        return job.warn("8.8", "an XObject is neither an image nor a form");
    }
    form(run, found, "8.10.1")
}

/// Runs the form `stream` under the current matrix, as a group where it is one: composited once
/// as the state says, from plain transparency inside (8.10.1, 11.6.6); `clause` names what it is.
pub(crate) fn form(
    run: &mut Run<'_, '_>,
    stream: Resolved,
    clause: &'static str,
) -> Result<(), PdfError> {
    let job = run.job;
    let owner = stream.value();
    let bounds = data::numbers(job, owner, b"BBox")?.ok_or_else(|| job.objects.malformed())?;
    let resources = data::entry(job, owner, b"Resources")?;
    let attributes = data::entry(job, owner, b"Group")?;
    let kind = data::entry(job, attributes.value(), b"S")?;
    let grouped = matches!(kind.value(), Object::Name(name) if name == b"Transparency");
    let matrix = data::matrix(job, owner, b"Matrix")?;
    let content = Content {
        bounds: Some(bounds),
        clause,
        locked_color: false,
        matrix,
        resources: (!matches!(*resources.value(), Object::Null)).then_some(resources),
        serial: None,
        stream,
    };
    if !grouped {
        return nested(run, &content);
    }
    let area = matrix
        .then(&run.state.matrix)
        .bounds(bounds)
        .intersect(run.surface.area())
        .intersect(run.state.clip.area);
    let flag = |key: &[u8]| {
        data::entry(job, attributes.value(), key)
            .map(|found| matches!(*found.value(), Object::Boolean(true)))
    };
    let group = Group {
        area,
        isolated: flag(b"I")?,
        knockout: flag(b"K")?,
    };
    // The clip's edges are in each object already.
    let joined = Composite {
        clip: Clip {
            area: run.state.clip.area,
            mask: None,
        },
        ..paint::composite(run, paint::Which::Fill)
    };
    run.surface.begin(job, group)?;
    let saved = Box::new(run.state.clone());
    run.state.blend = blend::Mode::Normal;
    (run.state.fill_alpha, run.state.stroke_alpha) = (u16::MAX, u16::MAX);
    run.state.soft = None;
    let result = nested(run, &content);
    run.state = *saved;
    paint::ended(run, result, &joined)
}
