//! `RunLengthDecode` (ISO 32000-2:2020, 7.4.5).

use crate::pull::{Buffered, Pull};
use crate::{ErrorCode, PdfError};

/// The error operation name.
const OPERATION: &str = "RunLengthDecode";

/// Decodes `RunLengthDecode` data; the end of the data between runs counts as the EOD marker.
#[derive(Debug)]
pub(crate) struct RunLength<P> {
    /// The EOD marker was read.
    done: bool,
    /// The input.
    input: Buffered<P>,
    /// Bytes left in the current run.
    left: u8,
    /// The repeated byte, or `None` in a run of literal bytes.
    repeat: Option<u8>,
}

impl<P: Pull> RunLength<P> {
    /// A decoder of the data read from `inner`.
    pub(crate) fn new(inner: P) -> Self {
        Self {
            done: false,
            input: Buffered::new(inner),
            left: 0,
            repeat: None,
        }
    }

    /// Starts the next run; false at the EOD marker or the end of the data.
    fn start(&mut self) -> Result<bool, PdfError> {
        match self.input.byte()? {
            None | Some(128) => {
                self.done = true;
                return Ok(false);
            }
            Some(length @ 0..=127) => {
                self.left = length.wrapping_add(1);
                self.repeat = None;
            }
            Some(length) => {
                self.left = 1_u8.wrapping_sub(length);
                self.repeat = Some(self.input.byte()?.ok_or_else(malformed)?);
            }
        }
        Ok(true)
    }
}

impl<P: Pull> Pull for RunLength<P> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let mut filled = 0_usize;
        for slot in buf.iter_mut() {
            if self.done || (self.left == 0 && !self.start()?) {
                break;
            }
            *slot = match self.repeat {
                Some(byte) => byte,
                None => self.input.byte()?.ok_or_else(malformed)?,
            };
            self.left = self.left.wrapping_sub(1);
            filled = filled.wrapping_add(1);
        }
        Ok(filled)
    }
}

/// The error for any defect in the data.
fn malformed() -> PdfError {
    PdfError::new(ErrorCode::Malformed, OPERATION)
}
