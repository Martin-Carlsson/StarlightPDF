//! Text: state, positioning, modes and clipping (ISO 32000-2:2020, 9).

mod cmap;
pub(crate) mod font;
pub(crate) mod type3;

pub(crate) use font::Font;

use alloc::collections::BTreeSet;
use alloc::rc::Rc;

use crate::PdfError;
use crate::objects::ObjectId;
use crate::overlay::Object;
use crate::render::blend;
use crate::render::coverage::Rule;
use crate::render::data::{self, Resolved};
use crate::render::fixed::Fixed;
use crate::render::geometry::{Area, Matrix};
use crate::render::interpret::paint::{self, Which};
use crate::render::interpret::{Run, numbers};
use crate::render::path::Path;
use crate::render::plane::Plane;
use crate::render::surface::{Group, Shape};

/// The most fonts given directly a scope keeps.
const DIRECT_FONTS: usize = 64;

/// The most text objects kept as knocking out.
const KNOCKS: usize = 0x1_0000;

/// Work of a glyph.
const GLYPH_WORK: u64 = 256;

/// The longest name a font is keyed by (Annex C).
const NAME: usize = 127;

/// A text object (9.4).
pub(crate) struct Block {
    /// What its glyphs clip, united.
    clip: Option<Plane<u8>>,
    /// The knockout group begun and its pixels (9.3.8).
    group: Option<Area>,
    /// Where it begins.
    key: Place,
    /// Its glyphs knock out: in a band where one of them turns transparent; while measuring, always,
    /// as a band may.
    knocking: bool,
    /// The text line matrix.
    line: Matrix,
    /// Text of unknown advance moved the text matrix.
    lost: bool,
    /// The text matrix.
    matrix: Matrix,
    /// A glyph of it turned transparent.
    seen: bool,
}

/// The text objects some glyph of which turns transparent, by where they begin, as the pass that
/// measures finds them; past [`KNOCKS`] every text object knocks out.
#[derive(Default)]
pub(crate) struct Knocks {
    /// Every text object knocks out.
    all: bool,
    /// Those found.
    found: BTreeSet<Place>,
}

/// Where a text object begins: its stream, none for the page's content, and the offset after `BT`.
type Place = (Option<(u32, u16)>, u64);

/// The fonts a render read.
#[derive(Default)]
pub(crate) struct Fonts {
    /// The glyph procedures being run: depth and stream.
    pub glyphs: Vec<(u8, ObjectId)>,
    /// The fonts, their programs and their outlines.
    pub loaded: font::Cache,
}

/// What a rendering mode has a glyph do (9.3.6).
#[derive(Clone, Copy)]
struct Marking {
    /// It adds to the clip.
    clips: bool,
    /// It is filled.
    fills: bool,
    /// It is stroked.
    strokes: bool,
}

/// The text state (9.3).
#[derive(Clone)]
pub(crate) struct Settings {
    /// `Tc`.
    pub character: Fixed,
    /// The font.
    pub font: Option<Rc<Font>>,
    /// `TK`.
    pub knockout: bool,
    /// `TL`.
    pub leading: Fixed,
    /// `Tr`.
    pub mode: u8,
    /// `Ts`.
    pub rise: Fixed,
    /// `Tz`, over 100.
    pub scale: Fixed,
    /// The font's size.
    pub size: Fixed,
    /// `Tw`.
    pub word: Fixed,
}

impl Block {
    /// The text matrix.
    pub(crate) const fn matrix(&self) -> Matrix {
        self.matrix
    }

    /// Moves to the start of a line `across` and `down` from this one's (9.4.2).
    fn next_line(&mut self, across: Fixed, down: Fixed) {
        self.place(translated(across, down).then(&self.line));
    }

    /// Makes `matrix` the text matrix and the line's.
    const fn place(&mut self, matrix: Matrix) {
        (self.line, self.matrix, self.lost) = (matrix, matrix, false);
    }
}

impl Marking {
    /// Whether it needs the glyph.
    const fn draws(self) -> bool {
        self.clips || self.paints()
    }

