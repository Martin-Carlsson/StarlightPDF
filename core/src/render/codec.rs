//! The codecs that decode whole, behind the core's guards (ISO 32000-2:2020, 7.4.6, 7.4.7, 7.4.9).
//!
//! The decoders allocate infallibly and cannot be interrupted, so what one call takes is bounded
//! before it is made.

mod boxes;
pub(crate) mod fax;
pub(crate) mod jbig2;
pub(crate) mod jpx;
mod symbols;

use crate::render::function::room;
use crate::render::job::Job;
use crate::{PdfError, pull};

/// The most bytes one codec takes: 256 MiB.
const HELD: u64 = 0x1000_0000;

/// The most bytes a decoder reads, so that their bits fit 32.
const INPUT: usize = 0x1FFF_FFC0;

/// A colour space the data implies.
pub(crate) enum Implied {
    /// CMYK.
    Cmyk,
    /// Grey.
    Gray,
    /// An ICC profile.
    Profile(Vec<u8>),
    /// RGB.
    Rgb,
    /// YCC, of sRGB.
    Ycc,
}

/// Decoded samples, in packed rows as PDF stores them.
pub(crate) struct Held {
    /// The component that is alpha.
    pub alpha: Option<usize>,
    /// The samples.
    pub data: Vec<u8>,
    /// The colour space the data implies.
    pub implied: Option<Implied>,
    /// What they are.
    pub info: Info,
    /// Colour is premultiplied by alpha.
    pub premultiplied: bool,
}

/// What decoded samples are.
pub(crate) struct Info {
    /// Bits per sample: 1, 2 or 4 for one JPX component of that depth, so that indices stay.
    pub bits: u8,
    /// Samples per pixel.
    pub components: u8,
    /// Rows.
    pub height: u32,
    /// Columns.
    pub width: u32,
}

/// Big-endian fields of a header.
struct Reader<'data> {
    /// The bytes read.
    at: usize,
    /// All of them.
    data: &'data [u8],
}

impl<'data> Reader<'data> {
    /// The next byte.
    fn byte(&mut self) -> Option<u8> {
        let found = self.data.get(self.at).copied()?;
        self.at = self.at.saturating_add(1);
        Some(found)
    }

    /// The bytes not read.
    const fn left(&self) -> usize {
        self.data.len().saturating_sub(self.at)
    }

    /// The next four bytes as a number.
    fn long(&mut self) -> Option<u32> {
        self.take(4).map(pull::number)
    }

    /// A reader of `data`.
    const fn new(data: &'data [u8]) -> Self {
        Self { at: 0, data }
    }

    /// The bytes not read.
    fn rest(&self) -> &'data [u8] {
        self.data.get(self.at..).unwrap_or_default()
    }

    /// The next two bytes as a number.
    fn short(&mut self) -> Option<u16> {
        u16::try_from(pull::number(self.take(2)?)).ok()
    }

    /// Reads past `count` bytes; none are read when fewer are left.
    fn skip(&mut self, count: usize) -> Option<()> {
        self.take(count).map(|_skipped| ())
    }

    /// The next `count` bytes; none are read when fewer are left.
    fn take(&mut self, count: usize) -> Option<&'data [u8]> {
        let end = self.at.checked_add(count)?;
        let found = self.data.get(self.at..end)?;
        self.at = end;
        Some(found)
    }
}

/// Fails unless the decoders can count the bits of `data`.
fn admitted(job: &Job<'_>, data: &[u8]) -> Result<(), PdfError> {
    if data.len() > INPUT {
        Err(job.limit())
    } else {
        Ok(())
    }
}

/// Fails unless a codec may take `bytes`, beside all that is held.
fn charged(job: &Job<'_>, bytes: u64) -> Result<(), PdfError> {
    if bytes > most(job) {
        Err(job.limit())
    } else {
        job.fits(bytes)
    }
}

/// `bytes` bytes of `value`.
fn filled(job: &Job<'_>, bytes: u64, value: u8) -> Result<Vec<u8>, PdfError> {
    let mut found = reserved(job, bytes)?;
    found.resize(usize::try_from(bytes).map_err(|_error| job.limit())?, value);
    Ok(found)
}

/// The most bytes a codec may take.
fn most(job: &Job<'_>) -> u64 {
    job.objects.decoded_limit().min(HELD)
}

/// The most bytes to gather for a codec: no decoder is given more.
pub(crate) fn readable(job: &Job<'_>) -> u64 {
    job.objects.decoded_limit().min(wide(INPUT))
}

/// Room for `bytes` bytes.
fn reserved(job: &Job<'_>, bytes: u64) -> Result<Vec<u8>, PdfError> {
    charged(job, bytes)?;
    room(usize::try_from(bytes).map_err(|_error| job.limit())?)
}

/// A count of bytes or items as a budget holds it.
fn wide(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}
