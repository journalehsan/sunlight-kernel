//! sunlight-locale — minimal locale configuration foundation for SunlightOS.
//!
//! Provides:
//! - Parsing of /etc/locale.conf (KEY=VALUE with comments/blank lines)
//! - Known LC_* / LANG variable whitelist
//! - Fallback rules (missing LC_* -> LANG -> C.UTF-8 -> C)
//! - Parsing of /etc/locale.gen for available locales
//! - Simple Gregorian date/time formatting helpers for C / en_US.UTF-8
//!
//! This is intentionally small. No full i18n, no message catalogs,
//! no Jalali calendar. LC_TIME is the primary target for Calendar prep.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Locale categories we recognize (order roughly follows traditional locale.conf).
const KNOWN_KEYS: &[&str] = &[
    "LANG",
    "LC_ADDRESS",
    "LC_COLLATE",
    "LC_CTYPE",
    "LC_IDENTIFICATION",
    "LC_MEASUREMENT",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NAME",
    "LC_NUMERIC",
    "LC_PAPER",
    "LC_TELEPHONE",
    "LC_TIME",
    "LC_ALL",
];

/// Parsed locale configuration.
/// Stores only known keys. Unknown keys are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocaleConfig {
    vars: alloc::collections::BTreeMap<String, String>,
}

impl LocaleConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a value for a known key. Unknown keys are ignored.
    pub fn set(&mut self, key: &str, value: &str) {
        if is_known_key(key) {
            self.vars.insert(String::from(key), String::from(value));
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(|s| s.as_str())
    }

    /// Remove a key if present.
    pub fn unset(&mut self, key: &str) -> bool {
        self.vars.remove(key).is_some()
    }

    /// Iterate over (key, value) pairs (sorted by key for determinism).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Effective value for a category with full fallback chain:
    ///   LC_ALL (if set and non-empty) wins for everything,
    ///   then the specific LC_*,
    ///   then LANG,
    ///   then C.UTF-8,
    ///   then C.
    pub fn effective(&self, category: &str) -> &str {
        // LC_ALL overrides all when set and non-empty.
        if let Some(all) = self.get("LC_ALL") {
            if !all.is_empty() {
                return all;
            }
        }
        if let Some(v) = self.get(category) {
            if !v.is_empty() {
                return v;
            }
        }
        if let Some(lang) = self.get("LANG") {
            if !lang.is_empty() {
                return lang;
            }
        }
        // Default chain
        if self.is_available("C.UTF-8") {
            "C.UTF-8"
        } else {
            "C"
        }
    }

    /// Return the effective LC_TIME (primary for Calendar).
    pub fn lc_time(&self) -> &str {
        self.effective("LC_TIME")
    }

    /// Return the effective LANG (with fallbacks applied).
    pub fn lang(&self) -> &str {
        self.effective("LANG")
    }

    /// Whether this config currently has any values.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// Internal helper for tests / future: pretend "C.UTF-8" is always available
    /// in our minimal implementation.
    fn is_available(&self, _name: &str) -> bool {
        // In this foundation we only ship C, C.UTF-8, en_US.UTF-8 in locale.gen.
        // We treat C.UTF-8 and C as always resolvable for formatting purposes.
        true
    }
}

fn is_known_key(k: &str) -> bool {
    KNOWN_KEYS
        .iter()
        .any(|&known| known.eq_ignore_ascii_case(k))
}

/// Parse /etc/locale.conf content.
///
/// Rules:
/// - Ignore blank lines (after trim)
/// - Ignore lines starting with '#' (after ltrim)
/// - Split on first '=' only
/// - Trim whitespace from key and value
/// - Only keep known keys
/// - Later lines override earlier ones for the same key
pub fn parse_locale_conf(data: &[u8]) -> LocaleConfig {
    let mut cfg = LocaleConfig::new();
    let text = core::str::from_utf8(data).unwrap_or("");
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim();
            let val = line[eq + 1..].trim();
            if !key.is_empty() {
                cfg.set(key, val);
            }
        }
    }
    cfg
}

/// Parse /etc/locale.gen into list of locale names.
/// Each non-comment, non-empty line's first token is kept.
/// Supports simple "name" or "name UTF-8" style lines.
pub fn parse_locale_gen(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let text = core::str::from_utf8(data).unwrap_or("");
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Take first whitespace-separated token
        let name = line.split_whitespace().next().unwrap_or("");
        if !name.is_empty() && !out.iter().any(|s| s == name) {
            out.push(String::from(name));
        }
    }
    out
}

