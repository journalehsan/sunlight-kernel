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
use crate::digest::{ContentDigest, FastFingerprint, LegacyFnvContentHash};
use crate::hash::StablePathHash;
use crate::import_key::ImportKey;

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
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::PermissionDenied), 2 => Some(Self::FileTooLarge),
            3 => Some(Self::UnsupportedFormat), 4 => Some(Self::InvalidUtf8),
            5 => Some(Self::BinaryContent), 6 => Some(Self::ChangedDuringRead),
            7 => Some(Self::ParseFailed), 8 => Some(Self::TokenizationFailed),
            9 => Some(Self::DatabaseUnavailable), 10 => Some(Self::TransactionRejected),
            11 => Some(Self::QuotaExceeded), 12 => Some(Self::SourceDisappeared),
            13 => Some(Self::PathRejected), 14 => Some(Self::ImportConflict),
            15 => Some(Self::DigestMigrationFailed), _ => None,
        }
    }

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
    ///
    /// Content-stable rejections (`QuotaExceeded`, parse/tokenize failures with a
    /// stored strong digest) must not re-enter the parser on every unchanged
    /// scan. Digest or policy change still forces reclassification.
    pub const fn is_permanent(self) -> bool {
        matches!(
            self,
            Self::UnsupportedFormat
                | Self::InvalidUtf8
                | Self::BinaryContent
                | Self::FileTooLarge
                | Self::PathRejected
                | Self::ImportConflict
                | Self::QuotaExceeded
                | Self::ParseFailed
                | Self::TokenizationFailed
        )
    }
}

/// Failure tracking record (rejection identity without raw source content).
///
/// Durable rejected-source cache identity is the combination of:
/// - strong content digest on the parent [`SourceManifest`]
/// - size_bytes on the parent manifest
/// - pipeline versions (validator / parser / tokenizer / ignore) on the parent
/// - this failure kind
///
/// Confirmation bookkeeping lives here so unchanged rejected files can be
/// reconfirmed without re-running the parser or tokenizer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct SourceFailure {
    pub kind: SourceFailureKind,
    pub first_failure_ns: u64,
    pub latest_failure_ns: u64,
    /// Total classification attempts (new + cached confirmations).
    pub attempt_count: u32,
    /// Cached rejection confirmations after the first durable rejection.
    pub confirmation_count: u32,
    /// Hash of relevant source metadata at failure (path / size identity; not payload).
    pub metadata_hash: u64,
    pub retry_after_ns: u64,
    /// Validator version active when this rejection was recorded.
    pub validator_version: u32,
}

/// Pending import metadata persisted before MemoryDB commit (crash window).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct PendingImport {
    pub format_version: u16,
    pub import_key: ImportKey,
    pub source_id: SourceId,
    pub expected_revision: u32,
    pub content_digest: ContentDigest,
    pub pipeline_versions: PipelineVersions,
    pub state: PendingImportState,
    pub created_at: u64,
    pub latest_attempt_at: u64,
    pub attempt_count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum PendingImportState {
    Prepared = 1,
    TransactionStarted = 2,
    CommitSent = 3,
    ReconcileRequired = 4,
    Committed = 5,
    Aborted = 6,
    Conflict = 7,
}

impl PendingImportState {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Prepared), 2 => Some(Self::TransactionStarted),
            3 => Some(Self::CommitSent), 4 => Some(Self::ReconcileRequired),
            5 => Some(Self::Committed), 6 => Some(Self::Aborted),
            7 => Some(Self::Conflict), _ => None,
        }
    }
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
    pub legacy_content_hash: Option<LegacyFnvContentHash>,
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
        self.legacy_content_hash = Some(LegacyFnvContentHash::new(legacy_fnv));
        self.content_digest = ContentDigest::unset();
        self.needs_digest_upgrade = true;
        self.fast_fingerprint = Some(FastFingerprint::new(legacy_fnv));
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
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
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

    /// Durable rejected-source cache hit: same strong digest, size, policy versions,
    /// and permanent failure kind. Does not authorize parse/tokenize skip for accepted files.
    pub fn can_reuse_rejection(
        &self,
        digest: &ContentDigest,
        size_bytes: u64,
        v: &PipelineVersions,
        validator_version: u32,
    ) -> bool {
        if self.state != SourceState::Failed {
            return false;
        }
        let Some(ref f) = self.failure else {
            return false;
        };
        f.kind.is_permanent()
            && self.has_strong_digest()
            && self.content_digest.equals(digest)
            && self.size_bytes == size_bytes
            && self.pipeline_matches(v)
            && f.validator_version == validator_version
    }
}

/// Text validator identity version (bump when UTF-8/binary heuristics change).
pub const VALIDATOR_VERSION: u32 = 1;

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
    fn rejection_reuse_requires_strong_digest_and_policy() {
        let dig = digest_bytes(b"\xff\xfe");
        let mut m = SourceManifest::new_v2(
            SourceId::from_raw_unchecked(2),
            1,
            MemoryScope::User,
            1,
            String::from("bad.txt"),
            2,
            dig,
            None,
        );
        m.state = SourceState::Failed;
        m.size_bytes = 2;
        m.parser_id = 1;
        m.parser_version = 1;
        m.tokenizer_id = 1;
        m.tokenizer_version = 1;
        m.chunking_id = 1;
        m.chunking_version = 1;
        m.ignore_config_version = 1;
        m.failure = Some(SourceFailure {
            kind: SourceFailureKind::InvalidUtf8,
            first_failure_ns: 1,
            latest_failure_ns: 1,
            attempt_count: 1,
            confirmation_count: 0,
            metadata_hash: 0,
            retry_after_ns: u64::MAX,
            validator_version: VALIDATOR_VERSION,
        });
        let v = PipelineVersions {
            parser_id: 1,
            parser_version: 1,
            tokenizer_id: 1,
            tokenizer_version: 1,
            chunking_id: 1,
            chunking_version: 1,
            ignore_config_version: 1,
        };
        assert!(m.can_reuse_rejection(&dig, 2, &v, VALIDATOR_VERSION));
        assert!(!m.can_reuse_rejection(&digest_bytes(b"ok"), 2, &v, VALIDATOR_VERSION));
        assert!(!m.can_reuse_rejection(&dig, 3, &v, VALIDATOR_VERSION));
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
            fast_fingerprint: Some(FastFingerprint::new(99)),
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
        assert_eq!(m.legacy_content_hash, Some(LegacyFnvContentHash::new(0xdead)));
    }
}
