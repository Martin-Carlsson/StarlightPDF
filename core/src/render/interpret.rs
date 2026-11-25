//! The content interpreter (ISO 32000-2:2020, 7.8.2, Annex A).

pub(crate) mod annotations;
pub(crate) mod content;
pub(crate) mod inline;
pub(crate) mod layers;
pub(crate) mod mask;
pub(crate) mod paint;
pub(crate) mod state;
pub(crate) mod xobject;

use alloc::rc::Rc;

use crate::PdfError;
use crate::objects::ObjectId;
use crate::overlay::Object;
use crate::page::display;
use crate::pull::Pull;
use crate::render::data::{self, Category, Resolved, Scope};
use crate::render::fixed::Fixed;
use crate::render::geometry::{Area, Matrix, Point};
use crate::render::job::{self, Job};
use crate::render::path::Path;
use crate::render::plane::Plane;
use crate::render::surface::{Clip, Group, Shape, Surface};
use crate::render::{colour, coverage, image, shading, text, tiling};

/// The bytes of an operator's operands held uncounted while it runs: nesting keeps a few levels'.
const OPERANDS_LENT: usize = 0x4000;

/// What a render keeps across passes and bands.
#[derive(Default)]
pub(crate) struct Caches<'doc> {
    /// How many streams within others began, which numbers them.
    pub begun: u64,
    /// Fonts.
    pub fonts: text::Fonts,
    /// Open and held images.
    pub images: image::Images<'doc>,
    /// The text objects some glyph of which turns transparent, found while measuring.
    pub knocks: text::Knocks,
    /// Visibility of optional content.
    pub layers: layers::Layers,
    /// Colour spaces.
    pub spaces: colour::Spaces,
    /// Small streams.
    pub streams: data::Streams,
    /// Tile cells.
    pub tiles: tiling::Tiles,
}

/// Content that content runs: a form, an appearance, a soft mask's group, a tile, a glyph.
pub(crate) struct Content {
    /// Its box, in its own space, which clips it.
    pub bounds: Option<[Fixed; 4]>,
    /// The clause of what it is, which a warning names.
    pub clause: &'static str,
    /// Colour operators are ignored (8.7.3.3, 9.6.4).
    pub locked_color: bool,
    /// The map from its space to the caller's.
    pub matrix: Matrix,
    /// Its resources; none: the caller's.
    pub resources: Option<Resolved>,
    /// Its number, where it begins as a stream before did; none: a new one.
    pub serial: Option<u64>,
    /// The stream.
    pub stream: Resolved,
}

/// What lasts as long as one stream.
#[derive(Default)]
struct Frame {
    /// How deep `BX` nests.
    compatibility: u32,
    /// The current path.
    drawing: paint::Drawing,
    /// The marked-content levels.
    marked: layers::Marked,
    /// The saved states.
    saved: Vec<state::State>,
    /// The text object.
    text: Option<text::Block>,
}

/// One pass of content over a surface.
pub(crate) struct Run<'run, 'doc> {
    /// The streams being run.
    pub active: Vec<(ObjectId, &'static str)>,
    /// The caches.
    pub caches: &'run mut Caches<'doc>,
    /// How deep content nests.
    pub depth: u8,
    /// How many hidden levels enclose the content.
    pub hidden: u32,
    /// The render.
    pub job: &'run Job<'doc>,
    /// Colour operators are ignored.
    pub locked_color: bool,
    /// The resources.
    pub scope: Rc<Scope>,
    /// The graphics state.
    pub state: state::State,
    /// What it paints on.
    pub surface: &'run mut Surface,
}

impl Caches<'_> {
    /// A number no stream has.
    pub(crate) const fn number(&mut self) -> u64 {
        self.begun = self.begun.saturating_add(1);
        self.begun
    }
}

