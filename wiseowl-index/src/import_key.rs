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

pub const IMPORT_KEY_PROTOCOL_VERSION: u16 = 1;
pub const IMPORT_KEY_ENCODED_LEN: usize = 86;

/// Stable import key components for one document generation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct ImportKey {
    pub protocol_version: u16,
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
    pub ingestion_config_generation: u32,
}

impl ImportKey {
    /// Encode a canonical LE byte stream for hashing.
    pub fn encode_canonical(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(IMPORT_KEY_ENCODED_LEN);
        buf.extend_from_slice(&self.protocol_version.to_le_bytes());
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
        buf.extend_from_slice(&self.ingestion_config_generation.to_le_bytes());
        buf
    }

    pub fn decode_canonical(data: &[u8]) -> Result<Self, crate::error::IndexError> {
        use crate::error::IndexError;
        if data.len() != IMPORT_KEY_ENCODED_LEN {
            return Err(IndexError::InvalidValue("import key length"));
        }
        let protocol_version = u16::from_le_bytes(data[0..2].try_into().unwrap());
        if protocol_version != IMPORT_KEY_PROTOCOL_VERSION {
            return Err(IndexError::InvalidValue("import key protocol version"));
        }
        let source_id = SourceId::from_raw(u64::from_le_bytes(data[2..10].try_into().unwrap()))
            .map_err(|_| IndexError::InvalidValue("import key source id"))?;
        let source_revision = u32::from_le_bytes(data[10..14].try_into().unwrap());
        if source_revision == 0 {
            return Err(IndexError::InvalidValue("import key source revision"));
        }
        let content_digest = ContentDigest::decode(&data[14..49])?;
        let parser_id = u32::from_le_bytes(data[49..53].try_into().unwrap());
        let parser_version = u32::from_le_bytes(data[53..57].try_into().unwrap());
        let tokenizer_id = u32::from_le_bytes(data[57..61].try_into().unwrap());
        let tokenizer_version = u32::from_le_bytes(data[61..65].try_into().unwrap());
        let chunking_id = u32::from_le_bytes(data[65..69].try_into().unwrap());
        let chunking_version = u32::from_le_bytes(data[69..73].try_into().unwrap());
        let scope = MemoryScope::from_u8(data[73])
            .ok_or(IndexError::InvalidValue("import key scope"))?;
        let owner = u64::from_le_bytes(data[74..82].try_into().unwrap());
        let ingestion_config_generation =
            u32::from_le_bytes(data[82..86].try_into().unwrap());
        Ok(Self {
            protocol_version,
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
            ingestion_config_generation,
        })
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
    ingestion_config_generation: u32,
) -> ImportKey {
    ImportKey {
        protocol_version: IMPORT_KEY_PROTOCOL_VERSION,
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
        ingestion_config_generation,
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
            1,
        );
        assert_ne!(a.key_hex(), b.key_hex());
    }

    #[test]
    fn canonical_roundtrip_and_malformed_rejection() {
        let key = build_import_key(
            SourceId::from_raw_unchecked(9),
            3,
            digest_bytes(b"body"),
            1,
            2,
            3,
            4,
            5,
            6,
            MemoryScope::User,
            7,
            8,
        );
        let bytes = key.encode_canonical();
        assert_eq!(bytes.len(), IMPORT_KEY_ENCODED_LEN);
        assert_eq!(ImportKey::decode_canonical(&bytes).unwrap(), key);
        assert!(ImportKey::decode_canonical(&bytes[..85]).is_err());
        let mut unsupported = bytes;
        unsupported[0..2].copy_from_slice(&2u16.to_le_bytes());
        assert!(ImportKey::decode_canonical(&unsupported).is_err());
    }
}
