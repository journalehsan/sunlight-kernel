use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrainError {
    InvalidRequest(&'static str),
    UnsupportedProtocolVersion { got: u16, want: u16 },
    PayloadTooLarge { size: u32, max: u32 },
    PermissionDenied(&'static str),
    Unauthorized,
    ProviderUnavailable,
    ProviderTimeout,
    ContextBuildFailed(&'static str),
    ResponseShapingFailed(&'static str),
    Internal(&'static str),
    UnsupportedRequestKind,
    TruncatedHeader,
    TruncatedBody,
    BadEncoding,
    UnknownOperation(u16),
}

#[cfg(feature = "host")]
impl fmt::Display for BrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(s) => write!(f, "invalid request: {}", s),
            Self::UnsupportedProtocolVersion { got, want } => {
                write!(f, "protocol version mismatch: got {}, want {}", got, want)
            }
            Self::PayloadTooLarge { size, max } => {
                write!(f, "payload too large: {} > {}", size, max)
            }
            Self::PermissionDenied(cap) => write!(f, "permission denied: {}", cap),
            Self::Unauthorized => write!(f, "unauthorized"),
            Self::ProviderUnavailable => write!(f, "provider unavailable"),
            Self::ProviderTimeout => write!(f, "provider timeout"),
            Self::ContextBuildFailed(s) => write!(f, "context build failed: {}", s),
            Self::ResponseShapingFailed(s) => write!(f, "response shaping failed: {}", s),
            Self::Internal(s) => write!(f, "internal error: {}", s),
            Self::UnsupportedRequestKind => write!(f, "unsupported request kind"),
            Self::TruncatedHeader => write!(f, "truncated header"),
            Self::TruncatedBody => write!(f, "truncated body"),
            Self::BadEncoding => write!(f, "bad encoding"),
            Self::UnknownOperation(op) => write!(f, "unknown operation: 0x{:04X}", op),
        }
    }
}

#[cfg(feature = "host")]
impl std::error::Error for BrainError {}

pub type BrainResult<T> = Result<T, BrainError>;
