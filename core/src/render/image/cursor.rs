//! Decoders kept open across bands and images held decoded (ISO 32000-2:2020, 7.4, 8.9.5).

use alloc::rc::Rc;
use core::ops::Range;

use crate::objects::ObjectId;
use crate::overlay::Object;
use crate::pull::Pull;
use crate::render::function::room;
use crate::render::geometry::Area;
use crate::render::image::samples::Layout;
use crate::render::job::Job;
use crate::render::lent::Yielding;
use crate::render::{OPERATION, codec, colour, data};
use crate::{PdfError, dct};

/// The bytes the open decoders hold.
const CURSORS: u64 = 0x30_0000;

/// The least a swath counts beside its data, so that few are searched.
const SLOT: u64 = 0x4000;

/// The bytes the open decoders and the swaths hold together.
const KEPT: u64 = 0x70_0000;

/// The bytes held decoded.
const DECODED: u64 = 0x80_0000;

/// What one stage of decoding holds, and the least an entry of either cache counts.
const STAGE: u64 = 0x1_0000;

/// Bytes passed over at a time.
const STEP: usize = 4096;

/// A decoder and the row it read last.
pub(super) struct Cursor<'data> {
    /// What it holds, lent while it is alive: a JPEG decoder's.
    _lent: Option<Rc<()>>,
    /// That row's number.
    at: Option<u32>,
    /// The layout.
    layout: Layout,
    /// The row.
    row: Vec<u8>,
    /// The decoder.
    rows: Box<dyn Rows + 'data>,
    /// The bytes it holds; none: it is not kept.
    size: Option<u64>,
    /// The bytes of the row that are data.
    valid: usize,
}

/// The decoders kept open, and the swaths held of turned images.
#[derive(Default)]
pub(super) struct Cursors<'doc> {
    /// The decoders, the least recent first.
    open: Yielding<Vec<Kept<'doc>>>,
    /// The swaths, the least recent first.
    swaths: Vec<Rc<Swath>>,
    /// The decoded bytes of the turned images drawn, as the pass that measures met them.
    turned: u64,
}

/// An image decoded whole.
pub(super) struct Decoded {
    /// The channel that is opacity.
    pub alpha: Option<usize>,
    /// The components of the space the data names.
    pub colours: Option<usize>,
    /// The samples.
    pub data: Vec<u8>,
    /// The layout.
    pub layout: Layout,
    /// Colour is premultiplied by opacity.
    pub premultiplied: bool,
    /// The bytes the profile within holds.
    pub profiled: u64,
    /// The space of the profile within.
    pub space: Option<Rc<colour::Space>>,
    /// The samples are YCC of sRGB.
    pub ycc: bool,
}

/// The images held decoded.
#[derive(Default)]
pub(super) struct Decodings {
    /// Them, the least recent first.
    held: Yielding<Vec<Decoding>>,
}

/// An image held decoded, or that it cannot be.
#[derive(Clone)]
pub(super) struct Decoding {
    /// The image.
    pub key: Key,
    /// It; none where it cannot be.
    pub result: Option<Rc<Decoded>>,
}

/// The rows of samples held.
struct HeldRows {
    /// The image.
    image: Rc<Decoded>,
}

/// The rows of a JPEG.
struct Jpeg<'data> {
    /// The decoder.
    decoder: dct::Decoder<'data>,
    /// No more rows come.
    ended: bool,
    /// The next row.
    next: u32,
}

/// An image as it is decoded.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Key {
    /// Its stream.
    pub id: ObjectId,
    /// Its samples are indices.
    pub indices: bool,
    /// It is a mask.
    pub mask: bool,
    /// The size it was decoded toward.
    pub toward: [u32; 2],
}

/// A decoder kept.
struct Kept<'doc> {
    /// It.
    cursor: Cursor<'doc>,
    /// Its stream.
    id: ObjectId,
    /// The page row after the image's last.
    until: i32,
}

/// The rows of a stream.
struct Plain<'data> {
    /// The data.
    data: Box<dyn Pull + 'data>,
    /// No more data comes.
    ended: bool,
    /// The next row.
    next: u32,
    /// Its bytes are still to be counted as work.
    uncounted: bool,
}

