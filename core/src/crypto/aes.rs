//! AES (FIPS 197), each round four table lookups per column.

use core::hint::black_box;

/// The S-box (FIPS 197, 5.1.1).
static SBOX: [u8; 256] = [
    0x63, 0x7C, 0x77, 0x7B, 0xF2, 0x6B, 0x6F, 0xC5, 0x30, 0x01, 0x67, 0x2B, 0xFE, 0xD7, 0xAB, 0x76,
    0xCA, 0x82, 0xC9, 0x7D, 0xFA, 0x59, 0x47, 0xF0, 0xAD, 0xD4, 0xA2, 0xAF, 0x9C, 0xA4, 0x72, 0xC0,
    0xB7, 0xFD, 0x93, 0x26, 0x36, 0x3F, 0xF7, 0xCC, 0x34, 0xA5, 0xE5, 0xF1, 0x71, 0xD8, 0x31, 0x15,
    0x04, 0xC7, 0x23, 0xC3, 0x18, 0x96, 0x05, 0x9A, 0x07, 0x12, 0x80, 0xE2, 0xEB, 0x27, 0xB2, 0x75,
    0x09, 0x83, 0x2C, 0x1A, 0x1B, 0x6E, 0x5A, 0xA0, 0x52, 0x3B, 0xD6, 0xB3, 0x29, 0xE3, 0x2F, 0x84,
    0x53, 0xD1, 0x00, 0xED, 0x20, 0xFC, 0xB1, 0x5B, 0x6A, 0xCB, 0xBE, 0x39, 0x4A, 0x4C, 0x58, 0xCF,
    0xD0, 0xEF, 0xAA, 0xFB, 0x43, 0x4D, 0x33, 0x85, 0x45, 0xF9, 0x02, 0x7F, 0x50, 0x3C, 0x9F, 0xA8,
    0x51, 0xA3, 0x40, 0x8F, 0x92, 0x9D, 0x38, 0xF5, 0xBC, 0xB6, 0xDA, 0x21, 0x10, 0xFF, 0xF3, 0xD2,
    0xCD, 0x0C, 0x13, 0xEC, 0x5F, 0x97, 0x44, 0x17, 0xC4, 0xA7, 0x7E, 0x3D, 0x64, 0x5D, 0x19, 0x73,
    0x60, 0x81, 0x4F, 0xDC, 0x22, 0x2A, 0x90, 0x88, 0x46, 0xEE, 0xB8, 0x14, 0xDE, 0x5E, 0x0B, 0xDB,
    0xE0, 0x32, 0x3A, 0x0A, 0x49, 0x06, 0x24, 0x5C, 0xC2, 0xD3, 0xAC, 0x62, 0x91, 0x95, 0xE4, 0x79,
    0xE7, 0xC8, 0x37, 0x6D, 0x8D, 0xD5, 0x4E, 0xA9, 0x6C, 0x56, 0xF4, 0xEA, 0x65, 0x7A, 0xAE, 0x08,
    0xBA, 0x78, 0x25, 0x2E, 0x1C, 0xA6, 0xB4, 0xC6, 0xE8, 0xDD, 0x74, 0x1F, 0x4B, 0xBD, 0x8B, 0x8A,
    0x70, 0x3E, 0xB5, 0x66, 0x48, 0x03, 0xF6, 0x0E, 0x61, 0x35, 0x57, 0xB9, 0x86, 0xC1, 0x1D, 0x9E,
    0xE1, 0xF8, 0x98, 0x11, 0x69, 0xD9, 0x8E, 0x94, 0x9B, 0x1E, 0x87, 0xE9, 0xCE, 0x55, 0x28, 0xDF,
    0x8C, 0xA1, 0x89, 0x0D, 0xBF, 0xE6, 0x42, 0x68, 0x41, 0x99, 0x2D, 0x0F, 0xB0, 0x54, 0xBB, 0x16,
];

