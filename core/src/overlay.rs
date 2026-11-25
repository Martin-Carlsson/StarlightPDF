//! A document's changed and added objects, held in memory over its source (ISO 32000-2:2020, 7.3).

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::fmt::Debug;
use core::mem::replace;
use core::ops::Deref;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::object_stream::Sequences;
use crate::objects::{ObjectId, XrefKind};
use crate::pull::Pull;
use crate::store::Objects;
use crate::syntax::{write_name, write_real, write_string};
use crate::{ErrorCode, PdfError};

/// A dictionary's entries, in order, each key once.
pub(crate) type Dictionary = Vec<(Vec<u8>, Object)>;

/// A stream's data.
#[derive(Clone, Debug)]
pub(crate) enum Data {
    /// These bytes, shared, so a copy of the object copies none.
    Bytes(Arc<[u8]>),
    /// Written at save time.
    Produced(Arc<dyn Produce>),
    /// The stored bytes of the source's stream object, decrypted.
    Stored(ObjectId),
}

/// The overlay's state.
#[derive(Debug, Default)]
struct Edits {
    /// Changes made, which make [`Reading`]s from before them stale.
    changes: u64,
    /// The number the next added object gets.
    next: u32,
    /// Changed and added objects, by number.
    objects: BTreeMap<u32, (u16, Object)>,
    /// Content was redacted, which the source's bytes still hold.
    redacted: bool,
    /// The trailer's `Root` and `Info` entries, when changed.
    trailer: Option<Dictionary>,
}

/// An object held in memory (ISO 32000-2:2020, 7.3).
#[derive(Clone, Debug)]
pub(crate) enum Object {
    /// An array.
    Array(Vec<Self>),
    /// A boolean.
    Boolean(bool),
    /// A dictionary.
    Dictionary(Dictionary),
    /// An integer.
    Integer(i64),
    /// A name.
    Name(Vec<u8>),
    /// The null object.
    Null,
    /// A real.
    Real(f64),
    /// An indirect reference.
    Reference(ObjectId),
    /// A stream: its dictionary, `Length` left out, and data; only as an indirect object.
    Stream(Dictionary, Data),
    /// A string's bytes.
    String(Vec<u8>),
}

/// Changed and added objects over a source.
#[derive(Debug, Default)]
pub(crate) struct Overlay {
    /// The state.
    edits: Mutex<Edits>,
    /// What objects look like through this overlay, when not as stored.
    view: Option<Box<dyn View>>,
}

/// An overlay read by a split's parts, stale once it changes.
#[derive(Debug)]
pub(crate) struct Reading {
    /// The overlay's changes when read first.
    changes: u64,
    /// The overlay.
    overlay: Arc<Overlay>,
}

/// Objects computed or rewritten on read.
pub(crate) trait View: Debug + Send + Sync {
    /// The generation of object `number` when not changed; `None` when free or missing.
    fn generation(&self, objects: Objects<'_>, number: u32) -> Result<Option<u16>, PdfError>;

    /// Object `number` and its generation, given `edited`, the overlay's own change of it, members
    /// of an object stream read on in `sequence`; `None` when free or missing.
    fn object<'view>(
        &'view self,
        number: u32,
        edited: Option<(u16, Object)>,
        sequence: &mut Sequences<'view>,
    ) -> Result<Option<(u16, Object)>, PdfError>;
}

/// Stream data written at save time.
pub(crate) trait Produce: Debug + Send + Sync {
    /// Bytes a reader holds whole while it is alive.
    #[cfg(feature = "rendering")]
    fn held(&self) -> u64;

    /// Bytes [`Produce::write`] writes.
    fn length(&self) -> u64;

    /// The data read in order, by a reader that keeps it alive.
    fn reader(&self) -> Box<dyn Pull>;

    /// Writes the data in pieces to `out`.
    ///
    /// # Errors
    ///
    /// Whatever `out` or reading the data fails with.
    fn write(&self, out: &mut dyn FnMut(&[u8]) -> Result<(), PdfError>) -> Result<(), PdfError>;
}

