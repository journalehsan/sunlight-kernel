//! Stable import identity for idempotent document generations.
//!
//! Import identity is independent of process-local transaction IDs.
//! Content identity (strong digest) is separate from token identity (FNV token IDs).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wiseowl_memory::SourceId;
use wiseowl_memorydb::record::{MemoryScope, OwnerId};

use crate::digest::{digest_bytes, ContentDigest};
use crate::hash::fnv1a64;

/// Stable import key components for one document generation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct ImportKey {
    pub source_id: SourceId,
    pub source_revision: u32,
    pub content_digest: ContentDigest,
    pub parser_id: u32,
    pub parser_version: u32,
    pub tokenizer_id: u32,
    pub tokenizer_version: u32,
    pub chunking_id: u32,
    pub chunking_version: u32,
    pub scope: MemoryScope,
    pub owner: OwnerId,
}

impl ImportKey {
    /// Encode a canonical LE byte stream for hashing.
    pub fn encode_canonical(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(35 + 48);
        buf.extend_from_slice(&self.source_id.get().to_le_bytes());
        buf.extend_from_slice(&self.source_revision.to_le_bytes());
        buf.extend_from_slice(&self.content_digest.encode());
        buf.extend_from_slice(&self.parser_id.to_le_bytes());
        buf.extend_from_slice(&self.parser_version.to_le_bytes());
        buf.extend_from_slice(&self.tokenizer_id.to_le_bytes());
        buf.extend_from_slice(&self.tokenizer_version.to_le_bytes());
        buf.extend_from_slice(&self.chunking_id.to_le_bytes());
        buf.extend_from_slice(&self.chunking_version.to_le_bytes());
        buf.push(self.scope.as_u8());
        buf.extend_from_slice(&self.owner.to_le_bytes());
        buf
    }

    /// Strong digest of the import key (hex for attributes / pending metadata).
    pub fn key_digest(&self) -> ContentDigest {
        digest_bytes(&self.encode_canonical())
    }

    pub fn key_hex(&self) -> String {
        self.key_digest().to_hex()
    }

    /// Compact u64 for secondary indexes only.
    pub fn fingerprint64(&self) -> u64 {
        fnv1a64(&self.encode_canonical())
    }
}

/// State of an import generation in MemoryDB (reconciliation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum ImportState {
    NotFound = 0,
    InProgress = 1,
    Committed = 2,
    Aborted = 3,
    Conflict = 4,
    AlreadyCommitted = 5,
}

impl ImportState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::InProgress => "in_progress",
            Self::Committed => "committed",
            Self::Aborted => "aborted",
            Self::Conflict => "conflict",
            Self::AlreadyCommitted => "already_committed",
        }
    }
}

/// Result of reconcile_import.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct ImportReconcileResult {
    pub state: ImportState,
    pub document_memory_id: Option<u64>,
    pub source_revision: Option<u32>,
}

/// Build import key from manifest + planned revision + digest.
pub fn build_import_key(
    source_id: SourceId,
    source_revision: u32,
    content_digest: ContentDigest,
    parser_id: u32,
    parser_version: u32,
    tokenizer_id: u32,
    tokenizer_version: u32,
    chunking_id: u32,
    chunking_version: u32,
    scope: MemoryScope,
    owner: OwnerId,
) -> ImportKey {
    ImportKey {
        source_id,
        source_revision,
        content_digest,
        parser_id,
        parser_version,
        tokenizer_id,
        tokenizer_version,
        chunking_id,
        chunking_version,
        scope,
        owner,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::digest_bytes;

    #[test]
    fn same_components_same_key() {
        let d = digest_bytes(b"body");
        let a = build_import_key(
            SourceId::from_raw_unchecked(1),
            2,
            d,
            1,
            1,
            1,
            1,
            1,
            1,
            MemoryScope::User,
            1,
        );
        let b = build_import_key(
            SourceId::from_raw_unchecked(1),
            2,
            d,
            1,
            1,
            1,
            1,
            1,
            1,
            MemoryScope::User,
            1,
        );
        assert_eq!(a.key_hex(), b.key_hex());
    }

    #[test]
    fn revision_changes_key() {
        let d = digest_bytes(b"body");
        let a = build_import_key(
            SourceId::from_raw_unchecked(1),
            1,
            d,
            1,
            1,
            1,
            1,
            1,
            1,
            MemoryScope::User,
            1,
        );
        let b = build_import_key(
            SourceId::from_raw_unchecked(1),
            2,
            d,
            1,
            1,
            1,
            1,
            1,
            1,
            MemoryScope::User,
            1,
        );
        assert_ne!(a.key_hex(), b.key_hex());
    }
}
