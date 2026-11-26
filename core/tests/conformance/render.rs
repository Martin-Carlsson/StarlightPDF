//! The render cases: their keys, and the pixels and errors they expect.

use core::sync::atomic::{AtomicU64, Ordering};

use starlightpdf::document::PageOptions;
use starlightpdf::images::RasterFormat;
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::progress::Progress;
use starlightpdf::rendering::{ColorSpace, RenderOptions, RenderUsage};
use starlightpdf::{ErrorCode, PdfError};

use crate::decode::Image;
use crate::{Failure, OPEN, lookup, open, parse, source, warnings};

/// The keys of a render besides those of open.
const KEYS: [&str; 9] = [
    "cancel",
    "color_space",
    "fit",
    "format",
    "index",
    "quality",
    "read",
    "resolution",
    "usage",
];

/// Each `format=` value.
const RASTERS: [(RasterFormat, &str); 4] = [
    (RasterFormat::Heif, "Heif"),
    (RasterFormat::Jpeg, "Jpeg"),
    (RasterFormat::Png, "Png"),
    (RasterFormat::Tiff, "Tiff"),
];

/// Each `color_space=` value.
const SPACES: [(ColorSpace, &str); 3] = [
    (ColorSpace::Cmyk, "Cmyk"),
    (ColorSpace::Gray, "Gray"),
    (ColorSpace::Rgb, "Rgb"),
];

/// Each `usage=` value.
const USAGES: [(RenderUsage, &str); 2] = [
    (RenderUsage::Display, "Display"),
    (RenderUsage::Print, "Print"),
];

/// A pixel a case names.
pub(crate) struct Expected<'case> {
    /// The colour, a byte a channel.
    colour: Vec<u8>,
    /// Pixels from the left.
    column: usize,
    /// Pixels from the top.
    row: usize,
    /// The token.
    token: &'case str,
    /// The most each channel may differ.
    tolerance: u8,
}

/// A progress callback that returns false after `allowed` calls.
struct After {
    /// The calls let through.
    allowed: u64,
    /// The calls so far.
    calls: AtomicU64,
}

/// A case's render.
#[derive(Default)]
struct Request<'case> {
    /// `cancel` (0) or `cancel=N`: the progress calls let through before one returns false.
    cancel: Option<u64>,
    /// `index=`.
    index: u32,
    /// The options.
    options: RenderOptions,
    /// The keys given.
    seen: Vec<&'case str>,
    /// `read=warnings`.
    warnings: bool,
}

impl Progress for After {
    fn update(&self, done: u64, total: u64) -> bool {
        let _: (u64, u64) = (done, total);
        self.calls.fetch_add(1, Ordering::Relaxed) < self.allowed
    }
}

impl<'case> Request<'case> {
    /// Takes `key` with its `value`; `None` when either is bad.
    fn set(&mut self, key: &'case str, value: Option<&str>) -> Option<()> {
        if self.seen.contains(&key) {
            return None;
        }
        self.seen.push(key);
        let options = &mut self.options;
        match (key, value) {
            ("cancel", None) => self.cancel = Some(0),
            ("cancel", Some(text)) if text.bytes().all(|byte| byte.is_ascii_digit()) => {
                self.cancel = Some(text.parse().ok()?);
            }
            ("color_space", Some(text)) => options.color_space = lookup(&SPACES, text)?,
            ("fit", Some(text)) => options.fit = Some(text.parse().ok()?),
            ("format", Some(text)) => options.format = lookup(&RASTERS, text)?,
            ("index", Some(text)) => self.index = text.parse().ok()?,
            ("quality", Some(text)) => options.quality = Some(text.parse().ok()?),
            ("read", Some("warnings")) => self.warnings = true,
            ("resolution", Some(text)) => options.resolution = Some(text.parse().ok()?),
            ("usage", Some(text)) => options.usage = lookup(&USAGES, text)?,
            _ => return None,
        }
        Some(())
    }
}

