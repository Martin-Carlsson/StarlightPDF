//! Images, their masks and their samples as paint (ISO 32000-2:2020, 8.9, 11.6.4.3, 11.6.5.3).
//!
//! Each band takes the samples it needs, in the order of the image's rows, into two planes: the
//! shape, and the colour with the opacity.

mod cursor;
mod sampler;
mod samples;

use alloc::rc::Rc;
use core::slice;

use crate::objects::ObjectId;
use crate::overlay::Object;
use crate::pull::Pull;
use crate::render::colour::{self, Intent, Space};
use crate::render::data::{self, Chunked, Chunks, Reopen, Resolved};
use crate::render::fixed::Fixed;
use crate::render::geometry::Area;
use crate::render::interpret::{Run, paint, state};
use crate::render::job::{BAND_ROWS, Job};
use crate::render::plane::Plane;
use crate::render::surface::{Paint, Sample, Shape};
use crate::render::{codec, function};
use crate::{PdfError, stream};
use cursor::{Cursor, Cursors, Decoded, Decodings, Key, swath_bytes};
use sampler::{Placed, Texels, Walks, Written};
use samples::{Cell, Colouring, Decode, Layout, Samples, WHOLE};

/// What the predictors before a JPEG may hold, in windows: each of its scans has its own.
const SCAN_ROWS: u64 = 1;

/// Work of a pixel of an image.
const PIXEL_WORK: u64 = 3;

/// Work of a stage of decoding opened.
const STAGE_WORK: u64 = 4096;

/// What the predictors of one image may hold, in windows: two of the widest rows, and half again.
const STREAM_ROWS: u64 = 192;

/// The bytes of a window.
const WINDOW: u64 = 0x1_0000;

/// The message for an image that cannot be drawn.
const UNUSABLE: &str = "an image cannot be drawn";

/// Work of joining a texel and its mask's, to average.
const VEIL_WORK: u64 = 2;

/// The message for a mask that cannot be used.
const UNUSABLE_MASK: &str = "an image's mask cannot be used";

/// How the data of an image is coded.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Coding {
    /// `DCTDecode`.
    Dct,
    /// `CCITTFaxDecode`.
    Fax,
    /// `JBIG2Decode`.
    Jbig2,
    /// `JPXDecode`.
    Jpx,
    /// The samples themselves.
    Plain,
}

/// An image ready to be sampled.
struct Drawn<'data> {
    /// The image; none while there is only its size to count.
    main: Option<Part<'data>>,
    /// Its own mask outranks the soft mask of the state.
    masked: bool,
    /// What it is.
    role: Role,
    /// Its mask.
    veil: Option<Part<'data>>,
}

/// What an image's dictionary declares.
struct Header {
    /// How it is coded.
    coding: Coding,
    /// The layout it declares; a JPEG's frame declares its own.
    declared: Option<Layout>,
    /// JPX samples index its colour space.
    indices: bool,
    /// The coding's parameters.
    parameters: Object,
    /// What it is.
    role: Role,
    /// Its colour space, where it gives one.
    space: Option<Rc<Space>>,
    /// JPX data is decoded whole: it holds indices, whose means would be other colours, or is
    /// taken texel by texel with a mask.
    whole: bool,
}

/// The decoders kept open and the images held.
#[derive(Default)]
pub(crate) struct Images<'doc> {
    /// The decoders.
    cursors: Cursors<'doc>,
    /// The images.
    decoded: Decodings,
}

/// The masks beside an image.
struct Masks<'data> {
    /// `SMaskInData`.
    inside: i64,
    /// The colour key.
    key: Vec<[i64; 2]>,
    /// The `Matte` of the soft one.
    matte: Vec<Fixed>,
    /// The mask image.
    part: Option<Part<'data>>,
}

/// One image to sample: an image or the mask of one.
struct Part<'data> {
    /// What the rows mean.
    samples: Samples,
    /// Where they come from.
    source: Source<'data>,
}

/// An image's colour and opacity as paint.
struct Picture {
    /// The pixels.
    pixels: Plane<[u8; 4]>,
}

/// What a band wants of the rows of an image's parts.
struct Reach<'walks> {
    /// The place in the band.
    view: &'walks View,
    /// The walks.
    walks: &'walks Walks,
}

/// An image on the band's pixels.
struct Raster {
    /// The shape.
    cover: Plane<u8>,
    /// Its own mask outranks the soft mask of the state.
    masked: bool,
    /// The colour and the opacity; none for a stencil.
    pixels: Option<Plane<[u8; 4]>>,
}

/// What an image is.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Role {
    /// Colour.
    Colour,
    /// An image mask, which paints where its value is 0 (8.9.6.2).
    Cutout,
    /// A soft-mask image.
    Soft,
}

/// An image's parts opened to be read.
struct Scanned<'data> {
    /// The image.
    main: (Cursor<'data>, Samples),
    /// The mask of its size, read beside it.
    veil: Option<(Cursor<'data>, Samples)>,
}

/// Where rows come from.
struct Source<'data> {
    /// What is held decoded.
    decoded: Option<Rc<Decoded>>,
    /// A decoder at the start, unless one is known to be open.
    fresh: Option<Cursor<'data>>,
    /// The layout.
    layout: Layout,
    /// The stream, which a decoder may open again, and the parameters.
    stream: Option<(Resolved, Object)>,
}

