//! Typed entries, resources and stream data (ISO 32000-2:2020, 7.3, 7.4, 7.8.3).

use alloc::collections::BTreeMap;
use alloc::rc::{Rc, Weak};
use core::cell::{OnceCell, RefCell};
use core::ops::Deref;

use crate::objects::ObjectId;
use crate::overlay::{Dictionary, Object};
use crate::pull::{self, Held, Pull};
use crate::render::fixed::Fixed;
use crate::render::geometry::Matrix;
use crate::render::job::{self, Job, Pace};
use crate::render::lent::Yielding;
use crate::render::text::Font;
use crate::render::{OPERATION, function};
use crate::stream::{self, Opened};
use crate::{ErrorCode, PdfError};

/// The entries of a stream's dictionary that decoding its data reads (ISO 32000-2:2020, 7.3.8.2).
const DECODING: [&[u8]; 7] = [
    b"DL",
    b"DecodeParms",
    b"F",
    b"FDecodeParms",
    b"FFilter",
    b"Filter",
    b"Length",
];

/// The bytes one decoding stage left open holds: its window and buffers.
const STAGE: u64 = 0xC000;

/// The bytes of an object resolved past which it counts against the decoded limit while alive.
const LENT: u64 = 0x1000;

/// What is missing.
const NULL: Object = Object::Null;

/// The most items of a colour space array: `DeviceN` has five.
pub(crate) const SPACE_ITEMS: usize = 5;

/// Work of each part of an object copied.
const PART_WORK: u64 = 16;

/// The most work of copying one object kept resolved.
const KNOWN: u64 = 4096;

/// The work of copying all that are kept.
const KNOWN_ALL: u64 = 0x1_0000;

/// Bytes counted for the bookkeeping of one stream held, beside its data.
const ENTRY: usize = 64;

/// The most bytes of data read whole that grow as they are read, by doubling; longer data is
/// measured first.
const GROWN: u64 = 0x10_0000;

/// Bytes read at a time.
const STEP: usize = 4096;

/// A category of resources (7.8.3).
#[derive(Clone, Copy)]
pub(crate) enum Category {
    /// `ColorSpace`.
    ColorSpace,
    /// `ExtGState`.
    ExtGState,
    /// `Font`.
    Font,
    /// `Pattern`.
    Pattern,
    /// `Properties`.
    Properties,
    /// `Shading`.
    Shading,
    /// `XObject`.
    XObject,
}

/// A larger object resolved, while something holds it.
struct Alive {
    /// About the bytes it holds.
    bytes: u64,
    /// The id it was reached by.
    id: Option<ObjectId>,
    /// It.
    value: Weak<Object>,
    /// The work of copying it.
    work: u64,
}

/// Small objects resolved, so that a band replayed reads and decodes none again.
#[derive(Default)]
pub(crate) struct Known {
    /// Larger ones while something holds them, so that none is read twice at once, by the same.
    alive: BTreeMap<(u32, u16), Alive>,
    /// Them and the work of copying each, by the number and generation referred to.
    held: BTreeMap<(u32, u16), (Resolved, u64)>,
    /// The work of copying all.
    size: u64,
    /// How many larger ones there are when those gone are next dropped.
    sweep: usize,
}

/// An object resolved and the id it was reached by: shared, and counted against the decoded
/// limit while alive when larger than [`LENT`]. Only [`Resolved::small`] copies it out.
#[derive(Clone)]
pub(crate) struct Resolved {
    /// About the bytes it holds.
    bytes: u64,
    /// Its id; none when direct.
    id: Option<ObjectId>,
    /// The object.
    value: Rc<Object>,
}

/// A stream's data, which checks progress as it is read.
struct Paced<'doc> {
    /// Alive while the bytes its reader holds are counted.
    _lent: Option<Rc<()>>,
    /// The data.
    data: Box<dyn Pull + 'doc>,
    /// The progress.
    pace: &'doc Pace<'doc>,
}

/// The start of a stream read ahead, then the rest of it.
struct Resumed<'doc> {
    /// Alive while what it holds is counted: its start and its decoding stages.
    _lent: Option<Rc<()>>,
    /// The start.
    head: Held<Vec<u8>>,
    /// The rest.
    rest: Box<dyn Pull + 'doc>,
}

/// The resources content sees, each category read once.
pub(crate) struct Scope {
    /// The categories read, in [`Category`]'s order.
    pub categories: [OnceCell<Resolved>; 7],
    /// Fonts given directly, by name, so that each is read once.
    pub fonts: RefCell<Vec<(Vec<u8>, Rc<Font>)>>,
    /// The resource dictionary.
    pub resources: Resolved,
}

