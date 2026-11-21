//! Peak allocation of a page count when the catalog holds a huge token or container, or a
//! stream is huge.

extern crate alloc;

use alloc::sync::Arc;
use std::io;

use starlightpdf::ErrorCode;
use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{ReadAt, ReadAtSourceOptions, Source};
use starlightpdf_memory::{adler_run, adler_value, peak, stored_lengths};

/// Most bytes a page count may hold allocated at once.
const BOUND: usize = 3_072;

/// Most bytes a page count may hold allocated at once with a stream.
const STREAM_BOUND: usize = 45_528;

/// A valid header (ISO 32000-2:2020, 7.5.2).
const HEADER: &[u8] = b"%PDF-2.0\n";

/// Bytes per cross-reference entry, `W [1 4 1]` (ISO 32000-2:2020, 7.5.8.2).
const ENTRY: u64 = 6;

/// A digit (ISO 32000-2:2020, 7.3.3).
const DIGIT: Fill = |_| Some(b'1');

/// Bytes of a huge token.
const HUGE: u64 = 100_000_000;

/// A key and its value; [`key`] numbers the zeros (ISO 32000-2:2020, 7.3.7).
const KEY: &[u8] = b"/K0000000()";

/// A letter, regular and a hexadecimal digit (ISO 32000-2:2020, 7.2.3 and 7.3.4.3).
const LETTER: Fill = |_| Some(b'a');

/// Matches of 258 bytes in the 100 MB object: 99,999,768 bytes.
const OBJECT_RUNS: u64 = 387_596;

/// Bytes of a longest match (RFC 1951, 3.2.5).
const RUN: u64 = 258;

/// Elements or keys of a huge container.
const MANY: u64 = 10_000_000;

/// The byte of a filler at an offset into it; `None` on overflow.
type Fill = fn(u64) -> Option<u8>;

/// How a stream's data is stored (ISO 32000-2:2020, 7.3.8).
#[derive(Clone, Copy)]
enum Coding {
    /// `FlateDecode`: stored blocks around one dynamic block of matches.
    Flate,
    /// No filter.
    Stored,
}

/// A stream's data, `prefix`, `runs` times 258 copies of `filler`, `suffix`.
struct Plain {
    /// The repeated byte; the last of `prefix`.
    filler: u8,
    /// The bytes before the repeats.
    prefix: Vec<u8>,
    /// Repeats of 258 bytes.
    runs: u64,
    /// The bytes after the repeats.
    suffix: Vec<u8>,
}

/// A stream's data as stored in the file.
struct Encoded {
    /// The body's byte at an offset into it.
    body: Box<dyn Fn(u64) -> Option<u8> + Send + Sync>,
    /// Bytes of the body.
    body_length: u64,
    /// The bytes before the body.
    head: Vec<u8>,
    /// The bytes after the body.
    tail: Vec<u8>,
}

/// The PDF of [`generated`].
struct Generated {
    /// The filler.
    fill: Box<dyn Fn(u64) -> Option<u8> + Send + Sync>,
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

impl Plain {
    /// Adler-32 of the data, computed byte by byte, runs by their sums (RFC 1950, 8.2).
    fn adler(&self) -> Option<u32> {
        let mut state = (1_u64, 0_u64);
        for &byte in &self.prefix {
            state = adler_run(state, byte, 1)?;
        }
        state = adler_run(state, self.filler, self.runs.checked_mul(RUN)?)?;
        for &byte in &self.suffix {
            state = adler_run(state, byte, 1)?;
        }
        adler_value(state)
    }
}

impl Encoded {
    /// Bytes of the stored data.
    fn length(&self) -> Option<u64> {
        u64::try_from(self.head.len())
            .ok()?
            .checked_add(self.body_length)?
            .checked_add(u64::try_from(self.tail.len()).ok()?)
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
        fill: Box::new(fill),
        head,
        length,
        size,
        tail,
    })
}

