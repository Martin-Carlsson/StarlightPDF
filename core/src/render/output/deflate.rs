//! Deflate with fixed Huffman codes (RFC 1951, 3.2.6) in zlib's wrapper (RFC 1950).

use crate::render::output::{lowest, word};

/// The modulus of Adler-32 (RFC 1950, 8.2).
const ADLER: u32 = 0xFFF1;

/// The bits that index the hash table.
const HASH: u32 = 15;

/// The longest match (3.2.5).
const LONGEST: usize = 258;

/// The bytes a match is sought by, and its shortest.
const SHORTEST: usize = 4;

/// How far the window slides.
const SLID: u16 = 0x8000;

/// The most bytes Adler-32's sums take before they are reduced.
const SUMMED: usize = 5552;

/// The farthest a match reaches back (3.2).
const WINDOW: usize = 0x8000;

/// A zlib header: deflate, a 32 KiB window, the fastest algorithm (RFC 1950, 2.2).
const ZLIB: [u8; 2] = [0x78, 0x01];

/// Bytes being compressed as one block of fixed codes.
pub(super) struct Deflate {
    /// Adler-32's two sums.
    adler: (u32, u32),
    /// Bits not yet written, the first lowest.
    bits: u64,
    /// How many `bits` holds.
    count: u32,
    /// The first byte of `held` not compressed.
    done: usize,
    /// The last place in `held` of each hash.
    head: Vec<u16>,
    /// The window, then bytes not compressed.
    held: Vec<u8>,
}

impl Deflate {
    /// Compresses the bytes held.
    fn compress(&mut self, out: &mut Vec<u8>) {
        let mut at = self.done;
        while let Some(&byte) = self.held.get(at) {
            let found = self.remember(at).and_then(|near| self.matched(at, near));
            if let Some((length, distance)) = found {
                self.copy(length, distance, out);
                at = at.saturating_add(length);
                // So that a run goes on at distance 1.
                let _: Option<usize> = self.remember(at.saturating_sub(1));
            } else {
                self.symbol(u32::from(byte), out);
                at = at.saturating_add(1);
            }
        }
        self.done = at;
    }

    /// Writes a match (3.2.5).
    fn copy(&mut self, length: usize, distance: usize, out: &mut Vec<u8>) {
        let over = u32::try_from(length.saturating_sub(3)).unwrap_or_default();
        let (symbol, extra) = if length == LONGEST {
            (285, 0)
        } else if over < 8 {
            (over.saturating_add(257), 0)
        } else {
            let width = over.ilog2().saturating_sub(2);
            let group = width.saturating_mul(4).saturating_add(257);
            (group.saturating_add(over.wrapping_shr(width)), width)
        };
        self.symbol(symbol, out);
        self.push(lowest(over, extra), extra, out);
        let back = u32::try_from(distance.saturating_sub(1)).unwrap_or_default();
        let (code, more) = if back < 4 {
            (back, 0)
        } else {
            let width = back.ilog2().saturating_sub(1);
            let group = width.saturating_mul(2);
            (group.saturating_add(back.wrapping_shr(width)), width)
        };
        self.push(code.reverse_bits().wrapping_shr(27), 5, out);
        self.push(lowest(back, more), more, out);
    }

    /// Ends the stream.
    pub(super) fn finish(&mut self, out: &mut Vec<u8>) {
        self.compress(out);
        self.symbol(256, out);
        let padding = self.count.wrapping_neg() & 7;
        self.push(0, padding, out);
        out.extend_from_slice(&word(self.adler.1.wrapping_shl(16) | self.adler.0));
    }

    /// The match at `at` with `near` before it: its length and distance.
    fn matched(&self, at: usize, near: usize) -> Option<(usize, usize)> {
        let distance = at.checked_sub(near)?;
        if !(1..=WINDOW).contains(&distance) {
            return None;
        }
        let ahead = self.held.get(at..)?;
        let behind = self.held.get(near..)?;
        let most = ahead.len().min(LONGEST);
        let length = common(ahead.get(..most)?, behind.get(..most)?);
        (length >= SHORTEST).then_some((length, distance))
    }

