//! A part of a split: a run of a document's pages under a page tree of its own, its objects
//! rewritten as read so that it reaches no other page (ISO 32000-2:2020, 7.7.3).

use alloc::collections::{BTreeSet, VecDeque};
use alloc::sync::Arc;
use core::fmt::{self, Debug, Formatter};
use std::sync::{Mutex, PoisonError};

use crate::objects::{ObjectId, XrefKind};
use crate::overlay::{Dictionary, Object, Overlay, Reading, View};
use crate::store::{Objects, Store};
use crate::syntax::{At, Token, Value};
use crate::{ErrorCode, PdfError};

/// The catalog entries a part keeps as they are (ISO 32000-2:2020, 7.7.2, 8.11.4, 14.3.2,
/// 14.11.5, 12.6.4.8).
const CARRIED: [&[u8]; 11] = [
    b"Extensions",
    b"Lang",
    b"MarkInfo",
    b"Metadata",
    b"OCProperties",
    b"OutputIntents",
    b"PageLayout",
    b"PageMode",
    b"URI",
    b"Version",
    b"ViewerPreferences",
];

/// Kids read from the source at a time.
const BATCH: usize = 64;

/// Kids of a new page tree node.
const FANOUT: u32 = 64;

/// The attributes a page inherits (ISO 32000-2:2020, 7.7.3.4).
const INHERITED: [&[u8]; 4] = [b"CropBox", b"MediaBox", b"Resources", b"Rotate"];

/// The operation errors name.
const OPERATION: &str = "Document::split";

/// The mark of a page tree node in [`Pages`].
const NODE: u32 = 1 << 31;

/// A name tree kid's `Limits`, as read (ISO 32000-2:2020, 7.9.6).
#[derive(Clone, Debug)]
enum Bounds {
    /// Its first and last key.
    Held(Vec<u8>, Vec<u8>),
    /// Missing or not two strings.
    Missing,
    /// Not read yet.
    Unread,
}

/// Where a walk of the page tree stopped: the next page's index and the kids left per level.
#[derive(Debug)]
struct Cursor {
    /// The index of the next page.
    next: u32,
    /// The kids left, the root's first.
    stack: Vec<Level>,
}

/// The kids of a page tree node left to read, a batch at a time.
#[derive(Debug)]
struct Level {
    /// Where the next batch starts in the source, until the array ends.
    at: Option<At>,
    /// Kids read and not taken.
    batch: VecDeque<ObjectId>,
    /// The most kids a node can have: the document's objects.
    room: usize,
}

/// Named destinations read last: the keys they cover, `None` all, and the entries, sorted by key
/// (ISO 32000-2:2020, 7.9.6, 12.3.2.4).
#[derive(Debug)]
struct Named {
    /// The first and last key covered.
    covers: Option<(Vec<u8>, Vec<u8>)>,
    /// Keys and values.
    entries: Vec<(Vec<u8>, Object)>,
}

/// Where a document's pages are in its page tree: 4 bytes per object.
pub(crate) struct Pages {
    /// Whether a page has annotations.
    annotated: bool,
    /// The document's catalog.
    catalog: Object,
    /// The number of pages.
    count: u32,
    /// The catalog's `Dests`, once read.
    dests: Mutex<Option<Arc<Named>>>,
    /// Per object number: a page's index plus 1, [`NODE`] with the index of the first page
    /// under it for a page tree node, else 0.
    index: Vec<u32>,
    /// The page tree node looked up last and the attributes it and its ancestors give.
    inherited: Mutex<Option<(u32, Dictionary)>>,
    /// The `Dests` name tree leaf read last.
    leaf: Mutex<Option<Arc<Named>>>,
    /// The root page tree node.
    top: ObjectId,
    /// Where the last walk stopped, which a walk from there or later goes on from, so parts
    /// taken in order read each node once.
    walked: Mutex<Option<Cursor>>,
}

/// Pages `first..end` of a document under new page tree nodes numbered from `base`.
#[derive(Debug)]
pub(crate) struct Part {
    /// The first new node's number.
    base: u32,
    /// The new node built last and its number.
    built: Mutex<Option<(u32, Object)>>,
    /// The document's catalog.
    catalog: Object,
    /// One past the part's last page.
    end: u32,
    /// The part's first page.
    first: u32,
    /// New nodes per level, the pages' parents first, the root last.
    levels: Vec<u32>,
    /// The part's pages, when [`Part::prepare`] has listed them.
    listed: Option<Vec<ObjectId>>,
    /// The document's pages.
    pages: Arc<Pages>,
    /// The destinations the strings its pages' links name name, null when none, once
    /// [`Part::prepare`] has read them.
    resolved: Option<Named>,
    /// The document's changes, as when split.
    source: Reading,
    /// The document's shared state.
    store: Arc<Store>,
    /// One more than the highest object number the source holds.
    stored: u32,
}

impl Debug for Pages {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pages")
            .field("annotated", &self.annotated)
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}

