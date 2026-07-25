//! NTP regional pool selection derived from timezone identity.
//!
//! Region is derived from the IANA zone id / continent metadata, never from
//! the current UTC offset (offsets cannot reliably identify continents).

/// NTP pool region used to construct `N.region.pool.ntp.org` hostnames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NtpRegion {
    Africa = 1,
    Asia = 2,
    Europe = 3,
    NorthAmerica = 4,
    SouthAmerica = 5,
    Oceania = 6,
    /// Global fallback (`*.pool.ntp.org`).
    Global = 0,
}

impl NtpRegion {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Africa,
            2 => Self::Asia,
            3 => Self::Europe,
            4 => Self::NorthAmerica,
            5 => Self::SouthAmerica,
            6 => Self::Oceania,
            _ => Self::Global,
        }
    }

    /// Subdomain used by the public NTP pool project, without trailing dots.
    /// Empty string means the global pool (`N.pool.ntp.org`).
    pub const fn pool_subdomain(self) -> &'static str {
        match self {
            Self::Africa => "africa",
            Self::Asia => "asia",
            Self::Europe => "europe",
            Self::NorthAmerica => "north-america",
            Self::SouthAmerica => "south-america",
            Self::Oceania => "oceania",
            Self::Global => "",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Africa => "africa",
            Self::Asia => "asia",
            Self::Europe => "europe",
            Self::NorthAmerica => "north-america",
            Self::SouthAmerica => "south-america",
            Self::Oceania => "oceania",
            Self::Global => "global",
        }
    }
}

/// Maximum number of automatically selected pool servers (`0`..`3`).
pub const NTP_POOL_SERVER_COUNT: usize = 4;

/// Maximum length of a constructed pool hostname (`3.north-america.pool.ntp.org`).
pub const NTP_HOSTNAME_MAX: usize = 48;

/// Derive the NTP pool region from an IANA timezone id (e.g. `Asia/Baku`).
///
/// This is pure static metadata: it does not need wall-clock time and cannot
/// create a circular dependency with NTP synchronization.
pub fn ntp_region_from_zone_id(zone_id: &str) -> NtpRegion {
    let id = zone_id.trim();
    if id.is_empty() || eq_ignore_ascii_case(id, "UTC") || eq_ignore_ascii_case(id, "GMT") {
        return NtpRegion::Global;
    }

    // Prefer the IANA continent / top-level component when present.
    let (head, tail) = match id.find('/') {
        Some(pos) => (&id[..pos], &id[pos + 1..]),
        None => (id, ""),
    };

    match head {
        "Africa" => NtpRegion::Africa,
        "Asia" => NtpRegion::Asia,
        "Europe" => NtpRegion::Europe,
        "America" => america_region(tail),
        "Pacific" | "Australia" | "Indian" | "Antarctica" => NtpRegion::Oceania,
        "Atlantic" => atlantic_region(tail),
        "Arctic" => NtpRegion::Europe,
        "US" | "Canada" | "Mexico" | "Navajo" => NtpRegion::NorthAmerica,
        "Brazil" | "Chile" => NtpRegion::SouthAmerica,
        "Etc" | "Factory" | "SystemV" => NtpRegion::Global,
        // Legacy aliases that still appear in the CSV database.
        "MIT" | "NST" | "HST" | "AST" | "PST" | "MST" | "CST" | "EST" | "PNT" | "IET" | "PRT"
        | "CNT" | "AGT" | "BET" | "CAT" | "EET" | "ART" | "ECT" | "EAT" | "MET" | "NET" | "PLT"
        | "IST" | "BST" | "VST" | "CTT" | "JST" | "ACT" | "AET" | "SST" => legacy_region(head),
        _ => NtpRegion::Global,
    }
}

