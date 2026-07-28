//! Indexer operational state (manifests, cursors, roots).
//!
//! Durable document records live in wiseowl-memorydb.
//! This state is medium-term operational data (rebuildable).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use wiseowl_memory::SourceId;

use crate::config::{IndexRootConfig, RootId};
use crate::digest::ContentDigest;
use crate::hash::fnv1a64;
use crate::source::SourceManifest;

/// Full indexer state snapshot.
#[derive(Debug, Clone, Default)]
pub struct IndexerState {
    pub roots: BTreeMap<RootId, IndexRootConfig>,
    pub sources_by_path: BTreeMap<(RootId, String), SourceId>,
    pub sources: BTreeMap<u64, SourceManifest>,
    /// Secondary index: (root, digest fingerprint64) → source ids (rename hints only).
    pub content_digest_index: BTreeMap<(RootId, u64), Vec<u64>>,
    pub file_identity_index: BTreeMap<(u64, u64), u64>,
    pub scan_cursors: BTreeMap<RootId, crate::discover::ScanCursor>,
    pub next_root_id: u64,
    pub next_source_counter: u64,
    pub source_id_generation: u16,
    pub last_successful_scan_ns: u64,
    pub config_generation: u64,
    /// Manifest migrations completed this process.
    pub migrations_completed: u64,
}

impl IndexerState {
    pub fn new() -> Self {
        Self {
            next_root_id: 1,
            next_source_counter: 1,
            source_id_generation: 1,
            ..Default::default()
        }
    }

    pub fn alloc_root_id(&mut self) -> RootId {
        let id = self.next_root_id.max(1);
        self.next_root_id = id.saturating_add(1);
        id
    }

    pub fn alloc_source_id(&mut self) -> SourceId {
        let gen = self.source_id_generation.max(1);
        let counter = self.next_source_counter.max(1);
        self.next_source_counter = counter.saturating_add(1);
        let raw = ((gen as u64) << 48) | (counter & ((1u64 << 48) - 1));
        SourceId::from_raw_unchecked(if raw == 0 { 1 } else { raw })
    }

    pub fn note_source_id(&mut self, id: SourceId) {
        let raw = id.get();
        let counter = raw & ((1u64 << 48) - 1);
        if counter >= self.next_source_counter {
            self.next_source_counter = counter.saturating_add(1);
        }
    }

    pub fn bump_generation_on_restart(&mut self) {
        self.source_id_generation = self.source_id_generation.saturating_add(1).max(1);
        if self.source_id_generation == 0 {
            self.source_id_generation = 1;
        }
        self.next_source_counter = 1;
    }

    pub fn path_hash(root_id: RootId, rel: &str) -> u64 {
        let mut buf = Vec::new();
        buf.extend_from_slice(&root_id.to_le_bytes());
        buf.extend_from_slice(rel.as_bytes());
        fnv1a64(&buf)
    }

    pub fn get_by_path(&self, root_id: RootId, rel: &str) -> Option<&SourceManifest> {
        let sid = self.sources_by_path.get(&(root_id, String::from(rel)))?;
        self.sources.get(&sid.get())
    }

    pub fn insert_manifest(&mut self, m: SourceManifest) {
        let sid = m.source_id.get();
        self.note_source_id(m.source_id);
        self.sources_by_path
            .insert((m.root_id, m.relative_path.clone()), m.source_id);
        if let Some(fi) = m.file_identity {
            self.file_identity_index.insert((fi.device, fi.inode), sid);
        }
        if m.content_digest.is_set() {
            let fp = m.content_digest.fingerprint64();
            self.content_digest_index
                .entry((m.root_id, fp))
                .or_default()
                .push(sid);
        }
        self.sources.insert(sid, m);
    }

    pub fn remove_path_binding(&mut self, root_id: RootId, rel: &str) {
        self.sources_by_path.remove(&(root_id, String::from(rel)));
    }

    /// Count pending imports requiring reconciliation.
    pub fn pending_import_count(&self) -> u64 {
        self.sources
            .values()
            .filter(|m| m.pending_import.is_some())
            .count() as u64
    }

    pub fn find_rename_candidate(
        &self,
        root_id: RootId,
        content_digest: &ContentDigest,
        identity: Option<crate::source::FileIdentity>,
    ) -> Option<SourceId> {
        if let Some(fi) = identity {
            if let Some(&sid) = self.file_identity_index.get(&(fi.device, fi.inode)) {
                if let Some(m) = self.sources.get(&sid) {
                    if m.root_id == root_id {
                        if m.has_strong_digest() && m.content_digest.equals(content_digest) {
                            return Some(m.source_id);
                        }
                        // Same inode even if content changed — still rename of same file.
                        return Some(m.source_id);
                    }
                }
            }
        }
        // Strong digest within same root (rename hint only; not sole identity proof).
        let fp = content_digest.fingerprint64();
        if let Some(ids) = self.content_digest_index.get(&(root_id, fp)) {
            if ids.len() == 1 {
                if let Ok(sid) = SourceId::from_raw(ids[0]) {
                    if let Some(m) = self.sources.get(&sid.get()) {
                        if m.has_strong_digest() && m.content_digest.equals(content_digest) {
                            return Some(sid);
                        }
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_ids_nonzero_and_advance() {
        let mut s = IndexerState::new();
        let a = s.alloc_source_id();
        let b = s.alloc_source_id();
        assert_ne!(a.get(), 0);
        assert_ne!(a, b);
    }
}
