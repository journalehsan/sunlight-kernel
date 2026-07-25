#![cfg_attr(not(test), no_std)]

//! sunlight-tz — timezone library for SunlightOS
//!
//! Provides CSV-embedded zone data, integer offset/DST math,
//! LocalDateTime formatting, and /etc/localtime VFS-backed config.

pub mod catalog; // city location catalog: search + nearest (shared GUI/CLI)
pub mod cli_flags; // tzutils --sync/-sf parser (host-testable)
pub mod client; // TimeClient / TzClient IPC (shared GUI/CLI; no text scraping)
pub mod config; // moved from timed — parse_offset_string, validate_offset
pub mod csv; // TzEntry, all_zones, tz_by_id, tz_by_display_name
pub mod localtime;
pub mod ntp_region; // continent → NTP pool region mapping
pub mod offset; // local_offset_secs, local_now, LocalDateTime, is_dst_active // LocalTimeCfg, read_localtime, write_localtime, TzError

// Re-export the most commonly used items at crate root
pub use catalog::{
    all_locations, location_by_city, location_by_zone_id, nearest_locations, search_locations,
    selection_is_pending, validate_catalog_zones, MilliDeg, NearestHit, NearestResults, SearchHit,
    SearchRank, SearchResults, TzLocation, DEFAULT_MAX_DISTANCE_MD, MAX_NEAREST_RESULTS,
    MAX_SEARCH_RESULTS,
};
pub use client::{
    LocalTimeSnapshot, SyncStatusSnapshot, TimeClient, TimeClientError, TzClient, TzClientError,
    ZoneSnapshot,
};
pub use csv::{all_zones, tz_by_display_name, tz_by_id, tz_count, TzEntry};
pub use localtime::{read_localtime, write_localtime, LocalTimeCfg, TzError};
pub use ntp_region::{
    format_pool_hostname_into, ntp_region_from_parts, ntp_region_from_zone_id, pool_hostnames,
    NtpRegion, NTP_HOSTNAME_MAX, NTP_POOL_SERVER_COUNT,
};
pub use offset::{local_now, local_offset_secs, weekday_iso, weekday_sun0, LocalDateTime};

/// Convenience: look up zone by id (or None), then compute LocalDateTime.
/// If lookup fails, returns None.
pub fn tz_lookup_by_id_and_now(utc_secs: u64, id: &str) -> Option<LocalDateTime> {
    tz_by_id(id).map(|e| local_now(utc_secs, e))
}

/// Return a best-effort LocalDateTime using "tz" service if available via VFS /etc/localtime,
/// falling back to UTC representation when the service or file is unavailable.
/// (Callers that want pure UTC should query "timed" directly.)
pub fn local_now_best_effort(utc_secs: u64) -> LocalDateTime {
    let cfg = read_localtime();
    // Reconstruct a minimal TzEntry-like for offset math when we don't have full CSV entry.
    // For full fidelity use tz_by_id on cfg.id_str() first.
    if let Some(entry) = tz_by_id(cfg.id_str()) {
        return local_now(utc_secs, entry);
    }
    // Fallback: synthesize a TzEntry from the cfg fields (no region/city/display needed for math)
    let fake = TzEntry {
        id: "UTC",
        region: "UTC",
        city: "UTC",
        display_name: "Coordinated Universal Time",
        utc_offset_hours: cfg.utc_offset_hours,
        utc_offset_minutes: cfg.utc_offset_minutes,
        dst_offset_minutes: cfg.dst_offset_minutes,
        dst_start_month: cfg.dst_start_month,
        dst_end_month: cfg.dst_end_month,
    };
    local_now(utc_secs, &fake)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utc_decompose() {
        // 2026-06-14 10:45:30 UTC
        let secs = 1_781_433_930u64;
        let dt = local_now(
            secs,
            &TzEntry {
                id: "UTC",
                region: "UTC",
                city: "UTC",
                display_name: "UTC",
                utc_offset_hours: 0,
                utc_offset_minutes: 0,
                dst_offset_minutes: 0,
                dst_start_month: 0,
                dst_end_month: 0,
            },
        );
        assert_eq!(dt.year, 2026);
        assert_eq!(dt.month, 6);
        assert_eq!(dt.day, 14);
        assert_eq!(dt.hour, 10);
        assert_eq!(dt.minute, 45);
        assert_eq!(dt.second, 30);
    }

    #[test]
    fn test_tehran_offset_no_dst() {
        // November (month 11) — outside DST window (Mar-Sep)
        // Tehran UTC+3:30, no DST in November
        let entry = TzEntry {
            id: "Asia/Tehran",
            region: "Asia",
            city: "Tehran",
            display_name: "Iran Standard Time",
            utc_offset_hours: 3,
            utc_offset_minutes: 30,
            dst_offset_minutes: 60,
            dst_start_month: 3,
            dst_end_month: 9,
        };
        // Some UTC secs that land in November
        let utc = 1_793_476_800u64; // 2026-11-01
        let offset = local_offset_secs(&entry, utc);
        assert_eq!(offset, 3 * 3600 + 30 * 60); // 12600 secs
    }

    #[test]
    fn test_tehran_offset_with_dst() {
        // June (month 6) — inside DST window (Mar-Sep)
        // Tehran in DST: UTC+3:30 + 1h = UTC+4:30
        let entry = TzEntry {
            id: "Asia/Tehran",
            region: "Asia",
            city: "Tehran",
            display_name: "Iran Standard Time",
            utc_offset_hours: 3,
            utc_offset_minutes: 30,
            dst_offset_minutes: 60,
            dst_start_month: 3,
            dst_end_month: 9,
        };
        let utc = 1_781_428_800u64; // 2026-06-14
        let offset = local_offset_secs(&entry, utc);
        assert_eq!(offset, 3 * 3600 + 30 * 60 + 3600); // 16200 secs = UTC+4:30
    }

    #[test]
    fn test_iso8601_format() {
        // verify fmt_iso8601 output shape
        let dt = LocalDateTime {
            year: 2026,
            month: 6,
            day: 14,
            hour: 14,
            minute: 15,
            second: 30,
            utc_offset_secs: 16200, // +4:30
            is_dst: true,
            abbr: *b"IRDT\0\0\0\0",
        };
        let mut buf = [0u8; 32];
        dt.fmt_iso8601(&mut buf);
        let s = core::str::from_utf8(&buf).unwrap().trim_end_matches('\0');
        assert_eq!(s, "2026-06-14T14:15:30+04:30");
    }
}
