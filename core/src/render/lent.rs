//! What caches lent, counted while it is alive: states keep it beyond the caches. What caches keep
//! only for reuse gives way to any later check of room that needs it.

use alloc::collections::BTreeMap;
use alloc::rc::{Rc, Weak};
use core::cell::{Ref, RefCell, RefMut};

use crate::PdfError;
use crate::render::job::Job;

/// What was lent and the bytes of it alive.
#[derive(Default)]
pub(crate) struct Lent<'doc> {
    /// The bytes alive when last swept and added since.
    alive: u64,
    /// The items and their bytes.
    items: Vec<(Box<dyn Alive>, u64)>,
    /// The count of items at which they are next swept.
    sweep: usize,
    /// What caches keep for reuse, which gives way when room is needed.
    yielding: Vec<Weak<dyn Yield + 'doc>>,
}

/// What a cache keeps for reuse only: its entries give way to any later check of room that needs
/// what they hold, once offered to the render.
pub(crate) struct Yielding<T> {
    /// The entries.
    entries: Rc<RefCell<T>>,
    /// They are offered.
    offered: bool,
}

/// Entries that can all be dropped.
pub(crate) trait Clear {
    /// Drops them all.
    fn clear(&mut self);
}

/// Entries that give way.
trait Yield {
    /// Drops them, unless they are in use this moment.
    fn give_way(&self);
}

/// What is lent, seen without holding it.
trait Alive {
    /// Whether it is still held.
    fn alive(&self) -> bool;
}

impl<T> Alive for Weak<T>
where
    T: ?Sized,
{
    fn alive(&self) -> bool {
        self.strong_count() > 0
    }
}

impl<K, V> Clear for BTreeMap<K, V> {
    fn clear(&mut self) {
        Self::clear(self);
    }
}

impl<T> Clear for Vec<T> {
    fn clear(&mut self) {
        Self::clear(self);
    }
}

impl<T> Default for Yielding<T>
where
    T: Default,
{
    fn default() -> Self {
        Self {
            entries: Rc::default(),
            offered: false,
        }
    }
}

impl<T> Yield for RefCell<T>
where
    T: Clear,
{
    fn give_way(&self) {
        if let Ok(mut entries) = self.try_borrow_mut() {
            entries.clear();
        }
    }
}

impl Lent<'_> {
    /// Counts `item` of `size` bytes as alive; over `limit` at once is
    /// [`ErrorCode::LimitExceeded`](crate::ErrorCode::LimitExceeded).
    pub(crate) fn add<T>(
        &mut self,
        job: &Job<'_>,
        item: &Rc<T>,
        (size, limit): (u64, u64),
    ) -> Result<(), PdfError>
    where
        T: ?Sized + 'static,
    {
        self.items.push((Box::new(Rc::downgrade(item)), size));
        self.alive = self.alive.saturating_add(size);
        if self.items.len() >= self.sweep {
            self.swept(job)?;
        }
        self.within(job, 0, limit)
    }

    /// Has every cache kept for reuse give way, then recounts what is alive.
    pub(crate) fn give_way(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        job.objects.drop_containers();
        self.yielding.retain(|entries| entries.strong_count() > 0);
        for entries in self.yielding.iter().filter_map(Weak::upgrade) {
            entries.give_way();
        }
        self.swept(job)
    }

    /// Recounts what is alive, now, as something held was given up.
    pub(crate) fn recount(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        self.swept(job)
    }

    /// The bytes that fit in `limit` beside what is alive, at least `enough`, recounted when fewer
    /// seem to.
    pub(crate) fn room(
        &mut self,
        job: &Job<'_>,
        (enough, limit): (u64, u64),
    ) -> Result<u64, PdfError> {
        if limit.saturating_sub(self.alive) < enough {
            self.swept(job)?;
        }
        Ok(limit.saturating_sub(self.alive))
    }

    /// Recounts what is alive.
    fn swept(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        job.spend(u64::try_from(self.items.len()).unwrap_or(u64::MAX))?;
        self.items.retain(|known| known.0.alive());
        self.alive = self
            .items
            .iter()
            .fold(0, |sum: u64, known| sum.saturating_add(known.1));
        self.sweep = self.items.len().saturating_mul(2).saturating_add(64);
        Ok(())
    }

    /// Fails unless `bytes` more fit in `limit` beside what is alive.
    pub(crate) fn within(&mut self, job: &Job<'_>, bytes: u64, kept: u64) -> Result<(), PdfError> {
        let mut limit = kept;
        if self.alive.saturating_add(bytes) > limit {
            self.swept(job)?;
            self.yielding.retain(|entries| entries.strong_count() > 0);
            if self.alive.saturating_add(bytes) > limit {
                // What is kept for reuse is made again instead; the object streams kept leave
                // their room.
                let before = job.objects.containers();
                job.objects.drop_containers();
                limit = limit.saturating_add(before.saturating_sub(job.objects.containers()));
                for entries in self.yielding.iter().filter_map(Weak::upgrade) {
                    entries.give_way();
                }
                self.swept(job)?;
            }
            if self.alive.saturating_add(bytes) > limit {
                return Err(job.limit());
            }
        }
        Ok(())
    }
}

impl<'doc, T> Yielding<T>
where
    T: Clear + 'doc,
{
    /// The entries, while the result is held.
    pub(crate) fn entries(&self) -> Ref<'_, T> {
        self.entries.borrow()
    }

    /// The entries to change, while the result is held.
    pub(crate) fn entries_mut(&self) -> RefMut<'_, T> {
        self.entries.borrow_mut()
    }

    /// Offers the entries to `job`, once, so that they give way when room is needed.
    pub(crate) fn offer(&mut self, job: &Job<'doc>) {
        if !self.offered {
            let entries: Rc<dyn Yield + 'doc> = Rc::<RefCell<T>>::clone(&self.entries);
            job.lent.borrow_mut().yielding.push(Rc::downgrade(&entries));
            self.offered = true;
        }
    }
}
