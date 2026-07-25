//! Extensible timezone city location catalog.
//!
//! Owned by the timezone library shared by `tzctl`, `tzutils`, the timezone
//! service, and Control Panel. Coordinates are fixed-point millidegrees so the
//! catalog stays `no_std`-friendly and free of floating point in ranking code.
//!
//! This is an initial seed, not worldwide coverage. New cities are added by
//! extending [`LOCATIONS`]; GUI code does not need to change.

use crate::csv::{tz_by_id, TzEntry};
use crate::ntp_region::{ntp_region_from_zone_id, NtpRegion};

/// Latitude / longitude in millidegrees (`degrees × 1000`).
/// Range: lat ∈ [-90_000, 90_000], lon ∈ [-180_000, 180_000].
pub type MilliDeg = i32;

/// Maximum results returned by [`search_locations`].
pub const MAX_SEARCH_RESULTS: usize = 12;

/// Maximum candidates returned by [`nearest_locations`].
pub const MAX_NEAREST_RESULTS: usize = 3;

/// Default maximum great-circle distance for map hit selection (millidegrees of
/// arc on a unit sphere approximation). 12_000 md ≈ 1 333 km.
pub const DEFAULT_MAX_DISTANCE_MD: MilliDeg = 12_000;

/// One catalog location.
#[derive(Clone, Copy, Debug)]
pub struct TzLocation {
    /// Canonical IANA timezone id (`Asia/Tehran`).
    pub zone_id: &'static str,
    /// Display city name.
    pub city: &'static str,
    /// ISO 3166-1 alpha-2 country code.
    pub country_code: &'static str,
    /// Human country name.
    pub country: &'static str,
    /// Continent / NTP region head (matches IANA or catalog convention).
    pub continent: &'static str,
    /// Latitude millidegrees.
    pub lat_md: MilliDeg,
    /// Longitude millidegrees.
    pub lon_md: MilliDeg,
    /// Extra search aliases (ASCII).
    pub aliases: &'static [&'static str],
    /// Lower = higher display priority in ties.
    pub display_priority: u16,
}

/// Rank tier used by search (lower is better).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum SearchRank {
    ExactCityOrAlias = 0,
    ExactZoneId = 1,
    Prefix = 2,
    Substring = 3,
}

/// One ranked search hit.
#[derive(Clone, Copy, Debug)]
pub struct SearchHit {
    pub location: &'static TzLocation,
    pub rank: SearchRank,
}

/// Bounded search result list (no heap).
#[derive(Clone, Copy, Debug)]
pub struct SearchResults {
    hits: [Option<SearchHit>; MAX_SEARCH_RESULTS],
    len: usize,
}

impl SearchResults {
    pub const fn empty() -> Self {
        Self {
            hits: [None; MAX_SEARCH_RESULTS],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, i: usize) -> Option<SearchHit> {
        self.hits.get(i).and_then(|h| *h)
    }

    pub fn iter(&self) -> impl Iterator<Item = SearchHit> + '_ {
        self.hits[..self.len].iter().filter_map(|h| *h)
    }

    fn push_ranked(&mut self, hit: SearchHit, cap: usize) {
        let cap = cap.min(MAX_SEARCH_RESULTS).max(1);
        let mut pos = self.len;
        for i in 0..self.len {
            if let Some(existing) = self.hits[i] {
                if cmp_hits(&hit, &existing) == core::cmp::Ordering::Less {
                    pos = i;
                    break;
                }
            }
        }
        if pos >= cap {
            return;
        }
        if self.len < cap {
            // Shift right to make room at `pos`.
            let mut i = self.len;
            while i > pos {
                self.hits[i] = self.hits[i - 1];
                i -= 1;
            }
            self.hits[pos] = Some(hit);
            self.len += 1;
        } else {
            let mut i = cap - 1;
            while i > pos {
                self.hits[i] = self.hits[i - 1];
                i -= 1;
            }
            self.hits[pos] = Some(hit);
            self.len = cap;
        }
    }
}

