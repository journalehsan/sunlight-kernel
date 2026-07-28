//! Bounded cold spill storage.
//!
//! Host mode uses a directory of segment files with atomic temp-write + rename.
//! Incomplete temporary files are rejected on restart.
//!
//! # Quarantine policy (Phase 1.1)
//!
//! Corrupt or incomplete spill files never block startup. Quarantine is hard-bounded:
//!
//! | Limit | Default |
//! |-------|---------|
//! | maximum quarantine size | 1 MiB |
//! | maximum quarantine files | 16 |
//! | maximum single quarantined file | 256 KiB |
//! | maximum files inspected per startup | 64 |
//! | maximum bytes inspected per startup | 4 MiB |
//!
//! Cleanup order: expired quarantine → oldest by count → oldest by bytes.
//! Already-quarantined names (`quarantine-*`) are never re-quarantined under a
//! new name (no growth storm). Reason codes are stored as a short suffix, not
//! payload content.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::MemoryError;
use crate::ids::SegmentId;
use crate::segments::{Segment, SEGMENT_HEADER_LEN};

/// Default maximum total quarantine bytes.
pub const MAX_QUARANTINE_BYTES: u64 = 1024 * 1024;
/// Default maximum number of quarantined files.
pub const MAX_QUARANTINE_FILES: usize = 16;
/// Default maximum single quarantined file size.
pub const MAX_QUARANTINE_FILE_BYTES: u64 = 256 * 1024;
/// Default maximum files inspected during recovery.
pub const MAX_RECOVERY_FILES: usize = 64;
/// Default maximum bytes inspected during recovery.
pub const MAX_RECOVERY_BYTES: u64 = 4 * 1024 * 1024;
/// Quarantine retention for cleanup (seconds). Very old files are deleted first.
pub const QUARANTINE_MAX_AGE_SECS: u64 = 7 * 24 * 3600;

/// Metadata about a spilled segment on disk.
#[derive(Debug, Clone)]
pub struct SpillRecordMeta {
    pub segment_id: SegmentId,
    pub path: PathBuf,
    pub size: u64,
}

/// Bounded quarantine configuration.
#[derive(Debug, Clone)]
pub struct QuarantineConfig {
    pub max_bytes: u64,
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_inspect_files: usize,
    pub max_inspect_bytes: u64,
}

impl Default for QuarantineConfig {
    fn default() -> Self {
        Self {
            max_bytes: MAX_QUARANTINE_BYTES,
            max_files: MAX_QUARANTINE_FILES,
            max_file_bytes: MAX_QUARANTINE_FILE_BYTES,
            max_inspect_files: MAX_RECOVERY_FILES,
            max_inspect_bytes: MAX_RECOVERY_BYTES,
        }
    }
}

/// Filesystem-backed spill store (host).
#[derive(Debug)]
pub struct SpillStore {
    root: PathBuf,
    /// In-memory index of valid spilled segments.
    index: HashMap<SegmentId, SpillRecordMeta>,
    /// Paths currently in quarantine (bounded).
    pub quarantined: Vec<PathBuf>,
    /// Reason codes for quarantine (parallel to `quarantined`, short labels).
    pub quarantine_reasons: Vec<&'static str>,
    pub quarantine_cfg: QuarantineConfig,
    /// Files inspected during last recovery.
    pub files_inspected: u32,
    /// Bytes inspected during last recovery.
    pub bytes_inspected: u64,
    /// Generation file path for restart-safe IDs.
    generation_path: PathBuf,
}

impl SpillStore {
    pub fn open(root: impl AsRef<Path>, max_decompress: u32) -> Result<Self, MemoryError> {
        Self::open_with_quarantine(root, max_decompress, QuarantineConfig::default())
    }

