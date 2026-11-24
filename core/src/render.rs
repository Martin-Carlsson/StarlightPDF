//! Rendering a page to an image in bands: a pass without pixels, which picks the band height, then
//! each band replays the content (ISO 32000-2:2020, 8, 9, 11, 12.5).

mod blend;
mod codec;
mod colour;
mod coverage;
mod data;
pub(crate) mod fixed;
mod function;
mod geometry;
mod image;
mod interpret;
mod job;
mod lent;
mod math;
mod output;
mod path;
mod plane;
mod shading;
mod stroke;
mod surface;
mod text;
mod tiling;

use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};

use crate::images::RasterFormat;
use crate::objects::ObjectId;
use crate::output::Sink;
use crate::overlay::{Object, Overlay, Reading};
use crate::page::display::{CYCLE, Display, SCALE, display};
use crate::progress::Progress;
use crate::render::data::Resolved;
use crate::render::fixed::{Fixed, quotient, saturated};
use crate::render::geometry::{Area, Matrix};
use crate::render::interpret::Caches;
use crate::render::job::{BAND, BAND_ROWS, Job, OUTPUT_WORK, Pace, WORK_STEP, work_limit};
use crate::render::output::{Encoder, Settings};
use crate::render::plane::Planes;
use crate::render::surface::Surface;
use crate::rendering::{ColorSpace, RenderOptions};
use crate::store::{Bounds, Objects, Store};
use crate::syntax::Value;
use crate::{ErrorCode, PdfError};

pub(crate) use function::room;

/// The deviation of a page tree deeper than is read.
const DEEP: &str = "a page tree is deeper than this implementation reads";

/// The most pixels of a side of a JPEG (ITU-T T.81, B.2.2).
const JPEG_SIDE: u32 = 0xFFFF;

/// The operation errors name.
pub(crate) const OPERATION: &str = "Page::render";

/// Points per inch (ISO 32000-2:2020, 8.3.2.3).
const POINTS: i128 = 72;

/// The quality of a JPEG by default.
const QUALITY: u8 = 85;

/// The highest resolution.
const RESOLUTION: u32 = 4800;

/// The most pixels of a side.
const SIDE: u32 = 0x1_0000;

/// What the options ask for, in range.
struct Asked {
    /// The longer side, over the resolution.
    fit: Option<u32>,
    /// Grey, not RGB.
    gray: bool,
    /// JPEG, not PNG.
    jpeg: bool,
    /// The JPEG quality.
    quality: u8,
    /// Pixels per inch.
    resolution: u32,
}

/// The image's pixels and the map onto them.
struct Frame {
    /// The map from default user space to pixels.
    device: Matrix,
    /// The crop box has no area, so the image is white.
    empty: bool,
    /// Rows.
    height: u32,
    /// Columns.
    width: u32,
}

/// The page to render.
pub(crate) struct Subject<'doc> {
    /// The page object.
    pub id: ObjectId,
    /// The document's changes.
    pub overlay: &'doc Arc<Overlay>,
    /// The document's shared state.
    pub store: &'doc Store,
}

/// The options, checked: the ranges first, then what is not implemented.
fn asked(options: &RenderOptions) -> Result<Asked, PdfError> {
    let quality = options.quality.unwrap_or(QUALITY);
    let resolution = options.resolution.unwrap_or(72);
    if !(1..=100).contains(&quality)
        || !(1..=RESOLUTION).contains(&resolution)
        || options.fit.is_some_and(|fit| !(1..=SIDE).contains(&fit))
    {
        return Err(PdfError::new(ErrorCode::InvalidArgument, OPERATION));
    }
    let unimplemented = || PdfError::new(ErrorCode::NotImplemented, OPERATION);
    let jpeg = match options.format {
        RasterFormat::Heif | RasterFormat::Tiff => return Err(unimplemented()),
        RasterFormat::Jpeg => true,
        RasterFormat::Png => false,
    };
    let gray = match options.color_space {
        ColorSpace::Cmyk => return Err(unimplemented()),
        ColorSpace::Gray => true,
        ColorSpace::Rgb => false,
    };
    if !options.hidden_layers.is_empty() {
        return Err(unimplemented());
    }
    Ok(Asked {
        fit: options.fit,
        gray,
        jpeg,
        quality,
        resolution,
    })
}

