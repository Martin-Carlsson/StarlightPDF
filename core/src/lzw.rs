//! `LZWDecode` (ISO 32000-2:2020, 7.4.4.2 and 7.4.4.3).

use core::cmp::Ordering;

use crate::pull::{Buffered, Pull};
use crate::{ErrorCode, PdfError};

/// The clear-table code.
const CLEAR: u16 = 256;

/// The EOD code.
const EOD: u16 = 257;

/// The first code the table assigns.
const FIRST: u16 = 258;

/// The table's size: every 12-bit code.
const TABLE: usize = 0x1000;

/// The error operation name.
const OPERATION: &str = "LZWDecode";

/// Decodes `LZWDecode` data; the end of the data counts as the EOD code.
#[derive(Debug)]
pub(crate) struct Lzw<P> {
    /// Unread input bits, the last `count` of them.
    bits: u32,
    /// How many bits `bits` holds.
    count: u32,
    /// The EOD code was read.
    done: bool,
    /// 1 to widen codes one code early, else 0.
    early: u16,
    /// Each code's first byte.
    first: Vec<u8>,
    /// The input.
    input: Buffered<P>,
    /// Each code's last byte.
    last: Vec<u8>,
    /// The next code the table assigns.
    next: u16,
    /// Each code's string without its last byte, as a code.
    prefix: Vec<u16>,
    /// The string of the previous code, unless the table was just cleared.
    previous: Option<u16>,
    /// The current code's string, reversed; served from the end.
    string: Vec<u8>,
}

impl<P: Pull> Lzw<P> {
    /// Adds the string of `previous` and the first byte of `code`'s to the table, unless full.
    fn add(&mut self, previous: u16, code: u16) -> Result<(), PdfError> {
        let start = *self
            .first
            .get(usize::from(previous))
            .ok_or_else(malformed)?;
        let first = match code.cmp(&self.next) {
            Ordering::Less => *self.first.get(usize::from(code)).ok_or_else(malformed)?,
            Ordering::Equal => start,
            Ordering::Greater => return Err(malformed()),
        };
        let added = usize::from(self.next);
        if added < TABLE {
            *self.first.get_mut(added).ok_or_else(malformed)? = start;
            *self.last.get_mut(added).ok_or_else(malformed)? = first;
            *self.prefix.get_mut(added).ok_or_else(malformed)? = previous;
            self.next = self.next.wrapping_add(1);
        }
        Ok(())
    }

    /// The next code, `None` at the end of the data.
    fn code(&mut self) -> Result<Option<u16>, PdfError> {
        let sum = self.next.wrapping_add(self.early);
        let width = (u16::BITS.wrapping_sub(sum.leading_zeros())).clamp(9, 12);
        while self.count < width {
            let Some(byte) = self.input.byte()? else {
                return Ok(None);
            };
            self.bits = self.bits.wrapping_shl(8) | u32::from(byte);
            self.count = self.count.wrapping_add(8);
        }
        self.count = self.count.wrapping_sub(width);
        let code = self.bits.wrapping_shr(self.count);
        self.bits &= !u32::MAX.wrapping_shl(self.count);
        u16::try_from(code).map(Some).ok().ok_or_else(malformed)
    }

    /// Reads codes until one gives a string; `false` at the end.
    fn fetch(&mut self) -> Result<bool, PdfError> {
        loop {
            let Some(code) = self.code()?.filter(|&code| code != EOD) else {
                self.done = true;
                return Ok(false);
            };
            if code == CLEAR {
                self.next = FIRST;
                self.previous = None;
                continue;
            }
            match self.previous {
                Some(previous) => self.add(previous, code)?,
                None if code >= CLEAR => return Err(malformed()),
                None => {}
            }
            self.previous = Some(code);
            let mut walk = code;
            while walk >= FIRST {
                let at = usize::from(walk);
                self.string.push(*self.last.get(at).ok_or_else(malformed)?);
                walk = *self.prefix.get(at).ok_or_else(malformed)?;
            }
            self.string
                .push(u8::try_from(walk).ok().ok_or_else(malformed)?);
            return Ok(true);
        }
    }

    /// A decoder of the data read from `inner`; `early_change` widens codes one code early.
    pub(crate) fn new(inner: P, early_change: bool) -> Self {
        let mut first = vec![0; TABLE];
        for (byte, value) in first.iter_mut().zip(0..=u8::MAX) {
            *byte = value;
        }
        let last = first.clone();
        Self {
            bits: 0,
            count: 0,
            done: false,
            early: u16::from(early_change),
            first,
            input: Buffered::new(inner),
            last,
            next: FIRST,
            prefix: vec![0; TABLE],
            previous: None,
            string: Vec::with_capacity(TABLE),
        }
    }
}

impl<P: Pull> Pull for Lzw<P> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let mut filled = 0_usize;
        for slot in buf.iter_mut() {
            if self.string.is_empty() && (self.done || !self.fetch()?) {
                break;
            }
            *slot = self.string.pop().ok_or_else(malformed)?;
            filled = filled.wrapping_add(1);
        }
        Ok(filled)
    }
}

/// The error for any defect in the data.
fn malformed() -> PdfError {
    PdfError::new(ErrorCode::Malformed, OPERATION)
}
