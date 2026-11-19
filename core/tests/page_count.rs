//! Counting pages through a caller's source.

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::io::{self, ErrorKind};

use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{ReadAt, ReadAtSourceOptions, Source};
use starlightpdf::{ErrorCode, PdfError};

/// The catalog's object number, mid-file; the page tree root and pages follow it.
const CATALOG: u64 = 500_000;

/// Bytes per cross-reference entry (ISO 32000-2:2020, 7.5.4).
const ENTRY: u64 = 20;

/// A valid header (ISO 32000-2:2020, 7.5.2).
const HEADER: &[u8] = b"%PDF-2.0\n";

/// Objects, numbered from 1.
const OBJECTS: u64 = 1_000_000;

/// Page objects, the root's only kids.
const PAGES: u64 = 3;

/// Bytes per object; makes the file just over 1 GiB.
const RECORD: u64 = 1054;

/// Entries in the cross-reference table, object 0 included.
const SIZE: u64 = OBJECTS + 1;

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
fn open(generated: &Arc<Generated>) -> Result<Document, Failure> {
    let read_at: Arc<dyn ReadAt> = Arc::<Generated>::clone(generated);
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
