//! Annotations' appearance streams (ISO 32000-2:2020, 12.5.3, 12.5.5).

use crate::PdfError;
use crate::overlay::Object;
use crate::render::data::{self, Resolved};
use crate::render::fixed::Fixed;
use crate::render::geometry::{Matrix, Point};
use crate::render::interpret::{Run, layers, state, xobject};
use crate::render::job;
use crate::rendering::RenderUsage;

/// `Hidden`.
const HIDDEN: i64 = 2;

/// `Invisible`.
const INVISIBLE: i64 = 1;

/// `NoRotate`.
pub(crate) const NO_ROTATE: i64 = 16;

/// `NoView`.
const NO_VIEW: i64 = 32;

/// `Print`.
const PRINT: i64 = 4;

/// Draws the annotations of the page `dictionary` that the usage shows, each from the state as
/// it is.
pub(crate) fn draw(run: &mut Run<'_, '_>, dictionary: &Resolved) -> Result<(), PdfError> {
    let job = run.job;
    let message = "Annots is not an array";
    let found = data::entry(job, dictionary.value(), b"Annots");
    let Some(listed) = job.tolerate(found, "12.5.2", message)? else {
        return Ok(());
    };
    if matches!(*listed.value(), Object::Null) {
        return Ok(());
    }
    let Some(items) = data::items(listed.value()) else {
        return job.warn("12.5.2", message);
    };
    let initial = Box::new(run.state.clone());
    for item in items {
        job.spend(job::NESTING_WORK)?;
        let drawn = data::resolve(job, item).and_then(|annotation| one(run, annotation.value()));
        run.state.clone_from(&initial);
        let _drawn: Option<()> = job.tolerate(
            drawn,
            "12.5.5",
            "an annotation or its appearance is malformed",
        )?;
    }
    Ok(())
}

/// Draws `annotation`.
fn one(run: &mut Run<'_, '_>, annotation: &Object) -> Result<(), PdfError> {
    let job = run.job;
    if matches!(annotation, Object::Null) {
        return Ok(());
    }
    if !matches!(annotation, Object::Dictionary(_)) {
        return Err(job.objects.malformed());
    }
    let value = |key: &[u8]| data::entry(job, annotation, key);
    let flags = Fixed::from_object(value(b"F")?.value()).map_or(0, Fixed::floor);
    let kind = value(b"Subtype")?;
    if !shown(job.usage, flags, kind.value()) || !layers::allows(run, annotation)? {
        return Ok(());
    }
    let Some(stream) = selected(run, annotation)? else {
        return Ok(());
    };
    if !layers::allows(run, stream.value())? {
        return Ok(());
    }
    let numbers = |owner: &Object, key: &[u8]| {
        data::numbers(job, owner, key)?.ok_or_else(|| job.objects.malformed())
    };
    let rectangle = Matrix::identity().extent(numbers(annotation, b"Rect")?);
    let matrix = data::matrix(job, stream.value(), b"Matrix")?;
    let boxed = matrix.extent(numbers(stream.value(), b"BBox")?);
    let Some(mut placed) = placement(boxed, rectangle) else {
        return Ok(());
    };
    if flags & NO_ROTATE != 0 {
        let [left, _, _, top] = rectangle;
        let corner = Point {
            across: left,
            down: top,
        };
        placed = placed.then(&upright(&job.device, corner));
    }
    run.state.matrix = placed.then(&job.device);
    let mixed = data::entry(job, annotation, b"BM")?;
    run.state.blend = state::mode(mixed.value());
    xobject::form(run, stream, "12.5.5")
}

/// The map of the box `from` onto `onto`; none when either has no area.
fn placement(from: [Fixed; 4], onto: [Fixed; 4]) -> Option<Matrix> {
    let [left, bottom, right, top] = from;
    let [to_left, to_bottom, to_right, to_top] = onto;
    if left == right || bottom == top || to_left == to_right || to_bottom == to_top {
        return None;
    }
    let wide = to_right.minus(to_left).over(right.minus(left))?;
    let tall = to_top.minus(to_bottom).over(top.minus(bottom))?;
    let none = Fixed::default();
    Some(Matrix {
        terms: [
            wide,
            none,
            none,
            tall,
            to_left.minus(left.times(wide)),
            to_bottom.minus(bottom.times(tall)),
        ],
    })
}

/// The normal appearance, by `AS` where there are several; none to draw is none.
fn selected(run: &Run<'_, '_>, annotation: &Object) -> Result<Option<Resolved>, PdfError> {
    let job = run.job;
    let appearances = data::entry(job, annotation, b"AP")?;
    let mut found = data::entry(job, appearances.value(), b"N")?;
    if matches!(*found.value(), Object::Dictionary(_)) {
        let chosen = data::entry(job, annotation, b"AS")?;
        let Some(key) = data::name(chosen.value()) else {
            job.warn("12.5.5", "an annotation with several appearances has no AS")?;
            return Ok(None);
        };
        found = data::entry(job, found.value(), key)?;
    }
    if matches!(*found.value(), Object::Null) {
        return Ok(None);
    }
    if matches!(*found.value(), Object::Stream(..)) {
        Ok(Some(found))
    } else {
        Err(job.objects.malformed())
    }
}

/// Whether `usage` shows an annotation of `flags` and the subtype `kind`.
fn shown(usage: RenderUsage, flags: i64, kind: &Object) -> bool {
    let set = |flag: i64| flags & flag != 0;
    if set(HIDDEN) || matches!(kind, Object::Name(name) if name == b"Popup") {
        return false;
    }
    if set(INVISIBLE) && !matches!(kind, Object::Name(name) if standard(name)) {
        return false;
    }
    match usage {
        RenderUsage::Display => !set(NO_VIEW),
        RenderUsage::Print => set(PRINT),
    }
}

/// Whether `name` is a standard type of annotation (12.5.6.1).
const fn standard(name: &[u8]) -> bool {
    matches!(
        name,
        b"3D"
            | b"Caret"
            | b"Circle"
            | b"FileAttachment"
            | b"FreeText"
            | b"Highlight"
            | b"Ink"
            | b"Line"
            | b"Link"
            | b"Movie"
            | b"Polygon"
            | b"PolyLine"
            | b"Popup"
            | b"PrinterMark"
            | b"Projection"
            | b"Redact"
            | b"RichMedia"
            | b"Screen"
            | b"Sound"
            | b"Square"
            | b"Squiggly"
            | b"Stamp"
            | b"StrikeOut"
            | b"Text"
            | b"TrapNet"
            | b"Underline"
            | b"Watermark"
            | b"Widget"
    )
}

/// The turn about `corner` that stands upright under `device`, whose linear part is the page's
/// `Rotate` and scale (12.5.3).
pub(crate) fn upright(device: &Matrix, corner: Point) -> Matrix {
    let [across, down, ..] = device.terms;
    let none = Fixed::default();
    let one = Fixed::from_i64(1);
    let back = none.minus(one);
    let [aa, bb, cc, dd] = if down > none {
        [none, one, back, none]
    } else if down < none {
        [none, back, one, none]
    } else if across < none {
        [back, none, none, back]
    } else {
        [one, none, none, one]
    };
    let turn = Matrix {
        terms: [aa, bb, cc, dd, none, none],
    };
    let turned = turn.linear(corner);
    Matrix {
        terms: [
            aa,
            bb,
            cc,
            dd,
            corner.across.minus(turned.across),
            corner.down.minus(turned.down),
        ],
    }
}
