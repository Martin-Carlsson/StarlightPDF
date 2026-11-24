//! Streaming zlib decoder (RFC 1950, RFC 1951).

use crate::pull::{Buffered, Pull};
use crate::store::Meter;
use crate::{ErrorCode, PdfError};

/// Work of a compressed block's header, which may bring no output.
const BLOCK_WORK: u64 = 256;

/// Work of a stored block's header, which may bring no output.
const STORED_WORK: u64 = 4;

/// Base lengths of length symbols 257 to 285 (RFC 1951, 3.2.5).
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];

/// Extra bits of length symbols 257 to 285 (RFC 1951, 3.2.5).
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// Base distances of distance symbols 0 to 29 (RFC 1951, 3.2.5).
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12_289, 0x4001, 0x6001,
];

/// Extra bits of distance symbols 0 to 29 (RFC 1951, 3.2.5).
const DISTANCE_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// The error operation name.
const OPERATION: &str = "inflate";

/// Order of the code length code lengths (RFC 1951, 3.2.7).
const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// The window size (RFC 1951, 2).
const WINDOW: usize = 0x8000;

/// The Adler-32 modulus (RFC 1950, 8.2).
const ADLER_MOD: u32 = 0xFFF1;

/// Reads bits and bytes from the input.
#[derive(Debug)]
struct Bits<P> {
    /// Unread bits left in `current`.
    count: u32,
    /// The byte being read bit by bit.
    current: u8,
    /// The input.
    input: Buffered<P>,
}

impl<P: Pull> Bits<P> {
    /// Drops the unread bits of the current byte.
    const fn align(&mut self) {
        self.count = 0;
    }

    /// The next bit.
    fn bit(&mut self) -> Result<u16, PdfError> {
        if self.count == 0 {
            self.current = self.byte()?;
            self.count = 8;
        }
        let value = u16::from(self.current & 1);
        self.current >>= 1_u8;
        self.count = self.count.wrapping_sub(1);
        Ok(value)
    }

    /// The next `count` bits, least significant first.
    fn bits(&mut self, count: u8) -> Result<u16, PdfError> {
        let mut value = 0_u16;
        for shift in 0..count {
            value |= self.bit()?.wrapping_shl(u32::from(shift));
        }
        Ok(value)
    }

    /// The next whole byte; the input ending is an error.
    fn byte(&mut self) -> Result<u8, PdfError> {
        self.input.byte()?.ok_or_else(malformed)
    }

    /// The next symbol of `table` (RFC 1951, 3.2.2).
    fn decode(&mut self, table: &Huffman) -> Result<u16, PdfError> {
        let mut code = 0_u16;
        let mut first = 0_u16;
        let mut index = 0_u16;
        for length in 1..16_usize {
            code |= self.bit()?;
            let count = *table.counts.get(length).ok_or_else(malformed)?;
            if code < first.wrapping_add(count) {
                let slot = usize::from(index.wrapping_add(code.wrapping_sub(first)));
                return table.symbols.get(slot).copied().ok_or_else(malformed);
            }
            index = index.wrapping_add(count);
            first = first.wrapping_add(count).wrapping_shl(1);
            code = code.wrapping_shl(1);
        }
        Err(malformed())
    }

    /// A little-endian 16-bit value.
    fn word(&mut self) -> Result<u16, PdfError> {
        let low = self.byte()?;
        let high = self.byte()?;
        Ok(u16::from(low) | u16::from(high).wrapping_shl(8))
    }
}

/// A canonical Huffman code.
#[derive(Clone, Debug)]
struct Huffman {
    /// How many codes have each length.
    counts: [u16; 16],
    /// The symbols ordered by code.
    symbols: [u16; 288],
}

impl Huffman {
    /// The fixed distance code (RFC 1951, 3.2.6).
    fn fixed_distance() -> Result<Self, PdfError> {
        Self::new(&[5_u8; 32])
            .map(|(table, _)| table)
            .ok_or_else(malformed)
    }

