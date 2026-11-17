//! Error codes through the public API.

use starlightpdf::{ErrorCode, header_version};

/// Bytes without a header fail with `InvalidHeader` (ISO 32000-2:2020, 7.5.2).
#[test]
fn invalid_header_has_its_code() {
    assert_eq!(
        header_version(b"").map_err(|error| error.code()),
        Err(ErrorCode::InvalidHeader),
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