/// The inverse S-box (FIPS 197, 5.3.2).
static INVERSE_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6A, 0xD5, 0x30, 0x36, 0xA5, 0x38, 0xBF, 0x40, 0xA3, 0x9E, 0x81, 0xF3, 0xD7, 0xFB,
    0x7C, 0xE3, 0x39, 0x82, 0x9B, 0x2F, 0xFF, 0x87, 0x34, 0x8E, 0x43, 0x44, 0xC4, 0xDE, 0xE9, 0xCB,
    0x54, 0x7B, 0x94, 0x32, 0xA6, 0xC2, 0x23, 0x3D, 0xEE, 0x4C, 0x95, 0x0B, 0x42, 0xFA, 0xC3, 0x4E,
    0x08, 0x2E, 0xA1, 0x66, 0x28, 0xD9, 0x24, 0xB2, 0x76, 0x5B, 0xA2, 0x49, 0x6D, 0x8B, 0xD1, 0x25,
    0x72, 0xF8, 0xF6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xD4, 0xA4, 0x5C, 0xCC, 0x5D, 0x65, 0xB6, 0x92,
    0x6C, 0x70, 0x48, 0x50, 0xFD, 0xED, 0xB9, 0xDA, 0x5E, 0x15, 0x46, 0x57, 0xA7, 0x8D, 0x9D, 0x84,
    0x90, 0xD8, 0xAB, 0x00, 0x8C, 0xBC, 0xD3, 0x0A, 0xF7, 0xE4, 0x58, 0x05, 0xB8, 0xB3, 0x45, 0x06,
    0xD0, 0x2C, 0x1E, 0x8F, 0xCA, 0x3F, 0x0F, 0x02, 0xC1, 0xAF, 0xBD, 0x03, 0x01, 0x13, 0x8A, 0x6B,
    0x3A, 0x91, 0x11, 0x41, 0x4F, 0x67, 0xDC, 0xEA, 0x97, 0xF2, 0xCF, 0xCE, 0xF0, 0xB4, 0xE6, 0x73,
    0x96, 0xAC, 0x74, 0x22, 0xE7, 0xAD, 0x35, 0x85, 0xE2, 0xF9, 0x37, 0xE8, 0x1C, 0x75, 0xDF, 0x6E,
    0x47, 0xF1, 0x1A, 0x71, 0x1D, 0x29, 0xC5, 0x89, 0x6F, 0xB7, 0x62, 0x0E, 0xAA, 0x18, 0xBE, 0x1B,
    0xFC, 0x56, 0x3E, 0x4B, 0xC6, 0xD2, 0x79, 0x20, 0x9A, 0xDB, 0xC0, 0xFE, 0x78, 0xCD, 0x5A, 0xF4,
    0x1F, 0xDD, 0xA8, 0x33, 0x88, 0x07, 0xC7, 0x31, 0xB1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xEC, 0x5F,
    0x60, 0x51, 0x7F, 0xA9, 0x19, 0xB5, 0x4A, 0x0D, 0x2D, 0xE5, 0x7A, 0x9F, 0x93, 0xC9, 0x9C, 0xEF,
    0xA0, 0xE0, 0x3B, 0x4D, 0xAE, 0x2A, 0xF5, 0xB0, 0xC8, 0xEB, 0xBB, 0x3C, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2B, 0x04, 0x7E, 0xBA, 0x77, 0xD6, 0x26, 0xE1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0C, 0x7D,
];

/// `SubBytes` then `MixColumns` of one byte, by its row: words with row 0 most significant
/// (FIPS 197, 5.1.1, 5.1.3).
static MIX: [[u32; 256]; 4] = tables(&SBOX, [2, 1, 1, 3]);

/// `InvSubBytes` then `InvMixColumns` of one byte, by its row (FIPS 197, 5.3.2, 5.3.3).
static INVERSE_MIX: [[u32; 256]; 4] = tables(&INVERSE_SBOX, [14, 9, 13, 11]);

/// An AES-128 or AES-256 key expanded into round keys (FIPS 197, 5.2).
#[derive(Clone, Debug)]
pub(crate) struct Aes {
    /// The round keys of the equivalent inverse cipher, in the order used (FIPS 197, 5.3.5).
    inverse: Keys,
    /// Nr, the number of rounds.
    rounds: usize,
    /// The round keys.
    words: Keys,
}

/// Round keys, a power of two of them so an index masked to it needs no check.
type Keys = [[u32; 4]; 16];

impl Aes {
    /// Decrypts `blocks` in place in CBC mode after the ciphertext block `previous`, which
    /// becomes the last of them (FIPS 197, 5.3.5; NIST SP 800-38A, 6.2).
    pub(crate) fn decrypt_cbc(&self, previous: &mut [u8; 16], blocks: &mut [[u8; 16]]) {
        for block in blocks {
            let cipher = *block;
            let keys = (&self.inverse, self.rounds);
            run::<true>(block, &[0; 16], keys, &INVERSE_MIX, &INVERSE_SBOX);
            for (byte, mask) in block.iter_mut().zip(*previous) {
                *byte ^= mask;
            }
            *previous = cipher;
        }
    }