/// One nearest-location candidate.
#[derive(Clone, Copy, Debug)]
pub struct NearestHit {
    pub location: &'static TzLocation,
    /// Squared equirectangular distance in millidegree² (relative ranking only).
    pub dist_sq: u64,
}

/// Bounded nearest result list.
#[derive(Clone, Copy, Debug)]
pub struct NearestResults {
    hits: [Option<NearestHit>; MAX_NEAREST_RESULTS],
    len: usize,
}

impl NearestResults {
    pub const fn empty() -> Self {
        Self {
            hits: [None; MAX_NEAREST_RESULTS],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, i: usize) -> Option<NearestHit> {
        self.hits.get(i).and_then(|h| *h)
    }

    pub fn first(&self) -> Option<NearestHit> {
        self.get(0)
    }

    pub fn iter(&self) -> impl Iterator<Item = NearestHit> + '_ {
        self.hits[..self.len].iter().filter_map(|h| *h)
    }

    fn push_sorted(&mut self, hit: NearestHit, cap: usize) {
        let cap = cap.min(MAX_NEAREST_RESULTS).max(1);
        let mut pos = self.len;
        for i in 0..self.len {
            if let Some(existing) = self.hits[i] {
                let ord = hit
                    .dist_sq
                    .cmp(&existing.dist_sq)
                    .then_with(|| {
                        hit.location
                            .display_priority
                            .cmp(&existing.location.display_priority)
                    })
                    .then_with(|| hit.location.zone_id.cmp(existing.location.zone_id));
                if ord == core::cmp::Ordering::Less {
                    pos = i;
                    break;
                }
            }
        }
        if pos >= cap {
            return;
        }
        if self.len < cap {
            let mut i = self.len;
            while i > pos {
                self.hits[i] = self.hits[i - 1];
                i -= 1;
            }
            self.hits[pos] = Some(hit);
            self.len += 1;
        } else {
            let mut i = cap - 1;
            while i > pos {
                self.hits[i] = self.hits[i - 1];
                i -= 1;
            }
            self.hits[pos] = Some(hit);
            self.len = cap;
        }
    }
}

