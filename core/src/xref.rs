//! Cross-reference tables and streams and the indirect objects they locate (ISO 32000-2:2020, 7.5).

use crate::input::Source;
use crate::object_stream;
use crate::objects::{ObjectId, XrefEntry, XrefKind};
use crate::pull::Pull;
use crate::stream::decoder;
use crate::syntax::{Dictionary, Input, Keyword, Object, Reader, Token, is_white_space};
use crate::xref_stream;
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
    /// Whether the newest trailer has `Encrypt`.
    encrypted: bool,
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

    /// The object `number` in the object stream `entry` names, at the entry's index
    /// (ISO 32000-2:2020, 7.5.7).
    fn compressed(&self, entry: XrefEntry, number: u32) -> Result<Object, PdfError> {
        if self.encrypted {
            return Err(PdfError::new(ErrorCode::Unsupported, self.operation));
        }
        let container = self
            .entry(entry.stream)?
            .filter(|found| found.kind == XrefKind::InUse)
            .ok_or_else(|| self.malformed())?;
        let mut reader = Reader::new(self.source, container.offset, self.operation);
        if object_header(&mut reader, self.operation)?
            != Some((i64::from(entry.stream), i64::from(container.generation)))
        {
            return Err(self.malformed());
        }
        let dictionary = reader
            .object(self.depth_limit)?
            .dictionary()
            .ok_or_else(|| self.malformed())?;
        let mut data = self.data(&mut reader, &dictionary, true)?;
        object_stream::find(
            &mut *data,
            &dictionary,
            number,
            entry.index,
            self.depth_limit,
            self.operation,
        )
    }

    /// The decoded data of the stream whose dictionary `reader` has just read; `length_indirect`
    /// allows a `Length` that is a reference (ISO 32000-2:2020, 7.3.8.2).
    fn data<I>(
        &self,
        reader: &mut Reader<I>,
        dictionary: &Dictionary,
        length_indirect: bool,
    ) -> Result<Box<dyn Pull + 'source>, PdfError>
    where
        I: Input,
    {
        let start = reader.stream_start()?;
        let found = dictionary.get(b"Length").ok_or_else(|| self.malformed())?;
        let length = if let Some(value) = found.integer() {
            value
        } else if let Some(id) = found.reference() {
            if !length_indirect {
                return Err(PdfError::new(ErrorCode::Unsupported, self.operation));
            }
            self.length(id)?
        } else {
            return Err(self.malformed());
        };
        let size = self.source.size(self.operation)?;
        let end = u64::try_from(length)
            .ok()
            .and_then(|bytes| start.checked_add(bytes))
            .filter(|end| *end <= size)
            .ok_or_else(|| self.malformed())?;
        decoder(
            self.source,
            start,
            end.saturating_sub(start),
            dictionary,
            self.operation,
        )
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
                Lookup::Trailer(trailer) => {
                    if let Some(hybrid) = trailer.get(b"XRefStm") {
                        let at = hybrid
                            .integer()
                            .and_then(|stream| u64::try_from(stream).ok())
                            .ok_or_else(|| self.malformed())?;
                        if let Lookup::Entry(entry) = self.section(at, Some(number))? {
                            return Ok(Some(entry));
                        }
                    }
                    match self.prev(&trailer)? {
                        Some(prev) => {
                            chain.step(prev, self.operation)?;
                            offset = prev;
                        }
                        None => return Ok(None),
                    }
                }
            }
        }
    }

    /// The integer in the indirect object `id`, which must be in use and not in an object stream.
    fn length(&self, id: ObjectId) -> Result<i64, PdfError> {
        let entry = self
            .entry(id.number)?
            .filter(|found| found.kind == XrefKind::InUse)
            .ok_or_else(|| self.malformed())?;
        self.read_object(entry.offset, id)?
            .integer()
            .ok_or_else(|| self.malformed())
    }

    /// A [`ErrorCode::Malformed`] error.
    fn malformed(&self) -> PdfError {
        PdfError::new(ErrorCode::Malformed, self.operation)
    }

    /// Indirect object `id`; null ([`Object::Other`]) when free or missing (ISO 32000-2:2020, 7.3.10).
    pub(crate) fn object(&self, id: ObjectId) -> Result<Object, PdfError> {
        match self.entry(id.number)? {
            Some(entry) if entry.kind == XrefKind::InUse && entry.generation == id.generation => {
                self.read_object(entry.offset, id)
            }
            Some(entry) if entry.kind == XrefKind::Compressed && id.generation == 0 => {
                self.compressed(entry, id.number)
            }
            Some(_) | None => Ok(Object::Other),
        }
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
        let mut xref = Self {
            depth_limit,
            encrypted: false,
            newest: u64::try_from(newest).ok().ok_or_else(malformed)?,
            operation,
            source,
        };
        let Lookup::Trailer(trailer) = xref.section(xref.newest, None)? else {
            return Err(malformed());
        };
        xref.encrypted = trailer.get(b"Encrypt").is_some();
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

    /// The object `id` at `offset`.
    fn read_object(&self, offset: u64, id: ObjectId) -> Result<Object, PdfError> {
        let mut reader = Reader::new(self.source, offset, self.operation);
        if object_header(&mut reader, self.operation)?
            != Some((i64::from(id.number), i64::from(id.generation)))
        {
            return Err(self.malformed());
        }
        reader.object(self.depth_limit)
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
            return self.stream_section(offset, number);
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

    /// The cross-reference stream at `offset`: object `number`'s entry if it has one, else its
    /// dictionary, which is its trailer (ISO 32000-2:2020, 7.5.8).
    fn stream_section(&self, offset: u64, number: Option<u32>) -> Result<Lookup, PdfError> {
        let mut reader = Reader::new(self.source, offset, self.operation);
        if object_header(&mut reader, self.operation)?.is_none() {
            return Err(self.malformed());
        }
        let dictionary = reader
            .object(self.depth_limit)
            .map_err(|error| malformed_unless_io(error, self.operation))?
            .dictionary()
            .filter(|found| found.get(b"Type").is_some_and(|kind| kind.is_name(b"XRef")))
            .ok_or_else(|| self.malformed())?;
        let Some(wanted) = number else {
            return Ok(Lookup::Trailer(dictionary));
        };
        let mut data = self.data(&mut reader, &dictionary, false)?;
        Ok(
            xref_stream::lookup(self.source, &dictionary, &mut *data, wanted, self.operation)?
                .map_or(Lookup::Trailer(dictionary), Lookup::Entry),
        )
    }
}

/// `number generation obj` right at the reader, the start of an indirect object
/// (ISO 32000-2:2020, 7.3.10); a failure other than [`ErrorCode::Io`] is [`ErrorCode::Malformed`].
fn object_header<I>(
    reader: &mut Reader<I>,
    operation: &'static str,
) -> Result<Option<(i64, i64)>, PdfError>
where
    I: Input,
{
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
