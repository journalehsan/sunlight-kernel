//! Context source adapters. Session/System always available; KV/MemoryDB/Index
//! degrade cleanly when optional services are missing.

use super::context::BrainBudget;
use super::grounded::{
    AuthIdentity, BrainContextSource, ContextSourceKind, FactFreshness, FactKind, GroundedFact,
};
use super::mtm::{self, BrainPreferences, WelcomeMemoryState};
use super::protocol::MAX_HIGHLIGHT_VALUE;
use core::fmt::Write;

fn fact_str<const N: usize>(text: &str) -> heapless::String<N> {
    let mut out: heapless::String<N> = heapless::String::new();
    for c in text.chars().take(N) {
        let _ = out.push(c);
    }
    out
}

fn fact_u64(v: u64) -> heapless::String<MAX_HIGHLIGHT_VALUE> {
    let mut out: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
    let _ = write!(&mut out, "{}", v);
    out
}

pub struct SessionContextSource;

impl BrainContextSource for SessionContextSource {
    fn source_kind(&self) -> ContextSourceKind {
        ContextSourceKind::Session
    }
    fn is_available(&self) -> bool {
        true
    }

    fn collect(
        &self,
        _budget: &BrainBudget,
        identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 16> {
        let mut facts: heapless::Vec<GroundedFact, 16> = heapless::Vec::new();

        let _ = facts.push(GroundedFact {
            kind: FactKind::UserId,
            source: ContextSourceKind::Session,
            freshness: FactFreshness::CurrentSession,
            confidence: 100,
            value: fact_u64(identity.caller_uid),
        });

        if identity.session_id != 0 {
            let _ = facts.push(GroundedFact {
                kind: FactKind::SessionId,
                source: ContextSourceKind::Session,
                freshness: FactFreshness::CurrentSession,
                confidence: 100,
                value: fact_u64(identity.session_id),
            });
        }

        facts
    }
}

pub struct SystemContextSource;

impl BrainContextSource for SystemContextSource {
    fn source_kind(&self) -> ContextSourceKind {
        ContextSourceKind::System
    }
    fn is_available(&self) -> bool {
        true
    }

    fn collect(
        &self,
        _budget: &BrainBudget,
        _identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 16> {
        let mut facts: heapless::Vec<GroundedFact, 16> = heapless::Vec::new();

        #[cfg(feature = "sunlightos")]
        {
            let info = sunlight_ipc::sysinfo();
            let ram_mib = (info.total_ram_kb / 1024) as u32;
            if ram_mib > 0 {
                let mut v: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
                let _ = write!(&mut v, "{}", ram_mib);
                let _ = facts.push(GroundedFact {
                    kind: FactKind::RamMib,
                    source: ContextSourceKind::System,
                    freshness: FactFreshness::CurrentBoot,
                    confidence: 100,
                    value: v,
                });
            }
        }

        facts
    }
}

/// Injected MTM snapshot (loaded once per request by the pipeline / native body).
pub struct KvContextSource {
    pub loaded: bool,
    pub degraded: bool,
    pub welcome: WelcomeMemoryState,
    pub preferences: BrainPreferences,
    pub used_defaults: bool,
}

impl Default for KvContextSource {
    fn default() -> Self {
        Self {
            loaded: false,
            degraded: true,
            welcome: WelcomeMemoryState::default(),
            preferences: BrainPreferences::default(),
            used_defaults: true,
        }
    }
}

impl BrainContextSource for KvContextSource {
    fn source_kind(&self) -> ContextSourceKind {
        ContextSourceKind::Kv
    }
    fn is_available(&self) -> bool {
        self.loaded && !self.degraded
    }

