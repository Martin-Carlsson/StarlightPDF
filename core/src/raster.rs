//! Image files to draw: JPEG (ITU-T T.81) and PNG (ISO/IEC 15948:2004).

mod exif;
mod jpeg;
mod png;

use core::slice::Iter;

use crate::images::ImageFormat;
use crate::inflate::Inflate;
use crate::input::Source;
use crate::pull::Pull;
use crate::{ErrorCode, PdfError};

/// The operation reading errors name.
const READ: &str = "Page::draw_image";

/// Bytes read at a time.
const STEP: usize = 0x1000;

/// The operation writing errors name.
const WRITE: &str = "Document::save";

/// A source read forward through a buffer.
struct Cursor<'source> {
    /// Bytes read ahead.
    buffer: Vec<u8>,
    /// The count of valid bytes in `buffer`.
    end: usize,
    /// The source offset just past `buffer`'s valid bytes.
    next: u64,
    /// The next unread byte in `buffer`.
    position: usize,
    /// The source.
    source: &'source Source,
}

/// The filter of an image's stored data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Filter {
    /// `DCTDecode` (ISO 32000-2:2020, 7.4.8).
    Dct,
    /// `FlateDecode` with `DecodeParms` `Predictor` 15, these `Colors`, `BitsPerComponent` of
    /// [`Image::bits`] and `Columns` of [`Image::width`] (7.4.4.4).
    Png {
        /// `Colors`.
        colors: u8,
    },
}

/// An image file read for drawing; its data stays in the source until written.
#[derive(Debug)]
pub(crate) struct Image {
    /// Bits per component.
    bits: u8,
    /// Components of the base colour space.
    components: u8,
    /// Rows.
    height: u32,
    /// Stored values are inverted.
    inverted: bool,
    /// Bytes of stored data.
    length: u64,
    /// Exif orientation, 1 to 8.
    orientation: u8,
    /// The `Indexed` lookup table of RGB entries.
    palette: Option<Vec<u8>>,
    /// The PNG's layout; `None`: the data is the whole file.
    png: Option<png::Png>,
    /// The ICC profile.
    profile: Option<Profile>,
    /// The file.
    source: Source,
    /// Columns.
    width: u32,
}

/// An embedded ICC profile (ICC.1:2010), stored in ranges of the file.
#[derive(Debug)]
pub(crate) struct Profile {
    /// Stored as zlib data.
    compressed: bool,
    /// Bytes stored.
    length: u64,
    /// Offsets and lengths of its stored bytes, in order.
    parts: Vec<(u64, u64)>,
}

/// Ranges of a source, read in order.
struct Ranges<'source> {
    /// Bytes left in the current range.
    left: u64,
    /// The offset of the next byte.
    offset: u64,
    /// The operation errors name.
    operation: &'static str,
    /// The ranges after the current one.
    parts: Iter<'source, (u64, u64)>,
    /// The source.
    source: &'source Source,
}

impl<'source> Cursor<'source> {
    /// The next byte, or `None` at the end.
    fn byte(&mut self) -> Result<Option<u8>, PdfError> {
        let mut byte = [0];
        Ok((self.fill(&mut byte)? == 1).then_some(byte[0]))
    }

    /// Fills `out`, stopping early at the end; returns the count read.
    fn fill(&mut self, out: &mut [u8]) -> Result<usize, PdfError> {
        let mut filled = 0_usize;
        while let Some(rest) = out.get_mut(filled..).filter(|rest| !rest.is_empty()) {
            if self.position >= self.end {
                self.end = self.source.read(self.next, &mut self.buffer, READ)?;
                self.next = self
                    .next
                    .saturating_add(u64::try_from(self.end).unwrap_or(0));
                self.position = 0;
                if self.end == 0 {
                    break;
                }
            }
            let held = self.buffer.get(self.position..self.end).unwrap_or_default();
            let count = held.len().min(rest.len());
            for (slot, byte) in rest.iter_mut().zip(held) {
                *slot = *byte;
            }
            self.position = self.position.saturating_add(count);
            filled = filled.saturating_add(count);
        }
        Ok(filled)
    }

    /// A cursor at `offset`.
    fn new(source: &'source Source, offset: u64) -> Self {
        Self {
            buffer: vec![0; STEP],
            end: 0,
            next: offset,
            position: 0,
            source,
        }
    }

    /// The offset of the next byte.
    fn offset(&self) -> u64 {
        self.next
            .saturating_sub(u64::try_from(self.end.saturating_sub(self.position)).unwrap_or(0))
    }