/// What copying an object takes.
#[derive(Default)]
struct Weight {
    /// The bytes it holds.
    bytes: u64,
    /// The work.
    work: u64,
}

/// Bytes shared apart from their count, so that what watches them does not keep them.
pub(crate) struct Bytes(Vec<u8>);

/// Bytes held in parts, each allocated once.
pub(crate) struct Chunks {
    /// The bytes held.
    length: usize,
    /// The parts.
    parts: Vec<Vec<u8>>,
}

/// [`Chunks`] read in order.
pub(crate) struct Chunked {
    /// The next byte of the part.
    at: usize,
    /// The bytes.
    chunks: Rc<Chunks>,
    /// The part read.
    part: usize,
}

/// Data read to its end.
enum Read {
    /// Held, grown to at most [`GROWN`] bytes.
    Grown(Vec<u8>),
    /// Longer, measured: its size.
    Measured(usize),
}

/// Opens data again from its start.
pub(crate) type Reopen<'data> = dyn Fn() -> Result<Box<dyn Pull + 'data>, PdfError> + 'data;

/// Small streams, decoded, by object number.
#[derive(Default)]
pub(crate) struct Streams {
    /// Their data.
    held: BTreeMap<u32, Rc<[u8]>>,
    /// The bytes held.
    size: usize,
    /// Larger streams held whole for the passes after the first, kept while there is room; none
    /// where there was no room.
    whole: Yielding<BTreeMap<u32, Option<Rc<Bytes>>>>,
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Chunked {
    /// Reads `chunks` from the first.
    pub(crate) const fn new(chunks: Rc<Chunks>) -> Self {
        Self {
            at: 0,
            chunks,
            part: 0,
        }
    }
}

impl Chunks {
    /// The bytes they take.
    pub(crate) fn bytes(&self) -> u64 {
        self.parts.iter().map(|part| wide(part.capacity())).fold(
            wide(size_of_val(self.parts.as_slice())),
            u64::saturating_add,
        )
    }

    /// Them as one, shared and counted while alive.
    pub(crate) fn joined(&self, job: &Job<'_>) -> Result<Rc<Bytes>, PdfError> {
        job.fits(wide(self.length))?;
        let mut all = function::room(self.length)?;
        for part in &self.parts {
            all.extend_from_slice(part);
        }
        let shared = Rc::new(Bytes(all));
        job.lend(&shared, wide(self.length))?;
        Ok(shared)
    }
}

impl Deref for Bytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl Pull for Chunked {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let mut filled = 0;
        while let Some(part) = self.chunks.parts.get(self.part) {
            let mut rest = part.get(self.at..).unwrap_or_default();
            let into = buf.get_mut(filled..).unwrap_or_default();
            if into.is_empty() {
                break;
            }
            let count = rest.pull(into)?;
            filled = filled.saturating_add(count);
            self.at = self.at.saturating_add(count);
            if self.at >= part.len() {
                (self.part, self.at) = (self.part.saturating_add(1), 0);
            }
        }
        Ok(filled)
    }
}

impl Pull for Paced<'_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let count = self.data.pull(buf)?;
        self.pace.check()?;
        Ok(count)
    }
}

impl Pull for Resumed<'_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        match self.head.pull(buf)? {
            0 => self.rest.pull(buf),
            count => Ok(count),
        }
    }
}

impl Known {
    /// What `key` refers to and the work of copying it, when kept or still held elsewhere.
    fn find(&self, key: (u32, u16)) -> Option<(Resolved, u64)> {
        if let Some(found) = self.held.get(&key) {
            return Some(found.clone());
        }
        let entry = self.alive.get(&key)?;
        let value = entry.value.upgrade()?;
        Some((
            Resolved {
                bytes: entry.bytes,
                id: entry.id,
                value,
            },
            entry.work,
        ))
    }

    /// Keeps `found`, which `work` copies, as what `key` refers to: a small one held, a larger one
    /// while something else holds it.
    fn keep(&mut self, key: (u32, u16), found: &Resolved, work: u64) {
        if work > KNOWN {
            if self.alive.len() >= self.sweep {
                self.alive
                    .retain(|_key, entry| entry.value.strong_count() > 0);
                self.sweep = self.alive.len().saturating_mul(2).saturating_add(64);
            }
            let shared = Alive {
                bytes: found.bytes,
                id: found.id,
                value: Rc::downgrade(&found.value),
                work,
            };
            let _none: Option<Alive> = self.alive.insert(key, shared);
            return;
        }
        if self.size.saturating_add(work) > KNOWN_ALL {
            self.held.clear();
            self.size = 0;
        }
        self.size = self.size.saturating_add(work);
        let _none: Option<(Resolved, u64)> = self.held.insert(key, (found.clone(), work));
    }
}

