//! Peak allocation of a page count when the catalog holds a huge token or container.

extern crate alloc;

use alloc::sync::Arc;
use std::io;

use starlightpdf::ErrorCode;
use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{ReadAt, ReadAtSourceOptions, Source};
use starlightpdf_memory::peak;

/// Most bytes a page count may hold allocated at once.
const BOUND: usize = 3_072;

/// A digit (ISO 32000-2:2020, 7.3.3).
const DIGIT: Fill = |_| Some(b'1');

/// Bytes of a huge token.
const HUGE: u64 = 100_000_000;

/// A key and its value; [`key`] numbers the zeros (ISO 32000-2:2020, 7.3.7).
const KEY: &[u8] = b"/K0000000()";

/// A letter, regular and a hexadecimal digit (ISO 32000-2:2020, 7.2.3 and 7.3.4.3).
const LETTER: Fill = |_| Some(b'a');

/// Elements or keys of a huge container.
const MANY: u64 = 10_000_000;

/// The byte of a filler at an offset into it; `None` on overflow.
type Fill = fn(u64) -> Option<u8>;

/// The PDF of [`generated`].
struct Generated {
    /// The filler.
    fill: Fill,
    /// Bytes before the filler.
    head: Vec<u8>,
    /// Bytes of the filler.
    length: u64,
    /// Bytes in all.
    size: u64,
    /// Bytes after the filler.
    tail: Vec<u8>,
}

impl Generated {
    /// The byte at `offset`, `None` past the end.
    fn byte(&self, offset: u64) -> Option<u8> {
        let head = u64::try_from(self.head.len()).ok()?;
        let Some(in_fill) = offset.checked_sub(head) else {
            return self.head.get(usize::try_from(offset).ok()?).copied();
        };
        match in_fill.checked_sub(self.length) {
            Some(in_tail) => self.tail.get(usize::try_from(in_tail).ok()?).copied(),
            None => (self.fill)(in_fill),
        }
    }
}

impl ReadAt for Generated {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let mut count: usize = 0;
        for (to, at) in buffer.iter_mut().zip(offset..self.size) {
            *to = self.byte(at).ok_or_else(overflow)?;
            count = count.checked_add(1).ok_or_else(overflow)?;
        }
        Ok(count)
    }

    fn size(&self) -> io::Result<u64> {
        Ok(self.size)
    }
}

/// Copies of [`KEY`] numbered from zero.
fn key(offset: u64) -> Option<u8> {
    let width = u64::try_from(KEY.len()).ok()?;
    let at = usize::try_from(offset.checked_rem(width)?).ok()?;
    let byte = *KEY.get(at)?;
    if byte != b'0' {
        return Some(byte);
    }
    let lower = KEY
        .iter()
        .rposition(|&zero| zero == b'0')?
        .checked_sub(at)?;
    let digit = offset
        .checked_div(width)?
        .checked_div(10_u64.checked_pow(u32::try_from(lower).ok()?)?)?
        .checked_rem(10)?;
    byte.checked_add(u8::try_from(digit).ok()?)
}

/// Empty string elements, `()()()` (ISO 32000-2:2020, 7.3.6).
fn element(offset: u64) -> Option<u8> {
    b"()"
        .get(usize::try_from(offset.checked_rem(2)?).ok()?)
        .copied()
}

/// The failure of an offset computation.
fn overflow() -> io::Error {
    io::Error::other("offset overflow")
}

/// A one-page PDF whose catalog holds `open`, `length` bytes of `fill` and `close`
/// (ISO 32000-2:2020, 7.5).
fn generated(open: &str, fill: Fill, length: u64, close: &str) -> Option<Generated> {
    let head = format!("%PDF-2.0\n1 0 obj\n<< /Type /Catalog {open}").into_bytes();
    let catalog_end = format!("{close} /Pages 2 0 R >>\nendobj\n");
    let pages = "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
    let leaf = "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> >>\nendobj\n";
    let pages_at = u64::try_from(head.len().checked_add(catalog_end.len())?)
        .ok()?
        .checked_add(length)?;
    let leaf_at = pages_at.checked_add(u64::try_from(pages.len()).ok()?)?;
    let xref_at = leaf_at.checked_add(u64::try_from(leaf.len()).ok()?)?;
    let tail = format!(
        "{catalog_end}{pages}{leaf}xref\n0 4\n0000000000 65535 f\r\n0000000009 00000 n\r\n\
         {pages_at:010} 00000 n\r\n{leaf_at:010} 00000 n\r\n\
         trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n"
    )
    .into_bytes();
    let size = u64::try_from(head.len().checked_add(tail.len())?)
        .ok()?
        .checked_add(length)?;
    Some(Generated {
        fill,
        head,
        length,
        size,
        tail,
    })
}

/// Asserts the page count of `generated` and that its peak allocation is within [`BOUND`].
fn assert_page_count(generated: Option<Generated>, expected: Result<u32, ErrorCode>) {
    let measured = generated.map(|source| {
        let read_at: Arc<dyn ReadAt> = Arc::new(source);
        peak(|| {
            Source::from_read_at(&read_at, &ReadAtSourceOptions::default())
                .and_then(|opened| Document::open(&opened, None, &OpenOptions::default()))
                .and_then(|document| document.page_count())
                .map_err(|error| error.code())
        })
    });
    assert_eq!(
        measured.as_ref().map(|&(counted, _)| counted),
        Some(expected),
        "page count"
    );
    let top = measured.map_or(0, |(_, bytes)| bytes);
    assert!((1..=BOUND).contains(&top), "peak {top} bytes");
}

/// ISO 32000-2:2020, 7.2.4.
#[test]
fn comment() {
    assert_page_count(generated("%", LETTER, HUGE, "\n"), Ok(1));
}

/// ISO 32000-2:2020, 7.3.4.2.
#[test]
fn literal_string() {
    assert_page_count(generated("/Big (", LETTER, HUGE, ")"), Ok(1));
}

/// ISO 32000-2:2020, 7.3.4.3.
#[test]
fn hexadecimal_string() {
    assert_page_count(generated("/Big <", LETTER, HUGE, ">"), Ok(1));
}

/// ISO 32000-2:2020, 7.3.5.
#[test]
fn name() {
    assert_page_count(generated("/Big /", LETTER, HUGE, ""), Ok(1));
}

/// ISO 32000-2:2020, 7.3.7.
#[test]
fn ten_million_keys() {
    let length = u64::try_from(KEY.len())
        .ok()
        .and_then(|width| MANY.checked_mul(width));
    assert_page_count(
        length.and_then(|bytes| generated("", key, bytes, "")),
        Ok(1),
    );
}

/// ISO 32000-2:2020, 7.3.6.
#[test]
fn ten_million_array_elements() {
    assert_page_count(
        MANY.checked_mul(2)
            .and_then(|bytes| generated("/Big [", element, bytes, "]")),
        Ok(1),
    );
}

/// ISO 32000-2:2020, 7.3.3.
#[test]
fn number_of_digits_fails_with_limit_exceeded() {
    assert_page_count(
        generated("/Big ", DIGIT, HUGE, ""),
        Err(ErrorCode::LimitExceeded),
    );
}