/// Initial seed catalog. Validated against the bundled zone database in tests.
pub static LOCATIONS: &[TzLocation] = &[
    TzLocation {
        zone_id: "Asia/Tehran",
        city: "Tehran",
        country_code: "IR",
        country: "Iran",
        continent: "Asia",
        lat_md: 35_689,
        lon_md: 51_389,
        aliases: &["teheran"],
        display_priority: 10,
    },
    TzLocation {
        zone_id: "Asia/Baku",
        city: "Baku",
        country_code: "AZ",
        country: "Azerbaijan",
        continent: "Asia",
        lat_md: 40_410,
        lon_md: 49_867,
        aliases: &[],
        display_priority: 20,
    },
    TzLocation {
        zone_id: "Europe/Berlin",
        city: "Berlin",
        country_code: "DE",
        country: "Germany",
        continent: "Europe",
        lat_md: 52_520,
        lon_md: 13_405,
        aliases: &[],
        display_priority: 15,
    },
    TzLocation {
        zone_id: "Europe/London",
        city: "London",
        country_code: "GB",
        country: "United Kingdom",
        continent: "Europe",
        lat_md: 51_507,
        lon_md: -128,
        aliases: &["uk", "britain"],
        display_priority: 12,
    },
    TzLocation {
        zone_id: "America/New_York",
        city: "New York",
        country_code: "US",
        country: "United States",
        continent: "America",
        lat_md: 40_713,
        lon_md: -74_006,
        aliases: &["nyc", "newyork"],
        display_priority: 11,
    },
    TzLocation {
        zone_id: "America/Los_Angeles",
        city: "Los Angeles",
        country_code: "US",
        country: "United States",
        continent: "America",
        lat_md: 34_052,
        lon_md: -118_244,
        aliases: &["la", "losangeles"],
        display_priority: 14,
    },
    TzLocation {
        zone_id: "America/Sao_Paulo",
        city: "Sao Paulo",
        country_code: "BR",
        country: "Brazil",
        continent: "America",
        lat_md: -23_551,
        lon_md: -46_633,
        aliases: &["sao paulo", "saopaulo"],
        display_priority: 18,
    },
    TzLocation {
        zone_id: "Africa/Cairo",
        city: "Cairo",
        country_code: "EG",
        country: "Egypt",
        continent: "Africa",
        lat_md: 30_044,
        lon_md: 31_236,
        aliases: &[],
        display_priority: 22,
    },
    TzLocation {
        zone_id: "Africa/Johannesburg",
        city: "Johannesburg",
        country_code: "ZA",
        country: "South Africa",
        continent: "Africa",
        lat_md: -26_204,
        lon_md: 28_047,
        aliases: &["joburg", "jozi"],
        display_priority: 25,
    },
    TzLocation {
        zone_id: "Europe/Moscow",
        city: "Moscow",
        country_code: "RU",
        country: "Russia",
        continent: "Europe",
        lat_md: 55_756,
        lon_md: 37_617,
        aliases: &["moskva"],
        display_priority: 16,
    },
    // Bundled DB uses historical Asia/Calcutta id (not Asia/Kolkata).
    TzLocation {
        zone_id: "Asia/Calcutta",
        city: "Delhi",
        country_code: "IN",
        country: "India",
        continent: "Asia",
        lat_md: 28_614,
        lon_md: 77_209,
        aliases: &["new delhi", "kolkata", "calcutta", "india"],
        display_priority: 13,
    },
    // China Standard Time is Asia/Shanghai in the bundled DB.
    TzLocation {
        zone_id: "Asia/Shanghai",
        city: "Beijing",
        country_code: "CN",
        country: "China",
        continent: "Asia",
        lat_md: 39_904,
        lon_md: 116_407,
        aliases: &["peking", "china"],
        display_priority: 17,
    },
    TzLocation {
        zone_id: "Asia/Tokyo",
        city: "Tokyo",
        country_code: "JP",
        country: "Japan",
        continent: "Asia",
        lat_md: 35_676,
        lon_md: 139_650,
        aliases: &[],
        display_priority: 9,
    },
    TzLocation {
        zone_id: "Asia/Singapore",
        city: "Singapore",
        country_code: "SG",
        country: "Singapore",
        continent: "Asia",
        lat_md: 1_352,
        lon_md: 103_820,
        aliases: &[],
        display_priority: 19,
    },
    TzLocation {
        zone_id: "Australia/Sydney",
        city: "Sydney",
        country_code: "AU",
        country: "Australia",
        continent: "Australia",
        lat_md: -33_869,
        lon_md: 151_209,
        aliases: &[],
        display_priority: 21,
    },
    TzLocation {
        zone_id: "Pacific/Auckland",
        city: "Auckland",
        country_code: "NZ",
        country: "New Zealand",
        continent: "Pacific",
        lat_md: -36_849,
        lon_md: 174_763,
        aliases: &[],
        display_priority: 24,
    },
];

/// Full catalog slice.
pub fn all_locations() -> &'static [TzLocation] {
    LOCATIONS
}

/// Look up a catalog entry by exact canonical zone id.
pub fn location_by_zone_id(zone_id: &str) -> Option<&'static TzLocation> {
    LOCATIONS.iter().find(|l| eq_ascii_ci(l.zone_id, zone_id))
}

/// Look up by exact display city (case-insensitive).
pub fn location_by_city(city: &str) -> Option<&'static TzLocation> {
    LOCATIONS.iter().find(|l| eq_ascii_ci(l.city, city))
}

/// Validate that every catalog zone id exists in the bundled timezone database.
/// Returns the first missing id, or `None` if all are valid.
pub fn validate_catalog_zones() -> Option<&'static str> {
    for loc in LOCATIONS {
        if tz_by_id(loc.zone_id).is_none() {
            return Some(loc.zone_id);
        }
    }
    None
}