/// Samples held of a turned image, for the bands to come: decoding gives them by rows and each
/// band wants a few of them across every row, or a few rows from the far end.
pub(super) struct Swath {
    /// The samples, row after row.
    data: Vec<u8>,
    /// The stream.
    id: ObjectId,
    /// The layout.
    layout: Layout,
    /// The bytes held of each row.
    span: Range<usize>,
    /// The page row after the last they serve.
    until: i32,
    /// The bytes of each row that are data.
    valid: Vec<u32>,
    /// The texels.
    window: Area,
}

/// The rows of a swath.
struct SwathRows {
    /// The swath.
    swath: Rc<Swath>,
}

/// What gives rows in order.
trait Rows {
    /// Puts row `index`, not above the last, in `out` and returns the bytes that are data.
    ///
    /// # Errors
    ///
    /// Whatever decoding fails with, but damage, which is warned.
    fn read(
        &mut self,
        job: &Job<'_>,
        layout: &Layout,
        index: u32,
        out: &mut [u8],
    ) -> Result<usize, PdfError>;
}

impl Rows for HeldRows {
    fn read(
        &mut self,
        job: &Job<'_>,
        layout: &Layout,
        index: u32,
        out: &mut [u8],
    ) -> Result<usize, PdfError> {
        let start = usize::try_from(index)
            .unwrap_or(usize::MAX)
            .saturating_mul(layout.stride);
        let source = self.image.data.get(start..).unwrap_or_default();
        let count = source.len().min(out.len());
        if count < out.len() {
            job.warn("8.9.5", "an image's data is short")?;
        }
        for (slot, &byte) in out.iter_mut().zip(source) {
            *slot = byte;
        }
        Ok(count)
    }
}

impl Rows for Jpeg<'_> {
    fn read(
        &mut self,
        job: &Job<'_>,
        layout: &Layout,
        index: u32,
        out: &mut [u8],
    ) -> Result<usize, PdfError> {
        while !self.ended && self.next <= index {
            let wanted = self.next == index;
            job.spend(self.work(layout.stride, wanted))?;
            self.next = self.next.saturating_add(1);
            let before = self.decoder.read();
            let read = self.decoder.row(out, wanted);
            // Each byte of entropy-coded data costs about as much as a unit.
            job.spend(self.decoder.read().saturating_sub(before))?;
            match job.tolerate(read, "8.9.5", "an image's data is damaged")? {
                Some(true) if wanted => return Ok(layout.stride),
                Some(true) => {}
                // Damage to detail only is reported at the last row, which is then whole.
                None if wanted && self.next == layout.height => {
                    self.ended = true;
                    return Ok(layout.stride);
                }
                Some(false) | None => self.ended = true,
            }
        }
        Ok(0)
    }
}

impl Rows for Plain<'_> {
    fn read(
        &mut self,
        job: &Job<'_>,
        layout: &Layout,
        index: u32,
        out: &mut [u8],
    ) -> Result<usize, PdfError> {
        let mut passed = u64::from(index.saturating_sub(self.next))
            .saturating_mul(u64::try_from(layout.stride).unwrap_or(u64::MAX));
        self.next = index.saturating_add(1);
        if passed > 0 {
            let mut scratch = [0; STEP];
            while passed > 0 {
                let most = usize::try_from(passed).unwrap_or(STEP).min(STEP);
                let count = self.pulled(job, scratch.get_mut(..most).unwrap_or_default())?;
                if count == 0 {
                    return Ok(0);
                }
                passed = passed.saturating_sub(u64::try_from(count).unwrap_or(u64::MAX));
            }
        }
        let mut filled = 0_usize;
        while let Some(rest) = out.get_mut(filled..).filter(|rest| !rest.is_empty()) {
            match self.pulled(job, rest)? {
                0 => break,
                count => filled = filled.saturating_add(count),
            }
        }
        Ok(filled)
    }
}