/// Derive region from a CSV `TzEntry.region` field plus optional city tail.
pub fn ntp_region_from_parts(region: &str, city: &str) -> NtpRegion {
    if region.is_empty() {
        return NtpRegion::Global;
    }
    // Reconstruct a pseudo id when possible so America/* classification works.
    if city.is_empty() {
        ntp_region_from_zone_id(region)
    } else {
        // region may already be "America/Argentina" for nested ids.
        let mut buf = [0u8; 96];
        let r = region.as_bytes();
        let c = city.as_bytes();
        if r.len() + 1 + c.len() > buf.len() {
            return ntp_region_from_zone_id(region);
        }
        buf[..r.len()].copy_from_slice(r);
        buf[r.len()] = b'/';
        buf[r.len() + 1..r.len() + 1 + c.len()].copy_from_slice(c);
        let id = core::str::from_utf8(&buf[..r.len() + 1 + c.len()]).unwrap_or(region);
        ntp_region_from_zone_id(id)
    }
}

/// Build the four standard pool hostnames for a region into `out`.
///
/// Each hostname is null-terminated and at most `NTP_HOSTNAME_MAX` bytes
/// including the terminator.
pub fn pool_hostnames(
    region: NtpRegion,
    out: &mut [[u8; NTP_HOSTNAME_MAX]; NTP_POOL_SERVER_COUNT],
) {
    for (i, slot) in out.iter_mut().enumerate() {
        format_pool_hostname_into(i as u8, region, slot);
    }
}

/// Format a single pool hostname for index `0..=3` into `out` (null-terminated).
/// Returns the length excluding the terminator.
pub fn format_pool_hostname_into(
    index: u8,
    region: NtpRegion,
    out: &mut [u8; NTP_HOSTNAME_MAX],
) -> usize {
    *out = [0u8; NTP_HOSTNAME_MAX];
    let mut pos = 0usize;
    out[pos] = b'0' + (index % 10);
    pos += 1;
    out[pos] = b'.';
    pos += 1;
    let sub = region.pool_subdomain();
    if !sub.is_empty() {
        let sb = sub.as_bytes();
        out[pos..pos + sb.len()].copy_from_slice(sb);
        pos += sb.len();
        out[pos] = b'.';
        pos += 1;
    }
    let tail = b"pool.ntp.org";
    out[pos..pos + tail.len()].copy_from_slice(tail);
    pos += tail.len();
    pos
}

/// Host test helper: return an owned hostname string.
#[cfg(test)]
fn format_pool_hostname(index: u8, region: NtpRegion) -> alloc::string::String {
    let mut buf = [0u8; NTP_HOSTNAME_MAX];
    let n = format_pool_hostname_into(index, region, &mut buf);
    core::str::from_utf8(&buf[..n]).unwrap_or("").into()
}

fn america_region(tail: &str) -> NtpRegion {
    // Nested regions: America/Argentina/*, America/Indiana/*, etc.
    let primary = match tail.find('/') {
        Some(pos) => &tail[..pos],
        None => tail,
    };
    if is_south_america_zone(primary) {
        NtpRegion::SouthAmerica
    } else {
        NtpRegion::NorthAmerica
    }
}

fn atlantic_region(tail: &str) -> NtpRegion {
    match tail {
        "Cape_Verde" | "St_Helena" | "South_Georgia" | "Stanley" => NtpRegion::Africa,
        // Azores, Canary, Faroe, Madeira, Reykjavik, Bermuda, …
        _ => NtpRegion::Europe,
    }
}

fn is_south_america_zone(name: &str) -> bool {
    // Continental South America only. Central America and the Caribbean use
    // the North America pool (default for the rest of America/*).
    matches!(
        name,
        "Argentina"
            | "Buenos_Aires"
            | "Catamarca"
            | "Cordoba"
            | "Jujuy"
            | "Mendoza"
            | "Rosario"
            | "ComodRivadavia"
            | "Sao_Paulo"
            | "Rio_Branco"
            | "Porto_Velho"
            | "Porto_Acre"
            | "Noronha"
            | "Recife"
            | "Fortaleza"
            | "Belem"
            | "Bahia"
            | "Maceio"
            | "Araguaina"
            | "Santarem"
            | "Manaus"
            | "Cuiaba"
            | "Campo_Grande"
            | "Boa_Vista"
            | "Eirunepe"
            | "Santiago"
            | "Punta_Arenas"
            | "Easter"
            | "Lima"
            | "Bogota"
            | "Caracas"
            | "La_Paz"
            | "Asuncion"
            | "Montevideo"
            | "Guyana"
            | "Paramaribo"
            | "Cayenne"
            | "Guayaquil"
            | "Quito"
    )
}

