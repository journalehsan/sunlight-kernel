use core::fmt::Write;

use crate::context::BrainContext;
use crate::foundation::FoundationMemory;
use crate::protocol::{
    MAX_HIGHLIGHT_LABEL, MAX_HIGHLIGHT_VALUE, MAX_NAME_LEN,
};

/// Short-term memory: current request-local working context, fast and bounded.
#[derive(Debug, Clone)]
pub struct ShortTermMemory {
    pub current_context: BrainContext,
    pub request_facts: heapless::Vec<StmFact, 8>,
}

#[derive(Debug, Clone)]
pub struct StmFact {
    pub key: heapless::String<MAX_HIGHLIGHT_LABEL>,
    pub value: heapless::String<MAX_HIGHLIGHT_VALUE>,
}

impl ShortTermMemory {
    pub fn new(ctx: BrainContext) -> Self {
        Self {
            current_context: ctx,
            request_facts: heapless::Vec::new(),
        }
    }

    pub fn add_fact(&mut self, key: &str, value: &str) {
        let mut k: heapless::String<MAX_HIGHLIGHT_LABEL> = heapless::String::new();
        for c in key.chars().take(MAX_HIGHLIGHT_LABEL) {
            let _ = k.push(c);
        }
        let mut v: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
        for c in value.chars().take(MAX_HIGHLIGHT_VALUE) {
            let _ = v.push(c);
        }
        let _ = self.request_facts.push(StmFact { key: k, value: v });
    }
}

/// Medium-term memory: persisted user/session preferences and state via KV.
#[derive(Debug, Clone, Default)]
pub struct MediumTermMemory {
    pub user_preferences: heapless::Vec<MtmEntry, 8>,
    pub onboarding_state: Option<OnboardingState>,
}

#[derive(Debug, Clone)]
pub struct MtmEntry {
    pub key: heapless::String<MAX_HIGHLIGHT_LABEL>,
    pub value: heapless::String<MAX_HIGHLIGHT_VALUE>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingState {
    New,
    InProgress,
    Completed,
    Repeated,
}

impl Default for OnboardingState {
    fn default() -> Self {
        Self::New
    }
}

/// Long-term memory: indexed document knowledge (placeholder for future use).
#[derive(Debug, Clone, Default)]
pub struct LongTermMemory {
    pub has_indexed_docs: bool,
    pub indexed_doc_count: u64,
}

/// Foundation memory: immutable build-time identity and policy records.
#[derive(Debug, Clone, Default)]
pub struct FoundationMemoryLayer {
    pub loaded: bool,
    pub record_count: u16,
    pub token_count: u32,
}

impl FoundationMemoryLayer {
    pub fn from_foundation(foundation: Option<&FoundationMemory>) -> Self {
        match foundation {
            Some(foundation) => Self {
                loaded: true,
                record_count: foundation.record_count() as u16,
                token_count: foundation.token_count() as u32,
            },
            None => Self::default(),
        }
    }
}

/// Runtime context: live per-boot and per-session facts.
///
/// This is intentionally a placeholder in Foundation Memory v1. The shape exists
/// so later milestones can add runtime facts without changing layer boundaries.
#[derive(Debug, Clone, Default)]
pub struct RuntimeContextLayer {
    pub available: bool,
}

/// Bounded context set assembled from all three memory layers.
#[derive(Debug, Clone)]
pub struct BoundedContextSet {
    pub foundation: FoundationMemoryLayer,
    pub stm: ShortTermMemory,
    pub mtm: MediumTermMemory,
    pub ltm: LongTermMemory,
    pub runtime: RuntimeContextLayer,
    pub machine_summary_available: bool,
    pub network_online: Option<bool>,
}

impl BoundedContextSet {
    pub fn from_context(ctx: BrainContext) -> Self {
        let stm = ShortTermMemory::new(ctx);
        let machine_available =
            stm.current_context.cpu_cores.is_some() || stm.current_context.ram_mib.is_some();
        Self {
            foundation: FoundationMemoryLayer::default(),
            stm,
            mtm: MediumTermMemory::default(),
            ltm: LongTermMemory::default(),
            runtime: RuntimeContextLayer::default(),
            machine_summary_available: machine_available,
            network_online: None,
        }
    }