    /// The fixed literal/length code (RFC 1951, 3.2.6).
    fn fixed_literal() -> Result<Self, PdfError> {
        let mut lengths = [8_u8; 288];
        for (symbol, length) in lengths.iter_mut().enumerate() {
            *length = match symbol {
                144..=255 => 9,
                256..=279 => 7,
                _ => 8,
            };
        }
        Self::new(&lengths)
            .map(|(table, _)| table)
            .ok_or_else(malformed)
    }

    /// The code for `lengths`, and whether it is complete; `None` when over-subscribed.
    fn new(lengths: &[u8]) -> Option<(Self, bool)> {
        let mut counts = [0_u16; 16];
        for &length in lengths {
            let count = counts.get_mut(usize::from(length))?;
            *count = count.wrapping_add(1);
        }
        let mut left = 1_i32;
        let mut offsets = [0_u16; 16];
        let mut total = 0_u16;
        for (length, &count) in counts.iter().enumerate().skip(1) {
            left = left.wrapping_shl(1).wrapping_sub(i32::from(count));
            if left < 0_i32 {
                return None;
            }
            *offsets.get_mut(length)? = total;
            total = total.wrapping_add(count);
        }
        let mut symbols = [0_u16; 288];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length != 0 {
                let slot = offsets.get_mut(usize::from(length))?;
                *symbols.get_mut(usize::from(*slot))? = u16::try_from(symbol).ok()?;
                *slot = slot.wrapping_add(1);
            }
        }
        Some((Self { counts, symbols }, left == 0))
    }
}

/// Where decoding is.
#[derive(Clone, Copy, Debug)]
enum State {
    /// At a block header.
    Block,
    /// Inside a compressed block.
    Codes,
    /// Past the end.
    Done,
    /// At the zlib header.
    Header,
    /// Copying a match.
    Match {
        /// The distance back.
        distance: u16,
        /// The bytes left to copy.
        length: u16,
    },
    /// Copying this many bytes of a stored block.
    Stored(u16),
    /// At the Adler-32.
    Trailer,
}

/// A zlib stream decoded on demand (RFC 1950, RFC 1951).
#[derive(Debug)]
pub(crate) struct Inflate<'work, P> {
    /// Adler-32, high half.
    adler_high: u32,
    /// Adler-32, low half.
    adler_low: u32,
    /// The input.
    bits: Bits<P>,
    /// The distance code.
    distance: Huffman,
    /// The failure met, after output it still gives: given at the next pull, then its code at
    /// each after.
    failed: Option<(ErrorCode, Option<PdfError>)>,
    /// Whether the current block is the last.
    last: bool,
    /// The literal/length code.
    literal: Huffman,
    /// Where decoding is.
    state: State,
    /// The last 32 KiB of output.
    window: Vec<u8>,
    /// The work left, where it is bounded.
    work: Option<Meter<'work>>,
    /// The bytes output so far.
    written: u64,
}

impl<'work, P: Pull> Inflate<'work, P> {
    /// Starts the next block (RFC 1951, 3.2.3).
    fn block(&mut self) -> Result<(), PdfError> {
        self.last = self.bits.bit()? == 1;
        let kind = self.bits.bits(2)?;
        if let Some(work) = self.work {
            let units = if kind == 0 { STORED_WORK } else { BLOCK_WORK };
            work.take(units, OPERATION)?;
        }
        match kind {
            0 => {
                self.bits.align();
                let length = self.bits.word()?;
                let complement = self.bits.word()?;
                if length != !complement {
                    return Err(malformed());
                }
                self.state = State::Stored(length);
            }
            1 => {
                self.literal = Huffman::fixed_literal()?;
                self.distance = Huffman::fixed_distance()?;
                self.state = State::Codes;
            }
            2 => {
                self.dynamic()?;
                self.state = State::Codes;
            }
            _ => return Err(malformed()),
        }
        Ok(())
    }

