//! Glyphs a mark meets: their boxes, and the text that shows the rest where they were
//! (ISO 32000-2:2020, 9.2.4, 9.4.3, 9.4.4, 9.6.4).

use alloc::rc::Rc;
use core::mem;

use crate::objects::ObjectId;
use crate::overlay::Object;
use crate::render::data::{self, Scope};
use crate::render::fixed::Fixed;
use crate::render::geometry::{Matrix, Point};
use crate::render::interpret::Run;
use crate::render::job;
use crate::render::redact::marks::{self, Mark};
use crate::render::redact::walk::{self, Context, Ink, Mode};
use crate::render::text::font::{Code, THOUSAND};
use crate::render::text::type3::Procedures;
use crate::render::text::{self, Block, Font};
use crate::syntax::write_string;
use crate::{ErrorCode, PdfError};

/// Work of a glyph.
const GLYPH_WORK: u64 = 256;

/// What a `TJ` array shows: strings and moves.
enum Piece {
    /// A move back, in thousandths of the size.
    Move(Fixed),
    /// Codes shown.
    Shown(Vec<u8>),
}

/// The box of the glyph `code` of `shown` at the text matrix of `block`, in default user space;
/// none: it may paint anywhere (9.2.4).
fn boxed(
    run: &mut Run<'_, '_>,
    cx: &mut Context<'_>,
    block: &Block,
    shown: &Font,
    code: &Code,
) -> Result<Option<Vec<Point>>, PdfError> {
    let job = run.job;
    let space = text::scaled(&run.state.text, code.origin);
    let full = space.then(&block.matrix()).then(&run.state.matrix);
    let none = Fixed::default();
    let wide = if shown.is_vertical() {
        Fixed::from_i64(1)
    } else {
        code.advance.over(shown.divisor()).unwrap_or_default()
    };
    let area = if let Some(procedures) = shown.procedures() {
        let mut ink = Ink {
            all: false,
            area: procedures.bounds(),
        };
        if let Some(id) = procedures.stream(code.selector) {
            ink.merge(procedure(run, cx, procedures, id)?);
        }
        if ink.all {
            return Ok(None);
        }
        let inked = ink.area.map(|found| extent(&procedures.matrix(), found));
        let [_, bottom, _, top] = inked.unwrap_or_default();
        walk::joined(inked, [none, bottom, wide, top])
    } else {
        let [descent, ascent] = shown.extent();
        let advance = [none, descent, wide, ascent];
        match shown.outline(job, &mut run.caches.fonts.loaded, code)? {
            Some((traced, units)) => traced.corners().map_or(advance, |found| {
                walk::joined(Some(advance), extent(&units, found))
            }),
            None => advance,
        }
    };
    Ok(Some(walk::corners(&full, area)))
}

/// The box of the rectangle `corners` under `matrix`.
fn extent(matrix: &Matrix, corners: [Fixed; 4]) -> [Fixed; 4] {
    matrix.extent(corners)
}

/// Where the glyph procedure `id` of `procedures` paints, in glyph space, measured once (9.6.4).
fn procedure(
    run: &mut Run<'_, '_>,
    cx: &mut Context<'_>,
    procedures: &Procedures,
    id: ObjectId,
) -> Result<Ink, PdfError> {
    if let Some(found) = cx.procedures.get(&id.number) {
        return Ok(*found);
    }
    let job = run.job;
    let anywhere = Ink {
        all: true,
        area: None,
    };
    if run.depth >= job::NESTING || run.active.iter().any(|entry| entry.0 == id) {
        return Ok(anywhere);
    }
    let found = data::resolved(job, Object::Reference(id))?;
    if !matches!(*found.value(), Object::Stream(..)) {
        return Ok(Ink::default());
    }
    let opened = data::open(job, &mut run.caches.streams, &found)?;
    let (saved, scope) = (Box::new(run.state.clone()), Rc::clone(&run.scope));
    run.state.matrix = Matrix::identity();
    if let Some(resources) = procedures.resources()
        && !resources.same(&run.scope.resources)
    {
        run.scope = Rc::new(Scope::new(resources.clone()));
    }
    // Counted by its stack: see job::NESTING.
    run.depth = run.depth.saturating_add(2);
    run.active.push((id, "9.6.4"));
    let result = walk::stream(run, cx, opened, Mode::Measure, false);
    let _ended: Option<(ObjectId, &str)> = run.active.pop();
    run.depth = run.depth.saturating_sub(2);
    (run.state, run.scope) = (*saved, scope);
    let ink = match result {
        Ok(walked) => walked.ink,
        Err(error) if error.code() == ErrorCode::Malformed => {
            job.warn("9.6.4", "a glyph procedure is damaged")?;
            anywhere
        }
        Err(error) => return Err(error),
    };
    let _old: Option<Ink> = cx.procedures.insert(id.number, ink);
    Ok(ink)
}

