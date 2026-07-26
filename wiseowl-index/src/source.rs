//! Source identity, manifests, and failure tracking.
//!
//! # Manifest versions
//!
//! - **v1 (Phase 3):** FNV-1a64 final content hash (`legacy_content_hash`).
//! - **v2 (Phase 3.5):** strong [`ContentDigest`] + optional fast fingerprint.
//!
//! v1 manifests are never treated as having a strong digest; they are marked
//! for controlled upgrade that preserves `SourceId` when content is unchanged.

use alloc::string::String;

use wiseowl_memory::SourceId;
use wiseowl_memorydb::record::{MemoryScope, OwnerId};

use crate::config::RootId;
use crate::digest::{ContentDigest, FastFingerprint};
use crate::hash::StablePathHash;

/// Optional filesystem identity (inode + device) when available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}

/// Source lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum SourceState {
    Discovered = 1,
    Stable = 2,
    Indexed = 3,
    Changed = 4,
    Missing = 5,
    Failed = 6,
    Excluded = 7,
    DeletePending = 8,
    /// Awaiting MemoryDB commit reconciliation after crash window.
    ImportPending = 9,
}

impl SourceState {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Discovered),
            2 => Some(Self::Stable),
            3 => Some(Self::Indexed),
            4 => Some(Self::Changed),
            5 => Some(Self::Missing),
            6 => Some(Self::Failed),
            7 => Some(Self::Excluded),
            8 => Some(Self::DeletePending),
            9 => Some(Self::ImportPending),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Stable => "stable",
            Self::Indexed => "indexed",
            Self::Changed => "changed",
            Self::Missing => "missing",
            Self::Failed => "failed",
            Self::Excluded => "excluded",
            Self::DeletePending => "delete_pending",
            Self::ImportPending => "import_pending",
        }
    }
}

/// Bounded failure reason (no payloads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum SourceFailureKind {
    PermissionDenied = 1,
    FileTooLarge = 2,
    UnsupportedFormat = 3,
    InvalidUtf8 = 4,
    BinaryContent = 5,
    ChangedDuringRead = 6,
    ParseFailed = 7,
    TokenizationFailed = 8,
    DatabaseUnavailable = 9,
    TransactionRejected = 10,
    QuotaExceeded = 11,
    SourceDisappeared = 12,
    PathRejected = 13,
    ImportConflict = 14,
    DigestMigrationFailed = 15,
}

impl SourceFailureKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::FileTooLarge => "file_too_large",
            Self::UnsupportedFormat => "unsupported_format",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::BinaryContent => "binary_content",
            Self::ChangedDuringRead => "changed_during_read",
            Self::ParseFailed => "parse_failed",
            Self::TokenizationFailed => "tokenization_failed",
            Self::DatabaseUnavailable => "database_unavailable",
            Self::TransactionRejected => "transaction_rejected",
            Self::QuotaExceeded => "quota_exceeded",
            Self::SourceDisappeared => "source_disappeared",
            Self::PathRejected => "path_rejected",
            Self::ImportConflict => "import_conflict",
            Self::DigestMigrationFailed => "digest_migration_failed",
        }
    }

    /// Permanent failures are not retried until content/metadata identity changes.
    pub const fn is_permanent(self) -> bool {
        matches!(
            self,
            Self::UnsupportedFormat
                | Self::InvalidUtf8
                | Self::BinaryContent
                | Self::FileTooLarge
                | Self::PathRejected
                | Self::ImportConflict
        )
    }
}

/// Failure tracking record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct SourceFailure {
    pub kind: SourceFailureKind,
    pub first_failure_ns: u64,
    pub latest_failure_ns: u64,
    pub attempt_count: u32,
    /// Hash of relevant source metadata at failure (not payload).
    pub metadata_hash: u64,
    pub retry_after_ns: u64,
}

