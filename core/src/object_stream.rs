//! Object streams (ISO 32000-2:2020, 7.5.7).

use crate::pull::Pull;
use crate::stream::Feeder;
use crate::syntax::{Dictionary, Object, Reader, Token};
use crate::{ErrorCode, PdfError};

/// The object `number` at `index` in the object stream `dictionary` whose decoded data is `data`;
/// reads all of `data`, so a fault anywhere in it fails; objects nest at most `depth_limit` deep.
pub(crate) fn find(
    data: &mut dyn Pull,
    dictionary: &Dictionary,
    number: u32,
    index: u32,
    depth_limit: u32,
    operation: &'static str,
) -> Result<Object, PdfError> {
    let malformed = || PdfError::new(ErrorCode::Malformed, operation);
    let non_negative = |key: &[u8]| {
        dictionary
            .get(key)
            .and_then(Object::integer)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(malformed)
    };
    let first = non_negative(b"First")?;
    if u64::from(index) >= non_negative(b"N")? {
        return Err(malformed());
    }
    let mut feeder = Feeder::new(data);
    let mut reader = Reader::new(&mut feeder, 0, operation);
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
    let object = reader.object(depth_limit)?;
    feeder.finish()?;
    Ok(object)
}