fn legacy_region(code: &str) -> NtpRegion {
    match code {
        "JST" | "CTT" | "VST" | "IST" | "PLT" | "NET" | "BST" => NtpRegion::Asia,
        "EET" | "MET" | "ART" | "ECT" | "CAT" => NtpRegion::Europe,
        "EAT" => NtpRegion::Africa,
        "AET" | "ACT" | "SST" | "NST" => NtpRegion::Oceania,
        "AST" | "PST" | "MST" | "CST" | "EST" | "PNT" | "IET" | "PRT" | "CNT" | "HST" => {
            NtpRegion::NorthAmerica
        }
        "AGT" | "BET" => NtpRegion::SouthAmerica,
        _ => NtpRegion::Global,
    }
}

fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes()
        .iter()
        .zip(b.as_bytes())
        .all(|(&x, &y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

#[cfg(test)]
extern crate alloc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asia_baku_and_tehran_select_asia_pool() {
        assert_eq!(ntp_region_from_zone_id("Asia/Baku"), NtpRegion::Asia);
        assert_eq!(ntp_region_from_zone_id("Asia/Tehran"), NtpRegion::Asia);
        let mut hosts = [[0u8; NTP_HOSTNAME_MAX]; NTP_POOL_SERVER_COUNT];
        pool_hostnames(NtpRegion::Asia, &mut hosts);
        assert_eq!(
            core::str::from_utf8(&hosts[0])
                .unwrap()
                .trim_end_matches('\0'),
            "0.asia.pool.ntp.org"
        );
        assert_eq!(
            core::str::from_utf8(&hosts[3])
                .unwrap()
                .trim_end_matches('\0'),
            "3.asia.pool.ntp.org"
        );
    }

    #[test]
    fn unknown_and_utc_use_global_fallback() {
        assert_eq!(ntp_region_from_zone_id("UTC"), NtpRegion::Global);
        assert_eq!(ntp_region_from_zone_id("Etc/UTC"), NtpRegion::Global);
        assert_eq!(ntp_region_from_zone_id("NoSuch/Place"), NtpRegion::Global);
        assert_eq!(format_pool_hostname(1, NtpRegion::Global), "1.pool.ntp.org");
    }

    #[test]
    fn america_classification() {
        assert_eq!(
            ntp_region_from_zone_id("America/New_York"),
            NtpRegion::NorthAmerica
        );
        assert_eq!(
            ntp_region_from_zone_id("America/Sao_Paulo"),
            NtpRegion::SouthAmerica
        );
        assert_eq!(
            ntp_region_from_zone_id("America/Argentina/Buenos_Aires"),
            NtpRegion::SouthAmerica
        );
        assert_eq!(
            ntp_region_from_zone_id("US/Pacific"),
            NtpRegion::NorthAmerica
        );
    }

    #[test]
    fn europe_africa_oceania() {
        assert_eq!(ntp_region_from_zone_id("Europe/London"), NtpRegion::Europe);
        assert_eq!(ntp_region_from_zone_id("Africa/Cairo"), NtpRegion::Africa);
        assert_eq!(
            ntp_region_from_zone_id("Australia/Sydney"),
            NtpRegion::Oceania
        );
        assert_eq!(
            ntp_region_from_zone_id("Pacific/Auckland"),
            NtpRegion::Oceania
        );
    }

    #[test]
    fn does_not_use_offset_semantics() {
        // Same +04:00 offset can appear in Asia and Europe; selection is by id.
        assert_eq!(ntp_region_from_zone_id("Asia/Baku"), NtpRegion::Asia);
        assert_eq!(ntp_region_from_zone_id("Europe/Samara"), NtpRegion::Europe);
    }
}
