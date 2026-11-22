//! Making sources and opening them.

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::io::{self, ErrorKind};
use std::path::PathBuf;

use starlightpdf::arrange::InsertBlankPageOptions;
use starlightpdf::document::{CreateOptions, Document, OpenOptions};
use starlightpdf::drawing::DrawImageOptions;
use starlightpdf::input::{FileSourceOptions, ReadAt, ReadAtSourceOptions, Source};
use starlightpdf::{ErrorCode, PdfError};

/// The corpus directory of header files.
const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/corpus/header");

/// 1 GiB.
const GIB: usize = 0x4000_0000;

/// A valid header (ISO 32000-2:2020, 7.5.2).
const HEADER: &[u8] = b"%PDF-2.0\n";

/// Most bytes open may be served of a 1 GiB file whose header, objects, table and trailer lie in
/// its first and last 4 KiB: each of those ends served at most twice. A rebuild reads it all.
const SERVED_BOUND: usize = 0x4000;

/// An error's code and I/O cause.
type Failure = (ErrorCode, Option<ErrorKind>);

/// Fails every read.
struct Failing;

/// `HEADER`, zeros, then `tail` ending at `size`; counts the bytes it serves.
struct Header {
    /// Bytes served so far.
    served: AtomicUsize,
    /// The reported size.
    size: usize,
    /// The last bytes.
    tail: Vec<u8>,
}

impl ReadAt for Failing {
    fn read_at(&self, _offset: u64, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(ErrorKind::TimedOut.into())
    }

    fn size(&self) -> io::Result<u64> {
        u64::try_from(GIB).map_err(io::Error::other)
    }
}

impl ReadAt for Header {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self.size.saturating_sub(start).min(buffer.len());
        let tail_at = self.size.saturating_sub(self.tail.len());
        for (byte, position) in buffer.iter_mut().take(count).zip(start..) {
            *byte = position
                .checked_sub(tail_at)
                .map_or_else(|| HEADER.get(position), |at| self.tail.get(at))
                .copied()
                .unwrap_or_default();
        }
        let _previous = self.served.fetch_add(count, Ordering::Relaxed);
        Ok(count)
    }

    fn size(&self) -> io::Result<u64> {
        u64::try_from(self.size).map_err(io::Error::other)
    }
}

/// A `Header` reporting `size`, without a tail.
const fn header(size: usize) -> Header {
    Header {
        served: AtomicUsize::new(0),
        size,
        tail: Vec::new(),
    }
}

/// A `Header` of `size` bytes ending in a one-page PDF's objects, cross-reference table and
/// trailer (ISO 32000-2:2020, 7.5.4, 7.5.5).
fn valid(size: usize) -> Header {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> >>",
    ];
    // Every offset, `startxref`'s too, is zero-padded, so the length does not depend on `start`.
    let lay = |start: usize| {
        let mut laid = Vec::new();
        let mut entries = b"0000000000 65535 f\r\n".to_vec();
        for (number, object) in (1_u32..).zip(objects) {
            let at = start.saturating_add(laid.len());
            entries.extend_from_slice(format!("{at:010} 00000 n\r\n").as_bytes());
            laid.extend_from_slice(format!("{number} 0 obj\n{object}\nendobj\n").as_bytes());
        }
        let xref = start.saturating_add(laid.len());
        laid.extend_from_slice(b"xref\n0 4\n");
        laid.extend_from_slice(&entries);
        laid.extend_from_slice(
            format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref:010}\n%%EOF\n")
                .as_bytes(),
        );
        laid
    };
    Header {
        served: AtomicUsize::new(0),
        size,
        tail: lay(size.saturating_sub(lay(0).len())),
    }
}

/// Opens a made source without a policy; per step, the failure.
fn open(made: Result<Source, PdfError>) -> Result<Result<(), Failure>, Failure> {
    made.map_err(|error| (error.code(), error.io_cause()))
        .map(|source| {
            Document::open(&source, None, &OpenOptions::default())
                .map(drop)
                .map_err(|error| (error.code(), error.io_cause()))
        })
}

#[test]
fn source_over_missing_file_fails_with_io_not_found() {
    let mut options = FileSourceOptions::default();
    options.file = PathBuf::from(format!("{CORPUS}/missing.pdf"));
    assert_eq!(
        open(Source::from_file(&options)),
        Err((ErrorCode::Io, Some(ErrorKind::NotFound))),
        "source"
    );
}

#[test]
fn open_does_not_read_the_body_of_a_1_gib_source() {
    let counted = Arc::new(valid(GIB));
    let read_at: Arc<dyn ReadAt> = Arc::<Header>::clone(&counted);
    assert_eq!(
        open(Source::from_read_at(
            &read_at,
            &ReadAtSourceOptions::default()
        )),
        Ok(Ok(())),
        "open"
    );
    let served = counted.served.load(Ordering::Relaxed);
    assert!(served <= SERVED_BOUND, "served {served} bytes");
}

#[test]
fn open_fails_with_the_cause_of_a_failed_read() {
    let read_at: Arc<dyn ReadAt> = Arc::new(Failing);
    assert_eq!(
        open(Source::from_read_at(
            &read_at,
            &ReadAtSourceOptions::default()
        )),
        Ok(Err((ErrorCode::Io, Some(ErrorKind::TimedOut)))),
        "open"
    );
}

#[test]
fn open_forward_only_without_policy_fails_not_seekable() {
    let read_at: Arc<dyn ReadAt> = Arc::new(header(HEADER.len()));
    let mut options = ReadAtSourceOptions::default();
    options.forward_only = true;
    assert_eq!(
        open(Source::from_read_at(&read_at, &options))
            .map(|opened| opened.map_err(|(code, _)| code)),
        Ok(Err(ErrorCode::NotSeekable)),
        "open"
    );
}

#[test]
fn draw_image_from_a_forward_only_source_fails_not_seekable() {
    let read_at: Arc<dyn ReadAt> = Arc::new(Failing);
    let mut options = ReadAtSourceOptions::default();
    options.forward_only = true;
    let drawn = Source::from_read_at(&read_at, &options).and_then(|image| {
        let document = Document::create(None, &CreateOptions::default())?;
        let page = document.insert_blank_page(&InsertBlankPageOptions::default())?;
        page.draw_image(&image, None, None, &DrawImageOptions::default())
    });
    assert_eq!(
        drawn.map_err(|error| error.code()),
        Err(ErrorCode::NotSeekable),
        "draw_image"
    );
}
