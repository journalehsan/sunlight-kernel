//! Append-only single-file storage engine for sunlight-kv.
//!
//! File layout on disk: sequence of records.
//!   /var/lib/sunlight/kv.store  (or SUNLIGHT_KV_STORE)
//!
//! Startup recovery algorithm (must match spec exactly):
//!   open file (create if needed)
//!   offset = 0
//!   loop:
//!       read 4 bytes -> magic
//!       if EOF -> break
//!       read rest of header
//!       if header invalid -> break
//!       read key/value/acl
//!       compute crc
//!       if crc matches:
//!           if PUT: index[key] = IndexEntry(offset, total_len, flags)
//!           if DELETE: remove key from index
//!       offset += total_len
//!
//! put(key, value, caller):
//!     determine ACL (new or inherit from previous)
//!     encode ACL with bincode
//!     write header + key + value + acl
//!     fsync
//!     update index
//!
//! get/delete: locate via index, seek, read (re-validate ACL against caller at runtime).

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use bincode::Error as BincodeError;

use crate::acl::Acl;
use crate::storage::index::{Index, IndexEntry};
use crate::storage::record::{
    read_record, write_record, FLAG_DELETE, FLAG_PUT, RecordHeader,
};

/// Errors from the storage engine.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("serialization error: {0}")]
    Serialize(#[from] BincodeError),

    #[error("permission denied for caller '{caller}' on key '{key}'")]
    PermissionDenied { key: String, caller: String },

    #[error("key not found: {0}")]
    NotFound(String),

    #[error("corrupt record at offset {offset}: {reason}")]
    Corrupt { offset: u64, reason: String },
}

/// Core append-only KV engine. Holds the file open for the lifetime of the daemon.
pub struct StorageEngine {
    file: File,
    index: Index,
    /// Absolute path for diagnostics.
    #[allow(dead_code)]
    path: std::path::PathBuf,
}

impl StorageEngine {
    /// Open (or create) the store file at `path`, run full log recovery to rebuild the index,
    /// and leave the file handle open for subsequent appends + reads.
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();

        // Ensure parent directory exists (best effort).
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;

        let index = Self::recover(&mut file)?;