impl Object {
    /// The value of `key`, when a dictionary or stream holds it.
    pub(crate) fn get(&self, key: &[u8]) -> Option<Self> {
        self.clone().take(key)
    }

    /// Sets `key` of a dictionary or stream to `value`, or with `None` removes it.
    pub(crate) fn set(&mut self, key: &[u8], value: Option<Self>) {
        let edit = |mut entries: Dictionary| {
            entries.retain(|entry| entry.0 != key);
            entries.extend(value.map(|found| (key.to_vec(), found)));
            entries
        };
        *self = match replace(self, Self::Null) {
            Self::Dictionary(entries) => Self::Dictionary(edit(entries)),
            Self::Stream(entries, data) => Self::Stream(edit(entries), data),
            other @ (Self::Array(_)
            | Self::Boolean(_)
            | Self::Integer(_)
            | Self::Name(_)
            | Self::Null
            | Self::Real(_)
            | Self::Reference(_)
            | Self::String(_)) => other,
        };
    }

    /// The object in canonical PDF syntax, as [`Document::object`](crate::document::Document::object)
    /// gives it; a stream as its dictionary (ISO 32000-2:2020, 7.3).
    pub(crate) fn syntax(self, out: &mut String) {
        match self {
            Self::Array(items) => {
                out.push('[');
                for (at, item) in items.into_iter().enumerate() {
                    if at > 0 {
                        out.push(' ');
                    }
                    item.syntax(out);
                }
                out.push(']');
            }
            Self::Boolean(flag) => out.push_str(if flag { "true" } else { "false" }),
            Self::Dictionary(entries) | Self::Stream(entries, _) => {
                out.push_str("<<");
                for (key, value) in entries {
                    out.push(' ');
                    write_name(out, &key);
                    out.push(' ');
                    value.syntax(out);
                }
                out.push_str(" >>");
            }
            Self::Integer(number) => out.push_str(&number.to_string()),
            Self::Name(bytes) => write_name(out, &bytes),
            Self::Null => out.push_str("null"),
            Self::Real(number) => write_real(out, number),
            Self::Reference(id) => {
                out.push_str(&id.number.to_string());
                out.push(' ');
                out.push_str(&id.generation.to_string());
                out.push_str(" R");
            }
            Self::String(bytes) => write_string(out, &bytes),
        }
    }

    /// The value of `key`, when this dictionary or stream holds it.
    pub(crate) fn take(self, key: &[u8]) -> Option<Self> {
        match self {
            Self::Dictionary(entries) | Self::Stream(entries, _) => entries
                .into_iter()
                .find(|entry| entry.0 == key)
                .map(|entry| entry.1),
            Self::Array(_)
            | Self::Boolean(_)
            | Self::Integer(_)
            | Self::Name(_)
            | Self::Null
            | Self::Real(_)
            | Self::Reference(_)
            | Self::String(_) => None,
        }
    }
}

