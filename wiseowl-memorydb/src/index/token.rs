//! Token inverted index: (tokenizer_id, version, token_id) -> postings.
//!
//! Tokenizer versions are never mixed: keys include both id and version.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use wiseowl_memory::MemoryId;

use crate::record::LongTermMemoryRecord;
use crate::tokens::{TokenMatchMode, TokenQuery};

/// One posting list entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenPosting {
    pub memory_id: u64,
    pub revision: u32,
    pub frequency: u16,
}

#[derive(Debug, Default)]
pub struct TokenIndex {
    /// Key: (tokenizer_id, tokenizer_version, token_id)
    map: BTreeMap<(u32, u32, u64), Vec<TokenPosting>>,
    dictionary_entries: u64,
}

impl TokenIndex {
    pub fn dictionary_len(&self) -> u64 {
        self.map.len() as u64
    }

    pub fn posting_entries(&self) -> u64 {
        self.dictionary_entries
    }

    pub fn index_record(&mut self, rec: &LongTermMemoryRecord) {
        let Some(ts) = rec.tokens else {
            return;
        };
        for t in &rec.token_entries {
            let key = (ts.tokenizer_id, ts.tokenizer_version, t.token_id);
            let list = self.map.entry(key).or_default();
            // Replace same memory_id posting.
            list.retain(|p| p.memory_id != rec.id.get());
            list.push(TokenPosting {
                memory_id: rec.id.get(),
                revision: rec.revision,
                frequency: t.frequency,
            });
            list.sort_by_key(|p| p.memory_id);
            self.dictionary_entries = self.dictionary_entries.saturating_add(1);
        }
    }

    pub fn lookup(
        &self,
        tokenizer_id: u32,
        tokenizer_version: u32,
        token_id: u64,
    ) -> &[TokenPosting] {
        self.map
            .get(&(tokenizer_id, tokenizer_version, token_id))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Deterministic token query. Returns matching MemoryIds sorted.
    pub fn query(&self, q: &TokenQuery, limit: usize) -> Vec<MemoryId> {
        if q.token_ids.is_empty() {
            return Vec::new();
        }
        let mut sets: Vec<alloc::collections::BTreeSet<u64>> = Vec::new();
        for tid in &q.token_ids {
            let postings = self.lookup(q.tokenizer_id, q.tokenizer_version, *tid);
            let set: alloc::collections::BTreeSet<u64> =
                postings.iter().map(|p| p.memory_id).collect();
            sets.push(set);
        }
        let mut result: Vec<u64> = match q.mode {
            TokenMatchMode::Any => {
                let mut u = alloc::collections::BTreeSet::new();
                for s in sets {
                    u.extend(s);
                }
                u.into_iter().collect()
            }
            TokenMatchMode::All => {
                let mut iter = sets.into_iter();
                let Some(mut acc) = iter.next() else {
                    return Vec::new();
                };
                for s in iter {
                    acc = acc.intersection(&s).copied().collect();
                }
                acc.into_iter().collect()
            }
            TokenMatchMode::MinimumCount(min) => {
                let mut counts: BTreeMap<u64, u16> = BTreeMap::new();
                for s in sets {
                    for id in s {
                        *counts.entry(id).or_insert(0) =
                            counts.get(&id).copied().unwrap_or(0).saturating_add(1);
                    }
                }
                counts
                    .into_iter()
                    .filter(|(_, c)| *c >= min)
                    .map(|(id, _)| id)
                    .collect()
            }
        };
        result.sort_unstable();
        result.truncate(limit);
        result
            .into_iter()
            .filter_map(|id| MemoryId::from_raw(id).ok())
            .collect()
    }

    pub fn remove_memory(&mut self, memory_id: u64) {
        for list in self.map.values_mut() {
            let before = list.len();
            list.retain(|p| p.memory_id != memory_id);
            let removed = before - list.len();
            self.dictionary_entries = self.dictionary_entries.saturating_sub(removed as u64);
        }
    }
}