/// Pending import metadata persisted before MemoryDB commit (crash window).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct PendingImport {
    /// Stable import identity (hex of ImportKey digest).
    pub import_key_hex: String,
    pub source_revision: u32,
    pub content_digest: ContentDigest,
    pub started_at_ns: u64,
    /// Process-local tx id when known (not sole idempotency key).
    pub local_tx_id: Option<u64>,
}

/// Durable source manifest (indexer operational state + identity).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct SourceManifest {
    /// 1 = Phase 3 FNV final hash; 2 = Phase 3.5 strong digest.
    pub manifest_version: u16,
    pub source_id: SourceId,
    pub root_id: RootId,
    pub scope: MemoryScope,
    pub owner: OwnerId,
    pub relative_path: String,
    pub canonical_path_hash: StablePathHash,
    pub file_identity: Option<FileIdentity>,
    /// Strong content digest (authoritative for v2+). Unset until upgraded.
    pub content_digest: ContentDigest,
    /// Optional FNV prefilter fingerprint only.
    pub fast_fingerprint: Option<FastFingerprint>,
    /// Historical Phase 3 FNV content hash (never treated as strong identity).
    pub legacy_content_hash: Option<u64>,
    /// True when v1 manifest needs controlled strong-digest upgrade.
    pub needs_digest_upgrade: bool,
    pub size_bytes: u64,
    pub modified_at_ns: Option<u64>,
    pub parser_id: u32,
    pub parser_version: u32,
    pub tokenizer_id: u32,
    pub tokenizer_version: u32,
    pub chunking_id: u32,
    pub chunking_version: u32,
    pub ignore_config_version: u32,
    pub indexed_at_ns: u64,
    pub state: SourceState,
    pub chunk_count: u32,
    /// memorydb document record id (when indexed).
    pub document_memory_id: Option<u64>,
    pub source_revision: u32,
    pub missing_confirmations: u16,
    pub failure: Option<SourceFailure>,
    /// Persisted before begin_import for crash recovery.
    pub pending_import: Option<PendingImport>,
}

impl SourceManifest {
    /// Phase 3.5 manifest format.
    pub const MANIFEST_VERSION: u16 = 2;
    /// Phase 3 legacy manifests.
    pub const MANIFEST_VERSION_V1: u16 = 1;

    /// Construct a v2 manifest with strong digest.
    #[allow(clippy::too_many_arguments)]
    pub fn new_v2(
        source_id: SourceId,
        root_id: RootId,
        scope: MemoryScope,
        owner: OwnerId,
        relative_path: String,
        canonical_path_hash: StablePathHash,
        content_digest: ContentDigest,
        fast_fingerprint: Option<FastFingerprint>,
    ) -> Self {
        Self {
            manifest_version: Self::MANIFEST_VERSION,
            source_id,
            root_id,
            scope,
            owner,
            relative_path,
            canonical_path_hash,
            file_identity: None,
            content_digest,
            fast_fingerprint,
            legacy_content_hash: None,
            needs_digest_upgrade: false,
            size_bytes: 0,
            modified_at_ns: None,
            parser_id: 0,
            parser_version: 0,
            tokenizer_id: 0,
            tokenizer_version: 0,
            chunking_id: 0,
            chunking_version: 0,
            ignore_config_version: 0,
            indexed_at_ns: 0,
            state: SourceState::Discovered,
            chunk_count: 0,
            document_memory_id: None,
            source_revision: 0,
            missing_confirmations: 0,
            failure: None,
            pending_import: None,
        }
    }

    /// Upgrade a Phase 3 v1-shaped manifest (legacy FNV only) to v2 envelope.
    /// Does not claim a strong digest until rehash completes.
    pub fn mark_for_digest_upgrade(mut self, legacy_fnv: u64) -> Self {
        self.manifest_version = Self::MANIFEST_VERSION;
        self.legacy_content_hash = Some(legacy_fnv);
        self.content_digest = ContentDigest::unset();
        self.needs_digest_upgrade = true;
        self.fast_fingerprint = Some(legacy_fnv);
        self
    }

