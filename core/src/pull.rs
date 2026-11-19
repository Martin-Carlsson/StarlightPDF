//! A source of bytes read on demand.

use crate::PdfError;

/// A source of bytes read on demand.
pub(crate) trait Pull {
    /// Fills up to `buf.len()` bytes and returns how many; returns 0 only at the end of the data.
    ///
    /// # Errors
    ///
    /// Whatever the source fails with, unchanged.
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError>;
}
