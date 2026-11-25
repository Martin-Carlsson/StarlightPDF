//! What one render shares: its limits, work, warnings and cancellation.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use crate::overlay::Reading;
use crate::progress::Progress;
use crate::render::OPERATION;
use crate::render::data::Known;
use crate::render::geometry::{Area, Matrix};
use crate::render::lent::Lent;
use crate::render::plane::Planes;
use crate::rendering::RenderUsage;
use crate::store::{Objects, Pacing};
use crate::{ErrorCode, PdfError};

/// Bytes caches may hold beyond the decoded limit, so that small entries do not count.
const CACHES: u64 = 0x20_0000;

/// Bytes the planes of a band take.
pub(crate) const BAND: u64 = 0x50_0000;

/// The most rows of a band.
pub(crate) const BAND_ROWS: u64 = 1024;

/// How deep content nests. A form counts one; a tile, a soft mask and a Type 3 glyph two and a
/// pattern's soft mask three, by the stack they take, measured to fit WebAssembly's 1 MiB and
/// Python's 512 KiB threads.
pub(crate) const NESTING: u8 = 32;

/// The most lengths of a dash array, which each saved state may keep its own of.
pub(crate) const DASHES: usize = 64;

/// Work of nested content opened or a tile replayed.
pub(crate) const NESTING_WORK: u64 = 1024;

/// The deviation of a content or `CMap` string or name longer than [`OPERANDS_HELD`].
pub(crate) const LONG: &str = "a string or name is longer than this implementation keeps";

/// The most operands before an operator.
pub(crate) const OPERANDS: usize = 64;

/// The most bytes of one operator's operands.
pub(crate) const OPERANDS_HELD: usize = 0x10_0000;

/// Work of an operator.
pub(crate) const OPERATOR_WORK: u64 = 64;

/// Work of an object resolved.
pub(crate) const RESOLVE_WORK: u64 = 1024;

/// The most saved graphics states of one stream.
pub(crate) const SAVED: usize = 4096;

/// The most decoded bytes of a stream cached.
pub(crate) const STREAM: u64 = 0x1_0000;

/// The bytes the cache of streams holds.
pub(crate) const STREAMS: usize = 0x8_0000;

/// Work a render gets whatever its size.
const WORK_BASE: u64 = 0x100_0000;

/// Work per byte of the decoded limit.
const WORK_FACTOR: u64 = 16;

/// The most work the decoded limit gives: about 3.4 s at 5 ns a unit.
const WORK_MOST: u64 = 0x2800_0000;

/// Work per pixel of the image: a turned scan of 25 megapixels in RGB takes up to 2.6 G.
const WORK_PIXEL: u64 = 80;

/// The most pixels that add work, so that a page's size does not lift the ceiling.
const WORK_PIXELS: u64 = 25_000_000;

/// Work of each pixel output: cleared, laid over white and encoded.
pub(crate) const OUTPUT_WORK: u64 = 4;

/// Work between two checks of progress and staleness.
pub(crate) const WORK_STEP: u64 = 0x100_0000;

/// Work of a warning: looking it up among those recorded.
const WARN_WORK: u64 = 8;

/// Progress and staleness, checked as work is taken.
pub(crate) struct Pace<'doc> {
    /// The work left at which progress and staleness are next checked.
    pub checkpoint: Cell<u64>,
    /// The rows finished.
    pub done: Cell<u64>,
    /// The image's rows, once known.
    pub height: Cell<u64>,
    /// The document as it was when the render began.
    pub overlay: Reading,
    /// The caller's callback.
    pub progress: Option<&'doc dyn Progress>,
    /// The work left.
    pub work: &'doc Cell<u64>,
}

/// What one render shares.
pub(crate) struct Job<'doc> {
    /// The pixels of this pass: the page, then each band.
    pub area: Area,
    /// A content stream ended at damage since this was last cleared.
    pub damaged: Cell<bool>,
    /// The map from default user space to pixels.
    pub device: Matrix,
    /// The small objects resolved.
    pub known: RefCell<Known>,
    /// What caches lent, alive: all that is held counts against the decoded limit at once.
    pub lent: RefCell<Lent<'doc>>,
    /// The source's objects.
    pub objects: Objects<'doc>,
    /// The document as it was when the render began.
    pub overlay: Reading,
    /// How progress is reported, which reads of streams check too.
    pub pace: &'doc Pace<'doc>,
    /// The whole image's pixels.
    pub page: Area,
    /// The planes of this pass.
    pub planes: Planes,
    /// A warning failed the render, under strict; nothing tolerates that.
    pub refused: Cell<bool>,
    /// What the rendering is for.
    pub usage: RenderUsage,
    /// The work left, which the reads and decoding of objects and streams take from too.
    pub work: &'doc Cell<u64>,
}

impl Pace<'_> {
    /// Reports when the work left passed the checkpoint.
    pub(crate) fn check(&self) -> Result<(), PdfError> {
        let left = self.work.get();
        if left < self.checkpoint.get() {
            self.checkpoint.set(left.saturating_sub(WORK_STEP));
            self.report()?;
        }
        Ok(())
    }

    /// Reports the rows finished, then checks that the document is unchanged.
    pub(crate) fn report(&self) -> Result<(), PdfError> {
        // A total of 0 while the image's rows are not yet known.
        if let Some(callback) = self.progress
            && !callback.update(self.done.get(), self.height.get())
        {
            return Err(PdfError::new(ErrorCode::Cancelled, OPERATION));
        }
        self.overlay.fresh(OPERATION)
    }
}

