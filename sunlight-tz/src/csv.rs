//! Timezone database, generated at build time from docs/Timezones.csv
//! (see sunlight-tz/build.rs). Generating a static array at build time
//! avoids parsing a 559-row CSV at runtime in a no_std/no_alloc context,
//! which previously was capped at MAX_ZONES=256 and silently dropped any
//! zone past that point (e.g. Asia/Tehran).

/// One entry from the timezone database
#[derive(Clone, Copy, Debug)]
pub struct TzEntry {
    pub id:                 &'static str,   // "Asia/Tehran"
    pub region:             &'static str,   // "Asia"
    pub city:               &'static str,   // "Tehran"
    pub display_name:       &'static str,   // "Iran Standard Time"
    pub utc_offset_hours:   i8,             // 3  (can be negative)
    pub utc_offset_minutes: u8,             // 30 (always 0 or 30)
    pub dst_offset_minutes: u8,             // 60 (0 = no DST)
    pub dst_start_month:    u8,             // 3  (0 = no DST)
    pub dst_end_month:      u8,             // 9  (0 = no DST)
}

include!(concat!(env!("OUT_DIR"), "/zones_data.rs"));

/// All known timezones.
pub fn all_zones() -> &'static [TzEntry] {
    ZONES
}

/// Lookup by exact IANA id.
pub fn tz_by_id(id: &str) -> Option<&'static TzEntry> {
    all_zones().iter().find(|e| e.id == id)
}

/// Case-insensitive substring match on display_name. Byte-level, no alloc.
pub fn tz_by_display_name(name: &str) -> Option<&'static TzEntry> {
    let needle = name.as_bytes();
    all_zones().iter().find(|e| ci_contains(e.display_name.as_bytes(), needle))
}

/// Total number of loaded zones.
pub fn tz_count() -> usize { all_zones().len() }

/// Byte-wise case-insensitive contains (ascii only). No heap, no_std.
fn ci_contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() { return true; }
    if hay.len() < needle.len() { return false; }
    for i in 0..=hay.len() - needle.len() {
        let mut match_ok = true;
        for j in 0..needle.len() {
            let h = to_ascii_lower(hay[i + j]);
            let n = to_ascii_lower(needle[j]);
            if h != n {
                match_ok = false;
                break;
            }
        }
        if match_ok {
            return true;
        }
    }
    false
}

#[inline]
fn to_ascii_lower(b: u8) -> u8 {
    if b'A' <= b && b <= b'Z' { b + 32 } else { b }
}