/// An image's place in this band.
struct View {
    /// Alpha is shape.
    alpha_is_shape: bool,
    /// The pixels.
    area: Area,
    /// The map.
    placed: Placed,
}

impl Paint for Picture {
    fn row(
        &mut self,
        _job: &Job<'_>,
        row: i32,
        left: i32,
        out: &mut [Sample],
    ) -> Result<(), PdfError> {
        let from = usize::try_from(left.saturating_sub(self.pixels.area.left));
        let values = from
            .ok()
            .and_then(|start| self.pixels.row(row).get(start..))
            .unwrap_or_default();
        for (sample, value) in out.iter_mut().zip(values) {
            let [red, green, blue, opacity] =
                value.map(|part| u16::from(part).wrapping_mul(0x0101));
            *sample = Sample {
                color: [red, green, blue],
                opacity,
                shape: u16::MAX,
            };
        }
        Ok(())
    }
}

impl Texels for Scanned<'_> {
    fn bits(
        &mut self,
        job: &Job<'_>,
        span: (u32, u32),
    ) -> Result<Option<[(Cell, u64); 2]>, PdfError> {
        if self.veil.is_some() {
            return Ok(None);
        }
        self.main.1.bits(job, self.main.0.row(), span)
    }

    fn cell(&mut self, job: &Job<'_>, column: u32) -> Result<Option<Cell>, PdfError> {
        let Some(veil) = self.veil.as_mut() else {
            return self.main.1.unveiled(job, (self.main.0.row(), column));
        };
        let found = veil.1.unveiled(job, (veil.0.row(), column))?;
        let under = found.unwrap_or(Cell { shape: 0, ..WHOLE });
        self.main.1.cell(job, (self.main.0.row(), column), under)
    }

    fn read(&mut self, job: &Job<'_>, row: u32) -> Result<(), PdfError> {
        if let Some(veil) = self.veil.as_mut() {
            veil.0.seek(job, row)?;
        }
        self.main.0.seek(job, row)
    }

    fn work(&self) -> u64 {
        // A veiled texel is read twice, then joined.
        let veiled = self
            .veil
            .as_ref()
            .map_or(0, |veil| veil.1.work().saturating_add(VEIL_WORK));
        self.main.1.work().saturating_add(veiled)
    }
}