    /// Encrypts `data` in place in CBC mode from `iv`, a partial last block left as is
    /// (FIPS 197, 5.1; NIST SP 800-38A, 6.2).
    pub(crate) fn encrypt_cbc(&self, iv: &[u8; 16], data: &mut [u8]) {
        // On the stack so a sanitizer skips bounds checks: Algorithm 2.B encrypts much.
        let (tables, sbox, words) = black_box((MIX, SBOX, self.words));
        let mut previous = iv;
        for block in data.as_chunks_mut().0 {
            run::<false>(block, previous, (&words, self.rounds), &tables, &sbox);
            previous = block;
        }
    }

    /// Expands a 16- or 32-byte key; `None` for any other length (FIPS 197, 5.2).
    pub(crate) fn new(key: &[u8]) -> Option<Self> {
        let rounds = match key.len() {
            16 => 10,
            32 => 14,
            _ => return None,
        };
        let length = key.len() >> 2_u8;
        let mut words = [0_u32; 60];
        for (word, bytes) in words.iter_mut().zip(key.as_chunks().0) {
            *word = join(*bytes);
        }
        let mut constant = 1;
        for (index, position) in (length..60).zip((0..length).cycle()) {
            let mut temp = words
                .get(index.wrapping_sub(1))
                .copied()
                .unwrap_or_default();
            match (position, length) {
                (0, _) => {
                    temp = substitute(&SBOX, temp.rotate_left(8)) ^ (u32::from(constant) << 24_u8);
                    constant = double(constant);
                }
                (4, 8) => temp = substitute(&SBOX, temp),
                _ => {}
            }
            let back = words
                .get(index.wrapping_sub(length))
                .copied()
                .unwrap_or_default();
            if let Some(word) = words.get_mut(index) {
                *word = temp ^ back;
            }
        }
        let mut keys = Keys::default();
        for (slot, chunk) in keys.iter_mut().zip(words.as_chunks::<4>().0) {
            *slot = *chunk;
        }
        let mut inverse = Keys::default();
        for (round, (target, source)) in inverse
            .iter_mut()
            .zip(keys.get(..=rounds).unwrap_or_default().iter().rev())
            .enumerate()
        {
            *target = if round == 0 || round == rounds {
                *source
            } else {
                source.map(|column| {
                    let word = substitute(&SBOX, column);
                    lookup(&INVERSE_MIX[0], word >> 24_u8)
                        ^ lookup(&INVERSE_MIX[1], word >> 16_u8)
                        ^ lookup(&INVERSE_MIX[2], word >> 8_u8)
                        ^ lookup(&INVERSE_MIX[3], word)
                })
            };
        }
        Some(Self {
            inverse,
            rounds,
            words: keys,
        })
    }
}

/// Multiplies by x in GF(2^8) (FIPS 197, 4.2.1).
const fn double(value: u8) -> u8 {
    (value << 1_u8) ^ if value & 0x80 == 0 { 0 } else { 0x1B }
}

/// The word of four bytes, the first most significant.
fn join(bytes: [u8; 4]) -> u32 {
    bytes
        .into_iter()
        .fold(0, |word, byte| (word << 8_u8) | u32::from(byte))
}

/// The entry of `table` for the low byte of `value`.
fn lookup(table: &[u32; 256], value: u32) -> u32 {
    table
        .get(usize::try_from(value & 0xFF).unwrap_or_default())
        .copied()
        .unwrap_or_default()
}

/// Multiplies in GF(2^8) (FIPS 197, 4.2).
const fn multiply(mut left: u8, mut right: u8) -> u8 {
    let mut product = 0;
    while right != 0 {
        if right & 1 != 0 {
            product ^= left;
        }
        left = double(left);
        right >>= 1_u8;
    }
    product
}

/// One round's `ShiftRows` or `InvShiftRows`, `SubBytes` or `InvSubBytes` and `MixColumns` or
/// `InvMixColumns` through `tables`, then `AddRoundKey` (FIPS 197, 5.1, 5.3.5).
fn round(tables: &[[u32; 256]; 4], sources: [[u32; 4]; 4], key: [u32; 4]) -> [u32; 4] {
    let column = |[w0, w1, w2, w3]: [u32; 4], round_key: u32| {
        lookup(&tables[0], w0 >> 24_u8)
            ^ lookup(&tables[1], w1 >> 16_u8)
            ^ lookup(&tables[2], w2 >> 8_u8)
            ^ lookup(&tables[3], w3)
            ^ round_key
    };
    [
        column(sources[0], key[0]),
        column(sources[1], key[1]),
        column(sources[2], key[2]),
        column(sources[3], key[3]),
    ]
}