    pub fn open_with_quarantine(
        root: impl AsRef<Path>,
        max_decompress: u32,
        quarantine_cfg: QuarantineConfig,
    ) -> Result<Self, MemoryError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .map_err(|_| MemoryError::InternalInvariantViolation("spill mkdir"))?;
        let generation_path = root.join("generation.bin");
        let mut store = Self {
            root,
            index: HashMap::new(),
            quarantined: Vec::new(),
            quarantine_reasons: Vec::new(),
            quarantine_cfg,
            files_inspected: 0,
            bytes_inspected: 0,
            generation_path,
        };
        store.recover(max_decompress)?;
        store.enforce_quarantine_quotas();
        Ok(store)
    }

    /// Load persisted generation (0 if missing/corrupt).
    pub fn load_generation(&self) -> u16 {
        match fs::read(&self.generation_path) {
            Ok(bytes) if bytes.len() >= 2 => u16::from_le_bytes([bytes[0], bytes[1]]),
            _ => 0,
        }
    }

    /// Persist generation (atomic temp + rename).
    pub fn store_generation(&self, generation: u16) -> Result<(), MemoryError> {
        let tmp = self.root.join("generation.bin.tmp");
        {
            let mut f = fs::File::create(&tmp)
                .map_err(|_| MemoryError::InternalInvariantViolation("gen create"))?;
            f.write_all(&generation.to_le_bytes())
                .map_err(|_| MemoryError::InternalInvariantViolation("gen write"))?;
            f.sync_all()
                .map_err(|_| MemoryError::InternalInvariantViolation("gen sync"))?;
        }
        fs::rename(&tmp, &self.generation_path)
            .map_err(|_| MemoryError::InternalInvariantViolation("gen rename"))?;
        Ok(())
    }

    fn recover(&mut self, max_decompress: u32) -> Result<(), MemoryError> {
        // Index existing quarantine files without re-renaming them.
        self.scan_existing_quarantine();

        let entries = match fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(_) => return Err(MemoryError::InternalInvariantViolation("spill readdir")),
        };

        for ent in entries.flatten() {
            if self.files_inspected as usize >= self.quarantine_cfg.max_inspect_files {
                break;
            }
            if self.bytes_inspected >= self.quarantine_cfg.max_inspect_bytes {
                break;
            }

            let path = ent.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            // Skip generation file, quarantine, and non-segment names.
            if name == "generation.bin" || name == "generation.bin.tmp" {
                continue;
            }
            if name.starts_with("quarantine-") {
                continue;
            }
            // Incomplete temp files from crashed writes.
            if name.ends_with(".tmp") {
                let _ = fs::remove_file(&path);
                continue;
            }
            if !name.starts_with("seg-") || !name.ends_with(".owls") {
                continue;
            }

            let meta_len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            self.files_inspected = self.files_inspected.saturating_add(1);
            self.bytes_inspected = self.bytes_inspected.saturating_add(meta_len);

            match self.load_and_validate(&path, max_decompress) {
                Ok(meta) => {
                    self.index.insert(meta.segment_id, meta);
                }
                Err(reason) => {
                    self.quarantine_file(&path, &name, reason_code(reason));
                }
            }
        }
        Ok(())
    }

    fn scan_existing_quarantine(&mut self) {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        let mut found: Vec<(PathBuf, u64)> = Vec::new();
        for ent in entries.flatten() {
            let path = ent.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !name.starts_with("quarantine-") {
                continue;
            }
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            found.push((path, size));
        }
        // Deterministic: sort by name (stable).
        found.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, _) in found {
            if self.quarantined.len() >= self.quarantine_cfg.max_files {
                // Drop excess oldest-by-name (already sorted).
                let _ = fs::remove_file(&path);
                continue;
            }
            self.quarantined.push(path);
            self.quarantine_reasons.push("prior");
        }
    }

    fn quarantine_file(&mut self, path: &Path, name: &str, reason: &'static str) {
        // Never re-quarantine under a new name if already quarantine-*.
        if name.starts_with("quarantine-") {
            return;
        }
        // Bound single file size: oversize corrupt files are deleted, not kept.
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size > self.quarantine_cfg.max_file_bytes {
            let _ = fs::remove_file(path);
            return;
        }
        // Stable quarantine name from original basename (no per-restart rename churn).
        let qname = format!("quarantine-{name}");
        let q = self.root.join(&qname);
        if q.exists() {
            // Already quarantined from a prior run — do not create another copy.
            let _ = fs::remove_file(path);
            if !self.quarantined.iter().any(|p| p == &q) {
                self.quarantined.push(q);
                self.quarantine_reasons.push(reason);
            }
            return;
        }
        if fs::rename(path, &q).is_ok() {
            self.quarantined.push(q);
            self.quarantine_reasons.push(reason);
            self.enforce_quarantine_quotas();
        } else {
            let _ = fs::remove_file(path);
        }
    }

    /// Deterministic cleanup of quarantine directory.
    pub fn enforce_quarantine_quotas(&mut self) {
        // 1. Remove expired quarantine files (by mtime if available).
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut i = 0;
        while i < self.quarantined.len() {
            let path = &self.quarantined[i];
            let expired = fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| now.saturating_sub(d.as_secs()) > QUARANTINE_MAX_AGE_SECS)
                .unwrap_or(false);
            if expired {
                let _ = fs::remove_file(path);
                self.quarantined.remove(i);
                if i < self.quarantine_reasons.len() {
                    self.quarantine_reasons.remove(i);
                }
            } else {
                i += 1;
            }
        }

        // Sort oldest first by path name for deterministic eviction.
        let mut order: Vec<usize> = (0..self.quarantined.len()).collect();
        order.sort_by(|&a, &b| self.quarantined[a].cmp(&self.quarantined[b]));

        // 2. Count quota: remove oldest until within max_files.
        while self.quarantined.len() > self.quarantine_cfg.max_files {
            if let Some(idx) = order.first().copied() {
                if idx < self.quarantined.len() {
                    let _ = fs::remove_file(&self.quarantined[idx]);
                    self.quarantined.remove(idx);
                    if idx < self.quarantine_reasons.len() {
                        self.quarantine_reasons.remove(idx);
                    }
                }
                order = (0..self.quarantined.len()).collect();
                order.sort_by(|&a, &b| self.quarantined[a].cmp(&self.quarantined[b]));
            } else {
                break;
            }
        }

        // 3. Byte quota: remove oldest until within max_bytes.
        loop {
            let total: u64 = self
                .quarantined
                .iter()
                .map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
                .sum();
            if total <= self.quarantine_cfg.max_bytes || self.quarantined.is_empty() {
                break;
            }
            // Remove lexicographically first (oldest stable key).
            let mut idxs: Vec<usize> = (0..self.quarantined.len()).collect();
            idxs.sort_by(|&a, &b| self.quarantined[a].cmp(&self.quarantined[b]));
            let idx = idxs[0];
            let _ = fs::remove_file(&self.quarantined[idx]);
            self.quarantined.remove(idx);
            if idx < self.quarantine_reasons.len() {
                self.quarantine_reasons.remove(idx);
            }
        }
    }

    fn load_and_validate(
        &self,
        path: &Path,
        max_decompress: u32,
    ) -> Result<SpillRecordMeta, MemoryError> {
        let mut f = fs::File::open(path).map_err(|_| MemoryError::SpillIncomplete)?;
        let mut blob = Vec::new();
        f.read_to_end(&mut blob)
            .map_err(|_| MemoryError::SpillIncomplete)?;
        if blob.len() < SEGMENT_HEADER_LEN {
            return Err(MemoryError::SpillIncomplete);
        }
        // Full validation including checksum and v2 record metadata.
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

    pub fn quarantine_total_bytes(&self) -> u64 {
        self.quarantined
            .iter()
            .map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum()
    }
}

