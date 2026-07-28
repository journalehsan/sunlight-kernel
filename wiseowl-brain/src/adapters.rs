//! Context source adapters. Live operating-system state enters through
//! `RuntimeContextSource`; optional persisted sources degrade cleanly.

use super::context::BrainBudget;
use super::foundation::FoundationMemory;
use super::grounded::{
    AuthIdentity, BrainContextSource, ContextSourceKind, FactFreshness, FactKind, GroundedFact,
};
use super::mtm::{self, BrainPreferences, WelcomeMemoryState};
use super::protocol::MAX_HIGHLIGHT_VALUE;
use super::runtime_context::RuntimeContextSnapshot;
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
    ) -> heapless::Vec<GroundedFact, 32> {
        #[allow(unused_mut)]
        let mut facts: heapless::Vec<GroundedFact, 32> = heapless::Vec::new();

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

pub struct FoundationContextSource<'a> {
    pub foundation: Option<&'a FoundationMemory>,
}

impl<'a> BrainContextSource for FoundationContextSource<'a> {
    fn source_kind(&self) -> ContextSourceKind {
        ContextSourceKind::Foundation
    }

    fn is_available(&self) -> bool {
        self.foundation.is_some()
    }

    fn collect(
        &self,
        budget: &BrainBudget,
        _identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 32> {
        let Some(foundation) = self.foundation else {
            return heapless::Vec::new();
        };
        let mut facts: heapless::Vec<GroundedFact, 32> = heapless::Vec::new();
        let max = budget.max_facts as usize;
        for record in foundation.records.iter() {
            if facts.len() >= max {
                break;
            }
            let _ = facts.push(GroundedFact {
                kind: record.key.fact_kind(),
                source: ContextSourceKind::Foundation,
                freshness: FactFreshness::Persisted,
                confidence: 100,
                value: fact_str(record.value.as_str()),
            });
        }
        facts
    }
}

pub struct RuntimeContextSource<'a> {
    pub snapshot: &'a RuntimeContextSnapshot,
}

impl<'a> BrainContextSource for RuntimeContextSource<'a> {
    fn source_kind(&self) -> ContextSourceKind {
        ContextSourceKind::Runtime
    }

    fn is_available(&self) -> bool {
        self.snapshot.available
    }