/// Return the canonical default content for a fresh /etc/locale.conf.
pub fn default_locale_conf() -> &'static str {
    "LANG=en_US.UTF-8\n\
LC_ADDRESS=en_US.UTF-8\n\
LC_IDENTIFICATION=en_US.UTF-8\n\
LC_MEASUREMENT=en_US.UTF-8\n\
LC_MONETARY=en_US.UTF-8\n\
LC_NAME=en_US.UTF-8\n\
LC_NUMERIC=en_US.UTF-8\n\
LC_PAPER=en_US.UTF-8\n\
LC_TELEPHONE=en_US.UTF-8\n\
LC_TIME=en_US.UTF-8\n"
}

/// Return the canonical small list for /etc/locale.gen.
pub fn default_locale_gen() -> &'static str {
    "C\nC.UTF-8\nen_US.UTF-8\n"
}

/// Validate that `name` appears in the provided available list (exact match).
pub fn is_valid_locale(name: &str, available: &[String]) -> bool {
    available.iter().any(|s| s == name)
}

/// Apply a full set operation: set LANG and all LC_* (except LC_ALL) to `value`.
/// LC_ALL is cleared (so that specific categories can be seen).
pub fn apply_set_all(cfg: &mut LocaleConfig, value: &str) {
    // Clear LC_ALL so the specific values are visible.
    let _ = cfg.unset("LC_ALL");
    for &k in KNOWN_KEYS.iter() {
        if k != "LC_ALL" {
            cfg.set(k, value);
        }
    }
}

/// Apply set-time: only touch LC_TIME.
pub fn apply_set_time(cfg: &mut LocaleConfig, value: &str) {
    cfg.set("LC_TIME", value);
}

// ---------------------------------------------------------------------------
// Date / time formatting helpers (Gregorian only)
// ---------------------------------------------------------------------------

/// Names for C / POSIX style (English abbreviations and full where traditional).
const C_WEEKDAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const C_WEEKDAYS_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const C_MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const C_MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Very small date/time representation used by the helpers.
/// (We do not perform calendar math here; the caller supplies fields from
///  the existing sunlight-tz LocalDateTime or UTC decomposition.)
#[derive(Clone, Copy, Debug, Default)]
pub struct SimpleDateTime {
    pub year: i32,
    pub month: u8, // 1..=12
    pub day: u8,   // 1..=31
    pub hour: u8,  // 0..=23
    pub minute: u8,
    pub second: u8,
    /// ISO weekday (1=Mon ... 7=Sun). If unknown, use 0 and helpers will map conservatively.
    pub weekday_iso: u8,
}

/// Format a short date according to the effective locale for LC_TIME.
/// Current supported:
/// - C / C.UTF-8 / POSIX-ish -> 2026-07-07
/// - en_US.UTF-8 -> 07/07/2026  (US style)
pub fn format_short_date(dt: &SimpleDateTime, locale: &str) -> String {
    let l = normalize_locale(locale);
    match l {
        LocaleTag::EnUs => alloc::format!("{:02}/{:02}/{}", dt.month, dt.day, dt.year),
        _ => alloc::format!("{:04}-{:02}-{:02}", dt.year, dt.month, dt.day),
    }
}

/// Format a long date.
/// C -> Tue 07 Jul 2026
/// en_US -> Tuesday, July 07, 2026
pub fn format_long_date(dt: &SimpleDateTime, locale: &str) -> String {
    let l = normalize_locale(locale);
    let (wd, mon) = weekday_and_month(dt, l);
    match l {
        LocaleTag::EnUs => alloc::format!("{}, {} {:02}, {}", wd, mon, dt.day, dt.year),
        _ => alloc::format!("{} {:02} {} {}", wd, dt.day, mon, dt.year),
    }
}

/// Format short time as HH:MM (24h for all current locales).
pub fn format_short_time(dt: &SimpleDateTime, _locale: &str) -> String {
    alloc::format!("{:02}:{:02}", dt.hour, dt.minute)
}

/// Return month name (short or long) for the locale.
pub fn month_name(month: u8, long: bool, locale: &str) -> &'static str {
    if !(1..=12).contains(&month) {
        return "";
    }
    let idx = (month - 1) as usize;
    let l = normalize_locale(locale);
    match (l, long) {
        (LocaleTag::EnUs, true) => C_MONTHS_LONG[idx],
        (LocaleTag::EnUs, false) => C_MONTHS_SHORT[idx],
        (_, true) => C_MONTHS_LONG[idx],
        (_, false) => C_MONTHS_SHORT[idx],
    }
}