impl Weight {
    /// Counts `count` bytes of a string, name or key.
    fn add(&mut self, count: usize) {
        let bytes = u64::try_from(count).unwrap_or(u64::MAX);
        self.bytes = self.bytes.saturating_add(bytes);
        self.work = self.work.saturating_add(bytes >> 4_u32);
    }

    /// Counts one part.
    fn part(&mut self) {
        let own = u64::try_from(size_of::<Object>()).unwrap_or(u64::MAX);
        self.bytes = self.bytes.saturating_add(own);
        self.work = self.work.saturating_add(PART_WORK);
    }
}

impl Resolved {
    /// Its id; none when direct.
    pub(crate) const fn id(&self) -> Option<ObjectId> {
        self.id
    }

    /// Whether it is `other`: the same object read, or reached by the same reference.
    pub(crate) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.value, &other.value) || (self.id.is_some() && self.id == other.id)
    }

    /// A copy of it, which counts nowhere: so only of one of at most [`LENT`] bytes; a larger one
    /// is past a cap, as [`Job::capped`] tells.
    pub(crate) fn small(&self, job: &Job<'_>) -> Result<Object, PdfError> {
        if self.bytes > LENT {
            return Err(job.capped());
        }
        Ok(Object::clone(&self.value))
    }

    /// The object.
    pub(crate) fn value(&self) -> &Object {
        &self.value
    }
}

impl Scope {
    /// The resources `resources`.
    pub(crate) fn new(resources: Resolved) -> Self {
        Self {
            categories: <[OnceCell<Resolved>; 7]>::default(),
            fonts: RefCell::default(),
            resources,
        }
    }
}

/// The resolved items of the array `key` of `owner`, at most `most` of them, each checked beside
/// those before; none when null; another object, or more items, is malformed.
pub(crate) fn array(
    job: &Job<'_>,
    owner: &Object,
    key: &[u8],
    most: usize,
) -> Result<Vec<Resolved>, PdfError> {
    let listed = entry(job, owner, key)?;
    if matches!(*listed.value, Object::Null) {
        return Ok(Vec::new());
    }
    let items = items(&listed.value).ok_or_else(|| job.objects.malformed())?;
    if items.len() > most {
        return Err(job.objects.malformed());
    }
    let mut found = Vec::new();
    let mut total = 0_u64;
    for item in items {
        let one = resolve(job, item)?;
        total = total
            .saturating_add(one.bytes)
            .saturating_add(wide(size_of::<Resolved>()));
        job.fits(total)?;
        found.push(one);
    }
    Ok(found)
}

/// The items of `value` when it is an array, borrowed.
pub(crate) fn items(value: &Object) -> Option<&[Object]> {
    match *value {
        Object::Array(ref items) => Some(items),
        Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => None,
    }
}

/// The name `value` is, borrowed.
pub(crate) fn name(value: &Object) -> Option<&[u8]> {
    match *value {
        Object::Name(ref bytes) => Some(bytes),
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => None,
    }
}

/// The string `value` is, borrowed.
pub(crate) fn text(value: &Object) -> Option<&[u8]> {
    match *value {
        Object::String(ref bytes) => Some(bytes),
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..) => None,
    }
}

/// The value of `key` of `owner` as it is stored, unresolved; null when missing.
pub(crate) fn stored<'owner>(owner: &'owner Object, key: &[u8]) -> &'owner Object {
    let listed = pairs(owner).map_or(&[][..], Vec::as_slice);
    listed
        .iter()
        .find(|pair| pair.0 == key)
        .map_or(&NULL, |pair| &pair.1)
}

/// The entries of `value` when it is a dictionary or stream, borrowed.
pub(crate) const fn pairs(value: &Object) -> Option<&Dictionary> {
    match *value {
        Object::Dictionary(ref listed) | Object::Stream(ref listed, _) => Some(listed),
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::String(_) => None,
    }
}

