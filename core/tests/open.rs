//! Making sources and opening them.

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::io::{self, ErrorKind};
use std::path::PathBuf;

use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{FileSourceOptions, ReadAt, ReadAtSourceOptions, Source};
use starlightpdf::{ErrorCode, PdfError};

/// The corpus directory of header files.
const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/corpus/header");

/// 1 GiB.
const GIB: usize = 0x4000_0000;

/// A valid header (ISO 32000-2:2020, 7.5.2).
const HEADER: &[u8] = b"%PDF-2.0\n";

/// An error's code and I/O cause.
type Failure = (ErrorCode, Option<ErrorKind>);

/// Fails every read.
struct Failing;

/// `HEADER`, then zeros up to `size`; counts the bytes it serves.
struct Header {
    /// Bytes served so far.
    served: AtomicUsize,
    /// The reported size.
    size: usize,
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
        for (byte, position) in buffer.iter_mut().take(count).zip(start..) {
            *byte = HEADER.get(position).copied().unwrap_or_default();
        }
        let _previous = self.served.fetch_add(count, Ordering::Relaxed);
        Ok(count)
    }

    fn size(&self) -> io::Result<u64> {
        u64::try_from(self.size).map_err(io::Error::other)
    }
}

/// A `Header` reporting `size`.
const fn header(size: usize) -> Header {
    Header {
        served: AtomicUsize::new(0),
        size,
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
fn open_serves_at_most_1_kib_of_a_1_gib_source() {
    let counted = Arc::new(header(GIB));
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
    assert!(served <= 1024, "served {served} bytes");
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