fn reason_code(err: MemoryError) -> &'static str {
    match err {
        MemoryError::ChecksumMismatch => "checksum",
        MemoryError::SpillIncomplete => "incomplete",
        MemoryError::SpillCorrupt => "corrupt",
        MemoryError::UnsupportedProtocolVersion { .. } => "version",
        MemoryError::PayloadTooLarge { .. } => "oversized",
        MemoryError::DecompressionFailure => "decompress",
        _ => "invalid",
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
        drop(store);

        // Corrupt the file
        for ent in fs::read_dir(dir.path()).unwrap() {
            let p = ent.unwrap().path();
            if p.extension().and_then(|e| e.to_str()) == Some("owls") {
                let mut d = fs::read(&p).unwrap();
                if d.len() > 40 {
                    d[36] ^= 0xFF; // checksum field
                    fs::write(&p, &d).unwrap();
                }
            }
        }

        let store2 = SpillStore::open(dir.path(), 64 * 1024).unwrap();
        assert!(!store2.quarantined.is_empty());
        assert!(store2.index.is_empty());
    }

    #[test]
    fn incomplete_tmp_removed() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("seg-9.owls.tmp"), b"partial").unwrap();
        let store = SpillStore::open(dir.path(), 64 * 1024).unwrap();
        assert!(!dir.path().join("seg-9.owls.tmp").exists());
        assert!(store.quarantined.is_empty());
    }

    #[test]
    fn quarantine_not_re_renamed_on_restart() {
        let dir = tempdir().unwrap();
        // Seed a quarantine file
        fs::write(dir.path().join("quarantine-seg-1.owls"), b"bad").unwrap();
        let store = SpillStore::open(dir.path(), 64 * 1024).unwrap();
        assert_eq!(store.quarantined.len(), 1);
        // Second open must not grow
        let store2 = SpillStore::open(dir.path(), 64 * 1024).unwrap();
        assert_eq!(store2.quarantined.len(), 1);
        let count = fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .ok()
                    .and_then(|e| e.file_name().into_string().ok())
                    .map(|n| n.starts_with("quarantine-"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn quarantine_file_count_bounded() {
        let dir = tempdir().unwrap();
        let cfg = QuarantineConfig {
            max_files: 2,
            max_bytes: MAX_QUARANTINE_BYTES,
            ..QuarantineConfig::default()
        };
        for i in 0..5 {
            fs::write(dir.path().join(format!("quarantine-seg-{i}.owls")), b"x").unwrap();
        }
        let store = SpillStore::open_with_quarantine(dir.path(), 64 * 1024, cfg).unwrap();
        assert!(store.quarantined.len() <= 2);
    }

    #[test]
    fn generation_persist() {
        let dir = tempdir().unwrap();
        let store = SpillStore::open(dir.path(), 64 * 1024).unwrap();
        assert_eq!(store.load_generation(), 0);
        store.store_generation(7).unwrap();
        let store2 = SpillStore::open(dir.path(), 64 * 1024).unwrap();
        assert_eq!(store2.load_generation(), 7);
    }
}
