//! `ASCII85Decode` (ISO 32000-2:2020, 7.4.3).

use crate::pull::{Buffered, Pull};
use crate::syntax::is_white_space;
use crate::{ErrorCode, PdfError};

/// The error operation name.
const OPERATION: &str = "ASCII85Decode";

/// Decodes `ASCII85Decode` data; the end of the data counts as the EOD marker.
#[derive(Debug)]
pub(crate) struct Ascii85<P> {
    /// The decoded bytes of the current group.
    decoded: [u8; 4],
    /// The EOD marker was read.
    done: bool,
    /// The input.
    input: Buffered<P>,
    /// How many of `decoded` hold data.
    length: usize,
    /// How many of `decoded` are served.
    served: usize,
}

impl<P: Pull> Ascii85<P> {
    /// Reads the `>` that ends the EOD marker after `~`.
    fn end(&mut self) -> Result<(), PdfError> {
        self.done = true;
        loop {
            match self.input.byte()? {
                Some(b'>') => return Ok(()),
                Some(byte) if is_white_space(byte) => {}
                Some(_) | None => return Err(malformed()),
            }
        }
    }

    /// Decodes the next group into `decoded`; `false` at the EOD marker.
    fn group(&mut self) -> Result<bool, PdfError> {
        let mut value = 0_u64;
        let mut count = 0_usize;
        while count < 5 {
            let Some(byte) = self.input.byte()? else {
                break;
            };
            match byte {
                b'!'..=b'u' => {
                    value = value
                        .wrapping_mul(85)
                        .wrapping_add(u64::from(byte.wrapping_sub(b'!')));
                    count = count.wrapping_add(1);
                }
                b'z' if count == 0 => {
                    self.decoded = [0; 4];
                    self.length = 4;
                    return Ok(true);
                }
                b'~' => {
                    self.end()?;
                    break;
                }
                _ if is_white_space(byte) => {}
                _ => return Err(malformed()),
            }
        }
        match count {
            0 => return Ok(false),
            1 => return Err(malformed()),
            _ => {}
        }
        for _ in count..5 {
            value = value.wrapping_mul(85).wrapping_add(84);
        }
        let word = u32::try_from(value).ok().ok_or_else(malformed)?;
        for (index, byte) in self.decoded.iter_mut().enumerate() {
            let shift = u32::try_from(index).unwrap_or_default().wrapping_mul(8);
            *byte = u8::try_from(word.wrapping_shl(shift).wrapping_shr(24)).unwrap_or_default();
        }
        self.length = count.wrapping_sub(1);
        Ok(true)
    }

    /// A decoder of the data read from `inner`.
    pub(crate) fn new(inner: P) -> Self {
        Self {
            decoded: [0; 4],
            done: false,
            input: Buffered::new(inner),
            length: 0,
            served: 0,
        }
    }
}

impl<P: Pull> Pull for Ascii85<P> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let mut filled = 0_usize;
        for slot in buf.iter_mut() {
            if self.served >= self.length {
                if self.done || !self.group()? {
                    break;
                }
                self.served = 0;
            }
            *slot = self
                .decoded
                .get(self.served)
                .copied()
                .ok_or_else(malformed)?;
            self.served = self.served.wrapping_add(1);
            filled = filled.wrapping_add(1);
        }
        Ok(filled)
    }
}

/// The error for any defect in the data.
fn malformed() -> PdfError {
    PdfError::new(ErrorCode::Malformed, OPERATION)
}
