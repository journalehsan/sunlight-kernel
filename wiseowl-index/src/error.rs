//! Indexer errors (bounded, never carry raw file payloads).

use alloc::string::String;
use core::fmt;

/// Indexer error codes. Messages are diagnostic only — never file content.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum IndexError {
    InvalidRequest(&'static str),
    InvalidValue(&'static str),
    Unauthorized(&'static str),
    CapabilityDenied(&'static str),
    PathRejected(&'static str),
    RootNotFound,
    RootUnavailable,
    SourceNotFound,
    ScanAlreadyRunning,
    ScanBudgetExhausted,
    FileTooLarge { size: u64, max: u64 },
    InvalidUtf8,
    BinaryContent,
    ChangedDuringRead,
    UnsupportedFormat,
    ParseFailed(&'static str),
    TokenizationFailed(&'static str),
    TokenCollision,
    QuotaExceeded(&'static str),
    DatabaseUnavailable,
    TransactionRejected(String),
    Io(&'static str),
    NotConfigured(&'static str),
    Internal(&'static str),
    UnsupportedProtocolVersion { got: u16, want: u16 },
    PayloadTooLarge { size: u32, max: u32 },
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(s) => write!(f, "invalid request: {s}"),
            Self::InvalidValue(s) => write!(f, "invalid value: {s}"),
            Self::Unauthorized(s) => write!(f, "unauthorized: {s}"),
            Self::CapabilityDenied(s) => write!(f, "capability denied: {s}"),
            Self::PathRejected(s) => write!(f, "path rejected: {s}"),
            Self::RootNotFound => write!(f, "root not found"),
            Self::RootUnavailable => write!(f, "root unavailable"),
            Self::SourceNotFound => write!(f, "source not found"),
            Self::ScanAlreadyRunning => write!(f, "scan already running"),
            Self::ScanBudgetExhausted => write!(f, "scan budget exhausted"),
            Self::FileTooLarge { size, max } => write!(f, "file too large: {size} > {max}"),
            Self::InvalidUtf8 => write!(f, "invalid utf-8"),
            Self::BinaryContent => write!(f, "binary content"),
            Self::ChangedDuringRead => write!(f, "file changed during read"),
            Self::UnsupportedFormat => write!(f, "unsupported format"),
            Self::ParseFailed(s) => write!(f, "parse failed: {s}"),
            Self::TokenizationFailed(s) => write!(f, "tokenization failed: {s}"),
            Self::TokenCollision => write!(f, "token id collision"),
            Self::QuotaExceeded(s) => write!(f, "quota exceeded: {s}"),
            Self::DatabaseUnavailable => write!(f, "memory database unavailable"),
            Self::TransactionRejected(s) => write!(f, "transaction rejected: {s}"),
            Self::Io(s) => write!(f, "io: {s}"),
            Self::NotConfigured(s) => write!(f, "not configured: {s}"),
            Self::Internal(s) => write!(f, "internal: {s}"),
            Self::UnsupportedProtocolVersion { got, want } => {
                write!(f, "unsupported protocol version {got} (want {want})")
            }
            Self::PayloadTooLarge { size, max } => {
                write!(f, "payload too large: {size} > {max}")
            }
        }
    }
}

#[cfg(feature = "host")]
impl std::error::Error for IndexError {}

impl From<wiseowl_memorydb::DbError> for IndexError {
    fn from(e: wiseowl_memorydb::DbError) -> Self {
        match e {
            wiseowl_memorydb::DbError::QuotaExceeded(s) => Self::QuotaExceeded(s),
            wiseowl_memorydb::DbError::PayloadTooLarge { size, max } => {
                Self::PayloadTooLarge { size, max }
            }
            wiseowl_memorydb::DbError::CrossScopeDenied => Self::Unauthorized("cross-scope"),
            wiseowl_memorydb::DbError::PermissionDenied(s) => Self::CapabilityDenied(s),
            other => Self::TransactionRejected(alloc::format!("{other}")),
        }
    }
}