    /// Complete digest upgrade after strong rehash of unchanged content.
    pub fn complete_digest_upgrade(&mut self, digest: ContentDigest, fast: FastFingerprint) {
        self.content_digest = digest;
        self.fast_fingerprint = Some(fast);
        self.needs_digest_upgrade = false;
        self.manifest_version = Self::MANIFEST_VERSION;
    }

    pub fn has_strong_digest(&self) -> bool {
        self.manifest_version >= Self::MANIFEST_VERSION
            && self.content_digest.is_set()
            && !self.needs_digest_upgrade
    }
}

/// Pipeline versions that force re-index when any changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineVersions {
    pub parser_id: u32,
    pub parser_version: u32,
    pub tokenizer_id: u32,
    pub tokenizer_version: u32,
    pub chunking_id: u32,
    pub chunking_version: u32,
    pub ignore_config_version: u32,
}

impl SourceManifest {
    pub fn pipeline_matches(&self, v: &PipelineVersions) -> bool {
        self.parser_id == v.parser_id
            && self.parser_version == v.parser_version
            && self.tokenizer_id == v.tokenizer_id
            && self.tokenizer_version == v.tokenizer_version
            && self.chunking_id == v.chunking_id
            && self.chunking_version == v.chunking_version
            && self.ignore_config_version == v.ignore_config_version
    }

    /// Fast-path skip: same strong digest + pipeline + still Indexed.
    pub fn can_skip_reparse(&self, digest: &ContentDigest, v: &PipelineVersions) -> bool {
        self.state == SourceState::Indexed
            && self.has_strong_digest()
            && self.content_digest.equals(digest)
            && self.pipeline_matches(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::digest_bytes;
    use wiseowl_memory::SourceId;

    #[test]
    fn permanent_failures() {
        assert!(SourceFailureKind::InvalidUtf8.is_permanent());
        assert!(!SourceFailureKind::ChangedDuringRead.is_permanent());
    }

    #[test]
    fn skip_requires_indexed_and_strong_digest() {
        let dig = digest_bytes(b"hello");
        let m = SourceManifest {
            manifest_version: 2,
            source_id: SourceId::from_raw_unchecked(1),
            root_id: 1,
            scope: MemoryScope::User,
            owner: 1,
            relative_path: String::from("a.txt"),
            canonical_path_hash: 1,
            file_identity: None,
            content_digest: dig,
            fast_fingerprint: Some(99),
            legacy_content_hash: None,
            needs_digest_upgrade: false,
            size_bytes: 10,
            modified_at_ns: None,
            parser_id: 1,
            parser_version: 1,
            tokenizer_id: 1,
            tokenizer_version: 1,
            chunking_id: 1,
            chunking_version: 1,
            ignore_config_version: 1,
            indexed_at_ns: 1,
            state: SourceState::Indexed,
            chunk_count: 1,
            document_memory_id: Some(1),
            source_revision: 1,
            missing_confirmations: 0,
            failure: None,
            pending_import: None,
        };
        let v = PipelineVersions {
            parser_id: 1,
            parser_version: 1,
            tokenizer_id: 1,
            tokenizer_version: 1,
            chunking_id: 1,
            chunking_version: 1,
            ignore_config_version: 1,
        };
        assert!(m.can_skip_reparse(&dig, &v));
        assert!(!m.can_skip_reparse(&digest_bytes(b"other"), &v));
    }

    #[test]
    fn v1_upgrade_not_strong() {
        let m = SourceManifest::new_v2(
            SourceId::from_raw_unchecked(1),
            1,
            MemoryScope::User,
            1,
            String::from("a.txt"),
            1,
            ContentDigest::unset(),
            None,
        )
        .mark_for_digest_upgrade(0xdead);
        assert!(m.needs_digest_upgrade);
        assert!(!m.has_strong_digest());
        assert_eq!(m.legacy_content_hash, Some(0xdead));
    }
}