    /// What `run`'s mode has a glyph do; a Type 3 one, `procedural`, is only shown or not.
    const fn of(run: &Run<'_, '_>, procedural: bool) -> Self {
        let mode = run.state.text.mode;
        let shown = run.hidden == 0;
        if procedural {
            let visible = shown && !matches!(mode, 3 | 7);
            return Self {
                clips: false,
                fills: visible,
                strokes: visible,
            };
        }
        Self {
            // A clip is state, which hidden content sets too (8.11.3).
            clips: mode >= 4,
            fills: shown && matches!(mode, 0 | 2 | 4 | 6),
            strokes: shown && matches!(mode, 1 | 2 | 5 | 6),
        }
    }

    /// Whether it paints.
    const fn paints(self) -> bool {
        self.fills || self.strokes
    }

    /// Whether the state paints it with transparency.
    fn transparent(self, run: &Run<'_, '_>) -> bool {
        let state = &run.state;
        (self.fills && state.fill_alpha < u16::MAX)
            || (self.strokes && state.stroke_alpha < u16::MAX)
            || state.blend != blend::Mode::Normal
            || state.soft.is_some()
    }
}

/// Moves past `width` and up `rise`, in `divisor`s of the size, and `spacing` (9.4.4).
pub(crate) fn advance(text: &mut Block, settings: &Settings, vertical: bool, moved: [Fixed; 4]) {
    let [width, rise, divisor, spacing] = moved;
    let scaled = |value: Fixed| value.times(settings.size).over(divisor).unwrap_or_default();
    let span = scaled(width).plus(spacing);
    let shift = if vertical {
        translated(Fixed::default(), span)
    } else {
        translated(span.times(settings.scale), scaled(rise))
    };
    text.matrix = shift.then(&text.matrix);
}

/// `BT` at `key`.
pub(crate) fn begin(run: &Run<'_, '_>, key: Place) -> Result<Block, PdfError> {
    run.job.spend(1)?;
    // Known from the pass that measures, so a band begins the group at the first glyph it paints;
    // that pass counts a group from the first glyph of every text object, as a band may hold one.
    let knocks = &run.caches.knocks;
    let knocking = run.job.planes.measuring() || knocks.all || knocks.found.contains(&key);
    Ok(Block {
        clip: None,
        group: None,
        key,
        knocking,
        line: Matrix::identity(),
        lost: false,
        matrix: Matrix::identity(),
        seen: false,
    })
}

/// `ET`: ends the knockout group, then clips (9.3.6, 9.3.8).
pub(crate) fn end(run: &mut Run<'_, '_>, text: Block) -> Result<(), PdfError> {
    if let Some(area) = text.group {
        paint::ended(run, Ok(()), &paint::plain(area))?;
    }
    if let Some(cover) = text.clip {
        let shape = Shape {
            area: cover.area,
            cover: Some(&cover),
        };
        run.state.clip = run.state.clip.within(run.job, shape)?;
    }
    Ok(())
}

/// Ends the knockout group of `text`, before what is no glyph paints within it; a glyph after
/// begins another.
pub(crate) fn interrupt(run: &mut Run<'_, '_>, text: &mut Block) -> Result<(), PdfError> {
    if let Some(area) = text.group.take() {
        paint::ended(run, Ok(()), &paint::plain(area))?;
    }
    Ok(())
}

/// Whether `matrix` takes all to a line or a point.
fn flat(matrix: &Matrix) -> bool {
    let [aa, bb, cc, dd, _, _] = matrix.terms.map(|term| i128::from(term.bits()));
    aa.saturating_mul(dd) == bb.saturating_mul(cc)
}

