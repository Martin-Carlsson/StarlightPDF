//! A document's changed and added objects, held in memory over its source (ISO 32000-2:2020, 7.3).

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::fmt::Debug;
use core::mem::replace;
use core::ops::Deref;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::objects::{ObjectId, XrefKind};
use crate::store::Objects;
use crate::{ErrorCode, PdfError};

/// A dictionary's entries, in order, each key once.
pub(crate) type Dictionary = Vec<(Vec<u8>, Object)>;

/// A stream's data.
#[derive(Clone, Debug)]
pub(crate) enum Data {
    /// These bytes.
    Bytes(Vec<u8>),
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

    /// Object `number` and its generation, given `edited`, the overlay's own change of it; `None`
    /// when free or missing.
    fn object(
        &self,
        number: u32,
        edited: Option<(u16, Object)>,
    ) -> Result<Option<(u16, Object)>, PdfError>;
}

/// Stream data written at save time.
pub(crate) trait Produce: Debug + Send + Sync {
    /// Bytes [`Produce::write`] writes.
    fn length(&self) -> u64;

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

    /// `value`, the object it refers to when a reference.
    pub(crate) fn direct(&self, objects: Objects<'_>, value: Object) -> Result<Object, PdfError> {
        match value {
            Object::Reference(id) => self.resolve(objects, id),
            other @ (Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Stream(..)
            | Object::String(_)) => Ok(other),
        }
    }

    /// The changed or added object `number` and its generation, not through the view.
    fn edited(&self, number: u32) -> Option<(u16, Object)> {
        self.edits().objects.get(&number).cloned()
    }

    /// The state, locked.
    fn edits(&self) -> MutexGuard<'_, Edits> {
        self.edits.lock().unwrap_or_else(PoisonError::into_inner)
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
        let edited = self.edited(number);
        match self.view.as_deref() {
            Some(view) => view.object(number, edited),
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
