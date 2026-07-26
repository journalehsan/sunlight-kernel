//! Memory class, kind, source, and trust enumerations (Phase 0 contracts).

/// Local short-term memory class.
///
/// Promotion into `sunlight-kv` is an **output operation**, not a fourth local class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum MemoryClass {
    /// Request-local mutable state with the shortest lifetime. RAM-only.
    Working = 1,
    /// Recently used session/context data kept in RAM, uncompressed while active.
    Hot = 2,
    /// Sealed short-term data eligible for compression and spill.
    Cold = 3,
}

impl MemoryClass {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Working),
            2 => Some(Self::Hot),
            3 => Some(Self::Cold),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Hot => "hot",
            Self::Cold => "cold",
        }
    }
}

/// Bounded, versioned memory payload kinds for Phase 0/1.
///
/// Deliberately excludes Fact, Pattern, Embedding, ModelWeights (later phases).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum MemoryKind {
    Input = 1,
    Observation = 2,
    ToolResult = 3,
    CandidateResponse = 4,
    SessionContext = 5,
    SessionSummary = 6,
    Feedback = 7,
    Diagnostic = 8,
}

impl MemoryKind {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Input),
            2 => Some(Self::Observation),
            3 => Some(Self::ToolResult),
            4 => Some(Self::CandidateResponse),
            5 => Some(Self::SessionContext),
            6 => Some(Self::SessionSummary),
            7 => Some(Self::Feedback),
            8 => Some(Self::Diagnostic),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Observation => "observation",
            Self::ToolResult => "tool_result",
            Self::CandidateResponse => "candidate_response",
            Self::SessionContext => "session_context",
            Self::SessionSummary => "session_summary",
            Self::Feedback => "feedback",
            Self::Diagnostic => "diagnostic",
        }
    }
}

/// Provenance source classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum SourceKind {
    UserInput = 1,
    LocalService = 2,
    LocalTool = 3,
    RemoteUnverified = 4,
    SystemGenerated = 5,
}

impl SourceKind {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::UserInput),
            2 => Some(Self::LocalService),
            3 => Some(Self::LocalTool),
            4 => Some(Self::RemoteUnverified),
            5 => Some(Self::SystemGenerated),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserInput => "user_input",
            Self::LocalService => "local_service",
            Self::LocalTool => "local_tool",
            Self::RemoteUnverified => "remote_unverified",
            Self::SystemGenerated => "system_generated",
        }
    }
}

/// Trust classification for payload content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum TrustLevel {
    Trusted = 1,
    Untrusted = 2,
    SystemDerived = 3,
}

impl TrustLevel {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Trusted),
            2 => Some(Self::Untrusted),
            3 => Some(Self::SystemDerived),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
            Self::SystemDerived => "system_derived",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_roundtrip() {
        for c in [MemoryClass::Working, MemoryClass::Hot, MemoryClass::Cold] {
            assert_eq!(MemoryClass::from_u8(c.as_u8()), Some(c));
        }
        assert_eq!(MemoryClass::from_u8(0), None);
        assert_eq!(MemoryClass::from_u8(99), None);
    }

    #[test]
    fn kind_no_future_variants() {
        // Ensure we reject values that would map to later-phase kinds.
        assert_eq!(MemoryKind::from_u8(0), None);
        assert_eq!(MemoryKind::from_u8(9), None);
        assert_eq!(MemoryKind::from_u8(255), None);
    }
}