/// Paints and clips with an outline, which `matrix` takes to pixels.
fn glyph(
    run: &mut Run<'_, '_>,
    text: &mut Block,
    traced: &font::Traced,
    matrix: &Matrix,
    marking: Marking,
) -> Result<(), PdfError> {
    let job = run.job;
    let area = run.surface.area().intersect(run.state.clip.area);
    if !marking.strokes && !job.planes.measuring() && traced.area(matrix).intersect(area).is_empty()
    {
        return Ok(());
    }
    let mut path = Path::new(area, run.state.matrix, run.state.style.clone());
    traced.draw(job, &mut path, matrix)?;
    let inside = path.bounds();
    let inner = if marking.fills || marking.clips {
        Some(path.fill(job, Rule::NonZero, inside)?)
    } else {
        None
    };
    if marking.clips
        && let (Some(clip), Some(cover)) = (text.clip.as_mut(), inner.as_ref())
    {
        unite(run, clip, cover)?;
    }
    if !marking.paints() {
        return Ok(());
    }
    let outer = if marking.strokes {
        Some(path.stroke(job, &run.state.style, &run.state.matrix, area)?)
    } else {
        None
    };
    let touched = outer.as_ref().map_or(inside, |cover| cover.area);
    if !job.planes.measuring() && touched.is_empty() {
        return Ok(());
    }
    knockout(run, text, touched)?;
    // Filled and stroked it is one object (11.7.4.4).
    let grouped =
        marking.fills && marking.strokes && text.group.is_none() && marking.transparent(run);
    if grouped {
        let group = Group {
            area: touched,
            isolated: false,
            knockout: true,
        };
        run.surface.begin(job, group)?;
    }
    let mut result = if marking.fills {
        painted(run, inner.as_ref(), Which::Fill)
    } else {
        Ok(())
    };
    if marking.strokes {
        result = result.and_then(|()| painted(run, outer.as_ref(), Which::Stroke));
    }
    if grouped {
        paint::ended(run, result, &paint::plain(touched))
    } else {
        result
    }
}

/// `d0` when `colored`, else `d1` (9.6.4).
pub(crate) fn glyph_metrics(
    run: &mut Run<'_, '_>,
    colored: bool,
    operands: &[Object],
) -> Result<(), PdfError> {
    // No glyph is ended: what one paints past the box d1 gives would depend on the bands.
    type3::metrics(run, colored, operands)
}

/// Begins the group in which glyphs knock out, where one of them needs it: over what `touched`
/// covers, growing to what is painted in it, so that its work follows the glyphs.
fn knockout(run: &mut Run<'_, '_>, text: &mut Block, touched: Area) -> Result<(), PdfError> {
    if !text.knocking {
        return Ok(());
    }
    let job = run.job;
    let within = run.surface.area().intersect(run.state.clip.area);
    if let Some(open) = text.group {
        // The clip grew within the text object.
        let wider = open.joined(within);
        if wider != open {
            run.surface.bound(wider);
            run.surface.grow(job, wider)?;
            text.group = Some(wider);
        }
        return Ok(());
    }
    let area = touched.intersect(within);
    if !job.planes.measuring() {
        // Beginning fills every pixel.
        let pixels = area.width().saturating_mul(area.height());
        job.spend(u64::try_from(pixels >> 2_u32).unwrap_or(u64::MAX))?;
    }
    let group = Group {
        area,
        isolated: false,
        knockout: true,
    };
    run.surface.begin(job, group)?;
    run.surface.bound(within);
    // While measuring, all a band may grow it to.
    run.surface.grow(job, within)?;
    text.group = Some(within);
    Ok(())
}

/// Marks what `code` selects at the text matrix.
fn mark(
    run: &mut Run<'_, '_>,
    text: &mut Block,
    shown: &Font,
    code: &font::Code,
    marking: Marking,
) -> Result<(), PdfError> {
    let job = run.job;
    let space = scaled(&run.state.text, code.origin);
    if flat(&space.then(&text.matrix).then(&run.state.matrix)) {
        return Ok(());
    }
    if text.lost || !shown.is_embedded() {
        return if !shown.is_embedded() && shown.is_blank(code) {
            Ok(())
        } else {
            Err(job.unsupported())
        };
    }
    // Decided before anything is culled, else pixels would depend on the bands.
    let knocks = run.state.text.knockout && marking.paints() && marking.transparent(run);
    if knocks && !text.seen && job.planes.measuring() {
        let found = &mut run.caches.knocks;
        found.all |= found.found.len() >= KNOCKS;
        if !found.all {
            let _new: bool = found.found.insert(text.key);
        }
    }
    text.seen |= knocks;
    text.knocking |= knocks;
    if let Some(procedures) = shown.procedures() {
        let Some(id) = procedures.stream(code.selector) else {
            return job.warn("9.6.4", "a code has no glyph procedure");
        };
        // What the glyph paints widens the group.
        knockout(run, text, Area::default())?;
        let matrix = procedures.matrix().then(&space).then(&text.matrix);
        return type3::show(run, procedures, id, matrix);
    }
    let found = shown.outline(job, &mut run.caches.fonts.loaded, code)?;
    let Some((traced, units)) = found else {
        return Ok(());
    };
    let matrix = units
        .then(&space)
        .then(&text.matrix)
        .then(&run.state.matrix);
    glyph(run, text, &traced, &matrix, marking)
}