impl Level {
    /// Reads the next batch when none is left; all the kids in an object stream, whose data a
    /// later batch would decode again from its start.
    fn fill(&mut self, objects: Objects<'_>) -> Result<(), PdfError> {
        let Some(at) = self.at.filter(|_| self.batch.is_empty()) else {
            return Ok(());
        };
        let mut reader = objects.reader(at)?;
        self.at = None;
        let size = if at.packed().is_some() {
            usize::MAX
        } else {
            BATCH
        };
        while self.batch.len() < size {
            match reader.item()? {
                Token::Reference(_) if self.batch.len() >= self.room => {
                    return Err(objects.malformed());
                }
                Token::Reference(id) => {
                    self.batch
                        .try_reserve(1)
                        .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))?;
                    self.batch.push_back(id);
                }
                Token::ArrayEnd => return Ok(()),
                Token::ArrayStart
                | Token::DictionaryEnd
                | Token::DictionaryStart
                | Token::Integer(_)
                | Token::Keyword(_)
                | Token::Name(_)
                | Token::Real(_)
                | Token::String(_) => return Err(objects.malformed()),
            }
        }
        self.at = Some(At::new(reader.position(), at.packed()));
        Ok(())
    }

    /// The kids of page tree node `id` of the document of `objects` and `overlay`; read from the
    /// source a batch at a time unless the overlay holds the node (ISO 32000-2:2020, 7.7.3.2).
    fn new(objects: Objects<'_>, overlay: &Overlay, id: ObjectId) -> Result<Self, PdfError> {
        let malformed = || objects.malformed();
        let room = usize::try_from(overlay.next().max(objects.size())).unwrap_or(usize::MAX);
        if let Some((generation, node)) = overlay.get(id.number)? {
            let kids = if generation == id.generation {
                overlay.entry(objects, &node, b"Kids")?
            } else {
                Object::Null
            };
            let Object::Array(items) = kids else {
                return Err(malformed());
            };
            let batch = items
                .iter()
                .map(|item| target(item).ok_or_else(malformed))
                .collect::<Result<_, _>>()?;
            return Ok(Self {
                at: None,
                batch,
                room,
            });
        }
        let node = objects.dictionary(Some(Value::Reference(id)))?;
        let array = match objects.get(node, b"Kids")? {
            Some(Value::Reference(kids)) => objects.resolve(kids)?.array(),
            found => found.as_ref().and_then(Value::array),
        }
        .ok_or_else(malformed)?;
        let mut reader = objects.reader(array)?;
        if reader.token()? != Token::ArrayStart {
            return Err(malformed());
        }
        let mut level = Self {
            at: Some(At::new(reader.position(), array.packed())),
            batch: VecDeque::new(),
            room,
        };
        level.fill(objects)?;
        Ok(level)
    }

    /// The next kid, taken.
    fn next(&mut self, objects: Objects<'_>) -> Result<Option<ObjectId>, PdfError> {
        self.fill(objects)?;
        Ok(self.batch.pop_front())
    }

    /// The next kid, not taken.
    fn peek(&mut self, objects: Objects<'_>) -> Result<Option<ObjectId>, PdfError> {
        self.fill(objects)?;
        Ok(self.batch.front().copied())
    }
}

impl Named {
    /// The value of `key`, the first when held twice.
    fn find(&self, key: &[u8]) -> Option<Object> {
        let at = self
            .entries
            .partition_point(|entry| entry.0.as_slice() < key);
        self.entries
            .get(at)
            .filter(|entry| entry.0 == key)
            .map(|entry| entry.1.clone())
    }

    /// `entries`, sorted, covering `covers`.
    fn new(covers: Option<(Vec<u8>, Vec<u8>)>, mut entries: Vec<(Vec<u8>, Object)>) -> Self {
        entries.sort_by(|before, after| before.0.cmp(&after.0));
        Self { covers, entries }
    }
}

impl Pages {
    /// The number of pages.
    pub(crate) const fn count(&self) -> u32 {
        self.count
    }

    /// The index of page `number`, when it is a page.
    fn page(&self, number: u32) -> Option<u32> {
        let tag = self.tag(number);
        (tag < NODE).then(|| tag.checked_sub(1)).flatten()
    }

    /// The pages of the document of `objects` and `overlay`, in page tree order; a node or page
    /// reached twice is malformed (ISO 32000-2:2020, 7.7.3).
    pub(crate) fn read(objects: Objects<'_>, overlay: &Overlay) -> Result<Self, PdfError> {
        let malformed = || objects.malformed();
        let limit = || PdfError::new(ErrorCode::LimitExceeded, OPERATION);
        let trailer = Object::Dictionary(overlay.trailer(objects)?);
        let catalog = overlay.entry(objects, &trailer, b"Root")?;
        let Some(Object::Reference(top)) = catalog.get(b"Pages") else {
            return Err(malformed());
        };
        let size = objects.slots(overlay.next().max(objects.size()))?;
        // Reserved whole, so no growth holds two copies.
        let mut index = Vec::new();
        index.try_reserve_exact(size).map_err(|_error| limit())?;
        index.resize(size, 0);
        *slot(&mut index, top.number).ok_or_else(malformed)? = NODE;
        let (mut annotated, mut count) = (false, 0_u32);
        let mut stack = vec![Level::new(objects, overlay, top)?];
        let depth = usize::try_from(objects.depth_limit()).unwrap_or(usize::MAX);
        while let Some(level) = stack.last_mut() {
            let Some(id) = level.next(objects)? else {
                drop(stack.pop());
                continue;
            };
            let (node, annotations) = kind(objects, overlay, id)?;
            let mark = slot(&mut index, id.number)
                .filter(|mark| **mark == 0)
                .ok_or_else(malformed)?;
            if node {
                *mark = NODE | count;
                if stack.len() >= depth {
                    return Err(limit());
                }
                stack.push(Level::new(objects, overlay, id)?);
            } else {
                count = count
                    .checked_add(1)
                    .filter(|&next| next < NODE)
                    .ok_or_else(limit)?;
                *mark = count;
                annotated |= annotations;
            }
        }
        Ok(Self {
            annotated,
            catalog,
            count,
            dests: Mutex::new(None),
            index,
            inherited: Mutex::new(None),
            leaf: Mutex::new(None),
            top,
            walked: Mutex::new(None),
        })
    }

    /// The index of page `number`, or of the first page under page tree node `number`.
    fn start(&self, number: u32) -> Option<u32> {
        match self.tag(number) {
            0 => None,
            tag if tag >= NODE => Some(tag & !NODE),
            tag => tag.checked_sub(1),
        }
    }

    /// The entry of object `number`.
    fn tag(&self, number: u32) -> u32 {
        usize::try_from(number)
            .ok()
            .and_then(|at| self.index.get(at))
            .copied()
            .unwrap_or_default()
    }

