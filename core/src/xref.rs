//! Classic cross-reference tables and the indirect objects they locate (ISO 32000-2:2020, 7.5).

use crate::input::Source;
use crate::objects::{ObjectId, XrefEntry, XrefKind};
use crate::syntax::{Dictionary, Keyword, Object, Reader, Token, is_white_space};
use crate::{ErrorCode, PdfError};

/// Bytes per entry (ISO 32000-2:2020, 7.5.4).
const ENTRY_LEN: usize = 20;

/// The keyword a section starts with (ISO 32000-2:2020, 7.5.4).
const XREF: &[u8] = b"xref";

/// The keyword before the newest section's offset (ISO 32000-2:2020, 7.5.5).
const STARTXREF: &[u8] = b"startxref";

/// Bytes from the end searched for [`STARTXREF`].
const TAIL_LEN: u64 = 1024;

/// Detects a loop in the `Prev` chain in constant memory (Brent's algorithm).
struct Chain {
    /// Steps before `saved` moves.
    power: u64,
    /// The offset later ones are compared with.
    saved: u64,
    /// Steps since `saved` moved.
    steps: u64,
}

/// What a section holds for a looked-up object.
enum Lookup {
    /// Its entry.
    Entry(XrefEntry),
    /// No entry; the section's trailer.
    Trailer(Dictionary),
}

/// The cross-reference sections of a source, read on demand.
pub(crate) struct Xref<'source> {
    /// Nesting depth of arrays and dictionaries.
    depth_limit: u32,
    /// The newest section's offset.
    newest: u64,
    /// The operation errors name.
    operation: &'static str,
    /// The source.
    source: &'source Source,
}

impl Chain {
    /// Records the next offset; a repeat fails with [`ErrorCode::Malformed`].
    fn step(&mut self, offset: u64, operation: &'static str) -> Result<(), PdfError> {
        if offset == self.saved {
            return Err(PdfError::new(ErrorCode::Malformed, operation));
        }
        self.steps = self.steps.saturating_add(1);
        if self.steps == self.power {
            self.saved = offset;
            self.power = self.power.saturating_mul(2);
            self.steps = 0;
        }
        Ok(())
    }
}

impl<'source> Xref<'source> {
    /// A [`Chain`] from the newest section.
    const fn chain(&self) -> Chain {
        Chain {
            power: 1,
            saved: self.newest,
            steps: 0,
        }
    }

    /// The dictionary `reference` points at; anything else is [`ErrorCode::Malformed`].
    pub(crate) fn dictionary(&self, reference: Option<&Object>) -> Result<Dictionary, PdfError> {
        match reference.and_then(Object::reference) {
            Some(id) => self.object(id)?.dictionary(),
            None => None,
        }
        .ok_or_else(|| self.malformed())
    }

    /// The newest entry for object `number` along the `Prev` chain (ISO 32000-2:2020, 7.5.6).
    fn entry(&self, number: u32) -> Result<Option<XrefEntry>, PdfError> {
        let mut chain = self.chain();
        let mut offset = self.newest;
        loop {
            match self.section(offset, Some(number))? {
                Lookup::Entry(entry) => return Ok(Some(entry)),
                Lookup::Trailer(trailer) => match self.prev(&trailer)? {
                    Some(prev) => {
                        chain.step(prev, self.operation)?;
                        offset = prev;
                    }
                    None => return Ok(None),
                },
            }
        }
    }

    /// A [`ErrorCode::Malformed`] error.
    fn malformed(&self) -> PdfError {
        PdfError::new(ErrorCode::Malformed, self.operation)
    }

    /// Indirect object `id`; null ([`Object::Other`]) when free or missing (ISO 32000-2:2020, 7.3.10).
    pub(crate) fn object(&self, id: ObjectId) -> Result<Object, PdfError> {
        let Some(entry) = self
            .entry(id.number)?
            .filter(|found| found.kind == XrefKind::InUse && found.generation == id.generation)
        else {
            return Ok(Object::Other);
        };
        let mut reader = Reader::new(self.source, entry.offset, self.operation);
        if object_header(&mut reader, self.operation)?
            != Some((i64::from(id.number), i64::from(id.generation)))
        {
            return Err(self.malformed());
        }
        reader.object(self.depth_limit)
    }