/// Runs a text positioning or showing operator.
pub(crate) fn operator(
    run: &mut Run<'_, '_>,
    text: &mut Block,
    name: &[u8],
    operands: &[Object],
) -> Result<(), PdfError> {
    let job = run.job;
    match name {
        b"T*" => return_line(run, text),
        b"Td" | b"TD" => {
            let [across, down] = numbers(job, operands)?;
            if name == b"TD" {
                run.state.text.leading = Fixed::default().minus(down);
            }
            text.next_line(across, down);
        }
        b"Tm" => {
            let terms = numbers(job, operands)?;
            text.place(Matrix { terms });
        }
        b"TJ" => {
            let items = operands.first().and_then(data::items);
            return show_array(run, text, items.ok_or_else(|| job.objects.malformed())?);
        }
        _ => {
            if name == b"\"" {
                let [word, character] = numbers(job, operands.get(..2).unwrap_or_default())?;
                run.state.text.word = word;
                run.state.text.character = character;
            }
            if name != b"Tj" {
                return_line(run, text);
            }
            let bytes = operands.last().and_then(data::text);
            return show(run, text, bytes.ok_or_else(|| job.objects.malformed())?);
        }
    }
    Ok(())
}

/// Fills or strokes `cover`.
fn painted(run: &mut Run<'_, '_>, cover: Option<&Plane<u8>>, which: Which) -> Result<(), PdfError> {
    cover.map_or(Ok(()), |found| {
        let shape = Shape {
            area: found.area,
            cover: Some(found),
        };
        paint::fill(run, shape, which)
    })
}

/// `T*`.
fn return_line(run: &Run<'_, '_>, text: &mut Block) {
    let none = Fixed::default();
    text.next_line(none, none.minus(run.state.text.leading));
}

/// The map to the space of the text matrix of a glyph whose position vector is `origin`
/// (9.4.4, 9.7.4.3).
pub(crate) fn scaled(settings: &Settings, origin: [Fixed; 2]) -> Matrix {
    let none = Fixed::default();
    let across = settings.size.times(settings.scale);
    let [right, up] = origin;
    let back = |part: Fixed, side: Fixed| part.times(side).over(font::THOUSAND).unwrap_or_default();
    Matrix {
        terms: [
            across,
            none,
            none,
            settings.size,
            none.minus(back(right, across)),
            settings.rise.minus(back(up, settings.size)),
        ],
    }
}

/// Sets the font `font`, the resource `name` where it is one, at `size`.
pub(crate) fn set_font(
    run: &mut Run<'_, '_>,
    (font, name): (&Resolved, Option<&[u8]>),
    size: Fixed,
) -> Result<(), PdfError> {
    run.state.text.size = size;
    // One given directly has no number to be kept by: its scope keeps it by name.
    let direct = name.filter(|_name| font.id().is_none());
    let kept = run
        .scope
        .fonts
        .borrow()
        .iter()
        .find(|entry| Some(entry.0.as_slice()) == direct)
        .map(|entry| Rc::clone(&entry.1));
    let loaded = if let Some(found) = kept {
        found
    } else {
        let made = font::load(run, font)?;
        // A longer name than a name may be (Annex C) is not copied to be a key.
        if let Some(key) = direct.filter(|given| given.len() <= NAME) {
            let mut fonts = run.scope.fonts.borrow_mut();
            if fonts.len() >= DIRECT_FONTS {
                fonts.clear();
            }
            fonts.push((key.to_vec(), Rc::clone(&made)));
        }
        made
    };
    run.state.text.font = Some(loaded);
    Ok(())
}

