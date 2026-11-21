//! The document information dictionary and XMP metadata.

use core::ops::Range;

use crate::decryption::{PDF_DOC, pdf_doc};

/// The document information dictionary (ISO 32000-2:2020, 14.3.3).
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct InfoDictionary {
    /// The author.
    pub author: String,
    /// The creation [date](crate#dates).
    pub created: String,
    /// The application that created the original.
    pub creator: String,
    /// Keywords.
    pub keywords: String,
    /// The last modification [date](crate#dates).
    pub modified: String,
    /// The application that wrote the PDF.
    pub producer: String,
    /// The subject.
    pub subject: String,
    /// The title.
    pub title: String,
}

/// Options for [`Document::set_info_dictionary`](crate::document::Document::set_info_dictionary); `None` leaves an
/// entry unchanged, empty removes it.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetInfoDictionaryOptions {
    /// The author.
    pub author: Option<String>,
    /// The creation [date](crate#dates).
    pub created: Option<String>,
    /// The application that created the original.
    pub creator: Option<String>,
    /// Keywords.
    pub keywords: Option<String>,
    /// The last modification [date](crate#dates).
    pub modified: Option<String>,
    /// The application that wrote the PDF.
    pub producer: Option<String>,
    /// The subject.
    pub subject: Option<String>,
    /// The title.
    pub title: Option<String>,
}

/// Options for [`Document::set_xmp`](crate::document::Document::set_xmp).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetXmpOptions {
    /// The XMP packet; empty removes it.
    pub xmp: String,
}

/// The ISO 8601 date of PDF date `text`, omitted fields at their defaults; `None` when it is not
/// one (ISO 32000-2:2020, 7.9.4).
pub(crate) fn iso_date(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let rest = bytes.strip_prefix(b"D:").unwrap_or(bytes);
    let year = digits(rest.get(..4)?)?;
    let mut fields = [1, 1, 0, 0, 0];
    let mut at = 4_usize;
    for field in &mut fields {
        let Some(found) = rest.get(at..at.checked_add(2)?).and_then(digits) else {
            break;
        };
        *field = found;
        at = at.checked_add(2)?;
    }
    let zone = match *rest.get(at..)? {
        [] => String::new(),
        [b'Z', ..] => String::from("Z"),
        [sign @ (b'+' | b'-'), ..] => {
            let offset = rest.get(at.checked_add(1)?..)?;
            let hours = digits(offset.get(..2)?)?;
            let minutes = match *offset.get(2..)? {
                [] | [b'\''] => 0,
                [b'\'', _, _] | [b'\'', _, _, b'\''] => digits(offset.get(3..5)?)?,
                _ => return None,
            };
            if hours > 23 || minutes > 59 {
                return None;
            }
            format!("{}{hours:02}:{minutes:02}", char::from(sign))
        }
        _ => return None,
    };
    let [month, day, hour, minute, second] = fields;
    valid([year, month, day], hour, minute, second)
        .then(|| format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{zone}"))
}

/// The PDF date of ISO 8601 `text`, `YYYY-MM-DDTHH:MM:SS` then `Z`, `±HH:MM` or nothing; `None`
/// when it is not one (ISO 32000-2:2020, 7.9.4).
pub(crate) fn pdf_date(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let field = |range: Range<usize>| bytes.get(range).and_then(digits);
    let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if separators
        .iter()
        .any(|&(at, separator)| bytes.get(at) != Some(&separator))
    {
        return None;
    }
    let (year, month, day) = (field(0..4)?, field(5..7)?, field(8..10)?);
    let (hour, minute, second) = (field(11..13)?, field(14..16)?, field(17..19)?);
    let zone = match *bytes.get(19..)? {
        [] => String::new(),
        [b'Z'] => String::from("Z"),
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let (hours, minutes) = (field(20..22)?, field(23..25)?);
            if hours > 23 || minutes > 59 {
                return None;
            }
            format!("{}{hours:02}'{minutes:02}", char::from(sign))
        }
        _ => return None,
    };
    valid([year, month, day], hour, minute, second)
        .then(|| format!("D:{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}{zone}"))
}

/// Text string `bytes` as text: UTF-16BE or UTF-8 after their byte order marks, else
/// `PDFDocEncoding`; a code with no character becomes U+FFFD (ISO 32000-2:2020, 7.9.2.2, Annex D).
pub(crate) fn text(bytes: &[u8]) -> String {
    if let Some(utf16) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units = utf16
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[high, low]| u16::from(high).wrapping_shl(8) | u16::from(low));
        return char::decode_utf16(units)
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
    }
    if let Some(utf8) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(utf8).into_owned();
    }
    bytes
        .iter()
        .map(|&byte| match byte {
            0x09 | 0x0A | 0x0D | 0x20..=0x7E | 0xA1..=0xAC | 0xAE..=0xFF => char::from(byte),
            _ => PDF_DOC
                .iter()
                .find(|entry| entry.1 == byte)
                .map_or(char::REPLACEMENT_CHARACTER, |entry| entry.0),
        })
        .collect()
}

/// `text` as a text string: `PDFDocEncoding` when it holds every character and does not begin as
/// a byte order mark would, else UTF-16BE with one (ISO 32000-2:2020, 7.9.2.2).
pub(crate) fn text_string(text: &str) -> Vec<u8> {
    if let Some(bytes) = pdf_doc(text).filter(|bytes| {
        !bytes.starts_with(&[0xFE, 0xFF]) && !bytes.starts_with(&[0xEF, 0xBB, 0xBF])
    }) {
        return bytes;
    }
    let mut bytes = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        bytes.extend(
            [unit.wrapping_shr(8), unit & 0xFF].map(|half| u8::try_from(half).unwrap_or_default()),
        );
    }
    bytes
}

/// The number ASCII digits `bytes` spell; `None` for any other byte.
fn digits(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0_u32, |value, &byte| {
        byte.is_ascii_digit()
            .then(|| {
                value
                    .checked_mul(10)?
                    .checked_add(u32::from(byte.wrapping_sub(b'0')))
            })
            .flatten()
    })
}

/// Whether the fields name a time of day on a day of the Gregorian calendar (ISO 8601).
const fn valid([year, month, day]: [u32; 3], hour: u32, minute: u32, second: u32) -> bool {
    let leap = year.is_multiple_of(4) && !year.is_multiple_of(100) || year.is_multiple_of(400);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    matches!(month, 1..=12) && day >= 1 && day <= days && hour < 24 && minute < 60 && second < 60
}