impl Rows for SwathRows {
    fn read(
        &mut self,
        _job: &Job<'_>,
        _layout: &Layout,
        index: u32,
        out: &mut [u8],
    ) -> Result<usize, PdfError> {
        let swath = &*self.swath;
        let first = u32::try_from(swath.window.top).unwrap_or(0);
        let Some(place) = index
            .checked_sub(first)
            .and_then(|below| usize::try_from(below).ok())
        else {
            return Ok(0);
        };
        let width = swath.span.len();
        let start = place.saturating_mul(width);
        let held = swath.data.get(start..start.saturating_add(width));
        if let (Some(slots), Some(source)) = (out.get_mut(swath.span.clone()), held) {
            slots.copy_from_slice(source);
        }
        let valid = swath.valid.get(place).copied().unwrap_or(0);
        Ok(usize::try_from(valid).unwrap_or(0))
    }
}

impl Swath {
    /// The bytes it holds.
    fn bytes(&self) -> u64 {
        held_bytes(self.span.len(), self.valid.len())
    }
}

impl<'data> Cursor<'data> {
    /// The rows of `image`.
    pub(super) fn decoded(image: &Rc<Decoded>) -> Self {
        let rows = HeldRows {
            image: Rc::clone(image),
        };
        Self::new(image.layout, Box::new(rows), None)
    }

    /// A decoder of the JPEG `open` gives with `parameters`, kept as of `stages` where given.
    pub(super) fn jpeg(
        job: &Job<'_>,
        open: &dct::Open<'_, 'data>,
        parameters: &Object,
        stages: Option<u64>,
    ) -> Result<Self, PdfError> {
        let transform = match *data::entry(job, parameters, b"ColorTransform")?.value() {
            Object::Integer(value) => Some(value != 0),
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => None,
        };
        let fits = |bytes| job.fits(bytes);
        // Each stage before `DCTDecode`, its own stage aside.
        let before = stages.unwrap_or(1).saturating_sub(1);
        let bounds = dct::Bounds {
            capped: job.capped().code(),
            fits: &fits,
            limit: job.objects.decoded_limit(),
            stages: before.saturating_mul(STAGE),
        };
        let decoder = dct::Decoder::new(open, transform, &bounds, OPERATION)?;
        let frame = decoder.frame();
        // The frame outranks Width and Height. What the decoder holds it checked itself.
        let size = [frame.width, frame.height];
        let layout = Layout::new(job, size, usize::from(frame.components), 8)?;
        // Every scan reads through stages of its own.
        let kept = stages.map(|_count| decoder.held().saturating_add(STAGE));
        // Counted beside all else held, such as the decoder of the image's mask.
        let lent = Rc::new(());
        job.lend(&lent, decoder.held())?;
        let rows = Jpeg {
            decoder,
            ended: false,
            next: 0,
        };
        Ok(Self {
            _lent: Some(lent),
            ..Self::new(layout, Box::new(rows), kept)
        })
    }

    /// The layout.
    pub(super) const fn layout(&self) -> Layout {
        self.layout
    }

    /// A cursor before the first row.
    fn new(layout: Layout, rows: Box<dyn Rows + 'data>, size: Option<u64>) -> Self {
        let row = u64::try_from(layout.stride).unwrap_or(u64::MAX);
        Self {
            _lent: None,
            at: None,
            layout,
            row: Vec::new(),
            rows,
            // The row, and the two a predictor holds.
            size: size.map(|held| held.saturating_add(row.saturating_mul(3))),
            valid: 0,
        }
    }

    /// The rows of `samples`, kept as of `stages` where given, else their bytes work too.
    pub(super) fn plain(
        layout: Layout,
        samples: Box<dyn Pull + 'data>,
        stages: Option<u64>,
    ) -> Self {
        let rows = Plain {
            data: samples,
            ended: false,
            next: 0,
            uncounted: stages.is_none(),
        };
        let kept = stages.map(|count| STAGE.saturating_mul(count.saturating_add(1)));
        Self::new(layout, Box::new(rows), kept)
    }

    /// The row read last, as far as its data came.
    pub(super) fn row(&self) -> &[u8] {
        self.row.get(..self.valid).unwrap_or_default()
    }

