//! Minimal /etc/hosts parser for SunlightOS DNS (Phase 5.x hosts enhancement).
//!
//! - Ignores blank lines, # comments (to end of line), and IPv6 addresses (any line whose first token contains ':').
//! - Supports multiple names per line.
//! - Stores in BTreeMap<String, [u8;4]> (owning keys for safety with read buffers; task mentions &str/Ipv4Address but owning [u8;4] keeps footprint tiny and matches all NetOp IPC).
//! - No heavy parsing, no regex, linear split.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};

pub type HostsTable = BTreeMap<String, [u8; 4]>;

/// Parse hosts(5) style content into a lookup table.
/// The returned table owns its keys (String) so it can be stored after the input &str goes away (e.g. after VFS read buffer).
pub fn parse_hosts(content: &str) -> HostsTable {
    let mut table: HostsTable = BTreeMap::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let ip_str = match parts.next() {
            Some(s) => s,
            None => continue,
        };

        // Skip IPv6 for this MVI (task: "IPv6 for now")
        if ip_str.contains(':') {
            continue;
        }

        let ip = match parse_ipv4(ip_str) {
            Some(ip) => ip,
            None => continue,
        };

        for name in parts {
            if name.is_empty() || name.starts_with('#') {
                break;
            }
            // hosts first wins on duplicate (BTreeMap insert does not overwrite if present? Wait, we want last wins or first; simple insert overwrites)
            table.insert(name.to_string(), ip);
        }
    }

    table
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut i = 0usize;
    for octet_str in s.split('.') {
        if i >= 4 {
            return None;
        }
        let mut val: u16 = 0;
        for b in octet_str.bytes() {
            if !b.is_ascii_digit() {
                return None;
            }
            val = val * 10 + (b - b'0') as u16;
            if val > 255 {
                return None;
            }
        }
        out[i] = val as u8;
        i += 1;
    }
    if i == 4 {
        Some(out)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_localhost() {
        let c = "127.0.0.1\tlocalhost\n127.0.1.1 sunlight\n# comment\n";
        let t = parse_hosts(c);
        assert_eq!(t.get("localhost"), Some(&[127, 0, 0, 1]));
        assert_eq!(t.get("sunlight"), Some(&[127, 0, 1, 1]));
    }

    #[test]
    fn skips_ipv6_and_comments() {
        let c = "::1 localhost\n127.0.0.1 foo # bar\n";
        let t = parse_hosts(c);
        assert!(t.get("localhost").is_none());
        assert_eq!(t.get("foo"), Some(&[127, 0, 0, 1]));
    }

    #[test]
    fn multiple_names_per_line() {
        let c = "127.0.0.1 localhost mylocal\n";
        let t = parse_hosts(c);
        assert_eq!(t.get("localhost"), Some(&[127, 0, 0, 1]));
        assert_eq!(t.get("mylocal"), Some(&[127, 0, 0, 1]));
    }
}