/// Detect duplicate zone ids in the catalog. Returns the first duplicate id.
pub fn find_duplicate_zone_ids() -> Option<&'static str> {
    for (i, a) in LOCATIONS.iter().enumerate() {
        for b in LOCATIONS.iter().skip(i + 1) {
            if eq_ascii_ci(a.zone_id, b.zone_id) {
                return Some(a.zone_id);
            }
        }
    }
    None
}

/// Detect duplicate city/alias keys (case-insensitive). Returns first collision.
pub fn find_duplicate_aliases() -> Option<&'static str> {
    for (i, a) in LOCATIONS.iter().enumerate() {
        if let Some(key) = colliding_key_after(a, i + 1) {
            return Some(key);
        }
    }
    None
}

fn colliding_key_after(a: &TzLocation, start: usize) -> Option<&'static str> {
    for b in LOCATIONS.iter().skip(start) {
        if eq_ascii_ci(a.city, b.city) {
            return Some(a.city);
        }
        for alias in a.aliases {
            if eq_ascii_ci(alias, b.city) {
                return Some(alias);
            }
            for balias in b.aliases {
                if eq_ascii_ci(alias, balias) {
                    return Some(alias);
                }
            }
        }
        for balias in b.aliases {
            if eq_ascii_ci(a.city, balias) {
                return Some(balias);
            }
        }
    }
    None
}

/// Whether `proposed` differs from the applied zone (pending Apply).
pub fn selection_is_pending(applied_zone_id: &str, proposed_zone_id: &str) -> bool {
    !proposed_zone_id.is_empty() && !eq_ascii_ci(applied_zone_id, proposed_zone_id)
}

/// NTP region for a catalog location (derived from zone id, not offset).
pub fn location_ntp_region(loc: &TzLocation) -> NtpRegion {
    ntp_region_from_zone_id(loc.zone_id)
}

/// Zone database entry for a catalog location, if present.
pub fn location_tz_entry(loc: &TzLocation) -> Option<&'static TzEntry> {
    tz_by_id(loc.zone_id)
}

/// Search the catalog with bounded, deterministic ranking.
///
/// Ranking (best first):
/// 1. exact city or alias match
/// 2. exact canonical zone id
/// 3. prefix match on city / alias / zone id / country
/// 4. substring match
///
/// Ties break by `display_priority` (lower first), then zone id.
/// Empty query returns the highest-priority locations (bounded).
/// Results are capped at `max_results.min(MAX_SEARCH_RESULTS)`.
pub fn search_locations(query: &str, max_results: usize) -> SearchResults {
    let cap = max_results.min(MAX_SEARCH_RESULTS).max(1);
    let mut out = SearchResults::empty();
    let q = trim_ascii(query);
    if q.is_empty() {
        // Collect indices and sort by priority.
        let mut idxs = [0usize; 32];
        let n = LOCATIONS.len().min(32);
        for i in 0..n {
            idxs[i] = i;
        }
        for i in 1..n {
            let mut j = i;
            while j > 0 {
                let a = &LOCATIONS[idxs[j - 1]];
                let b = &LOCATIONS[idxs[j]];
                if (a.display_priority, a.zone_id) <= (b.display_priority, b.zone_id) {
                    break;
                }
                idxs.swap(j - 1, j);
                j -= 1;
            }
        }
        for i in 0..cap.min(n) {
            out.hits[i] = Some(SearchHit {
                location: &LOCATIONS[idxs[i]],
                rank: SearchRank::Substring,
            });
            out.len = i + 1;
        }
        return out;
    }

    for loc in LOCATIONS {
        if let Some(rank) = rank_location(loc, q) {
            out.push_ranked(SearchHit { location: loc, rank }, cap);
        }
    }
    out
}