impl Header {
    /// Decodes `encoded`, which `again` opens anew, held whole, toward what `view` shows.
    fn decode(
        &self,
        job: &Job<'_>,
        view: &View,
        (encoded, again): (&mut dyn Pull, &Reopen<'_>),
    ) -> Result<Decoded, PdfError> {
        let (size, most) = (self.size(), codec::readable(job));
        let held = match self.coding {
            Coding::Jpx => {
                // The decoder counts it.
                let mut read = data::all(job, encoded, again, (most, codec::jpx::ROOM))?;
                codec::jpx::decode(job, &mut read, self.toward(view), self.indices)?
            }
            Coding::Fax => {
                let read = data::shared(job, encoded, again, most)?;
                codec::fax::decode(job, &read, &self.parameters, size)?
            }
            Coding::Jbig2 => {
                let read = data::shared(job, encoded, again, most)?;
                let shared = data::entry(job, &self.parameters, b"JBIG2Globals")?;
                let globals = if matches!(*shared.value(), Object::Null) {
                    None
                } else {
                    let open = || data::coded(job, &shared).map(|found| found.data);
                    Some(data::shared(job, &mut open()?, &open, most)?)
                };
                codec::jbig2::decode(job, &read, globals.as_deref().map(AsRef::as_ref))?
            }
            Coding::Dct | Coding::Plain => return Err(job.objects.malformed()),
        };
        self.made(job, held)
    }

    /// The image `held` decodes to.
    fn made(&self, job: &Job<'_>, held: codec::Held) -> Result<Decoded, PdfError> {
        let size = self.size();
        let colored = self.role == Role::Colour && self.space.is_none();
        let mut image = Decoded::new(job, held, colored)?;
        let layout = &mut image.layout;
        if self.coding != Coding::Jpx && [layout.columns, layout.height] != size {
            job.warn("8.9.5", "an image's data is not of its Width and Height")?;
            let [width, height] = size;
            layout.columns = layout.columns.min(width);
            layout.height = layout.height.min(height);
        }
        Ok(image)
    }

    /// Reads the dictionary `owner` of an image of `role` whose last filter is `filter`.
    fn read(
        run: &mut Run<'_, '_>,
        owner: &Object,
        filter: Option<(Vec<u8>, Object)>,
        role: Role,
    ) -> Result<Self, PdfError> {
        let job = run.job;
        let (name, parameters) = filter.unwrap_or((Vec::new(), Object::Null));
        let coding = match name.as_slice() {
            b"CCITTFaxDecode" => Coding::Fax,
            b"DCTDecode" => Coding::Dct,
            b"JBIG2Decode" => Coding::Jbig2,
            b"JPXDecode" => Coding::Jpx,
            _ => Coding::Plain,
        };
        let mut space = None;
        if role == Role::Colour {
            let family = data::entry(job, owner, b"ColorSpace")?;
            if matches!(*family.value(), Object::Null) {
                if coding == Coding::Plain {
                    return Err(job.objects.malformed());
                }
                // JPX data brings its own.
                if coding != Coding::Jpx {
                    job.warn("8.9.5", "an image has no colour space")?;
                }
            } else {
                let found = colour::resolve(job, &mut run.caches.spaces, &run.scope, &family)?;
                space = Some(found);
            }
        }
        let mut whole = role != Role::Colour;
        let mut indices = false;
        if coding == Coding::Jpx && !whole {
            // Only an Indexed space's default Decode follows the bits (8.9.5.2).
            indices = space.as_ref().is_some_and(|found: &Rc<Space>| {
                found.default_decode(0, 1) != found.default_decode(0, 3)
            });
            whole = indices || !matches!(*data::entry(job, owner, b"SMask")?.value(), Object::Null);
        }
        let declared = if coding == Coding::Dct {
            None
        } else {
            let size = [side(job, owner, b"Width")?, side(job, owner, b"Height")?];
            let (components, bits) = match coding {
                Coding::Fax | Coding::Jbig2 => (1, 1),
                Coding::Jpx | Coding::Dct => (1, 8),
                Coding::Plain => (
                    space.as_ref().map_or(1, |found| found.components()),
                    depth(job, owner, role)?,
                ),
            };
            Some(Layout::new(job, size, components, bits)?)
        };
        Ok(Self {
            coding,
            declared,
            indices,
            parameters,
            role,
            space,
            whole,
        })
    }

    /// The size it declares.
    fn size(&self) -> [u32; 2] {
        self.declared
            .map_or([1; 2], |layout| [layout.columns, layout.height])
    }

    /// The source of the image `XObject` `stream` whose data opened as `samples`; `None` where
    /// nothing can be decoded, or not in the pass that measures.
    fn source<'doc>(
        &self,
        job: &Job<'doc>,
        images: &Images<'doc>,
        view: &View,
        (stream, samples): (&Resolved, Box<dyn Pull + 'doc>),
    ) -> Result<Option<Source<'doc>>, PdfError> {
        let open = || data::coded(job, stream).map(|found| found.data);
        let cursor = match self.coding {
            Coding::Plain => {
                let layout = self.declared.ok_or_else(|| job.objects.malformed())?;
                Cursor::plain(
                    layout,
                    samples,
                    Some(stages(job, stream.value(), STREAM_ROWS)?),
                )
            }
            Coding::Dct => {
                let reopened = Some((stream.clone(), self.parameters.clone()));
                if let Some(layout) = stream.id().and_then(|id| images.cursors.known(id)) {
                    return Ok(Some(Source {
                        decoded: None,
                        fresh: None,
                        layout,
                        stream: reopened,
                    }));
                }
                let count = stages(job, stream.value(), SCAN_ROWS)?;
                let cursor = Cursor::jpeg(job, &open, &self.parameters, Some(count))?;
                return Ok(Some(Source {
                    decoded: None,
                    layout: cursor.layout(),
                    fresh: Some(cursor),
                    stream: reopened,
                }));
            }
            Coding::Fax | Coding::Jbig2 | Coding::Jpx => {
                if job.planes.measuring() {
                    return Ok(None);
                }
                let key = stream.id().map(|id| Key {
                    id,
                    indices: self.indices,
                    mask: self.role != Role::Colour,
                    toward: self.toward(view),
                });
                let found = if let Some(entry) = key.and_then(|known| images.decoded.get(known)) {
                    entry.result
                } else {
                    // What it decodes to at most: a bit a pixel, or for JPX five components of 16.
                    let [width, height] = self.size().map(u64::from);
                    let most = if self.coding == Coding::Jpx {
                        width.saturating_mul(height).saturating_mul(10)
                    } else {
                        width.div_ceil(8).saturating_mul(height)
                    };
                    images.decoded.trim(most);
                    let again = || data::coded(job, stream).map(|found| found.data);
                    let made =
                        again().and_then(|mut first| self.decode(job, view, (&mut first, &again)));
                    let held = job.tolerate(made, "8.9.5", UNUSABLE)?.map(Rc::new);
                    if let Some(image) = held.as_ref() {
                        job.lend(image, cursor::held_size(Some(image)))?;
                    }
                    if let Some(known) = key {
                        images.decoded.put(known, held.clone());
                    }
                    held
                };
                return Ok(found.map(Source::held));
            }
        };
        Ok(Some(Source {
            decoded: None,
            layout: cursor.layout(),
            fresh: Some(cursor),
            stream: Some((stream.clone(), self.parameters.clone())),
        }))
    }

    /// The size worth decoding JPX data toward.
    fn toward(&self, view: &View) -> [u32; 2] {
        let size = self.size();
        if self.coding == Coding::Jpx && !self.whole {
            view.placed.target(size)
        } else {
            size
        }
    }
}

impl Part<'_> {
    /// Its columns and rows.
    const fn size(&self) -> [u32; 2] {
        let layout = self.samples.layout();
        [layout.columns, layout.height]
    }
}

impl Raster {
    /// Paints it: a stencil with the fill colour, the rest as it is.
    fn paint(self, run: &mut Run<'_, '_>) -> Result<(), PdfError> {
        let shape = Shape {
            area: self.cover.area,
            cover: Some(&self.cover),
        };
        let Some(pixels) = self.pixels else {
            return paint::fill(run, shape, paint::Which::Fill);
        };
        let mut how = paint::composite(run, paint::Which::Fill);
        if self.masked {
            how.soft = None;
        }
        run.surface
            .draw(run.job, shape, &mut Picture { pixels }, &how)
    }
}