    fn collect(
        &self,
        budget: &BrainBudget,
        _identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 16> {
        let mut facts: heapless::Vec<GroundedFact, 16> = heapless::Vec::new();
        if !self.loaded {
            return facts;
        }
        let max = budget.max_facts as usize;

        if facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::VisitCount,
                source: ContextSourceKind::Kv,
                freshness: FactFreshness::Persisted,
                confidence: if self.degraded { 50 } else { 100 },
                value: fact_u64(self.welcome.visit_count as u64),
            });
        }
        if let Some(gen) = self.welcome.last_completed_generation {
            if facts.len() < max {
                let _ = facts.push(GroundedFact {
                    kind: FactKind::LastCompletedGeneration,
                    source: ContextSourceKind::Kv,
                    freshness: FactFreshness::Persisted,
                    confidence: 100,
                    value: fact_u64(gen),
                });
            }
        }
        if facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::GreetingStyle,
                source: ContextSourceKind::Kv,
                freshness: FactFreshness::Persisted,
                confidence: 100,
                value: fact_str(self.preferences.greeting_style.as_str()),
            });
        }
        if facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::ShowMachineSummary,
                source: ContextSourceKind::Kv,
                freshness: FactFreshness::Persisted,
                confidence: 100,
                value: fact_str(if self.preferences.show_machine_summary {
                    "1"
                } else {
                    "0"
                }),
            });
        }
        if facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::ShowIndexStatus,
                source: ContextSourceKind::Kv,
                freshness: FactFreshness::Persisted,
                confidence: 100,
                value: fact_str(if self.preferences.show_index_status {
                    "1"
                } else {
                    "0"
                }),
            });
        }
        facts
    }
}

/// MemoryDB status-only adapter (no document retrieval).
pub struct WiseOwlStatusContextSource {
    pub available: bool,
    pub healthy: bool,
    pub generation: u64,
    pub record_count: u64,
    pub queried: bool,
    pub degraded: bool,
}

impl Default for WiseOwlStatusContextSource {
    fn default() -> Self {
        Self {
            available: false,
            healthy: false,
            generation: 0,
            record_count: 0,
            queried: false,
            degraded: true,
        }
    }
}

impl WiseOwlStatusContextSource {
    /// Query MemoryDB GetHealth + GetStats with short timeout (native only).
    #[cfg(feature = "sunlightos")]
    pub fn query_native() -> Self {
        use sunlight_ipc::{ipc_call_timeout, nameserver_lookup_timeout, IpcMsg};

        const EP: &str = "wiseowl.memorydb.v1";
        const GET_HEALTH: u64 = 0x4D0F;
        const GET_STATS: u64 = 0x4D0E;
        const REPLY: u64 = 0x4D80;
        const TIMEOUT: u64 = 40;

        let mut out = Self::default();
        let Some(cap) = nameserver_lookup_timeout(EP, TIMEOUT) else {
            out.degraded = true;
            out.queried = true;
            return out;
        };
        out.queried = true;
        match ipc_call_timeout(cap, IpcMsg::with_label(GET_HEALTH), TIMEOUT) {
            Ok(r) if r.label == REPLY => {
                out.available = true;
                out.healthy = r.words[0] != 0 && r.words[1] == 2; // Ready=2
                out.degraded = !out.healthy;
            }
            _ => {
                out.degraded = true;
                return out;
            }
        }
        if let Ok(r) = ipc_call_timeout(cap, IpcMsg::with_label(GET_STATS), TIMEOUT) {
            if r.label == REPLY {
                out.generation = r.words[0];
                out.record_count = r.words[2];
            }
        }
        out
    }

    #[cfg(not(feature = "sunlightos"))]
    pub fn query_native() -> Self {
        Self::default()
    }
}

impl BrainContextSource for WiseOwlStatusContextSource {
    fn source_kind(&self) -> ContextSourceKind {
        ContextSourceKind::WiseOwlStatus
    }
    fn is_available(&self) -> bool {
        self.available && !self.degraded
    }

    fn collect(
        &self,
        budget: &BrainBudget,
        _identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 16> {
        let mut facts: heapless::Vec<GroundedFact, 16> = heapless::Vec::new();
        if !self.queried {
            return facts;
        }
        let max = budget.max_facts as usize;
        if facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::MemoryDbAvailable,
                source: ContextSourceKind::WiseOwlStatus,
                freshness: FactFreshness::ServiceSnapshot,
                confidence: 100,
                value: fact_str(if self.available { "1" } else { "0" }),
            });
        }
        if self.available && facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::MemoryDbHealthy,
                source: ContextSourceKind::WiseOwlStatus,
                freshness: FactFreshness::ServiceSnapshot,
                confidence: if self.healthy { 100 } else { 40 },
                value: fact_str(if self.healthy { "1" } else { "0" }),
            });
        }
        if self.available && facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::MemoryDbGeneration,
                source: ContextSourceKind::WiseOwlStatus,
                freshness: FactFreshness::ServiceSnapshot,
                confidence: 100,
                value: fact_u64(self.generation),
            });
        }
        if self.available && facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::MemoryDbRecordCount,
                source: ContextSourceKind::WiseOwlStatus,
                freshness: FactFreshness::ServiceSnapshot,
                confidence: 100,
                value: fact_u64(self.record_count),
            });
        }
        facts
    }
}

