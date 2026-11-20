//! RC4, the stream cipher of RFC 6229's vectors.

/// An RC4 keystream.
#[derive(Clone, Debug)]
pub(crate) struct Rc4 {
    /// The second index.
    jump: u8,
    /// The first index.
    next: u8,
    /// The permutation of all byte values.
    state: [u8; 256],
}

impl Rc4 {
    /// XORs the next keystream bytes into `data`.
    pub(crate) fn apply(&mut self, data: &mut [u8]) {
        for byte in data {
            self.next = self.next.wrapping_add(1);
            self.jump = self.jump.wrapping_add(self.at(self.next));
            self.state.swap(self.next.into(), self.jump.into());
            *byte ^= self.at(self.at(self.next).wrapping_add(self.at(self.jump)));
        }
    }

    /// The permutation's byte at `index`.
    fn at(&self, index: u8) -> u8 {
        self.state
            .get(usize::from(index))
            .copied()
            .unwrap_or_default()
    }

    /// Schedules a 1- to 256-byte key; `None` for any other length.
    pub(crate) fn new(key: &[u8]) -> Option<Self> {
        if key.is_empty() || key.len() > 256 {
            return None;
        }
        let mut rc4 = Self {
            jump: 0,
            next: 0,
            state: [0; 256],
        };
        for (slot, value) in rc4.state.iter_mut().zip(0..=255) {
            *slot = value;
        }
        let mut jump = 0_u8;
        for (index, &key_byte) in (0..=255).zip(key.iter().cycle()) {
            jump = jump.wrapping_add(rc4.at(index)).wrapping_add(key_byte);
            rc4.state.swap(index.into(), jump.into());
        }
        Some(rc4)
    }
}