impl Reach<'_> {
    /// The texels of the most bands from this one that fit `spare` bytes, and the page row after
    /// them; none where no later band would gain.
    fn ahead(&self, job: &Job<'_>, layout: &Layout, spare: u64) -> Option<(Area, i32)> {
        let (view, walks) = (self.view, self.walks);
        let last = view.placed.bounds().bottom;
        let band = i32::try_from(job.area.height()).ok()?.max(1_i32);
        let mut found = None;
        let (mut few, mut many) = (1_i32, i32::try_from(BAND_ROWS).ok()?);
        while few <= many {
            let count = few.saturating_add(many) >> 1_u32;
            let until = job
                .area
                .top
                .saturating_add(band.saturating_mul(count))
                .min(last);
            let area = Area {
                bottom: until.max(view.area.bottom),
                ..view.area
            };
            let window = walks.window(&view.placed, area);
            if swath_bytes(layout, window) <= spare {
                if until > job.area.bottom {
                    found = Some((window, until));
                }
                few = count.saturating_add(1);
            } else {
                many = count.saturating_sub(1);
            }
        }
        found
    }

    /// The decoder to read the band's rows from. A turned image's decoding would start again at
    /// each band, so it fills a swath for as many bands as may be held.
    fn cursor<'doc, 'data>(
        &self,
        job: &Job<'doc>,
        source: Source<'data>,
        cache: Option<&mut Cursors<'data>>,
    ) -> Result<Cursor<'data>, PdfError>
    where
        'doc: 'data,
    {
        let (view, walks) = (self.view, self.walks);
        let first = walks.first();
        let streamed = source.id().filter(|_id| !view.placed.rising());
        let Some(id) = streamed else {
            return source.opened(job, cache, first);
        };
        let Some(cursors) = cache else {
            return source.opened(job, None, first);
        };
        let layout = source.layout;
        let wanted = walks.window(&view.placed, view.area);
        if let Some(swath) = cursors.swath(id, layout, wanted) {
            return Ok(Cursor::swathed(&swath));
        }
        let spare = cursors.spare(job, &layout);
        let Some((window, until)) = self.ahead(job, &layout, spare) else {
            return source.opened(job, Some(cursors), first);
        };
        let from = u32::try_from(window.top).unwrap_or(0);
        let mut decoder = source.opened(job, Some(&mut *cursors), from)?;
        let swath = cursors.hold(job, (id, &mut decoder), (window, until))?;
        Ok(Cursor::swathed(&swath))
    }
}

impl<'data> Source<'data> {
    /// The source of what is held decoded.
    fn held(image: Rc<Decoded>) -> Self {
        Self {
            fresh: Some(Cursor::decoded(&image)),
            layout: image.layout,
            decoded: Some(image),
            stream: None,
        }
    }

    /// The stream's object.
    fn id(&self) -> Option<ObjectId> {
        self.stream.as_ref()?.0.id()
    }

    /// The decoder to read from row `first` on: one kept, else one at the start.
    fn opened<'doc>(
        self,
        job: &Job<'doc>,
        cache: Option<&mut Cursors<'data>>,
        first: u32,
    ) -> Result<Cursor<'data>, PdfError>
    where
        'doc: 'data,
    {
        if let Some((id, cursors)) = self.id().zip(cache)
            && let Some(kept) = cursors.take(id, self.layout, first)
        {
            return Ok(kept);
        }
        if let Some(fresh) = self.fresh {
            return Ok(fresh);
        }
        let Some((stored, parameters)) = self.stream else {
            return Err(job.objects.malformed());
        };
        let open = || data::coded(job, &stored).map(|found| found.data);
        let count = stages(job, stored.value(), SCAN_ROWS)?;
        Cursor::jpeg(job, &open, &parameters, Some(count))
    }
}

impl View {
    /// The image of the current matrix; `None` where it paints nothing.
    fn new(run: &Run<'_, '_>) -> Option<Self> {
        let placed = Placed::new(&run.state.matrix)?;
        let area = placed
            .bounds()
            .intersect(run.surface.area())
            .intersect(run.state.clip.area);
        (run.job.planes.measuring() || !area.is_empty()).then_some(Self {
            alpha_is_shape: run.state.alpha_is_shape,
            area,
            placed,
        })
    }
}

/// The planes of an image, nothing in them.
fn blank(job: &Job<'_>, view: &View, role: Role) -> Result<Raster, PdfError> {
    if !job.planes.measuring() {
        // Clearing and sampling each pixel.
        let (width, height) = (view.area.width(), view.area.height());
        let pixels = u64::try_from(width.saturating_mul(height)).unwrap_or(u64::MAX);
        job.spend(pixels.saturating_mul(PIXEL_WORK))?;
    }
    let pixels = if role == Role::Colour {
        Some(Plane::new(&job.planes, view.area, [0_u8; 4])?)
    } else {
        None
    };
    Ok(Raster {
        cover: Plane::new(&job.planes, view.area, 0)?,
        masked: false,
        pixels,
    })
}