    /// Reads row `index`, not above the last.
    pub(super) fn seek(&mut self, job: &Job<'_>, index: u32) -> Result<(), PdfError> {
        if self.at == Some(index) {
            return Ok(());
        }
        if self.row.is_empty() {
            job.spend(u64::try_from(self.layout.stride).unwrap_or(u64::MAX))?;
            self.row = room(self.layout.stride)?;
            self.row.resize(self.layout.stride, 0);
        }
        self.valid = self.rows.read(job, &self.layout, index, &mut self.row)?;
        self.at = Some(index);
        Ok(())
    }

    /// The rows of `swath`, within its window.
    pub(super) fn swathed(swath: &Rc<Swath>) -> Self {
        let rows = SwathRows {
            swath: Rc::clone(swath),
        };
        Self::new(swath.layout, Box::new(rows), None)
    }
}

impl<'doc> Cursors<'doc> {
    /// Closes those of images that end above page row `top`.
    pub(super) fn close(&mut self, top: i32) {
        self.open.entries_mut().retain(|kept| kept.until > top);
        self.swaths.retain(|swath| swath.until > top);
    }

    /// The bytes the decoders kept open hold.
    fn decoders(&self) -> u64 {
        self.open
            .entries()
            .iter()
            .filter_map(|kept| kept.cursor.size)
            .fold(0, u64::saturating_add)
    }

    /// Counts a turned image of `layout`, which the pass that measures meets.
    pub(super) fn expect(&mut self, layout: &Layout) {
        self.turned = self.turned.saturating_add(layout.whole());
    }

    /// Holds of what `from` gives the texels of `window`, for the page rows above `until`.
    pub(super) fn hold(
        &mut self,
        job: &Job<'_>,
        (id, from): (ObjectId, &mut Cursor<'doc>),
        (window, until): (Area, i32),
    ) -> Result<Rc<Swath>, PdfError> {
        let layout = from.layout;
        let span = spanned(&layout, window);
        let bytes = held_bytes(span.len(), window.height());
        self.make_room(bytes);
        job.reserve(bytes)?;
        job.spend(bytes >> 3_u32)?;
        let size = span.len().checked_mul(window.height());
        let count = size.ok_or_else(|| job.limit())?;
        let mut data = room(count)?;
        let mut valid = room(window.height())?;
        for row in window.top..window.bottom {
            from.seek(job, u32::try_from(row).unwrap_or(0))?;
            let source = from.row.get(span.clone());
            data.extend_from_slice(source.ok_or_else(|| job.objects.malformed())?);
            valid.push(u32::try_from(from.valid).unwrap_or(u32::MAX));
        }
        let made = Rc::new(Swath {
            data,
            id,
            layout,
            span,
            until,
            valid,
            window,
        });
        self.swaths.push(Rc::clone(&made));
        Ok(made)
    }

    /// The layout of the stream `id`, where a decoder of it is open or a swath is held.
    pub(super) fn known(&self, id: ObjectId) -> Option<Layout> {
        let open = self
            .open
            .entries()
            .iter()
            .find(|kept| kept.id == id)
            .map(|found| found.cursor.layout);
        let held = || self.swaths.iter().find(|swath| swath.id == id);
        open.or_else(|| held().map(|found| found.layout))
    }

    /// Drops the least recent swaths until `more` bytes fit beside what is kept.
    fn make_room(&mut self, more: u64) {
        let mut total = self
            .swaths
            .iter()
            .map(|swath| swath.bytes())
            .fold(self.decoders().saturating_add(more), u64::saturating_add);
        while total > KEPT && !self.swaths.is_empty() {
            let dropped = self.swaths.remove(0);
            total = total.saturating_sub(dropped.bytes());
        }
    }

    /// Offers the decoders kept to `job`, so that they give way when room is needed.
    pub(super) fn offer(&mut self, job: &Job<'doc>) {
        self.open.offer(job);
    }

    /// Keeps `cursor` of the stream `id`, drawn down to page row `until`; one alone may be larger
    /// than all.
    pub(super) fn put(&mut self, id: ObjectId, until: i32, cursor: Cursor<'doc>) {
        let Some(size) = cursor.size else {
            return;
        };
        {
            let mut open = self.open.entries_mut();
            let mut total = open
                .iter()
                .filter_map(|kept| kept.cursor.size)
                .fold(size, u64::saturating_add);
            while total > CURSORS && !open.is_empty() {
                let closed = open.remove(0);
                total = total.saturating_sub(closed.cursor.size.unwrap_or(0));
            }
            open.push(Kept { cursor, id, until });
        }
        self.make_room(0);
    }

    /// The bytes a swath of an image of `layout` may hold: its share among all that are turned, so
    /// that those read together, and those drawn one after another, all keep theirs.
    pub(super) fn spare(&self, job: &Job<'_>, layout: &Layout) -> u64 {
        let most = KEPT
            .saturating_sub(self.decoders())
            .min(job.objects.decoded_limit());
        let whole = layout.whole();
        let all = u128::from(self.turned.max(whole));
        let share = u128::from(most).saturating_mul(u128::from(whole));
        u64::try_from(share.checked_div(all).unwrap_or(0)).unwrap_or(most)
    }

    /// A swath of the stream `id` with all of `window`.
    pub(super) fn swath(
        &mut self,
        id: ObjectId,
        layout: Layout,
        window: Area,
    ) -> Option<Rc<Swath>> {
        let index = self.swaths.iter().position(|held| {
            held.id == id
                && held.layout == layout
                && held.window.left <= window.left
                && held.window.right >= window.right
                && held.window.top <= window.top
                && held.window.bottom >= window.bottom
        })?;
        let found = self.swaths.remove(index);
        self.swaths.push(Rc::clone(&found));
        Some(found)
    }

    /// The furthest decoder of the stream `id` that can still give row `row`.
    pub(super) fn take(&self, id: ObjectId, layout: Layout, row: u32) -> Option<Cursor<'doc>> {
        let mut open = self.open.entries_mut();
        let (index, _) = open
            .iter()
            .enumerate()
            .filter(|&(_, kept)| {
                kept.id == id
                    && kept.cursor.layout == layout
                    && kept.cursor.at.is_none_or(|at| at <= row)
            })
            .max_by_key(|&(_, kept)| kept.cursor.at)?;
        Some(open.remove(index).cursor)
    }
}