/// How many operands the operator `name` takes, the colour setters at least; `None` when it is no
/// operator (Annex A).
const fn arity(name: &[u8]) -> Option<usize> {
    Some(match name {
        b"b" | b"B" | b"b*" | b"B*" | b"BI" | b"BT" | b"BX" | b"EMC" | b"ET" | b"EX" | b"f"
        | b"F" | b"f*" | b"h" | b"n" | b"q" | b"Q" | b"s" | b"S" | b"T*" | b"W" | b"W*" => 0,
        b"BMC" | b"cs" | b"CS" | b"Do" | b"g" | b"G" | b"gs" | b"i" | b"j" | b"J" | b"M"
        | b"MP" | b"ri" | b"sc" | b"SC" | b"scn" | b"SCN" | b"sh" | b"Tc" | b"Tj" | b"TJ"
        | b"TL" | b"Tr" | b"Ts" | b"Tw" | b"Tz" | b"w" | b"'" => 1,
        b"BDC" | b"d" | b"d0" | b"DP" | b"l" | b"m" | b"Td" | b"TD" | b"Tf" => 2,
        b"rg" | b"RG" | b"\"" => 3,
        b"k" | b"K" | b"re" | b"v" | b"y" => 4,
        b"c" | b"cm" | b"d1" | b"Tm" => 6,
        _ => return None,
    })
}

/// The clip within the rectangle `bounds` under the current matrix; one on pixel edges needs no
/// mask.
// Never inlined: content nests through its caller, and the stack is small.
#[inline(never)]
fn clipped(run: &Run<'_, '_>, bounds: [Fixed; 4]) -> Result<Clip, PdfError> {
    let (job, matrix) = (run.job, run.state.matrix);
    let area = matrix
        .bounds(bounds)
        .intersect(run.surface.area())
        .intersect(run.state.clip.area);
    if area.is_empty() || pixel_edged(&matrix, bounds) {
        return run.state.clip.within(job, Shape { area, cover: None });
    }
    let cover = rectangle(run, &matrix, bounds, area)?;
    run.state.clip.within(
        job,
        Shape {
            area,
            cover: Some(&cover),
        },
    )
}

/// Runs one stream's operations, keeping what was drawn before damage (7.8.2).
fn execute(run: &mut Run<'_, '_>, data: Box<dyn Pull + '_>) -> Result<(), PdfError> {
    let job = run.job;
    // Boxed: content nests, and the stack is small on WebAssembly.
    let ending: Box<dyn Pull + '_> = Box::new(content::Ending::new(job, data));
    let mut operations = Box::new(content::Operations::new(ending));
    let mut frame = Box::new(Frame::default());
    let mut operands = Vec::new();
    loop {
        let read = operations.next(job, &mut operands);
        let Some(Some(name)) = job.tolerate(read, "7.8.2", "a content stream is damaged")? else {
            break;
        };
        job.spend(job::OPERATOR_WORK)?;
        // Its operands stay while it runs, and content it nests runs inside: counted, past a few.
        let held = operations.held();
        let lent = if held > OPERANDS_LENT {
            let token = Rc::new(());
            job.lend(&token, u64::try_from(held).unwrap_or(u64::MAX))?;
            Some(token)
        } else {
            None
        };
        let stepped = step(run, &mut frame, &mut operations, name.bytes(), &operands);
        drop(lent);
        let going = job.tolerate(stepped, "7.8.2", "an operator or its objects are malformed")?;
        if !going.unwrap_or(true) {
            break;
        }
    }
    // Its group and clip end with its stream.
    if let Some(block) = frame.text.take() {
        job.warn("9.4.1", "BT without ET")?;
        let ended = text::end(run, block);
        let _ended: Option<()> = job.tolerate(ended, "9.4.1", "a text object cannot be ended")?;
    }
    Ok(())
}

/// The name operand at `index`.
pub(crate) fn name_operand<'operands>(
    job: &Job<'_>,
    operands: &'operands [Object],
    index: usize,
) -> Result<&'operands [u8], PdfError> {
    operands
        .get(index)
        .and_then(data::name)
        .ok_or_else(|| job.objects.malformed())
}