/// The data of `stream` up to an image filter, with that filter, its bytes counted as work; a
/// new reader at each call.
pub(crate) fn coded<'doc>(job: &Job<'doc>, stream: &Resolved) -> Result<Opened<'doc>, PdfError> {
    // Only what decoding reads is copied, not the rest of the dictionary.
    let Object::Stream(ref listed, ref shared) = *stream.value else {
        return Err(job.objects.malformed());
    };
    let open = || {
        let wanted = listed
            .iter()
            .filter(|pair| DECODING.contains(&pair.0.as_slice()))
            .cloned()
            .collect();
        let copy = copied(job, Object::Stream(wanted, shared.clone()))?;
        let Object::Stream(entries, data) = copy else {
            return Err(job.objects.malformed());
        };
        stream::opened(job.objects, &job.overlay, (entries, data))
    };
    let mut opened = match open() {
        // The room reads were told may be taken by what is kept for reuse: it gives way, once.
        Err(error) if error.code() == ErrorCode::LimitExceeded => {
            job.give_way()?;
            open()?
        }
        other => other?,
    };
    // What its reader holds whole counts while the reader is alive.
    let lent = if opened.held > 0 {
        let token = Rc::new(());
        job.lend(&token, opened.held)?;
        Some(token)
    } else {
        None
    };
    // Reading checks progress too, so that a long read can be cancelled.
    opened.data = Box::new(Paced {
        _lent: lent,
        data: opened.data,
        pace: job.pace,
    });
    Ok(opened)
}

/// `value`, just copied or read, its size taken as work, so a large object used often is bounded,
/// and checked against the decoded limit beside all that is held.
fn copied(job: &Job<'_>, value: Object) -> Result<Object, PdfError> {
    let mut weight = Weight::default();
    let same = weighed(value, &mut weight);
    job.spend(weight.work)?;
    job.fits(weight.bytes)?;
    Ok(same)
}

/// The value of `key` of `owner`, resolved; null when missing.
pub(crate) fn entry(job: &Job<'_>, owner: &Object, key: &[u8]) -> Result<Resolved, PdfError> {
    let listed = pairs(owner).map_or(&[][..], Vec::as_slice);
    job.spend(wide(listed.len()))?;
    listed
        .iter()
        .find(|pair| pair.0 == key)
        .map_or_else(|| resolved(job, Object::Null), |pair| resolve(job, &pair.1))
}

/// `data` after `spare` zeros, up to its end or one byte past `most`; whether it ended. The
/// room doubles, each counted, so that what grows is copied, at most `most` bytes.
fn gathered(
    job: &Job<'_>,
    data: &mut dyn Pull,
    most: u64,
    spare: usize,
) -> Result<(Vec<u8>, bool), PdfError> {
    let ceiling = usize::try_from(most)
        .unwrap_or(usize::MAX)
        .saturating_add(spare)
        .saturating_add(1);
    let mut bytes = function::room(spare)?;
    bytes.resize(spare, 0);
    let mut scratch = vec![0; STEP];
    loop {
        let wanted = ceiling.saturating_sub(bytes.len()).min(STEP);
        if wanted == 0 {
            return Ok((bytes, false));
        }
        let count = data.pull(scratch.get_mut(..wanted).unwrap_or_default())?;
        if count == 0 {
            return Ok((bytes, true));
        }
        let (length, held) = (bytes.len().saturating_add(count), bytes.capacity());
        if length > held {
            let grown = held.saturating_mul(2).max(STEP).min(ceiling).max(length);
            job.fits(wide(grown))?;
            bytes
                .try_reserve_exact(grown.saturating_sub(bytes.len()))
                .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))?;
        }
        bytes.extend_from_slice(scratch.get(..count).unwrap_or_default());
    }
}

/// The matrix `key` of `owner`; the identity when missing.
pub(crate) fn matrix(job: &Job<'_>, owner: &Object, key: &[u8]) -> Result<Matrix, PdfError> {
    Ok(numbers(job, owner, key)?.map_or_else(Matrix::identity, |terms| Matrix { terms }))
}