/// The map from user space to pixels of the page `shown`, `scale` pixels, with 32 fraction bits,
/// to a unit of user space.
fn device(shown: &Display, scale: i128) -> Option<Matrix> {
    let [across_x, down_x, across_y, down_y, left, top] = shown.matrix;
    // The display's map is a signed permutation, so it is inverted by its signs, exactly.
    let [xx, xy, yx, yy] = [across_x, down_x, across_y, down_y].map(i128::signum);
    let moved = |x_sign: i128, y_sign: i128| -> Option<i128> {
        let offset = x_sign
            .checked_mul(left)?
            .checked_add(y_sign.checked_mul(top)?)?;
        quotient(offset.checked_neg()?.checked_mul(scale)?, SCALE)
    };
    let terms = [
        xx.checked_mul(scale)?,
        yx.checked_mul(scale)?,
        xy.checked_mul(scale)?,
        yy.checked_mul(scale)?,
        moved(xx, xy)?,
        moved(yx, yy)?,
    ];
    Some(Matrix {
        terms: terms.map(|bits| Fixed::from_bits(saturated(bits))),
    })
}

/// The image of the page `shown`: one factor for both sides, each rounded to the nearest pixel, at
/// least 1; `None` past what a render may be.
fn frame(shown: &Display, asked: &Asked) -> Option<Frame> {
    let [across, down] = shown.size;
    let longer = across.max(down);
    let empty = across <= 0 || down <= 0;
    // Pixels are sides times `factor` over `divisor`.
    let (factor, divisor) = match asked.fit {
        Some(fit) => (i128::from(fit), longer.max(1)),
        None => (
            shown.unit.checked_mul(i128::from(asked.resolution))?,
            POINTS.checked_mul(SCALE)?.checked_mul(SCALE)?,
        ),
    };
    let pixels = |side: i128| -> Option<u32> {
        let count = quotient(side.checked_mul(factor)?, divisor)?;
        u32::try_from(count.max(1))
            .ok()
            .filter(|found| *found <= SIDE)
    };
    let [width, height] = [pixels(across)?, pixels(down)?];
    let scale = if empty {
        0
    } else {
        let wide = factor
            .checked_mul(SCALE)?
            .checked_mul(1_i128 << 32_u32)
            .and_then(|bits| quotient(bits, divisor));
        // Too large a product only for a huge factor, whose low bits do not count.
        match wide {
            Some(found) => found,
            None => quotient(factor.checked_mul(SCALE)?, divisor >> 32_u32)?,
        }
    };
    let limit = if asked.jpeg { JPEG_SIDE } else { SIDE };
    (width.max(height) <= limit).then_some(Frame {
        device: device(shown, scale)?,
        empty,
        height,
        width,
    })
}

/// The entries of the stored page `id` that rendering reads, those inherited taken from its
/// nearest ancestor that has them (ISO 32000-2:2020, 7.7.3.4); others are not read.
fn stored(objects: Objects<'_>, id: ObjectId) -> Result<Object, PdfError> {
    let page = objects.dictionary(Some(Value::Reference(id)))?;
    let mut entries = Vec::new();
    for key in [&b"Annots"[..], b"Contents", b"Group", b"UserUnit"] {
        if let Some(found) = objects.owned_entry(id, page, key)? {
            entries.push((key.to_vec(), found));
        }
    }
    for key in [&b"CropBox"[..], b"MediaBox", b"Resources", b"Rotate"] {
        let (mut node, mut at) = (id, page);
        let mut visited = BTreeSet::from([(id.number, id.generation)]);
        let mut levels = 0..=objects.depth_limit();
        loop {
            // Deeper than read: a cap of this implementation (7.7.3.4).
            if levels.next().is_none() {
                if objects.strict() {
                    return Err(PdfError::new(ErrorCode::LimitExceeded, OPERATION));
                }
                objects.deviation("7.7.3.4", DEEP, None)?;
                break;
            }
            let found = objects.owned_entry(node, at, key)?;
            if let Some(value) = found.filter(|value| !matches!(value, Object::Null)) {
                entries.push((key.to_vec(), value));
                break;
            }
            let Some(Value::Reference(parent)) = objects.get(at, b"Parent")? else {
                break;
            };
            // A tree has no cycle (7.7.3.4): an ancestor met again ends the walk.
            if !visited.insert((parent.number, parent.generation)) {
                objects.deviation("7.7.3.4", CYCLE, None)?;
                break;
            }
            (node, at) = (parent, objects.dictionary(Some(Value::Reference(parent)))?);
        }
    }
    Ok(Object::Dictionary(entries))
}

/// The pass that measures: the rows of a band whose planes fit [`BAND`].
fn measure<'doc>(
    job: &Job<'doc>,
    caches: &mut Caches<'doc>,
    dictionary: &Resolved,
    frame: &Frame,
) -> Result<u32, PdfError> {
    let mut surface = Surface::new(job, job.area, None)?;
    if !frame.empty {
        interpret::page(job, caches, &mut surface, dictionary)?;
    }
    let peak = job.planes.peak();
    if peak > BAND.max(u64::from(frame.width).saturating_mul(128)) {
        return Err(job.limit());
    }
    let rows = BAND.checked_div(peak).unwrap_or(BAND_ROWS);
    Ok(u32::try_from(rows.clamp(1, BAND_ROWS)).unwrap_or(1))
}