    /// The newest section through `startxref`, and its trailer, after checking the `Prev` chain
    /// (ISO 32000-2:2020, 7.5.5 and 7.5.6); objects nest at most `depth_limit` deep.
    pub(crate) fn open(
        source: &'source Source,
        depth_limit: u32,
        operation: &'static str,
    ) -> Result<(Self, Dictionary), PdfError> {
        let malformed = || PdfError::new(ErrorCode::Malformed, operation);
        let size = source.size(operation)?;
        let tail_start = size.saturating_sub(TAIL_LEN);
        let mut tail = vec![0; usize::try_from(size.min(TAIL_LEN)).unwrap_or_default()];
        let count = source.read(tail_start, &mut tail, operation)?;
        let keyword = tail
            .get(..count)
            .and_then(|bytes| {
                bytes
                    .windows(STARTXREF.len())
                    .rposition(|window| window == STARTXREF)
            })
            .and_then(|index| u64::try_from(index.saturating_add(STARTXREF.len())).ok())
            .ok_or_else(malformed)?;
        let mut reader = Reader::new(source, tail_start.saturating_add(keyword), operation);
        let Token::Integer(newest) = reader.token()? else {
            return Err(malformed());
        };
        let xref = Self {
            depth_limit,
            newest: u64::try_from(newest).ok().ok_or_else(malformed)?,
            operation,
            source,
        };
        let Lookup::Trailer(trailer) = xref.section(xref.newest, None)? else {
            return Err(malformed());
        };
        let mut chain = xref.chain();
        let mut prev = xref.prev(&trailer)?;
        while let Some(offset) = prev {
            chain.step(offset, operation)?;
            let Lookup::Trailer(older) = xref.section(offset, None)? else {
                return Err(malformed());
            };
            prev = xref.prev(&older)?;
        }
        Ok((xref, trailer))
    }

    /// The offset of the section before the one `trailer` ends, from its `Prev`.
    fn prev(&self, trailer: &Dictionary) -> Result<Option<u64>, PdfError> {
        trailer.get(b"Prev").map_or(Ok(None), |prev| {
            prev.integer()
                .and_then(|offset| u64::try_from(offset).ok())
                .map(Some)
                .ok_or_else(|| self.malformed())
        })
    }

    /// The entry at `offset` for object `number` (ISO 32000-2:2020, 7.5.4).
    fn read_entry(&self, offset: u64, number: u32) -> Result<XrefEntry, PdfError> {
        let mut bytes = [0; ENTRY_LEN];
        let read = self.source.read(offset, &mut bytes, self.operation)?;
        let [
            offset_digits @ ..,
            b' ',
            g0,
            g1,
            g2,
            g3,
            g4,
            b' ',
            letter,
            end0,
            end1,
        ] = bytes
        else {
            return Err(self.malformed());
        };
        let kind = match letter {
            b'f' => Some(XrefKind::Free),
            b'n' => Some(XrefKind::InUse),
            _ => None,
        };
        let generation = decimal(&[g0, g1, g2, g3, g4]).and_then(|value| u16::try_from(value).ok());
        match (kind, decimal(&offset_digits), generation) {
            (Some(found), Some(at), Some(version))
                if read == bytes.len()
                    && matches!((end0, end1), (b' ', b'\r' | b'\n') | (b'\r', b'\n')) =>
            {
                Ok(XrefEntry {
                    generation: version,
                    index: 0,
                    kind: found,
                    number,
                    offset: at,
                    stream: 0,
                })
            }
            _ => Err(self.malformed()),
        }
    }