/// The resource `name` of `category`; `None` when missing.
pub(crate) fn named(
    job: &Job<'_>,
    scope: &Scope,
    category: Category,
    name: &[u8],
) -> Result<Option<Resolved>, PdfError> {
    let [
        spaces,
        states,
        fonts,
        patterns,
        properties,
        shadings,
        xobjects,
    ] = scope.categories.each_ref();
    let (cell, key): (&OnceCell<Resolved>, &[u8]) = match category {
        Category::ColorSpace => (spaces, b"ColorSpace"),
        Category::ExtGState => (states, b"ExtGState"),
        Category::Font => (fonts, b"Font"),
        Category::Pattern => (patterns, b"Pattern"),
        Category::Properties => (properties, b"Properties"),
        Category::Shading => (shadings, b"Shading"),
        Category::XObject => (xobjects, b"XObject"),
    };
    let held = if let Some(found) = cell.get() {
        found
    } else {
        let found = entry(job, &scope.resources.value, key)?;
        cell.get_or_init(|| found)
    };
    let entries = pairs(&held.value).map_or(&[][..], Vec::as_slice);
    job.spend(wide(entries.len()))?;
    let Some(listed) = entries.iter().find(|listed| listed.0 == name) else {
        return Ok(None);
    };
    let found = if matches!(category, Category::ColorSpace) {
        space(job, &listed.1)?
    } else {
        resolve(job, &listed.1)?
    };
    Ok((!matches!(*found.value, Object::Null)).then_some(found))
}

/// `value`, reached by `id`, shared and counted as what is resolved is.
pub(crate) fn made(
    job: &Job<'_>,
    value: Object,
    id: Option<ObjectId>,
) -> Result<Resolved, PdfError> {
    held(job, value, id).map(|found| found.0)
}

/// What `found` refers to, when it is a reference; else it again.
pub(crate) fn followed(job: &Job<'_>, found: &Resolved) -> Result<Resolved, PdfError> {
    if let Object::Reference(id) = *found.value {
        resolved(job, Object::Reference(id))
    } else {
        Ok(found.clone())
    }
}

/// What refers to `id`.
pub(crate) fn reference(id: ObjectId) -> Resolved {
    Resolved {
        bytes: wide(size_of::<Object>()),
        id: Some(id),
        value: Rc::new(Object::Reference(id)),
    }
}

/// `value`, reached by `id`, and the work of copying it, which is taken; a large one counts
/// against the decoded limit while alive.
fn held(job: &Job<'_>, value: Object, id: Option<ObjectId>) -> Result<(Resolved, u64), PdfError> {
    let mut weight = Weight::default();
    let shared = Rc::new(weighed(value, &mut weight));
    job.spend(weight.work)?;
    job.reserve(weight.bytes)?;
    if weight.bytes > LENT {
        job.lend(&shared, weight.bytes)?;
    }
    let found = Resolved {
        bytes: weight.bytes,
        id,
        value: shared,
    };
    Ok((found, weight.work))
}

/// The `COUNT` numbers of the array `key` of `owner`; `None` when null.
pub(crate) fn numbers<const COUNT: usize>(
    job: &Job<'_>,
    owner: &Object,
    key: &[u8],
) -> Result<Option<[Fixed; COUNT]>, PdfError> {
    let listed = entry(job, owner, key)?;
    if matches!(*listed.value, Object::Null) {
        return Ok(None);
    }
    let items = items(&listed.value)
        .filter(|items| items.len() == COUNT)
        .ok_or_else(|| job.objects.malformed())?;
    let mut found = [Fixed::default(); COUNT];
    for (slot, item) in found.iter_mut().zip(items) {
        *slot = Fixed::from_object(&resolve(job, item)?.value)
            .ok_or_else(|| job.objects.malformed())?;
    }
    Ok(Some(found))
}

/// The decoded data of `stream`, its bytes counted as work; small streams are cached by object
/// number, so forms and glyph procedures are not inflated again.
pub(crate) fn open<'doc>(
    job: &Job<'doc>,
    streams: &mut Streams,
    stream: &Resolved,
) -> Result<Box<dyn Pull + 'doc>, PdfError> {
    let number = stream.id.map(|id| id.number);
    if let Some(held) = number.and_then(|key| streams.held.get(&key)) {
        job.spend(u64::try_from(held.len()).unwrap_or(u64::MAX))?;
        return Ok(Box::new(Held::new(Rc::clone(held))));
    }
    let mut found = coded(job, stream)?;
    if found.image.is_some() {
        return Err(job.objects.malformed());
    }
    let Some(key) = number else {
        let lent = staged(job, stream, 0)?;
        return Ok(Box::new(Resumed {
            _lent: lent,
            head: Held::new(Vec::new()),
            rest: found.data,
        }));
    };
    let (bytes, ended) = match gathered(job, &mut found.data, job::STREAM, 0) {
        Ok(read) => read,
        // Damaged: read again, so its user keeps what comes before the damage, its stages counted.
        Err(error) if error.code() == ErrorCode::Malformed => {
            // The first reader goes before the second comes, and what it held is free again.
            drop(found);
            job.recount()?;
            let lent = staged(job, stream, 0)?;
            return Ok(Box::new(Resumed {
                _lent: lent,
                head: Held::new(Vec::new()),
                rest: coded(job, stream)?.data,
            }));
        }
        Err(error) => return Err(error),
    };
    if !ended {
        // Read on while content inside it runs, so counted.
        let lent = staged(job, stream, bytes.capacity())?;
        return Ok(Box::new(Resumed {
            _lent: lent,
            head: Held::new(bytes),
            rest: found.data,
        }));
    }
    // Each entry counts its bookkeeping too, so that many empty streams are bounded as well.
    let size = bytes.len().saturating_add(ENTRY);
    if streams.size.saturating_add(size) > job::STREAMS {
        streams.held.clear();
        streams.size = 0;
    }
    // A copy of at most `job::STREAM` bytes.
    let shared: Rc<[u8]> = Rc::from(bytes);
    streams.size = streams.size.saturating_add(size);
    let _none: Option<Rc<[u8]>> = streams.held.insert(key, Rc::clone(&shared));
    Ok(Box::new(Held::new(shared)))
}

