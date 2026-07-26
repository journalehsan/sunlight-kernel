//! Service health and degraded-state flags (Phase 1.1).

/// Service health exposed to supervisors and the diagnostic CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum ServiceHealth {
    Starting = 1,
    Ready = 2,
    Degraded = 3,
    Stopping = 4,
    Failed = 5,
}

impl ServiceHealth {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "Starting",
            Self::Ready => "Ready",
            Self::Degraded => "Degraded",
            Self::Stopping => "Stopping",
            Self::Failed => "Failed",
        }
    }

    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Starting),
            2 => Some(Self::Ready),
            3 => Some(Self::Degraded),
            4 => Some(Self::Stopping),
            5 => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Degraded reason bits.
pub mod degraded {
    pub const KV_UNAVAILABLE: u32 = 1 << 0;
    pub const SPILL_QUARANTINED: u32 = 1 << 1;
    pub const COLD_STORAGE_UNAVAILABLE: u32 = 1 << 2;
    pub const TELEMETRY_UNAVAILABLE: u32 = 1 << 3;
}