    /// Gives `visit` the pages from `from` to before `until`, in order, reading only the page
    /// tree nodes that hold them, through `overlay`, from where `cursor` stopped when it stopped
    /// at or before `from`, and leaves `cursor` where it stops (ISO 32000-2:2020, 7.7.3.2).
    fn walk(
        &self,
        (objects, overlay): (Objects<'_>, &Overlay),
        (from, until): (u32, u32),
        cursor: &mut Option<Cursor>,
        visit: &mut dyn FnMut(ObjectId) -> Result<(), PdfError>,
    ) -> Result<(), PdfError> {
        let malformed = || objects.malformed();
        let depth = usize::try_from(objects.depth_limit()).unwrap_or(usize::MAX);
        let mut stack = match cursor.take() {
            Some(found) if found.next <= from => found.stack,
            Some(_) | None => vec![Level::new(objects, overlay, self.top)?],
        };
        while let Some(level) = stack.last_mut() {
            let Some(id) = level.next(objects)? else {
                drop(stack.pop());
                continue;
            };
            // A kid followed by one starting at or before `from` holds none of the pages.
            if level
                .peek(objects)?
                .and_then(|next| self.start(next.number))
                .is_some_and(|next| next <= from)
            {
                continue;
            }
            let start = self.start(id.number).ok_or_else(malformed)?;
            if start >= until {
                level.batch.push_front(id);
                *cursor = Some(Cursor { next: until, stack });
                return Ok(());
            }
            if self.tag(id.number) >= NODE {
                if stack.len() >= depth {
                    return Err(PdfError::new(ErrorCode::LimitExceeded, OPERATION));
                }
                stack.push(Level::new(objects, overlay, id)?);
                continue;
            }
            if start >= from {
                visit(id)?;
            }
        }
        Ok(())
    }
}

impl Part {
    /// `destination`, explicit, and whether its page is outside the part; `None` when it names
    /// no page of the document (ISO 32000-2:2020, 12.3.2).
    fn aim(
        &self,
        objects: Objects<'_>,
        destination: Object,
    ) -> Result<Option<(Object, bool)>, PdfError> {
        let mut found = self.source.direct(objects, destination)?;
        if matches!(found, Object::Name(_) | Object::String(_)) {
            found = self.named(objects, found)?;
        }
        if matches!(found, Object::Dictionary(_)) {
            found = self.source.entry(objects, &found, b"D")?;
        }
        let Object::Array(items) = found else {
            return Ok(None);
        };
        let Some(&Object::Reference(page)) = items.first() else {
            return Ok(None);
        };
        if self.pages.page(page.number).is_none() {
            return Ok(None);
        }
        Ok(Some((Object::Array(items), self.outside(page.number))))
    }

    /// The attributes the page tree node `start`, a page's parent, and its ancestors give the
    /// page (ISO 32000-2:2020, 7.7.3.4).
    fn ancestors(&self, objects: Objects<'_>, start: ObjectId) -> Result<Dictionary, PdfError> {
        let last = || {
            self.pages
                .inherited
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        };
        let cached = last()
            .as_ref()
            .filter(|entry| entry.0 == start.number)
            .map(|entry| entry.1.clone());
        if let Some(found) = cached {
            return Ok(found);
        }
        let mut found = Dictionary::new();
        let mut node = start;
        for _ in 0..=objects.depth_limit() {
            let dictionary = self.source.resolve(objects, node)?;
            for key in INHERITED {
                if !found.iter().any(|entry| entry.0 == key)
                    && let Some(value) = dictionary
                        .get(key)
                        .filter(|value| !matches!(*value, Object::Null))
                {
                    found.push((key.to_vec(), value));
                }
            }
            let Some(Object::Reference(up)) = dictionary.take(b"Parent") else {
                *last() = Some((start.number, found.clone()));
                return Ok(found);
            };
            node = up;
        }
        Err(PdfError::new(ErrorCode::LimitExceeded, OPERATION))
    }

    /// The new catalog: the page tree, the carried entries, the page labels from the part's
    /// first page and an open action into the part, one leading elsewhere left out; `MarkInfo`
    /// goes with the structure tree (ISO 32000-2:2020, 7.7.2, 12.3.2, 12.4.2, 14.7.1).
    pub(crate) fn catalog(&self) -> Result<Dictionary, PdfError> {
        let objects = self.store.objects(OPERATION);
        let mut entries = vec![
            (b"Type".to_vec(), Object::Name(b"Catalog".to_vec())),
            (b"Pages".to_vec(), reference(self.root())),
        ];
        let tagged = self
            .catalog
            .get(b"StructTreeRoot")
            .is_some_and(|tree| !matches!(tree, Object::Null));
        for key in CARRIED {
            if let Some(value) = self.catalog.get(key)
                && !(key == b"MarkInfo" && tagged)
            {
                entries.push((key.to_vec(), value));
            }
        }
        let labels = self.labels(objects)?;
        if !labels.is_empty() {
            entries.push((
                b"PageLabels".to_vec(),
                Object::Dictionary(vec![(b"Nums".to_vec(), Object::Array(labels))]),
            ));
        }
        let action = self.source.entry(objects, &self.catalog, b"OpenAction")?;
        if !matches!(action, Object::Null)
            && let Some(kept) = self.open(objects, action)?
        {
            entries.push((b"OpenAction".to_vec(), kept));
        }
        Ok(entries)
    }