    /// Reads the codes of a dynamic block (RFC 1951, 3.2.7).
    fn dynamic(&mut self) -> Result<(), PdfError> {
        let literals = usize::from(self.bits.bits(5)?).wrapping_add(257);
        let distances = usize::from(self.bits.bits(5)?).wrapping_add(1);
        let lengths_count = usize::from(self.bits.bits(4)?).wrapping_add(4);
        let mut code_lengths = [0_u8; 19];
        for &slot in ORDER.iter().take(lengths_count) {
            *code_lengths.get_mut(slot).ok_or_else(malformed)? = u8::try_from(self.bits.bits(3)?)
                .ok()
                .ok_or_else(malformed)?;
        }
        let (table, complete) = Huffman::new(&code_lengths).ok_or_else(malformed)?;
        if !complete {
            return Err(malformed());
        }
        let total = literals.wrapping_add(distances);
        let mut lengths = [0_u8; 320];
        let mut index = 0_usize;
        while index < total {
            let symbol = self.bits.decode(&table)?;
            let (value, repeat) = match symbol {
                0..=15 => (u8::try_from(symbol).ok().ok_or_else(malformed)?, 1_u16),
                16 => {
                    let previous = index.checked_sub(1).and_then(|last| lengths.get(last));
                    (
                        *previous.ok_or_else(malformed)?,
                        self.bits.bits(2)?.wrapping_add(3),
                    )
                }
                17 => (0, self.bits.bits(3)?.wrapping_add(3)),
                18 => (0, self.bits.bits(7)?.wrapping_add(11)),
                _ => return Err(malformed()),
            };
            for _ in 0..repeat {
                if index >= total {
                    return Err(malformed());
                }
                *lengths.get_mut(index).ok_or_else(malformed)? = value;
                index = index.wrapping_add(1);
            }
        }
        let literal_lengths = lengths.get(..literals).ok_or_else(malformed)?;
        let distance_lengths = lengths.get(literals..total).ok_or_else(malformed)?;
        if literal_lengths.get(256).copied().unwrap_or(0) == 0 {
            return Err(malformed());
        }
        let (literal, literal_complete) = Huffman::new(literal_lengths).ok_or_else(malformed)?;
        let (distance, distance_complete) = Huffman::new(distance_lengths).ok_or_else(malformed)?;
        if !literal_complete
            || (!distance_complete && distance_lengths.iter().any(|&length| length > 1))
        {
            return Err(malformed());
        }
        self.literal = literal;
        self.distance = distance;
        Ok(())
    }

    /// Records an output byte.
    fn emit(&mut self, byte: u8) -> Result<u8, PdfError> {
        let slot = usize::try_from(self.written & 0x7FFF)
            .ok()
            .ok_or_else(malformed)?;
        *self.window.get_mut(slot).ok_or_else(malformed)? = byte;
        self.written = self.written.saturating_add(1);
        self.adler_low = self.adler_low.wrapping_add(u32::from(byte));
        if self.adler_low >= ADLER_MOD {
            self.adler_low = self.adler_low.wrapping_sub(ADLER_MOD);
        }
        self.adler_high = self.adler_high.wrapping_add(self.adler_low);
        if self.adler_high >= ADLER_MOD {
            self.adler_high = self.adler_high.wrapping_sub(ADLER_MOD);
        }
        Ok(byte)
    }

    /// Ends the current block.
    const fn end_block(&mut self) {
        self.state = if self.last {
            State::Trailer
        } else {
            State::Block
        };
    }

    /// Reads the zlib header (RFC 1950, 2.2).
    fn header(&mut self) -> Result<(), PdfError> {
        let method = self.bits.byte()?;
        let flags = self.bits.byte()?;
        let check = u16::from(method).wrapping_shl(8) | u16::from(flags);
        if method & 0xF != 8
            || method >> 4_u8 > 7
            || check.checked_rem(31) != Some(0)
            || flags & 0x20 != 0
        {
            return Err(malformed());
        }
        self.state = State::Block;
        Ok(())
    }

    /// A decoder of the zlib stream read from `inner`.
    pub(crate) fn new(inner: P) -> Self {
        Self {
            adler_high: 0,
            adler_low: 1,
            bits: Bits {
                count: 0,
                current: 0,
                input: Buffered::new(inner),
            },
            distance: Huffman {
                counts: [0; 16],
                symbols: [0; 288],
            },
            failed: None,
            last: false,
            literal: Huffman {
                counts: [0; 16],
                symbols: [0; 288],
            },
            state: State::Header,
            window: vec![0; WINDOW],
            work: None,
            written: 0,
        }
    }