    /// The section at `offset`: object `number`'s entry if it has one, else its trailer
    /// (ISO 32000-2:2020, 7.5.4 and 7.5.5).
    ///
    /// Reads only the subsection headers, the entry and the last entry of each skipped subsection.
    fn section(&self, offset: u64, number: Option<u32>) -> Result<Lookup, PdfError> {
        let entry_len = u64::try_from(ENTRY_LEN)
            .ok()
            .ok_or_else(|| self.malformed())?;
        let mut keyword = [0; XREF.len() + 1];
        let read = self.source.read(offset, &mut keyword, self.operation)?;
        let at_xref = read == keyword.len()
            && keyword
                .split_last()
                .is_some_and(|(last, start)| start == XREF && is_white_space(*last));
        if !at_xref {
            let mut reader = Reader::new(self.source, offset, self.operation);
            let stream = object_header(&mut reader, self.operation)?.is_some()
                && reader
                    .object(self.depth_limit)
                    .map_err(|error| malformed_unless_io(error, self.operation))?
                    .dictionary()
                    .is_some_and(|dictionary| {
                        dictionary.get(b"Type") == Some(&Object::Name(b"XRef".to_vec()))
                    });
            return Err(if stream {
                PdfError::new(ErrorCode::Unsupported, self.operation)
            } else {
                self.malformed()
            });
        }
        let mut reader = Reader::new(
            self.source,
            offset.saturating_add(u64::try_from(XREF.len()).unwrap_or_default()),
            self.operation,
        );
        loop {
            let first = match reader.token()? {
                Token::Keyword(Keyword::Trailer) => {
                    return reader
                        .object(self.depth_limit)?
                        .dictionary()
                        .map(Lookup::Trailer)
                        .ok_or_else(|| self.malformed());
                }
                Token::Integer(first) => first,
                Token::ArrayEnd
                | Token::ArrayStart
                | Token::DictionaryEnd
                | Token::DictionaryStart
                | Token::Keyword(_)
                | Token::Name(_)
                | Token::Other => return Err(self.malformed()),
            };
            let Token::Integer(count) = reader.token()? else {
                return Err(self.malformed());
            };
            reader.skip_white_space()?;
            let start = reader.position();
            let (Ok(first_number), Ok(entries)) = (u64::try_from(first), u64::try_from(count))
            else {
                return Err(self.malformed());
            };
            let end = entries
                .checked_mul(entry_len)
                .and_then(|len| start.checked_add(len))
                .ok_or_else(|| self.malformed())?;
            if let Some(wanted) = number
                && let Some(index) = u64::from(wanted).checked_sub(first_number)
                && index < entries
            {
                let at = index.saturating_mul(entry_len).saturating_add(start);
                return self.read_entry(at, wanted).map(Lookup::Entry);
            }
            if let Some(last) = end.checked_sub(entry_len).filter(|_| entries > 0) {
                let _entry = self.read_entry(last, 0)?;
            }
            reader.seek(end);
        }
    }
}

/// `number generation obj` right at the reader, the start of an indirect object
/// (ISO 32000-2:2020, 7.3.10); a failure other than [`ErrorCode::Io`] is [`ErrorCode::Malformed`].
fn object_header(
    reader: &mut Reader<'_>,
    operation: &'static str,
) -> Result<Option<(i64, i64)>, PdfError> {
    if !reader.at_token()? {
        return Ok(None);
    }
    let mut next = || {
        reader
            .token()
            .map_err(|error| malformed_unless_io(error, operation))
    };
    if let Token::Integer(number) = next()?
        && let Token::Integer(generation) = next()?
        && next()? == Token::Keyword(Keyword::Obj)
    {
        Ok(Some((number, generation)))
    } else {
        Ok(None)
    }
}

/// `error`, or [`ErrorCode::Malformed`] unless it is [`ErrorCode::Io`].
fn malformed_unless_io(error: PdfError, operation: &'static str) -> PdfError {
    if error.code() == ErrorCode::Io {
        error
    } else {
        PdfError::new(ErrorCode::Malformed, operation)
    }
}

/// The value of ASCII decimal `digits`.
fn decimal(digits: &[u8]) -> Option<u64> {
    digits.iter().try_fold(0_u64, |value, digit| {
        if digit.is_ascii_digit() {
            value
                .checked_mul(10)?
                .checked_add(u64::from(digit.wrapping_sub(b'0')))
        } else {
            None
        }
    })
}
