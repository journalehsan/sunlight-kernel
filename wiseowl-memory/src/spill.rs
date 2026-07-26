//! Bounded cold spill storage.
//!
//! Host mode uses a directory of segment files with atomic temp-write + rename.
//! Incomplete temporary files are rejected on restart.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::MemoryError;
use crate::ids::SegmentId;
use crate::segments::{Segment, SEGMENT_HEADER_LEN};

/// Metadata about a spilled segment on disk.
#[derive(Debug, Clone)]
pub struct SpillRecordMeta {
    pub segment_id: SegmentId,
    pub path: PathBuf,
    pub size: u64,
}

/// Filesystem-backed spill store (host).
#[derive(Debug)]
pub struct SpillStore {
    root: PathBuf,
    /// In-memory index of valid spilled segments.
    index: HashMap<SegmentId, SpillRecordMeta>,
    /// Paths quarantined due to corruption (not loaded).
    pub quarantined: Vec<PathBuf>,
}

impl SpillStore {
    pub fn open(root: impl AsRef<Path>, max_decompress: u32) -> Result<Self, MemoryError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|_| MemoryError::InternalInvariantViolation("spill mkdir"))?;
        let mut store = Self {
            root,
            index: HashMap::new(),
            quarantined: Vec::new(),
        };
        store.recover(max_decompress)?;
        Ok(store)
    }

    fn recover(&mut self, max_decompress: u32) -> Result<(), MemoryError> {
        let entries = fs::read_dir(&self.root)
            .map_err(|_| MemoryError::InternalInvariantViolation("spill readdir"))?;
        for ent in entries.flatten() {
            let path = ent.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Incomplete temp files from crashed writes.
            if name.ends_with(".tmp") {
                let _ = fs::remove_file(&path);
                continue;
            }
            if !name.starts_with("seg-") || !name.ends_with(".owls") {
                continue;
            }
            match self.load_and_validate(&path, max_decompress) {
                Ok(meta) => {
                    self.index.insert(meta.segment_id, meta);
                }
                Err(_) => {
                    // Quarantine corrupt records; do not crash service startup.
                    let q = self.root.join(format!("quarantine-{name}"));
                    let _ = fs::rename(&path, &q);
                    self.quarantined.push(q);
                }
            }
        }
        Ok(())
    }

    fn load_and_validate(
        &self,
        path: &Path,
        max_decompress: u32,
    ) -> Result<SpillRecordMeta, MemoryError> {
        let mut f = fs::File::open(path)
            .map_err(|_| MemoryError::SpillIncomplete)?;
        let mut blob = Vec::new();
        f.read_to_end(&mut blob)
            .map_err(|_| MemoryError::SpillIncomplete)?;
        if blob.len() < SEGMENT_HEADER_LEN {
            return Err(MemoryError::SpillIncomplete);
        }
        // Full validation including checksum.
        let (_seg, _) = Segment::from_spill_blob(&blob, max_decompress)?;
        let header = crate::segments::ColdSegmentHeader::decode(&blob)?;
        Ok(SpillRecordMeta {
            segment_id: header.segment_id,
            path: path.to_path_buf(),
            size: blob.len() as u64,
        })
    }

    /// Atomic write: write temp then rename.
    pub fn write_segment(&mut self, segment: &Segment) -> Result<SpillRecordMeta, MemoryError> {
        let blob = segment.encode_spill_blob()?;
        let final_name = format!("seg-{}.owls", segment.id.get());
        let final_path = self.root.join(&final_name);
        let tmp_path = self.root.join(format!("seg-{}.owls.tmp", segment.id.get()));

        {
            let mut f = fs::File::create(&tmp_path)
                .map_err(|_| MemoryError::InternalInvariantViolation("spill create"))?;
            f.write_all(&blob)
                .map_err(|_| MemoryError::InternalInvariantViolation("spill write"))?;
            f.sync_all()
                .map_err(|_| MemoryError::InternalInvariantViolation("spill sync"))?;
        }
        fs::rename(&tmp_path, &final_path)
            .map_err(|_| MemoryError::InternalInvariantViolation("spill rename"))?;

        let meta = SpillRecordMeta {
            segment_id: segment.id,
            path: final_path,
            size: blob.len() as u64,
        };
        self.index.insert(segment.id, meta.clone());
        Ok(meta)
    }

    pub fn read_blob(&self, segment_id: SegmentId) -> Result<Vec<u8>, MemoryError> {
        let meta = self
            .index
            .get(&segment_id)
            .ok_or(MemoryError::SegmentNotFound)?;
        fs::read(&meta.path).map_err(|_| MemoryError::SpillIncomplete)
    }

    pub fn delete(&mut self, segment_id: SegmentId) -> Result<(), MemoryError> {
        if let Some(meta) = self.index.remove(&segment_id) {
            let _ = fs::remove_file(&meta.path);
        }
        Ok(())
    }

    pub fn contains(&self, segment_id: SegmentId) -> bool {
        self.index.contains_key(&segment_id)
    }

    pub fn total_bytes(&self) -> u64 {
        self.index.values().map(|m| m.size).sum()
    }

    pub fn segment_ids(&self) -> impl Iterator<Item = SegmentId> + '_ {
        self.index.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{MemoryId, SegmentId, SessionId};
    use crate::segments::Segment;
    use tempfile::tempdir;

    #[test]
    fn atomic_spill_and_recover() {
        let dir = tempdir().unwrap();
        let mut store = SpillStore::open(dir.path(), 64 * 1024).unwrap();
        let mut seg = Segment::new_open(
            SegmentId::from_raw(3).unwrap(),
            SessionId::from_raw(1).unwrap(),
            1,
            99999,
        );
        seg.append_record(MemoryId::from_raw(9).unwrap(), b"cold-data", 10, 4096)
            .unwrap();
        seg.seal().unwrap();
        seg.compress_once(4096).unwrap();
        store.write_segment(&seg).unwrap();

        // Reopen
        let store2 = SpillStore::open(dir.path(), 64 * 1024).unwrap();
        assert!(store2.contains(SegmentId::from_raw(3).unwrap()));
        assert!(store2.quarantined.is_empty());
    }

    #[test]
    fn corrupt_segment_quarantined_on_restart() {
        let dir = tempdir().unwrap();
        let mut store = SpillStore::open(dir.path(), 64 * 1024).unwrap();
        let mut seg = Segment::new_open(
            SegmentId::from_raw(1).unwrap(),
            SessionId::from_raw(1).unwrap(),
            1,
            99999,
        );
        seg.append_record(MemoryId::from_raw(1).unwrap(), b"x", 1, 4096)
            .unwrap();
        seg.seal().unwrap();
        seg.compress_once(4096).unwrap();
        store.write_segment(&seg).unwrap();

        // Corrupt the checksum field (bytes 36..40) so validation fails.
        let path = dir.path().join("seg-1.owls");
        let mut data = fs::read(&path).unwrap();
        assert!(data.len() > 40);
        data[36] ^= 0xFF;
        fs::write(&path, data).unwrap();

        let store2 = SpillStore::open(dir.path(), 64 * 1024).unwrap();
        assert!(!store2.contains(SegmentId::from_raw(1).unwrap()));
        assert_eq!(store2.quarantined.len(), 1);
    }

    #[test]
    fn incomplete_tmp_removed() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("seg-99.owls.tmp"), b"partial").unwrap();
        let store = SpillStore::open(dir.path(), 64 * 1024).unwrap();
        assert!(!dir.path().join("seg-99.owls.tmp").exists());
        assert!(store.is_empty());
    }
}