    /// A stream begun in `out`.
    pub(super) fn new(out: &mut Vec<u8>) -> Self {
        out.extend_from_slice(&ZLIB);
        Self {
            adler: (1, 0),
            // The final block, of fixed codes (3.2.3).
            bits: 3,
            count: 3,
            done: 0,
            head: vec![0; 1_usize << HASH],
            held: Vec::with_capacity(WINDOW.saturating_mul(2)),
        }
    }

    /// Writes the `width` bits of `value`, lowest first.
    fn push(&mut self, value: u32, width: u32, out: &mut Vec<u8>) {
        self.bits |= u64::from(value).wrapping_shl(self.count);
        self.count = self.count.saturating_add(width);
        while self.count >= 8 {
            out.push(u8::try_from(self.bits & 0xFF).unwrap_or_default());
            self.bits = self.bits.wrapping_shr(8);
            self.count = self.count.saturating_sub(8);
        }
    }

    /// Makes `at` the last place of its hash; the place before.
    fn remember(&mut self, at: usize) -> Option<usize> {
        let &key = self.held.get(at..)?.first_chunk::<SHORTEST>()?;
        let slot = self.head.get_mut(hash(key))?;
        let before = usize::from(*slot);
        *slot = u16::try_from(at).ok()?;
        Some(before)
    }

    /// Writes a literal, a length's symbol or the end of the block (3.2.6).
    fn symbol(&mut self, symbol: u32, out: &mut Vec<u8>) {
        let (code, width) = match symbol {
            0..=143 => (symbol.saturating_add(0x30), 8_u32),
            144..=255 => (symbol.saturating_add(0x100), 9),
            256..=279 => (symbol.saturating_sub(256), 7),
            _ => (symbol.saturating_sub(88), 8),
        };
        let shift = 32_u32.saturating_sub(width);
        self.push(code.reverse_bits().wrapping_shr(shift), width, out);
    }

    /// Takes `data`, compressing each time the buffer fills.
    pub(super) fn write(&mut self, data: &[u8], out: &mut Vec<u8>) {
        for part in data.chunks(SUMMED) {
            let (mut first, mut second) = self.adler;
            for &byte in part {
                first = first.wrapping_add(u32::from(byte));
                second = second.wrapping_add(first);
            }
            self.adler = (
                first.checked_rem(ADLER).unwrap_or_default(),
                second.checked_rem(ADLER).unwrap_or_default(),
            );
        }
        let full = WINDOW.saturating_mul(2);
        let mut rest = data;
        while !rest.is_empty() {
            let room = full.saturating_sub(self.held.len()).min(rest.len());
            let (taken, later) = rest.split_at_checked(room).unwrap_or_default();
            self.held.extend_from_slice(taken);
            rest = later;
            if self.held.len() >= full {
                self.compress(out);
                self.held.copy_within(WINDOW.., 0);
                self.held.truncate(WINDOW);
                self.done = WINDOW;
                for place in &mut self.head {
                    *place = place.saturating_sub(SLID);
                }
            }
        }
    }
}

/// How many leading bytes the two share.
fn common(left: &[u8], right: &[u8]) -> usize {
    let (wide, narrow) = (left.as_chunks::<16>().0, right.as_chunks::<16>().0);
    let same = wide
        .iter()
        .zip(narrow)
        .take_while(|&(one, other)| one == other)
        .count()
        .saturating_mul(16);
    let rest = left.iter().zip(right).skip(same);
    same.saturating_add(rest.take_while(|&(one, other)| one == other).count())
}

/// The hash of `key`.
fn hash(key: [u8; SHORTEST]) -> usize {
    let value = key
        .iter()
        .fold(0_u32, |sum, byte| sum.wrapping_shl(8) | u32::from(*byte));
    let mixed = value.wrapping_mul(0x9E37_79B1);
    usize::try_from(mixed.wrapping_shr(32_u32.saturating_sub(HASH))).unwrap_or_default()
}