/// Return weekday name (short or long). weekday_iso: 1=Mon..7=Sun.
/// If 0 is passed we treat as Sunday (7) conservatively for display.
pub fn weekday_name(weekday_iso: u8, long: bool, locale: &str) -> &'static str {
    let mut w = weekday_iso;
    if w == 0 {
        w = 7;
    }
    if !(1..=7).contains(&w) {
        return "";
    }
    // Map ISO (Mon=1) to our Sunday=0 array
    let sun0 = (w % 7) as usize; // Mon=1 -> 1, Sun=7 -> 0
    let l = normalize_locale(locale);
    match (l, long) {
        (LocaleTag::EnUs, true) => C_WEEKDAYS_LONG[sun0],
        (LocaleTag::EnUs, false) => C_WEEKDAYS_SHORT[sun0],
        (_, true) => C_WEEKDAYS_LONG[sun0],
        (_, false) => C_WEEKDAYS_SHORT[sun0],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocaleTag {
    C,
    EnUs,
}

fn normalize_locale(s: &str) -> LocaleTag {
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("en_us") || lower.starts_with("en-us") {
        LocaleTag::EnUs
    } else if lower == "c" || lower.starts_with("c.") || lower == "posix" {
        LocaleTag::C
    } else {
        // Default to C behavior for unknown in this foundation
        LocaleTag::C
    }
}

fn weekday_and_month(dt: &SimpleDateTime, l: LocaleTag) -> (&'static str, &'static str) {
    let m_idx = (dt.month.saturating_sub(1) as usize).min(11);
    let mut w = dt.weekday_iso;
    if w == 0 {
        w = 7;
    }
    let w_idx = (w % 7) as usize;
    match l {
        LocaleTag::EnUs => (C_WEEKDAYS_LONG[w_idx], C_MONTHS_LONG[m_idx]),
        LocaleTag::C => (C_WEEKDAYS_SHORT[w_idx], C_MONTHS_SHORT[m_idx]),
    }
}

// ---------------------------------------------------------------------------
// Small helper to produce a LocaleConfig pre-filled with safe C defaults.
// ---------------------------------------------------------------------------

/// Return a config representing the absolute fallback (no file present).
pub fn fallback_config() -> LocaleConfig {
    let mut c = LocaleConfig::new();
    c.set("LANG", "C.UTF-8");
    c.set("LC_TIME", "C.UTF-8");
    c
}

/// Serialize the config back to KEY=VALUE lines suitable for /etc/locale.conf.
/// Only emits known keys that are present. Order is deterministic (by key).
pub fn serialize_locale_conf(cfg: &LocaleConfig) -> String {
    let mut out = String::new();
    // Emit in a conventional order: LANG first, then LC_* in the order of KNOWN_KEYS
    if let Some(v) = cfg.get("LANG") {
        out.push_str("LANG=");
        out.push_str(v);
        out.push('\n');
    }
    for &k in KNOWN_KEYS.iter() {
        if k == "LANG" || k == "LC_ALL" {
            continue;
        }
        if let Some(v) = cfg.get(k) {
            out.push_str(k);
            out.push('=');
            out.push_str(v);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn parse_locale_conf_basic() {
        let data = b"LANG=en_US.UTF-8\nLC_TIME=C.UTF-8\n";
        let cfg = parse_locale_conf(data);
        assert_eq!(cfg.get("LANG"), Some("en_US.UTF-8"));
        assert_eq!(cfg.get("LC_TIME"), Some("C.UTF-8"));
    }

    #[test]
    fn parse_locale_conf_ignores_comments_and_blank() {
        let data = b"\n# comment\nLANG=C.UTF-8\n\nLC_NUMERIC=en_US.UTF-8\n# trailing\n";
        let cfg = parse_locale_conf(data);
        assert_eq!(cfg.get("LANG"), Some("C.UTF-8"));
        assert_eq!(cfg.get("LC_NUMERIC"), Some("en_US.UTF-8"));
        assert!(cfg.get("LC_TIME").is_none());
    }

    #[test]
    fn parse_locale_conf_trims_and_unknown_ignored() {
        let data = b"  LANG  =  en_US.UTF-8  \nFOO=bar\nLC_TIME = C \n";
        let cfg = parse_locale_conf(data);
        assert_eq!(cfg.get("LANG"), Some("en_US.UTF-8"));
        assert_eq!(cfg.get("LC_TIME"), Some("C"));
        assert!(cfg.get("FOO").is_none());
    }

    #[test]
    fn fallback_when_missing_lc_time() {
        let mut cfg = LocaleConfig::new();
        cfg.set("LANG", "en_US.UTF-8");
        // no LC_TIME
        assert_eq!(cfg.lc_time(), "en_US.UTF-8");
    }

    #[test]
    fn fallback_when_missing_lang() {
        let cfg = LocaleConfig::new();
        // nothing set
        assert_eq!(cfg.lang(), "C.UTF-8");
    }

    #[test]
    fn effective_respects_lc_all() {
        let mut cfg = LocaleConfig::new();
        cfg.set("LANG", "en_US.UTF-8");
        cfg.set("LC_TIME", "C");
        cfg.set("LC_ALL", "POSIX");
        assert_eq!(cfg.lc_time(), "POSIX");
        assert_eq!(cfg.effective("LC_NUMERIC"), "POSIX");
    }

    #[test]
    fn locale_gen_parse() {
        let data = b"C\nC.UTF-8\nen_US.UTF-8\n# comment\n\n";
        let list = parse_locale_gen(data);
        assert_eq!(list, vec!["C", "C.UTF-8", "en_US.UTF-8"]);
    }

    #[test]
    fn set_all_and_set_time() {
        let mut cfg = LocaleConfig::new();
        cfg.set("LANG", "C.UTF-8");
        apply_set_all(&mut cfg, "en_US.UTF-8");
        assert_eq!(cfg.get("LANG"), Some("en_US.UTF-8"));
        assert_eq!(cfg.get("LC_TIME"), Some("en_US.UTF-8"));
        assert!(cfg.get("LC_ALL").is_none());

        apply_set_time(&mut cfg, "C.UTF-8");
        assert_eq!(cfg.get("LC_TIME"), Some("C.UTF-8"));
        // LANG should remain from previous set-all
        assert_eq!(cfg.get("LANG"), Some("en_US.UTF-8"));
    }

    #[test]
    fn formatters_c_and_en_us() {
        let dt = SimpleDateTime {
            year: 2026,
            month: 7,
            day: 7,
            hour: 9,
            minute: 5,
            second: 0,
            weekday_iso: 2, // Tuesday (ISO Mon=1)
        };
        assert_eq!(format_short_date(&dt, "C"), "2026-07-07");
        assert_eq!(format_short_date(&dt, "C.UTF-8"), "2026-07-07");
        assert_eq!(format_short_date(&dt, "en_US.UTF-8"), "07/07/2026");

        assert_eq!(format_long_date(&dt, "C"), "Tue 07 Jul 2026");
        assert_eq!(
            format_long_date(&dt, "en_US.UTF-8"),
            "Tuesday, July 07, 2026"
        );

        assert_eq!(format_short_time(&dt, "en_US.UTF-8"), "09:05");

        assert_eq!(month_name(7, false, "C"), "Jul");
        assert_eq!(month_name(7, true, "en_US.UTF-8"), "July");

        assert_eq!(weekday_name(2, false, "C"), "Tue");
        assert_eq!(weekday_name(2, true, "en_US.UTF-8"), "Tuesday");
    }

    #[test]
    fn validation_against_gen() {
        let avail: Vec<String> = vec!["C".into(), "C.UTF-8".into(), "en_US.UTF-8".into()];
        assert!(is_valid_locale("en_US.UTF-8", &avail));
        assert!(!is_valid_locale("fa_IR.UTF-8", &avail));
        assert!(!is_valid_locale("en_GB.UTF-8", &avail));
    }

    #[test]
    fn serialize_roundtrip_minimal() {
        let mut cfg = LocaleConfig::new();
        cfg.set("LANG", "en_US.UTF-8");
        cfg.set("LC_TIME", "C.UTF-8");
        let s = serialize_locale_conf(&cfg);
        // LANG first, then others
        assert!(s.starts_with("LANG=en_US.UTF-8\n"));
        assert!(s.contains("LC_TIME=C.UTF-8\n"));
        // Re-parse should give same effective values
        let cfg2 = parse_locale_conf(s.as_bytes());
        assert_eq!(cfg2.get("LANG"), Some("en_US.UTF-8"));
        assert_eq!(cfg2.lc_time(), "C.UTF-8");
    }
}