fn rank_location(loc: &TzLocation, q: &str) -> Option<SearchRank> {
    if eq_ascii_ci(loc.city, q) {
        return Some(SearchRank::ExactCityOrAlias);
    }
    for alias in loc.aliases {
        if eq_ascii_ci(alias, q) {
            return Some(SearchRank::ExactCityOrAlias);
        }
    }
    if eq_ascii_ci(loc.zone_id, q) {
        return Some(SearchRank::ExactZoneId);
    }
    if let Some(tail) = loc.zone_id.rsplit('/').next() {
        if eq_ascii_ci(tail, q) {
            return Some(SearchRank::ExactZoneId);
        }
    }

    if starts_with_ci(loc.city, q)
        || loc.aliases.iter().any(|a| starts_with_ci(a, q))
        || starts_with_ci(loc.zone_id, q)
        || starts_with_ci(loc.country, q)
        || starts_with_ci(loc.country_code, q)
        || loc
            .zone_id
            .rsplit('/')
            .next()
            .map(|t| starts_with_ci(t, q))
            .unwrap_or(false)
    {
        return Some(SearchRank::Prefix);
    }

    if contains_ci(loc.city, q)
        || loc.aliases.iter().any(|a| contains_ci(a, q))
        || contains_ci(loc.zone_id, q)
        || contains_ci(loc.country, q)
        || contains_ci(loc.continent, q)
    {
        return Some(SearchRank::Substring);
    }
    None
}

fn cmp_hits(a: &SearchHit, b: &SearchHit) -> core::cmp::Ordering {
    a.rank
        .cmp(&b.rank)
        .then_with(|| {
            a.location
                .display_priority
                .cmp(&b.location.display_priority)
        })
        .then_with(|| a.location.zone_id.cmp(b.location.zone_id))
}

/// Normalize longitude millidegrees into [-180_000, 180_000].
pub fn wrap_lon_md(lon: MilliDeg) -> MilliDeg {
    let mut l = lon;
    while l > 180_000 {
        l -= 360_000;
    }
    while l < -180_000 {
        l += 360_000;
    }
    l
}

/// Clamp latitude millidegrees into [-90_000, 90_000].
pub fn clamp_lat_md(lat: MilliDeg) -> MilliDeg {
    lat.clamp(-90_000, 90_000)
}

/// Convert degrees (f32, as used by map widgets) to millidegrees.
pub fn deg_to_md(deg: f32) -> MilliDeg {
    (deg * 1000.0) as MilliDeg
}

/// Convert millidegrees to degrees (f32) for map widgets.
pub fn md_to_deg(md: MilliDeg) -> f32 {
    md as f32 / 1000.0
}

/// Approximate squared equirectangular distance in millidegree².
pub fn distance_sq_md(lat1: MilliDeg, lon1: MilliDeg, lat2: MilliDeg, lon2: MilliDeg) -> u64 {
    let lat1 = clamp_lat_md(lat1);
    let lat2 = clamp_lat_md(lat2);
    let lon1 = wrap_lon_md(lon1);
    let lon2 = wrap_lon_md(lon2);
    let dlat = (lat1 as i64) - (lat2 as i64);
    let mut dlon = (lon1 as i64) - (lon2 as i64);
    if dlon > 180_000 {
        dlon -= 360_000;
    } else if dlon < -180_000 {
        dlon += 360_000;
    }
    let mean_lat = ((lat1 as i64) + (lat2 as i64)) / 2;
    let cos_q8 = cos_lat_q8(mean_lat as i32);
    let dlon_scaled = (dlon * cos_q8 as i64) / 256;
    let dlat2 = dlat.saturating_mul(dlat) as u64;
    let dlon2 = dlon_scaled.saturating_mul(dlon_scaled) as u64;
    dlat2.saturating_add(dlon2)
}

/// Cheap |cos(lat)| in Q8 fixed point (0..256) for lat in millidegrees.
fn cos_lat_q8(lat_md: MilliDeg) -> u32 {
    let a = lat_md.unsigned_abs().min(90_000);
    if a <= 30_000 {
        256 - (a * 34) / 30_000
    } else if a <= 45_000 {
        222 - ((a - 30_000) * 41) / 15_000
    } else if a <= 60_000 {
        181 - ((a - 45_000) * 53) / 15_000
    } else if a <= 75_000 {
        128 - ((a - 60_000) * 62) / 15_000
    } else {
        66 - ((a - 75_000) * 66) / 15_000
    }
}

