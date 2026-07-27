use super::context::BrainBudget;
use super::grounded::{
    AuthIdentity, BrainContextSource, ContextSourceKind, FactFreshness, FactKind, GroundedFact,
};
use super::protocol::MAX_HIGHLIGHT_VALUE;
use core::fmt::Write;

fn fact_str<const N: usize>(text: &str) -> heapless::String<N> {
    let mut out: heapless::String<N> = heapless::String::new();
    for c in text.chars().take(N) {
        let _ = out.push(c);
    }
    out
}

pub struct SessionContextSource;

impl BrainContextSource for SessionContextSource {
    fn source_kind(&self) -> ContextSourceKind { ContextSourceKind::Session }
    fn is_available(&self) -> bool { true }

    fn collect(
        &self,
        _budget: &BrainBudget,
        identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 16> {
        let mut facts: heapless::Vec<GroundedFact, 16> = heapless::Vec::new();

        let _ = facts.push(GroundedFact {
            kind: FactKind::UserId,
            source: ContextSourceKind::Session,
            freshness: FactFreshness::Current,
            confidence: 100,
            value: fact_str("authenticated"),
        });

        if identity.session_id != 0 {
            let mut v: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
            let _ = write!(&mut v, "{}", identity.session_id);
            let _ = facts.push(GroundedFact {
                kind: FactKind::SessionId,
                source: ContextSourceKind::Session,
                freshness: FactFreshness::Current,
                confidence: 100,
                value: v,
            });
        }

        facts
    }
}

pub struct SystemContextSource;

impl BrainContextSource for SystemContextSource {
    fn source_kind(&self) -> ContextSourceKind { ContextSourceKind::System }
    fn is_available(&self) -> bool { true }

    fn collect(
        &self,
        _budget: &BrainBudget,
        _identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 16> {
        let mut facts: heapless::Vec<GroundedFact, 16> = heapless::Vec::new();

        #[cfg(feature = "sunlightos")]
        {
            let info = sunlight_ipc::sysinfo();
            let ram_mib = info.total_ram_kb / 1024;

            let mut v: heapless::String<MAX_HIGHLIGHT_VALUE> = heapless::String::new();
            if ram_mib >= 1024 {
                let _ = write!(&mut v, "{} GiB", ram_mib / 1024);
            } else {
                let _ = write!(&mut v, "{} MiB", ram_mib);
            }
            let _ = facts.push(GroundedFact {
                kind: FactKind::RamMib,
                source: ContextSourceKind::System,
                freshness: FactFreshness::Current,
                confidence: 100,
                value: v,
            });
        }

        facts
    }
}

pub struct KvContextSource;

impl BrainContextSource for KvContextSource {
    fn source_kind(&self) -> ContextSourceKind { ContextSourceKind::Kv }
    fn is_available(&self) -> bool { false }

    fn collect(
        &self,
        _budget: &BrainBudget,
        _identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 16> {
        heapless::Vec::new()
    }
}

pub struct WiseOwlStatusContextSource;

impl BrainContextSource for WiseOwlStatusContextSource {
    fn source_kind(&self) -> ContextSourceKind { ContextSourceKind::WiseOwlStatus }
    fn is_available(&self) -> bool { false }

    fn collect(
        &self,
        _budget: &BrainBudget,
        _identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 16> {
        heapless::Vec::new()
    }
}

pub struct IndexContextSource;

impl BrainContextSource for IndexContextSource {
    fn source_kind(&self) -> ContextSourceKind { ContextSourceKind::Index }
    fn is_available(&self) -> bool { false }

    fn collect(
        &self,
        _budget: &BrainBudget,
        _identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 16> {
        heapless::Vec::new()
    }
}
