use super::grounded::ContextSourceMask;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BrainProviderKind {
    LocalBounded = 1,
    FutureOnline = 2,
    Fallback = 0xFF,
}

impl BrainProviderKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalBounded => "local-bounded",
            Self::FutureOnline => "future-online",
            Self::Fallback => "fallback",
        }
    }
}

/// Bounded response flags (bitmask).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BrainResponseFlags(pub u16);

impl BrainResponseFlags {
    pub const DEGRADED_CONTEXT: u16 = 1 << 0;
    pub const USED_DEFAULT_PREFERENCES: u16 = 1 << 1;
    pub const USED_PERSISTED_MEMORY: u16 = 1 << 2;
    pub const INDEX_READY: u16 = 1 << 3;
    pub const MACHINE_SUMMARY_INCLUDED: u16 = 1 << 4;
    pub const RETURNING_USER_GREETING: u16 = 1 << 5;
    pub const FIRST_VISIT_GREETING: u16 = 1 << 6;
    pub const AFTER_UPGRADE_GREETING: u16 = 1 << 7;
    pub const MEMORYDB_HEALTHY: u16 = 1 << 8;

    pub const fn empty() -> Self {
        Self(0)
    }
    pub fn set(&mut self, bit: u16) {
        self.0 |= bit;
    }
    pub fn has(self, bit: u16) -> bool {
        self.0 & bit != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrainResponseMeta {
    pub provider: BrainProviderKind,
    pub sources_consulted: ContextSourceMask,
    pub sources_succeeded: ContextSourceMask,
    pub sources_degraded: ContextSourceMask,
    pub fact_count: u8,
    pub generation_time_us: u32,
    pub used_persisted_context: bool,
    pub response_flags: BrainResponseFlags,
}

impl BrainResponseMeta {
    pub const fn empty() -> Self {
        Self {
            provider: BrainProviderKind::Fallback,
            sources_consulted: ContextSourceMask(0),
            sources_succeeded: ContextSourceMask(0),
            sources_degraded: ContextSourceMask(0),
            fact_count: 0,
            generation_time_us: 0,
            used_persisted_context: false,
            response_flags: BrainResponseFlags(0),
        }
    }

    pub fn is_real_brain_response(&self) -> bool {
        matches!(
            self.provider,
            BrainProviderKind::LocalBounded | BrainProviderKind::FutureOnline
        )
    }

    pub fn is_fallback(&self) -> bool {
        matches!(self.provider, BrainProviderKind::Fallback)
    }
}