    /// `object` without references to pages and page tree nodes outside the part.
    fn clean(&self, objects: Objects<'_>, object: Object) -> Result<Object, PdfError> {
        Ok(match object {
            Object::Reference(id) if self.outside(id.number) => Object::Null,
            Object::Array(items) => Object::Array(
                items
                    .into_iter()
                    .map(|item| self.clean(objects, item))
                    .collect::<Result<_, _>>()?,
            ),
            Object::Dictionary(entries) => Object::Dictionary(self.entries(objects, entries)?),
            Object::Stream(entries, data) => Object::Stream(self.entries(objects, entries)?, data),
            other @ (Object::Boolean(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::String(_)) => other,
        })
    }

    /// The catalog's `Dests`, kept once read (ISO 32000-2:2020, 12.3.2.4).
    fn dests(&self, objects: Objects<'_>) -> Result<Arc<Named>, PdfError> {
        let kept = || {
            self.pages
                .dests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        };
        let held = kept().clone();
        if let Some(found) = held {
            return Ok(found);
        }
        let entries = match self.source.entry(objects, &self.catalog, b"Dests")? {
            Object::Dictionary(entries) => entries,
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => Dictionary::new(),
        };
        let made = Arc::new(Named::new(None, entries));
        *kept() = Some(Arc::clone(&made));
        Ok(made)
    }

    /// `entries`, links rewritten, values cleaned.
    fn entries(&self, objects: Objects<'_>, entries: Dictionary) -> Result<Dictionary, PdfError> {
        let rewritten = if entries
            .iter()
            .any(|entry| matches!(entry.0.as_slice(), b"A" | b"D" | b"Dest" | b"Subtype"))
        {
            self.link(objects, entries)?
        } else {
            entries
        };
        rewritten
            .into_iter()
            .map(|(key, value)| Ok((key, self.clean(objects, value)?)))
            .collect()
    }

    /// The kid of `kids` whose `Limits` hold `key`, searched by them, and its last key; the
    /// `Limits` read kept in `limits` (ISO 32000-2:2020, 7.9.6).
    fn kid(
        &self,
        objects: Objects<'_>,
        (kids, limits): (&[Object], &mut [Bounds]),
        key: &[u8],
    ) -> Result<Option<(Object, Vec<u8>)>, PdfError> {
        let source = &*self.source;
        let (mut low, mut high) = (0, kids.len());
        while low < high {
            let middle =
                low.saturating_add(high.saturating_sub(low).checked_div(2).unwrap_or_default());
            let Some(slot) = limits.get_mut(middle) else {
                return Ok(None);
            };
            let kid = kids.get(middle).cloned().unwrap_or(Object::Null);
            if matches!(slot, Bounds::Unread) {
                let child = source.direct(objects, kid.clone())?;
                *slot = match source.entry(objects, &child, b"Limits")? {
                    Object::Array(pair) => {
                        let mut bounds = pair.into_iter();
                        match (bounds.next(), bounds.next()) {
                            (Some(Object::String(first)), Some(Object::String(last))) => {
                                Bounds::Held(first, last)
                            }
                            _ => Bounds::Missing,
                        }
                    }
                    Object::Boolean(_)
                    | Object::Dictionary(_)
                    | Object::Integer(_)
                    | Object::Name(_)
                    | Object::Null
                    | Object::Real(_)
                    | Object::Reference(_)
                    | Object::Stream(..)
                    | Object::String(_) => Bounds::Missing,
                };
            }
            let Bounds::Held(first, last) = slot.clone() else {
                return Ok(None);
            };
            if key < first.as_slice() {
                high = middle;
            } else if key > last.as_slice() {
                low = middle.saturating_add(1);
            } else {
                return Ok(Some((kid, last)));
            }
        }
        Ok(None)
    }

    /// The part's page label ranges, keys and dictionaries of a number tree's `Nums`: the range
    /// of its first page from 0, its `St` moved on, then those starting in the part
    /// (ISO 32000-2:2020, 7.9.7, 12.4.2).
    fn labels(&self, objects: Objects<'_>) -> Result<Vec<Object>, PdfError> {
        let source = &*self.source;
        let malformed = || objects.malformed();
        let mut labels = Vec::new();
        let mut stack = vec![(source.entry(objects, &self.catalog, b"PageLabels")?, 0)];
        let mut seen = BTreeSet::new();
        let mut before: Option<(i64, Object)> = None;
        let (first, end) = (i64::from(self.first), i64::from(self.end));
        while let Some((node, depth)) = stack.pop() {
            if let Object::Array(kids) = source.entry(objects, &node, b"Kids")? {
                if depth >= objects.depth_limit() {
                    return Err(PdfError::new(ErrorCode::LimitExceeded, OPERATION));
                }
                for kid in kids.into_iter().rev() {
                    let id = target(&kid).ok_or_else(malformed)?;
                    if !seen.insert(id.number) {
                        return Err(malformed());
                    }
                    stack.push((source.resolve(objects, id)?, depth.saturating_add(1)));
                }
                continue;
            }
            let Object::Array(pairs) = source.entry(objects, &node, b"Nums")? else {
                continue;
            };
            let mut items = pairs.into_iter();
            while let (Some(key), Some(value)) = (items.next(), items.next()) {
                let Object::Integer(start) = source.direct(objects, key)? else {
                    return Err(malformed());
                };
                let label = source.direct(objects, value)?;
                if start <= first {
                    before = Some((start, label));
                } else if start < end {
                    labels.push(Object::Integer(start.saturating_sub(first)));
                    labels.push(label);
                } else {
                    break;
                }
            }
        }
        if let Some((start, mut label)) = before {
            let at = match label.get(b"St") {
                Some(Object::Integer(found)) => found,
                Some(_) | None => 1,
            };
            label.set(
                b"St",
                Some(Object::Integer(
                    at.saturating_add(first.saturating_sub(start)),
                )),
            );
            labels.insert(0, label);
            labels.insert(0, Object::Integer(0));
        }
        Ok(labels)
    }

    /// The `Dests` name tree leaf that would hold `key`, its kids searched by their `Limits`,
    /// kept as the last read; `None` when none (ISO 32000-2:2020, 7.9.6, 12.3.2.4).
    fn leaf(&self, objects: Objects<'_>, key: &[u8]) -> Result<Option<Arc<Named>>, PdfError> {
        let source = &*self.source;
        let kept = self
            .pages
            .leaf
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .filter(|leaf| {
                leaf.covers
                    .as_ref()
                    .is_none_or(|covers| covers.0.as_slice() <= key && key <= covers.1.as_slice())
            });
        if kept.is_some() {
            return Ok(kept);
        }
        let names = source.entry(objects, &self.catalog, b"Names")?;
        let mut node = source.entry(objects, &names, b"Dests")?;
        let mut covers = None;
        for _ in 0..=objects.depth_limit() {
            if let Object::Array(pairs) = source.entry(objects, &node, b"Names")? {
                let mut entries = Vec::new();
                let mut items = pairs.into_iter();
                while let (Some(held), Some(value)) = (items.next(), items.next()) {
                    if let Object::String(found) = source.direct(objects, held)? {
                        entries.push((found, value));
                    }
                }
                let leaf = Arc::new(Named::new(covers, entries));
                *self
                    .pages
                    .leaf
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&leaf));
                return Ok(Some(leaf));
            }
            let Object::Array(kids) = source.entry(objects, &node, b"Kids")? else {
                return Ok(None);
            };
            let (mut low, mut high) = (0, kids.len());
            let mut below = None;
            while low < high {
                let middle =
                    low.saturating_add(high.saturating_sub(low).checked_div(2).unwrap_or_default());
                let kid = kids.get(middle).cloned().unwrap_or(Object::Null);
                let child = source.direct(objects, kid)?;
                let Object::Array(limits) = source.entry(objects, &child, b"Limits")? else {
                    return Ok(None);
                };
                let mut bounds = limits.into_iter();
                let (Some(Object::String(first)), Some(Object::String(last))) =
                    (bounds.next(), bounds.next())
                else {
                    return Ok(None);
                };
                if key < first.as_slice() {
                    high = middle;
                } else if key > last.as_slice() {
                    low = middle.saturating_add(1);
                } else {
                    below = Some((child, (first, last)));
                    break;
                }
            }
            let Some((found, limits)) = below else {
                return Ok(None);
            };
            node = found;
            covers = Some(limits);
        }
        Err(PdfError::new(ErrorCode::LimitExceeded, OPERATION))
    }

