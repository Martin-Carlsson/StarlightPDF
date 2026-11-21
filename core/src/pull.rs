//! A source of bytes read on demand.

use crate::PdfError;

/// The read buffer's size.
const BUFFER: usize = 0x1000;

/// A [`Pull`] read a byte at a time through a buffer.
#[derive(Debug)]
pub(crate) struct Buffered<P> {
    /// Read-ahead bytes.
    buffer: Vec<u8>,
    /// The number of valid bytes in `buffer`.
    end: usize,
    /// The input.
    inner: P,
    /// The next unread byte in `buffer`.
    position: usize,
}

/// A source of bytes read on demand.
pub(crate) trait Pull {
    /// Fills up to `buf.len()` bytes and returns how many; returns 0 only at the end of the data.
    ///
    /// # Errors
    ///
    /// Whatever the source fails with, unchanged.
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError>;
}

impl<P: Pull> Buffered<P> {
    /// The next byte, or `None` at the end of the data.
    pub(crate) fn byte(&mut self) -> Result<Option<u8>, PdfError> {
        if self.position >= self.end {
            self.end = self.inner.pull(&mut self.buffer)?.min(self.buffer.len());
            self.position = 0;
            if self.end == 0 {
                return Ok(None);
            }
        }
        let byte = self.buffer.get(self.position).copied();
        self.position = self.position.wrapping_add(1);
        Ok(byte)
    }

    /// Reads `inner` through a buffer.
    pub(crate) fn new(inner: P) -> Self {
        Self {
            buffer: vec![0; BUFFER],
            end: 0,
            inner,
            position: 0,
        }
    }
}

impl Pull for &[u8] {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let count = self.len().min(buf.len());
        let (head, rest) = self.split_at_checked(count).unwrap_or_default();
        for (slot, byte) in buf.iter_mut().zip(head) {
            *slot = *byte;
        }
        *self = rest;
        Ok(count)
    }
}
