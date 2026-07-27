//! Medium-term memory: typed, user-scoped, bounded Brain preferences and welcome state.
//!
//! Persistence is via sunlight-kv under compact keys (≤16 bytes for PUT_SHM/GET_SHM).
//! Missing or malformed values fall back to safe defaults; failed writes never fail greetings.

use core::fmt::Write;

use crate::provenance::BrainProviderKind;

/// Greeting tone (deterministic planning variants only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum GreetingStyle {
    #[default]
    Concise = 0,
    Friendly = 1,
    Technical = 2,
}

impl GreetingStyle {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Concise => "concise",
            Self::Friendly => "friendly",
            Self::Technical => "technical",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "concise" => Some(Self::Concise),
            "friendly" => Some(Self::Friendly),
            "technical" => Some(Self::Technical),
            _ => None,
        }
    }

    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Concise),
            1 => Some(Self::Friendly),
            2 => Some(Self::Technical),
            _ => None,
        }
    }
}

/// User-scoped welcome/onboarding counters (MTM).
///
/// `visit_count` = number of **successful completed Welcome visits** recorded via
/// the explicit WelcomeCompleted notification (not every greeting request).
/// Saturates at `u32::MAX` (no wrap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WelcomeMemoryState {
    pub visit_count: u32,
    pub last_completed_generation: Option<u64>,
    pub last_successful_provider: Option<BrainProviderKind>,
}

impl WelcomeMemoryState {
    pub fn is_returning_visit(&self) -> bool {
        self.visit_count > 0
    }

    pub fn record_successful_provider(&mut self, provider: BrainProviderKind) {
        self.last_successful_provider = Some(provider);
    }

    /// Explicit Welcome completion event (ownership remains with Welcome/session).
    pub fn record_completion(&mut self, system_generation: u64) {
        self.visit_count = self.visit_count.saturating_add(1);
        self.last_completed_generation = Some(system_generation);
    }

    pub fn is_after_upgrade(&self, current_generation: u64) -> bool {
        match self.last_completed_generation {
            Some(prev) if prev != current_generation && self.visit_count > 0 => true,
            _ => false,
        }
    }
}

/// User Brain preferences (defaults preserve current Welcome tone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrainPreferences {
    pub show_machine_summary: bool,
    pub show_index_status: bool,
    pub greeting_style: GreetingStyle,
}

impl Default for BrainPreferences {
    fn default() -> Self {
        Self {
            show_machine_summary: true,
            show_index_status: false,
            greeting_style: GreetingStyle::Concise,
        }
    }
}

// ── Compact KV key layout (≤16 bytes for sunlight-kv register packing) ──
//
// Pattern: `wb1:{hex_uid}:{code}`
// u32 uid as lowercase hex (1–8 chars) → max `wb1:ffffffff:sms` = 16.

pub const KEY_CODE_VISIT: &str = "vc";
pub const KEY_CODE_GEN: &str = "gen";
pub const KEY_CODE_PROV: &str = "lp";
pub const KEY_CODE_STYLE: &str = "gs";
pub const KEY_CODE_SMS: &str = "sms";
pub const KEY_CODE_SIS: &str = "sis";

/// Build a user-scoped key into `out`. Returns false if it would exceed 16 bytes.
pub fn format_user_key(uid: u64, code: &str, out: &mut heapless::String<16>) -> bool {
    out.clear();
    let uid32 = uid as u32;
    let mut ok = true;
    ok &= out.push_str("wb1:").is_ok();
    // hex without leading zeros except for 0
    let mut buf = [0u8; 8];
    let hex = write_hex_u32(uid32, &mut buf);
    ok &= out.push_str(hex).is_ok();
    ok &= out.push(':').is_ok();
    ok &= out.push_str(code).is_ok();
    ok && out.len() <= 16
}

fn write_hex_u32(v: u32, buf: &mut [u8; 8]) -> &str {
    if v == 0 {
        buf[0] = b'0';
        return core::str::from_utf8(&buf[..1]).unwrap_or("0");
    }
    let mut x = v;
    let mut i = 8;
    while x > 0 {
        i -= 1;
        let nibble = (x & 0xf) as u8;
        buf[i] = if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + (nibble - 10)
        };
        x >>= 4;
    }
    core::str::from_utf8(&buf[i..]).unwrap_or("0")
}

/// Encode visit_count as decimal ASCII (bounded).
pub fn encode_u32(v: u32, out: &mut heapless::String<16>) {
    out.clear();
    let _ = write!(out, "{}", v);
}

pub fn decode_u32(bytes: &[u8]) -> Option<u32> {
    let s = core::str::from_utf8(bytes).ok()?;
    if s.is_empty() || s.len() > 10 {
        return None;
    }
    s.parse().ok()
}

pub fn encode_u64(v: u64, out: &mut heapless::String<24>) {
    out.clear();
    let _ = write!(out, "{}", v);
}