impl Overlay {
    /// Adds `object` under a new number, generation 0.
    pub(crate) fn add(&self, object: Object) -> Result<ObjectId, PdfError> {
        let mut edits = self.writable();
        let number = edits.next;
        edits.next = number
            .checked_add(1)
            .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, "Overlay::add"))?;
        let _old = edits.objects.insert(number, (0, object));
        drop(edits);
        Ok(ObjectId {
            generation: 0,
            number,
        })
    }

    /// `value`, followed through references, also to references; a cycle or a chain longer
    /// than the depth limit is [`ErrorCode::LimitExceeded`] (ISO 32000-2:2020, 7.3.10).
    pub(crate) fn direct(&self, objects: Objects<'_>, value: Object) -> Result<Object, PdfError> {
        self.ended(objects, value).map(|found| found.0)
    }

    /// [`Self::direct`], members of an object stream read on in `sequence`.
    pub(crate) fn direct_in<'store>(
        &self,
        objects: Objects<'store>,
        value: Object,
        sequence: &mut Sequences<'store>,
    ) -> Result<Object, PdfError> {
        let mut found = value;
        for _ in 0..=objects.depth_limit() {
            let Object::Reference(id) = found else {
                return Ok(found);
            };
            found = match self.get(id.number)? {
                Some((generation, object)) => {
                    if generation == id.generation {
                        object
                    } else {
                        Object::Null
                    }
                }
                None => objects.owned_in(id, sequence)?,
            };
        }
        Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()))
    }

    /// The changed or added object `number` and its generation, not through the view.
    fn edited(&self, number: u32) -> Option<(u16, Object)> {
        self.edits().objects.get(&number).cloned()
    }

    /// The state, locked.
    fn edits(&self) -> MutexGuard<'_, Edits> {
        self.edits.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// [`Self::direct`] and the id the chain ends at, `None` when `value` is direct.
    pub(crate) fn ended(
        &self,
        objects: Objects<'_>,
        value: Object,
    ) -> Result<(Object, Option<ObjectId>), PdfError> {
        let (mut found, mut last) = (value, None);
        for _ in 0..=objects.depth_limit() {
            let Object::Reference(id) = found else {
                return Ok((found, last));
            };
            found = self.resolve(objects, id)?;
            last = Some(id);
        }
        Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()))
    }

    /// The value of `key` of `object`, resolved; null when missing.
    pub(crate) fn entry(
        &self,
        objects: Objects<'_>,
        object: &Object,
        key: &[u8],
    ) -> Result<Object, PdfError> {
        self.direct(objects, object.get(key).unwrap_or(Object::Null))
    }

    /// The generation of object `number`, changed here, given by the view or the source's entry;
    /// `None` when free or missing (ISO 32000-2:2020, 7.3.10, 7.5.4).
    pub(crate) fn generation(
        &self,
        objects: Objects<'_>,
        number: u32,
    ) -> Result<Option<u16>, PdfError> {
        if let Some((generation, _)) = self.edited(number) {
            return Ok(Some(generation));
        }
        if let Some(view) = self.view.as_deref() {
            return view.generation(objects, number);
        }
        Ok(match objects.entry(number)? {
            Some(entry) if entry.kind == XrefKind::InUse => Some(entry.generation),
            Some(entry) if entry.kind == XrefKind::Compressed => Some(0),
            Some(_) | None => None,
        })
    }

    /// Object `number` and its generation when changed, added or given by the view.
    pub(crate) fn get(&self, number: u32) -> Result<Option<(u16, Object)>, PdfError> {
        self.get_in(number, &mut Sequences::default())
    }

    /// [`Self::get`], members of an object stream the view reads read on in `sequence`.
    pub(crate) fn get_in<'view>(
        &'view self,
        number: u32,
        sequence: &mut Sequences<'view>,
    ) -> Result<Option<(u16, Object)>, PdfError> {
        let edited = self.edited(number);
        match self.view.as_deref() {
            Some(view) => view.object(number, edited, sequence),
            None => Ok(edited),
        }
    }

    /// Whether it holds a change or a view.
    pub(crate) fn is_changed(&self) -> bool {
        let edits = self.edits();
        self.view.is_some() || !edits.objects.is_empty() || edits.trailer.is_some()
    }

    /// An overlay whose first added object gets `next`.
    pub(crate) fn new(next: u32) -> Self {
        Self {
            edits: Mutex::new(Edits {
                next: next.max(1),
                ..Edits::default()
            }),
            view: None,
        }
    }

    /// One more than the highest object number in use, the source's included.
    pub(crate) fn next(&self) -> u32 {
        self.edits().next
    }

    /// Whether content was redacted, so that the source's bytes must not be kept.
    pub(crate) fn redacted(&self) -> bool {
        self.edits().redacted
    }

    /// A new number, generation 0, for an object [`Reading::commit`] adds; no change.
    #[cfg(feature = "rendering")]
    pub(crate) fn reserve(&self) -> Result<ObjectId, PdfError> {
        let mut edits = self.edits();
        let number = edits.next;
        edits.next = number
            .checked_add(1)
            .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, "Overlay::reserve"))?;
        drop(edits);
        Ok(ObjectId {
            generation: 0,
            number,
        })
    }

    /// Object `id`: changed or added here, else read from `objects`; null when missing or
    /// another generation.
    pub(crate) fn resolve(&self, objects: Objects<'_>, id: ObjectId) -> Result<Object, PdfError> {
        match self.get(id.number)? {
            Some((generation, object)) => Ok(if generation == id.generation {
                object
            } else {
                Object::Null
            }),
            None => objects.owned(id),
        }
    }

    /// Replaces object `id`.
    pub(crate) fn set(&self, id: ObjectId, object: Object) {
        let _old = self
            .writable()
            .objects
            .insert(id.number, (id.generation, object));
    }

    /// Replaces the trailer's `Root` and `Info` with those of `trailer`.
    pub(crate) fn set_trailer(&self, trailer: Dictionary) {
        self.writable().trailer = Some(trailer);
    }

    /// The trailer's `Root` and `Info`, changed here or read from `objects`; none without a
    /// source (ISO 32000-2:2020, 7.5.5).
    pub(crate) fn trailer(&self, objects: Objects<'_>) -> Result<Dictionary, PdfError> {
        let changed = self.edits().trailer.clone();
        if let Some(trailer) = changed {
            return Ok(trailer);
        }
        let Object::Dictionary(entries) = objects.owned_trailer()? else {
            return Err(objects.malformed());
        };
        Ok(entries
            .into_iter()
            .filter(|entry| matches!(entry.0.as_slice(), b"Root" | b"Info"))
            .collect())
    }

    /// An empty overlay seen through `view`, whose first added object gets `next`.
    pub(crate) fn with_view(view: Box<dyn View>, next: u32) -> Self {
        Self {
            view: Some(view),
            ..Self::new(next)
        }
    }

    /// The state, locked to change, the change counted.
    fn writable(&self) -> MutexGuard<'_, Edits> {
        let mut edits = self.edits();
        edits.changes = edits.changes.wrapping_add(1);
        edits
    }
}