        Ok(Self { file, index, path })
    }

    /// Run the exact recovery algorithm described in the system requirements.
    /// Returns a fully populated index of only the live (non-deleted) keys.
    fn recover(file: &mut File) -> Result<Index, StorageError> {
        let mut index: Index = Index::new();

        // Seek to start.
        file.seek(SeekFrom::Start(0))?;

        let _offset: u64 = 0;

        loop {
            // Read first 4 bytes to detect magic / EOF (per pseudocode).
            let mut magic_buf = [0u8; 4];
            match file.read_exact(&mut magic_buf) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    // Clean EOF at record boundary.
                    break;
                }
                Err(e) => return Err(e.into()),
            }

            let magic = u32::from_le_bytes(magic_buf);
            if magic != super::record::RECORD_MAGIC {
                // Per spec pseudocode: "if header invalid -> break".
                // We already consumed 4 bytes; stop recovery here (corruption or partial tail).
                break;
            }

            // Rewind the 4 bytes so read_record can consume the full header.
            file.seek(SeekFrom::Current(-4))?;

            match read_record(file) {
                Ok(Some((header, key_bytes, _value_bytes, acl_bytes))) => {
                    let key = match String::from_utf8(key_bytes) {
                        Ok(k) => k,
                        Err(_) => {
                            // Skip non-UTF8 keys (corrupt or future-proofing).
                            // Position is already after the bad record.
                            continue;
                        }
                    };

                    // Deserialize ACL (must succeed for valid records; otherwise skip).
                    let _acl: Acl = match bincode::deserialize::<Acl>(&acl_bytes) {
                        Ok(a) => a,
                        Err(_) => {
                            // Treat as unreadable record for this key; skip updating index.
                            continue;
                        }
                    };

                    let total_len = (RecordHeader::SIZE as u32)
                        + header.key_len
                        + header.value_len
                        + header.acl_len;

                    // Reconstruct absolute offset of this record start.
                    let current = file.stream_position()?;
                    let record_start = current - (total_len as u64);

                    if header.flags == FLAG_PUT {
                        index.insert(
                            key,
                            IndexEntry {
                                offset: record_start,
                                total_len,
                                flags: header.flags,
                            },
                        );
                    } else if header.flags == FLAG_DELETE {
                        index.remove(&key);
                    }

                    // offset kept only for parity with the documented pseudocode; we primarily
                    // rely on the file cursor for positioning.
                    let _ = current;
                }
                Ok(None) => {
                    // EOF inside read_record
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::InvalidData || e.kind() == io::ErrorKind::UnexpectedEof => {
                    // Corrupted or truncated record at tail: stop recovery (per spec "break").
                    // Leave any prior good records in the index.
                    break;
                }
                Err(e) => return Err(e.into()),
            }
        }

        // After recovery, position the file at the true end for appends.
        let _ = file.seek(SeekFrom::End(0))?;
        Ok(index)
    }

    /// Current number of live keys (for diagnostics / tests).
    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Store or overwrite a value.
    /// On first insert: caller becomes owner with default ACL (self only).
    /// On update: previous ACL is inherited (read from the old record).
    pub fn put(&mut self, key: &str, value: &[u8], caller: &str) -> Result<(), StorageError> {
        // Determine ACL to use for the new record.
        // Copy the entry (small struct) so we release the immutable borrow on self.index
        // before calling any &mut self methods.
        let acl = if let Some(entry) = self.index.get(key).copied() {
            // Existing key: must have write permission. Load previous ACL to inherit it.
            if !self.check_write_permission_from_entry(&entry, caller) {
                return Err(StorageError::PermissionDenied {
                    key: key.to_string(),
                    caller: caller.to_string(),
                });
            }
            // Read the old record's ACL (ignore value).
            let (_old_val, old_acl) = self.read_value_and_acl_at(&entry)?;
            old_acl
        } else {
            // New key: caller is owner.
            Acl::new(caller)
        };

        let acl_bytes = bincode::serialize(&acl)?;

        // Append at current end.
        let write_offset = self.file.seek(SeekFrom::End(0))?;

        let total_len = write_record(
            &mut self.file,
            FLAG_PUT,
            key.as_bytes(),
            value,
            &acl_bytes,
        )?;

        self.file.sync_all()?;

        // Update in-memory index to point at the new record.
        self.index.insert(
            key.to_string(),
            IndexEntry {
                offset: write_offset,
                total_len,
                flags: FLAG_PUT,
            },
        );

        Ok(())
    }

    /// Retrieve value for key (after permission check).
    pub fn get(&mut self, key: &str, caller: &str) -> Result<Vec<u8>, StorageError> {
        let entry = self
            .index
            .get(key)
            .copied()
            .ok_or_else(|| StorageError::NotFound(key.to_string()))?;

        // Permission check requires reading the ACL.
        let (value, acl) = self.read_value_and_acl_at(&entry)?;

        if !acl.allows_read(caller) {
            return Err(StorageError::PermissionDenied {
                key: key.to_string(),
                caller: caller.to_string(),
            });
        }

        Ok(value)
    }

    /// Delete a key by writing a tombstone record and removing from index.
    pub fn delete(&mut self, key: &str, caller: &str) -> Result<(), StorageError> {
        let entry = self
            .index
            .get(key)
            .copied()
            .ok_or_else(|| StorageError::NotFound(key.to_string()))?;

        // Must be allowed to delete (write permission).
        if !self.check_write_permission_from_entry(&entry, caller) {
            return Err(StorageError::PermissionDenied {
                key: key.to_string(),
                caller: caller.to_string(),
            });
        }

        // Load ACL to embed in the tombstone record (for recovery fidelity; not used for index).
        let (_val, acl) = self.read_value_and_acl_at(&entry)?;
        let acl_bytes = bincode::serialize(&acl)?;

        let write_offset = self.file.seek(SeekFrom::End(0))?;

        let total_len = write_record(
            &mut self.file,
            FLAG_DELETE,
            key.as_bytes(),
            &[], // value empty for tombstone
            &acl_bytes,
        )?;

        self.file.sync_all()?;

        // Remove from live index. The tombstone record remains in the log for replay on restart.
        self.index.remove(key);

        // We do not keep tombstones in the index (per design).
        let _ = (write_offset, total_len); // recorded in log only

        Ok(())
    }

    /// Return all keys with the given prefix, in arbitrary order.
    /// Permission is not required to discover keys via SCAN (only for subsequent GET).
    /// This matches many KV designs; adjust if stronger semantics desired.
    pub fn scan_prefix(&self, prefix: &str) -> Vec<String> {
        self.index
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }

    // ---------------------------------------------------------------------
    // Internal helpers
    // ---------------------------------------------------------------------

    /// Check write permission using only the IndexEntry (must read ACL from disk).
    fn check_write_permission_from_entry(&mut self, entry: &IndexEntry, caller: &str) -> bool {
        if let Ok((_v, acl)) = self.read_value_and_acl_at(entry) {
            acl.allows_write(caller)
        } else {
            // If we cannot read the ACL we conservatively deny.
            false
        }
    }

    /// Seek to the record, read it, deserialize value + ACL.
    /// Does not perform permission check (caller does that).
    fn read_value_and_acl_at(&mut self, entry: &IndexEntry) -> Result<(Vec<u8>, Acl), StorageError> {
        self.file.seek(SeekFrom::Start(entry.offset))?;

        match read_record(&mut self.file) {
            Ok(Some((header, key_bytes, value_bytes, acl_bytes))) => {
                // Basic sanity: key should match what we expect (best effort).
                let _ = key_bytes; // we trust the index for live entries

                let acl: Acl = bincode::deserialize(&acl_bytes)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

                // For a live index entry we expect a PUT record.
                if header.flags == FLAG_PUT {
                    Ok((value_bytes, acl))
                } else {
                    // Should never happen for indexed entries.
                    Err(StorageError::Corrupt {
                        offset: entry.offset,
                        reason: "indexed entry points at non-PUT record".into(),
                    })
                }
            }
            Ok(None) => Err(StorageError::Corrupt {
                offset: entry.offset,
                reason: "unexpected EOF while reading indexed record".into(),
            }),
            Err(e) => Err(e.into()),
        }
    }

    /// For testing / diagnostics: force a full re-recovery from disk.
    #[cfg(test)]
    pub fn force_recover(&mut self) -> Result<(), StorageError> {
        self.file.seek(SeekFrom::Start(0))?;
        self.index = Self::recover(&mut self.file)?;
        Ok(())
    }
}