/// Shows `items`, strings and moves as `TJ` takes them, from `block`'s text matrix: the array
/// that shows the same but the glyphs whose boxes meet `marks`, moving past them instead, when
/// any do, and where all the glyphs paint (9.4.3).
pub(super) fn show(
    run: &mut Run<'_, '_>,
    cx: &mut Context<'_>,
    block: &mut Block,
    items: &[Object],
    marks: &[Mark],
) -> Result<(Option<String>, Ink), PdfError> {
    let job = run.job;
    let mut ink = Ink::default();
    let Some(shown) = run.state.text.font.clone() else {
        // Not shown here, and where other readers would show it is not known.
        if items
            .iter()
            .any(|item| data::text(item).is_some_and(|bytes| !bytes.is_empty()))
        {
            return Err(job.unsupported());
        }
        return Ok((None, ink));
    };
    let (vertical, divisor) = (shown.is_vertical(), shown.divisor());
    let mut pieces = Vec::new();
    let mut removed = false;
    for item in items {
        if let Some(number) = Fixed::from_object(item) {
            job.spend(1)?;
            let none = Fixed::default();
            let moved = [none.minus(number), none, THOUSAND, none];
            text::advance(block, &run.state.text, vertical, moved);
            pieces.push(Piece::Move(number));
            continue;
        }
        let Some(bytes) = data::text(item) else {
            job.warn("9.4.3", "TJ holds neither a string nor a number")?;
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        if !shown.is_embedded() || !shown.is_readable() {
            return Err(job.unsupported());
        }
        let mut kept = Vec::new();
        let mut rest = bytes;
        while !rest.is_empty() {
            job.spend(GLYPH_WORK)?;
            let Some(code) = shown.next(job, rest)? else {
                job.warn("9.7.6.3", "a string ends within a code")?;
                kept.extend_from_slice(rest);
                break;
            };
            let shape = boxed(run, cx, block, &shown, &code)?;
            ink.add(shape.as_deref());
            let meets = shape
                .as_deref()
                .map_or(!marks.is_empty(), |found| marks::meets(marks, found));
            let settings = &run.state.text;
            let mut spacing = settings.character;
            if code.spaced {
                spacing = spacing.plus(settings.word);
            }
            if meets {
                removed = true;
                if !kept.is_empty() {
                    pieces.push(Piece::Shown(mem::take(&mut kept)));
                }
                let span = code
                    .advance
                    .times(settings.size)
                    .over(divisor)
                    .unwrap_or_default()
                    .plus(spacing);
                let back = if span == Fixed::default() {
                    span
                } else {
                    let thousandths = span.times(THOUSAND).over(settings.size);
                    Fixed::default().minus(thousandths.ok_or_else(|| job.unsupported())?)
                };
                pieces.push(Piece::Move(back));
            } else {
                kept.extend_from_slice(rest.get(..code.length).unwrap_or_default());
            }
            let moved = [code.advance, code.rise, divisor, spacing];
            text::advance(block, &run.state.text, vertical, moved);
            rest = rest.get(code.length..).unwrap_or_default();
        }
        if !kept.is_empty() {
            pieces.push(Piece::Shown(kept));
        }
    }
    if !removed {
        return Ok((None, ink));
    }
    Ok((Some(written(pieces)), ink))
}

/// `pieces` as a `TJ` array, moves next to each other added up.
fn written(pieces: Vec<Piece>) -> String {
    let mut merged: Vec<Piece> = Vec::new();
    for piece in pieces {
        match (merged.last_mut(), piece) {
            (Some(&mut Piece::Move(ref mut total)), Piece::Move(more)) => *total = total.plus(more),
            (_, other) => merged.push(other),
        }
    }
    let mut out = String::from("[");
    for (at, piece) in merged.into_iter().enumerate() {
        if at > 0 {
            out.push(' ');
        }
        match piece {
            Piece::Move(number) => marks::decimal(&mut out, number),
            Piece::Shown(bytes) => write_string(&mut out, &bytes),
        }
    }
    out.push(']');
    out
}
