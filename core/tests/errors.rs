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
