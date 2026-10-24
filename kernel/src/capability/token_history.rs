//! Exact non-reassignment within one kernel broker lifetime, including tokens
//! burned by failed preparations. This is bounded retention, not cryptography.
//! Reboot resets it; replacing the global broker during a boot is forbidden.
use super::CapError;
use alloc::vec::Vec;

// At most 512 KiB of token words (3.125% of the current 16 MiB kernel heap).
// Exhaustion permanently refuses fresh issuance; existing authority can still
// be consumed/retired and existing exact-rights derivations can be reused.
pub(super) const MAX_PREPARED_TOKENS: usize = 65_536;
const RESERVE_CHUNK: usize = 256;

pub(super) struct TokenHistory {
    words: Vec<u64>,
}

impl TokenHistory {
    pub const fn new() -> Self {
        Self { words: Vec::new() }
    }

    pub fn contains(&self, word: u64) -> bool {
        self.words.binary_search(&word).is_ok()
    }

    pub fn reserve(&mut self) -> Result<(), CapError> {
        if self.words.len() >= MAX_PREPARED_TOKENS {
            return Err(CapError::TokenHistoryFull);
        }
        if self.words.len() == self.words.capacity() {
            self.words
                .try_reserve_exact(RESERVE_CHUNK.min(MAX_PREPARED_TOKENS - self.words.len()))
                .map_err(|_| CapError::CapabilityStoreFull)?;
        }
        Ok(())
    }

    pub fn record(&mut self, word: u64) {
        // The exclusive broker borrow spans reserve, lookup and insertion.
        // No allocation is permitted here; at most 65,535 words are moved.
        assert!(self.words.len() < self.words.capacity());
        assert!(self.words.len() < MAX_PREPARED_TOKENS);
        let index = self
            .words
            .binary_search(&word)
            .expect_err("duplicate token identity");
        self.words.insert(index, word);
    }

    #[cfg(any(
        test,
        feature = "production_grant_test",
        feature = "cap_issuance_test",
        feature = "native_session_race_test"
    ))]
    pub fn usage(&self) -> (usize, usize) {
        (self.words.len(), self.words.capacity())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_history_fails_closed_without_evicting_old_identity() {
        let mut history = TokenHistory::new();
        for word in 1..=MAX_PREPARED_TOKENS as u64 {
            history.reserve().unwrap();
            history.record(word);
        }
        assert_eq!(history.reserve(), Err(CapError::TokenHistoryFull));
        assert!(history.contains(1));
        assert!(history.contains(MAX_PREPARED_TOKENS as u64));
        assert!(!history.contains(MAX_PREPARED_TOKENS as u64 + 1));
        assert_eq!(history.usage(), (MAX_PREPARED_TOKENS, MAX_PREPARED_TOKENS));
    }
}