/// How a colour image's samples become paint, and whether its own opacity counts; `None` where
/// its space marks nothing.
fn coloured(
    run: &mut Run<'_, '_>,
    owner: &Object,
    header: &Header,
    source: &Source<'_>,
    inside: i64,
) -> Result<Option<Colouring>, PdfError> {
    let job = run.job;
    let count = source.layout.components;
    let jpx = header.coding == Coding::Jpx;
    let decoded = source.decoded.as_deref();
    let given = header.space.clone();
    let mut alpha = decoded.and_then(|image| image.alpha);
    if jpx && alpha.is_none() {
        let known = given
            .as_ref()
            .map(|space| space.components())
            .or_else(|| decoded?.colours);
        let extra = known.map_or_else(
            || count == 2 || count == 5 || (inside > 0 && count == 4),
            |wanted| wanted < count,
        );
        alpha = extra.then(|| count.saturating_sub(1));
    }
    let colours = count.saturating_sub(usize::from(alpha.is_some()));
    let fitting = given.as_ref().filter(|space| space.components() == colours);
    let implied = fitting.is_none();
    if implied && given.is_some() {
        job.warn("8.9.5", "an image's colour space is not of its data")?;
    }
    let space = if let Some(found) = fitting {
        Rc::clone(found)
    } else if let Some(found) = decoded
        .and_then(|image| image.space.as_ref())
        .filter(|space| space.components() == colours)
    {
        Rc::clone(found)
    } else {
        // A space the data names is no guess, and a profile that failed was warned.
        if jpx && decoded.and_then(|image| image.colours) != Some(colours) {
            job.warn(
                "7.4.9",
                "JPX data names no colour space that can be used; its components decide",
            )?;
        }
        colour::device(job, &mut run.caches.spaces, &run.scope, colours)?
    };
    if !space.marks() {
        return Ok(None);
    }
    let named = data::entry(job, owner, b"Intent")?;
    Ok(Some(Colouring {
        alpha,
        intent: intended(named.value(), run.state.intent),
        key: Vec::new(),
        matte: Vec::new(),
        opacity: alpha.is_some() && inside > 0,
        space,
        ycc: implied && colours == 3 && decoded.is_some_and(|image| image.ycc),
    }))
}

/// The map `Decode` of `owner` gives, else `usual`; JPX data ignores it but as a mask (7.4.9).
fn decoding(
    job: &Job<'_>,
    owner: &Object,
    header: &Header,
    bits: u32,
    usual: Vec<[Fixed; 2]>,
) -> Result<Decode, PdfError> {
    if header.coding == Coding::Jpx && header.role != Role::Cutout {
        return Decode::new(bits, usual);
    }
    let read = function::numbers(job, owner, b"Decode");
    let given = job
        .tolerate(read, "8.9.5.2", "an image's Decode cannot be used")?
        .unwrap_or_default();
    if given.is_empty() {
        return Decode::new(bits, usual);
    }
    let (pairs, odd) = given.as_chunks::<2>();
    let count = usual.len();
    if pairs.len() != count || !odd.is_empty() {
        job.warn("8.9.5.2", "an image's Decode is not of its components")?;
    }
    Decode::new(
        bits,
        pairs.get(..count).map_or(usual, <[[Fixed; 2]]>::to_vec),
    )
}

/// The bits per sample of `owner`, which are its own.
fn depth(job: &Job<'_>, owner: &Object, role: Role) -> Result<u32, PdfError> {
    let value = data::entry(job, owner, b"BitsPerComponent")?;
    let found = Fixed::from_object(value.value()).map(Fixed::floor);
    if role == Role::Cutout {
        if found.is_some_and(|bits| bits != 1) {
            job.warn("8.9.6.2", "an image mask is not of one bit")?;
        }
        return Ok(1);
    }
    found
        .and_then(|bits| u32::try_from(bits).ok())
        .ok_or_else(|| job.objects.malformed())
}

/// Paints the image `image`.
pub(crate) fn draw(run: &mut Run<'_, '_>, image: &Resolved) -> Result<(), PdfError> {
    let job = run.job;
    let Some(view) = View::new(run) else {
        return Ok(());
    };
    let made = stored(run, &view, image);
    job.tolerate(made, "8.9.5", UNUSABLE)?
        .flatten()
        .map_or(Ok(()), |raster| raster.paint(run))
}

/// Paints the inline image of `dictionary` whose data as stored `data` gives.
pub(crate) fn draw_inline(
    run: &mut Run<'_, '_>,
    dictionary: &Object,
    data: &mut dyn Pull,
) -> Result<(), PdfError> {
    let job = run.job;
    let Some(view) = View::new(run) else {
        return Ok(());
    };
    let made = inline(run, &view, dictionary, &mut *data);
    job.tolerate(made, "8.9.5", UNUSABLE)?
        .flatten()
        .map_or(Ok(()), |raster| raster.paint(run))
}

/// Inline data, read once in parts, counted while alive; each band reads it again, as it reads its
/// content again.
fn inline_data(job: &Job<'_>, data: &mut dyn Pull) -> Result<Rc<Chunks>, PdfError> {
    let chunks = Rc::new(data::chunked(job, data, codec::readable(job))?);
    job.lend(&chunks, chunks.bytes())?;
    Ok(chunks)
}

/// Whether the key `key` of `owner` is true.
fn flagged(job: &Job<'_>, owner: &Object, key: &[u8]) -> Result<bool, PdfError> {
    data::entry(job, owner, key).map(|found| matches!(*found.value(), Object::Boolean(true)))
}