impl Decoded {
    /// What `held` is; `colored` when the dictionary gives no space, so the data's counts.
    pub(super) fn new(job: &Job<'_>, held: codec::Held, colored: bool) -> Result<Self, PdfError> {
        let info = held.info;
        job.reserve(u64::try_from(held.data.len()).unwrap_or(u64::MAX))?;
        let components = usize::from(info.components);
        let layout = Layout::new(
            job,
            [info.width, info.height],
            components,
            u32::from(info.bits),
        )?;
        let alpha = held.alpha.filter(|channel| *channel < components);
        let ycc = matches!(held.implied, Some(codec::Implied::Ycc));
        let mut colours = match held.implied {
            Some(codec::Implied::Cmyk) => Some(4),
            Some(codec::Implied::Gray) => Some(1),
            Some(codec::Implied::Rgb | codec::Implied::Ycc) => Some(3),
            Some(codec::Implied::Profile(_)) | None => None,
        };
        let found = if colored && let Some(codec::Implied::Profile(bytes)) = held.implied {
            // The profile's colour space (ICC.1:2022, 7.2.6).
            colours = match bytes.get(16..20) {
                Some(b"GRAY") => Some(1),
                Some(b"RGB " | b"Lab ") => Some(3),
                Some(b"CMYK") => Some(4),
                Some(_) | None => None,
            };
            match colours {
                Some(count) => colour::profile(job, &bytes, count)?,
                None => None,
            }
        } else {
            None
        };
        let (space, profiled) = found.map_or((None, 0), |(made, bytes)| (Some(made), bytes));
        Ok(Self {
            alpha,
            colours,
            data: held.data,
            layout,
            premultiplied: held.premultiplied,
            profiled,
            space,
            ycc,
        })
    }
}