// ==========================================================================
// Compile-time required test examples (create, read, overwrite, delete, scan,
// permission enforcement). All exercised via direct StorageEngine API.
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;

    fn fresh_store() -> (StorageEngine, PathBuf) {
        let tmp = NamedTempFile::new().expect("tempfile");
        let path = tmp.path().to_path_buf();
        // Keep the NamedTempFile alive by leaking the handle; the path remains valid.
        // We need the file to not be deleted until end of test.
        let _ = tmp.keep().ok();
        let engine = StorageEngine::open(&path).expect("open storage");
        (engine, path)
    }

    #[test]
    fn create_and_read_key() {
        let (mut kv, _p) = fresh_store();
        let owner = "svc-alpha";

        kv.put("mykey", b"hello world", owner).unwrap();
        let v = kv.get("mykey", owner).unwrap();
        assert_eq!(v, b"hello world");
    }

    #[test]
    fn overwrite_key_preserves_acl_and_updates_value() {
        let (mut kv, _p) = fresh_store();
        let owner = "svc-alpha";

        kv.put("k", b"v1", owner).unwrap();
        kv.put("k", b"v2", owner).unwrap(); // overwrite by same owner

        let v = kv.get("k", owner).unwrap();
        assert_eq!(v, b"v2");

        // Different caller without rights must be rejected even for read.
        let other = "svc-beta";
        let err = kv.get("k", other).unwrap_err();
        match err {
            StorageError::PermissionDenied { .. } => {}
            other => panic!("expected permission denied, got {:?}", other),
        }
    }

    #[test]
    fn delete_key_and_not_found_after() {
        let (mut kv, _p) = fresh_store();
        let owner = "svc-alpha";

        kv.put("temp", b"data", owner).unwrap();
        assert!(kv.get("temp", owner).is_ok());

        kv.delete("temp", owner).unwrap();

        let err = kv.get("temp", owner).unwrap_err();
        match err {
            StorageError::NotFound(_) => {}
            other => panic!("expected not found after delete, got {:?}", other),
        }
    }

    #[test]
    fn scan_prefix() {
        let (mut kv, _p) = fresh_store();
        let owner = "svc-alpha";

        kv.put("user:1", b"a", owner).unwrap();
        kv.put("user:2", b"b", owner).unwrap();
        kv.put("config:foo", b"c", owner).unwrap();

        let mut users: Vec<_> = kv.scan_prefix("user:");
        users.sort();
        assert_eq!(users, vec!["user:1".to_string(), "user:2".to_string()]);

        let configs = kv.scan_prefix("config:");
        assert_eq!(configs, vec!["config:foo".to_string()]);

        let none = kv.scan_prefix("nonexistent:");
        assert!(none.is_empty());
    }

    #[test]
    fn reject_unauthorized_access() {
        let (mut kv, _p) = fresh_store();
        let owner = "svc-alpha";
        let intruder = "svc-evil";

        // Intruder cannot create? First write always succeeds for the caller (they become owner).
        // But we test that the owner can, and a different identity cannot read/write afterwards.
        kv.put("secret", b"s3cr3t", owner).unwrap();

        // Intruder cannot read
        assert!(matches!(
            kv.get("secret", intruder),
            Err(StorageError::PermissionDenied { .. })
        ));

        // Intruder cannot overwrite
        assert!(matches!(
            kv.put("secret", b"nope", intruder),
            Err(StorageError::PermissionDenied { .. })
        ));

        // Intruder cannot delete
        assert!(matches!(
            kv.delete("secret", intruder),
            Err(StorageError::PermissionDenied { .. })
        ));

        // Owner still can
        assert!(kv.get("secret", owner).is_ok());
        kv.delete("secret", owner).unwrap();
    }

    #[test]
    fn crash_recovery_rebuilds_index_and_tombstones() {
        let (mut kv, path) = fresh_store();
        let owner = "svc-alpha";

        kv.put("a", b"one", owner).unwrap();
        kv.put("b", b"two", owner).unwrap();
        kv.delete("a", owner).unwrap(); // tombstone for a

        // Simulate crash: drop and reopen (forces recovery).
        drop(kv);
        let mut kv2 = StorageEngine::open(&path).expect("reopen after crash");

        // a was deleted -> not present
        assert!(kv2.get("a", owner).is_err());
        // b still present
        assert_eq!(kv2.get("b", owner).unwrap(), b"two");

        // Re-put a as different owner? No: new first writer becomes new owner.
        // Here we re-create as same logical owner string.
        kv2.put("a", b"three", owner).unwrap();
        assert_eq!(kv2.get("a", owner).unwrap(), b"three");
    }

    #[test]
    fn root_bypasses_acl() {
        let (mut kv, _p) = fresh_store();
        let owner = "svc-alpha";

        kv.put("root-test", b"data", owner).unwrap();

        // "root" can read, write, delete regardless of ACL lists.
        assert_eq!(kv.get("root-test", "root").unwrap(), b"data");
        kv.put("root-test", b"root-overwrite", "root").unwrap();
        assert_eq!(kv.get("root-test", owner).unwrap(), b"root-overwrite");
        kv.delete("root-test", "root").unwrap();
        assert!(matches!(kv.get("root-test", owner), Err(StorageError::NotFound(_))));
    }
}