/// An inline image on the band's pixels (8.9.7); it keeps no decoder.
// Its frame is gone before a stencil's pattern nests content.
#[inline(never)]
fn inline<'data, 'doc>(
    run: &mut Run<'_, 'doc>,
    view: &View,
    owner: &Object,
    stored_data: &'data mut dyn Pull,
) -> Result<Option<Raster>, PdfError>
where
    'doc: 'data,
{
    let job = run.job;
    let Some(role) = role(run, owner)? else {
        return Ok(None);
    };
    // The dictionary is copied, so counted first.
    job.fits(data::size(owner))?;
    let Object::Dictionary(entries) = owner.clone() else {
        return Err(job.objects.malformed());
    };
    let mut opened = stream::undone(job.objects, &job.overlay, entries, Box::new(stored_data))?;
    // What its predictors' rows hold counts while it is read.
    let rows = Rc::new(());
    job.lend(&rows, opened.held)?;
    let header = Header::read(run, owner, opened.image.take(), role)?;
    let source = match header.coding {
        Coding::Plain => header.declared.map(|layout| Source {
            decoded: None,
            fresh: Some(Cursor::plain(layout, opened.data, None)),
            layout,
            stream: None,
        }),
        Coding::Dct => {
            let chunks = inline_data(job, &mut opened.data)?;
            let open = || {
                let reader: Box<dyn Pull + 'data> = Box::new(Chunked::new(Rc::clone(&chunks)));
                Ok(reader)
            };
            let cursor = Cursor::jpeg(job, &open, &header.parameters, None)?;
            Some(Source {
                decoded: None,
                layout: cursor.layout(),
                fresh: Some(cursor),
                stream: None,
            })
        }
        Coding::Fax => {
            if job.planes.measuring() {
                None
            } else {
                let chunks = inline_data(job, &mut opened.data)?;
                let joined = chunks.joined(job)?;
                drop(chunks);
                let size = header.size();
                let held = codec::fax::decode(job, &joined, &header.parameters, size)?;
                drop(joined);
                let image = Rc::new(header.made(job, held)?);
                job.lend(&image, cursor::held_size(Some(&image)))?;
                Some(Source::held(image))
            }
        }
        // Neither may be inline.
        Coding::Jbig2 | Coding::Jpx => return Err(job.objects.malformed()),
    };
    let masks = Masks {
        inside: 0,
        key: if role == Role::Colour {
            keyed(job, owner)?
        } else {
            Vec::new()
        },
        matte: Vec::new(),
        part: None,
    };
    let Some(drawn) = prepared(run, (owner, &header), source, masks)? else {
        return Ok(None);
    };
    rastered(job, view, drawn, None).map(Some)
}

/// The rendering intent `value` names, else `current` (8.6.5.8).
fn intended(value: &Object, current: Intent) -> Intent {
    data::name(value).map_or(current, state::intent)
}

/// The ranges of a colour key in `Mask` (8.9.6.4).
fn keyed(job: &Job<'_>, owner: &Object) -> Result<Vec<[i64; 2]>, PdfError> {
    let value = data::entry(job, owner, b"Mask")?;
    if !matches!(*value.value(), Object::Array(_)) {
        return Ok(Vec::new());
    }
    let read = function::numbers(job, owner, b"Mask");
    let given = job
        .tolerate(read, "8.9.6.4", UNUSABLE_MASK)?
        .unwrap_or_default();
    let (pairs, odd) = given.as_chunks::<2>();
    if !odd.is_empty() {
        job.warn("8.9.6.4", UNUSABLE_MASK)?;
        return Ok(Vec::new());
    }
    Ok(pairs.iter().map(|pair| pair.map(Fixed::floor)).collect())
}

/// The masks beside the image `owner`: a soft one, else the opacity in its data, else an
/// explicit one or a colour key (11.6.4.3).
fn masking<'doc>(
    run: &mut Run<'_, 'doc>,
    view: &View,
    owner: &Object,
    header: &Header,
) -> Result<Masks<'doc>, PdfError> {
    let job = run.job;
    let mut found = Masks {
        inside: 0,
        key: Vec::new(),
        matte: Vec::new(),
        part: None,
    };
    let soft = data::entry(job, owner, b"SMask")?;
    let hard = data::entry(job, owner, b"Mask")?;
    let absent = |value: &Resolved| matches!(*value.value(), Object::Null);
    if header.role != Role::Colour {
        if !absent(&soft) || !absent(&hard) {
            job.warn("8.9.6.2", "an image mask has a mask of its own")?;
        }
        return Ok(found);
    }
    if !absent(&soft) {
        let read = subsidiary(run, view, &soft, Role::Soft);
        if let Some(part) = job.tolerate(read, "11.6.5.3", UNUSABLE_MASK)? {
            found.part = part;
            if found.part.is_some() {
                let matte = function::numbers(job, soft.value(), b"Matte");
                found.matte = job
                    .tolerate(matte, "11.6.5.3", "a soft mask's Matte cannot be used")?
                    .unwrap_or_default();
                return Ok(found);
            }
            // One held is decoded only in the bands, and still outranks Mask.
            if job.planes.measuring() {
                return Ok(found);
            }
        }
    }
    if header.coding == Coding::Jpx {
        let kind = data::entry(job, owner, b"SMaskInData")?;
        found.inside = Fixed::from_object(kind.value()).map_or(0, Fixed::floor);
    }
    if matches!(*hard.value(), Object::Array(_)) {
        found.key = keyed(job, owner)?;
        return Ok(found);
    }
    if !absent(&hard) {
        let read = subsidiary(run, view, &hard, Role::Cutout);
        found.part = job.tolerate(read, "8.9.6.3", UNUSABLE_MASK)?.flatten();
    }
    Ok(found)
}