    /// `entries` with a destination named or to a page of the part made explicit, a link to a
    /// page outside it without `Dest` and `A`, a widget without its field (ISO 32000-2:2020,
    /// 12.3.2, 12.5.6.5, 12.6.4.2, 12.7.4.1).
    fn link(&self, objects: Objects<'_>, entries: Dictionary) -> Result<Dictionary, PdfError> {
        let mut object = Object::Dictionary(entries);
        if let Some(destination) = goto(&object)
            && let Some((explicit, false)) = self.aim(objects, destination)?
        {
            object.set(b"D", Some(explicit));
        }
        let lost = self.lost(objects, &object)?;
        if lost {
            object.set(b"Dest", None);
            object.set(b"A", None);
        }
        if !lost
            && let Some(destination) = object.get(b"Dest")
            && let Some((explicit, _)) = self.aim(objects, destination)?
        {
            object.set(b"Dest", Some(explicit));
        }
        if matches!(object.get(b"Subtype"), Some(Object::Name(kind)) if kind == b"Widget") {
            object.set(b"Parent", None);
        }
        let Object::Dictionary(kept) = object else {
            return Ok(Dictionary::new());
        };
        Ok(kept)
    }

    /// The level and index of new node `number`.
    fn locate(&self, number: u32) -> Option<(usize, u32)> {
        let mut offset = number.checked_sub(self.base)?;
        for (at, &nodes) in self.levels.iter().enumerate() {
            if offset < nodes {
                return Some((at.saturating_add(1), offset));
            }
            offset = offset.saturating_sub(nodes);
        }
        None
    }

    /// Whether `object`, an annotation, links to a page outside the part by its `Dest` or a
    /// `GoTo` action (ISO 32000-2:2020, 12.5.6.5, 12.6.4.2).
    fn lost(&self, objects: Objects<'_>, object: &Object) -> Result<bool, PdfError> {
        if let Some(destination) = object.get(b"Dest")
            && let Some((_, true)) = self.aim(objects, destination)?
        {
            return Ok(true);
        }
        let Some(action) = object.get(b"A") else {
            return Ok(false);
        };
        let Some(destination) = goto(&self.source.direct(objects, action)?) else {
            return Ok(false);
        };
        Ok(matches!(self.aim(objects, destination)?, Some((_, true))))
    }

    /// The destination `name` names, by the catalog's `Dests` for a name and its `Dests` name
    /// tree for a string; null when none (ISO 32000-2:2020, 7.9.6, 12.3.2.4).
    fn named(&self, objects: Objects<'_>, name: Object) -> Result<Object, PdfError> {
        let found = match name {
            Object::Name(key) => self.dests(objects)?.find(&key),
            Object::String(key) => match self.resolved.as_ref().and_then(|held| held.find(&key)) {
                Some(found) => Some(found),
                None => self.leaf(objects, &key)?.and_then(|leaf| leaf.find(&key)),
            },
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Integer(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..) => None,
        };
        self.source.direct(objects, found.unwrap_or(Object::Null))
    }

    /// The strings by which `annotation` names its destination, by `Dest` or a `GoTo` action
    /// (ISO 32000-2:2020, 12.3.2.4, 12.5.6.5, 12.6.4.2).
    fn names(&self, objects: Objects<'_>, annotation: &Object) -> Result<Vec<Vec<u8>>, PdfError> {
        let source = &*self.source;
        let action = source.entry(objects, annotation, b"A")?;
        let mut found = Vec::new();
        for destination in [annotation.get(b"Dest"), goto(&action)]
            .into_iter()
            .flatten()
        {
            if let Object::String(key) = source.direct(objects, destination)? {
                found.push(key);
            }
        }
        Ok(found)
    }