    /// Skips `count` bytes.
    fn skip(&mut self, count: u64) {
        let held = self.end.saturating_sub(self.position);
        match usize::try_from(count) {
            Ok(within) if within <= held => self.position = self.position.saturating_add(within),
            Ok(_) | Err(_) => {
                self.next = self.offset().saturating_add(count);
                self.end = 0;
                self.position = 0;
            }
        }
    }
}

impl Image {
    /// Bits per component, also of the colour key and lookup indices (ISO 32000-2:2020, 8.9.5.1).
    pub(crate) const fn bits(&self) -> u8 {
        self.bits
    }

    /// Components of the base colour space: 1 gray, 3 RGB, 4 CMYK; `N` of [`Image::profile`].
    pub(crate) const fn components(&self) -> u8 {
        self.components
    }

    /// The image needs `Decode` `[1 0]` per component (8.9.5.2).
    pub(crate) const fn decode_inverted(&self) -> bool {
        self.inverted
    }

    /// The filter of the stored data; `None`: unfiltered samples (8.9.3).
    pub(crate) const fn filter(&self) -> Option<Filter> {
        match self.png.as_ref() {
            None => Some(Filter::Dct),
            Some(png) if png.decoded() => None,
            Some(png) => Some(Filter::Png {
                colors: png.channels(),
            }),
        }
    }

    /// Rows.
    pub(crate) const fn height(&self) -> u32 {
        self.height
    }

    /// Bytes [`Image::write_data`] writes.
    pub(crate) const fn length(&self) -> u64 {
        self.length
    }

    /// The soft mask's bits per component and the bytes [`Image::write_mask`] writes: unfiltered
    /// `DeviceGray` samples of the image's size (11.6.5.3).
    pub(crate) fn mask(&self) -> Option<(u8, u64)> {
        self.png.as_ref().and_then(png::Png::mask)
    }

    /// Exif orientation (CIPA DC-008-2019, 4.6.4 A, tag 274): 1 to 8, 1 when absent.
    pub(crate) const fn orientation(&self) -> u8 {
        self.orientation
    }

    /// The `Indexed` lookup table over the base space, 3 bytes per entry; `hival` is entries less 1
    /// (8.6.6.3).
    pub(crate) fn palette(&self) -> Option<&[u8]> {
        self.palette.as_deref()
    }

    /// The base space's ICC profile, for `ICCBased` (8.6.5.5).
    pub(crate) const fn profile(&self) -> Option<&Profile> {
        self.profile.as_ref()
    }

    /// Columns.
    pub(crate) const fn width(&self) -> u32 {
        self.width
    }

    /// Writes the stored data in pieces, as read from the source.
    ///
    /// # Errors
    ///
    /// Whatever `out` fails with; [`ErrorCode::Io`], or [`ErrorCode::Malformed`] when the source
    /// changed.
    pub(crate) fn write_data(
        &self,
        out: &mut dyn FnMut(&[u8]) -> Result<(), PdfError>,
    ) -> Result<(), PdfError> {
        match self.png.as_ref() {
            None => copy(
                &mut Ranges::new(&self.source, &[(0, self.length)], WRITE),
                out,
            ),
            Some(png) if png.decoded() => png.write_samples(&self.source, out),
            Some(png) => copy(&mut png.idat(&self.source), out),
        }
    }

    /// Writes the soft mask's samples in pieces; nothing without one.
    ///
    /// # Errors
    ///
    /// As [`Image::write_data`].
    pub(crate) fn write_mask(
        &self,
        out: &mut dyn FnMut(&[u8]) -> Result<(), PdfError>,
    ) -> Result<(), PdfError> {
        self.png
            .as_ref()
            .map_or(Ok(()), |png| png.write_mask(&self.source, out))
    }

    /// Writes the ICC profile's stored bytes in pieces; nothing without one.
    ///
    /// # Errors
    ///
    /// As [`Image::write_data`].
    pub(crate) fn write_profile(
        &self,
        out: &mut dyn FnMut(&[u8]) -> Result<(), PdfError>,
    ) -> Result<(), PdfError> {
        self.profile.as_ref().map_or(Ok(()), |profile| {
            copy(&mut Ranges::new(&self.source, &profile.parts, WRITE), out)
        })
    }
}