/// Runs `content` inside the current content, the one point of recursion: it checks depth and
/// repetition, culls by the box in band passes, and leaves the state as it found it (8.10.1).
pub(crate) fn nested(run: &mut Run<'_, '_>, content: &Content) -> Result<(), PdfError> {
    let job = run.job;
    let id = content.stream.id();
    // Content that runs itself nests past any limit; under strict that fails, else it is left out.
    let again = id.and_then(|found| {
        let entry = run.active.iter().find(|entry| entry.0 == found)?;
        Some(entry.1)
    });
    if run.depth >= job::NESTING || again.is_some() {
        if job.objects.strict() {
            return Err(job.limit());
        }
        let clause = again.unwrap_or(content.clause);
        return job.warn(clause, "content nests too deep or runs itself");
    }
    job.spend(job::NESTING_WORK)?;
    let matrix = content.matrix.then(&run.state.matrix);
    if let Some(bounds) = content.bounds
        && !job.planes.measuring()
        && matrix
            .bounds(bounds)
            .intersect(run.surface.area())
            .intersect(run.state.clip.area)
            .is_empty()
    {
        // What cannot be opened fails in every band, as in the pass without pixels.
        return match data::coded(job, &content.stream)?.image {
            Some(_) => Err(job.objects.malformed()),
            None => Ok(()),
        };
    }
    let opened = data::open(job, &mut run.caches.streams, &content.stream)?;
    let saved = Box::new(run.state.clone());
    let (hidden, locked, scope) = (run.hidden, run.locked_color, Rc::clone(&run.scope));
    run.state.matrix = matrix;
    run.locked_color |= content.locked_color;
    // The same resources keep their scope, so that what it read is not read and held again.
    if let Some(resources) = content.resources.as_ref()
        && !resources.same(&run.scope.resources)
    {
        run.scope = Rc::new(Scope::new(resources.clone()));
    }
    run.depth = run.depth.saturating_add(1);
    run.active.extend(id.map(|found| (found, content.clause)));
    let clip = content
        .bounds
        .map(|bounds| clipped(run, bounds))
        .transpose();
    let result = clip.and_then(|found| {
        if let Some(inner) = found {
            run.state.clip = inner;
        }
        retint(run)?;
        let serial = content.serial.unwrap_or_else(|| run.caches.number());
        run.state.begin(run.active.len(), serial);
        execute(run, opened)
    });
    if id.is_some() {
        let _ended: Option<(ObjectId, &str)> = run.active.pop();
    }
    run.depth = run.depth.saturating_sub(1);
    (run.hidden, run.locked_color, run.scope) = (hidden, locked, scope);
    run.state = *saved;
    result
}

/// All `COUNT` operands, as numbers.
pub(crate) fn numbers<const COUNT: usize>(
    job: &Job<'_>,
    operands: &[Object],
) -> Result<[Fixed; COUNT], PdfError> {
    if operands.len() != COUNT {
        return Err(job.objects.malformed());
    }
    let mut found = [Fixed::default(); COUNT];
    for (slot, operand) in found.iter_mut().zip(operands) {
        *slot = Fixed::from_object(operand).ok_or_else(|| job.objects.malformed())?;
    }
    Ok(found)
}

