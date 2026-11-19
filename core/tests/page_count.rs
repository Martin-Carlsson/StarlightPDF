//! Counting pages through a caller's source.

extern crate alloc;

use alloc::sync::Arc;
use core::ops::Range;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::thread;

use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{FileSourceOptions, ReadAt, ReadAtSourceOptions, Source};
use starlightpdf::{ErrorCode, PdfError};

/// The catalog's object number, mid-file; the page tree root and pages follow it.
const CATALOG: u64 = 500_000;

/// Bytes per cross-reference entry (ISO 32000-2:2020, 7.5.4).
const ENTRY: u64 = 20;

/// Bytes of the file of [`Streams`], 1 GiB.
const GIB: u64 = 0x4000_0000;

/// A valid header (ISO 32000-2:2020, 7.5.2).
const HEADER: &[u8] = b"%PDF-2.0\n";

/// Three pages.
const NESTED_TREE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../tests/corpus/page-count/nested-tree.pdf"
);

/// Objects, numbered from 1.
const OBJECTS: u64 = 1_000_000;

/// Page objects, the root's only kids.
const PAGES: u64 = 3;

/// Bytes of the object stream's data (ISO 32000-2:2020, 7.5.7).
const OBJECT_STREAM_DATA: usize = 4_096;

/// Bytes per object; makes the file just over 1 GiB.
const RECORD: u64 = 1054;

/// Entries in the cross-reference table, object 0 included.
const SIZE: u64 = OBJECTS + 1;

/// Entries in the cross-reference stream: 683 of 6 bytes are about 4 KiB (ISO 32000-2:2020,
/// 7.5.8.3).
const STREAM_ENTRIES: u64 = 683;

/// An error's code and I/O cause.
type Failure = (ErrorCode, Option<ErrorKind>);

/// A valid PDF of `OBJECTS` objects and one classic cross-reference table, each byte made from
/// its offset; counts the bytes it serves.
struct Generated {
    /// Fails every read with `ErrorKind::TimedOut`.
    failing: AtomicBool,
    /// Bytes served so far.
    served: AtomicUsize,
}

impl ReadAt for Generated {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        if self.failing.load(Ordering::Relaxed) {
            return Err(ErrorKind::TimedOut.into());
        }
        let size = self.size()?;
        let mut position = offset;
        let mut filled = 0;
        while let Some(rest) = buffer.get_mut(filled..)
            && !rest.is_empty()
            && position < size
        {
            let (start, bytes) = segment(position).ok_or_else(overflow)?;
            let skip = position
                .checked_sub(start)
                .and_then(|skip| usize::try_from(skip).ok())
                .ok_or_else(overflow)?;
            let chunk = bytes
                .get(skip..)
                .filter(|chunk| !chunk.is_empty())
                .ok_or_else(overflow)?;
            let count = chunk.len().min(rest.len());
            for (to, from) in rest.iter_mut().zip(chunk) {
                *to = *from;
            }
            filled = filled.checked_add(count).ok_or_else(overflow)?;
            position = u64::try_from(count)
                .ok()
                .and_then(|step| position.checked_add(step))
                .ok_or_else(overflow)?;
        }
        let _previous = self.served.fetch_add(filled, Ordering::Relaxed);
        Ok(filled)
    }

    fn size(&self) -> io::Result<u64> {
        tail_start()
            .zip(xref_offset())
            .and_then(|(start, xref)| start.checked_add(u64::try_from(tail(xref).len()).ok()?))
            .ok_or_else(overflow)
    }
}

/// A PDF of 1 GiB: an object stream of 4 KiB, a stream of `x` and a cross-reference stream of
/// about 4 KiB, at the end; counts the bytes it serves.
struct Streams {
    /// Bytes of the middle stream's data.
    bulk: u64,
    /// The offsets of the object stream's data, where a read fails when `failing`.
    data: Range<u64>,
    /// Fails a read that touches the object stream's data with `ErrorKind::ConnectionReset`.
    failing: bool,
    /// The bytes before the bulk.
    head: Vec<u8>,
    /// Bytes served so far.
    served: AtomicUsize,
    /// The bytes after the bulk.
    tail: Vec<u8>,
}

