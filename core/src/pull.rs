//! A source of bytes read on demand.

use alloc::sync::Arc;
use core::iter;
use core::ops::Deref;

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

/// Bytes held, read in order.
pub(crate) struct Held<B> {
    /// The bytes.
    bytes: B,
    /// The next unread byte.
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

impl<B> Held<B>
where
    B: Deref<Target: AsRef<[u8]>>,
{
    /// Reads `bytes` from the one at `position`.
    pub(crate) const fn at(bytes: B, position: usize) -> Self {
        Self { bytes, position }
    }

    /// Reads `bytes` from the first.
    pub(crate) const fn new(bytes: B) -> Self {
        Self { bytes, position: 0 }
    }
}

impl<B> Pull for Held<B>
where
    B: Deref<Target: AsRef<[u8]>>,
{
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let mut rest = (*self.bytes)
            .as_ref()
            .get(self.position..)
            .unwrap_or_default();
        let count = rest.pull(buf)?;
        self.position = self.position.saturating_add(count);
        Ok(count)
    }
}

impl<P: Pull + ?Sized> Pull for &mut P {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        (**self).pull(buf)
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

/// Pulls until `out` is full or the data ends; returns the count.
pub(crate) fn fill(data: &mut dyn Pull, out: &mut [u8]) -> Result<usize, PdfError> {
    let mut filled = 0_usize;
    while let Some(rest) = out.get_mut(filled..).filter(|rest| !rest.is_empty()) {
        match data.pull(rest)? {
            0 => break,
            count => filled = filled.saturating_add(count),
        }
    }
    Ok(filled)
}

/// The big-endian number in `bytes`.
pub(crate) fn number(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |value: u32, byte| {
        value.wrapping_shl(8) | u32::from(*byte)
    })
}

/// All of `data` in one buffer, where exactly `size` bytes; none where shorter or longer.
pub(crate) fn exactly(data: &mut dyn Pull, size: usize) -> Result<Option<Arc<[u8]>>, PdfError> {
    let mut bytes: Arc<[u8]> = iter::repeat_n(0, size).collect();
    let Some(slots) = Arc::get_mut(&mut bytes) else {
        return Ok(None);
    };
    let mut filled = 0_usize;
    while let Some(rest) = slots.get_mut(filled..).filter(|rest| !rest.is_empty()) {
        match data.pull(rest)? {
            0 => return Ok(None),
            count => filled = filled.saturating_add(count),
        }
    }
    let mut more = [0_u8; 1];
    Ok((data.pull(&mut more)? == 0).then_some(bytes))
}
