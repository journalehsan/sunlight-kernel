//! compile-time embedded timezone database from docs/Timezones.csv

use core::sync::atomic::{AtomicBool, Ordering};

use crate::config::parse_i32;

/// One parsed entry from the CSV
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

const MAX_ZONES: usize = 256;

struct ZoneTable {
    entries: [TzEntry; MAX_ZONES],
    count:   usize,
}

static mut ZONE_TABLE: ZoneTable = ZoneTable {
    entries: [TzEntry {
        id: "",
        region: "",
        city: "",
        display_name: "",
        utc_offset_hours: 0,
        utc_offset_minutes: 0,
        dst_offset_minutes: 0,
        dst_start_month: 0,
        dst_end_month: 0,
    }; MAX_ZONES],
    count: 0,
};
static ZONES_INIT: AtomicBool = AtomicBool::new(false);

static CSV_RAW: &str = include_str!("../../docs/Timezones.csv");

/// Parse the embedded CSV into the static table.
/// Skips comments, header, and any malformed rows.
fn init_zones() {
    let mut count: usize = 0;
    let mut is_first_data = true;

    for line in CSV_RAW.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if is_first_data {
            // skip header
            is_first_data = false;
            continue;
        }

        // Split on comma. Real CSV has 4 columns.
        let mut fields: [&str; 4] = ["", "", "", ""];
        let mut fcount = 0usize;
        let mut start = 0usize;
        let bytes = trimmed.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            if b == b',' {
                if fcount < 4 {
                    fields[fcount] = trimmed[start..i].trim();
                    fcount += 1;
                }
                start = i + 1;
            }
        }
        if fcount < 4 {
            fields[fcount] = trimmed[start..].trim();
            fcount += 1;
        }
        if fcount != 4 {
            // malformed, skip row
            continue;
        }

        let id = fields[0];
        let offset_str = fields[1];
        let dst_str = fields[2];
        let display_name = fields[3];

        if id.is_empty() || display_name.is_empty() {
            continue;
        }

        // Parse offset "H : M" or "H:M" possibly with spaces
        let (utc_h, utc_m) = match parse_offset_hm(offset_str) {
            Some(v) => v,
            None => continue, // skip bad row
        };

        let dst_mins: u8 = if dst_str.is_empty() {
            0
        } else if let Ok(v) = parse_i32(dst_str) {
            if v < 0 || v > 255 { 0 } else { v as u8 }
        } else {
            0
        };

        // Derive region/city from id (e.g. "Asia/Tehran", "Etc/GMT+12")
        let (region, city) = split_region_city(id); // id is &'static from CSV_RAW line slice

        // DST months: approximate from presence of dst offset (CSV lacks per-zone months)
        let (dst_start, dst_end) = if dst_mins > 0 { (3u8, 9u8) } else { (0u8, 0u8) };

        if count < MAX_ZONES {
            // SAFETY: writing during single-threaded init before any reader sees the table.
            unsafe {
                ZONE_TABLE.entries[count] = TzEntry {
                    id,
                    region,
                    city,
                    display_name,
                    utc_offset_hours: utc_h,
                    utc_offset_minutes: utc_m,
                    dst_offset_minutes: dst_mins,
                    dst_start_month: dst_start,
                    dst_end_month: dst_end,
                };
            }
            count += 1;
        } else {
            break;
        }
    }

    // SAFETY: single init, count written before flag release.
    unsafe { ZONE_TABLE.count = count; }
}

/// Parse " -12 : 00 " style into (hours i8, minutes u8). Returns None on error.
fn parse_offset_hm(s: &str) -> Option<(i8, u8)> {
    let s = s.trim();
    // Normalize: replace " : " with ":", remove spaces
    let mut norm = [0u8; 16];
    let mut n = 0usize;
    for &b in s.as_bytes() {
        if b == b' ' { continue; }
        if n < norm.len() {
            norm[n] = b;
            n += 1;
        }
    }
    let norm_s = core::str::from_utf8(&norm[..n]).ok()?;

    // Split on ':'
    let mut parts = norm_s.split(':');
    let hpart = parts.next()?;
    let mpart = parts.next().unwrap_or("0");

    let h = parse_i32(hpart).ok()? as i8;  // i8 range is sufficient for offsets
    let m = if mpart.is_empty() { 0u8 } else {
        let mv = parse_i32(mpart).ok()?;
        if mv < 0 || mv > 59 { return None; }
        mv as u8
    };
    Some((h, m))
}

/// Split id into (region, city). For "Asia/Tehran" -> ("Asia","Tehran")
/// For "Etc/GMT+12" -> ("Etc","GMT+12")
/// For "UTC" or no-slash -> (id, id)
/// Input must be a subslice of the 'static CSV data (during init).
fn split_region_city(id: &'static str) -> (&'static str, &'static str) {
    if let Some(pos) = id.rfind('/') {
        let region = &id[..pos];
        let city = &id[pos + 1..];
        (region, city)
    } else {
        (id, id)
    }
}

/// Lazy-init accessor. All access goes through here.
pub fn all_zones() -> &'static [TzEntry] {
    if !ZONES_INIT.load(Ordering::Acquire) {
        // SAFETY: single-threaded user-space process; init runs exactly once before flag is set.
        // init_zones performs the (internal) unsafe writes to the static table.
        init_zones();
        ZONES_INIT.store(true, Ordering::Release);
    }
    // SAFETY: zones are fully initialized before this line (Acquire/Release guarantees visibility).
    unsafe { &ZONE_TABLE.entries[..ZONE_TABLE.count] }
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