/// Runs the page `dictionary` on `surface`: its content, then its annotations (7.7.3.3).
pub(crate) fn page<'doc>(
    job: &Job<'doc>,
    caches: &mut Caches<'doc>,
    surface: &mut Surface,
    dictionary: &Resolved,
) -> Result<(), PdfError> {
    let (objects, overlay) = (job.objects, &*job.overlay);
    let inherited = job.tolerate(
        display::inherited(objects, overlay, dictionary.value(), b"Resources"),
        "7.7.3.4",
        "the page's Resources are damaged",
    )?;
    let resources = data::made(job, inherited.unwrap_or(Object::Null), None)?;
    let contents = job.tolerate(
        display::streams(
            objects,
            overlay,
            Some(data::stored(dictionary.value(), b"Contents").clone()),
        ),
        "7.7.3.3",
        "Contents is neither a stream nor an array",
    )?;
    let group = data::entry(job, dictionary.value(), b"Group")
        .and_then(|found| data::entry(job, found.value(), b"K"));
    let flag = job.tolerate(group, "11.4.7", "the page's Group is damaged")?;
    let knockout = flag.is_some_and(|found| matches!(*found.value(), Object::Boolean(true)));
    let initial = state::State::new(job.device, surface.area());
    let mut run = Run {
        active: Vec::new(),
        caches,
        depth: 0,
        hidden: 0,
        job,
        locked_color: false,
        scope: Rc::new(Scope::new(resources)),
        state: initial.clone(),
        surface,
    };
    // Counted while the content runs.
    let listed = data::made(job, Object::Array(contents.unwrap_or_default()), None)?;
    let joined = content::Joined::new(job, listed);
    retint(&mut run)?;
    // The page's is the same in each band.
    run.state.begin(0, 0);
    let area = run.surface.area();
    if knockout {
        let whole = Group {
            area,
            isolated: true,
            knockout: true,
        };
        run.surface.begin(job, whole)?;
    }
    let drawn = execute(&mut run, Box::new(joined));
    if knockout {
        paint::ended(&mut run, drawn, &paint::plain(area))?;
    } else {
        drawn?;
    }
    run.state = initial;
    run.hidden = 0;
    run.locked_color = false;
    annotations::draw(&mut run, dictionary)
}

/// Whether the rectangle `bounds` under `matrix` is upright with its sides on pixel edges.
fn pixel_edged(matrix: &Matrix, bounds: [Fixed; 4]) -> bool {
    let [left, bottom, right, top] = bounds;
    let whole = |value: Fixed| value == Fixed::from_i64(value.floor());
    let [aa, bb, cc, dd, _, _] = matrix.terms;
    let none = Fixed::default();
    ((bb == none && cc == none) || (aa == none && dd == none))
        && [(left, bottom), (right, top)]
            .into_iter()
            .all(|(across, down)| {
                let corner = matrix.apply(Point { across, down });
                whole(corner.across) && whole(corner.down)
            })
}

/// What the rectangle `bounds` under `matrix` covers within `area`.
pub(crate) fn rectangle(
    run: &Run<'_, '_>,
    matrix: &Matrix,
    bounds: [Fixed; 4],
    area: Area,
) -> Result<Plane<u8>, PdfError> {
    let job = run.job;
    let [left, bottom, right, top] = bounds;
    let corner = |across, down| matrix.apply(Point { across, down });
    let mut path = Path::new(area, *matrix, run.state.style.clone());
    path.move_to(job, corner(left, bottom))?;
    for point in [corner(right, bottom), corner(right, top), corner(left, top)] {
        path.line_to(job, point)?;
    }
    path.close(job)?;
    path.fill(job, coverage::Rule::NonZero, area)
}

/// Sets the colours content begins with.
fn retint(run: &mut Run<'_, '_>) -> Result<(), PdfError> {
    let job = run.job;
    let (fill, stroke) = (&run.state.fill_space, &run.state.stroke_space);
    if !matches!(fill, state::Selected::Initial) && !matches!(stroke, state::Selected::Initial) {
        return Ok(());
    }
    let done = paint::retint(run);
    let _begun: Option<()> =
        job.tolerate(done, "8.6.5.6", "a default colour space cannot be read")?;
    Ok(())
}