/// Runs the cipher, or the equivalent inverse cipher when `INVERSE`, on `block` XOR `mask` with
/// the round keys `keys`, their number less one Nr (FIPS 197, 5.1, 5.3.5).
fn run<const INVERSE: bool>(
    block: &mut [u8; 16],
    mask: &[u8; 16],
    (keys, rounds): (&Keys, usize),
    tables: &[[u32; 256]; 4],
    sbox: &[u8; 256],
) {
    let key = |index: usize| keys.get(index & 15).copied().unwrap_or_default();
    let mut input = *block;
    for (byte, masking) in input.iter_mut().zip(mask) {
        *byte ^= masking;
    }
    let (&[b0, b1, b2, b3], first) = (input.as_chunks().0, key(0)) else {
        return;
    };
    let mut state = [
        join(b0) ^ first[0],
        join(b1) ^ first[1],
        join(b2) ^ first[2],
        join(b3) ^ first[3],
    ];
    for index in 1..rounds {
        state = round(tables, shifted::<INVERSE>(state), key(index));
    }
    let last = key(rounds);
    let substituted = |byte: u8| sbox.get(usize::from(byte)).copied().unwrap_or_default();
    let column = |[w0, w1, w2, w3]: [u32; 4], round_key: u32| {
        let [r0, ..] = split(w0);
        let [_, r1, ..] = split(w1);
        let [_, _, r2, _] = split(w2);
        let [.., r3] = split(w3);
        split(
            join([
                substituted(r0),
                substituted(r1),
                substituted(r2),
                substituted(r3),
            ]) ^ round_key,
        )
    };
    let sources = shifted::<INVERSE>(state);
    let out = [
        column(sources[0], last[0]),
        column(sources[1], last[1]),
        column(sources[2], last[2]),
        column(sources[3], last[3]),
    ];
    *block = *out.as_flattened().as_array().unwrap_or(&[0; 16]);
}

/// For each column of `state` after `ShiftRows`, or `InvShiftRows` when `INVERSE`, the columns
/// its rows 0 to 3 come from (FIPS 197, 5.1.2, 5.3.1).
const fn shifted<const INVERSE: bool>([s0, s1, s2, s3]: [u32; 4]) -> [[u32; 4]; 4] {
    if INVERSE {
        [
            [s0, s3, s2, s1],
            [s1, s0, s3, s2],
            [s2, s1, s0, s3],
            [s3, s2, s1, s0],
        ]
    } else {
        [
            [s0, s1, s2, s3],
            [s1, s2, s3, s0],
            [s2, s3, s0, s1],
            [s3, s0, s1, s2],
        ]
    }
}

/// The four bytes of `word`, the most significant first.
fn split(word: u32) -> [u8; 4] {
    [24_u8, 16, 8, 0].map(|shift| u8::try_from((word >> shift) & 0xFF).unwrap_or_default())
}

/// Replaces each byte by its entry in `sbox` (FIPS 197, 5.1.1, 5.3.2).
fn substitute(sbox: &[u8; 256], word: u32) -> u32 {
    join(split(word).map(|byte| sbox.get(usize::from(byte)).copied().unwrap_or_default()))
}

/// Each byte through `sbox`, times the column `column`, as words, then rotated right a byte per
/// row (FIPS 197, 4.3).
const fn tables(sbox: &[u8; 256], column: [u8; 4]) -> [[u32; 256]; 4] {
    let mut tables = [[0; 256]; 4];
    let mut rows: &mut [[u32; 256]] = &mut tables;
    let mut rotation = 0_u32;
    while let Some((table, others)) = rows.split_first_mut() {
        let mut entries: &mut [u32] = table;
        let mut values: &[u8] = sbox;
        while let (Some((entry, rest)), Some((value, more))) =
            (entries.split_first_mut(), values.split_first())
        {
            let mut word = 0_u32;
            let mut factors: &[u8] = &column;
            while let Some((factor, after)) = factors.split_first() {
                word = (word << 8_u8) | widen(multiply(*value, *factor));
                factors = after;
            }
            *entry = word.rotate_right(rotation);
            entries = rest;
            values = more;
        }
        rows = others;
        rotation = rotation.wrapping_add(8);
    }
    tables
}

/// `byte` as a `u32`, in a constant.
const fn widen(byte: u8) -> u32 {
    let mut wide = 0_u32;
    let mut weight = 1_u32;
    let mut left = byte;
    while left != 0 {
        if left & 1 != 0 {
            wide |= weight;
        }
        weight <<= 1_u8;
        left >>= 1_u8;
    }
    wide
}
