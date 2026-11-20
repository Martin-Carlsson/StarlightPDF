//! `ASCIIHexDecode` (ISO 32000-2:2020, 7.4.2).

use crate::pull::{Buffered, Pull};
use crate::syntax::is_white_space;
use crate::{ErrorCode, PdfError};

/// The error operation name.
const OPERATION: &str = "ASCIIHexDecode";

/// Decodes `ASCIIHexDecode` data; the end of the data counts as the EOD marker.
#[derive(Debug)]
pub(crate) struct AsciiHex<P> {
    /// The EOD marker was read.
    done: bool,
    /// The input.
    input: Buffered<P>,
}

impl<P: Pull> AsciiHex<P> {
    /// The next digit's value, skipping white space; `None` at the EOD marker.
    fn digit(&mut self) -> Result<Option<u8>, PdfError> {
        loop {
            let Some(byte) = self.input.byte()?.filter(|&byte| byte != b'>') else {
                self.done = true;
                return Ok(None);
            };
            let value = match byte {
                b'0'..=b'9' => byte.wrapping_sub(b'0'),
                b'A'..=b'F' => byte.wrapping_sub(b'A').wrapping_add(10),
                b'a'..=b'f' => byte.wrapping_sub(b'a').wrapping_add(10),
                _ if is_white_space(byte) => continue,
                _ => return Err(PdfError::new(ErrorCode::Malformed, OPERATION)),
            };
            return Ok(Some(value));
        }
    }

    /// A decoder of the data read from `inner`.
    pub(crate) fn new(inner: P) -> Self {
        Self {
            done: false,
            input: Buffered::new(inner),
        }
    }

    /// The next decoded byte; a final odd digit is followed by 0.
    fn next_byte(&mut self) -> Result<Option<u8>, PdfError> {
        if self.done {
            return Ok(None);
        }
        let Some(high) = self.digit()? else {
            return Ok(None);
        };
        let low = self.digit()?.unwrap_or(0);
        Ok(Some(high.wrapping_shl(4) | low))
    }
}

impl<P: Pull> Pull for AsciiHex<P> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let mut filled = 0_usize;
        for slot in buf.iter_mut() {
            let Some(byte) = self.next_byte()? else {
                break;
            };
            *slot = byte;
            filled = filled.wrapping_add(1);
        }
        Ok(filled)
    }
}
