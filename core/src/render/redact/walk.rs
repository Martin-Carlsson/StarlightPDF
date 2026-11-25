//! Content walked for what lies under the marks, keeping the graphics state as the renderer does
//! but painting nothing (ISO 32000-2:2020, 7.8.2, 8, 9).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::mem;

use crate::PdfError;
use crate::objects::ObjectId;
use crate::overlay::{Data, Object};
use crate::pull::Pull;
use crate::render::data::{self, Category, Resolved, Scope};
use crate::render::fixed::Fixed;
use crate::render::geometry::{Matrix, Point};
use crate::render::interpret::state::State;
use crate::render::interpret::{self, Run, content, inline, name_operand, numbers, state};
use crate::render::job;
use crate::render::redact::marks::{self, Mark};
use crate::render::redact::stage::Stage;
use crate::render::redact::{cut, glyph, image, structure};
use crate::render::text::{self, Block};
use crate::syntax::write_name;

/// The bytes of an operator's operands held uncounted while it runs.
const OPERANDS_LENT: usize = 0x4000;

/// Keys of a stream copy leaves out: its data is new and unfiltered (7.3.8.2).
const UNFILTERED: [&[u8]; 7] = [
    b"DL",
    b"DecodeParms",
    b"F",
    b"FDecodeParms",
    b"FFilter",
    b"Filter",
    b"Length",
];

/// What a walk needs beside the run.
pub(super) struct Context<'cx> {
    /// The marks.
    pub marks: &'cx [Mark],
    /// The `XObject` names content that is measured draws, which no copy leaves out.
    pub measured: BTreeSet<(&'static [u8], Vec<u8>)>,
    /// Where each glyph procedure paints, in glyph space, by its stream's number.
    pub procedures: BTreeMap<u32, Ink>,
    /// What is changed.
    pub stage: &'cx mut Stage,
}

/// What became of a form walked.
enum Copied {
    /// It stays.
    Kept,
    /// Its copy, which a mark changed.
    Made(ObjectId),
    /// It is left out.
    Out,
}

/// A change to a stream: the bytes from `from` to `to` become `text`.
pub(super) struct Edit {
    /// The first byte.
    pub from: u64,
    /// What they become.
    pub text: String,
    /// Past the last byte.
    pub to: u64,
}