impl Profile {
    /// Fails unless the profile is for `components` components and decodes within `limit` bytes.
    fn check(&self, source: &Source, components: u8, limit: u64) -> Result<(), PdfError> {
        let raw = Ranges::new(source, &self.parts, READ);
        let mut data: Box<dyn Pull + '_> = if self.compressed {
            Box::new(Inflate::new(raw))
        } else {
            Box::new(raw)
        };
        // The header's data colour space (ICC.1:2010, 7.2.6).
        let mut header = [0; 128];
        let expected: &[u8] = match components {
            1 => b"GRAY",
            3 => b"RGB ",
            _ => b"CMYK",
        };
        if fill(&mut data, &mut header)? != header.len() || header.get(16..20) != Some(expected) {
            return Err(malformed());
        }
        let mut total = 128_u64;
        let mut scratch = vec![0; STEP];
        loop {
            match data.pull(&mut scratch)? {
                0 => return Ok(()),
                count => total = total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX)),
            }
            if total > limit {
                return Err(PdfError::new(ErrorCode::LimitExceeded, READ));
            }
        }
    }

    /// The profile is stored as zlib data, for `FlateDecode` (7.4.4).
    pub(crate) const fn compressed(&self) -> bool {
        self.compressed
    }

    /// Bytes [`Image::write_profile`] writes.
    pub(crate) const fn length(&self) -> u64 {
        self.length
    }
}

impl<'source> Ranges<'source> {
    /// Reads `parts` of `source`, failing as `operation`.
    fn new(source: &'source Source, parts: &'source [(u64, u64)], operation: &'static str) -> Self {
        Self {
            left: 0,
            offset: 0,
            operation,
            parts: parts.iter(),
            source,
        }
    }
}

impl Pull for Ranges<'_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        while self.left == 0 {
            let Some(&(offset, length)) = self.parts.next() else {
                return Ok(0);
            };
            self.offset = offset;
            self.left = length;
        }
        let wanted = usize::try_from(self.left)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let Some(part) = buf.get_mut(..wanted).filter(|part| !part.is_empty()) else {
            return Ok(0);
        };
        let count = self.source.read(self.offset, part, self.operation)?;
        if count == 0 {
            return Err(PdfError::new(ErrorCode::Malformed, self.operation));
        }
        let step = u64::try_from(count).unwrap_or(0);
        self.offset = self.offset.saturating_add(step);
        self.left = self.left.saturating_sub(step);
        Ok(count)
    }
}

/// Pulls all of `data` into `out` in pieces.
fn copy(
    data: &mut dyn Pull,
    out: &mut dyn FnMut(&[u8]) -> Result<(), PdfError>,
) -> Result<(), PdfError> {
    let mut buffer = vec![0; STEP];
    loop {
        match data.pull(&mut buffer)? {
            0 => return Ok(()),
            count => out(buffer.get(..count).unwrap_or_default())?,
        }
    }
}

/// Pulls until `out` is full or the data ends; returns the count.
fn fill(data: &mut dyn Pull, out: &mut [u8]) -> Result<usize, PdfError> {
    let mut filled = 0_usize;
    while let Some(rest) = out.get_mut(filled..).filter(|rest| !rest.is_empty()) {
        match data.pull(rest)? {
            0 => break,
            count => filled = filled.saturating_add(count),
        }
    }
    Ok(filled)
}

/// The error for a damaged file.
fn malformed() -> PdfError {
    PdfError::new(ErrorCode::Malformed, READ)
}

/// The big-endian number in `bytes`.
fn number(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |value: u32, byte| {
        value.wrapping_shl(8) | u32::from(*byte)
    })
}

/// Reads the image file in `source`, of `format` or the format its signature shows; a PNG's
/// samples may decode to at most `limit` bytes.
///
/// # Errors
///
/// [`ErrorCode::Unsupported`] for another format or a feature beyond these readers,
/// [`ErrorCode::Malformed`] for a damaged file or one not of `format`, [`ErrorCode::LimitExceeded`]
/// past `limit`, [`ErrorCode::NotSeekable`] for a forward-only source, [`ErrorCode::Io`].
pub(crate) fn read(
    source: &Source,
    format: Option<ImageFormat>,
    limit: u64,
) -> Result<Image, PdfError> {
    if source.forward_only() {
        return Err(PdfError::new(ErrorCode::NotSeekable, READ));
    }
    let mut head = [0; 8];
    let count = source.read(0, &mut head, READ)?;
    let detected = head.get(..count).unwrap_or_default();
    let chosen = match format {
        Some(given) => Some(given),
        None if detected.starts_with(&[0xFF, 0xD8, 0xFF]) => Some(ImageFormat::Jpeg),
        None if detected == png::SIGNATURE => Some(ImageFormat::Png),
        None => None,
    };
    if chosen == Some(ImageFormat::Jpeg) {
        jpeg::read(source, limit)
    } else if chosen == Some(ImageFormat::Png) {
        png::read(source, limit)
    } else {
        Err(PdfError::new(ErrorCode::Unsupported, READ))
    }
}
