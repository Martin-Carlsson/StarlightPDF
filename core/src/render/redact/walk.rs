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
use crate::render::interpret::{self, Run, content, inline, name_operand, numbers, paint, state};
use crate::render::job;
use crate::render::output::deflate::Deflate;
use crate::render::redact::cut::{self, Outline};
use crate::render::redact::marks::{self, Marks};
use crate::render::redact::shared;
use crate::render::redact::stage::Stage;
use crate::render::redact::structure::{self, Level};
use crate::render::redact::{glyph, image};
use crate::render::text::{self, Block};
use crate::syntax::write_name;

/// The bytes of an operator's operands held uncounted while it runs.
const OPERANDS_LENT: usize = 0x4000;

/// The resource categories copies replace entries of.
const REPLACED: [&[u8]; 3] = [b"ExtGState", b"Properties", b"XObject"];

/// Keys of a stream copy leaves out: its data is new (7.3.8.2).
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
    /// The objects and property lists content draws as they are.
    pub drawn: shared::Keys,
    /// The marks.
    pub marks: &'cx Marks,
    /// The `XObject` names content that is measured draws, which no copy leaves out.
    pub measured: BTreeSet<(&'static [u8], Vec<u8>)>,
    /// The `StructParents` key of the stream walked.
    pub parents: Option<i64>,
    /// Where the resources of the stream walked are.
    pub place: shared::Place,
    /// Where each glyph procedure paints, in glyph space, by its stream's number and the line
    /// width and miter limit it starts with.
    pub procedures: BTreeMap<(u32, i64, i64), Ink>,
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
    /// The `StructParent` keys of the forms a mark touched in it (14.7.5.3).
    pub elements: Vec<i64>,
    /// Where damage ended its operations, which its copy ends at.
    pub end: Option<u64>,
    /// Where it paints, when measured.
    pub ink: Ink,
    /// The property lists in resources a mark touched in it and the forms it draws.
    pub lists: Vec<shared::Key>,
    /// The marked-content levels open at its end.
    pub marked: usize,
    /// The forms and images a mark touched, their copies or none where left out, and their
    /// parent tree keys (14.7.4.4).
    pub moved: Vec<(ObjectId, Option<ObjectId>, Vec<i64>)>,
    /// The number of the last name its copy adds.
    pub named: u32,
    /// The `q` open at its end.
    pub open: usize,
    /// The resources its copy leaves out, by category and name: only used where it uses copies.
    pub removed: Vec<(&'static [u8], Vec<u8>)>,
    /// The `Q` without a `q`.
    pub stray: usize,
    /// The marked content a mark touched in it and the forms it draws: `StructParents` keys and
    /// `MCID`s.
    pub struck: Vec<(i64, i64)>,
    /// A text object is open at its end.
    pub texting: bool,
    /// Content a mark meets is in it, which was drawn within marked content structure keeps.
    pub touched: bool,
    /// The resources it uses unchanged, by category and name.
    pub used: BTreeSet<(&'static [u8], Vec<u8>)>,
}

/// What lasts as long as one stream.
struct Frame {
    /// The box of the clip; none: unclipped.
    clip: Option<[Fixed; 4]>,
    /// `W` or `W*` came: the path clips.
    clipping: bool,
    /// In the text object, a show that clips had glyphs, and one still has.
    clips: (bool, bool),
    /// What it found.
    found: Found,
    /// It runs within marked content that structure keeps.
    inside: bool,
    /// The marked-content levels open.
    marked: Vec<Level>,
    /// Whether it changes content or measures it.
    mode: Mode,
    /// The path, held to be cut.
    outline: Outline,
    /// The box of the path, in default user space.
    path: Option<[Fixed; 4]>,
    /// Where the path's construction and clipping operators are.
    pieces: Vec<(u64, u64)>,
    /// The resources it uses copies of, by category and name.
    replaced: Vec<(&'static [u8], Vec<u8>)>,
    /// The saved states and their clips.
    saved: Vec<(State, Option<[Fixed; 4]>)>,
    /// Where the operator run is.
    span: (u64, u64),
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
        self.inside || self.marked.iter().any(Level::kept)
    }

    /// Whether what paints `shape`, none being anywhere, meets a mark; measuring, it is counted.
    fn reach(
        &mut self,
        job: &job::Job<'_>,
        marks: &Marks,
        shape: Option<&[Point]>,
    ) -> Result<bool, PdfError> {
        if self.mode == Mode::Measure {
            self.found.ink.add(shape);
            return Ok(false);
        }
        shape.map_or_else(
            || Ok(!marks.list().is_empty()),
            |found| marks.meets(job, found),
        )
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

/// The stream `owner` with `data`, Flate-coded, and `resources`.
fn copied(owner: &Object, data: Vec<u8>, resources: Object) -> Object {
    let entries = data::pairs(owner).map_or_else(Vec::new, |listed| {
        listed
            .iter()
            .filter(|pair| !UNFILTERED.contains(&pair.0.as_slice()))
            .cloned()
            .collect()
    });
    let mut made = Object::Stream(entries, Data::Bytes(Arc::from(data)));
    made.set(b"Filter", Some(Object::Name(b"FlateDecode".to_vec())));
    made.set(b"Resources", Some(resources));
    made
}

/// The corners of the rectangle `corners` under `matrix`.
pub(super) fn corners(matrix: &Matrix, corners: [Fixed; 4]) -> Vec<Point> {
    marks::rectangle(corners)
        .into_iter()
        .map(|point| matrix.apply(point))
        .collect()
}

/// `data` with `found`'s edits made, cut where damage ended it, Flate-coded within `room` bytes.
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
    let (mut out, mut buffer, mut plain) = (Vec::new(), vec![0; 0x4000], Vec::new());
    let mut deflate = Deflate::new(&mut out);
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
                plain.push(b'\n');
                plain.extend_from_slice(edit.text.as_bytes());
                skipped = skipped.max(edit.to);
            }
            if at >= skipped {
                plain.push(*byte);
            }
            at = at.saturating_add(1);
        }
        deflate.write(&plain, &mut out);
        plain.clear();
        if u64::try_from(out.len()).unwrap_or(u64::MAX) > room {
            return Err(job.limit());
        }
    }
    for edit in edits {
        plain.push(b'\n');
        plain.extend_from_slice(edit.text.as_bytes());
    }
    deflate.write(&plain, &mut out);
    deflate.finish(&mut out);
    job.damaged.set(outer);
    Ok(out)
}