/// A token lent for what a reader of `stream` left open holds: `head` bytes read ahead and a
/// window and buffers for each of its filters.
fn staged(job: &Job<'_>, stream: &Resolved, head: usize) -> Result<Option<Rc<()>>, PdfError> {
    let filters = entry(job, &stream.value, b"Filter")?;
    let count = items(&filters.value).map_or_else(
        || usize::from(name(&filters.value).is_some()),
        <[Object]>::len,
    );
    let bytes = wide(head).saturating_add(wide(count).saturating_mul(STAGE));
    if bytes == 0 {
        return Ok(None);
    }
    let token = Rc::new(());
    job.lend(&token, bytes)?;
    Ok(Some(token))
}

/// The data of `stream`, held whole and lent for the render's later passes where it is over
/// [`job::STREAM`] bytes and fits beside all else held and `beside` bytes more, so that each band
/// reads it without decoding it again; read anew otherwise.
pub(crate) fn kept<'doc>(
    job: &Job<'doc>,
    streams: &mut Streams,
    stream: &Resolved,
    beside: u64,
) -> Result<Box<dyn Pull + 'doc>, PdfError> {
    let Some(number) = stream.id.map(|id| id.number) else {
        return open(job, streams, stream);
    };
    let known = streams.whole.entries().get(&number).cloned();
    match known {
        Some(Some(held)) => {
            job.spend(wide(held.0.len()) >> 2_u32)?;
            return Ok(Box::new(Held::new(held)));
        }
        Some(None) => return Ok(coded(job, stream)?.data),
        None => {}
    }
    let again = || {
        let found = coded(job, stream)?;
        if found.image.is_some() {
            return Err(job.objects.malformed());
        }
        Ok(found.data)
    };
    let read = all(job, &mut again()?, &again, (job.objects.decoded_limit(), 0));
    // Kept for the bands, given up when what comes later needs the room. Without room, or damaged,
    // it is read anew in every pass, so that its user keeps what comes before the damage.
    let kept = read.and_then(|bytes| {
        let held = Rc::new(Bytes(bytes));
        if wide(held.0.len()) <= job::STREAM {
            return Ok(Err(held));
        }
        let size = wide(held.0.capacity().saturating_add(ENTRY));
        job.fits(size.saturating_add(beside))?;
        job.lend(&held, size)?;
        Ok(Ok(held))
    });
    let entry = match kept {
        // Small: read again is cheap, and recording it would cost as much.
        Ok(Err(small)) => return Ok(Box::new(Held::new(small))),
        Ok(Ok(held)) => Some(held),
        // Damaged: read again in each pass, as small data is, recording nothing.
        Err(error) if error.code() == ErrorCode::Malformed => return Ok(coded(job, stream)?.data),
        Err(error) if error.code() == ErrorCode::LimitExceeded => None,
        Err(error) => return Err(error),
    };
    streams.whole.offer(job);
    let _none: Option<Option<Rc<Bytes>>> =
        streams.whole.entries_mut().insert(number, entry.clone());
    match entry {
        Some(held) => Ok(Box::new(Held::new(held))),
        None => Ok(coded(job, stream)?.data),
    }
}