/// Asserts the page count of `generated` and that its peak allocation is within `bound`.
fn assert_page_count(generated: Option<Generated>, expected: Result<u32, ErrorCode>, bound: usize) {
    assert_page_count_with(generated, &OpenOptions::default(), expected, bound);
}

/// [`assert_page_count`], opening with `options`.
fn assert_page_count_with(
    generated: Option<Generated>,
    options: &OpenOptions,
    expected: Result<u32, ErrorCode>,
    bound: usize,
) {
    let measured = generated.map(|source| {
        let read_at: Arc<dyn ReadAt> = Arc::new(source);
        peak(|| {
            Source::from_read_at(&read_at, &ReadAtSourceOptions::default())
                .and_then(|opened| Document::open(&opened, None, options))
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
    assert!((1..=bound).contains(&top), "peak {top} bytes");
}

/// ISO 32000-2:2020, 7.2.4.
#[test]
fn comment() {
    assert_page_count(generated("%", LETTER, HUGE, "\n"), Ok(1), BOUND);
}

/// ISO 32000-2:2020, 7.3.4.2.
#[test]
fn literal_string() {
    assert_page_count(generated("/Big (", LETTER, HUGE, ")"), Ok(1), BOUND);
}

/// ISO 32000-2:2020, 7.3.4.3.
#[test]
fn hexadecimal_string() {
    assert_page_count(generated("/Big <", LETTER, HUGE, ">"), Ok(1), BOUND);
}

/// ISO 32000-2:2020, 7.3.5.
#[test]
fn name() {
    assert_page_count(generated("/Big /", LETTER, HUGE, ""), Ok(1), BOUND);
}

/// A huge key after the key looked up is not read whole; `Pages` repeats, the last counting
/// (ISO 32000-2:2020, 7.3.5 and 7.3.7).
#[test]
fn key_after_the_key_looked_up() {
    assert_page_count(
        generated("/Pages 2 0 R /", LETTER, HUGE, " 1"),
        Ok(1),
        BOUND,
    );
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
        BOUND,
    );
}

/// ISO 32000-2:2020, 7.3.6.
#[test]
fn ten_million_array_elements() {
    assert_page_count(
        MANY.checked_mul(2)
            .and_then(|bytes| generated("/Big [", element, bytes, "]")),
        Ok(1),
        BOUND,
    );
}

/// ISO 32000-2:2020, 7.3.3; strict, as a tolerant open rebuilds the table.
#[test]
fn number_of_digits_fails_with_malformed() {
    let mut strict = OpenOptions::default();
    strict.strict = true;
    assert_page_count_with(
        generated("/Big ", DIGIT, HUGE, ""),
        &strict,
        Err(ErrorCode::Malformed),
        BOUND,
    );
}

/// A cross-reference stream of `entries` entries whose last five (the page, the catalog, the page
/// tree root, the object stream and itself) are in use, after an object stream of the three
/// (ISO 32000-2:2020, 7.5.7, 7.5.8); `parms` goes in its dictionary.
fn xref_file(entries: u64, coding: Coding, parms: &str) -> Option<Generated> {
    let page = entries.checked_sub(5)?;
    let catalog = entries.checked_sub(4)?;
    let pages = entries.checked_sub(3)?;
    let objects = entries.checked_sub(2)?;
    let xref = entries.checked_sub(1)?;
    let leaf_text =
        format!("<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 612 792] /Resources << >> >>\n");
    let catalog_text = format!("<< /Type /Catalog /Pages {pages} 0 R >>\n");
    let tree_text = format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>\n");
    let header = object_header(&[
        (catalog, length_of(&catalog_text)?),
        (pages, length_of(&tree_text)?),
        (page, length_of(&leaf_text)?),
    ])?;
    let data = format!("{header}{catalog_text}{tree_text}{leaf_text}");
    let stream = format!(
        "{objects} 0 obj\n<< /Type /ObjStm /N 3 /First {} /Length {} >>\nstream\n{data}\nendstream\nendobj\n",
        header.len(),
        data.len()
    );
    let objects_at = u64::try_from(HEADER.len()).ok()?;
    let xref_at = objects_at.checked_add(u64::try_from(stream.len()).ok()?)?;
    let used = |kind: u8, field: u64, index: u64| -> Option<Vec<u8>> {
        let mut bytes = vec![kind];
        bytes.extend(be(field, 4)?);
        bytes.extend(be(index, 1)?);
        Some(bytes)
    };
    let mut last = used(2, objects, 2)?;
    last.extend(used(2, objects, 0)?);
    last.extend(used(2, objects, 1)?);
    last.extend(used(1, objects_at, 0)?);
    last.extend(used(1, xref_at, 0)?);
    let free = entries.checked_sub(5)?.checked_mul(ENTRY)?.checked_sub(1)?;
    let runs = free.checked_div(RUN)?;
    let mut suffix = vec![0; usize::try_from(free.checked_rem(RUN)?).ok()?];
    suffix.extend(last);
    let plain = Plain {
        filler: 0,
        prefix: vec![0],
        runs,
        suffix,
    };
    let encoded = encode(&plain, coding)?;
    let length = encoded.length()?;
    let filter = filter(coding);
    let mut head = HEADER.to_vec();
    head.extend(stream.into_bytes());
    head.extend(
        format!(
            "{xref} 0 obj\n<< /Type /XRef /Size {entries} /W [1 4 1] /Root {catalog} 0 R {parms}/Length {length} {filter}>>\nstream\n"
        )
        .into_bytes(),
    );
    let post = format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n");
    assemble(head, encoded, post.into_bytes())
}

/// A cross-reference stream of seven entries after an object stream whose first object is a string
/// of about 100 MB, then the catalog, the page tree root and the page (ISO 32000-2:2020, 7.5.7).
fn object_file(runs: u64, coding: Coding) -> Option<Generated> {
    let catalog_text = "<< /Type /Catalog /Pages 4 0 R >>\n";
    let tree_text = "<< /Type /Pages /Kids [5 0 R] /Count 1 >>\n";
    let leaf_text = "<< /Type /Page /Parent 4 0 R /MediaBox [0 0 612 792] /Resources << >> >>\n";
    let huge = runs.checked_mul(RUN)?.checked_add(4)?;
    let header = object_header(&[
        (6, huge),
        (3, length_of(catalog_text)?),
        (4, length_of(tree_text)?),
        (5, length_of(leaf_text)?),
    ])?;
    let mut prefix = header.clone().into_bytes();
    prefix.extend(b"(a");
    let mut suffix = b")\n".to_vec();
    suffix.extend(format!("{catalog_text}{tree_text}{leaf_text}").into_bytes());
    let plain = Plain {
        filler: b'a',
        prefix,
        runs,
        suffix,
    };
    let encoded = encode(&plain, coding)?;
    let length = encoded.length()?;
    let filter = filter(coding);
    let mut head = HEADER.to_vec();
    head.extend(
        format!(
            "1 0 obj\n<< /Type /ObjStm /N 4 /First {} /Length {length} {filter}>>\nstream\n",
            header.len()
        )
        .into_bytes(),
    );
    let end = "\nendstream\nendobj\n";
    let xref_at = u64::try_from(head.len())
        .ok()?
        .checked_add(length)?
        .checked_add(length_of(end)?)?;
    let mut entries = Vec::new();
    for (kind, field, index) in [
        (0, 0, 0),
        (1, u64::try_from(HEADER.len()).ok()?, 0),
        (1, xref_at, 0),
        (2, 1, 1),
        (2, 1, 2),
        (2, 1, 3),
        (2, 1, 0),
    ] {
        entries.push(kind);
        entries.extend(be(field, 4)?);
        entries.extend(be(index, 1)?);
    }
    let mut post = end.as_bytes().to_vec();
    post.extend(
        format!(
            "2 0 obj\n<< /Type /XRef /Size 7 /W [1 4 1] /Root 3 0 R /Length {} >>\nstream\n",
            entries.len()
        )
        .into_bytes(),
    );
    post.extend(entries);
    post.extend(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").into_bytes());
    assemble(head, encoded, post)
}

/// The PDF of `head`, the data and `post`.
fn assemble(mut head: Vec<u8>, encoded: Encoded, post: Vec<u8>) -> Option<Generated> {
    head.extend(encoded.head);
    let mut tail = encoded.tail;
    tail.extend(post);
    let size = u64::try_from(head.len())
        .ok()?
        .checked_add(encoded.body_length)?
        .checked_add(u64::try_from(tail.len()).ok()?)?;
    Some(Generated {
        fill: encoded.body,
        head,
        length: encoded.body_length,
        size,
        tail,
    })
}

/// `value` as `width` big-endian bytes (ISO 32000-2:2020, 7.5.8.3).
fn be(value: u64, width: u32) -> Option<Vec<u8>> {
    (0..width)
        .rev()
        .map(|index| {
            let shift = index.checked_mul(8)?;
            u8::try_from(value.checked_shr(shift)?.checked_rem(256)?).ok()
        })
        .collect()
}

/// The bit `index` of the dynamic block: `header`, `runs` matches of length 258 at distance 1, end
/// of block, then the header bits of the final stored block and padding (RFC 1951, 3.2.3, 3.2.4).
fn block_bit(header: &[bool], runs: u64, index: u64) -> Option<bool> {
    let start = u64::try_from(header.len()).ok()?;
    let matches_end = start.checked_add(runs.checked_mul(2)?)?;
    Some(if index < start {
        *header.get(usize::try_from(index).ok()?)?
    } else if index < matches_end {
        index.checked_sub(start)?.checked_rem(2)? == 0
    } else {
        index == matches_end.checked_add(1)?
    })
}

/// The header of a dynamic block (RFC 1951, 3.2.7) whose literal/length code is 256 and 285, one
/// bit each, and whose distance code is 0 and 1, one bit each.
fn dynamic_header() -> Vec<bool> {
    let mut bits = Vec::new();
    push(&mut bits, 0, 1);
    push(&mut bits, 2, 2);
    push(&mut bits, 29, 5);
    push(&mut bits, 1, 5);
    push(&mut bits, 14, 4);
    // Lengths of the code length codes 16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14
    // and 1: symbols 18 and 1 have length 1, so 1 is `0` and 18 is `1`.
    for length in [0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1] {
        push(&mut bits, length, 3);
    }
    // Code lengths (zero runs are 11 plus the extra bits): 256 zeros, 1 for 256, 28 zeros, 1 for 285, then 1 and 1 for both distances.
    for extra in [127, 107] {
        bits.push(true);
        push(&mut bits, extra, 7);
    }
    bits.push(false);
    bits.push(true);
    push(&mut bits, 17, 7);
    bits.extend([false; 3]);
    bits
}

/// The stored form of `plain` (ISO 32000-2:2020, 7.3.8; RFC 1950, 2.2; RFC 1951, 3.2.4, 3.2.7).
fn encode(plain: &Plain, coding: Coding) -> Option<Encoded> {
    let run_bytes = plain.runs.checked_mul(RUN)?;
    let filler = plain.filler;
    if matches!(coding, Coding::Stored) {
        return Some(Encoded {
            head: plain.prefix.clone(),
            body: Box::new(move |_| Some(filler)),
            body_length: run_bytes,
            tail: plain.suffix.clone(),
        });
    }
    let header = dynamic_header();
    let bits = u64::try_from(header.len())
        .ok()?
        .checked_add(plain.runs.checked_mul(2)?)?
        .checked_add(4)?;
    let mut head = vec![0x78, 0x9c, 0x00];
    head.extend(stored_block(&plain.prefix)?);
    head.extend(&plain.prefix);
    let mut tail = stored_block(&plain.suffix)?.to_vec();
    tail.extend(&plain.suffix);
    let adler = plain.adler()?;
    for byte in [24, 16, 8, 0] {
        let shift = adler.checked_shr(byte)?;
        tail.push(u8::try_from(shift.checked_rem(256)?).ok()?);
    }
    let runs = plain.runs;
    Some(Encoded {
        head,
        body: Box::new(move |offset| {
            let mut byte = 0_u8;
            for bit in 0..8_u32 {
                let at = offset.checked_mul(8)?.checked_add(u64::from(bit))?;
                if block_bit(&header, runs, at)? {
                    byte |= 1_u8.checked_shl(bit)?;
                }
            }
            Some(byte)
        }),
        body_length: bits.checked_add(7)?.checked_div(8)?,
        tail,
    })
}

/// The dictionary entry for `coding`.
const fn filter(coding: Coding) -> &'static str {
    match coding {
        Coding::Flate => "/Filter /FlateDecode ",
        Coding::Stored => "",
    }
}

/// Bytes of `text`.
fn length_of(text: &str) -> Option<u64> {
    u64::try_from(text.len()).ok()
}

/// `LEN` and `NLEN` of a stored block of `data` (RFC 1951, 3.2.4), after the block's header.
fn stored_block(data: &[u8]) -> Option<[u8; 4]> {
    stored_lengths(u16::try_from(data.len()).ok()?)
}

/// The `N` pairs of object number and offset in an object stream's header, each object's length
/// given (ISO 32000-2:2020, 7.5.7).
fn object_header(objects: &[(u64, u64)]) -> Option<String> {
    let mut offset = 0_u64;
    let mut pairs = Vec::new();
    for &(number, length) in objects {
        pairs.push(format!("{number} {offset}"));
        offset = offset.checked_add(length)?;
    }
    Some(format!("{} ", pairs.join(" ")))
}

/// Appends the low `count` bits of `value`, least significant first (RFC 1951, 3.1.1).
fn push(bits: &mut Vec<bool>, value: u32, count: u32) {
    for index in 0..count {
        bits.push(
            value
                .checked_shr(index)
                .is_some_and(|rest| rest.checked_rem(2) == Some(1)),
        );
    }
}

/// ISO 32000-2:2020, 7.5.8.3.
#[test]
fn ten_million_xref_stream_entries_stored() {
    assert_page_count(xref_file(MANY, Coding::Stored, ""), Ok(1), STREAM_BOUND);
}

/// ISO 32000-2:2020, 7.5.8.3; RFC 1951.
#[test]
fn ten_million_xref_stream_entries_compressed() {
    assert_page_count(xref_file(MANY, Coding::Flate, ""), Ok(1), STREAM_BOUND);
}

/// ISO 32000-2:2020, 7.5.7.
#[test]
fn hundred_mb_object_before_catalog_stored() {
    assert_page_count(
        object_file(OBJECT_RUNS, Coding::Stored),
        Ok(1),
        STREAM_BOUND,
    );
}

/// ISO 32000-2:2020, 7.5.7; RFC 1951.
#[test]
fn hundred_mb_object_before_catalog_compressed() {
    assert_page_count(object_file(OBJECT_RUNS, Coding::Flate), Ok(1), STREAM_BOUND);
}

/// ISO 32000-2:2020, 7.4.4.4.
#[test]
fn predictor_columns_of_a_billion_fails_with_limit_exceeded() {
    assert_page_count(
        xref_file(
            10,
            Coding::Flate,
            "/DecodeParms << /Predictor 12 /Columns 1000000000 >> ",
        ),
        Err(ErrorCode::LimitExceeded),
        STREAM_BOUND,
    );
}
