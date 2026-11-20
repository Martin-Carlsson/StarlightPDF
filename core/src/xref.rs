//! Cross-reference tables and streams (ISO 32000-2:2020, 7.5.4 to 7.5.8).

use crate::objects::{XrefEntry, XrefKind};
use crate::store::Objects;
use crate::stream::decoder;
use crate::syntax::{At, Input, Keyword, Reader, Token, is_white_space};
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

/// What a section holds for a request.
enum Lookup {
    /// The entry asked for.
    Entry(XrefEntry),
    /// No such entry: the section's trailer and its entries, declared or, for
    /// [`Want::Count`], in use.
    Trailer(At, u64),
}

/// What is asked of a section.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Want {
    /// Its entries in use, counted.
    Count,
    /// The entry of an object.
    Entry(u32),
    /// Its trailer and declared entries.
    Trailer,
}

impl Chain {
    /// A chain from `offset`.
    const fn new(offset: u64) -> Self {
        Self {
            power: 1,
            saved: offset,
            steps: 0,
        }
    }

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

/// The non-negative integer at `key` of `trailer`, when present.
fn offset_at(objects: Objects<'_>, trailer: At, key: &[u8]) -> Result<Option<u64>, PdfError> {
    objects.get(trailer, key)?.map_or(Ok(None), |found| {
        found
            .integer()
            .and_then(|offset| u64::try_from(offset).ok())
            .map(Some)
            .ok_or_else(|| objects.malformed())
    })
}

/// The newest entry for object `number` along the `Prev` chain from `newest`
/// (ISO 32000-2:2020, 7.5.6 and 7.5.8.4).
pub(crate) fn entry(
    objects: Objects<'_>,
    newest: u64,
    number: u32,
) -> Result<Option<XrefEntry>, PdfError> {
    let mut chain = Chain::new(newest);
    let mut offset = newest;
    loop {
        let trailer = match section(objects, offset, Want::Entry(number))? {
            Lookup::Entry(entry) => return Ok(Some(entry)),
            Lookup::Trailer(trailer, _) => trailer,
        };
        if let Some(hybrid) = offset_at(objects, trailer, b"XRefStm")?
            && let Lookup::Entry(entry) = section(objects, hybrid, Want::Entry(number))?
        {
            return Ok(Some(entry));
        }
        match offset_at(objects, trailer, b"Prev")? {
            Some(prev) => {
                chain.step(prev, objects.operation())?;
                offset = prev;
            }
            None => return Ok(None),
        }
    }
}

/// The entries in use the sections along the `Prev` chain from `newest` hold, an object
/// counted once per section that lists it.
pub(crate) fn in_use(objects: Objects<'_>, newest: u64) -> Result<u64, PdfError> {
    walk_for(objects, newest, Want::Count).map(|(_, used)| used)
}

/// Whether a cross-reference table or stream starts at `offset` (ISO 32000-2:2020, 7.5.4 and 7.5.8).
pub(crate) fn is_section(objects: Objects<'_>, offset: u64) -> Result<bool, PdfError> {
    let mut keyword = [0; XREF.len() + 1];
    let read = objects
        .source()
        .read(offset, &mut keyword, objects.operation())?;
    if read == keyword.len()
        && keyword
            .split_last()
            .is_some_and(|(last, start)| start == XREF && is_white_space(*last))
    {
        return Ok(true);
    }
    let mut reader = Reader::new(objects.source(), offset, objects.operation());
    Ok(object_header(&mut reader, objects.operation())?.is_some())
}

/// `error`, or [`ErrorCode::Malformed`] unless it is [`ErrorCode::Io`].
fn malformed_unless_io(error: PdfError, operation: &'static str) -> PdfError {
    if error.code() == ErrorCode::Io {
        error
    } else {
        PdfError::new(ErrorCode::Malformed, operation)
    }
}

/// `number generation obj` right at the reader, the start of an indirect object
/// (ISO 32000-2:2020, 7.3.10); a failure other than [`ErrorCode::Io`] is [`ErrorCode::Malformed`].
pub(crate) fn object_header<I>(
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

/// The entry at `offset` for object `number` (ISO 32000-2:2020, 7.5.4).
fn read_entry(objects: Objects<'_>, offset: u64, number: u32) -> Result<XrefEntry, PdfError> {
    let mut bytes = [0; ENTRY_LEN];
    let read = objects
        .source()
        .read(offset, &mut bytes, objects.operation())?;
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
        return Err(objects.malformed());
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
        _ => Err(objects.malformed()),
    }
}

/// What the section at `offset` holds for `want` (ISO 32000-2:2020, 7.5.4 and 7.5.5).
///
/// Unless counting, reads only the subsection headers, the entry and the last entry of each
/// other subsection.
fn section(objects: Objects<'_>, offset: u64, want: Want) -> Result<Lookup, PdfError> {
    let malformed = || objects.malformed();
    let entry_len = u64::try_from(ENTRY_LEN).ok().ok_or_else(malformed)?;
    let mut keyword = [0; XREF.len() + 1];
    let read = objects
        .source()
        .read(offset, &mut keyword, objects.operation())?;
    let at_xref = read == keyword.len()
        && keyword
            .split_last()
            .is_some_and(|(last, start)| start == XREF && is_white_space(*last));
    if !at_xref {
        return stream_section(objects, offset, want);
    }
    let mut reader = Reader::new(
        objects.source(),
        offset.saturating_add(u64::try_from(XREF.len()).unwrap_or_default()),
        objects.operation(),
    );
    let mut total: u64 = 0;
    loop {
        let first = match reader.token()? {
            Token::Keyword(Keyword::Trailer) => {
                let trailer = reader
                    .head(objects.depth_limit())?
                    .dictionary()
                    .ok_or_else(malformed)?;
                return Ok(Lookup::Trailer(trailer, total));
            }
            Token::Integer(first) => first,
            Token::ArrayEnd
            | Token::ArrayStart
            | Token::DictionaryEnd
            | Token::DictionaryStart
            | Token::Keyword(_)
            | Token::Name(_)
            | Token::Real(_)
            | Token::Reference(_)
            | Token::String(_) => return Err(malformed()),
        };
        let Token::Integer(count) = reader.token()? else {
            return Err(malformed());
        };
        reader.skip_white_space()?;
        let start = reader.position();
        let (Ok(first_number), Ok(entries)) = (u64::try_from(first), u64::try_from(count)) else {
            return Err(malformed());
        };
        let end = entries
            .checked_mul(entry_len)
            .and_then(|len| start.checked_add(len))
            .ok_or_else(malformed)?;
        let number_at = |index: u64| u32::try_from(first_number.saturating_add(index));
        if let Want::Entry(wanted) = want
            && let Some(index) = u64::from(wanted).checked_sub(first_number)
            && index < entries
        {
            let at = index.saturating_mul(entry_len).saturating_add(start);
            return read_entry(objects, at, wanted).map(Lookup::Entry);
        }
        if want == Want::Count {
            for index in 0..entries {
                let at = index.saturating_mul(entry_len).saturating_add(start);
                let number = number_at(index).ok().ok_or_else(malformed)?;
                if read_entry(objects, at, number)?.kind == XrefKind::InUse {
                    total = total.saturating_add(1);
                }
            }
        } else {
            if let Some(last) = end.checked_sub(entry_len).filter(|_| entries > 0) {
                let _last = read_entry(objects, last, 0)?;
            }
            total = total.saturating_add(entries);
        }
        reader.seek(end);
    }
}

/// The offset of the newest section, from `startxref` in the last 1 KiB (ISO 32000-2:2020, 7.5.5).
pub(crate) fn startxref(objects: Objects<'_>) -> Result<u64, PdfError> {
    let malformed = || objects.malformed();
    let source = objects.source();
    let size = source.size(objects.operation())?;
    let tail_start = size.saturating_sub(TAIL_LEN);
    let mut tail = [0; 1024];
    let count = source.read(tail_start, &mut tail, objects.operation())?;
    let keyword = tail
        .get(..count)
        .and_then(|bytes| {
            bytes
                .windows(STARTXREF.len())
                .rposition(|window| window == STARTXREF)
        })
        .and_then(|index| u64::try_from(index.saturating_add(STARTXREF.len())).ok())
        .ok_or_else(malformed)?;
    let mut reader = Reader::new(
        source,
        tail_start.saturating_add(keyword),
        objects.operation(),
    );
    let Token::Integer(newest) = reader.token()? else {
        return Err(malformed());
    };
    u64::try_from(newest).ok().ok_or_else(malformed)
}

/// What the cross-reference stream at `offset` holds for `want`; its dictionary is its trailer
/// (ISO 32000-2:2020, 7.5.8).
fn stream_section(objects: Objects<'_>, offset: u64, want: Want) -> Result<Lookup, PdfError> {
    let operation = objects.operation();
    let mut reader = Reader::new(objects.source(), offset, operation);
    if object_header(&mut reader, operation)?.is_none() {
        return Err(objects.malformed());
    }
    let dictionary = reader
        .value(objects.depth_limit())
        .map_err(|error| malformed_unless_io(error, operation))?
        .dictionary()
        .ok_or_else(|| objects.malformed())?;
    if !objects
        .get(dictionary, b"Type")?
        .is_some_and(|kind| kind.is_name(b"XRef"))
    {
        return Err(objects.malformed());
    }
    if want == Want::Trailer {
        let declared = xref_stream::declared(objects, dictionary)?;
        return Ok(Lookup::Trailer(dictionary, declared));
    }
    let (start, length) = objects.extent(&mut reader, dictionary, false)?;
    let mut data = decoder(objects, None, start, length, dictionary, true)?;
    match want {
        Want::Entry(number) => Ok(
            xref_stream::lookup(objects, dictionary, &mut *data, number)?
                .map_or(Lookup::Trailer(dictionary, 0), Lookup::Entry),
        ),
        Want::Count | Want::Trailer => Ok(Lookup::Trailer(
            dictionary,
            xref_stream::count(objects, dictionary, &mut *data)?,
        )),
    }
}

/// The newest trailer and the entries the sections along the `Prev` chain from `newest` declare
/// (ISO 32000-2:2020, 7.5.6 and 7.5.8.4).
pub(crate) fn walk(objects: Objects<'_>, newest: u64) -> Result<(At, u64), PdfError> {
    walk_for(objects, newest, Want::Trailer)
}

/// The newest trailer and the sum over the sections along the `Prev` chain of what `want`
/// counts.
fn walk_for(objects: Objects<'_>, newest: u64, want: Want) -> Result<(At, u64), PdfError> {
    let mut chain = Chain::new(newest);
    let mut offset = newest;
    let mut first = None;
    let mut total: u64 = 0;
    loop {
        let Lookup::Trailer(trailer, entries) = section(objects, offset, want)? else {
            return Err(objects.malformed());
        };
        total = total.saturating_add(entries);
        let newest_trailer = *first.get_or_insert(trailer);
        if let Some(hybrid) = offset_at(objects, trailer, b"XRefStm")? {
            let Lookup::Trailer(_, more) = section(objects, hybrid, want)? else {
                return Err(objects.malformed());
            };
            total = total.saturating_add(more);
        }
        match offset_at(objects, trailer, b"Prev")? {
            Some(prev) => {
                chain.step(prev, objects.operation())?;
                offset = prev;
            }
            None => return Ok((newest_trailer, total)),
        }
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