/// The colour space `value`, followed through references; a stored array too long for one is
/// [`ErrorCode::Malformed`] before it is read (ISO 32000-2:2020, 8.6.3).
pub(crate) fn space(job: &Job<'_>, value: &Object) -> Result<Resolved, PdfError> {
    if let Object::Reference(id) = *value
        && !job.overlay.is_changed()
        && job.objects.longer(id, SPACE_ITEMS)?
    {
        return Err(job.objects.malformed());
    }
    resolve(job, value)
}

/// `value`, followed through references, which count as work; a direct one is checked against the
/// decoded limit before it is copied.
pub(crate) fn resolve(job: &Job<'_>, value: &Object) -> Result<Resolved, PdfError> {
    if let Object::Reference(id) = *value {
        return resolved(job, Object::Reference(id));
    }
    job.fits(size(value))?;
    resolved(job, value.clone())
}

/// About the bytes `value` holds, as copying it counts them; a stream's bytes are shared, not
/// copied.
pub(crate) fn size(value: &Object) -> u64 {
    let mut pending = vec![value];
    let mut total = 0_u64;
    while let Some(item) = pending.pop() {
        total = total.saturating_add(wide(size_of::<Object>()));
        if let Some(parts) = items(item) {
            pending.extend(parts);
        } else if let Some(listed) = pairs(item) {
            for pair in listed {
                total = total.saturating_add(wide(pair.0.len()));
                pending.push(&pair.1);
            }
        } else {
            let own = name(item).or_else(|| text(item)).map_or(0, <[u8]>::len);
            total = total.saturating_add(wide(own));
        }
    }
    total
}

/// `value`, followed through references, which count as work.
pub(crate) fn resolved(job: &Job<'_>, value: Object) -> Result<Resolved, PdfError> {
    let Object::Reference(first) = value else {
        // Lent like one reached by reference.
        return held(job, value, None).map(|found| found.0);
    };
    let key = (first.number, first.generation);
    let kept = job.known.borrow().find(key);
    if let Some((found, work)) = kept {
        // Its user may copy it again.
        job.spend(work.saturating_add(PART_WORK))?;
        return Ok(found);
    }
    job.spend(job::RESOLVE_WORK)?;
    let (object, id) = match job.overlay.ended(job.objects, Object::Reference(first)) {
        // Past the room that what caches keep for reuse took: they give way, and it is read again.
        Err(error) if error.code() == ErrorCode::LimitExceeded => {
            job.give_way()?;
            job.overlay.ended(job.objects, Object::Reference(first))?
        }
        read => read?,
    };
    let (found, work) = held(job, object, id)?;
    job.known.borrow_mut().keep(key, &found, work);
    Ok(found)
}

/// `value` again, the work and bytes of copying it added to `weight`; it nests as far as objects
/// are read.
fn weighed(value: Object, weight: &mut Weight) -> Object {
    weight.part();
    let pairs = |listed: Dictionary, total: &mut Weight| -> Dictionary {
        listed
            .into_iter()
            .map(|(key, item)| {
                total.add(key.capacity());
                (key, weighed(item, total))
            })
            .collect()
    };
    match value {
        Object::Array(items) => Object::Array(
            items
                .into_iter()
                .map(|item| weighed(item, weight))
                .collect(),
        ),
        Object::Dictionary(listed) => Object::Dictionary(pairs(listed, weight)),
        Object::Name(name) => {
            weight.add(name.capacity());
            Object::Name(name)
        }
        // A stream's bytes are shared, not copied.
        Object::Stream(listed, data) => Object::Stream(pairs(listed, weight), data),
        Object::String(text) => {
            weight.add(text.capacity());
            Object::String(text)
        }
        other @ (Object::Boolean(_)
        | Object::Integer(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)) => other,
    }
}

/// At most `most` bytes of `stream`'s data up to an image filter, charged to the decoded limit,
/// and whether that is all.
pub(crate) fn start(
    job: &Job<'_>,
    stream: &Resolved,
    most: u64,
) -> Result<(Vec<u8>, bool), PdfError> {
    let (mut bytes, ended) = gathered(job, &mut coded(job, stream)?.data, most, 0)?;
    bytes.truncate(usize::try_from(most).unwrap_or(usize::MAX));
    bytes.shrink_to_fit();
    Ok((bytes, ended))
}