/// What walking a stream found.
#[derive(Default)]
pub(super) struct Found {
    /// Resources its copy adds: category, name and value.
    pub added: Vec<(&'static [u8], Vec<u8>, Object)>,
    /// Damage ended it.
    pub damaged: bool,
    /// Its changes, in order.
    pub edits: Vec<Edit>,
    /// Where damage ended its operations, which its copy ends at.
    pub end: Option<u64>,
    /// Where it paints, when measured.
    pub ink: Ink,
    /// The `q` open at its end.
    pub open: usize,
    /// The resources its copy leaves out, by category and name: only used where it uses copies.
    pub removed: Vec<(&'static [u8], Vec<u8>)>,
    /// The `Q` without a `q`.
    pub stray: usize,
    /// A text object is open at its end.
    pub texting: bool,
    /// The resources it uses unchanged, by category and name.
    pub used: BTreeSet<(&'static [u8], Vec<u8>)>,
}

/// What lasts as long as one stream.
struct Frame {
    /// The box of the clip; none: unclipped.
    clip: Option<[Fixed; 4]>,
    /// `W` or `W*` came: the path clips.
    clipping: bool,
    /// What it found.
    found: Found,
    /// It runs within marked content that structure keeps.
    inside: bool,
    /// Whether each marked-content level open keeps `ActualText`, `Alt`, `E` or an `MCID`.
    marked: Vec<bool>,
    /// Whether it changes content or measures it.
    mode: Mode,
    /// The box of the path, in default user space.
    path: Option<[Fixed; 4]>,
    /// The resources it uses copies of, by category and name.
    replaced: Vec<(&'static [u8], Vec<u8>)>,
    /// The saved states and their clips.
    saved: Vec<(State, Option<[Fixed; 4]>)>,
    /// The text object.
    text: Option<Block>,
}

/// Where content paints.
#[derive(Clone, Copy, Default)]
pub(super) struct Ink {
    /// It may paint anywhere.
    pub all: bool,
    /// The box of what it paints: left, bottom, right and top.
    pub area: Option<[Fixed; 4]>,
}

/// Whether a walk changes content or measures where it paints.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Mode {
    /// Measures.
    Measure,
    /// Changes what the marks meet.
    Rewrite,
}

impl Found {
    /// Whether its stream must be copied.
    pub(super) const fn changed(&self) -> bool {
        self.damaged || !self.edits.is_empty() || !self.added.is_empty()
    }
}

impl Frame {
    /// Whether the content is within marked content that structure keeps.
    fn kept(&self) -> bool {
        self.inside || self.marked.iter().any(|kept| *kept)
    }

    /// Whether what paints `shape`, none being anywhere, meets a mark; measuring, it is counted.
    fn reach(&mut self, marks: &[Mark], shape: Option<&[Point]>) -> bool {
        if self.mode == Mode::Measure {
            self.found.ink.add(shape);
            return false;
        }
        shape.map_or(!marks.is_empty(), |found| marks::meets(marks, found))
    }
}

impl Ink {
    /// Adds `shape`, none being anywhere.
    pub(super) fn add(&mut self, shape: Option<&[Point]>) {
        let Some(found) = shape else {
            self.all = true;
            return;
        };
        for point in found {
            self.area = Some(joined(
                self.area,
                [point.across, point.down, point.across, point.down],
            ));
        }
    }

    /// Adds `other`.
    pub(super) fn merge(&mut self, other: Self) {
        self.all |= other.all;
        if let Some(area) = other.area {
            self.area = Some(joined(self.area, area));
        }
    }
}

/// The stream `owner` with `data`, unfiltered, and `resources` where given.
fn copied(owner: &Object, data: Vec<u8>, resources: Option<Object>) -> Object {
    let entries = data::pairs(owner).map_or_else(Vec::new, |listed| {
        listed
            .iter()
            .filter(|pair| !UNFILTERED.contains(&pair.0.as_slice()))
            .cloned()
            .collect()
    });
    let mut made = Object::Stream(entries, Data::Bytes(Arc::from(data)));
    if resources.is_some() {
        made.set(b"Resources", resources);
    }
    made
}

/// The corners of the rectangle `corners` under `matrix`.
pub(super) fn corners(matrix: &Matrix, corners: [Fixed; 4]) -> Vec<Point> {
    marks::rectangle(corners)
        .into_iter()
        .map(|point| matrix.apply(point))
        .collect()
}

/// `data` with `found`'s edits made, cut where damage ended it, within `room` bytes.
pub(super) fn emit(
    job: &job::Job<'_>,
    data: Box<dyn Pull + '_>,
    found: &Found,
    room: u64,
) -> Result<Vec<u8>, PdfError> {
    let outer = job.damaged.get();
    let mut reader = content::Ending::new(job, data);
    let mut edits = found.edits.iter().peekable();
    let end = found.end.unwrap_or(u64::MAX);
    let (mut out, mut buffer) = (Vec::new(), vec![0; 0x4000]);
    let (mut at, mut skipped) = (0_u64, 0_u64);
    'read: loop {
        let count = reader.pull(&mut buffer)?;
        if count == 0 {
            break;
        }
        for byte in buffer.get(..count).unwrap_or_default() {
            if at >= end {
                break 'read;
            }
            while let Some(edit) = edits.next_if(|edit| edit.from <= at) {
                out.push(b'\n');
                out.extend_from_slice(edit.text.as_bytes());
                skipped = skipped.max(edit.to);
            }
            if at >= skipped {
                out.push(*byte);
            }
            at = at.saturating_add(1);
        }
        if u64::try_from(out.len()).unwrap_or(u64::MAX) > room {
            return Err(job.limit());
        }
    }
    for edit in edits {
        out.push(b'\n');
        out.extend_from_slice(edit.text.as_bytes());
    }
    job.damaged.set(outer);
    Ok(out)
}

/// `Do` of the `XObject` `made`, under a name `frame`'s copy adds.
fn drawn(run: &Run<'_, '_>, frame: &mut Frame, made: ObjectId) -> Result<String, PdfError> {
    let name = fresh(run, &frame.found.added, (Category::XObject, b"XObject"))?;
    let mut text = String::new();
    write_name(&mut text, &name);
    text.push_str(" Do");
    frame
        .found
        .added
        .push((b"XObject", name, Object::Reference(made)));
    Ok(text)
}

/// A name of `category`, `key` in resources, that neither `run`'s resources nor `added` have.
fn fresh(
    run: &Run<'_, '_>,
    added: &[(&'static [u8], Vec<u8>, Object)],
    (category, key): (Category, &'static [u8]),
) -> Result<Vec<u8>, PdfError> {
    let job = run.job;
    for number in 1..=u32::MAX {
        let name = format!("Redacted{number}").into_bytes();
        let taken = added.iter().any(|entry| entry.0 == key && entry.1 == name);
        if !taken && data::named(job, &run.scope, category, &name)?.is_none() {
            return Ok(name);
        }
    }
    Err(job.limit())
}

/// The form `found` drawn: walked under its matrix (8.10.1). Where a mark meets it, its copy,
/// or none where it is left out; nothing where it stays.
fn form(
    run: &mut Run<'_, '_>,
    cx: &mut Context<'_>,
    frame: &mut Frame,
    found: &Resolved,
) -> Result<Copied, PdfError> {
    let job = run.job;
    let owner = found.value();
    let id = found.id();
    let again = id.is_some_and(|reached| run.active.iter().any(|entry| entry.0 == reached));
    if run.depth >= job::NESTING || again {
        if job.objects.strict() {
            return Err(job.limit());
        }
        job.warn("8.10.1", "content nests too deep or runs itself")?;
        // Not drawn, so left out: nothing unread stays.
        return Ok(if frame.mode == Mode::Rewrite {
            Copied::Out
        } else {
            Copied::Kept
        });
    }
    job.spend(job::NESTING_WORK)?;
    let matrix = data::matrix(job, owner, b"Matrix")?;
    let resources = data::entry(job, owner, b"Resources")?;
    let opened = data::open(job, &mut run.caches.streams, found)?;
    let (saved, scope) = (Box::new(run.state.clone()), Rc::clone(&run.scope));
    run.state.matrix = matrix.then(&run.state.matrix);
    if !matches!(*resources.value(), Object::Null) && !resources.same(&run.scope.resources) {
        run.scope = Rc::new(Scope::new(resources));
    }
    let inner = Rc::clone(&run.scope);
    let shared = Rc::ptr_eq(&inner, &scope);
    run.depth = run.depth.saturating_add(1);
    run.active.extend(id.map(|reached| (reached, "8.10.1")));
    let result = stream(run, cx, opened, frame.mode, frame.kept());
    if id.is_some() {
        let _ended: Option<(ObjectId, &str)> = run.active.pop();
    }
    run.depth = run.depth.saturating_sub(1);
    (run.state, run.scope) = (*saved, scope);
    let walked = result?;
    if shared {
        frame.found.used.extend(walked.used.iter().cloned());
    }
    if frame.mode == Mode::Measure {
        frame.found.ink.merge(walked.ink);
        return Ok(Copied::Kept);
    }
    if !walked.changed() {
        return Ok(Copied::Kept);
    }
    let reopened = data::open(job, &mut run.caches.streams, found)?;
    let bytes = emit(job, reopened, &walked, cx.stage.room(job))?;
    let own = if walked.added.is_empty() && walked.removed.is_empty() {
        None
    } else {
        Some(resources_with(job, inner.resources.value(), &walked)?)
    };
    Ok(Copied::Made(cx.stage.add(job, copied(owner, bytes, own))?))
}

/// `gs` of the `ExtGState` resource `key`: its font and line width, and its soft mask's group
/// walked as a form, in a copy where a mark meets it (8.4.5, 11.6.5.2).
fn graphics(
    run: &mut Run<'_, '_>,
    cx: &mut Context<'_>,
    frame: &mut Frame,
    key: &[u8],
) -> Result<Option<String>, PdfError> {
    let job = run.job;
    let Some(found) = data::named(job, &run.scope, Category::ExtGState, key)? else {
        job.warn("7.8.3", "a graphics state dictionary is missing")?;
        return Ok(None);
    };
    let width = data::entry(job, found.value(), b"LW")?;
    if let Some(value) = Fixed::from_object(width.value()) {
        run.state.style.width = value;
    }
    let font = data::entry(job, found.value(), b"Font")?;
    let pair = data::items(font.value()).unwrap_or_default();
    if let (2, Some(first), Some(second)) = (pair.len(), pair.first(), pair.get(1)) {
        let (chosen, size) = (data::resolve(job, first)?, data::resolve(job, second)?);
        let points = Fixed::from_object(size.value()).ok_or_else(|| job.objects.malformed())?;
        run.state.text.font = None;
        text::set_font(run, (&chosen, None), points)?;
    }
    let mask = data::entry(job, found.value(), b"SMask")?;
    let group = data::entry(job, mask.value(), b"G")?;
    let walked = if frame.mode == Mode::Rewrite && matches!(*group.value(), Object::Stream(..)) {
        form(run, cx, frame, &group)?
    } else {
        Copied::Kept
    };
    let made = match walked {
        Copied::Kept => {
            let _new: bool = frame.found.used.insert((b"ExtGState", key.to_vec()));
            return Ok(None);
        }
        Copied::Made(made) => made,
        Copied::Out => {
            frame.replaced.push((b"ExtGState", key.to_vec()));
            return Ok(Some(String::new()));
        }
    };
    frame.replaced.push((b"ExtGState", key.to_vec()));
    let mut changed_mask = mask.value().clone();
    changed_mask.set(b"G", Some(Object::Reference(made)));
    let mut state = found.value().clone();
    state.set(b"SMask", Some(changed_mask));
    let name = fresh(run, &frame.found.added, (Category::ExtGState, b"ExtGState"))?;
    let mut text = String::new();
    write_name(&mut text, &name);
    text.push_str(" gs");
    frame.found.added.push((b"ExtGState", name, state));
    Ok(Some(text))
}

/// The least box holding `area` and `more`.
pub(super) fn joined(area: Option<[Fixed; 4]>, more: [Fixed; 4]) -> [Fixed; 4] {
    let Some([left, bottom, right, top]) = area else {
        return more;
    };
    let [other_left, other_bottom, other_right, other_top] = more;
    [
        left.min(other_left),
        bottom.min(other_bottom),
        right.max(other_right),
        top.max(other_top),
    ]
}

/// A marked-content operator: whether the level it opens keeps what structure reads (14.6).
fn marked(
    run: &Run<'_, '_>,
    frame: &mut Frame,
    name: &[u8],
    operands: &[Object],
) -> Result<(), PdfError> {
    let job = run.job;
    match name {
        b"EMC" => {
            if frame.marked.pop().is_none() {
                job.warn("14.6", "EMC without BMC or BDC")?;
            }
        }
        b"BMC" => frame.marked.push(false),
        b"BDC" => {
            let given = operands.get(1).unwrap_or(&Object::Null);
            let properties = if let Some(key) = data::name(given) {
                data::named(job, &run.scope, Category::Properties, key)?
            } else if matches!(*given, Object::Dictionary(_)) {
                Some(data::resolve(job, given)?)
            } else {
                None
            };
            let kept = properties.is_some_and(|found| {
                [&b"ActualText"[..], b"Alt", b"E", b"MCID"]
                    .iter()
                    .any(|key| !matches!(*data::stored(found.value(), key), Object::Null))
            });
            frame.marked.push(kept);
        }
        _ => {}
    }
    Ok(())
}

/// A path painted, clipped or ended: `name` (8.5.3, 8.5.4).
fn painted(
    run: &Run<'_, '_>,
    cx: &Context<'_>,
    frame: &mut Frame,
    name: &[u8],
) -> Result<Option<String>, PdfError> {
    let Some(area) = frame.path.take() else {
        frame.clipping = false;
        return Ok(None);
    };
    if mem::take(&mut frame.clipping) {
        frame.clip = Some(frame.clip.map_or(area, |clip| intersected(clip, area)));
    }
    let stroked = matches!(name, b"b" | b"B" | b"b*" | b"B*" | b"s" | b"S");
    let inked = if stroked {
        widened(&run.state, area)
    } else {
        area
    };
    let shape = marks::rectangle(inked);
    if name == b"n" && frame.mode == Mode::Measure {
        return Ok(None);
    }
    if frame.reach(cx.marks, Some(&shape)) {
        touched(run, frame)?;
        return cut::path(run.job);
    }
    Ok(None)
}

/// The box both `first` and `second` hold, empty where none.
fn intersected(first: [Fixed; 4], second: [Fixed; 4]) -> [Fixed; 4] {
    let [left, bottom, right, top] = first;
    let [other_left, other_bottom, other_right, other_top] = second;
    let (low_x, low_y) = (left.max(other_left), bottom.max(other_bottom));
    [
        low_x,
        low_y,
        right.min(other_right).max(low_x),
        top.min(other_top).max(low_y),
    ]
}

/// A path construction operator `name`: its points widen the path's box (8.5.2).
fn path(
    run: &Run<'_, '_>,
    frame: &mut Frame,
    name: &[u8],
    operands: &[Object],
) -> Result<(), PdfError> {
    let job = run.job;
    let given: Vec<Fixed> = match name {
        b"m" | b"l" => numbers::<2>(job, operands)?.to_vec(),
        b"v" | b"y" => numbers::<4>(job, operands)?.to_vec(),
        b"c" => numbers::<6>(job, operands)?.to_vec(),
        b"re" => {
            let [x, y, width, height] = numbers::<4>(job, operands)?;
            let (right, top) = (x.plus(width), y.plus(height));
            vec![x, y, right, y, right, top, x, top]
        }
        _ => Vec::new(),
    };
    for &[across, down] in given.as_chunks::<2>().0 {
        let point = run.state.matrix.apply(Point { across, down });
        frame.path = Some(joined(
            frame.path,
            [point.across, point.down, point.across, point.down],
        ));
    }
    Ok(())
}

/// The resources `base` with what `found` adds and without what it leaves out (7.8.3).
pub(super) fn resources_with(
    job: &job::Job<'_>,
    base: &Object,
    found: &Found,
) -> Result<Object, PdfError> {
    let mut made = if matches!(*base, Object::Dictionary(_)) {
        base.clone()
    } else {
        Object::Dictionary(Vec::new())
    };
    let removed = found
        .removed
        .iter()
        .map(|entry| (entry.0, entry.1.clone(), None));
    let added = found
        .added
        .iter()
        .map(|entry| (entry.0, entry.1.clone(), Some(entry.2.clone())));
    for (key, name, value) in removed.chain(added) {
        let listed = job
            .overlay
            .direct(job.objects, made.get(key).unwrap_or(Object::Null))?;
        let mut category = if matches!(listed, Object::Dictionary(_)) {
            listed
        } else {
            Object::Dictionary(Vec::new())
        };
        category.set(&name, value);
        made.set(key, Some(category));
    }
    Ok(made)
}

/// A text showing operator `name` (9.4.3).
fn shown(
    run: &mut Run<'_, '_>,
    cx: &mut Context<'_>,
    frame: &mut Frame,
    name: &[u8],
    operands: &[Object],
) -> Result<Option<String>, PdfError> {
    let job = run.job;
    let Some(mut block) = frame.text.take() else {
        job.warn("9.4.1", "a text operator outside a text object")?;
        return Ok(None);
    };
    let mut before = String::new();
    if name == b"\"" {
        let [word, character] = numbers(job, operands.get(..2).unwrap_or_default())?;
        run.state.text.word = word;
        run.state.text.character = character;
        for (value, operator) in [(word, " Tw "), (character, " Tc ")] {
            marks::decimal(&mut before, value);
            before.push_str(operator);
        }
    }
    if name == b"'" || name == b"\"" {
        text::operator(run, &mut block, b"T*", &[])?;
        before.push_str("T* ");
    }
    let items = if name == b"TJ" {
        operands
            .first()
            .and_then(data::items)
            .ok_or_else(|| job.objects.malformed())?
            .to_vec()
    } else {
        operands.last().cloned().into_iter().collect()
    };
    let marks = if frame.mode == Mode::Rewrite {
        cx.marks
    } else {
        &[]
    };
    let result = glyph::show(run, cx, &mut block, &items, marks);
    frame.text = Some(block);
    let (array, ink) = result?;
    frame.found.ink.merge(ink);
    let Some(rewritten) = array else {
        return Ok(None);
    };
    touched(run, frame)?;
    before.push_str(&rewritten);
    before.push_str(" TJ");
    Ok(Some(before))
}

/// Runs the operator `name`; what it becomes, when changed.
fn step(
    run: &mut Run<'_, '_>,
    cx: &mut Context<'_>,
    frame: &mut Frame,
    operations: &mut content::Operations<'_>,
    (name, operands): (&[u8], &[Object]),
) -> Result<Option<String>, PdfError> {
    let job = run.job;
    let Some(count) = interpret::arity(name) else {
        return Ok(None);
    };
    let Some(surplus) = operands.len().checked_sub(count) else {
        job.warn("7.8.2", "an operator has too few operands")?;
        return Ok(None);
    };
    let all = matches!(name, b"sc" | b"SC" | b"scn" | b"SCN");
    let given = operands
        .get(if all { 0 } else { surplus }..)
        .unwrap_or_default();
    match name {
        b"q" => {
            // Fewer deeper, so nesting does not multiply them.
            if frame.saved.len() >= job::SAVED >> run.depth.min(6) {
                return Err(job.capped());
            }
            frame.saved.push((run.state.clone(), frame.clip));
        }
        b"Q" => {
            if let Some((saved, clip)) = frame.saved.pop() {
                (run.state, frame.clip) = (saved, clip);
            } else {
                frame.found.stray = frame.found.stray.saturating_add(1);
                job.warn("8.4.2", "Q without q")?;
            }
        }
        b"b" | b"B" | b"b*" | b"B*" | b"f" | b"F" | b"f*" | b"n" | b"s" | b"S" => {
            return painted(run, cx, frame, name);
        }
        b"c" | b"h" | b"l" | b"m" | b"re" | b"v" | b"y" => path(run, frame, name, given)?,
        b"W" | b"W*" => frame.clipping = true,
        b"BDC" | b"BMC" | b"DP" | b"EMC" | b"MP" => marked(run, frame, name, given)?,
        b"BT" => frame.text = Some(text::begin(run, (None, 0))?),
        b"ET" => frame.text = None,
        b"T*" | b"Td" | b"TD" | b"Tm" => match frame.text.as_mut() {
            Some(block) => text::operator(run, block, name, given)?,
            None => job.warn("9.4.1", "a text operator outside a text object")?,
        },
        b"'" | b"\"" | b"Tj" | b"TJ" => return shown(run, cx, frame, name, given),
        b"BI" => {
            let shape = corners(&run.state.matrix, unit());
            if !frame.reach(cx.marks, Some(&shape)) {
                return inline::pass(run, operations).map(|()| None);
            }
            touched(run, frame)?;
            return image::inline(run, (cx.marks, &mut *cx.stage), operations)?.map_or_else(
                || Ok(Some(String::new())),
                |made| drawn(run, frame, made).map(Some),
            );
        }
        b"Do" => return xobject(run, cx, frame, name_operand(job, given, 0)?),
        b"sh" => {
            let shape = frame.clip.map(marks::rectangle);
            if frame.reach(cx.marks, shape.as_deref()) {
                touched(run, frame)?;
                return cut::shading(job);
            }
        }
        b"gs" => return graphics(run, cx, frame, name_operand(job, given, 0)?),
        b"BX" | b"EX" | b"cs" | b"CS" | b"d0" | b"d1" | b"g" | b"G" | b"k" | b"K" | b"rg"
        | b"RG" | b"sc" | b"SC" | b"scn" | b"SCN" => {}
        _ => state::operator(run, &mut Vec::new(), name, given)?,
    }
    Ok(None)
}

/// Walks the content `data`, within marked content structure keeps when `inside`.
pub(super) fn stream(
    run: &mut Run<'_, '_>,
    cx: &mut Context<'_>,
    data: Box<dyn Pull + '_>,
    mode: Mode,
    inside: bool,
) -> Result<Found, PdfError> {
    let job = run.job;
    let outer = job.damaged.replace(false);
    // Boxed: content nests, and the stack is small on WebAssembly.
    let ending: Box<dyn Pull + '_> = Box::new(content::Ending::new(job, data));
    let mut operations = Box::new(content::Operations::new(ending));
    let mut frame = Box::new(Frame {
        clip: None,
        clipping: false,
        found: Found::default(),
        inside,
        marked: Vec::new(),
        mode,
        path: None,
        replaced: Vec::new(),
        saved: Vec::new(),
        text: None,
    });
    let mut operands = Vec::new();
    let mut from = 0_u64;
    loop {
        let read = operations.next(job, &mut operands);
        let Some(next) = job.tolerate(read, "7.8.2", "a content stream is damaged")? else {
            frame.found.end = Some(from);
            frame.found.damaged = true;
            break;
        };
        let Some(name) = next else {
            break;
        };
        job.spend(job::OPERATOR_WORK)?;
        let held = operations.held();
        let lent = if held > OPERANDS_LENT {
            let token = Rc::new(());
            job.lend(&token, u64::try_from(held).unwrap_or(u64::MAX))?;
            Some(token)
        } else {
            None
        };
        let stepped = step(
            run,
            cx,
            &mut frame,
            &mut operations,
            (name.bytes(), &operands),
        );
        drop(lent);
        let changed = job.tolerate(stepped, "7.8.2", "an operator or its objects are malformed")?;
        let to = operations.input().position();
        if mode == Mode::Rewrite
            && let Some(Some(text)) = changed
        {
            frame.found.edits.push(Edit { from, text, to });
        }
        from = to;
    }
    frame.found.damaged |= job.damaged.replace(outer);
    let replaced = mem::take(&mut frame.replaced);
    frame.found.removed = replaced
        .into_iter()
        .filter(|name| !frame.found.used.contains(name) && !cx.measured.contains(name))
        .collect();
    frame.found.open = frame.saved.len();
    frame.found.texting = frame.text.is_some();
    Ok(frame.found)
}

/// Content a mark meets within marked content that structure keeps.
fn touched(run: &Run<'_, '_>, frame: &Frame) -> Result<(), PdfError> {
    if frame.kept() {
        return structure::touched(run.job);
    }
    Ok(())
}

/// The unit square, where images are (8.9.4).
fn unit() -> [Fixed; 4] {
    let (none, one) = (Fixed::default(), Fixed::from_i64(1));
    [none, none, one, one]
}

/// The box `area` of a path stroked in `state`: wider by the most a stroke reaches (8.5.3.2).
fn widened(state: &State, area: [Fixed; 4]) -> [Fixed; 4] {
    let style = &state.style;
    let [aa, bb, cc, dd, _, _] = state.matrix.terms;
    let scale = aa
        .plus(cc)
        .max(bb.plus(dd))
        .max(Fixed::default().minus(aa).minus(cc));
    let reach = style
        .width
        .times(style.miter.max(Fixed::from_i64(2)))
        .times(scale.max(Fixed::from_i64(1)));
    let [left, bottom, right, top] = area;
    [
        left.minus(reach),
        bottom.minus(reach),
        right.plus(reach),
        top.plus(reach),
    ]
}

/// `Do` of the `XObject` resource `key` (8.8).
fn xobject(
    run: &mut Run<'_, '_>,
    cx: &mut Context<'_>,
    frame: &mut Frame,
    key: &[u8],
) -> Result<Option<String>, PdfError> {
    let job = run.job;
    let Some(found) = data::named(job, &run.scope, Category::XObject, key)? else {
        job.warn("7.8.3", "an XObject is missing")?;
        return Ok(None);
    };
    let kind = data::entry(job, found.value(), b"Subtype")?;
    let shape = corners(&run.state.matrix, unit());
    let copy = if matches!(*kind.value(), Object::Name(ref name) if name == b"Image")
        && frame.reach(cx.marks, Some(&shape))
    {
        touched(run, frame)?;
        image::painted(run, (cx.marks, &mut *cx.stage), &found)?.map_or(Copied::Out, Copied::Made)
    } else if matches!(*kind.value(), Object::Name(ref name) if name == b"Form") {
        form(run, cx, frame, &found)?
    } else {
        Copied::Kept
    };
    let entry = (&b"XObject"[..], key.to_vec());
    let copied = match copy {
        Copied::Kept => {
            if frame.mode == Mode::Measure {
                let _new: bool = cx.measured.insert(entry);
            } else {
                let _new: bool = frame.found.used.insert(entry);
            }
            return Ok(None);
        }
        Copied::Made(made) => made,
        Copied::Out => {
            frame.replaced.push(entry);
            return Ok(Some(String::new()));
        }
    };
    frame.replaced.push(entry);
    drawn(run, frame, copied).map(Some)
}
