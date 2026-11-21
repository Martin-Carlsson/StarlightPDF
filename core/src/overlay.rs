//! A document's changed and added objects, held in memory over its source (ISO 32000-2:2020, 7.3).

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::fmt::Debug;
use core::mem::replace;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::objects::ObjectId;
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
pub(crate) struct Overlay(Mutex<Edits>);

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
        let mut edits = self.edits();
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

    /// The state, locked.
    fn edits(&self) -> MutexGuard<'_, Edits> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
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

    /// The changed or added object `number` and its generation, when there is one.
    pub(crate) fn get(&self, number: u32) -> Option<(u16, Object)> {
        self.edits().objects.get(&number).cloned()
    }

    /// An overlay whose first added object gets `next`.
    pub(crate) fn new(next: u32) -> Self {
        Self(Mutex::new(Edits {
            next: next.max(1),
            ..Edits::default()
        }))
    }

    /// One more than the highest object number in use, the source's included.
    pub(crate) fn next(&self) -> u32 {
        self.edits().next
    }

    /// Object `id`: changed or added here, else read from `objects`; null when missing or
    /// another generation.
    pub(crate) fn resolve(&self, objects: Objects<'_>, id: ObjectId) -> Result<Object, PdfError> {
        match self.get(id.number) {
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
            .edits()
            .objects
            .insert(id.number, (id.generation, object));
    }

    /// Replaces the trailer's `Root` and `Info` with those of `trailer`.
    pub(crate) fn set_trailer(&self, trailer: Dictionary) {
        self.edits().trailer = Some(trailer);
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
}