    /// The next output byte, or `None` at the end.
    fn next_byte(&mut self) -> Result<Option<u8>, PdfError> {
        loop {
            match self.state {
                State::Block => self.block()?,
                State::Codes => {
                    if let Some(byte) = self.symbol()? {
                        return Ok(Some(byte));
                    }
                }
                State::Done => return Ok(None),
                State::Header => self.header()?,
                State::Match { distance, length } => {
                    let from = self.written.wrapping_sub(u64::from(distance)) & 0x7FFF;
                    let slot = usize::try_from(from).ok().ok_or_else(malformed)?;
                    let byte = *self.window.get(slot).ok_or_else(malformed)?;
                    let left = length.wrapping_sub(1);
                    self.state = if left == 0 {
                        State::Codes
                    } else {
                        State::Match {
                            distance,
                            length: left,
                        }
                    };
                    return self.emit(byte).map(Some);
                }
                State::Stored(0) => self.end_block(),
                State::Stored(left) => {
                    let byte = self.bits.byte()?;
                    self.state = State::Stored(left.wrapping_sub(1));
                    return self.emit(byte).map(Some);
                }
                State::Trailer => self.trailer()?,
            }
        }
    }

    /// Decodes one literal/length symbol.
    fn symbol(&mut self) -> Result<Option<u8>, PdfError> {
        let symbol = self.bits.decode(&self.literal)?;
        match symbol {
            0..=255 => {
                let byte = u8::try_from(symbol).ok().ok_or_else(malformed)?;
                return self.emit(byte).map(Some);
            }
            256 => self.end_block(),
            257..=285 => {
                let index = usize::from(symbol.wrapping_sub(257));
                let base = *LENGTH_BASE.get(index).ok_or_else(malformed)?;
                let extra = *LENGTH_EXTRA.get(index).ok_or_else(malformed)?;
                let length = base.wrapping_add(self.bits.bits(extra)?);
                let slot = usize::from(self.bits.decode(&self.distance)?);
                let distance_base = *DISTANCE_BASE.get(slot).ok_or_else(malformed)?;
                let distance_extra = *DISTANCE_EXTRA.get(slot).ok_or_else(malformed)?;
                let distance = distance_base.wrapping_add(self.bits.bits(distance_extra)?);
                if u64::from(distance) > self.written {
                    return Err(malformed());
                }
                self.state = State::Match { distance, length };
            }
            _ => return Err(malformed()),
        }
        Ok(None)
    }

    /// Checks the Adler-32 (RFC 1950, 2.2).
    fn trailer(&mut self) -> Result<(), PdfError> {
        let mut expected = 0_u32;
        for _ in 0..4_u8 {
            expected = expected.wrapping_shl(8) | u32::from(self.bits.byte()?);
        }
        if expected != self.adler_high.wrapping_shl(16) | self.adler_low {
            return Err(malformed());
        }
        self.state = State::Done;
        Ok(())
    }

    /// The decoder, the blocks' codes taken from `work`.
    pub(crate) const fn within(mut self, work: Option<Meter<'work>>) -> Self {
        self.work = work;
        self
    }
}

impl<P: Pull> Pull for Inflate<'_, P> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        if let Some((code, ref mut pending)) = self.failed {
            return Err(pending
                .take()
                .unwrap_or_else(|| PdfError::new(code, OPERATION)));
        }
        let mut filled = 0_usize;
        for slot in buf.iter_mut() {
            match self.next_byte() {
                Ok(Some(byte)) => {
                    *slot = byte;
                    filled = filled.wrapping_add(1);
                }
                Ok(None) => break,
                // What came before the damage is given first.
                Err(error) if filled > 0 => {
                    self.failed = Some((error.code(), Some(error)));
                    break;
                }
                Err(error) => {
                    self.failed = Some((error.code(), None));
                    return Err(error);
                }
            }
        }
        Ok(filled)
    }
}

/// The error for any defect in the stream.
fn malformed() -> PdfError {
    PdfError::new(ErrorCode::Malformed, OPERATION)
}
