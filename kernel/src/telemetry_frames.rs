pub(crate) struct FrameScratch<const WORDS: usize, const DIRTY_WORDS: usize> {
    seen: [u64; WORDS],
    shared: [u64; WORDS],
    local: [u64; WORDS],
    dirty: [u64; DIRTY_WORDS],
    external: [u64; 4096],
    external_used: bool,
}

impl<const WORDS: usize, const DIRTY_WORDS: usize> FrameScratch<WORDS, DIRTY_WORDS> {
    pub(crate) const fn new() -> Self {
        Self {
            seen: [0; WORDS],
            shared: [0; WORDS],
            local: [0; WORDS],
            dirty: [0; DIRTY_WORDS],
            external: [0; 4096],
            external_used: false,
        }
    }

    pub(crate) fn reset_sample(&mut self) {
        self.seen.fill(0);
        self.shared.fill(0);
        self.reset_space();
    }

    pub(crate) fn reset_space(&mut self) {
        for (index, dirty) in self.dirty.iter_mut().enumerate() {
            while *dirty != 0 {
                let bit = dirty.trailing_zeros() as usize;
                self.local[index * 64 + bit] = 0;
                *dirty &= *dirty - 1;
            }
        }
        if self.external_used {
            self.external.fill(0);
            self.external_used = false;
        }
    }

    pub(crate) fn first_external_in_space(&mut self, frame: usize) -> Option<bool> {
        debug_assert!(frame >= WORDS * 64);
        self.external_used = true;
        let key = frame as u64;
        let hash = (key ^ (key >> 12)).wrapping_mul(0x9e3779b97f4a7c15);
        for probe in 0..32 {
            let index = ((hash >> 52) as usize + probe) % self.external.len();
            let slot = &mut self.external[index];
            if *slot == key {
                return Some(false);
            }
            if *slot == 0 {
                *slot = key;
                return Some(true);
            }
        }
        None
    }

    pub(crate) fn classify_word(
        &mut self,
        word: usize,
        present: u64,
        private: u64,
        multiple_groups: bool,
    ) -> (u64, u64, u64, bool) {
        let fresh = self.mark_word(word, present);
        let private = fresh & private;
        let earlier = private & self.seen[word] & !self.shared[word];
        let shared = if multiple_groups {
            private
        } else {
            private & self.seen[word]
        };
        let transferred = shared & !self.shared[word];
        self.seen[word] |= private;
        self.shared[word] |= shared;
        let local_private = private & !self.shared[word];
        (
            u64::from(local_private.count_ones()),
            u64::from((fresh ^ local_private).count_ones()),
            u64::from(transferred.count_ones()),
            earlier != 0,
        )
    }

    pub(crate) fn account_word(&mut self, word: usize, present: u64) -> (u64, u64) {
        let fresh = self.mark_word(word, present);
        let private = fresh & self.seen[word] & !self.shared[word];
        (
            u64::from(private.count_ones()),
            u64::from((fresh ^ private).count_ones()),
        )
    }

    fn mark_word(&mut self, word: usize, mask: u64) -> u64 {
        let old = self.local[word];
        if old == 0 && mask != 0 {
            self.dirty[word / 64] |= 1 << (word % 64);
        }
        self.local[word] = old | mask;
        mask & !old
    }
}