/// Shows `bytes` (9.4.3).
fn show(run: &mut Run<'_, '_>, text: &mut Block, bytes: &[u8]) -> Result<(), PdfError> {
    let job = run.job;
    let Some(shown) = run.state.text.font.clone() else {
        return job.warn("9.3.1", "text without a font");
    };
    if bytes.is_empty() {
        return Ok(());
    }
    let marking = Marking::of(run, shown.procedures().is_some());
    if marking.clips && text.clip.is_none() {
        let area = run.surface.area().intersect(run.state.clip.area);
        if !job.planes.measuring() {
            let pixels = area.width().saturating_mul(area.height());
            job.spend(u64::try_from(pixels >> 4_u32).unwrap_or(u64::MAX))?;
        }
        text.clip = Some(Plane::new(&job.planes, area, 0_u8)?);
    }
    if !shown.is_readable() {
        let space = scaled(&run.state.text, [Fixed::default(); 2]);
        if marking.draws() && !flat(&space.then(&text.matrix).then(&run.state.matrix)) {
            return Err(job.unsupported());
        }
        // How far it goes is in the CMap.
        text.lost = true;
        return Ok(());
    }
    let (vertical, divisor) = (shown.is_vertical(), shown.divisor());
    let mut rest = bytes;
    while !rest.is_empty() {
        job.spend(GLYPH_WORK)?;
        let Some(code) = shown.next(job, rest)? else {
            job.warn("9.7.6.3", "a string ends within a code")?;
            break;
        };
        if marking.draws() {
            mark(run, text, &shown, &code, marking)?;
        }
        let settings = &run.state.text;
        let mut spacing = settings.character;
        if code.spaced {
            spacing = spacing.plus(settings.word);
        }
        let moved = [code.advance, code.rise, divisor, spacing];
        advance(text, settings, vertical, moved);
        rest = rest.get(code.length..).unwrap_or_default();
    }
    text.lost |= !shown.is_measured() && run.state.text.size != Fixed::default();
    Ok(())
}

/// Runs `TJ`: strings, and numbers that move back in thousandths (9.4.3).
fn show_array(run: &mut Run<'_, '_>, text: &mut Block, items: &[Object]) -> Result<(), PdfError> {
    let job = run.job;
    for item in items {
        if let Some(number) = Fixed::from_object(item) {
            job.spend(1)?;
            let settings = &run.state.text;
            let vertical = settings
                .font
                .as_ref()
                .is_some_and(|found| found.is_vertical());
            let none = Fixed::default();
            let moved = [none.minus(number), none, font::THOUSAND, none];
            advance(text, settings, vertical, moved);
        } else if let Some(bytes) = data::text(item) {
            show(run, text, bytes)?;
        } else {
            job.warn("9.4.3", "TJ holds neither a string nor a number")?;
        }
    }
    Ok(())
}

/// The map that moves by `across` and `down`.
const fn translated(across: Fixed, down: Fixed) -> Matrix {
    let [aa, bb, cc, dd, _, _] = Matrix::identity().terms;
    Matrix {
        terms: [aa, bb, cc, dd, across, down],
    }
}

/// Adds `cover` to the clip by its maximum (9.3.6).
fn unite(run: &Run<'_, '_>, clip: &mut Plane<u8>, cover: &Plane<u8>) -> Result<(), PdfError> {
    let job = run.job;
    if job.planes.measuring() {
        return Ok(());
    }
    // Its rows' spans no longer hold.
    clip.spans.clear();
    let area = clip.area.intersect(cover.area);
    let skipped = |origin: i32| usize::try_from(area.left.saturating_sub(origin)).unwrap_or(0);
    let (to, from) = (skipped(clip.area.left), skipped(cover.area.left));
    let width = area.width();
    for row in area.top..area.bottom {
        job.spend(u64::try_from(width).unwrap_or(u64::MAX))?;
        let added = cover.row(row).iter().skip(from).take(width);
        for (slot, value) in clip.row_mut(row).iter_mut().skip(to).zip(added) {
            *slot = (*slot).max(*value);
        }
    }
    Ok(())
}
