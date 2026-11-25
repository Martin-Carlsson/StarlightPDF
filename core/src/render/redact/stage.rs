//! What a redaction changes, held until all of it is known, then set at once.

use alloc::collections::BTreeMap;

use crate::PdfError;
use crate::objects::ObjectId;
use crate::overlay::{Data, Object, Overlay, Reading};
use crate::render::data;
use crate::render::job::Job;

/// Objects changed and added, not yet in the overlay.
#[derive(Default)]
pub(super) struct Stage {
    /// About the bytes they hold.
    bytes: u64,
    /// They, by number.
    objects: BTreeMap<u32, (u16, Object)>,
}

impl Stage {
    /// Adds `object` under a new number.
    pub(super) fn add(&mut self, job: &Job<'_>, object: Object) -> Result<ObjectId, PdfError> {
        let id = job.overlay.reserve()?;
        self.set(job, id, object)?;
        Ok(id)
    }

    /// Sets them all in `reading`'s overlay at once; [`crate::ErrorCode::StaleHandle`] once it changed.
    pub(super) fn commit(self, reading: &Reading, operation: &'static str) -> Result<(), PdfError> {
        reading.commit(self.objects, operation)
    }

    /// Object `id` as staged, else as `overlay` has it.
    pub(super) fn get(&self, job: &Job<'_>, id: ObjectId) -> Result<Object, PdfError> {
        let overlay: &Overlay = &job.overlay;
        match self.objects.get(&id.number) {
            Some(&(generation, ref object)) if generation == id.generation => Ok(object.clone()),
            Some(_) | None => overlay.resolve(job.objects, id),
        }
    }

    /// The bytes more that fit within the decoded limit.
    pub(super) const fn room(&self, job: &Job<'_>) -> u64 {
        job.objects.decoded_limit().saturating_sub(self.bytes)
    }

    /// Stages `object` as object `id`, all staged within the decoded limit.
    pub(super) fn set(
        &mut self,
        job: &Job<'_>,
        id: ObjectId,
        object: Object,
    ) -> Result<(), PdfError> {
        let held = match object {
            Object::Stream(_, Data::Bytes(ref bytes)) => {
                u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            }
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => 0,
        };
        self.bytes = self
            .bytes
            .saturating_add(held)
            .saturating_add(data::size(&object));
        if self.bytes > job.objects.decoded_limit() {
            return Err(job.limit());
        }
        drop(self.objects.insert(id.number, (id.generation, object)));
        Ok(())
    }
}