/// Find nearest catalog locations within `max_distance_md`.
/// Empty when nothing is close enough (ocean / far from seed catalog).
pub fn nearest_locations(
    lat_md: MilliDeg,
    lon_md: MilliDeg,
    max_results: usize,
    max_distance_md: MilliDeg,
) -> NearestResults {
    let cap = max_results.min(MAX_NEAREST_RESULTS).max(1);
    let lat = clamp_lat_md(lat_md);
    let lon = wrap_lon_md(lon_md);
    let max_sq = (max_distance_md as u64).saturating_mul(max_distance_md as u64);
    let mut best = NearestResults::empty();

    for loc in LOCATIONS {
        let dist_sq = distance_sq_md(lat, lon, loc.lat_md, loc.lon_md);
        if dist_sq > max_sq {
            continue;
        }
        best.push_sorted(
            NearestHit {
                location: loc,
                dist_sq,
            },
            cap,
        );
    }
    best
}

// ---------------------------------------------------------------------------
// ASCII helpers (no_std, no alloc)
// ---------------------------------------------------------------------------

fn trim_ascii(s: &str) -> &str {
    s.trim_matches(|c: char| c == ' ' || c == '\t' || c == '\n' || c == '\r')
}

fn to_lower(b: u8) -> u8 {
    if b'A' <= b && b <= b'Z' {
        b + 32
    } else {
        b
    }
}

fn eq_ascii_ci(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    ab.iter()
        .zip(bb.iter())
        .all(|(x, y)| to_lower(*x) == to_lower(*y))
}