impl Pacing for Pace<'_> {
    fn pace(&self) -> Result<(), PdfError> {
        self.check()
    }
}

impl Job<'_> {
    /// The error of a resource past a limit of this implementation: [`ErrorCode::LimitExceeded`]
    /// under strict, else [`ErrorCode::Malformed`], so that it is warned and skipped.
    pub(crate) fn capped(&self) -> PdfError {
        if self.objects.strict() {
            self.limit()
        } else {
            self.objects.malformed()
        }
    }

    /// Fails unless `bytes` more fit within the decoded limit beside all that is held; ask before
    /// allocating what is held whole.
    pub(crate) fn fits(&self, bytes: u64) -> Result<(), PdfError> {
        self.reserve(bytes)?;
        let limit = self.limit_bytes();
        let fitted = self.lent.borrow_mut().within(self, bytes, limit);
        self.free(limit)?;
        fitted
    }

    /// Tells reads the bytes free beside all that is held, so that an object read fits there.
    fn free(&self, limit: u64) -> Result<(), PdfError> {
        if self.objects.wants_room() {
            // An object stream was refused for room: what is kept for reuse gives way.
            self.lent.borrow_mut().give_way(self)?;
        }
        // Recounted when little seems free, so that what was given up is free again.
        let room = self.lent.borrow_mut().room(self, (CACHES, limit))?;
        self.objects.set_room(room);
        Ok(())
    }

    /// Has every cache kept for reuse give way, so that a read the free room stopped may be tried
    /// again.
    pub(crate) fn give_way(&self) -> Result<(), PdfError> {
        let limit = self.limit_bytes();
        self.lent.borrow_mut().give_way(self)?;
        self.free(limit)
    }

    /// Counts `item` of `size` bytes as alive, with all else held, within the decoded limit and
    /// the caches.
    pub(crate) fn lend<T>(&self, item: &Rc<T>, size: u64) -> Result<(), PdfError>
    where
        T: ?Sized + 'static,
    {
        let limit = self.limit_bytes();
        let added = self.lent.borrow_mut().add(self, item, (size, limit));
        self.free(limit)?;
        added
    }

    /// An [`ErrorCode::LimitExceeded`] error.
    pub(crate) fn limit(&self) -> PdfError {
        PdfError::new(ErrorCode::LimitExceeded, self.objects.operation())
    }

    /// The bytes all held may take: the decoded limit and the caches, less the object streams
    /// reads keep.
    fn limit_bytes(&self) -> u64 {
        self.objects
            .decoded_limit()
            .saturating_add(CACHES)
            .saturating_sub(self.objects.containers())
    }

    /// Recounts what is held, as something held was given up, and tells reads the room.
    pub(crate) fn recount(&self) -> Result<(), PdfError> {
        let limit = self.limit_bytes();
        self.lent.borrow_mut().recount(self)?;
        self.free(limit)
    }

    /// Reports the rows finished, then checks that the document is unchanged.
    pub(crate) fn report(&self) -> Result<(), PdfError> {
        self.pace.report()
    }

    /// Fails unless `bytes` are within the decoded limit; ask before each allocation the file
    /// sizes.
    pub(crate) fn reserve(&self, bytes: u64) -> Result<(), PdfError> {
        if bytes > self.objects.decoded_limit() {
            Err(self.limit())
        } else {
            Ok(())
        }
    }

    /// Takes `units` of work, at intervals reporting progress and checking staleness.
    pub(crate) fn spend(&self, units: u64) -> Result<(), PdfError> {
        let left = self
            .work
            .get()
            .checked_sub(units)
            .ok_or_else(|| self.limit())?;
        self.work.set(left);
        self.pace.check()
    }

    /// The value of `result`; [`ErrorCode::Malformed`] is a warning and `None`, every other error
    /// is passed on, and so is a warning's under strict.
    pub(crate) fn tolerate<T>(
        &self,
        result: Result<T, PdfError>,
        clause: &'static str,
        message: &'static str,
    ) -> Result<Option<T>, PdfError> {
        match result {
            Ok(found) => Ok(Some(found)),
            Err(error) if error.code() == ErrorCode::Malformed && !self.refused.get() => {
                self.warn(clause, message)?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// An [`ErrorCode::Unsupported`] error.
    pub(crate) fn unsupported(&self) -> PdfError {
        PdfError::new(ErrorCode::Unsupported, self.objects.operation())
    }

    /// Records the deviation `message` once a render; under strict it fails the render.
    pub(crate) fn warn(&self, clause: &'static str, message: &'static str) -> Result<(), PdfError> {
        // Looked up whether recorded or not.
        self.spend(WARN_WORK)?;
        self.objects
            .deviation(clause, message, None)
            .inspect_err(|_error| self.refused.set(true))
    }
}

/// The work of a render of `pixels` under a limit of `decoded` bytes.
pub(crate) fn work_limit(decoded: u64, pixels: u64) -> u64 {
    WORK_BASE
        .saturating_add(decoded.saturating_mul(WORK_FACTOR))
        .min(WORK_MOST)
        .saturating_add(pixels.min(WORK_PIXELS).saturating_mul(WORK_PIXEL))
}