impl Decodings {
    /// What is held of the image `key`.
    pub(super) fn get(&self, key: Key) -> Option<Decoding> {
        let mut held = self.held.entries_mut();
        let index = held.iter().position(|entry| entry.key == key)?;
        let entry = held.remove(index);
        held.push(entry.clone());
        Some(entry)
    }

    /// Offers the images held to `job`, so that they give way when room is needed.
    pub(super) fn offer(&mut self, job: &Job<'_>) {
        self.held.offer(job);
    }

    /// Holds `result` as the image `key`; one alone may be larger than all.
    pub(super) fn put(&self, key: Key, result: Option<Rc<Decoded>>) {
        self.trim(held_size(result.as_ref()));
        self.held.entries_mut().push(Decoding { key, result });
    }

    /// Drops the least recent until `more` bytes fit; before a decode, so that what it drops is not
    /// alive beside what decodes.
    pub(super) fn trim(&self, more: u64) {
        let mut held = self.held.entries_mut();
        let mut total = held
            .iter()
            .map(|entry| held_size(entry.result.as_ref()))
            .fold(more, u64::saturating_add);
        while total > DECODED && !held.is_empty() {
            let dropped = held.remove(0);
            total = total.saturating_sub(held_size(dropped.result.as_ref()));
        }
    }
}

impl Jpeg<'_> {
    /// The work of a row of `stride` samples, beside its data's bytes, more when `converted`:
    /// each scan after the first walks every block too, padded to whole blocks.
    fn work(&self, stride: usize, converted: bool) -> u64 {
        let samples = u64::try_from(stride).unwrap_or(u64::MAX);
        let more = self.decoder.scanned().saturating_sub(1);
        let colours = if converted { samples >> 2_u32 } else { 0 };
        let walked = self.decoder.padded().max(samples);
        (samples >> 1_u32)
            .saturating_add(colours)
            .saturating_add(walked.saturating_mul(more) >> 2_u32)
    }
}

impl Plain<'_> {
    /// Pulls into `out`; 0 at the end or at damage, which is warned.
    fn pulled(&mut self, job: &Job<'_>, out: &mut [u8]) -> Result<usize, PdfError> {
        if self.ended {
            return Ok(0);
        }
        let pulled = self.data.pull(out);
        let count = match job.tolerate(pulled, "8.9.5", "an image's data is damaged")? {
            Some(0) => {
                job.warn("8.9.5", "an image's data is short")?;
                0
            }
            Some(count) => count,
            None => 0,
        };
        self.ended = count == 0;
        let units = if self.uncounted {
            count
        } else {
            // Beside each stage's bytes.
            (count >> 1_u32).saturating_add(1)
        };
        job.spend(u64::try_from(units).unwrap_or(u64::MAX))?;
        Ok(count)
    }
}

/// The bytes an image held decoded counts.
pub(super) fn held_size(found: Option<&Rc<Decoded>>) -> u64 {
    let (bytes, profiled) = found.map_or((0, 0), |image| (image.data.len(), image.profiled));
    u64::try_from(bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(profiled)
        .saturating_add(STAGE)
}

/// The bytes of a swath of `rows`, each of `width` bytes, and how much of each is data.
fn held_bytes(width: usize, rows: usize) -> u64 {
    let each = u64::try_from(width).unwrap_or(u64::MAX).saturating_add(4);
    each.saturating_mul(u64::try_from(rows).unwrap_or(u64::MAX))
        .saturating_add(SLOT)
}

/// The bytes of a row that have the columns of `window`.
fn spanned(layout: &Layout, window: Area) -> Range<usize> {
    let depth = u64::try_from(layout.components)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::from(layout.bits));
    let bits = |column: i32| u64::try_from(column).unwrap_or(0).saturating_mul(depth);
    let byte = |count: u64| {
        usize::try_from(count)
            .unwrap_or(usize::MAX)
            .min(layout.stride)
    };
    byte(bits(window.left) >> 3_u32)..byte(bits(window.right).div_ceil(8))
}

/// The bytes of a swath of `window`.
pub(super) fn swath_bytes(layout: &Layout, window: Area) -> u64 {
    held_bytes(spanned(layout, window).len(), window.height())
}
