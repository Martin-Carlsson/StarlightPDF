//! Object streams (ISO 32000-2:2020, 7.5.7).

use crate::pull::Pull;
use crate::store::Objects;
use crate::stream::Feeder;
use crate::syntax::{At, Reader, Token, Value};
use crate::{ErrorCode, PdfError};

/// `First` and `N` of the object stream whose dictionary is at `dictionary`.
fn counts(objects: Objects<'_>, dictionary: At) -> Result<(u64, u64), PdfError> {
    let non_negative = |key: &[u8]| {
        objects
            .get(dictionary, key)?
            .as_ref()
            .and_then(Value::integer)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| objects.malformed())
    };
    Ok((non_negative(b"First")?, non_negative(b"N")?))
}

/// The object numbers the object stream whose dictionary is at `dictionary` and whose decoded
/// data is `data` holds, in index order; more than `limit` is [`ErrorCode::LimitExceeded`].
pub(crate) fn numbers(
    objects: Objects<'_>,
    data: Box<dyn Pull + '_>,
    dictionary: At,
    limit: u64,
) -> Result<Vec<u32>, PdfError> {
    let (_, count) = counts(objects, dictionary)?;
    if count > limit {
        return Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()));
    }
    let mut reader = Reader::new(Feeder::new(data), 0, objects.operation());
    let mut numbers = Vec::new();
    for _ in 0..count {
        let (Token::Integer(number), Token::Integer(_)) = (reader.token()?, reader.token()?) else {
            return Err(objects.malformed());
        };
        numbers.push(
            u32::try_from(number)
                .ok()
                .ok_or_else(|| objects.malformed())?,
        );
    }
    Ok(numbers)
}

/// A reader at object `number`, at `index` in the object stream whose dictionary is at
/// `dictionary` and whose decoded data is `data`.
pub(crate) fn member<'data>(
    objects: Objects<'_>,
    data: Box<dyn Pull + 'data>,
    dictionary: At,
    number: u32,
    index: u32,
) -> Result<Reader<Feeder<'data>>, PdfError> {
    let malformed = || objects.malformed();
    let (first, count) = counts(objects, dictionary)?;
    if u64::from(index) >= count {
        return Err(malformed());
    }
    let mut reader = Reader::new(Feeder::new(data), 0, objects.operation());
    let mut offset = 0;
    for position in 0..=index {
        let (Token::Integer(found), Token::Integer(at)) = (reader.token()?, reader.token()?) else {
            return Err(malformed());
        };
        if position == index {
            if u32::try_from(found) != Ok(number) {
                return Err(malformed());
            }
            offset = u64::try_from(at).ok().ok_or_else(malformed)?;
        }
    }
    reader.seek(first.checked_add(offset).ok_or_else(malformed)?);
    Ok(reader)
}