/// All of `data` after `spare` zeros, charged to the decoded limit; `again` opens it anew. More
/// than `most` bytes fail as [`past`] tells.
pub(crate) fn all(
    job: &Job<'_>,
    data: &mut dyn Pull,
    again: &Reopen<'_>,
    (most, spare): (u64, usize),
) -> Result<Vec<u8>, PdfError> {
    match read(job, data, (most, spare))? {
        Read::Grown(mut bytes) => {
            // Doubling left up to as much again unused: a copy of at most `GROWN` bytes.
            bytes.shrink_to_fit();
            Ok(bytes)
        }
        Read::Measured(size) => {
            let mut found = function::room(size)?;
            found.resize(size, 0);
            filled(job, again, found.get_mut(spare..).unwrap_or_default())?;
            Ok(found)
        }
    }
}

/// All of `data` in parts of at most [`GROWN`] bytes, read once, each part charged to the decoded
/// limit beside those before; more than `most` bytes fail as [`past`] tells.
pub(crate) fn chunked(job: &Job<'_>, data: &mut dyn Pull, most: u64) -> Result<Chunks, PdfError> {
    let (first, ended) = gathered(job, data, most.min(GROWN), 0)?;
    let (mut length, mut held) = (first.len(), wide(first.capacity()));
    let mut parts = vec![first];
    if ended {
        return Ok(Chunks { length, parts });
    }
    if most <= GROWN {
        return Err(past(job, most));
    }
    let size = usize::try_from(GROWN).map_err(|_error| job.limit())?;
    loop {
        job.fits(held.saturating_add(GROWN))?;
        let mut part = function::room(size)?;
        part.resize(size, 0);
        let count = pull::fill(data, &mut part)?;
        if count == 0 {
            break;
        }
        part.truncate(count);
        (length, held) = (length.saturating_add(count), held.saturating_add(GROWN));
        if wide(length) > most {
            return Err(past(job, most));
        }
        parts.push(part);
        if count < size {
            break;
        }
    }
    Ok(Chunks { length, parts })
}

/// Fills `room` from the start of the data `again` opens.
fn filled(job: &Job<'_>, again: &Reopen<'_>, room: &mut [u8]) -> Result<(), PdfError> {
    if pull::fill(&mut again()?, room)? < room.len() {
        // It was longer when measured.
        return Err(job.objects.malformed());
    }
    Ok(())
}

/// The error of data longer than `most` bytes: [`ErrorCode::LimitExceeded`], or below the decoded
/// limit, where `most` is a cap of this implementation, [`Job::capped`]'s.
fn past(job: &Job<'_>, most: u64) -> PdfError {
    if most < job.objects.decoded_limit() {
        job.capped()
    } else {
        job.limit()
    }
}

/// All of `data`, read to its end; past [`GROWN`] bytes measured, its size then checked.
fn read(job: &Job<'_>, data: &mut dyn Pull, (most, spare): (u64, usize)) -> Result<Read, PdfError> {
    let (head, ended) = gathered(job, data, most.min(GROWN), spare)?;
    if ended {
        return Ok(Read::Grown(head));
    }
    if most <= GROWN {
        return Err(past(job, most));
    }
    // Growing would copy it and hold both: it is measured, then read again into room of its size.
    let mut total = wide(head.len().saturating_sub(spare));
    drop(head);
    let mut scratch = vec![0; STEP];
    loop {
        let count = data.pull(&mut scratch)?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(wide(count));
        if total > most {
            return Err(past(job, most));
        }
    }
    let size = usize::try_from(total)
        .ok()
        .and_then(|found| found.checked_add(spare))
        .ok_or_else(|| job.limit())?;
    job.fits(wide(size))?;
    Ok(Read::Measured(size))
}

/// All of `data`, shared, charged to the decoded limit while alive; `again` opens it anew. More
/// than `most` bytes fail as [`past`] tells.
pub(crate) fn shared(
    job: &Job<'_>,
    data: &mut dyn Pull,
    again: &Reopen<'_>,
    most: u64,
) -> Result<Rc<Bytes>, PdfError> {
    let found = Rc::new(Bytes(all(job, data, again, (most, 0))?));
    job.lend(&found, wide(found.0.capacity()))?;
    Ok(found)
}

/// All of `stream`'s data up to an image filter after `spare` zeros, for what needs random
/// access, charged to the decoded limit; more than `most` bytes fail as [`past`] tells.
pub(crate) fn whole(
    job: &Job<'_>,
    stream: &Resolved,
    (most, spare): (u64, usize),
) -> Result<Vec<u8>, PdfError> {
    let again = || coded(job, stream).map(|found| found.data);
    all(job, &mut again()?, &again, (most, spare))
}

/// A count of bytes as the decoded limit counts them.
fn wide(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}