/// What the render of `relative` gives as `args` say, compared with the pixels `expected` names: each one's own
/// token where it is within its tolerance, else its true colour.
pub(crate) fn call(
    operation: &str,
    file: bool,
    relative: &str,
    args: &[&str],
    expected: &str,
) -> Result<String, String> {
    let mut request = Request::default();
    let mut others = Vec::new();
    for &arg in args {
        let (key, value) = arg
            .split_once('=')
            .map_or((arg, None), |(name, text)| (name, Some(text)));
        if KEYS.contains(&key) {
            request
                .set(key, value)
                .ok_or_else(|| format!("{operation}: bad argument '{arg}'"))?;
        } else {
            others.push(arg);
        }
    }
    let case = parse(operation, &OPEN, &others)?;
    let named = if request.warnings || is_code(expected) {
        Vec::new()
    } else {
        pixels(expected).ok_or_else(|| format!("{operation}: bad expected value '{expected}'"))?
    };
    let made = source(relative, file)?;
    let opened = made
        .and_then(|found| open(&found, &case))
        .and_then(|document| {
            let mut options = PageOptions::default();
            options.index = request.index;
            let page = document.page(&options)?;
            Ok((document, page))
        });
    let (document, page) = match opened {
        Ok(found) => found,
        // The render never ran: without rendering the case cannot pass.
        Err(error) if cfg!(feature = "rendering") => return Ok(String::from(error.code().name())),
        Err(error) => {
            return Err(format!(
                "{operation}: {} before the render",
                error.code().name()
            ));
        }
    };
    let counted = request.cancel.map(|allowed| After {
        allowed,
        calls: AtomicU64::new(0),
    });
    let rendered = Sink::in_memory(&MemorySinkOptions::default()).and_then(|sink| {
        let stop = counted.as_ref().map(|after| -> &dyn Progress { after });
        page.render(&sink, stop, &request.options)?;
        sink.take_bytes()
    });
    if !cfg!(feature = "rendering") {
        return without_rendering(operation, rendered.err(), expected);
    }
    let bytes = match rendered {
        Ok(bytes) => bytes,
        Err(error) => return Ok(String::from(error.code().name())),
    };
    let image = Image::read(&bytes).map_err(|why| format!("{operation}: {why}"))?;
    if image.jpeg != (request.options.format == RasterFormat::Jpeg) {
        return Err(format!("{operation}: the file is of another format"));
    }
    if request.warnings {
        return match warnings("warnings", &case, &document) {
            Ok(found) => Ok(found),
            Err(Failure::Pdf(error)) => Ok(String::from(error.code().name())),
            Err(Failure::Case(why)) => Err(format!("{operation}: {why}")),
        };
    }
    let mut parts = vec![format!("{}x{}", image.width, image.height)];
    parts.extend(named.iter().map(|pixel| matched(&image, pixel)));
    Ok(parts.join(" "))
}

/// Whether `expected` is an error code's name rather than pixels.
pub(crate) fn is_code(expected: &str) -> bool {
    !expected.is_empty() && expected.bytes().all(|byte| byte.is_ascii_alphabetic())
}

/// `pixel`'s token where `image` has its colour, else the colour it has.
pub(crate) fn matched(image: &Image, pixel: &Expected<'_>) -> String {
    let place = format!("{},{}", pixel.column, pixel.row);
    let Some(found) = image.pixel(pixel.column, pixel.row) else {
        return format!("{place}=none");
    };
    let close = found.len() == pixel.colour.len()
        && found
            .iter()
            .zip(&pixel.colour)
            .all(|(&have, &want)| have.abs_diff(want) <= pixel.tolerance);
    if close {
        String::from(pixel.token)
    } else {
        format!("{place}={}", crate::hex(found))
    }
}

/// The pixels of `WxH x,y=colour[~T]...`; `None` unless it is of that form with at least one.
pub(crate) fn pixels(expected: &str) -> Option<Vec<Expected<'_>>> {
    let (size, tokens) = expected.split_once(' ')?;
    let (width, height) = size.split_once('x')?;
    let _: (u32, u32) = (width.parse().ok()?, height.parse().ok()?);
    tokens.split(' ').map(pixel).collect()
}

/// The pixel `token` names.
fn pixel(token: &str) -> Option<Expected<'_>> {
    let (place, value) = token.split_once('=')?;
    let (column, row) = place.split_once(',')?;
    let (digits, tolerance) = match value.split_once('~') {
        Some((digits, allowed)) => (digits, allowed.parse().ok()?),
        None => (value, 0),
    };
    let lowercase = digits
        .bytes()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if !lowercase || !matches!(digits.len(), 2 | 6) {
        return None;
    }
    Some(Expected {
        colour: crate::unhex(digits)?,
        column: column.parse().ok()?,
        row: row.parse().ok()?,
        token,
        tolerance,
    })
}

/// Without rendering every case must fail with `CapabilityMissing`, before any option is looked at.
fn without_rendering(
    operation: &str,
    failed: Option<PdfError>,
    expected: &str,
) -> Result<String, String> {
    let code = failed.map(|error| error.code());
    if code == Some(ErrorCode::CapabilityMissing) {
        Ok(String::from(expected))
    } else {
        Err(format!(
            "{operation}: {} without rendering, not CapabilityMissing",
            code.map_or("ok", ErrorCode::name)
        ))
    }
}