pub fn decode_u64(bytes: &[u8]) -> Option<u64> {
    let s = core::str::from_utf8(bytes).ok()?;
    if s.is_empty() || s.len() > 20 {
        return None;
    }
    s.parse().ok()
}

pub fn encode_bool(v: bool) -> &'static str {
    if v {
        "1"
    } else {
        "0"
    }
}

pub fn decode_bool(bytes: &[u8]) -> Option<bool> {
    match bytes {
        b"1" | b"true" | b"yes" => Some(true),
        b"0" | b"false" | b"no" => Some(false),
        _ => None,
    }
}

pub fn encode_provider(p: BrainProviderKind) -> u8 {
    p as u8
}

pub fn decode_provider(bytes: &[u8]) -> Option<BrainProviderKind> {
    if bytes.len() == 1 {
        return match bytes[0] {
            1 | b'1' => Some(BrainProviderKind::LocalBounded),
            2 | b'2' => Some(BrainProviderKind::FutureOnline),
            0xFF | b'f' => Some(BrainProviderKind::Fallback),
            _ => None,
        };
    }
    let s = core::str::from_utf8(bytes).ok()?;
    match s {
        "1" | "local" | "local-bounded" => Some(BrainProviderKind::LocalBounded),
        "2" | "online" | "future-online" => Some(BrainProviderKind::FutureOnline),
        "255" | "fallback" => Some(BrainProviderKind::Fallback),
        _ => None,
    }
}

pub fn encode_style(s: GreetingStyle) -> &'static str {
    s.as_str()
}

pub fn decode_style(bytes: &[u8]) -> Option<GreetingStyle> {
    let s = core::str::from_utf8(bytes).ok()?;
    GreetingStyle::from_str(s).or_else(|| {
        if bytes.len() == 1 {
            GreetingStyle::from_u8(bytes[0].wrapping_sub(b'0'))
        } else {
            None
        }
    })
}

/// Format usable memory for user-facing text (deterministic).
/// Prefer decimal GiB with one fraction when ≥ 1024 MiB; never truncate 3714 → 3.
pub fn format_memory_mib<const N: usize>(ram_mib: u32, out: &mut heapless::String<N>) {
    out.clear();
    if ram_mib >= 1024 {
        let whole = ram_mib / 1024;
        let frac = ((ram_mib % 1024) * 10) / 1024; // one decimal
        if frac == 0 {
            let _ = write!(out, "{} GiB", whole);
        } else {
            let _ = write!(out, "{}.{} GiB", whole, frac);
        }
    } else {
        let _ = write!(out, "{} MiB", ram_mib);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_fits_register_limit() {
        let mut k = heapless::String::new();
        assert!(format_user_key(0, KEY_CODE_VISIT, &mut k));
        assert!(k.len() <= 16);
        assert_eq!(k.as_str(), "wb1:0:vc");
        assert!(format_user_key(0xffff_ffff, KEY_CODE_SMS, &mut k));
        assert!(k.len() <= 16);
        assert_eq!(k.as_str(), "wb1:ffffffff:sms");
    }

    #[test]
    fn defaults_match_current_welcome() {
        let p = BrainPreferences::default();
        assert!(p.show_machine_summary);
        assert!(!p.show_index_status);
        assert_eq!(p.greeting_style, GreetingStyle::Concise);
    }

    #[test]
    fn visit_count_saturates() {
        let mut s = WelcomeMemoryState {
            visit_count: u32::MAX,
            ..Default::default()
        };
        s.record_completion(1);
        assert_eq!(s.visit_count, u32::MAX);
    }

    #[test]
    fn returning_and_upgrade() {
        let mut s = WelcomeMemoryState::default();
        assert!(!s.is_returning_visit());
        s.record_completion(10);
        assert!(s.is_returning_visit());
        assert!(!s.is_after_upgrade(10));
        assert!(s.is_after_upgrade(11));
    }

    #[test]
    fn memory_format_not_truncated() {
        let mut o: heapless::String<32> = heapless::String::new();
        format_memory_mib(3714, &mut o);
        assert_eq!(o.as_str(), "3.6 GiB");
        format_memory_mib(512, &mut o);
        assert_eq!(o.as_str(), "512 MiB");
        format_memory_mib(2048, &mut o);
        assert_eq!(o.as_str(), "2 GiB");
    }

    #[test]
    fn style_roundtrip() {
        for s in [
            GreetingStyle::Concise,
            GreetingStyle::Friendly,
            GreetingStyle::Technical,
        ] {
            assert_eq!(GreetingStyle::from_str(s.as_str()), Some(s));
        }
        assert!(GreetingStyle::from_str("chatty").is_none());
    }

    #[test]
    fn decode_malformed_ignored() {
        assert!(decode_u32(b"").is_none());
        assert!(decode_u32(b"nope").is_none());
        assert!(decode_bool(b"maybe").is_none());
        assert!(decode_style(b"loud").is_none());
    }
}