/// `Do` of the `XObject` `made`, under a name `frame`'s copy adds.
fn drawn(run: &Run<'_, '_>, frame: &mut Frame, made: ObjectId) -> Result<String, PdfError> {
    let name = fresh(run, &mut frame.found.named, Category::XObject)?;
    let mut text = String::new();
    write_name(&mut text, &name);
    text.push_str(" Do");
    frame
        .found
        .added
        .push((b"XObject", name, Object::Reference(made)));
    Ok(text)
}

/// A name of `category` that `run`'s resources lack, numbered past `named`, which becomes its
/// number.
fn fresh(run: &Run<'_, '_>, named: &mut u32, category: Category) -> Result<Vec<u8>, PdfError> {
    let job = run.job;
    while let Some(number) = named.checked_add(1) {
        *named = number;
        let name = format!("Redacted{number}").into_bytes();
        if data::named(job, &run.scope, category, &name)?.is_none() {
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
        if frame.mode == Mode::Measure {
            return Ok(Copied::Kept);
        }
        // Not drawn, so left out: nothing unread stays.
        if let Some(reached) = id {
            let keys = [
                structure::key(job, owner, b"StructParent")?,
                structure::key(job, owner, b"StructParents")?,
            ];
            frame
                .found
                .moved
                .push((reached, None, keys.into_iter().flatten().collect()));
        }
        return Ok(Copied::Out);
    }
    job.spend(job::NESTING_WORK)?;
    let matrix = data::matrix(job, owner, b"Matrix")?;
    let resources = data::entry(job, owner, b"Resources")?;
    let own_parents = structure::key(job, owner, b"StructParents")?;
    let element = structure::key(job, owner, b"StructParent")?;
    let opened = data::open(job, &mut run.caches.streams, found)?;
    let (saved, scope) = (Box::new(run.state.clone()), Rc::clone(&run.scope));
    run.state.matrix = matrix.then(&run.state.matrix);
    let place = cx.place;
    if !matches!(*resources.value(), Object::Null) && !resources.same(&run.scope.resources) {
        cx.place = shared::own_place(&resources, id);
        run.scope = Rc::new(Scope::new(resources));
    }
    let inner = Rc::clone(&run.scope);
    let shared = Rc::ptr_eq(&inner, &scope);
    run.depth = run.depth.saturating_add(1);
    run.active.extend(id.map(|reached| (reached, "8.10.1")));
    let parents = mem::replace(&mut cx.parents, own_parents);
    let result = stream(run, cx, opened, frame.mode, frame.kept());
    (cx.parents, cx.place) = (parents, place);
    if id.is_some() {
        let _ended: Option<(ObjectId, &str)> = run.active.pop();
    }
    run.depth = run.depth.saturating_sub(1);
    (run.state, run.scope) = (*saved, scope);
    let walked = result?;
    frame.found.struck.extend_from_slice(&walked.struck);
    frame.found.elements.extend_from_slice(&walked.elements);
    frame.found.lists.extend(walked.lists.iter().cloned());
    frame.found.moved.extend(walked.moved.iter().cloned());
    if walked.touched {
        touched(run, frame)?;
    }
    if shared {
        frame.found.used.extend(walked.used.iter().cloned());
        frame.replaced.extend(walked.removed.iter().cloned());
    }
    if frame.mode == Mode::Measure {
        frame.found.ink.merge(walked.ink);
        return Ok(Copied::Kept);
    }
    if !walked.changed() {
        return Ok(Copied::Kept);
    }
    frame.found.elements.extend(element);
    let reopened = data::open(job, &mut run.caches.streams, found)?;
    let bytes = emit(job, reopened, &walked, cx.stage.room(job))?;
    let own = resources_with(run, (&inner, &[]), &walked, &cx.measured)?;
    let made = cx.stage.add(job, copied(owner, bytes, own))?;
    if let Some(reached) = id {
        let keys = element.into_iter().chain(own_parents).collect();
        frame.found.moved.push((reached, Some(made), keys));
    }
    Ok(Copied::Made(made))
}

/// `gs` of the `ExtGState` resource `key`: its parameters, and its soft mask's group walked as a
/// form, in a copy where a mark meets it or without the mask where the group cannot be walked
/// (8.4.5, 11.6.5.2).
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
    // Each as the renderer sets it, but the soft mask, which is walked.
    let entries = data::pairs(found.value()).ok_or_else(|| job.objects.malformed())?;
    for pair in entries.iter().filter(|pair| pair.0 != b"SMask") {
        job.spend(1)?;
        let done = state::parameter(run, found.id(), &pair.0, &pair.1);
        let _set: Option<()> =
            job.tolerate(done, "8.4.5", "a graphics state parameter is malformed")?;
    }
    let mask = data::entry(job, found.value(), b"SMask")?;
    let group = data::entry(job, mask.value(), b"G")?;
    let walked = if frame.mode == Mode::Rewrite && matches!(*group.value(), Object::Stream(..)) {
        // As the renderer, which then has no soft mask.
        let done = form(run, cx, frame, &group);
        job.tolerate(done, "8.4.5", "a graphics state parameter is malformed")?
            .unwrap_or(Copied::Out)
    } else {
        Copied::Kept
    };
    let changed_mask = match walked {
        Copied::Kept => {
            cx.drawn.extend(shared::referenced(job, &found)?);
            let _new: bool = frame.found.used.insert((b"ExtGState", key.to_vec()));
            return Ok(None);
        }
        Copied::Made(made) => {
            let mut copy = mask.value().clone();
            copy.set(b"G", Some(Object::Reference(made)));
            copy
        }
        Copied::Out => Object::Name(b"None".to_vec()),
    };
    frame.replaced.push((b"ExtGState", key.to_vec()));
    let mut state = found.value().clone();
    state.set(b"SMask", Some(changed_mask));
    let name = fresh(run, &mut frame.found.named, Category::ExtGState)?;
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

/// The marked-content level `level` closed: where content a mark meets is in it, its property
/// list without its words, and its `MCID` struck (14.6, 14.7.4.4, 14.9.3 to 14.9.5).
fn ended(
    run: &Run<'_, '_>,
    cx: &mut Context<'_>,
    frame: &mut Frame,
    level: Level,
) -> Result<(), PdfError> {
    let listed = match (level.name.as_deref(), level.properties.as_ref()) {
        (Some(key), Some(list)) => listed(run, cx, key, list)?,
        _ => None,
    };
    let named = level.name.map(|key| (&b"Properties"[..], key));
    let rewritten = level.properties.filter(|_list| level.touched);
    if level.touched
        && let (Some(key), Some(id)) = (cx.parents, level.id)
    {
        frame.found.struck.push((key, id));
    }
    let Some(list) = rewritten.filter(|_list| frame.mode == Mode::Rewrite) else {
        let _new: Option<bool> = named.map(|entry| frame.found.used.insert(entry));
        cx.drawn.extend(listed);
        return Ok(());
    };
    frame.found.lists.extend(listed);
    let mut text = String::new();
    write_name(&mut text, &level.tag);
    text.push(' ');
    let made = structure::stripped(&list);
    if let Some(entry) = named {
        frame.replaced.push(entry);
        let name = fresh(run, &mut frame.found.named, Category::Properties)?;
        write_name(&mut text, &name);
        frame.found.added.push((b"Properties", name, made));
    } else {
        made.syntax(&mut text);
    }
    text.push_str(" BDC");
    let (from, to) = level.span;
    frame.found.edits.push(Edit { from, text, to });
    Ok(())
}

/// The property list `name`, `list`, of the resources walked.
fn listed(
    run: &Run<'_, '_>,
    cx: &Context<'_>,
    name: &[u8],
    list: &Resolved,
) -> Result<Option<shared::Key>, PdfError> {
    let lists = data::entry(run.job, run.scope.resources.value(), b"Properties")?;
    Ok(shared::list(&lists, cx.place, (name, list.id())))
}

/// A marked-content operator (14.6).
fn marked(
    run: &Run<'_, '_>,
    cx: &mut Context<'_>,
    frame: &mut Frame,
    name: &[u8],
    operands: &[Object],
) -> Result<(), PdfError> {
    match name {
        b"EMC" => match frame.marked.pop() {
            Some(level) => ended(run, cx, frame, level)?,
            None => run.job.warn("14.6", "EMC without BMC or BDC")?,
        },
        b"DP" => {
            if let Some(key) = operands.get(1).and_then(data::name) {
                let _new: bool = frame.found.used.insert((b"Properties", key.to_vec()));
                if let Some(list) = data::named(run.job, &run.scope, Category::Properties, key)? {
                    cx.drawn.extend(listed(run, cx, key, &list)?);
                }
            }
        }
        b"BDC" | b"BMC" => {
            // As deep as states are saved.
            if frame.marked.len() >= job::SAVED {
                return Err(run.job.limit());
            }
            let level = structure::level(run, name, operands, frame.span)?;
            frame.marked.push(level);
        }
        _ => {}
    }
    Ok(())
}

/// A path painted, clipped or ended: `name` (8.5.3, 8.5.4).
fn painted(
    run: &mut Run<'_, '_>,
    cx: &Context<'_>,
    frame: &mut Frame,
    name: &[u8],
) -> Result<Option<String>, PdfError> {
    let outline = mem::take(&mut frame.outline);
    let pieces = mem::take(&mut frame.pieces);
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
    // Under a singular matrix only a stroke paints, as a hairline (8.4.4).
    let singular = !stroked && run.state.matrix.inverse().is_none();
    if (name == b"n" && frame.mode == Mode::Measure) || singular {
        return Ok(None);
    }
    if !frame.reach(run.job, cx.marks, Some(&shape))? {
        return Ok(None);
    }
    if name != b"n" {
        touched(run, frame)?;
        let filled = !matches!(name, b"S" | b"s");
        let paints = [
            filled.then(|| run.state.fill.clone()),
            stroked.then(|| run.state.stroke.clone()),
        ];
        for paint in paints.iter().flatten() {
            cut::cell(run, paint)?;
        }
    }
    let text = cut::path(run, (cx.marks, &shape), outline, name)?;
    for (from, to) in pieces {
        frame.found.edits.push(Edit {
            from,
            text: String::new(),
            to,
        });
    }
    Ok(Some(text))
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
    if frame.mode == Mode::Rewrite {
        frame.outline.add(job, &run.state.matrix, name, operands)?;
        if frame.outline.holds() {
            frame.pieces.push(frame.span);
        }
    }
    for &[across, down] in given.as_chunks::<2>().0 {
        let point = run.state.matrix.apply(Point { across, down });
        frame.path = Some(joined(
            frame.path,
            [point.across, point.down, point.across, point.down],
        ));
    }
    Ok(())
}

/// The resources of `scope` with what `found` adds, of the categories copies replace keeping
/// only what it uses unchanged, `measured` holds, or the appearances `shown` or content of
/// `scope` without resources of its own names (7.8.3, 8.7.3.1, 9.6.4, 12.5.5).
pub(super) fn resources_with(
    run: &mut Run<'_, '_>,
    (scope, shown): (&Rc<Scope>, &[shared::Shown]),
    found: &Found,
    measured: &BTreeSet<(&'static [u8], Vec<u8>)>,
) -> Result<Object, PdfError> {
    let job = run.job;
    let base = scope.resources.value();
    let mut made = if matches!(*base, Object::Dictionary(_)) {
        base.clone()
    } else {
        Object::Dictionary(Vec::new())
    };
    let direct = |owner: &Object, key: &[u8]| {
        job.overlay
            .direct(job.objects, owner.get(key).unwrap_or(Object::Null))
    };
    let mut dropped = BTreeSet::new();
    for key in REPLACED {
        let listed = direct(&made, key)?;
        for entry in data::pairs(&listed).map_or(&[][..], Vec::as_slice) {
            let named = (key, entry.0.clone());
            if !found.used.contains(&named) && !measured.contains(&named) {
                let _new: bool = dropped.insert(named);
            }
        }
    }
    if !dropped.is_empty() {
        let damaged = job.damaged.get();
        let outer = mem::replace(&mut run.scope, Rc::clone(scope));
        let listed: Vec<_> = dropped.iter().cloned().collect();
        let result = shared::inheriting(job, base).and_then(|mut all| {
            all.extend_from_slice(shown);
            shared::drawn(run, (&all, &listed))
        });
        run.scope = outer;
        job.damaged.set(damaged);
        for entry in result? {
            let _was: bool = dropped.remove(&entry);
        }
    }
    for key in REPLACED {
        let listed = direct(&made, key)?;
        let Some(entries) = data::pairs(&listed) else {
            continue;
        };
        let kept = entries
            .iter()
            .filter(|entry| !dropped.contains(&(key, entry.0.clone())))
            .cloned()
            .collect();
        made.set(key, Some(Object::Dictionary(kept)));
    }
    for &(key, ref name, ref value) in &found.added {
        let mut category = direct(&made, key)?;
        if !matches!(category, Object::Dictionary(_)) {
            category = Object::Dictionary(Vec::new());
        }
        category.set(name, Some(value.clone()));
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
        if frame.mode == Mode::Measure {
            return Ok(None);
        }
        // Drawn nowhere here, and where others would draw it is not known.
        touched(run, frame)?;
        return Ok(Some(String::new()));
    };
    let result = showing(run, cx, frame, &mut block, (name, operands));
    frame.text = Some(block);
    result
}

/// A text showing operator `name` within the text object `block`, as the renderer runs it.
fn showing(
    run: &mut Run<'_, '_>,
    cx: &mut Context<'_>,
    frame: &mut Frame,
    block: &mut Block,
    (name, operands): (&[u8], &[Object]),
) -> Result<Option<String>, PdfError> {
    let job = run.job;
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
        text::operator(run, block, b"T*", &[])?;
        before.push_str("T* ");
    }
    let items = if name == b"TJ" {
        operands
            .first()
            .and_then(data::items)
            .ok_or_else(|| job.objects.malformed())?
            .to_vec()
    } else {
        let Some(string) = operands.last().filter(|item| data::text(item).is_some()) else {
            // What it set stays; it shows nothing.
            let malformed = Err::<(), _>(job.objects.malformed());
            let _shown: Option<()> = job.tolerate(
                malformed,
                "7.8.2",
                "an operator or its objects are malformed",
            )?;
            return Ok(Some(before));
        };
        vec![string.clone()]
    };
    let marks = (frame.mode == Mode::Rewrite).then_some(cx.marks);
    let clipping = run.state.text.mode >= 4
        && run
            .state
            .text
            .font
            .as_ref()
            .is_some_and(|font| font.procedures().is_none());
    let (array, ink, kept) = glyph::show(run, cx, block, &items, marks)?;
    frame.found.ink.merge(ink);
    if clipping
        && items
            .iter()
            .any(|item| data::text(item).is_some_and(|bytes| !bytes.is_empty()))
    {
        frame.clips = (true, frame.clips.1 || kept);
    }
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
        return Ok(Some(String::new()));
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
        b"W" | b"W*" => {
            frame.clipping = true;
            frame.outline.clip(name == b"W*");
            if frame.mode == Mode::Rewrite && frame.outline.holds() {
                frame.pieces.push(frame.span);
            }
        }
        b"BDC" | b"BMC" | b"DP" | b"EMC" | b"MP" => marked(run, cx, frame, name, given)?,
        // As the renderer, which keeps the text object open (9.4.1).
        b"BT" if frame.text.is_some() => job.warn("9.4.1", "BT within a text object")?,
        b"BT" => {
            frame.text = Some(text::begin(run, (None, 0))?);
            frame.clips = (false, false);
        }
        b"ET" => {
            frame.text = None;
            // The glyphs that clipped are gone: their clip keeps nothing (9.3.6).
            if mem::take(&mut frame.clips) == (true, false) && frame.mode == Mode::Rewrite {
                let mut text = String::from("ET");
                cut::nowhere(&mut text, &run.state.matrix);
                text.push_str(" W n");
                return Ok(Some(text));
            }
        }
        b"T*" | b"Td" | b"TD" | b"Tm" => match frame.text.as_mut() {
            Some(block) => text::operator(run, block, name, given)?,
            None => job.warn("9.4.1", "a text operator outside a text object")?,
        },
        b"'" | b"\"" | b"Tj" | b"TJ" => return shown(run, cx, frame, name, given),
        b"BI" => {
            let shape = corners(&run.state.matrix, unit());
            if !frame.reach(job, cx.marks, Some(&shape))? {
                return inline::pass(run, operations).map(|()| None);
            }
            touched(run, frame)?;
            return image::inline(run, (cx.marks.list(), &mut *cx.stage), operations)?.map_or_else(
                || Ok(Some(String::new())),
                |made| drawn(run, frame, made).map(Some),
            );
        }
        b"Do" => return xobject(run, cx, frame, name_operand(job, given, 0)?),
        b"sh" => {
            let shape = frame.clip.map(marks::rectangle);
            // Under a singular matrix it paints nothing (8.7.4.2).
            if run.state.matrix.inverse().is_some()
                && frame.reach(job, cx.marks, shape.as_deref())?
            {
                touched(run, frame)?;
                let key = name_operand(job, given, 0)?;
                return cut::shading(run, (cx.marks, shape.as_deref()), key).map(Some);
            }
        }
        b"gs" => return graphics(run, cx, frame, name_operand(job, given, 0)?),
        b"cs" | b"CS" | b"g" | b"G" | b"k" | b"K" | b"rg" | b"RG" | b"sc" | b"SC" | b"scn"
        | b"SCN" => paint::color(run, name, given)?,
        b"BX" | b"EX" | b"d0" | b"d1" => {}
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
        clips: (false, false),
        found: Found::default(),
        inside,
        marked: Vec::new(),
        mode,
        outline: Outline::default(),
        path: None,
        pieces: Vec::new(),
        replaced: Vec::new(),
        saved: Vec::new(),
        span: (0, 0),
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
        frame.span = (from, operations.input().position());
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
        // An unreadable operator is dropped: what it shows is not known.
        if mode == Mode::Rewrite
            && let Some(text) = changed.unwrap_or_else(|| Some(String::new()))
        {
            frame.found.edits.push(Edit { from, text, to });
        }
        from = to;
    }
    frame.found.damaged |= job.damaged.replace(outer);
    frame.found.marked = frame.marked.len();
    while let Some(level) = frame.marked.pop() {
        ended(run, cx, &mut frame, level)?;
    }
    frame.found.edits.sort_by_key(|edit| edit.from);
    let replaced = mem::take(&mut frame.replaced);
    frame.found.removed = replaced
        .into_iter()
        .filter(|name| !frame.found.used.contains(name) && !cx.measured.contains(name))
        .collect();
    frame.found.open = frame.saved.len();
    frame.found.texting = frame.text.is_some();
    Ok(frame.found)
}

