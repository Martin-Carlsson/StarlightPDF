//! Stream data: reading it raw, undoing its filters and consuming it in constant memory
//! (ISO 32000-2:2020, 7.3.8 and 7.4).

use crate::inflate::Inflate;
use crate::input::Source;
use crate::predictor::Predictor;
use crate::pull::Pull;
use crate::syntax::{Dictionary, Input, Reader, Token};
use crate::{ErrorCode, PdfError};

/// Bytes discarded or read at a time.
const STEP: usize = 4096;

/// Decoded stream data as bytes at an offset that only moves forward.
pub(crate) struct Feeder<'data> {
    /// The data.
    data: &'data mut dyn Pull,
    /// The offset of the next byte to pull.
    end: u64,
}

/// The raw bytes of a stream.
struct Raw<'source> {
    /// The operation errors name.
    operation: &'static str,
    /// The offset of the next byte.
    position: u64,
    /// Bytes left.
    remaining: u64,
    /// The source.
    source: &'source Source,
}

impl<'data> Feeder<'data> {
    /// Reads the rest of the data and discards it, so a fault anywhere in it fails.
    pub(crate) fn finish(self) -> Result<(), PdfError> {
        drain(self.data)
    }

    /// A feeder at the start of `data`.
    pub(crate) const fn new(data: &'data mut dyn Pull) -> Self {
        Self { data, end: 0 }
    }

    /// Pulls into `buffer`; returns the count.
    fn pull(&mut self, buffer: &mut [u8]) -> Result<usize, PdfError> {
        let count = self.data.pull(buffer)?;
        self.end = self
            .end
            .saturating_add(u64::try_from(count).unwrap_or_default());
        Ok(count)
    }
}

impl Input for &mut Feeder<'_> {
    fn fill(
        &mut self,
        offset: u64,
        buffer: &mut [u8],
        operation: &'static str,
    ) -> Result<usize, PdfError> {
        if offset < self.end {
            return Err(PdfError::new(ErrorCode::Malformed, operation));
        }
        let mut scratch = [0; STEP];
        while self.end < offset {
            let wanted = usize::try_from(offset.saturating_sub(self.end)).unwrap_or(STEP);
            let step = scratch.get_mut(..wanted.min(STEP)).unwrap_or_default();
            if self.pull(step)? == 0 {
                return Ok(0);
            }
        }
        let mut filled: usize = 0;
        while let Some(rest) = buffer.get_mut(filled..).filter(|rest| !rest.is_empty()) {
            match self.pull(rest)? {
                0 => break,
                count => filled = filled.saturating_add(count),
            }
        }
        Ok(filled)
    }
}

impl Pull for Box<dyn Pull + '_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        (**self).pull(buf)
    }
}

impl Pull for Raw<'_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let wanted = usize::try_from(self.remaining)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let Some(part) = buf.get_mut(..wanted).filter(|part| !part.is_empty()) else {
            return Ok(0);
        };
        let count = self.source.read(self.position, part, self.operation)?;
        let step = u64::try_from(count).unwrap_or_default();
        self.position = self.position.saturating_add(step);
        self.remaining = self.remaining.saturating_sub(step);
        Ok(count)
    }
}

/// The stream data at `start`, with `FlateDecode` and a PNG predictor undone; another filter is [`ErrorCode::Unsupported`].
pub(crate) fn decoder<'source>(
    source: &'source Source,
    start: u64,
    length: u64,
    dictionary: &Dictionary,
    operation: &'static str,
) -> Result<Box<dyn Pull + 'source>, PdfError> {
    let raw = Raw {
        operation,
        position: start,
        remaining: length,
        source,
    };
    let inflate = flate(source, dictionary, operation)?;
    let parms = if inflate {
        parms(dictionary, operation)?
    } else {
        None
    };
    let data: Box<dyn Pull + 'source> = if inflate {
        Box::new(Inflate::new(raw))
    } else {
        Box::new(raw)
    };
    Ok(match parms {
        None => data,
        Some(columns) => Box::new(Predictor::new(data, columns)?),
    })
}