    /// A part of `pages`, those from `first` to before `end`, of the document of `source` and
    /// `store`.
    pub(crate) fn new(
        (source, store): (&Reading, &Arc<Store>),
        pages: &Arc<Pages>,
        (first, end): (u32, u32),
    ) -> Result<Self, PdfError> {
        let objects = store.objects(OPERATION);
        let mut levels = Vec::new();
        let mut below = end.saturating_sub(first);
        loop {
            let nodes = below.div_ceil(FANOUT);
            levels.push(nodes);
            if nodes <= 1 {
                break;
            }
            below = nodes;
        }
        let base = source.next();
        // Room for the nodes and the catalog.
        let _last: u32 = levels
            .iter()
            .try_fold(base, |sum, &nodes| sum.checked_add(nodes))
            .and_then(|sum| sum.checked_add(1))
            .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, OPERATION))?;
        Ok(Self {
            base,
            built: Mutex::new(None),
            catalog: pages.catalog.clone(),
            end,
            first,
            levels,
            listed: None,
            pages: Arc::clone(pages),
            resolved: None,
            source: source.again(),
            store: Arc::clone(store),
            stored: objects.size(),
        })
    }

    /// The first object number after the new nodes.
    pub(crate) fn next(&self) -> u32 {
        self.number(self.levels.len().saturating_add(1), 0)
    }

    /// The new node `index` of `level`, 1 the pages' parents: a page tree node over its share of
    /// the part's pages (ISO 32000-2:2020, 7.7.3.2).
    fn node(&self, objects: Objects<'_>, level: usize, index: u32) -> Result<Object, PdfError> {
        let limit = || PdfError::new(ErrorCode::LimitExceeded, OPERATION);
        let pages = self.end.saturating_sub(self.first);
        let span = u32::try_from(level)
            .ok()
            .and_then(|power| u64::from(FANOUT).checked_pow(power))
            .ok_or_else(limit)?;
        let low = u64::from(index).saturating_mul(span);
        let count = low
            .saturating_add(span)
            .min(u64::from(pages))
            .saturating_sub(low);
        let below = level.checked_sub(2).map_or(pages, |lower| {
            self.levels.get(lower).copied().unwrap_or_default()
        });
        let start = index.saturating_mul(FANOUT);
        let stop = start.saturating_add(FANOUT).min(below);
        let mut kids = Vec::new();
        if let Some(listed) = self.listed.as_ref().filter(|_| level == 1) {
            let range = usize::try_from(start).ok().zip(usize::try_from(stop).ok());
            kids.extend(
                range
                    .and_then(|(from, until)| listed.get(from..until))
                    .unwrap_or_default()
                    .iter()
                    .map(|&id| Object::Reference(id)),
            );
        } else if level == 1 {
            let range = (
                self.first.saturating_add(start),
                self.first.saturating_add(stop),
            );
            let mut walked = self
                .pages
                .walked
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            self.pages
                .walk((objects, &self.source), range, &mut walked, &mut |id| {
                    kids.push(Object::Reference(id));
                    Ok(())
                })?;
        } else {
            let lower = level.saturating_sub(1);
            kids.extend((start..stop).map(|kid| reference(self.number(lower, kid))));
        }
        let mut node = vec![
            (b"Type".to_vec(), Object::Name(b"Pages".to_vec())),
            (b"Kids".to_vec(), Object::Array(kids)),
            (
                b"Count".to_vec(),
                Object::Integer(i64::try_from(count).map_err(|_error| limit())?),
            ),
        ];
        if level < self.levels.len() {
            let parent = index.checked_div(FANOUT).unwrap_or_default();
            node.push((
                b"Parent".to_vec(),
                reference(self.number(level.saturating_add(1), parent)),
            ));
        }
        Ok(Object::Dictionary(node))
    }

    /// The number of the new node `index` of `level`, 1 the pages' parents.
    fn number(&self, level: usize, index: u32) -> u32 {
        self.levels
            .get(..level.saturating_sub(1))
            .unwrap_or_default()
            .iter()
            .fold(self.base, |sum, &nodes| sum.saturating_add(nodes))
            .saturating_add(index)
    }

    /// The open action `action` when it leads to a page of the part: a destination made
    /// explicit or a `GoTo` action to one, without a next action (ISO 32000-2:2020, 12.3.2,
    /// 12.6.2, 12.6.4.2).
    fn open(&self, objects: Objects<'_>, action: Object) -> Result<Option<Object>, PdfError> {
        let Object::Dictionary(_) = action else {
            return Ok(match self.aim(objects, action)? {
                Some((explicit, false)) => Some(explicit),
                Some((_, true)) | None => None,
            });
        };
        let Some(destination) = alone(&action) else {
            return Ok(None);
        };
        Ok(match self.aim(objects, destination)? {
            Some((explicit, false)) => Some(Object::Dictionary(vec![
                (b"S".to_vec(), Object::Name(b"GoTo".to_vec())),
                (b"D".to_vec(), explicit),
            ])),
            Some((_, true)) | None => None,
        })
    }

    /// Whether object `number` is a page or page tree node of the document outside the part.
    fn outside(&self, number: u32) -> bool {
        self.pages.tag(number) != 0 && self.position(number).is_none()
    }

    /// `object`, page `at` of the part: its new parent, the attributes it inherited its own, its
    /// article beads left out (ISO 32000-2:2020, 7.7.3.3, 7.7.3.4, 12.4.3).
    fn page(&self, objects: Objects<'_>, object: Object, at: u32) -> Result<Object, PdfError> {
        let mut entries = match object {
            Object::Dictionary(entries) => entries,
            other @ (Object::Array(_)
            | Object::Boolean(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_)) => return Ok(other),
        };
        let parent = entries
            .iter()
            .find(|entry| entry.0 == b"Parent")
            .and_then(|entry| target(&entry.1));
        entries.retain(|entry| {
            !(matches!(entry.0.as_slice(), b"B" | b"Parent")
                || INHERITED.contains(&entry.0.as_slice()) && matches!(entry.1, Object::Null))
        });
        if let Some(up) = parent
            && INHERITED
                .iter()
                .any(|key| !entries.iter().any(|entry| entry.0 == *key))
        {
            for (key, value) in self.ancestors(objects, up)? {
                if !entries.iter().any(|entry| entry.0 == key) {
                    entries.push((key, value));
                }
            }
        }
        let node = self.number(1, at.checked_div(FANOUT).unwrap_or_default());
        entries.push((b"Parent".to_vec(), reference(node)));
        Ok(Object::Dictionary(entries))
    }

    /// The part's index of object `number` when it is one of its pages.
    fn position(&self, number: u32) -> Option<u32> {
        self.pages
            .page(number)
            .filter(|&at| self.first <= at && at < self.end)
            .map(|at| at.saturating_sub(self.first))
    }

    /// Lists the part's pages and reads the destinations the strings their links name name, in
    /// one pass over the name tree; then, unless `allow_loss`, fails with
    /// [`ErrorCode::LossRefused`] when a link leads to a page outside the part
    /// (ISO 32000-2:2020, 7.9.6, 12.3.2.4, 12.5.6.5, 12.6.4.2).
    pub(crate) fn prepare(&mut self, allow_loss: bool) -> Result<(), PdfError> {
        if !self.pages.annotated {
            return Ok(());
        }
        let objects = self.store.objects(OPERATION);
        let (mut listed, mut keys) = (Vec::new(), Vec::new());
        let mut walked = self
            .pages
            .walked
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.pages.walk(
            (objects, &self.source),
            (self.first, self.end),
            &mut walked,
            &mut |id| {
                listed.push(id);
                annotations(objects, &self.source, id, &mut |annotation| {
                    keys.extend(self.names(objects, &annotation)?);
                    Ok(())
                })
            },
        )?;
        drop(walked);
        keys.sort_unstable();
        keys.dedup();
        let resolved = self.resolve(objects, keys)?;
        self.resolved = Some(resolved);
        if !allow_loss {
            for &id in &listed {
                annotations(objects, &self.source, id, &mut |annotation| {
                    if self.lost(objects, &annotation)? {
                        return Err(PdfError::new(ErrorCode::LossRefused, OPERATION));
                    }
                    Ok(())
                })?;
            }
        }
        self.listed = Some(listed);
        Ok(())
    }

    /// Object `number` of the document and its generation.
    fn raw(&self, objects: Objects<'_>, number: u32) -> Result<Option<(u16, Object)>, PdfError> {
        if let Some(found) = self.source.get(number)? {
            return Ok(Some(found));
        }
        if number >= self.stored {
            return Ok(None);
        }
        let generation = match objects.entry(number)? {
            Some(entry) if entry.kind == XrefKind::InUse => entry.generation,
            Some(entry) if entry.kind == XrefKind::Compressed => 0,
            Some(_) | None => return Ok(None),
        };
        Ok(Some((
            generation,
            objects.owned(ObjectId { generation, number })?,
        )))
    }

    /// The destinations `keys`, sorted, name in the catalog's `Dests` name tree, null when none,
    /// read in one pass: each node once, only those whose `Limits` hold a key, a kid's `Limits`
    /// read at most twice per key; a node reached twice is malformed (ISO 32000-2:2020, 7.9.6,
    /// 12.3.2.4).
    fn resolve(&self, objects: Objects<'_>, keys: Vec<Vec<u8>>) -> Result<Named, PdfError> {
        if keys.is_empty() {
            return Ok(Named::new(None, Vec::new()));
        }
        let source = &*self.source;
        let malformed = || objects.malformed();
        let mut values = vec![Object::Null; keys.len()];
        let mut filled = vec![false; keys.len()];
        let names = source.entry(objects, &self.catalog, b"Names")?;
        let root = names.get(b"Dests").unwrap_or(Object::Null);
        let mut stack = vec![(root, 0..keys.len(), 0_u32)];
        let mut seen = BTreeSet::new();
        while let Some((held, range, depth)) = stack.pop() {
            if depth > objects.depth_limit() {
                return Err(PdfError::new(ErrorCode::LimitExceeded, OPERATION));
            }
            if let Object::Reference(id) = held
                && !seen.insert(id.number)
            {
                return Err(malformed());
            }
            let node = source.direct(objects, held)?;
            let wanted = keys.get(range.clone()).unwrap_or_default();
            if let Object::Array(pairs) = source.entry(objects, &node, b"Names")? {
                let mut items = pairs.into_iter();
                while let (Some(name), Some(value)) = (items.next(), items.next()) {
                    let Object::String(key) = source.direct(objects, name)? else {
                        continue;
                    };
                    let Ok(offset) = wanted.binary_search(&key) else {
                        continue;
                    };
                    let at = range.start.saturating_add(offset);
                    if let (Some(slot), Some(done)) = (values.get_mut(at), filled.get_mut(at))
                        && !*done
                    {
                        *slot = value;
                        *done = true;
                    }
                }
                continue;
            }
            let Object::Array(kids) = source.entry(objects, &node, b"Kids")? else {
                continue;
            };
            let mut limits = vec![Bounds::Unread; kids.len()];
            let mut at = range.start;
            while let Some(key) = keys.get(at).filter(|_| at < range.end) {
                let Some((kid, last)) = self.kid(objects, (&kids, &mut limits), key)? else {
                    at = at.saturating_add(1);
                    continue;
                };
                let rest = keys.get(at..range.end).unwrap_or_default();
                let end = at.saturating_add(rest.partition_point(|other| *other <= last));
                stack.push((kid, at..end, depth.saturating_add(1)));
                at = end;
            }
        }
        Ok(Named::new(None, keys.into_iter().zip(values).collect()))
    }

    /// The new root node's number.
    fn root(&self) -> u32 {
        self.number(self.levels.len(), 0)
    }
}

