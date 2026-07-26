//! Errors for the long-term memory database.

use core::fmt;

/// Database and protocol errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbError {
    /// Requested record is not present (or not visible).
    NotFound,
    /// Record exists but is tombstoned (payload not returned by default).
    Tombstoned,
    /// Operation not allowed for the caller's capability set.
    PermissionDenied(&'static str),
    /// Hard quota would be exceeded.
    QuotaExceeded(&'static str),
    /// Transaction has too many operations or bytes.
    TransactionLimit(&'static str),
    /// No open transaction / wrong transaction id.
    InvalidTransaction,
    /// Duplicate MemoryId or conflicting insert.
    Conflict(&'static str),
    /// Payload or request body exceeds limits.
    PayloadTooLarge { size: u32, max: u32 },
    /// Malformed request body or protocol frame.
    InvalidRequest(&'static str),
    /// Unsupported protocol version.
    UnsupportedProtocolVersion { got: u16, want: u16 },
    /// Corrupt on-disk structure isolated to a file or region.
    Corrupt { reason: &'static str },
    /// Compression failed.
    CompressionFailure,
    /// Decompression failed (after size checks).
    DecompressionFailure,
    /// WAL recovery found an incomplete tail (not fatal for earlier commits).
    WalIncomplete,
    /// Index is degraded / incomplete for this query type.
    IndexDegraded(&'static str),
    /// Cursor is stale (generation or compaction changed).
    StaleCursor,
    /// Invalid enum or field value from disk/wire.
    InvalidValue(&'static str),
    /// Deduplication policy rejected the insert.
    DedupRejected,
    /// Exact payload already exists; existing id returned separately.
    DedupExisting,
    /// Service is not ready yet.
    NotReady,
    /// Internal invariant broken (bug or unrecoverable local state).
    Internal(&'static str),
    /// Filesystem / IO failure (host).
    Io(&'static str),
    /// OwlQL parse error.
    OwlQlParse(&'static str),
    /// Supersession would create a loop.
    SupersessionLoop,
    /// Tokenizer version mismatch for token index query.
    TokenizerMismatch,
    /// Source deletion batch still in progress (resume cursor returned).
    SourceDeleteInProgress,
    /// Cross-scope access denied.
    CrossScopeDenied,
    /// Trust escalation without capability.
    TrustEscalationDenied,
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "not found"),
            Self::Tombstoned => write!(f, "tombstoned"),
            Self::PermissionDenied(c) => write!(f, "permission denied: {c}"),
            Self::QuotaExceeded(q) => write!(f, "quota exceeded: {q}"),
            Self::TransactionLimit(r) => write!(f, "transaction limit: {r}"),
            Self::InvalidTransaction => write!(f, "invalid transaction"),
            Self::Conflict(r) => write!(f, "conflict: {r}"),
            Self::PayloadTooLarge { size, max } => {
                write!(f, "payload too large: {size} > {max}")
            }
            Self::InvalidRequest(r) => write!(f, "invalid request: {r}"),
            Self::UnsupportedProtocolVersion { got, want } => {
                write!(f, "unsupported protocol version {got} (want {want})")
            }
            Self::Corrupt { reason } => write!(f, "corrupt: {reason}"),
            Self::CompressionFailure => write!(f, "compression failure"),
            Self::DecompressionFailure => write!(f, "decompression failure"),
            Self::WalIncomplete => write!(f, "wal incomplete tail"),
            Self::IndexDegraded(r) => write!(f, "index degraded: {r}"),
            Self::StaleCursor => write!(f, "stale cursor"),
            Self::InvalidValue(r) => write!(f, "invalid value: {r}"),
            Self::DedupRejected => write!(f, "dedup rejected"),
            Self::DedupExisting => write!(f, "dedup existing"),
            Self::NotReady => write!(f, "service not ready"),
            Self::Internal(r) => write!(f, "internal: {r}"),
            Self::Io(r) => write!(f, "io: {r}"),
            Self::OwlQlParse(r) => write!(f, "owlql parse: {r}"),
            Self::SupersessionLoop => write!(f, "supersession loop"),
            Self::TokenizerMismatch => write!(f, "tokenizer version mismatch"),
            Self::SourceDeleteInProgress => write!(f, "source delete in progress"),
            Self::CrossScopeDenied => write!(f, "cross-scope denied"),
            Self::TrustEscalationDenied => write!(f, "trust escalation denied"),
        }
    }
}

#[cfg(feature = "host")]
impl std::error::Error for DbError {}
