use crate::context::BrainBudget;
use crate::protocol::MAX_HIGHLIGHT_VALUE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ContextSourceKind {
    Session = 1,
    System = 2,
    Kv = 3,
    WiseOwlStatus = 4,
    Index = 5,
    Foundation = 6,
    Runtime = 7,
    Request = 8,
}

impl ContextSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::System => "system",
            Self::Kv => "kv",
            Self::WiseOwlStatus => "wiseowl-status",
            Self::Index => "index",
            Self::Foundation => "foundation",
            Self::Runtime => "runtime",
            Self::Request => "request",
        }
    }

    pub const fn bit(self) -> u8 {
        1u8 << ((self as u8) - 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContextSourceMask(pub u8);

impl ContextSourceMask {
    pub const fn empty() -> Self {
        Self(0)
    }
    pub fn add(&mut self, kind: ContextSourceKind) {
        self.0 |= kind.bit();
    }
    pub fn has(self, kind: ContextSourceKind) -> bool {
        self.0 & kind.bit() != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FactKind {
    UserName = 1,
    UserId = 2,
    SessionId = 3,
    FirstLogin = 4,
    FirstAfterUpgrade = 5,
    Locale = 6,
    OsVersion = 7,
    CpuCores = 8,
    RamMib = 9,
    DeviceClass = 10,
    ModelName = 11,
    ScreenDims = 12,
    NetworkOnline = 13,
    IndexReady = 14,
    IndexedDocCount = 15,
    IndexGeneration = 16,
    VisitCount = 17,
    GreetingStyle = 18,
    ShowMachineSummary = 19,
    ShowIndexStatus = 20,
    MemoryDbAvailable = 21,
    MemoryDbHealthy = 22,
    MemoryDbGeneration = 23,
    MemoryDbRecordCount = 24,
    IndexAvailable = 25,
    IndexedSourceCount = 26,
    LastCompletedGeneration = 27,
    SystemGeneration = 28,
    AssistantName = 29,
    AssistantCodename = 30,
    SunlightIdentity = 31,
    FoundationRole = 32,
    FoundationCapabilities = 33,
    FoundationSafety = 34,
    FoundationSecurityModel = 35,
    FoundationRuntimeInfo = 36,
    FoundationMemoryModel = 37,
    RuntimeBootMode = 38,
    RuntimeDesktopMode = 39,
    RuntimeInstallerMode = 40,
    RuntimeRecoveryMode = 41,
    RuntimeBuild = 42,
    RuntimeArchitecture = 43,
    RuntimeTimezone = 44,
    RuntimeUptimeSecs = 45,
    RuntimeHostname = 46,
    RuntimeSessionState = 47,
    RuntimeNetworkAvailable = 48,
    RuntimeInterfaceCount = 49,
    RuntimeDisplayScale = 50,
    ServiceSunlightd = 51,
    ServiceSessiond = 52,
    ServiceNetworkd = 53,
    ServiceResolved = 54,
    ServiceTimezone = 55,
    ServiceTimed = 56,
    ServicePowerd = 57,
    ServiceThermald = 58,
    ServiceDisplay = 59,
}

/// Freshness categories (lower ordinal wins on conflict).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FactFreshness {
    RequestLocal = 0,
    CurrentSession = 1,
    CurrentBoot = 2,
    Persisted = 3,
    ServiceSnapshot = 4,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundedFact {
    pub kind: FactKind,
    pub source: ContextSourceKind,
    pub freshness: FactFreshness,
    pub confidence: u8,
    pub value: heapless::String<MAX_HIGHLIGHT_VALUE>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthIdentity {
    pub caller_pid: u64,
    pub caller_uid: u64,
    pub session_id: u64,
}

impl AuthIdentity {
    pub const fn empty() -> Self {
        Self {
            caller_pid: 0,
            caller_uid: 0,
            session_id: 0,
        }
    }

    /// Kernel badge stamps a real PID; root uid 0 is a valid subject.
    pub fn is_authenticated(&self) -> bool {
        self.caller_pid != 0
    }
}

pub trait BrainContextSource {
    fn source_kind(&self) -> ContextSourceKind;
    fn is_available(&self) -> bool;
    fn collect(
        &self,
        budget: &BrainBudget,
        identity: &AuthIdentity,
    ) -> heapless::Vec<GroundedFact, 32>;
}

pub struct ContextSourceResult {
    pub facts: heapless::Vec<GroundedFact, 16>,
    pub source: ContextSourceKind,
    pub degraded: bool,
}

/// Prefer current (lower ordinal) freshness, then higher confidence.
pub fn prefer_fact<'a>(a: &'a GroundedFact, b: &'a GroundedFact) -> &'a GroundedFact {
    let fa = a.freshness as u8;
    let fb = b.freshness as u8;
    if fa != fb {
        if fa < fb {
            a
        } else {
            b
        }
    } else if a.confidence >= b.confidence {
        a
    } else {
        b
    }
}
