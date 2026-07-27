//! Fast fingerprints and stable non-cryptographic hashes.
//!
//! # Content identity vs token identity
//!
//! - **Final document content identity** uses [`crate::digest::ContentDigest`]
//!   (SHA-256), never FNV-1a64 alone.
//! - **FNV-1a64** remains for:
//!   - cheap path fingerprinting
//!   - scan-table hash buckets
//!   - optional metadata prefilter (`FastFingerprint`)
//!   - token IDs when collision verification remains active
//!
//! A matching FNV value must not suppress strong-digest verification when
//! metadata indicates possible change.
//!
//! Reuses the same non-randomized FNV family as `wiseowl-memorydb` payload
//! hashes. **Never** use Rust's `DefaultHasher` for persistent identity.

/// FNV-1a 64-bit (IEEE FNV offset basis + prime). Stable across process restarts.
pub fn fnv1a64(data: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x1000_0000_01b3;
    let mut hash = OFFSET;
    for &b in data {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Streaming FNV-1a hasher for bounded prefilter fingerprints.
#[derive(Debug, Clone)]
pub struct Fnv1a64Hasher {
    state: u64,
    bytes: u64,
}

impl Default for Fnv1a64Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fnv1a64Hasher {
    pub const fn new() -> Self {
        Self {
            state: 0xcbf2_9ce4_8422_2325,
            bytes: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        const PRIME: u64 = 0x1000_0000_01b3;
        for &b in data {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(PRIME);
        }
        self.bytes = self.bytes.saturating_add(data.len() as u64);
    }

    pub fn finish(self) -> u64 {
        self.state
    }

    pub fn bytes_hashed(&self) -> u64 {
        self.bytes
    }
}

/// Historical Phase 3 alias: FNV content hash (no longer final identity).
#[deprecated(note = "use ContentDigest for final identity; FastFingerprint for prefilter")]
pub type ContentHash = u64;

/// Stable path hash (FNV of canonical relative path bytes).
pub type StablePathHash = u64;

/// Fast prefilter fingerprint (FNV). Not final content identity.
pub type FastFingerprint = crate::digest::FastFingerprint;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_matches_offset() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn streaming_matches_one_shot() {
        let data = b"hello wise owl phase 3";
        let mut h = Fnv1a64Hasher::new();
        h.update(&data[..5]);
        h.update(&data[5..]);
        assert_eq!(h.finish(), fnv1a64(data));
    }

    #[test]
    fn different_inputs_differ() {
        assert_ne!(fnv1a64(b"a"), fnv1a64(b"b"));
    }
}
