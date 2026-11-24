//! The image written, a row at a time over white.

mod baseline;
mod deflate;
mod png;

use crate::output::Sink;
use crate::render::output::baseline::Baseline;
use crate::render::output::png::Png;
use crate::render::{OPERATION, fixed};
use crate::{ErrorCode, PdfError};

/// Bytes gathered before each write to the sink.
const BLOCK: usize = 0x8000;

/// The weights of red, green and blue in grey, in hundredths (ISO 32000-2:2020, 10.4.2.2).
const GRAY: [u32; 3] = [30, 59, 11];

/// Writes one image.
pub(crate) struct Encoder<'sink> {
    /// The file being written.
    format: Box<dyn Format + 'sink>,
    /// Grey, not RGB.
    gray: bool,
    /// The row as samples.
    samples: Vec<u8>,
}

/// What to write.
#[derive(Clone, Copy)]
pub(crate) struct Settings {
    /// Grey, not RGB.
    pub gray: bool,
    /// Rows.
    pub height: u32,
    /// JPEG, not PNG.
    pub jpeg: bool,
    /// The JPEG quality.
    pub quality: u8,
    /// Columns.
    pub width: u32,
}

/// A file format being written.
trait Format {
    /// Ends the file.
    fn finish(&mut self) -> Result<(), PdfError>;

    /// Writes the next row; its samples may be changed.
    fn row(&mut self, samples: &mut [u8]) -> Result<(), PdfError>;
}

impl<'sink> Encoder<'sink> {
    /// Ends the image.
    pub(crate) fn finish(mut self) -> Result<(), PdfError> {
        self.format.finish()
    }

    /// Starts the image `settings` in `sink`.
    pub(crate) fn new(sink: &'sink Sink, settings: Settings) -> Result<Self, PdfError> {
        let format: Box<dyn Format + 'sink> = if settings.jpeg {
            Box::new(Baseline::new(sink, settings)?)
        } else {
            Box::new(Png::new(sink, settings)?)
        };
        let samples = buffer(settings.width, channels(settings.gray))?;
        Ok(Self {
            format,
            gray: settings.gray,
            samples,
        })
    }

    /// Writes the next row, premultiplied.
    pub(crate) fn row(&mut self, pixels: &[[u16; 4]]) -> Result<(), PdfError> {
        if self.gray {
            for (out, pixel) in self.samples.iter_mut().zip(pixels) {
                *out = fixed::byte(grey(over_white(*pixel)));
            }
        } else {
            for (out, pixel) in self.samples.as_chunks_mut::<3>().0.iter_mut().zip(pixels) {
                *out = over_white(*pixel).map(fixed::byte);
            }
        }
        self.format.row(&mut self.samples)
    }
}

/// Zeros, `times` a column.
fn buffer(width: u32, times: usize) -> Result<Vec<u8>, PdfError> {
    let length = usize::try_from(width)
        .ok()
        .and_then(|columns| columns.checked_mul(times))
        .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, OPERATION))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))?;
    bytes.resize(length, 0);
    Ok(bytes)
}

/// Samples a pixel.
const fn channels(gray: bool) -> usize {
    if gray { 1 } else { 3 }
}

/// The grey of an opaque colour, rounded to the nearest.
fn grey(colour: [u16; 3]) -> u16 {
    let weighed = colour.iter().zip(GRAY).fold(50_u32, |sum, (part, weight)| {
        sum.wrapping_add(u32::from(*part).wrapping_mul(weight))
    });
    u16::try_from(weighed.checked_div(100).unwrap_or_default()).unwrap_or(u16::MAX)
}

/// The lowest byte of `value`.
fn low(value: u32) -> u8 {
    u8::try_from(value & 0xFF).unwrap_or_default()
}

/// The `width` lowest bits of `value`.
const fn lowest(value: u32, width: u32) -> u32 {
    value & 1_u32.wrapping_shl(width).saturating_sub(1)
}

/// A premultiplied pixel over opaque white (ISO 32000-2:2020, 11.4.7).
fn over_white([red, green, blue, alpha]: [u16; 4]) -> [u16; 3] {
    let white = u16::MAX.saturating_sub(alpha);
    [red, green, blue].map(|part| part.saturating_add(white))
}

/// The bytes of `value`, highest first.
fn word(value: u32) -> [u8; 4] {
    [24_u32, 16, 8, 0].map(|shift| low(value.wrapping_shr(shift)))
}
