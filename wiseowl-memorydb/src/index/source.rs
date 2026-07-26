//! Source index: SourceId / content hash -> record IDs.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use wiseowl_memory::{MemoryId, SourceId};

use crate::record::LongTermMemoryRecord;

#[derive(Debug, Default)]
pub struct SourceIndex {
    by_source: BTreeMap<u64, Vec<u64>>,
    by_content_hash: BTreeMap<u64, Vec<u64>>,
    by_payload_hash: BTreeMap<u64, Vec<u64>>,
}

impl SourceIndex {
    pub fn len(&self) -> usize {
        self.by_source.len()
    }

    pub fn index_record(&mut self, rec: &LongTermMemoryRecord) {
        let id = rec.id.get();
        if let Some(sid) = rec.provenance.source_id {
            push_unique(self.by_source.entry(sid.get()).or_default(), id);
        }
        if let Some(h) = rec.provenance.source_content_hash {
            push_unique(self.by_content_hash.entry(h).or_default(), id);
        }
        push_unique(
            self.by_payload_hash
                .entry(rec.payload_ref.content_hash)
                .or_default(),
            id,
        );
    }

    pub fn by_source_id(&self, source: SourceId) -> &[u64] {
        self.by_source
            .get(&source.get())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn by_source_content_hash(&self, hash: u64) -> &[u64] {
        self.by_content_hash
            .get(&hash)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn by_payload_hash(&self, hash: u64) -> &[u64] {
        self.by_payload_hash
            .get(&hash)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn remove_record(&mut self, rec: &LongTermMemoryRecord) {
        let id = rec.id.get();
        if let Some(sid) = rec.provenance.source_id {
            if let Some(v) = self.by_source.get_mut(&sid.get()) {
                v.retain(|x| *x != id);
            }
        }
        if let Some(h) = rec.provenance.source_content_hash {
            if let Some(v) = self.by_content_hash.get_mut(&h) {
                v.retain(|x| *x != id);
            }
        }
        if let Some(v) = self.by_payload_hash.get_mut(&rec.payload_ref.content_hash) {
            v.retain(|x| *x != id);
        }
    }

    pub fn page_source(
        &self,
        source: SourceId,
        offset: usize,
        limit: usize,
    ) -> (Vec<MemoryId>, bool) {
        let all = self.by_source_id(source);
        let slice = if offset >= all.len() {
            &[][..]
        } else {
            &all[offset..all.len().min(offset + limit)]
        };
        let ids: Vec<MemoryId> = slice
            .iter()
            .filter_map(|r| MemoryId::from_raw(*r).ok())
            .collect();
        let more = offset + limit < all.len();
        (ids, more)
    }
}

fn push_unique(v: &mut Vec<u64>, id: u64) {
    if !v.contains(&id) {
        v.push(id);
        v.sort_unstable();
    }
}