/// Skips `count` bytes of `data`; the end before that is [`ErrorCode::Malformed`].
pub(crate) fn discard(
    data: &mut dyn Pull,
    count: u64,
    operation: &'static str,
) -> Result<(), PdfError> {
    let mut scratch = [0; STEP];
    let mut left = count;
    while left > 0 {
        let wanted = usize::try_from(left).unwrap_or(STEP).min(STEP);
        let step = scratch.get_mut(..wanted).unwrap_or_default();
        let pulled = data.pull(step)?;
        if pulled == 0 {
            return Err(PdfError::new(ErrorCode::Malformed, operation));
        }
        left = left.saturating_sub(u64::try_from(pulled).unwrap_or_default());
    }
    Ok(())
}

/// Reads and discards the rest of `data`.
pub(crate) fn drain(data: &mut dyn Pull) -> Result<(), PdfError> {
    let mut scratch = [0; STEP];
    while data.pull(&mut scratch)? > 0 {}
    Ok(())
}

/// Whether the stream's `Filter` is `FlateDecode`, as a name or in an array of one; no filter is
/// not (ISO 32000-2:2020, 7.3.8.2).
fn flate(
    source: &Source,
    dictionary: &Dictionary,
    operation: &'static str,
) -> Result<bool, PdfError> {
    let unsupported = || PdfError::new(ErrorCode::Unsupported, operation);
    let Some(filter) = dictionary.get(b"Filter") else {
        return Ok(false);
    };
    if let Some(start) = filter.array() {
        let mut reader = Reader::new(source, start, operation);
        let mut names = 0_u32;
        let mut is_flate = false;
        let Token::ArrayStart = reader.token()? else {
            return Err(PdfError::new(ErrorCode::Malformed, operation));
        };
        loop {
            match reader.token()? {
                Token::ArrayEnd => break,
                Token::Name(name) => {
                    names = names.saturating_add(1);
                    is_flate = name == b"FlateDecode";
                }
                Token::ArrayStart
                | Token::DictionaryEnd
                | Token::DictionaryStart
                | Token::Integer(_)
                | Token::Keyword(_)
                | Token::Other => return Err(PdfError::new(ErrorCode::Malformed, operation)),
            }
        }
        return if names == 1 && is_flate {
            Ok(true)
        } else {
            Err(unsupported())
        };
    }
    if filter.is_name(b"FlateDecode") {
        Ok(true)
    } else if filter.is_named() || filter.reference().is_some() {
        Err(unsupported())
    } else {
        Err(PdfError::new(ErrorCode::Malformed, operation))
    }
}

/// The `Columns` of the stream's PNG predictor, none for predictor 1 (ISO 32000-2:2020, 7.4.4.4).
fn parms(dictionary: &Dictionary, operation: &'static str) -> Result<Option<u64>, PdfError> {
    let malformed = || PdfError::new(ErrorCode::Malformed, operation);
    let unsupported = || PdfError::new(ErrorCode::Unsupported, operation);
    if dictionary
        .get(b"DecodeParms")
        .is_some_and(|found| found.reference().is_some())
    {
        return Err(unsupported());
    }
    let integer = |key: &[u8], default: i64| {
        dictionary
            .get(key)
            .map_or(Ok(default), |value| value.integer().ok_or_else(malformed))
    };
    let png = match integer(b"Predictor", 1)? {
        2 => return Err(unsupported()),
        1 => false,
        10..=15 => true,
        _ => return Err(malformed()),
    };
    if integer(b"Colors", 1)? != 1 || integer(b"BitsPerComponent", 8)? != 8 {
        return Err(unsupported());
    }
    let columns = u64::try_from(integer(b"Columns", 1)?)
        .ok()
        .ok_or_else(malformed)?;
    Ok(png.then_some(columns))
}
