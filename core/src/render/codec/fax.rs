//! `CCITTFaxDecode` through hayro-ccitt (ISO 32000-2:2020, 7.4.6).

use core::cmp::Ordering;
use core::slice::ChunksExactMut;

use hayro_ccitt::{DecodeSettings, Decoder, DecoderContext, EncodingMode};

use crate::PdfError;
use crate::overlay::Object;
use crate::render::codec::{self, Held, Info};
use crate::render::data;
use crate::render::job::Job;

/// The bytes of a decoder's fixed state.
const BASE: u64 = 64;

/// The most columns: their positions stay far from overflow.
const COLUMNS: u32 = 0x100_0000;

/// The bytes a change of colour takes: two lists that grow by doubling.
const CHANGE: u64 = 24;

/// The rows decoded so far.
struct Rows<'row> {
    /// The rows left.
    left: ChunksExactMut<'row, u8>,
    /// The columns.
    width: u32,
    /// The rows written.
    written: u32,
}

impl Decoder for Rows<'_> {
    fn push_row(&mut self, changes: &[u32]) {
        let Some(row) = self.left.next() else {
            return;
        };
        self.written = self.written.saturating_add(1);
        for span in changes.chunks(2) {
            let start = span.first().copied().unwrap_or(self.width);
            let end = span.get(1).copied().unwrap_or(self.width).min(self.width);
            if start < end {
                flip(row, start, end);
            }
        }
    }
}

/// The samples of `data`: as many rows as the image has, those missing white.
pub(crate) fn decode(
    job: &Job<'_>,
    data: &[u8],
    parameters: &Object,
    size: [u32; 2],
) -> Result<Held, PdfError> {
    codec::admitted(job, data)?;
    let settings = settings(job, parameters, size)?;
    let black_is_one = flag(job, parameters, b"BlackIs1", false)?;
    let [width, height] = [settings.columns, settings.rows];
    let stride = u64::from(width.div_ceil(8));
    let output = stride.saturating_mul(u64::from(height));
    let bits = codec::wide(data.len()).saturating_mul(8);
    // A row changes colour at most once a column and once a bit of the data.
    let changes = u64::from(width).min(bits.saturating_add(4));
    let state = changes.saturating_mul(CHANGE).saturating_add(BASE);
    codec::charged(job, output.saturating_add(state))?;
    // A bit can end a run, which costs about two units.
    job.spend(output.saturating_add(bits.saturating_mul(2)))?;
    let mut samples = codec::filled(job, output, if black_is_one { 0 } else { 0xFF })?;
    let mut rows = Rows {
        left: samples.chunks_exact_mut(usize::try_from(stride).map_err(|_error| job.limit())?),
        width,
        written: 0,
    };
    let result = hayro_ccitt::decode(data, &mut rows, &mut DecoderContext::new(settings));
    if result.is_err() || rows.written < height {
        job.warn("7.4.6", "CCITT data damaged or short")?;
    }
    Ok(Held {
        alpha: None,
        data: samples,
        implied: None,
        info: Info {
            bits: 1,
            components: 1,
            height,
            width,
        },
        premultiplied: false,
    })
}

/// The Boolean `key` of `parameters`, `default` when missing.
fn flag(job: &Job<'_>, parameters: &Object, key: &[u8], default: bool) -> Result<bool, PdfError> {
    match *data::entry(job, parameters, key)?.value() {
        Object::Null => Ok(default),
        Object::Boolean(value) => Ok(value),
        Object::Array(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => Err(job.objects.malformed()),
    }
}

/// Turns the pixels of `row` from `start` to before `end` to the other colour.
fn flip(row: &mut [u8], start: u32, end: u32) {
    let last_pixel = end.saturating_sub(1);
    let bytes = [start >> 3_u32, last_pixel >> 3_u32].map(usize::try_from);
    let [Ok(first), Ok(last)] = bytes else {
        return;
    };
    let Some(span) = row.get_mut(first..=last) else {
        return;
    };
    let tail = last.saturating_sub(first);
    for (index, byte) in span.iter_mut().enumerate() {
        let mut mask = 0xFF_u8;
        if index == 0 {
            mask &= 0xFF_u8.wrapping_shr(start & 7);
        }
        if index == tail {
            mask &= 0xFF_u8.wrapping_shl(7_u32.saturating_sub(last_pixel & 7));
        }
        *byte ^= mask;
    }
}

/// The integer `key` of `parameters`, `default` when missing.
fn integer(job: &Job<'_>, parameters: &Object, key: &[u8], default: i64) -> Result<i64, PdfError> {
    match *data::entry(job, parameters, key)?.value() {
        Object::Null => Ok(default),
        Object::Integer(value) => Ok(value),
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Name(_)
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => Err(job.objects.malformed()),
    }
}

/// The decoder's settings (Table 11). `Rows` gives way to the image's height, which is never 0:
/// without rows the decoder has no end.
fn settings(
    job: &Job<'_>,
    parameters: &Object,
    size: [u32; 2],
) -> Result<DecodeSettings, PdfError> {
    let declared = integer(job, parameters, b"Columns", 1728)?;
    let [_, rows] = size;
    if declared < 1 || rows == 0 {
        return Err(job.objects.malformed());
    }
    let columns = u32::try_from(declared)
        .ok()
        .filter(|found| *found <= COLUMNS)
        .ok_or_else(|| job.limit())?;
    let encoding = match integer(job, parameters, b"K", 0)?.cmp(&0) {
        Ordering::Less => EncodingMode::Group4,
        Ordering::Equal => EncodingMode::Group3_1D,
        Ordering::Greater => EncodingMode::Group3_2D,
    };
    Ok(DecodeSettings {
        columns,
        encoding,
        end_of_block: flag(job, parameters, b"EndOfBlock", true)?,
        end_of_line: flag(job, parameters, b"EndOfLine", false)?,
        rows,
        rows_are_byte_aligned: flag(job, parameters, b"EncodedByteAlign", false)?,
        strict: false,
    })
}