/// Runs the operator `name`; `false` ends the stream.
fn step(
    run: &mut Run<'_, '_>,
    frame: &mut Frame,
    operations: &mut content::Operations<'_>,
    name: &[u8],
    operands: &[Object],
) -> Result<bool, PdfError> {
    let job = run.job;
    let Some(count) = arity(name) else {
        if frame.compatibility == 0 {
            job.warn("7.8.2", "an unknown operator")?;
        }
        return Ok(true);
    };
    let Some(surplus) = operands.len().checked_sub(count) else {
        job.warn("7.8.2", "an operator has too few operands")?;
        return Ok(true);
    };
    // The colour setters take what there is.
    let all = matches!(name, b"sc" | b"SC" | b"scn" | b"SCN");
    if surplus > 0 && !all {
        job.warn("7.8.2", "an operator has too many operands")?;
    }
    let given = operands
        .get(if all { 0 } else { surplus }..)
        .unwrap_or_default();
    let stating = matches!(
        name,
        b"cm" | b"d" | b"gs" | b"i" | b"j" | b"J" | b"M" | b"q" | b"Q" | b"ri" | b"w"
    );
    if stating && frame.drawing.building() {
        job.warn("8.2", "a graphics state operator inside a path object")?;
    }
    let painting = matches!(
        name,
        b"b" | b"B" | b"b*" | b"B*" | b"BI" | b"Do" | b"f" | b"F" | b"f*" | b"s" | b"S" | b"sh"
    );
    if painting && let Some(block) = frame.text.as_mut() {
        job.warn("8.2", "a painting operator inside a text object")?;
        // What it paints is not in the text's knockout group, in every band alike.
        text::interrupt(run, block)?;
    }
    match name {
        b"b" | b"B" | b"b*" | b"B*" | b"f" | b"F" | b"f*" | b"n" | b"s" | b"S" => {
            paint::painted(run, &mut frame.drawing, name)?;
        }
        b"c" | b"h" | b"l" | b"m" | b"re" | b"v" | b"W" | b"W*" | b"y" => {
            paint::path(run, &mut frame.drawing, name, given)?;
        }
        b"cs" | b"CS" | b"g" | b"G" | b"k" | b"K" | b"rg" | b"RG" | b"sc" | b"SC" | b"scn"
        | b"SCN" => paint::color(run, name, given)?,
        b"BDC" | b"BMC" | b"DP" | b"EMC" | b"MP" => {
            layers::marked(run, &mut frame.marked, name, given)?;
        }
        b"BT" | b"ET" | b"T*" | b"Td" | b"TD" | b"Tj" | b"TJ" | b"Tm" | b"'" | b"\"" => {
            let at = operations.input().position();
            text_object(run, &mut frame.text, (name, at), given)?;
        }
        b"BI" => inline::draw(run, operations)?,
        b"BX" => frame.compatibility = frame.compatibility.saturating_add(1),
        b"EX" => frame.compatibility = frame.compatibility.saturating_sub(1),
        b"d0" => text::glyph_metrics(run, true, given)?,
        b"d1" => text::glyph_metrics(run, false, given)?,
        b"Do" => xobject::draw(run, name_operand(job, given, 0)?)?,
        b"sh" => {
            let key = name_operand(job, given, 0)?;
            match data::named(job, &run.scope, Category::Shading, key)? {
                Some(found) if run.hidden == 0 => shading::operator(run, &found)?,
                Some(_) => {}
                None => job.warn("7.8.3", "a shading is missing")?,
            }
        }
        _ => state::operator(run, &mut frame.saved, name, given)?,
    }
    Ok(true)
}

/// Runs an operator that starts, ends or is in a text object (9.4).
fn text_object(
    run: &mut Run<'_, '_>,
    object: &mut Option<text::Block>,
    (name, at): (&[u8], u64),
    operands: &[Object],
) -> Result<(), PdfError> {
    if name == b"BT" {
        if object.is_some() {
            return run.job.warn("9.4.1", "BT within a text object");
        }
        // Where it begins, the same in every pass.
        let stream = run
            .active
            .last()
            .map(|entry| (entry.0.number, entry.0.generation));
        *object = Some(text::begin(run, (stream, at))?);
    } else if name == b"ET" {
        match object.take() {
            Some(block) => text::end(run, block)?,
            None => run.job.warn("9.4.1", "ET without BT")?,
        }
    } else if let Some(block) = object.as_mut() {
        text::operator(run, block, name, operands)?;
    } else {
        run.job
            .warn("9.4.1", "a text operator outside a text object")?;
    }
    Ok(())
}