/// Content a mark meets: the marked content around it is touched, and that around the form it
/// is in where structure keeps that.
fn touched(run: &Run<'_, '_>, frame: &mut Frame) -> Result<(), PdfError> {
    run.job
        .spend(u64::try_from(frame.marked.len()).unwrap_or(u64::MAX))?;
    for level in &mut frame.marked {
        level.touched = true;
    }
    frame.found.touched |= frame.inside;
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
    let size = |term: Fixed| term.max(Fixed::default().minus(term));
    let scale = size(aa).plus(size(cc)).max(size(bb).plus(size(dd)));
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
        && frame.reach(job, cx.marks, Some(&shape))?
    {
        touched(run, frame)?;
        let made = image::painted(run, (cx.marks.list(), &mut *cx.stage), &found)?;
        let element = structure::key(job, found.value(), b"StructParent")?;
        frame.found.elements.extend(element);
        if let Some(reached) = found.id() {
            frame
                .found
                .moved
                .push((reached, made, element.into_iter().collect()));
        }
        made.map_or(Copied::Out, Copied::Made)
    } else if matches!(*kind.value(), Object::Name(ref name) if name == b"Form") {
        form(run, cx, frame, &found)?
    } else if matches!(*kind.value(), Object::Name(ref name) if name == b"Image") {
        Copied::Kept
    } else {
        job.warn("8.8", "an XObject is neither an image nor a form")?;
        // Drawn by none, so left out where content is changed.
        if frame.mode == Mode::Measure {
            Copied::Kept
        } else {
            touched(run, frame)?;
            Copied::Out
        }
    };
    let entry = (&b"XObject"[..], key.to_vec());
    let copied = match copy {
        Copied::Kept => {
            cx.drawn.extend(shared::referenced(job, &found)?);
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
