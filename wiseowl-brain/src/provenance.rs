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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrainResponseMeta {
    pub provider: BrainProviderKind,
    pub sources_consulted: ContextSourceMask,
    pub sources_degraded: ContextSourceMask,
    pub fact_count: u8,
    pub generation_time_us: u32,
}

impl BrainResponseMeta {
    pub const fn empty() -> Self {
        Self {
            provider: BrainProviderKind::Fallback,
            sources_consulted: ContextSourceMask(0),
            sources_degraded: ContextSourceMask(0),
            fact_count: 0,
            generation_time_us: 0,
        }
    }

    pub fn is_real_brain_response(&self) -> bool {
        matches!(self.provider, BrainProviderKind::LocalBounded | BrainProviderKind::FutureOnline)
    }

    pub fn is_fallback(&self) -> bool {
        matches!(self.provider, BrainProviderKind::Fallback)
    }
}