impl View for Part {
    fn generation(&self, objects: Objects<'_>, number: u32) -> Result<Option<u16>, PdfError> {
        self.source.fresh(OPERATION)?;
        if self.locate(number).is_some() {
            return Ok(Some(0));
        }
        self.source.generation(objects, number)
    }

    fn object(
        &self,
        number: u32,
        edited: Option<(u16, Object)>,
    ) -> Result<Option<(u16, Object)>, PdfError> {
        self.source.fresh(OPERATION)?;
        let objects = self.store.objects(OPERATION);
        let found = match edited {
            Some(own) => Some(own),
            None => match self.locate(number) {
                Some((level, index)) => {
                    let mut built = self.built.lock().unwrap_or_else(PoisonError::into_inner);
                    let node = if let Some(last) = built.as_ref().filter(|last| last.0 == number) {
                        last.1.clone()
                    } else {
                        let made = self.node(objects, level, index)?;
                        *built = Some((number, made.clone()));
                        made
                    };
                    drop(built);
                    Some((0, node))
                }
                None if self.outside(number) => self
                    .source
                    .generation(objects, number)?
                    .map(|generation| (generation, Object::Null)),
                None => self.raw(objects, number)?,
            },
        };
        let Some((generation, read)) = found else {
            return Ok(None);
        };
        let object = match self.position(number) {
            Some(at) => self.page(objects, read, at)?,
            None => read,
        };
        Ok(Some((generation, self.clean(objects, object)?)))
    }
}