    fn collect(
        &self,
        budget: &BrainBudget,
        _identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 32> {
        let mut facts: heapless::Vec<GroundedFact, 32> = heapless::Vec::new();
        let max = budget.max_facts as usize;

        let mut push = |kind, freshness, value: &str| {
            if !value.is_empty() && facts.len() < max {
                let _ = facts.push(GroundedFact {
                    kind,
                    source: ContextSourceKind::Runtime,
                    freshness,
                    confidence: 100,
                    value: fact_str(value),
                });
            }
        };

        if let Some(version) = self.snapshot.system.os_version.as_ref() {
            push(FactKind::OsVersion, FactFreshness::CurrentBoot, version.as_str());
        }
        if let Some(build) = self.snapshot.system.build.as_ref() {
            push(FactKind::RuntimeBuild, FactFreshness::CurrentBoot, build.as_str());
        }
        if let Some(arch) = self.snapshot.system.architecture.as_ref() {
            push(
                FactKind::RuntimeArchitecture,
                FactFreshness::CurrentBoot,
                arch.as_str(),
            );
        }
        if let Some(locale) = self.snapshot.system.locale.as_ref() {
            push(FactKind::Locale, FactFreshness::CurrentBoot, locale.as_str());
        }
        if let Some(timezone) = self.snapshot.timezone.identifier.as_ref() {
            push(
                FactKind::RuntimeTimezone,
                FactFreshness::ServiceSnapshot,
                timezone.as_str(),
            );
        }
        if let Some(uptime) = self.snapshot.system.uptime_secs {
            let value = fact_u64(uptime);
            push(
                FactKind::RuntimeUptimeSecs,
                FactFreshness::ServiceSnapshot,
                value.as_str(),
            );
        }
        if let Some(hostname) = self.snapshot.system.hostname.as_ref() {
            push(
                FactKind::RuntimeHostname,
                FactFreshness::CurrentBoot,
                hostname.as_str(),
            );
        }
        if let Some(cpu_count) = self.snapshot.system.cpu_count {
            let value = fact_u64(cpu_count as u64);
            push(FactKind::CpuCores, FactFreshness::CurrentBoot, value.as_str());
        }
        if let Some(ram_mib) = self.snapshot.system.ram_mib {
            let value = fact_u64(ram_mib as u64);
            push(FactKind::RamMib, FactFreshness::CurrentBoot, value.as_str());
        }
        if let Some(user) = self.snapshot.session.current_user.as_ref() {
            push(FactKind::UserName, FactFreshness::CurrentSession, user.as_str());
        }
        if let Some(boot_mode) = self.snapshot.session.boot_mode.as_ref() {
            push(
                FactKind::RuntimeBootMode,
                FactFreshness::CurrentSession,
                boot_mode.as_str(),
            );
        }
        if let Some(desktop_mode) = self.snapshot.session.desktop_mode {
            push(
                FactKind::RuntimeDesktopMode,
                FactFreshness::CurrentSession,
                if desktop_mode { "1" } else { "0" },
            );
        }
        if let Some(installer_mode) = self.snapshot.session.installer_mode {
            push(
                FactKind::RuntimeInstallerMode,
                FactFreshness::CurrentSession,
                if installer_mode { "1" } else { "0" },
            );
        }
        if let Some(recovery_mode) = self.snapshot.session.recovery_mode {
            push(
                FactKind::RuntimeRecoveryMode,
                FactFreshness::CurrentSession,
                if recovery_mode { "1" } else { "0" },
            );
        }
        if let Some(session_state) = self.snapshot.session.state.as_ref() {
            push(
                FactKind::RuntimeSessionState,
                FactFreshness::CurrentSession,
                session_state.as_str(),
            );
        }
        if let Some(available) = self.snapshot.network.available {
            push(
                FactKind::RuntimeNetworkAvailable,
                FactFreshness::ServiceSnapshot,
                if available { "1" } else { "0" },
            );
        }
        if let Some(connected) = self.snapshot.network.connected {
            push(
                FactKind::NetworkOnline,
                FactFreshness::ServiceSnapshot,
                if connected { "1" } else { "0" },
            );
        }
        if let Some(interface_count) = self.snapshot.network.interface_count {
            let value = fact_u64(interface_count as u64);
            push(
                FactKind::RuntimeInterfaceCount,
                FactFreshness::ServiceSnapshot,
                value.as_str(),
            );
        }
        if let (Some(width), Some(height)) = (self.snapshot.display.width_px, self.snapshot.display.height_px) {
            let mut dims: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
            let _ = write!(&mut dims, "{}x{}", width, height);
            push(
                FactKind::ScreenDims,
                FactFreshness::ServiceSnapshot,
                dims.as_str(),
            );
        }
        if let Some(scale) = self.snapshot.display.scale_percent {
            let value = fact_u64(scale as u64);
            push(
                FactKind::RuntimeDisplayScale,
                FactFreshness::ServiceSnapshot,
                value.as_str(),
            );
        }

        push_service(&mut facts, max, FactKind::ServiceSunlightd, self.snapshot.services.sunlightd);
        push_service(&mut facts, max, FactKind::ServiceSessiond, self.snapshot.services.sessiond);
        push_service(&mut facts, max, FactKind::ServiceNetworkd, self.snapshot.services.networkd);
        push_service(&mut facts, max, FactKind::ServiceResolved, self.snapshot.services.resolved);
        push_service(
            &mut facts,
            max,
            FactKind::ServiceTimezone,
            self.snapshot.services.timezone_service,
        );
        push_service(&mut facts, max, FactKind::ServiceTimed, self.snapshot.services.timed);
        push_service(&mut facts, max, FactKind::ServicePowerd, self.snapshot.services.powerd);
        push_service(&mut facts, max, FactKind::ServiceThermald, self.snapshot.services.thermald);
        push_service(&mut facts, max, FactKind::ServiceDisplay, self.snapshot.services.display);

        facts
    }
}

fn push_service(
    facts: &mut heapless::Vec<GroundedFact, 32>,
    max: usize,
    kind: FactKind,
    status: Option<super::runtime_context::RuntimeServiceStatus>,
) {
    if let Some(status) = status {
        if facts.len() < max {
            let _ = facts.push(GroundedFact {
                kind,
                source: ContextSourceKind::Runtime,
                freshness: FactFreshness::ServiceSnapshot,
                confidence: 100,
                value: fact_str(status.as_str()),
            });
        }
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
    ) -> heapless::Vec<GroundedFact, 32> {
        let mut facts: heapless::Vec<GroundedFact, 32> = heapless::Vec::new();
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
    ) -> heapless::Vec<GroundedFact, 32> {
        let mut facts: heapless::Vec<GroundedFact, 32> = heapless::Vec::new();
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
    ) -> heapless::Vec<GroundedFact, 32> {
        let mut facts: heapless::Vec<GroundedFact, 32> = heapless::Vec::new();
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