/// Closes what the bands from row `top` on do not need.
pub(crate) fn next_band(images: &mut Images<'_>, top: i32) {
    images.cursors.close(top);
}

/// What the rows of `source` mean and the masks that count with them; `None` where nothing is
/// painted.
fn prepared<'data>(
    run: &mut Run<'_, '_>,
    (owner, header): (&Object, &Header),
    source: Option<Source<'data>>,
    masks: Masks<'data>,
) -> Result<Option<Drawn<'data>>, PdfError> {
    let job = run.job;
    let mut drawn = Drawn {
        main: None,
        masked: false,
        role: header.role,
        veil: masks.part,
    };
    let Some(from) = source else {
        return Ok(job.planes.measuring().then_some(drawn));
    };
    let layout = from.layout;
    if header.role != Role::Colour {
        let samples = shared(job, owner, header, layout)?;
        drawn.main = Some(Part {
            samples,
            source: from,
        });
        return Ok(Some(drawn));
    }
    let Some(mut how) = coloured(run, owner, header, &from, masks.inside)? else {
        return Ok(None);
    };
    let count = how.space.components();
    let most = u32::from(samples::largest(layout.bits));
    let usual = (0..count)
        .map(|component| how.space.default_decode(component, most))
        .collect();
    let decode = decoding(job, owner, header, layout.bits, usual)?;
    if how.opacity {
        drawn.veil = None;
        let preblended = from
            .decoded
            .as_ref()
            .is_some_and(|image| image.premultiplied);
        if masks.inside == 2 || preblended {
            how.matte = decode.lows();
        }
    } else {
        if masks.inside > 0 {
            job.warn("7.4.9", "SMaskInData without opacity in the data")?;
        }
        if !masks.key.is_empty() {
            if masks.key.len() == count {
                how.key = masks.key;
            } else {
                job.warn("8.9.6.4", UNUSABLE_MASK)?;
            }
        }
        if !masks.matte.is_empty() {
            let size = [layout.columns, layout.height];
            let beside = drawn.veil.as_ref().map(Part::size) == Some(size);
            if beside && masks.matte.len() == count {
                how.matte = masks.matte;
            } else {
                job.warn(
                    "11.6.5.3",
                    "a Matte is not of its image's size and components",
                )?;
            }
        }
    }
    drawn.masked = drawn.veil.is_some() || how.opacity || !how.key.is_empty();
    drawn.main = Some(Part {
        samples: Samples::colour(job, layout, decode, how)?,
        source: from,
    });
    Ok(Some(drawn))
}

/// `drawn` on the band's pixels; decoders are taken from `cache` and kept there.
fn rastered<'doc, 'data>(
    job: &Job<'doc>,
    view: &View,
    drawn: Drawn<'data>,
    mut cache: Option<&mut Cursors<'data>>,
) -> Result<Raster, PdfError>
where
    'doc: 'data,
{
    let mut raster = blank(job, view, drawn.role)?;
    raster.masked = drawn.masked;
    if job.planes.measuring() {
        if let Some(cursors) = cache.filter(|_cursors| !view.placed.rising()) {
            for part in [drawn.main, drawn.veil].into_iter().flatten() {
                if part.source.stream.is_some() {
                    cursors.expect(&part.source.layout);
                }
            }
        }
        return Ok(raster);
    }
    let Some(main) = drawn.main else {
        return Ok(raster);
    };
    // A mask of the same size is read beside it, so colour is averaged by what shows.
    let (beside, after) = match drawn.veil {
        Some(veil) if veil.size() == main.size() => (Some(veil), None),
        other => (None, other),
    };
    let mut target = Written {
        alpha_is_shape: view.alpha_is_shape,
        cover: &mut raster.cover,
        first: true,
        pixels: raster.pixels.as_mut(),
    };
    sampled(job, view, (main, beside), &mut target, cache.as_deref_mut())?;
    if let Some(veil) = after {
        target.first = false;
        sampled(job, view, (veil, None), &mut target, cache)?;
    }
    Ok(raster)
}

/// What the image `owner` is; `None` where colour may not be given (8.7.3.3, 9.6.4).
fn role(run: &Run<'_, '_>, owner: &Object) -> Result<Option<Role>, PdfError> {
    if flagged(run.job, owner, b"ImageMask")? {
        return Ok(Some(Role::Cutout));
    }
    if run.locked_color {
        run.job.warn(
            "8.7.3.3",
            "an image in colour where colour may not be given",
        )?;
        return Ok(None);
    }
    Ok(Some(Role::Colour))
}

