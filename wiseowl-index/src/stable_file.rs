//! Stable-file validation and bounded strong content hashing.
//!
//! Streaming SHA-256; does not trust mtime alone as final identity.
//! Metadata before/after must match or the digest is discarded.

use alloc::vec::Vec;

use crate::digest::{digest_bytes, fast_fingerprint, ContentDigest, FastFingerprint};
#[cfg(feature = "host")]
use crate::digest::ContentDigestHasher;
use crate::error::IndexError;
use crate::quotas::IndexQuotaConfig;
use crate::source::FileIdentity;

/// Metadata snapshot used for stability checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMetaSnapshot {
    pub size_bytes: u64,
    pub modified_at_ns: Option<u64>,
    pub identity: Option<FileIdentity>,
}

/// Result of a stable strong-digest read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableRead {
    /// Authoritative content identity.
    pub content_digest: ContentDigest,
    /// Optional FNV prefilter only.
    pub fast_fingerprint: FastFingerprint,
    pub bytes: Vec<u8>,
    pub meta: FileMetaSnapshot,
}

/// Compare two snapshots for stability (identity + size + mtime when present).
pub fn meta_stable(a: &FileMetaSnapshot, b: &FileMetaSnapshot) -> bool {
    if a.size_bytes != b.size_bytes {
        return false;
    }
    match (a.identity, b.identity) {
        (Some(x), Some(y)) if x != y => return false,
        _ => {}
    }
    match (a.modified_at_ns, b.modified_at_ns) {
        (Some(x), Some(y)) if x != y => return false,
        _ => {}
    }
    true
}

/// Strong digest of in-memory bytes.
pub fn hash_bytes(data: &[u8]) -> ContentDigest {
    digest_bytes(data)
}

/// Fast fingerprint only (prefilter).
pub fn fingerprint_bytes(data: &[u8]) -> FastFingerprint {
    fast_fingerprint(data)
}

/// Validate size against quotas then return content if stable across two meta observations.
pub fn stable_read_from_bytes(
    initial: FileMetaSnapshot,
    data: &[u8],
    final_meta: FileMetaSnapshot,
    quotas: &IndexQuotaConfig,
) -> Result<StableRead, IndexError> {
    if data.len() as u64 > quotas.max_file_size_bytes {
        return Err(IndexError::FileTooLarge {
            size: data.len() as u64,
            max: quotas.max_file_size_bytes,
        });
    }
    if !meta_stable(&initial, &final_meta) {
        return Err(IndexError::ChangedDuringRead);
    }
    if final_meta.size_bytes != data.len() as u64 {
        return Err(IndexError::ChangedDuringRead);
    }
    Ok(StableRead {
        content_digest: hash_bytes(data),
        fast_fingerprint: fingerprint_bytes(data),
        bytes: data.to_vec(),
        meta: final_meta,
    })
}

/// Host: open, read bounded streaming hash, re-stat, verify stability.
#[cfg(feature = "host")]
pub fn stable_read_host(
    path: &str,
    max_size: u64,
    quotas: &IndexQuotaConfig,
) -> Result<StableRead, IndexError> {
    use std::fs::{self, File};
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;

    let meta1 = fs::metadata(path).map_err(|_| IndexError::Io("stat"))?;
    if !meta1.is_file() {
        return Err(IndexError::Io("not a file"));
    }
    let initial = snapshot_meta(&meta1);
    if initial.size_bytes > max_size.min(quotas.max_file_size_bytes) {
        return Err(IndexError::FileTooLarge {
            size: initial.size_bytes,
            max: max_size.min(quotas.max_file_size_bytes),
        });
    }

    let mut f = File::open(path).map_err(|_| IndexError::Io("open"))?;
    let mut buf = Vec::new();
    let limit = initial.size_bytes as usize;
    if limit > quotas.max_file_size_bytes as usize {
        return Err(IndexError::FileTooLarge {
            size: initial.size_bytes,
            max: quotas.max_file_size_bytes,
        });
    }
    buf.reserve(limit);
    let mut chunk = [0u8; 4096];
    let mut hasher = ContentDigestHasher::new();
    let mut total = 0usize;
    loop {
        let n = f.read(&mut chunk).map_err(|_| IndexError::Io("read"))?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n)
            .ok_or(IndexError::Internal("read overflow"))?;
        if total as u64 > quotas.max_file_size_bytes {
            return Err(IndexError::FileTooLarge {
                size: total as u64,
                max: quotas.max_file_size_bytes,
            });
        }
        hasher.update(&chunk[..n]);
        buf.extend_from_slice(&chunk[..n]);
    }
    let content_digest = hasher.finish();
    let fast = fingerprint_bytes(&buf);

    let meta2 = fs::metadata(path).map_err(|_| IndexError::Io("restat"))?;
    let final_meta = snapshot_meta(&meta2);
    if !meta_stable(&initial, &final_meta) || final_meta.size_bytes != buf.len() as u64 {
        return Err(IndexError::ChangedDuringRead);
    }
    Ok(StableRead {
        content_digest,
        fast_fingerprint: fast,
        bytes: buf,
        meta: final_meta,
    })
}

#[cfg(feature = "host")]
fn snapshot_meta(meta: &std::fs::Metadata) -> FileMetaSnapshot {
    use std::os::unix::fs::MetadataExt;
    let secs = meta.mtime() as u64;
    let nsec = meta.mtime_nsec() as u64;
    FileMetaSnapshot {
        size_bytes: meta.len(),
        modified_at_ns: Some(secs.saturating_mul(1_000_000_000).saturating_add(nsec)),
        identity: Some(FileIdentity {
            device: meta.dev(),
            inode: meta.ino(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ok() {
        let q = IndexQuotaConfig::default();
        let m = FileMetaSnapshot {
            size_bytes: 5,
            modified_at_ns: Some(1),
            identity: Some(FileIdentity {
                device: 1,
                inode: 2,
            }),
        };
        let r = stable_read_from_bytes(m, b"hello", m, &q).unwrap();
        assert_eq!(r.content_digest, hash_bytes(b"hello"));
        assert!(r.content_digest.is_set());
    }

    #[test]
    fn changed_rejected() {
        let q = IndexQuotaConfig::default();
        let a = FileMetaSnapshot {
            size_bytes: 5,
            modified_at_ns: Some(1),
            identity: None,
        };
        let b = FileMetaSnapshot {
            size_bytes: 6,
            modified_at_ns: Some(2),
            identity: None,
        };
        assert!(matches!(
            stable_read_from_bytes(a, b"hello", b, &q),
            Err(IndexError::ChangedDuringRead)
        ));
    }
}
