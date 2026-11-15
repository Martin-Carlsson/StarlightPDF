//! `StarlightPDF` core.

use core::error::Error;
use core::fmt::{self, Display, Formatter};

/// [`header_version`] found no valid header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HeaderError;

impl Display for HeaderError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("no valid PDF header")
    }
}

impl Error for HeaderError {
    fn cause(&self) -> Option<&dyn Error> {
        None
    }

    fn description(&self) -> &'static str {
        "no valid PDF header"
    }

    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

/// A PDF version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Version {
    /// Major version.
    pub major: u8,
    /// Minor version.
    pub minor: u8,
}

/// Reads the version from the header (ISO 32000-2:2020, 7.5.2).
///
/// # Errors
///
/// [`HeaderError`] unless `bytes` begins `%PDF-1.n` or `%PDF-2.n`, n a digit, then CR or LF.
pub const fn header_version(bytes: &[u8]) -> Result<Version, HeaderError> {
    match *bytes {
        [
            b'%',
            b'P',
            b'D',
            b'F',
            b'-',
            major @ (b'1' | b'2'),
            b'.',
            minor @ b'0'..=b'9',
            b'\r' | b'\n',
            ..,
        ] => Ok(Version {
            major: major.wrapping_sub(b'0'),
            minor: minor.wrapping_sub(b'0'),
        }),
        _ => Err(HeaderError),
    }
}
