//! Error codes through the public API.

extern crate alloc;

use alloc::sync::Arc;
use core::error::Error;
use std::fs;

use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::objects::StreamDataOptions;
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::{ErrorCode, PdfError, Version, header_version};

/// An error's code and offset.
type Failure = (ErrorCode, Option<u64>);

/// Fails with a boxed error, as `?` in a caller's `main` does.
fn boxed() -> Result<(), Box<dyn Error>> {
    let _: Version = header_version(b"")?;
    Ok(())
}

/// Bytes without a header fail with `InvalidHeader` (ISO 32000-2:2020, 7.5.2).
#[test]
fn invalid_header_has_its_code() {
    assert_eq!(
        header_version(b"").map_err(|error| error.code()),
        Err(ErrorCode::InvalidHeader),
        "code"
    );
}

/// `?` carries an error into `Box<dyn Error>` with its code intact.
#[test]
fn error_converts_into_a_boxed_error() {
    assert_eq!(
        boxed().map_err(|error| error.downcast_ref::<PdfError>().map(PdfError::code)),
        Err(Some(ErrorCode::InvalidHeader)),
        "code"
    );
}

/// Each code keeps its number: bindings and C callers compile them in.
#[test]
fn codes_keep_their_numbers() {
    let numbers: [(ErrorCode, u16); 18] = [
        (ErrorCode::BudgetExceeded, 1),
        (ErrorCode::Busy, 2),
        (ErrorCode::Cancelled, 3),
        (ErrorCode::CapabilityMissing, 4),
        (ErrorCode::DocumentClosed, 5),
        (ErrorCode::IncrementalRefused, 6),
        (ErrorCode::InvalidArgument, 7),
        (ErrorCode::InvalidHeader, 8),
        (ErrorCode::InvalidPassword, 9),
        (ErrorCode::Io, 10),
        (ErrorCode::LimitExceeded, 11),
        (ErrorCode::LossRefused, 12),
        (ErrorCode::Malformed, 13),
        (ErrorCode::NotImplemented, 14),
        (ErrorCode::NotSeekable, 15),
        (ErrorCode::SignaturesWouldBreak, 16),
        (ErrorCode::StaleHandle, 17),
        (ErrorCode::Unsupported, 18),
    ];
    for (code, number) in numbers {
        assert_eq!(code.number(), number, "number");
    }
}

/// Reading object 4 of `repair/length-wrong.pdf` strictly: the error's code and offset, and the
/// code and offset expected, those of the stream's data.
fn strict_stream_error() -> Result<[Failure; 2], Box<dyn Error>> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/corpus/repair/length-wrong.pdf"
    );
    let bytes = fs::read(path)?;
    // Object 4 is the only stream; its data follows `stream` and LF.
    let keyword = b"stream\n";
    let data = bytes
        .windows(keyword.len())
        .position(|window| window == keyword)
        .ok_or("no stream keyword")?
        .saturating_add(keyword.len());
    let mut source = BytesSourceOptions::default();
    source.bytes = Arc::new(bytes);
    let mut options = OpenOptions::default();
    options.strict = true;
    let document = Document::open(&Source::from_bytes(&source)?, None, &options)?;
    let mut read = StreamDataOptions::default();
    read.id.number = 4;
    let error = document
        .stream_data(&Sink::in_memory(&MemorySinkOptions::default())?, &read)
        .err()
        .ok_or("read")?;
    Ok([
        (error.code(), error.offset()),
        (ErrorCode::Malformed, Some(u64::try_from(data)?)),
    ])
}

/// With `strict`, a deviation fails with `Malformed` at its offset: a wrong `Length` at the start
/// of the stream's data (ISO 32000-2:2020, 7.3.8.1 and 7.3.8.2).
#[test]
fn strict_deviation_carries_its_offset() {
    let pair = strict_stream_error().map_err(|error| error.to_string());
    assert!(
        matches!(&pair, Ok([found, expected]) if found == expected),
        "code and offset: {pair:?}"
    );
}
