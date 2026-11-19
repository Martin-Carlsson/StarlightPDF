//! Cross-reference streams (ISO 32000-2:2020, 7.5.8).

use crate::input::Source;
use crate::objects::{XrefEntry, XrefKind};
use crate::pull::Pull;
use crate::stream::{discard, drain};
use crate::syntax::{Dictionary, Object, Reader, Token};
use crate::{ErrorCode, PdfError};

/// Bytes of the widest entry: three fields of at most 8 bytes.
const ENTRY_MAX: usize = 24;

/// The widest field in bytes.
const FIELD_MAX: i64 = 8;

/// The next subsection of `Index`: its first object number and its count; `None` at the end.
fn subsection(
    reader: &mut Reader<&Source>,
    operation: &'static str,
) -> Result<Option<(u64, u64)>, PdfError> {
    let malformed = || PdfError::new(ErrorCode::Malformed, operation);
    let start = reader.token()?;
    if start == Token::ArrayEnd {
        return Ok(None);
    }
    let Token::Integer(first) = start else {
        return Err(malformed());
    };
    let Token::Integer(count) = reader.token()? else {
        return Err(malformed());
    };
    u64::try_from(first)
        .ok()
        .zip(u64::try_from(count).ok())
        .map(Some)
        .ok_or_else(malformed)
}

/// The entry for object `number` in a cross-reference stream's `data`, scanning its whole data (7.5.8).
pub(crate) fn lookup(
    source: &Source,
    dictionary: &Dictionary,
    data: &mut dyn Pull,
    number: u32,
    operation: &'static str,
) -> Result<Option<XrefEntry>, PdfError> {
    let malformed = || PdfError::new(ErrorCode::Malformed, operation);
    let size = dictionary
        .get(b"Size")
        .and_then(Object::integer)
        .and_then(|size| u64::try_from(size).ok())
        .ok_or_else(malformed)?;
    let mut array = Reader::new(
        source,
        dictionary
            .get(b"W")
            .and_then(Object::array)
            .ok_or_else(malformed)?,
        operation,
    );
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
        return Err(PdfError::new(ErrorCode::LimitExceeded, operation));
    }
    let fields = [kind, middle, last].map(|width| usize::try_from(width).unwrap_or_default());
    let width = fields.iter().sum::<usize>();
    let mut index = match dictionary.get(b"Index") {
        None => None,
        Some(subsections) => {
            let mut reader = Reader::new(
                source,
                subsections.array().ok_or_else(malformed)?,
                operation,
            );
            drop(reader.token()?);
            Some(reader)
        }
    };
    let mut default = Some((0, size));
    let mut found = None;
    loop {
        let next = match index.as_mut() {
            Some(reader) => subsection(reader, operation)?,
            None => default.take(),
        };
        let Some((first, count)) = next else { break };
        let bytes = u64::try_from(width).unwrap_or_default();
        match u64::from(number)
            .checked_sub(first)
            .filter(|at| *at < count)
        {
            Some(at) => {
                discard(data, at.saturating_mul(bytes), operation)?;
                found = Some(entry(data, fields, number, operation)?);
            }
            None => discard(data, count.saturating_mul(bytes), operation)?,
        }
    }
    drain(data)?;
    Ok(found)
}

/// The entry of object `number`, the next `widths` bytes of `data`.
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
            _ => return Err(malformed()),
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
