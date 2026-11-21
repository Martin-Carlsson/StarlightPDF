//! A part of a split: a run of a document's pages under a page tree of its own, its objects
//! rewritten as read so that it reaches no other page (ISO 32000-2:2020, 7.7.3).

use alloc::sync::Arc;
use core::fmt::{self, Debug, Formatter};
use std::sync::{Mutex, PoisonError};

use crate::objects::{ObjectId, XrefKind};
use crate::overlay::{Dictionary, Object, Overlay, View};
use crate::store::{Objects, Store};
use crate::{ErrorCode, PdfError};

/// The catalog entries a part keeps (ISO 32000-2:2020, 7.7.2, 8.11.4, 14.11.5).
const CARRIED: [&[u8]; 5] = [
    b"Lang",
    b"OCProperties",
    b"OutputIntents",
    b"PageLayout",
    b"Version",
];

/// Kids of a new page tree node.
const FANOUT: u32 = 64;

/// The attributes a page inherits (ISO 32000-2:2020, 7.7.3.4).
const INHERITED: [&[u8]; 4] = [b"CropBox", b"MediaBox", b"Resources", b"Rotate"];

/// The operation errors name.
const OPERATION: &str = "Document::split";

/// The mark of a page tree node in [`Pages`].
const NODE: u32 = 1 << 31;

/// Where a document's pages are in its page tree: 4 bytes per object.
pub(crate) struct Pages {
    /// Whether a page has annotations.
    annotated: bool,
    /// The number of pages.
    count: u32,
    /// Per object number: a page's index plus 1, [`NODE`] with the index of the first page
    /// under it for a page tree node, else 0.
    index: Vec<u32>,
    /// The root page tree node.
    top: ObjectId,
}

