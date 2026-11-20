//! MD5 (RFC 1321).

/// The sine table (RFC 1321, 3.4).
const SINES: [u32; 64] = [
    0xD76A_A478,
    0xE8C7_B756,
    0x2420_70DB,
    0xC1BD_CEEE,
    0xF57C_0FAF,
    0x4787_C62A,
    0xA830_4613,
    0xFD46_9501,
    0x6980_98D8,
    0x8B44_F7AF,
    0xFFFF_5BB1,
    0x895C_D7BE,
    0x6B90_1122,
    0xFD98_7193,
    0xA679_438E,
    0x49B4_0821,
    0xF61E_2562,
    0xC040_B340,
    0x265E_5A51,
    0xE9B6_C7AA,
    0xD62F_105D,
    0x0244_1453,
    0xD8A1_E681,
    0xE7D3_FBC8,
    0x21E1_CDE6,
    0xC337_07D6,
    0xF4D5_0D87,
    0x455A_14ED,
    0xA9E3_E905,
    0xFCEF_A3F8,
    0x676F_02D9,
    0x8D2A_4C8A,
    0xFFFA_3942,
    0x8771_F681,
    0x6D9D_6122,
    0xFDE5_380C,
    0xA4BE_EA44,
    0x4BDE_CFA9,
    0xF6BB_4B60,
    0xBEBF_BC70,
    0x289B_7EC6,
    0xEAA1_27FA,
    0xD4EF_3085,
    0x0488_1D05,
    0xD9D4_D039,
    0xE6DB_99E5,
    0x1FA2_7CF8,
    0xC4AC_5665,
    0xF429_2244,
    0x432A_FF97,
    0xAB94_23A7,
    0xFC93_A039,
    0x655B_59C3,
    0x8F0C_CC92,
    0xFFEF_F47D,
    0x8584_5DD1,
    0x6FA8_7E4F,
    0xFE2C_E6E0,
    0xA301_4314,
    0x4E08_11A1,
    0xF753_7E82,
    0xBD3A_F235,
    0x2AD7_D2BB,
    0xEB86_D391,
];

/// The shift amounts of each round's steps (RFC 1321, 3.4).
const SHIFTS: [[u32; 4]; 4] = [
    [7, 12, 17, 22],
    [5, 9, 14, 20],
    [4, 11, 16, 23],
    [6, 10, 15, 21],
];

/// An MD5 digest being computed (RFC 1321, 3).
#[derive(Clone, Debug)]
pub(crate) struct Md5 {
    /// The current block.
    buffer: [u8; 64],
    /// Bytes hashed so far.
    length: u64,
    /// The buffer A, B, C, D (RFC 1321, 3.3).
    state: [u32; 4],
}

impl Default for Md5 {
    fn default() -> Self {
        Self {
            buffer: [0; 64],
            length: 0,
            state: [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476],
        }
    }
}

impl Md5 {
    /// Processes the full buffer (RFC 1321, 3.4).
    fn compress(&mut self) {
        let mut words = [0_u32; 16];
        for (word, bytes) in words.iter_mut().zip(self.buffer.as_chunks::<4>().0) {
            *word = bytes
                .iter()
                .rev()
                .fold(0, |acc, &byte| (acc << 8_u8) | u32::from(byte));
        }
        let mut work = self.state;
        for (step, sine) in SINES.into_iter().enumerate() {
            let [first, second, third, fourth] = work;
            let (mix, index) = match step >> 4_u8 {
                0 => ((second & third) | (!second & fourth), step),
                1 => (
                    (fourth & second) | (!fourth & third),
                    step.wrapping_mul(5).wrapping_add(1),
                ),
                2 => (
                    second ^ third ^ fourth,
                    step.wrapping_mul(3).wrapping_add(5),
                ),
                _ => (third ^ (second | !fourth), step.wrapping_mul(7)),
            };
            let shift = SHIFTS.get(step >> 4_u8).and_then(|row| row.get(step & 3));
            work.rotate_right(1);
            work[1] = first
                .wrapping_add(mix)
                .wrapping_add(sine)
                .wrapping_add(words.get(index & 15).copied().unwrap_or_default())
                .rotate_left(shift.copied().unwrap_or_default())
                .wrapping_add(second);
        }
        for (value, add) in self.state.iter_mut().zip(work) {
            *value = value.wrapping_add(add);
        }
    }

    /// The digest of the bytes given (RFC 1321, 3.5).
    pub(crate) fn finish(mut self) -> [u8; 16] {
        let mut length = [0; 8];
        put(&mut length, self.length.wrapping_shl(3));
        self.update(&[0x80]);
        while self.length & 63 != 56 {
            self.update(&[0]);
        }
        self.update(&length);
        let mut digest = [0; 16];
        for (bytes, word) in digest.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            put(bytes, word.into());
        }
        digest
    }

    /// Hashes more bytes.
    pub(crate) fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let position = usize::try_from(self.length & 63).unwrap_or_default();
            if let Some(slot) = self.buffer.get_mut(position) {
                *slot = byte;
            }
            self.length = self.length.wrapping_add(1);
            if position == 63 {
                self.compress();
            }
        }
    }
}

/// Writes `value`'s low bytes to `out`, least significant first.
fn put(out: &mut [u8], mut value: u64) {
    for byte in out {
        *byte = u8::try_from(value & 0xFF).unwrap_or_default();
        value >>= 8_u8;
    }
}
