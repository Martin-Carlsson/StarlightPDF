//! PNG predictor (ISO 32000-2:2020, 7.4.4.4; ISO/IEC 15948:2004, 9.2).

use core::mem::swap;

use crate::pull::Pull;
use crate::{ErrorCode, PdfError};

/// The error operation name.
const OPERATION: &str = "predictor";

/// The widest row accepted, in bytes.
const MAX_COLUMNS: u64 = 0x1_0000;

/// Undoes a predictor, with one component of eight bits per pixel.
#[derive(Debug)]
pub(crate) struct Predictor<P> {
    /// The row width in bytes.
    columns: usize,
    /// The row being served.
    current: Vec<u8>,
    /// The input.
    inner: P,
    /// The previous row.
    previous: Vec<u8>,
    /// How much of `current` is served.
    served: usize,
}

impl<P: Pull> Predictor<P> {
    /// Reads the next row into `current`; `false` at the end of the data.
    fn fetch(&mut self) -> Result<bool, PdfError> {
        swap(&mut self.previous, &mut self.current);
        let mut kind = [0_u8; 1];
        if self.inner.pull(&mut kind)? == 0 {
            return Ok(false);
        }
        let mut filled = 0_usize;
        while filled < self.columns {
            let rest = self.current.get_mut(filled..).ok_or_else(malformed)?;
            let read = self.inner.pull(rest)?;
            if read == 0 {
                return Err(malformed());
            }
            filled = filled.wrapping_add(read);
        }
        let mut left = 0_u8;
        let mut upper_left = 0_u8;
        let row_kind = kind.first().copied().ok_or_else(malformed)?;
        for (byte, &upper) in self.current.iter_mut().zip(self.previous.iter()) {
            let predicted = match row_kind {
                0 => 0,
                1 => left,
                2 => upper,
                3 => u8::try_from(
                    u16::from(left)
                        .wrapping_add(u16::from(upper))
                        .wrapping_shr(1),
                )
                .ok()
                .ok_or_else(malformed)?,
                4 => paeth(left, upper, upper_left),
                _ => return Err(malformed()),
            };
            *byte = byte.wrapping_add(predicted);
            left = *byte;
            upper_left = upper;
        }
        Ok(true)
    }

    /// Undoes a PNG predictor on rows of `columns` bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::LimitExceeded`] when a row is too wide.
    pub(crate) fn new(inner: P, columns: u64) -> Result<Self, PdfError> {
        if columns > MAX_COLUMNS {
            return Err(PdfError::new(ErrorCode::LimitExceeded, OPERATION));
        }
        let width = usize::try_from(columns).ok().ok_or_else(malformed)?;
        Ok(Self {
            columns: width,
            current: vec![0; width],
            inner,
            previous: vec![0; width],
            served: width,
        })
    }
}

impl<P: Pull> Pull for Predictor<P> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let mut filled = 0_usize;
        while filled < buf.len() {
            if self.served >= self.columns {
                if !self.fetch()? {
                    break;
                }
                self.served = 0;
            }
            let source = self.current.get(self.served..).ok_or_else(malformed)?;
            let target = buf.get_mut(filled..).ok_or_else(malformed)?;
            let count = source.len().min(target.len());
            let from = source.get(..count).ok_or_else(malformed)?;
            target
                .get_mut(..count)
                .ok_or_else(malformed)?
                .copy_from_slice(from);
            filled = filled.wrapping_add(count);
            self.served = self.served.wrapping_add(count);
        }
        Ok(filled)
    }
}

/// The error for any defect in the data.
fn malformed() -> PdfError {
    PdfError::new(ErrorCode::Malformed, OPERATION)
}

/// The Paeth predictor (ISO/IEC 15948:2004, 9.4).
fn paeth(left: u8, upper: u8, upper_left: u8) -> u8 {
    let pa = u16::from(upper.abs_diff(upper_left));
    let pb = u16::from(left.abs_diff(upper_left));
    let pc = u16::from(left)
        .wrapping_add(u16::from(upper))
        .abs_diff(u16::from(upper_left).wrapping_mul(2));
    if pa <= pb && pa <= pc {
        left
    } else if pb <= pc {
        upper
    } else {
        upper_left
    }
}
