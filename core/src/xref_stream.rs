//! Cross-reference streams (ISO 32000-2:2020, 7.5.8).

use crate::objects::{XrefEntry, XrefKind};
use crate::pull::Pull;
use crate::store::Objects;
use crate::stream::{discard, drain};
use crate::syntax::{At, Located, Reader, Token, Value};
use crate::{ErrorCode, PdfError};

/// Bytes of the widest entry: three fields of at most 8 bytes.
const ENTRY_MAX: usize = 24;

/// The widest field in bytes.
const FIELD_MAX: i64 = 8;

/// The subsections of a cross-reference stream, read from its `Index` on demand.
struct Subsections<'store> {
    /// `0 Size` when there is no `Index`.
    default: Option<(u64, u64)>,
    /// The operation errors name.
    operation: &'static str,
    /// A reader in `Index`.
    reader: Option<Reader<Located<'store>>>,
}

impl<'store> Subsections<'store> {
    /// The next subsection: its first object number and its count; `None` after the last.
    fn next(&mut self) -> Result<Option<(u64, u64)>, PdfError> {
        let Some(reader) = self.reader.as_mut() else {
            return Ok(self.default.take());
        };
        let malformed = || PdfError::new(ErrorCode::Malformed, self.operation);
        let start = reader.token()?;
        if start == Token::ArrayEnd {
            return Ok(None);
        }
        let (Token::Integer(first), Token::Integer(count)) = (start, reader.token()?) else {
            return Err(malformed());
        };
        u64::try_from(first)
            .ok()
            .zip(u64::try_from(count).ok())
            .map(Some)
            .ok_or_else(malformed)
    }

    /// The subsections of the stream whose dictionary is at `dictionary`.
    fn of(objects: Objects<'store>, dictionary: At) -> Result<Self, PdfError> {
        let malformed = || objects.malformed();
        let size = objects
            .get(dictionary, b"Size")?
            .as_ref()
            .and_then(Value::integer)
            .and_then(|size| u64::try_from(size).ok())
            .ok_or_else(malformed)?;
        let reader = match objects.get(dictionary, b"Index")? {
            None => None,
            Some(index) => {
                let mut reader = objects.reader(index.array().ok_or_else(malformed)?)?;
                drop(reader.token()?);
                Some(reader)
            }
        };
        Ok(Self {
            default: Some((0, size)),
            operation: objects.operation(),
            reader,
        })
    }
}

/// The entries in use among those of the stream whose dictionary is at `dictionary` and whose
/// decoded data is `data`.
pub(crate) fn count(
    objects: Objects<'_>,
    dictionary: At,
    data: &mut dyn Pull,
) -> Result<u64, PdfError> {
    let fields = widths(objects, dictionary)?;
    let mut subsections = Subsections::of(objects, dictionary)?;
    let mut used: u64 = 0;
    while let Some((first, entries)) = subsections.next()? {
        for at in 0..entries {
            let number = u32::try_from(first.saturating_add(at)).unwrap_or(u32::MAX);
            if entry(data, fields, number, objects.operation())?.kind != XrefKind::Free {
                used = used.saturating_add(1);
            }
        }
    }
    Ok(used)
}

/// The entries the stream whose dictionary is at `dictionary` declares: the sum of its
/// subsections' counts.
pub(crate) fn declared(objects: Objects<'_>, dictionary: At) -> Result<u64, PdfError> {
    let mut subsections = Subsections::of(objects, dictionary)?;
    let mut total: u64 = 0;
    while let Some((_, entries)) = subsections.next()? {
        total = total.saturating_add(entries);
    }
    Ok(total)
}

/// The entry of object `number`, the next `widths` bytes of `data`; a type but 1 or 2 is free
/// (ISO 32000-2:2020, 7.5.8.3).
fn entry(
    data: &mut dyn Pull,
    widths: [usize; 3],
    number: u32,
    operation: &'static str,
) -> Result<XrefEntry, PdfError> {
    let malformed = || PdfError::new(ErrorCode::Malformed, operation);
    let [kind, middle, last] = widths;
    let mut bytes = [0; ENTRY_MAX];
    let mut filled = 0;
    let total = kind.saturating_add(middle).saturating_add(last);
    while let Some(rest) = bytes.get_mut(filled..total).filter(|rest| !rest.is_empty()) {
        match data.pull(rest)? {
            0 => return Err(malformed()),
            count => filled = filled.saturating_add(count),
        }
    }
    let field = |from: usize, len: usize| {
        bytes
            .get(from..from.saturating_add(len))
            .unwrap_or_default()
            .iter()
            .fold(0_u64, |value, byte| {
                value.wrapping_shl(8) | u64::from(*byte)
            })
    };
    let second = field(kind, middle);
    let third = field(kind.saturating_add(middle), last);
    let version = |value: u64| u16::try_from(value).ok().ok_or_else(malformed);
    let (found, generation, index, offset, stream) =
        match if kind == 0 { 1 } else { field(0, kind) } {
            1 => (XrefKind::InUse, version(third)?, 0, second, 0),
            2 => (
                XrefKind::Compressed,
                0,
                u32::try_from(third).ok().ok_or_else(malformed)?,
                0,
                u32::try_from(second).ok().ok_or_else(malformed)?,
            ),
            _ => (XrefKind::Free, 0, 0, 0, 0),
        };
    Ok(XrefEntry {
        generation,
        index,
        kind: found,
        number,
        offset,
        stream,
    })
}

/// The entry for object `number` in a cross-reference stream's `data`, scanning its whole data.
pub(crate) fn lookup(
    objects: Objects<'_>,
    dictionary: At,
    data: &mut dyn Pull,
    number: u32,
) -> Result<Option<XrefEntry>, PdfError> {
    let operation = objects.operation();
    let fields = widths(objects, dictionary)?;
    let bytes = u64::try_from(fields.iter().sum::<usize>()).unwrap_or_default();
    let mut subsections = Subsections::of(objects, dictionary)?;
    let mut found = None;
    while let Some((first, count)) = subsections.next()? {
        match u64::from(number)
            .checked_sub(first)
            .filter(|at| *at < count)
        {
            Some(at) => {
                discard(data, at.saturating_mul(bytes), operation)?;
                found = Some(entry(data, fields, number, operation)?);
                discard(
                    data,
                    count
                        .saturating_sub(at)
                        .saturating_sub(1)
                        .saturating_mul(bytes),
                    operation,
                )?;
            }
            None => discard(data, count.saturating_mul(bytes), operation)?,
        }
    }
    drain(data)?;
    Ok(found)
}

/// The byte widths of the entries' three fields, `W` (ISO 32000-2:2020, 7.5.8.2).
fn widths(objects: Objects<'_>, dictionary: At) -> Result<[usize; 3], PdfError> {
    let malformed = || objects.malformed();
    let mut array = objects.reader(
        objects
            .get(dictionary, b"W")?
            .as_ref()
            .and_then(Value::array)
            .ok_or_else(malformed)?,
    )?;
    let (
        Token::ArrayStart,
        Token::Integer(kind),
        Token::Integer(middle),
        Token::Integer(last),
        Token::ArrayEnd,
    ) = (
        array.token()?,
        array.token()?,
        array.token()?,
        array.token()?,
        array.token()?,
    )
    else {
        return Err(malformed());
    };
    if kind < 0 || middle < 1 || last < 0 {
        return Err(malformed());
    }
    if kind > FIELD_MAX || middle > FIELD_MAX || last > FIELD_MAX {
        return Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()));
    }
    Ok([kind, middle, last].map(|width| usize::try_from(width).unwrap_or_default()))
}