impl Reading {
    /// The same overlay, as stale as this.
    pub(crate) fn again(&self) -> Self {
        Self {
            changes: self.changes,
            overlay: Arc::clone(&self.overlay),
        }
    }

    /// Sets `objects` at once, as one change, and marks the overlay redacted; fails with
    /// [`ErrorCode::StaleHandle`], setting nothing, once the overlay changed.
    #[cfg(feature = "rendering")]
    pub(crate) fn commit(
        &self,
        objects: BTreeMap<u32, (u16, Object)>,
        operation: &'static str,
    ) -> Result<(), PdfError> {
        let mut edits = self.overlay.edits();
        if edits.changes != self.changes {
            return Err(PdfError::new(ErrorCode::StaleHandle, operation));
        }
        edits.changes = edits.changes.wrapping_add(1);
        edits.objects.extend(objects);
        edits.redacted = true;
        drop(edits);
        Ok(())
    }

    /// Fails with [`ErrorCode::StaleHandle`] once the overlay changed.
    pub(crate) fn fresh(&self, operation: &'static str) -> Result<(), PdfError> {
        if self.overlay.edits().changes == self.changes {
            Ok(())
        } else {
            Err(PdfError::new(ErrorCode::StaleHandle, operation))
        }
    }

    /// `overlay`, as it is now.
    pub(crate) fn new(overlay: &Arc<Overlay>) -> Self {
        Self {
            changes: overlay.edits().changes,
            overlay: Arc::clone(overlay),
        }
    }
}

impl Deref for Reading {
    type Target = Overlay;

    fn deref(&self) -> &Overlay {
        &self.overlay
    }
}