impl ReadAt for Streams {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let end = u64::try_from(buffer.len())
            .ok()
            .and_then(|length| offset.checked_add(length))
            .ok_or_else(overflow)?;
        if self.failing && offset < self.data.end && end > self.data.start {
            return Err(ErrorKind::ConnectionReset.into());
        }
        let mut filled: usize = 0;
        for (to, at) in buffer.iter_mut().zip(offset..) {
            let Some(byte) = self.byte(at) else { break };
            *to = byte;
            filled = filled.checked_add(1).ok_or_else(overflow)?;
        }
        let _previous = self.served.fetch_add(filled, Ordering::Relaxed);
        Ok(filled)
    }

    fn size(&self) -> io::Result<u64> {
        u64::try_from(self.head.len())
            .ok()
            .zip(u64::try_from(self.tail.len()).ok())
            .and_then(|(head, tail)| head.checked_add(self.bulk)?.checked_add(tail))
            .ok_or_else(overflow)
    }
}

impl Streams {
    /// The head, tail and object stream data offsets of a file with `bulk` bytes of `x`: the
    /// header, the object stream, the bulk stream, then the cross-reference stream
    /// (ISO 32000-2:2020, 7.5.7, 7.5.8).
    fn assemble(bulk: u64) -> Option<(Vec<u8>, Vec<u8>, Range<u64>)> {
        let page = "<< /Type /Page /Parent 4 0 R /MediaBox [0 0 612 792] /Resources << >> >>";
        let texts = [
            "<< /Type /Catalog /Pages 4 0 R >>",
            "<< /Type /Pages /Kids [5 0 R 6 0 R 7 0 R] /Count 3 >>",
            page,
            page,
            page,
        ];
        let mut body = String::new();
        let mut pairs = Vec::new();
        for (number, text) in (3_u64..).zip(texts) {
            pairs.push(format!("{number} {}", body.len()));
            body.push_str(text);
            body.push('\n');
        }
        let header = format!("{} ", pairs.join(" "));
        let mut data = format!("{header}{body}").into_bytes();
        if data.len() > OBJECT_STREAM_DATA {
            return None;
        }
        data.resize(OBJECT_STREAM_DATA, b' ');
        let mut head = HEADER.to_vec();
        head.extend(
            format!(
                "2 0 obj\n<< /Type /ObjStm /N 5 /First {} /Length {OBJECT_STREAM_DATA} >>\nstream\n",
                header.len()
            )
            .into_bytes(),
        );
        let data_start = u64::try_from(head.len()).ok()?;
        head.extend(data);
        head.extend(b"\nendstream\nendobj\n");
        let bulk_object = u64::try_from(head.len()).ok()?;
        head.extend(format!("1 0 obj\n<< /Length {bulk} >>\nstream\n").into_bytes());
        let end = b"\nendstream\nendobj\n";
        let xref_at = u64::try_from(head.len())
            .ok()?
            .checked_add(bulk)?
            .checked_add(u64::try_from(end.len()).ok()?)?;
        let mut entries = Vec::new();
        for number in 0..STREAM_ENTRIES {
            let (kind, field, index) = if number == 1 {
                (1, bulk_object, 0)
            } else if number == 2 {
                (1, u64::try_from(HEADER.len()).ok()?, 0)
            } else if (3..=7).contains(&number) {
                (2, 2, number.checked_sub(3)?)
            } else if number == STREAM_ENTRIES.checked_sub(1)? {
                (1, xref_at, 0)
            } else {
                (0, 0, 0)
            };
            entries.push(kind);
            entries.extend(be(field, 4)?);
            entries.extend(be(index, 1)?);
        }
        let mut tail = end.to_vec();
        tail.extend(
            format!(
                "{} 0 obj\n<< /Type /XRef /Size {STREAM_ENTRIES} /W [1 4 1] /Root 3 0 R /Length {} >>\nstream\n",
                STREAM_ENTRIES.checked_sub(1)?,
                entries.len()
            )
            .into_bytes(),
        );
        tail.extend(entries);
        tail.extend(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").into_bytes());
        let data_end = data_start.checked_add(u64::try_from(OBJECT_STREAM_DATA).ok()?)?;
        Some((head, tail, data_start..data_end))
    }

    /// The byte at `offset`, `None` past the end.
    fn byte(&self, offset: u64) -> Option<u8> {
        let head = u64::try_from(self.head.len()).ok()?;
        let Some(in_bulk) = offset.checked_sub(head) else {
            return self.head.get(usize::try_from(offset).ok()?).copied();
        };
        match in_bulk.checked_sub(self.bulk) {
            Some(in_tail) => self.tail.get(usize::try_from(in_tail).ok()?).copied(),
            None => Some(b'x'),
        }
    }

    /// The file; fails reads of the object stream's data when `failing`.
    fn new(failing: bool) -> Option<Self> {
        let (first_head, first_tail, _) = Self::assemble(GIB)?;
        let used = u64::try_from(first_head.len().checked_add(first_tail.len())?).ok()?;
        let bulk = GIB.checked_sub(used)?;
        let (head, tail, data) = Self::assemble(bulk)?;
        Some(Self {
            bulk,
            data,
            failing,
            head,
            served: AtomicUsize::new(0),
            tail,
        })
    }
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

/// Object `number`, `RECORD` bytes: the catalog, the page tree root, its pages or `null`,
/// padded with a comment.
fn object(number: u64) -> Option<Vec<u8>> {
    let root = CATALOG.checked_add(1)?;
    let first = root.checked_add(1)?;
    let content = if number == CATALOG {
        format!("<< /Type /Catalog /Pages {root} 0 R >>")
    } else if number == root {
        let kids: Vec<String> = (first..first.checked_add(PAGES)?)
            .map(|page| format!("{page} 0 R"))
            .collect();
        format!(
            "<< /Type /Pages /Kids [{}] /Count {PAGES} >>",
            kids.join(" ")
        )
    } else if (first..first.checked_add(PAGES)?).contains(&number) {
        format!("<< /Type /Page /Parent {root} 0 R /MediaBox [0 0 612 792] /Resources << >> >>")
    } else {
        String::from("null")
    };
    let head = format!("{number} 0 obj\n{content}\n%");
    let end = "\nendobj\n";
    let padding = usize::try_from(RECORD)
        .ok()?
        .checked_sub(head.len())?
        .checked_sub(end.len())?;
    Some(format!("{head}{}{end}", "x".repeat(padding)).into_bytes())
}

/// The failure of an offset computation.
fn overflow() -> io::Error {
    io::Error::other("offset overflow")
}

/// Entry `number` of the cross-reference table (ISO 32000-2:2020, 7.5.4).
fn entry(number: u64) -> Option<Vec<u8>> {
    if number == 0 {
        return Some(b"0000000000 65535 f\r\n".to_vec());
    }
    let offset = RECORD
        .checked_mul(number.checked_sub(1)?)?
        .checked_add(u64::try_from(HEADER.len()).ok()?)?;
    Some(format!("{offset:010} 00000 n\r\n").into_bytes())
}

/// The `xref` line and subsection header.
fn xref_head() -> Vec<u8> {
    format!("xref\n0 {SIZE}\n").into_bytes()
}

/// The offset of `xref`, after the header and objects.
fn xref_offset() -> Option<u64> {
    OBJECTS
        .checked_mul(RECORD)?
        .checked_add(u64::try_from(HEADER.len()).ok()?)
}

/// The offset of the cross-reference entries.
fn entries_start() -> Option<u64> {
    xref_offset()?.checked_add(u64::try_from(xref_head().len()).ok()?)
}

/// The offset of the trailer, after the entries.
fn tail_start() -> Option<u64> {
    entries_start()?.checked_add(SIZE.checked_mul(ENTRY)?)
}

/// The trailer, `startxref` and `%%EOF` (ISO 32000-2:2020, 7.5.5).
fn tail(xref: u64) -> Vec<u8> {
    format!("trailer\n<< /Size {SIZE} /Root {CATALOG} 0 R >>\nstartxref\n{xref}\n%%EOF\n")
        .into_bytes()
}

/// The segment holding `offset`: its start and bytes.
fn segment(offset: u64) -> Option<(u64, Vec<u8>)> {
    let header = u64::try_from(HEADER.len()).ok()?;
    let xref = xref_offset()?;
    let entries = entries_start()?;
    let tail_at = tail_start()?;
    if offset < header {
        Some((0, HEADER.to_vec()))
    } else if offset < xref {
        let index = offset.checked_sub(header)?.checked_div(RECORD)?;
        Some((
            header.checked_add(index.checked_mul(RECORD)?)?,
            object(index.checked_add(1)?)?,
        ))
    } else if offset < entries {
        Some((xref, xref_head()))
    } else if offset < tail_at {
        let index = offset.checked_sub(entries)?.checked_div(ENTRY)?;
        Some((
            entries.checked_add(index.checked_mul(ENTRY)?)?,
            entry(index)?,
        ))
    } else {
        Some((tail_at, tail(xref)))
    }
}

/// Opens `generated` without a policy.
fn open<T>(generated: &Arc<T>) -> Result<Document, Failure>
where
    T: ReadAt + 'static,
{
    let read_at: Arc<dyn ReadAt> = Arc::<T>::clone(generated);
    Source::from_read_at(&read_at, &ReadAtSourceOptions::default())
        .and_then(|source| Document::open(&source, None, &OpenOptions::default()))
        .map_err(|error: PdfError| (error.code(), error.io_cause()))
}

/// A `Generated` that serves.
fn generated() -> Arc<Generated> {
    Arc::new(Generated {
        failing: AtomicBool::new(false),
        served: AtomicUsize::new(0),
    })
}

#[test]
fn eight_threads_count_pages_through_one_file_source() {
    let mut options = FileSourceOptions::default();
    options.file = PathBuf::from(NESTED_TREE);
    let opened = Source::from_file(&options)
        .and_then(|source| Document::open(&source, None, &OpenOptions::default()))
        .map_err(|error| error.code());
    assert!(opened.is_ok(), "open: {opened:?}");
    let Ok(document) = opened else { return };
    thread::scope(|scope| {
        for _ in 0..8_u8 {
            let _thread = scope.spawn(|| {
                for _ in 0..1_000_u16 {
                    assert_eq!(
                        document.page_count().map_err(|error| error.code()),
                        Ok(3),
                        "page count"
                    );
                }
            });
        }
    });
}

#[test]
fn page_count_serves_at_most_64_kib_of_a_1_gib_source() {
    let source = generated();
    assert_eq!(
        open(&source).map(|document| document
            .page_count()
            .map_err(|error| (error.code(), error.io_cause()))),
        Ok(Ok(u32::try_from(PAGES).unwrap_or_default())),
        "page count"
    );
    let served = source.served.load(Ordering::Relaxed);
    assert!(served <= 0x1_0000, "served {served} bytes");
}

#[test]
fn page_count_fails_with_the_io_cause_of_a_failed_read() {
    let source = generated();
    let counted = open(&source).map(|document| {
        source.failing.store(true, Ordering::Relaxed);
        document
            .page_count()
            .map_err(|error| (error.code(), error.io_cause()))
    });
    assert_eq!(
        counted,
        Ok(Err((ErrorCode::Io, Some(ErrorKind::TimedOut)))),
        "page count"
    );
}

#[test]
fn page_count_of_a_1_gib_stream_file_serves_at_most_64_kib() {
    let made = Streams::new(false).map(Arc::new);
    assert_eq!(made.as_ref().map(|file| file.size().ok()), Some(Some(GIB)));
    let Some(source) = made else { return };
    assert_eq!(
        open(&source).map(|document| document
            .page_count()
            .map_err(|error| (error.code(), error.io_cause()))),
        Ok(Ok(u32::try_from(PAGES).unwrap_or_default())),
        "page count"
    );
    let served = source.served.load(Ordering::Relaxed);
    assert!(served <= 0x1_0000, "served {served} bytes");
}

#[test]
fn page_count_fails_with_the_io_cause_of_a_failed_read_in_stream_data() {
    let made = Streams::new(true).map(Arc::new);
    assert!(made.is_some(), "source");
    let Some(source) = made else { return };
    assert_eq!(
        open(&source).and_then(|document| document
            .page_count()
            .map_err(|error| (error.code(), error.io_cause()))),
        Err((ErrorCode::Io, Some(ErrorKind::ConnectionReset))),
        "page count"
    );
}