/// Warns of what is wrong in the boxes and display entries `shown` gives (7.7.3.3).
fn flawed(job: &Job<'_>, shown: &Display) -> Result<(), PdfError> {
    for (bad, message) in [
        (shown.missing_media, "MediaBox missing"),
        (shown.bad_rotate, "Rotate not a multiple of 90"),
        (shown.bad_unit, "UserUnit not a positive number"),
    ] {
        if bad {
            job.warn("7.7.3.3", message)?;
        }
    }
    Ok(())
}

/// Renders the page `target` to `sink` as `options` say; the caller finishes the sink.
pub(crate) fn page(
    target: &Subject<'_>,
    sink: &Sink,
    progress: Option<&dyn Progress>,
    options: &RenderOptions,
) -> Result<(), PdfError> {
    let asked = asked(options)?;
    let unbounded = target.store.objects(OPERATION);
    let bounds = Bounds::default();
    // Paced and charged from the first read, the page's preparation too.
    let start = work_limit(unbounded.decoded_limit(), 0);
    bounds.work.set(start);
    let pace = Pace {
        checkpoint: Cell::new(start.saturating_sub(WORK_STEP)),
        done: Cell::new(0),
        height: Cell::new(0),
        overlay: Reading::new(target.overlay),
        progress,
        work: &bounds.work,
    };
    let objects = unbounded.within(&bounds).paced(&pace);
    let reading = Reading::new(target.overlay);
    let value = if reading.is_changed() {
        reading.resolve(objects, target.id)?
    } else {
        stored(objects, target.id)?
    };
    if !matches!(value, Object::Dictionary(_)) {
        return Err(objects.malformed());
    }
    let shown = display(objects, &reading, &value)?;
    let sized =
        frame(&shown, &asked).ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, OPERATION))?;
    let pixels = u64::from(sized.width).saturating_mul(u64::from(sized.height));
    let work = work_limit(objects.decoded_limit(), pixels);
    let spent = start.saturating_sub(bounds.work.get());
    let ready = work
        .checked_sub(pixels.saturating_mul(OUTPUT_WORK))
        .and_then(|left| left.checked_sub(spent))
        .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, OPERATION))?;
    // Taken before any content runs, so that a page of more pixels than the limit allows ends now.
    bounds.work.set(ready);
    pace.checkpoint.set(ready.saturating_sub(WORK_STEP));
    pace.height.set(u64::from(sized.height));
    let image = Area {
        bottom: i32::try_from(sized.height).unwrap_or(i32::MAX),
        left: 0,
        right: i32::try_from(sized.width).unwrap_or(i32::MAX),
        top: 0,
    };
    let mut job = Job {
        area: image,
        device: sized.device,
        known: RefCell::default(),
        lent: RefCell::default(),
        objects,
        overlay: reading,
        pace: &pace,
        page: image,
        planes: Planes::new(true),
        refused: Cell::new(false),
        usage: options.usage,
        work: &bounds.work,
    };
    job.report()?;
    flawed(&job, &shown)?;
    let dictionary = data::made(&job, value, Some(target.id))?;
    let mut caches = Caches::default();
    let rows = measure(&job, &mut caches, &dictionary, &sized)?;
    // Each band may replay what measuring spent; all replays, at most the content's part of the
    // limit again, the pixels' allowance left out.
    let replayed = ready.saturating_sub(bounds.work.get());
    let mut replays = work_limit(objects.decoded_limit(), 0);
    job.planes = Planes::new(false);
    let settings = Settings {
        gray: asked.gray,
        height: sized.height,
        jpeg: asked.jpeg,
        quality: asked.quality,
        width: sized.width,
    };
    let mut encoder = Encoder::new(sink, settings)?;
    let whole = job.area;
    let mut top = 0_u32;
    while top < sized.height {
        let bottom = top.saturating_add(rows).min(sized.height);
        job.area = Area {
            bottom: i32::try_from(bottom).unwrap_or(i32::MAX),
            top: i32::try_from(top).unwrap_or(i32::MAX),
            ..whole
        };
        let refund = replayed.min(replays);
        replays = replays.saturating_sub(refund);
        let left = bounds.work.get().saturating_add(refund);
        bounds.work.set(left);
        job.pace.checkpoint.set(left.saturating_sub(WORK_STEP));
        let mut surface = Surface::new(&job, job.area, None)?;
        if !sized.empty {
            interpret::page(&job, &mut caches, &mut surface, &dictionary)?;
        }
        for row in job.area.top..job.area.bottom {
            encoder.row(surface.pixels(row))?;
        }
        image::next_band(&mut caches.images, job.area.bottom);
        job.pace.done.set(u64::from(bottom));
        job.report()?;
        top = bottom;
    }
    encoder.finish()
}
