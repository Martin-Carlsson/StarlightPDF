//! Type 3 fonts and their glyph procedures (ISO 32000-2:2020, 9.6.4).

use alloc::collections::BTreeMap;

use crate::PdfError;
use crate::objects::ObjectId;
use crate::overlay::Object;
use crate::render::data::{self, Resolved};
use crate::render::fixed::Fixed;
use crate::render::geometry::Matrix;
use crate::render::interpret::{Content, Run, nested, numbers};
use crate::render::job::Job;
use crate::render::text::font;

/// The bytes a procedure takes.
const PROCEDURE: u64 = 16;

/// What a Type 3 font draws with.
pub(super) struct Procedures {
    /// `FontMatrix`.
    matrix: Matrix,
    /// Its resources; none: the caller's.
    resources: Option<Resolved>,
    /// Each code's procedure.
    streams: Vec<Option<ObjectId>>,
}

impl Procedures {
    /// The bytes it holds.
    pub(super) fn bytes(&self) -> u64 {
        let count = u64::try_from(self.streams.len()).unwrap_or(0);
        count.saturating_mul(PROCEDURE)
    }

    /// The map from glyph space to text space.
    pub(super) const fn matrix(&self) -> Matrix {
        self.matrix
    }

    /// The procedures of the font `owner`.
    pub(super) fn read(job: &Job<'_>, owner: &Object) -> Result<Self, PdfError> {
        let terms =
            data::numbers(job, owner, b"FontMatrix")?.ok_or_else(|| job.objects.malformed())?;
        let listed = data::entry(job, owner, b"CharProcs")?;
        let entries = data::pairs(listed.value()).ok_or_else(|| job.objects.malformed())?;
        let mut named = BTreeMap::new();
        for pair in entries {
            job.spend(1)?;
            let _slot: &mut &Object = named.entry(pair.0.as_slice()).or_insert(&pair.1);
        }
        let encoding = font::Encoding::read(job, owner)?;
        // Each code's name is looked up.
        font::looked_up(job)?;
        let mut streams = Vec::new();
        for code in 0..=u8::MAX {
            let procedure = match encoding.name(code).and_then(|name| named.get(name)) {
                Some(value) => {
                    if let Object::Reference(id) = **value {
                        Some(id)
                    } else {
                        job.warn("9.6.4", "a glyph procedure is no stream")?;
                        None
                    }
                }
                None => None,
            };
            streams.push(procedure);
        }
        // Counted while held, as all that is resolved.
        let found = data::entry(job, owner, b"Resources")?;
        let resources = matches!(*found.value(), Object::Dictionary(_)).then_some(found);
        Ok(Self {
            matrix: Matrix { terms },
            resources,
            streams,
        })
    }

    /// How far up the advance `width` of glyph space goes in text space: `FontMatrix` may turn it
    /// (9.6.4).
    pub(super) fn rise(&self, width: Fixed) -> Fixed {
        let [_, up, _, _, _, _] = self.matrix.terms;
        width.times(up)
    }

    /// The procedure of `code`; none paints nothing.
    pub(super) fn stream(&self, code: u32) -> Option<ObjectId> {
        let index = usize::try_from(code).ok()?;
        self.streams.get(index).copied().flatten()
    }

    /// The width `width` of glyph space in text space.
    pub(super) fn width(&self, width: Fixed) -> Fixed {
        let [across, _, _, _, _, _] = self.matrix.terms;
        width.times(across)
    }
}

/// `d0` when `colored`, else `d1`, which keeps the colour of the text (9.6.4).
pub(super) fn metrics(
    run: &mut Run<'_, '_>,
    colored: bool,
    operands: &[Object],
) -> Result<(), PdfError> {
    let job = run.job;
    // In the procedure's own stream only: elsewhere d1 would take the colours away.
    let inside = run.caches.fonts.glyphs.last().is_some_and(|glyph| {
        glyph.0 == run.depth && Some(glyph.1) == run.active.last().map(|entry| entry.0)
    });
    if !inside {
        return job.warn("9.6.4", "d0 or d1 outside a glyph procedure");
    }
    if colored {
        let [_, _]: [Fixed; 2] = numbers(job, operands)?;
    } else {
        run.locked_color = true;
        let [_, _, _, _, _, _]: [Fixed; 6] = numbers(job, operands)?;
    }
    Ok(())
}

/// Runs the procedure `id` of `procedures`, which `matrix` maps to user space.
pub(super) fn show(
    run: &mut Run<'_, '_>,
    procedures: &Procedures,
    id: ObjectId,
    matrix: Matrix,
) -> Result<(), PdfError> {
    let job = run.job;
    let found = data::resolved(job, Object::Reference(id))?;
    if !matches!(*found.value(), Object::Stream(..)) {
        return job.warn("9.6.4", "a glyph procedure is no stream");
    }
    let Some(reached) = found.id() else {
        return Err(job.objects.malformed());
    };
    let content = Content {
        bounds: None,
        clause: "9.6.4",
        locked_color: false,
        matrix,
        resources: procedures.resources.clone(),
        serial: None,
        stream: found,
    };
    let around = run.depth;
    // Counted by its stack: see job::NESTING.
    run.depth = around.saturating_add(1);
    let depth = run.depth.saturating_add(1);
    run.caches.fonts.glyphs.push((depth, reached));
    let result = nested(run, &content);
    run.depth = around;
    let _ended: Option<(u8, ObjectId)> = run.caches.fonts.glyphs.pop();
    let _run: Option<()> = job.tolerate(result, "9.6.4", "a glyph procedure is damaged")?;
    Ok(())
}