/// Pages `first..end` of a document under new page tree nodes numbered from `base`.
#[derive(Debug)]
pub(crate) struct Part {
    /// The first new node's number.
    base: u32,
    /// The document's catalog.
    catalog: Object,
    /// One past the part's last page.
    end: u32,
    /// The part's first page.
    first: u32,
    /// The page tree node looked up last and the attributes it and its ancestors give.
    inherited: Mutex<Option<(u32, Dictionary)>>,
    /// New nodes per level, the pages' parents first, the root last.
    levels: Vec<u32>,
    /// The document's pages.
    pages: Arc<Pages>,
    /// The document's changes.
    source: Arc<Overlay>,
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
        let size = usize::try_from(overlay.next()).map_err(|_error| limit())?;
        // Reserved whole, so no growth holds two copies.
        let mut index = Vec::new();
        index.try_reserve_exact(size).map_err(|_error| limit())?;
        index.resize(size, 0);
        *slot(&mut index, top.number).ok_or_else(malformed)? = NODE;
        let root = overlay.resolve(objects, top)?;
        let Object::Array(kids) = overlay.entry(objects, &root, b"Kids")? else {
            return Err(malformed());
        };
        let (mut annotated, mut count) = (false, 0_u32);
        let mut stack = vec![kids.into_iter()];
        let depth = usize::try_from(objects.depth_limit()).unwrap_or(usize::MAX);
        while let Some(level) = stack.last_mut() {
            let Some(kid) = level.next() else {
                drop(stack.pop());
                continue;
            };
            let Object::Reference(id) = kid else {
                return Err(malformed());
            };
            let child = overlay.resolve(objects, id)?;
            let mark = slot(&mut index, id.number)
                .filter(|mark| **mark == 0)
                .ok_or_else(malformed)?;
            if child.get(b"Kids").is_some() {
                *mark = NODE | count;
                let Object::Array(below) = overlay.entry(objects, &child, b"Kids")? else {
                    return Err(malformed());
                };
                if stack.len() >= depth {
                    return Err(limit());
                }
                stack.push(below.into_iter());
            } else if matches!(child, Object::Dictionary(_)) {
                count = count
                    .checked_add(1)
                    .filter(|&next| next < NODE)
                    .ok_or_else(limit)?;
                *mark = count;
                annotated |= child.get(b"Annots").is_some();
            } else {
                return Err(malformed());
            }
        }
        Ok(Self {
            annotated,
            count,
            index,
            top,
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
    /// tree nodes that hold them, through `overlay` (ISO 32000-2:2020, 7.7.3.2).
    fn walk(
        &self,
        objects: Objects<'_>,
        overlay: &Overlay,
        (from, until): (u32, u32),
        visit: &mut dyn FnMut(ObjectId) -> Result<(), PdfError>,
    ) -> Result<(), PdfError> {
        let malformed = || objects.malformed();
        let kids = |id| match overlay.entry(objects, &overlay.resolve(objects, id)?, b"Kids")? {
            Object::Array(items) => Ok(items.into_iter().peekable()),
            Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => Err(malformed()),
        };
        let depth = usize::try_from(objects.depth_limit()).unwrap_or(usize::MAX);
        let mut stack = vec![kids(self.top)?];
        while let Some(level) = stack.last_mut() {
            let Some(kid) = level.next() else {
                drop(stack.pop());
                continue;
            };
            // A kid followed by one starting at or before `from` holds none of the pages.
            if level
                .peek()
                .and_then(target)
                .and_then(|next| self.start(next.number))
                .is_some_and(|next| next <= from)
            {
                continue;
            }
            let id = target(&kid).ok_or_else(malformed)?;
            let start = self.start(id.number).ok_or_else(malformed)?;
            if start >= until {
                return Ok(());
            }
            if self.tag(id.number) >= NODE {
                if stack.len() >= depth {
                    return Err(PdfError::new(ErrorCode::LimitExceeded, OPERATION));
                }
                stack.push(kids(id)?);
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
            self.inherited
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

    /// The new catalog: the page tree and the carried entries (ISO 32000-2:2020, 7.7.2).
    pub(crate) fn catalog(&self) -> Dictionary {
        let mut entries = vec![
            (b"Type".to_vec(), Object::Name(b"Catalog".to_vec())),
            (b"Pages".to_vec(), reference(self.root())),
        ];
        for key in CARRIED {
            if let Some(value) = self.catalog.get(key) {
                entries.push((key.to_vec(), value));
            }
        }
        entries
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
        let source = &*self.source;
        let key = match name {
            Object::Name(key) => {
                let dests = source.entry(objects, &self.catalog, b"Dests")?;
                return source.entry(objects, &dests, &key);
            }
            Object::String(key) => key,
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Integer(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..) => return Ok(Object::Null),
        };
        let names = source.entry(objects, &self.catalog, b"Names")?;
        let mut node = source.entry(objects, &names, b"Dests")?;
        for _ in 0..=objects.depth_limit() {
            if let Object::Array(pairs) = source.entry(objects, &node, b"Names")? {
                let mut items = pairs.into_iter();
                while let (Some(held), Some(value)) = (items.next(), items.next()) {
                    if let Object::String(found) = source.direct(objects, held)?
                        && found == key
                    {
                        return source.direct(objects, value);
                    }
                }
                return Ok(Object::Null);
            }
            let Object::Array(kids) = source.entry(objects, &node, b"Kids")? else {
                return Ok(Object::Null);
            };
            let mut below = None;
            for kid in kids {
                let child = source.direct(objects, kid)?;
                if let Object::Array(limits) = source.entry(objects, &child, b"Limits")? {
                    let mut bounds = limits.into_iter();
                    if let (Some(Object::String(low)), Some(Object::String(high))) =
                        (bounds.next(), bounds.next())
                        && low <= key
                        && key <= high
                    {
                        below = Some(child);
                        break;
                    }
                }
            }
            let Some(found) = below else {
                return Ok(Object::Null);
            };
            node = found;
        }
        Err(PdfError::new(ErrorCode::LimitExceeded, OPERATION))
    }

    /// A part of `pages`, those from `first` to before `end`, of the document of `source` and
    /// `store`.
    pub(crate) fn new(
        (source, store): (&Arc<Overlay>, &Arc<Store>),
        pages: &Arc<Pages>,
        (first, end): (u32, u32),
    ) -> Result<Self, PdfError> {
        let objects = store.objects(OPERATION);
        let trailer = Object::Dictionary(source.trailer(objects)?);
        let catalog = source.entry(objects, &trailer, b"Root")?;
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
            catalog,
            end,
            first,
            inherited: Mutex::new(None),
            levels,
            pages: Arc::clone(pages),
            source: Arc::clone(source),
            store: Arc::clone(store),
            stored: objects.size()?,
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
        if level == 1 {
            let range = (
                self.first.saturating_add(start),
                self.first.saturating_add(stop),
            );
            self.pages.walk(objects, &self.source, range, &mut |id| {
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

    /// Fails with [`ErrorCode::LossRefused`] when an annotation of a page of the part links to
    /// a page outside it (ISO 32000-2:2020, 12.5.6.5, 12.6.4.2).
    pub(crate) fn refuse_links(&self) -> Result<(), PdfError> {
        if !self.pages.annotated {
            return Ok(());
        }
        let objects = self.store.objects(OPERATION);
        let source = &*self.source;
        self.pages
            .walk(objects, source, (self.first, self.end), &mut |id| {
                let page = source.resolve(objects, id)?;
                if let Object::Array(annotations) = source.entry(objects, &page, b"Annots")? {
                    for annotation in annotations {
                        if self.lost(objects, &source.direct(objects, annotation)?)? {
                            return Err(PdfError::new(ErrorCode::LossRefused, OPERATION));
                        }
                    }
                }
                Ok(())
            })
    }

    /// The new root node's number.
    fn root(&self) -> u32 {
        self.number(self.levels.len(), 0)
    }
}

impl View for Part {
    fn object(
        &self,
        number: u32,
        edited: Option<(u16, Object)>,
    ) -> Result<Option<(u16, Object)>, PdfError> {
        let objects = self.store.objects(OPERATION);
        let found = match edited {
            Some(own) => Some(own),
            None => match self.locate(number) {
                Some((level, index)) => Some((0, self.node(objects, level, index)?)),
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

/// A reference to object `number`, generation 0.
const fn reference(number: u32) -> Object {
    Object::Reference(ObjectId {
        generation: 0,
        number,
    })
}

/// Fails with [`ErrorCode::LossRefused`] when a part of the document of `objects` and `overlay`
/// would lose its encryption, when `encrypted`, its form fields, structure tree, bookmarks,
/// attachments or document scripts (ISO 32000-2:2020, 7.6, 7.7.2, 7.11.4, 12.3.3, 12.6.3,
/// 12.6.4.17, 12.7, 14.7).
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
    let catalog = overlay.entry(objects, &trailer, b"Root")?;
    let form = overlay.entry(objects, &catalog, b"AcroForm")?;
    let fields = matches!(
        overlay.entry(objects, &form, b"Fields")?,
        Object::Array(items) if !items.is_empty()
    );
    let outlines = overlay.entry(objects, &catalog, b"Outlines")?;
    let names = overlay.entry(objects, &catalog, b"Names")?;
    if encrypted
        || fields
        || held(&catalog, b"StructTreeRoot")?
        || held(&outlines, b"First")?
        || held(&names, b"EmbeddedFiles")?
        || held(&names, b"JavaScript")?
        || held(&catalog, b"AA")?
    {
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