/// The destination of `object` when it is a `GoTo` action (ISO 32000-2:2020, 12.6.4.2).
fn goto(object: &Object) -> Option<Object> {
    match object.get(b"S") {
        Some(Object::Name(kind)) if kind == b"GoTo" => object.get(b"D"),
        Some(_) | None => None,
    }
}

/// The destination of `action` when it is a `GoTo` action without a next action
/// (ISO 32000-2:2020, 12.6.2, 12.6.4.2).
fn alone(action: &Object) -> Option<Object> {
    goto(action).filter(|_| {
        action
            .get(b"Next")
            .is_none_or(|next| matches!(next, Object::Null))
    })
}

/// Gives `visit` each annotation of page `id` of the document of `objects` and `source`
/// (ISO 32000-2:2020, 12.5.2).
fn annotations(
    objects: Objects<'_>,
    source: &Overlay,
    id: ObjectId,
    visit: &mut dyn FnMut(Object) -> Result<(), PdfError>,
) -> Result<(), PdfError> {
    let page = source.resolve(objects, id)?;
    if let Object::Array(items) = source.entry(objects, &page, b"Annots")? {
        for item in items {
            visit(source.direct(objects, item)?)?;
        }
    }
    Ok(())
}

/// A reference to object `number`, generation 0.
const fn reference(number: u32) -> Object {
    Object::Reference(ObjectId {
        generation: 0,
        number,
    })
}

/// Fails with [`ErrorCode::LossRefused`] when a part of the document of `objects` and `overlay`
/// would lose its encryption, when `encrypted`, or an entry of its catalog: form fields, a
/// structure tree, bookmarks, name trees but destinations, threads, associated files,
/// document actions, an open action other than a destination or a `GoTo` action without a next
/// one, or any other entry a part neither keeps nor rewrites (ISO 32000-2:2020, 7.6, 7.7.2,
/// 7.7.4, 12.3.3, 12.4.3, 12.6.2, 12.6.3, 12.7, 14.7, 14.13).
pub(crate) fn refuse(
    objects: Objects<'_>,
    overlay: &Overlay,
    encrypted: bool,
) -> Result<(), PdfError> {
    let held = |object: &Object, key: &[u8]| {
        overlay
            .entry(objects, object, key)
            .map(|found| !matches!(found, Object::Null))
    };
    let trailer = Object::Dictionary(overlay.trailer(objects)?);
    let Object::Dictionary(catalog) = overlay.entry(objects, &trailer, b"Root")? else {
        return Err(objects.malformed());
    };
    let mut lost = encrypted;
    for (key, value) in catalog {
        lost |= match key.as_slice() {
            b"Dests" | b"PageLabels" | b"Pages" | b"Type" => false,
            b"OpenAction" => match overlay.direct(objects, value)? {
                action @ Object::Dictionary(_) => alone(&action).is_none(),
                Object::Array(_)
                | Object::Boolean(_)
                | Object::Integer(_)
                | Object::Name(_)
                | Object::Null
                | Object::Real(_)
                | Object::Reference(_)
                | Object::Stream(..)
                | Object::String(_) => false,
            },
            b"AcroForm" => matches!(
                overlay.entry(objects, &overlay.direct(objects, value)?, b"Fields")?,
                Object::Array(items) if !items.is_empty()
            ),
            b"Names" => match overlay.direct(objects, value)? {
                Object::Dictionary(trees) => trees
                    .into_iter()
                    .any(|(tree, root)| tree != b"Dests" && !matches!(root, Object::Null)),
                Object::Array(_)
                | Object::Boolean(_)
                | Object::Integer(_)
                | Object::Name(_)
                | Object::Null
                | Object::Real(_)
                | Object::Reference(_)
                | Object::Stream(..)
                | Object::String(_) => false,
            },
            b"Outlines" => held(&overlay.direct(objects, value)?, b"First")?,
            other if CARRIED.contains(&other) => false,
            _ => !matches!(overlay.direct(objects, value)?, Object::Null),
        };
    }
    if lost {
        return Err(PdfError::new(ErrorCode::LossRefused, OPERATION));
    }
    Ok(())
}

/// The object `value` refers to, when a reference.
const fn target(value: &Object) -> Option<ObjectId> {
    if let &Object::Reference(id) = value {
        Some(id)
    } else {
        None
    }
}

/// The entry of `index` for object `number`.
fn slot(index: &mut [u32], number: u32) -> Option<&mut u32> {
    index.get_mut(usize::try_from(number).ok()?)
}

/// Whether `id` of the document of `objects` and `overlay` is a page tree node, else a page, and
/// whether a page has annotations, read without its kids (ISO 32000-2:2020, 7.7.3).
fn kind(objects: Objects<'_>, overlay: &Overlay, id: ObjectId) -> Result<(bool, bool), PdfError> {
    if let Some((generation, object)) = overlay.get(id.number)? {
        if generation != id.generation || !matches!(object, Object::Dictionary(_)) {
            return Err(objects.malformed());
        }
        return Ok((
            object.get(b"Kids").is_some(),
            object.get(b"Annots").is_some(),
        ));
    }
    let dictionary = objects.dictionary(Some(Value::Reference(id)))?;
    Ok((
        objects.get(dictionary, b"Kids")?.is_some(),
        objects.get(dictionary, b"Annots")?.is_some(),
    ))
}