/// Takes `main` and the mask of its size beside it to `target`.
fn sampled<'doc, 'data>(
    job: &Job<'doc>,
    view: &View,
    (main, beside): (Part<'data>, Option<Part<'data>>),
    target: &mut Written<'_>,
    mut cache: Option<&mut Cursors<'data>>,
) -> Result<(), PdfError>
where
    'doc: 'data,
{
    let Some(walks) = Walks::new(job, &view.placed, view.area, main.size())? else {
        return Ok(());
    };
    let reach = Reach {
        view,
        walks: &walks,
    };
    let id = main.source.id();
    let cursor = reach.cursor(job, main.source, cache.as_deref_mut())?;
    let mut veil = None;
    let mut veil_id = None;
    if let Some(part) = beside {
        veil_id = part.source.id();
        let opened = reach.cursor(job, part.source, cache.as_deref_mut())?;
        veil = Some((opened, part.samples));
    }
    let mut scanned = Scanned {
        main: (cursor, main.samples),
        veil,
    };
    walks.sample(job, &mut scanned, target)?;
    let until = view.placed.bounds().bottom;
    // A decoder is kept only for the bands below this one that its image reaches.
    if let Some(cursors) = cache.filter(|_| view.placed.rising() && until > job.area.bottom) {
        if let Some(stream) = id {
            cursors.put(stream, until, scanned.main.0);
        }
        if let Some((stream, kept)) = veil_id.zip(scanned.veil) {
            cursors.put(stream, until, kept.0);
        }
    }
    Ok(())
}

/// The samples of a mask.
fn shared(
    job: &Job<'_>,
    owner: &Object,
    header: &Header,
    layout: Layout,
) -> Result<Samples, PdfError> {
    if layout.components > 1 {
        job.warn("8.9.6", "a mask has more than one component")?;
    }
    let usual = vec![[Fixed::default(), Fixed::from_i64(1)]];
    let decode = decoding(job, owner, header, layout.bits, usual)?;
    Ok(Samples::share(layout, decode, header.role == Role::Soft))
}

/// The side `key` of `owner`.
fn side(job: &Job<'_>, owner: &Object, key: &[u8]) -> Result<u32, PdfError> {
    Fixed::from_object(data::entry(job, owner, key)?.value())
        .and_then(|value| u32::try_from(value.floor()).ok())
        .ok_or_else(|| job.objects.malformed())
}

/// The windows the stages that decode `owner`'s data hold, at least one; their predictors' rows
/// (7.4.4.4) may be `most` of them.
fn stages(job: &Job<'_>, owner: &Object, most: u64) -> Result<u64, PdfError> {
    // The filters are at most the 16 that one stream may chain.
    let filters = data::entry(job, owner, b"Filter")?;
    let listed_filters = data::items(filters.value())
        .map_or(1, <[Object]>::len)
        .max(1);
    let count = u64::try_from(listed_filters).unwrap_or(u64::MAX);
    let mut rows = 0_u64;
    let listed = data::entry(job, owner, b"DecodeParms")?;
    let items = data::items(listed.value()).unwrap_or_else(|| slice::from_ref(listed.value()));
    for item in items.iter().take(stream::FILTERS) {
        let parameters = data::resolve(job, item)?;
        let number = |key: &[u8], usual: i64| -> Result<u64, PdfError> {
            let value = data::entry(job, parameters.value(), key)?;
            let found = Fixed::from_object(value.value()).map_or(usual, Fixed::floor);
            Ok(u64::try_from(found).unwrap_or(0))
        };
        if number(b"Predictor", 1)? > 1 {
            let row = number(b"Columns", 1)?
                .saturating_mul(number(b"Colors", 1)?)
                .saturating_mul(number(b"BitsPerComponent", 8)?)
                .div_ceil(8);
            rows = rows.saturating_add(row.saturating_mul(2));
        }
    }
    let windows = rows.div_ceil(WINDOW);
    if windows > most {
        return Err(job.capped());
    }
    // Each has a window to allocate. Their predictors' rows count as their reader reads.
    job.spend(count.saturating_mul(STAGE_WORK))?;
    Ok(count)
}

/// An image `XObject` on the band's pixels.
// Its frame is gone before a stencil's pattern nests content.
#[inline(never)]
fn stored(
    run: &mut Run<'_, '_>,
    view: &View,
    image: &Resolved,
) -> Result<Option<Raster>, PdfError> {
    let job = run.job;
    let owner = image.value();
    let Some(role) = role(run, owner)? else {
        return Ok(None);
    };
    // What the caches keep gives way to what this image needs.
    run.caches.images.cursors.offer(job);
    run.caches.images.decoded.offer(job);
    let opened = data::coded(job, image)?;
    let header = Header::read(run, owner, opened.image, role)?;
    let from = header.source(job, &run.caches.images, view, (image, opened.data))?;
    if from.is_none() && !job.planes.measuring() {
        return Ok(None);
    }
    let masks = masking(run, view, owner, &header)?;
    let Some(drawn) = prepared(run, (owner, &header), from, masks)? else {
        return Ok(None);
    };
    rastered(job, view, drawn, Some(&mut run.caches.images.cursors)).map(Some)
}

/// The mask image `mask` of `role`; `None` as [`Header::source`] gives it.
fn subsidiary<'doc>(
    run: &mut Run<'_, 'doc>,
    view: &View,
    mask: &Resolved,
    role: Role,
) -> Result<Option<Part<'doc>>, PdfError> {
    let job = run.job;
    let owner = mask.value();
    if !matches!(owner, Object::Stream(..))
        || (role == Role::Cutout && !flagged(job, owner, b"ImageMask")?)
    {
        return Err(job.objects.malformed());
    }
    let opened = data::coded(job, mask)?;
    let header = Header::read(run, owner, opened.image, role)?;
    let from = header.source(job, &run.caches.images, view, (mask, opened.data))?;
    let Some(source) = from else {
        return Ok(None);
    };
    let samples = shared(job, owner, &header, source.layout)?;
    Ok(Some(Part { samples, source }))
}