    /// Build relevant facts from context for greeting generation.
    pub fn relevant_facts(&self) -> heapless::Vec<StmFact, 8> {
        let mut facts: heapless::Vec<StmFact, 8> = heapless::Vec::new();
        let ctx = &self.stm.current_context;

        if !ctx.user_display_name.is_empty() {
            let mut n: heapless::String<MAX_NAME_LEN> = heapless::String::new();
            for c in ctx.user_display_name.chars().take(MAX_NAME_LEN) {
                let _ = n.push(c);
            }
            let mut f: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
            for c in n.chars() {
                let _ = f.push(c);
            }
            let _ = facts.push(StmFact {
                key: s("user_name"),
                value: f,
            });
        }

        if let Some(cores) = ctx.cpu_cores {
            let mut v: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
            let _ = write!(&mut v, "{} cores", cores);
            let _ = facts.push(StmFact { key: s("cpu"), value: v });
        }

        if let Some(ram) = ctx.ram_mib {
            let mut v: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
            if ram >= 1024 {
                let _ = write!(&mut v, "{} GiB", ram / 1024);
            } else {
                let _ = write!(&mut v, "{} MiB", ram);
            }
            let _ = facts.push(StmFact { key: s("ram"), value: v });
        }

        if !ctx.model_name.is_empty() {
            let mut val: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
            for c in ctx.model_name.chars() {
                let _ = val.push(c);
            }
            let _ = facts.push(StmFact {
                key: s("model"),
                value: val,
            });
        }

        if !ctx.sunlight_version.is_empty() {
            let mut val: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
            for c in ctx.sunlight_version.chars() {
                let _ = val.push(c);
            }
            let _ = facts.push(StmFact {
                key: s("os_version"),
                value: val,
            });
        }

        facts
    }
}

fn s(s: &str) -> heapless::String<MAX_HIGHLIGHT_LABEL> {
    let mut out: heapless::String<MAX_HIGHLIGHT_LABEL> = heapless::String::new();
    for c in s.chars().take(MAX_HIGHLIGHT_LABEL) {
        let _ = out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ContextBuilder;

    #[test]
    fn stm_bounded_facts() {
        let ctx = ContextBuilder::new()
            .user_id(1000)
            .user_display_name("Alice")
            .cpu_cores(Some(8))
            .ram_mib(Some(16384))
            .build();
        let stm = ShortTermMemory::new(ctx);
        assert!(!stm.current_context.user_display_name.is_empty());
    }

    #[test]
    fn bounded_context_set_facts() {
        let ctx = ContextBuilder::new()
            .user_id(1000)
            .user_display_name("Bob")
            .cpu_cores(Some(4))
            .ram_mib(Some(8192))
            .model_name("TestBox")
            .sunlight_version("0.2.0")
            .build();
        let bcs = BoundedContextSet::from_context(ctx);
        let facts = bcs.relevant_facts();
        assert!(facts.iter().any(|f| f.key == "user_name"));
        assert!(facts.iter().any(|f| f.key == "cpu"));
        assert!(facts.iter().any(|f| f.key == "ram"));
        assert!(facts.iter().any(|f| f.key == "model"));
        assert!(facts.iter().any(|f| f.key == "os_version"));
        assert!(facts.len() <= 8);
    }

    #[test]
    fn empty_context_safe() {
        let ctx = ContextBuilder::new().user_id(1000).build();
        let bcs = BoundedContextSet::from_context(ctx);
        let facts = bcs.relevant_facts();
        assert!(facts.is_empty() || facts.iter().all(|f| !f.value.is_empty()));
    }
}
