//! Stable token dictionary with collision detection.
//!
//! Token IDs are FNV-1a64 of (tokenizer_id, tokenizer_version, canonical_utf8).
//! Collisions between different canonical strings are detected and rejected
//! (never silently merged).

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};

use crate::error::IndexError;
use crate::hash::fnv1a64;
use crate::quotas::IndexQuotaConfig;

/// Dictionary entry for collision verification.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct TokenDictionaryEntry {
    pub tokenizer_id: u32,
    pub tokenizer_version: u32,
    pub token_id: u64,
    pub canonical_token_hash: u64,
    pub canonical_token: Option<String>,
}

/// In-memory token dictionary (bounded).
#[derive(Debug, Clone, Default)]
pub struct TokenDictionary {
    /// token_id → entry
    by_id: BTreeMap<u64, TokenDictionaryEntry>,
    /// (tokenizer_id, version, canonical) → token_id for fast path
    by_canonical: BTreeMap<(u32, u32, String), u64>,
    pub collisions_detected: u64,
}

impl TokenDictionary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Compute stable candidate token id (not yet collision-checked).
    pub fn candidate_id(tokenizer_id: u32, tokenizer_version: u32, canonical: &str) -> u64 {
        let mut buf = alloc::vec::Vec::with_capacity(8 + canonical.len());
        buf.extend_from_slice(&tokenizer_id.to_le_bytes());
        buf.extend_from_slice(&tokenizer_version.to_le_bytes());
        buf.extend_from_slice(canonical.as_bytes());
        let id = fnv1a64(&buf);
        // Reserve 0 as invalid.
        if id == 0 {
            1
        } else {
            id
        }
    }

    /// Intern a canonical token; detect collisions.
    pub fn intern(
        &mut self,
        tokenizer_id: u32,
        tokenizer_version: u32,
        canonical: &str,
        quotas: &IndexQuotaConfig,
        store_text: bool,
    ) -> Result<u64, IndexError> {
        if canonical.is_empty() {
            return Err(IndexError::TokenizationFailed("empty token"));
        }
        if canonical.len() as u16 > quotas.max_token_length {
            return Err(IndexError::QuotaExceeded("token length"));
        }
        let key = (tokenizer_id, tokenizer_version, canonical.to_string());
        if let Some(&id) = self.by_canonical.get(&key) {
            return Ok(id);
        }
        if self.by_id.len() as u32 >= quotas.max_token_dictionary_entries {
            return Err(IndexError::QuotaExceeded("token dictionary"));
        }
        let token_id = Self::candidate_id(tokenizer_id, tokenizer_version, canonical);
        let canon_hash = fnv1a64(canonical.as_bytes());
        if let Some(existing) = self.by_id.get(&token_id) {
            // Same id — must be same tokenizer + same canonical hash and text.
            if existing.tokenizer_id != tokenizer_id
                || existing.tokenizer_version != tokenizer_version
                || existing.canonical_token_hash != canon_hash
            {
                self.collisions_detected = self.collisions_detected.saturating_add(1);
                return Err(IndexError::TokenCollision);
            }
            if let Some(ref t) = existing.canonical_token {
                if t != canonical {
                    self.collisions_detected = self.collisions_detected.saturating_add(1);
                    return Err(IndexError::TokenCollision);
                }
            }
            self.by_canonical.insert(key, token_id);
            return Ok(token_id);
        }
        let entry = TokenDictionaryEntry {
            tokenizer_id,
            tokenizer_version,
            token_id,
            canonical_token_hash: canon_hash,
            canonical_token: if store_text {
                Some(canonical.to_string())
            } else {
                None
            },
        };
        self.by_id.insert(token_id, entry);
        self.by_canonical.insert(key, token_id);
        Ok(token_id)
    }

    /// Deliberate collision test helper: insert raw mapping.
    #[cfg(test)]
    pub fn force_insert_for_collision_test(&mut self, entry: TokenDictionaryEntry) {
        self.by_id.insert(entry.token_id, entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ids() {
        let q = IndexQuotaConfig::default();
        let mut d = TokenDictionary::new();
        let a = d.intern(1, 1, "hello", &q, true).unwrap();
        let b = d.intern(1, 1, "hello", &q, true).unwrap();
        assert_eq!(a, b);
        let c = d.intern(1, 1, "world", &q, true).unwrap();
        assert_ne!(a, c);
    }

    #[test]
    fn version_isolation() {
        let q = IndexQuotaConfig::default();
        let mut d = TokenDictionary::new();
        let a = d.intern(1, 1, "hello", &q, true).unwrap();
        let b = d.intern(1, 2, "hello", &q, true).unwrap();
        // Different version → different id domain (hash includes version).
        assert_ne!(a, b);
    }

    #[test]
    fn collision_detected() {
        let q = IndexQuotaConfig::default();
        let mut d = TokenDictionary::new();
        let id = TokenDictionary::candidate_id(1, 1, "alpha");
        d.force_insert_for_collision_test(TokenDictionaryEntry {
            tokenizer_id: 1,
            tokenizer_version: 1,
            token_id: id,
            canonical_token_hash: fnv1a64(b"other"),
            canonical_token: Some(String::from("other")),
        });
        // Interning "alpha" would claim same id as "other" only if hashes collide;
        // force: insert entry with alpha's id but different text.
        let r = d.intern(1, 1, "alpha", &q, true);
        // If candidate_id("alpha") == id, collision fires.
        if TokenDictionary::candidate_id(1, 1, "alpha") == id {
            assert!(matches!(r, Err(IndexError::TokenCollision)));
        }
    }
}