fn starts_with_ci(hay: &str, needle: &str) -> bool {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.len() > h.len() {
        return false;
    }
    h[..n.len()]
        .iter()
        .zip(n.iter())
        .all(|(x, y)| to_lower(*x) == to_lower(*y))
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() {
        return true;
    }
    if n.len() > h.len() {
        return false;
    }
    for i in 0..=(h.len() - n.len()) {
        if h[i..i + n.len()]
            .iter()
            .zip(n.iter())
            .all(|(x, y)| to_lower(*x) == to_lower(*y))
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_zones_exist_in_database() {
        assert_eq!(validate_catalog_zones(), None);
    }

    #[test]
    fn no_duplicate_zone_ids() {
        assert_eq!(find_duplicate_zone_ids(), None);
    }

    #[test]
    fn no_duplicate_aliases() {
        assert_eq!(find_duplicate_aliases(), None);
    }

    #[test]
    fn search_exact_tehran() {
        let hits = search_locations("tehran", 8);
        assert!(!hits.is_empty());
        assert_eq!(hits.get(0).unwrap().location.zone_id, "Asia/Tehran");
        assert_eq!(hits.get(0).unwrap().rank, SearchRank::ExactCityOrAlias);
    }

    #[test]
    fn search_exact_zone_id() {
        let hits = search_locations("Asia/Baku", 8);
        assert!(!hits.is_empty());
        assert_eq!(hits.get(0).unwrap().location.zone_id, "Asia/Baku");
        assert_eq!(hits.get(0).unwrap().rank, SearchRank::ExactZoneId);
    }

    #[test]
    fn search_prefix_before_substring() {
        let hits = search_locations("lon", 8);
        assert!(!hits.is_empty());
        assert_eq!(hits.get(0).unwrap().location.zone_id, "Europe/London");
        assert_eq!(hits.get(0).unwrap().rank, SearchRank::Prefix);
    }

    #[test]
    fn search_substring_delhi_alias() {
        let hits = search_locations("kolkata", 8);
        assert!(!hits.is_empty());
        assert_eq!(hits.get(0).unwrap().location.zone_id, "Asia/Calcutta");
    }

    #[test]
    fn search_result_limit() {
        let hits = search_locations("a", 3);
        assert!(hits.len() <= 3);
        assert!(hits.len() <= MAX_SEARCH_RESULTS);
    }

    #[test]
    fn search_empty_is_bounded() {
        let hits = search_locations("", 5);
        assert_eq!(hits.len(), 5);
    }

    #[test]
    fn nearest_tehran_coords() {
        let hits = nearest_locations(35_700, 51_400, 3, DEFAULT_MAX_DISTANCE_MD);
        assert!(!hits.is_empty());
        assert_eq!(hits.first().unwrap().location.zone_id, "Asia/Tehran");
    }

    #[test]
    fn nearest_ocean_rejected() {
        // Mid-Atlantic roughly 0°N, 30°W — far from seed cities.
        let hits = nearest_locations(0, -30_000, 3, DEFAULT_MAX_DISTANCE_MD);
        assert!(hits.is_empty());
    }

    #[test]
    fn nearest_max_distance_rejection() {
        let hits = nearest_locations(35_676, 139_650, 3, 500);
        assert_eq!(hits.first().unwrap().location.zone_id, "Asia/Tokyo");
        let far = nearest_locations(40_000, 145_000, 3, 500);
        assert!(far.is_empty());
    }

    #[test]
    fn date_line_wrap() {
        let d_near = distance_sq_md(0, 179_000, 0, -179_000);
        let d_far = distance_sq_md(0, 179_000, 0, 0);
        assert!(d_near < d_far);
    }

    #[test]
    fn pole_coordinates_clamp() {
        assert_eq!(clamp_lat_md(100_000), 90_000);
        assert_eq!(clamp_lat_md(-100_000), -90_000);
        let d = distance_sq_md(90_000, 0, 90_000, 180_000);
        assert!(d < 1_000_000);
    }

    #[test]
    fn wrap_lon_normalizes() {
        assert_eq!(wrap_lon_md(190_000), -170_000);
        assert_eq!(wrap_lon_md(-190_000), 170_000);
        assert_eq!(wrap_lon_md(0), 0);
    }

    #[test]
    fn proposed_vs_applied() {
        assert!(!selection_is_pending("Asia/Tehran", "Asia/Tehran"));
        assert!(selection_is_pending("Asia/Tehran", "Asia/Baku"));
        assert!(!selection_is_pending("Asia/Tehran", ""));
        assert!(selection_is_pending("", "UTC"));
    }

    #[test]
    fn md_deg_roundtrip_approx() {
        let md = deg_to_md(35.689);
        assert!((md - 35_689).abs() <= 1);
        let deg = md_to_deg(35_689);
        assert!((deg - 35.689).abs() < 0.002);
    }

    #[test]
    fn seeded_cities_present() {
        let expected = [
            "Asia/Tehran",
            "Asia/Baku",
            "Europe/Berlin",
            "Europe/London",
            "America/New_York",
            "America/Los_Angeles",
            "America/Sao_Paulo",
            "Africa/Cairo",
            "Africa/Johannesburg",
            "Europe/Moscow",
            "Asia/Calcutta",
            "Asia/Shanghai",
            "Asia/Tokyo",
            "Asia/Singapore",
            "Australia/Sydney",
            "Pacific/Auckland",
        ];
        for id in expected {
            assert!(
                location_by_zone_id(id).is_some(),
                "missing seed location {id}"
            );
        }
    }

    #[test]
    fn ntp_region_matches_continent() {
        let tehran = location_by_zone_id("Asia/Tehran").unwrap();
        assert_eq!(location_ntp_region(tehran), NtpRegion::Asia);
        let nyc = location_by_zone_id("America/New_York").unwrap();
        assert_eq!(location_ntp_region(nyc), NtpRegion::NorthAmerica);
        let sydney = location_by_zone_id("Australia/Sydney").unwrap();
        assert_eq!(location_ntp_region(sydney), NtpRegion::Oceania);
    }
}