/// Index status-only adapter (no document text).
pub struct IndexContextSource {
    pub available: bool,
    pub ready: bool,
    pub sources_tracked: u64,
    pub files_indexed: u64,
    pub generation: u64,
    pub queried: bool,
    pub degraded: bool,
}

impl Default for IndexContextSource {
    fn default() -> Self {
        Self {
            available: false,
            ready: false,
            sources_tracked: 0,
            files_indexed: 0,
            generation: 0,
            queried: false,
            degraded: true,
        }
    }
}

impl IndexContextSource {
    #[cfg(feature = "sunlightos")]
    pub fn query_native() -> Self {
        use sunlight_ipc::{ipc_call_timeout, nameserver_lookup_timeout, IpcMsg};

        const EP: &str = "wiseowl.index.v1";
        const GET_HEALTH: u64 = 0x4E0E;
        const GET_STATS: u64 = 0x4E0D;
        const REPLY: u64 = 0x4E80;
        const TIMEOUT: u64 = 40;

        let mut out = Self::default();
        let Some(cap) = nameserver_lookup_timeout(EP, TIMEOUT) else {
            out.degraded = true;
            out.queried = true;
            return out;
        };
        out.queried = true;
        match ipc_call_timeout(cap, IpcMsg::with_label(GET_HEALTH), TIMEOUT) {
            Ok(r) if r.label == REPLY => {
                out.available = true;
                // ready flag + state Ready=1 in index health (see HealthState)
                out.ready = r.words[0] != 0;
                out.generation = r.words[3];
                out.degraded = !out.ready;
            }
            _ => {
                out.degraded = true;
                return out;
            }
        }
        // page 0 stats: roots, files_indexed, …, sources_tracked, generations_created
        let stats_msg = IpcMsg::with_label(GET_STATS).word(0, 0);
        if let Ok(r) = ipc_call_timeout(cap, stats_msg, TIMEOUT) {
            if r.label == REPLY {
                out.files_indexed = r.words[1];
                out.sources_tracked = r.words[4];
                if out.generation == 0 {
                    out.generation = r.words[5];
                }
            }
        }
        out
    }

    #[cfg(not(feature = "sunlightos"))]
    pub fn query_native() -> Self {
        Self::default()
    }
}

impl BrainContextSource for IndexContextSource {
    fn source_kind(&self) -> ContextSourceKind {
        ContextSourceKind::Index
    }
    fn is_available(&self) -> bool {
        self.available
    }

    fn collect(
        &self,
        budget: &BrainBudget,
        _identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 16> {
        let mut facts: heapless::Vec<GroundedFact, 16> = heapless::Vec::new();
        if !self.queried {
            return facts;
        }
        let max = budget.max_facts as usize;
        if facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::IndexAvailable,
                source: ContextSourceKind::Index,
                freshness: FactFreshness::ServiceSnapshot,
                confidence: 100,
                value: fact_str(if self.available { "1" } else { "0" }),
            });
        }
        if self.available && facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::IndexReady,
                source: ContextSourceKind::Index,
                freshness: FactFreshness::ServiceSnapshot,
                confidence: if self.ready { 100 } else { 40 },
                value: fact_str(if self.ready { "1" } else { "0" }),
            });
        }
        if self.available && self.ready && self.sources_tracked > 0 && facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::IndexedSourceCount,
                source: ContextSourceKind::Index,
                freshness: FactFreshness::ServiceSnapshot,
                confidence: 100,
                value: fact_u64(self.sources_tracked),
            });
        }
        if self.available && self.files_indexed > 0 && facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::IndexedDocCount,
                source: ContextSourceKind::Index,
                freshness: FactFreshness::ServiceSnapshot,
                confidence: 100,
                value: fact_u64(self.files_indexed),
            });
        }
        if self.available && facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind: FactKind::IndexGeneration,
                source: ContextSourceKind::Index,
                freshness: FactFreshness::ServiceSnapshot,
                confidence: 100,
                value: fact_u64(self.generation),
            });
        }
        let _ = mtm::GreetingStyle::Concise; // keep mtm linked in host builds
        facts
    }
}
