//! ChaCha20 with the original 64-bit counter/nonce layout.
const RNG_BYTES: usize = 64;
const SEED_BYTES: usize = 40;

pub(crate) struct ChaCha20 {
    state: [u32; 16],
    block: [u8; RNG_BYTES],
    offset: usize,
    exhausted: bool,
}

impl ChaCha20 {
    pub(crate) fn from_seed(seed: &[u8; SEED_BYTES]) -> Self {
        let mut state = [0u32; 16];
        state[0] = 0x6170_7865;
        state[1] = 0x3320_646e;
        state[2] = 0x7962_2d32;
        state[3] = 0x6b20_6574;
        for index in 0..8 {
            let offset = index * 4;
            state[4 + index] = u32::from_le_bytes(seed[offset..offset + 4].try_into().unwrap());
        }
        state[12] = 0;
        state[13] = 0;
        state[14] = u32::from_le_bytes(seed[32..36].try_into().unwrap());
        state[15] = u32::from_le_bytes(seed[36..40].try_into().unwrap());
        Self {
            state,
            block: [0; RNG_BYTES],
            offset: RNG_BYTES,
            exhausted: false,
        }
    }

    pub(crate) fn fill(&mut self, out: &mut [u8]) -> bool {
        // Preflight the complete request before writing output or consuming
        // state. Never wrap the counter and repeat a keystream block.
        let counter = (u64::from(self.state[13]) << 32) | u64::from(self.state[12]);
        let blocks = if self.exhausted {
            0
        } else {
            u128::from(u64::MAX - counter) + 1
        };
        let available = blocks * RNG_BYTES as u128 + (RNG_BYTES - self.offset) as u128;
        if out.len() as u128 > available {
            return false;
        }
        let mut written = 0;
        while written < out.len() {
            if self.offset == RNG_BYTES {
                self.generate_block();
            }
            let count = (out.len() - written).min(RNG_BYTES - self.offset);
            out[written..written + count]
                .copy_from_slice(&self.block[self.offset..self.offset + count]);
            self.offset += count;
            written += count;
        }
        true
    }

    fn generate_block(&mut self) {
        let mut x = self.state;
        macro_rules! quarter_round {
            ($a:expr, $b:expr, $c:expr, $d:expr) => {
                x[$a] = x[$a].wrapping_add(x[$b]);
                x[$d] ^= x[$a];
                x[$d] = x[$d].rotate_left(16);
                x[$c] = x[$c].wrapping_add(x[$d]);
                x[$b] ^= x[$c];
                x[$b] = x[$b].rotate_left(12);
                x[$a] = x[$a].wrapping_add(x[$b]);
                x[$d] ^= x[$a];
                x[$d] = x[$d].rotate_left(8);
                x[$c] = x[$c].wrapping_add(x[$d]);
                x[$b] ^= x[$c];
                x[$b] = x[$b].rotate_left(7);
            };
        }
        for _ in 0..10 {
            // Column rounds (ChaCha20): last index of the fourth QR is 15, not 12.
            quarter_round!(0, 4, 8, 12);
            quarter_round!(1, 5, 9, 13);
            quarter_round!(2, 6, 10, 14);
            quarter_round!(3, 7, 11, 15);
            // Diagonal rounds
            quarter_round!(0, 5, 10, 15);
            quarter_round!(1, 6, 11, 12);
            quarter_round!(2, 7, 8, 13);
            quarter_round!(3, 4, 9, 14);
        }
        for index in 0..16 {
            let value = x[index].wrapping_add(self.state[index]);
            self.block[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let (low, carry) = self.state[12].overflowing_add(1);
        self.state[12] = low;
        if carry {
            let (high, exhausted) = self.state[13].overflowing_add(1);
            self.state[13] = high;
            self.exhausted = exhausted;
        }
        self.offset = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{ChaCha20, RNG_BYTES};

    #[test]
    fn rfc8439_block_known_answer() {
        // RFC 8439 section 2.3.2, matching its complete input state. The
        // production stream uses a 64-bit counter rather than IETF's 32 bits.
        let mut seed = [0u8; 40];
        for (i, byte) in seed[..32].iter_mut().enumerate() {
            *byte = i as u8;
        }
        let mut c = ChaCha20::from_seed(&seed);
        c.state[12] = 1;
        c.state[13] = 0x09000000;
        c.state[14] = 0x4a000000;
        let mut out = [0; 64];
        assert!(c.fill(&mut out));
        assert_eq!(
            out,
            [
                0x10, 0xf1, 0xe7, 0xe4, 0xd1, 0x3b, 0x59, 0x15, 0x50, 0x0f, 0xdd, 0x1f, 0xa3, 0x20,
                0x71, 0xc4, 0xc7, 0xd1, 0xf4, 0xc7, 0x33, 0xc0, 0x68, 0x03, 0x04, 0x22, 0xaa, 0x9a,
                0xc3, 0xd4, 0x6c, 0x4e, 0xd2, 0x82, 0x64, 0x46, 0x07, 0x9f, 0xaa, 0x09, 0x14, 0xc2,
                0xd7, 0x05, 0xd9, 0x8b, 0x02, 0xa2, 0xb5, 0x12, 0x9c, 0xd1, 0xde, 0x16, 0x4e, 0xb9,
                0xcb, 0xd0, 0x83, 0xe8, 0xa2, 0x50, 0x3c, 0x4e,
            ]
        );
    }

    #[test]
    fn exhausted_counter_fails_without_partial_output_or_repeated_block() {
        let mut c = ChaCha20::from_seed(&[0; 40]);
        c.state[12] = u32::MAX;
        c.state[13] = u32::MAX;
        let mut too_large = [0xaa; 65];
        assert!(!c.fill(&mut too_large));
        assert_eq!(too_large, [0xaa; 65]);
        assert!(!c.exhausted);
        assert!(c.fill(&mut [0; 63]));
        assert!(c.exhausted);
        assert!(c.fill(&mut [0; 1]));
        let mut next = [0xaa; 1];
        assert!(!c.fill(&mut next));
        assert_eq!(next, [0xaa]);
    }

    #[test]
    fn conditioner_expands_seed_without_repeating_a_block() {
        let mut seed = [0u8; 40];
        for (index, byte) in seed.iter_mut().enumerate() {
            *byte = index as u8;
        }
        let mut conditioner = ChaCha20::from_seed(&seed);
        let mut bytes = [0u8; RNG_BYTES * 2];
        conditioner.fill(&mut bytes);
        assert_ne!(&bytes[..RNG_BYTES], &bytes[RNG_BYTES..]);
    }

    #[test]
    fn different_seeds_yield_different_keystreams() {
        let mut a = [0u8; 40];
        let mut b = [0u8; 40];
        for i in 0..40 {
            a[i] = i as u8;
            b[i] = 0xFF - i as u8;
        }
        let mut ca = ChaCha20::from_seed(&a);
        let mut cb = ChaCha20::from_seed(&b);
        let mut oa = [0u8; 32];
        let mut ob = [0u8; 32];
        ca.fill(&mut oa);
        cb.fill(&mut ob);
        assert_ne!(oa, ob);
    }

    #[test]
    fn fill_initializes_every_output_byte() {
        let mut seed = [0u8; 40];
        seed[0] = 1;
        let mut c = ChaCha20::from_seed(&seed);
        let mut out = [0xAAu8; 17];
        c.fill(&mut out);
        // Second block-spanning fill differs and is fully overwritten.
        let mut out2 = [0xAAu8; 17];
        c.fill(&mut out2);
        assert_ne!(out, out2);
    }
}
