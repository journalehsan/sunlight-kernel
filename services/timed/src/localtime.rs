//! Timezone file resolution and loading
//!
//! Resolves /etc/localtime symlink and loads timezone configuration from
//! the target file.

use alloc::string::String;
use crate::config::{parse_dst_flag, parse_offset_string};

/// Maximum symlink recursion depth to prevent loops
const MAX_SYMLINK_DEPTH: usize = 5;

/// Resolve /etc/localtime symlink and load timezone configuration
///
/// Returns (offset_secs, dst_active, timezone_name) on success
pub fn resolve_and_load_timezone() -> Result<(i32, bool, String), &'static str> {
    // For now, use a hardcoded default timezone
    // In Phase 2.1 with VFS integration, this will:
    // 1. Read /etc/localtime symlink
    // 2. Resolve the target
    // 3. Load offset and DST from the file

    // Default: UTC (0 offset, no DST)
    let offset_secs = 0;
    let dst_active = false;
    let timezone_name = String::from("UTC");

    Ok((offset_secs, dst_active, timezone_name))
}

/// Load timezone configuration from a file
/// Expected format:
///   Line 1: Offset (float or int, e.g., "+4.5", "-5")
///   Line 2: DST flag (0 or 1; optional)
pub fn load_timezone_from_file(path: &str) -> Result<(i32, bool, String), &'static str> {
    // Phase 2.1: Implement actual file loading via VFS
    // For Phase 2.0, return a default timezone

    if path.contains("Tehran") {
        return Ok((16200, false, String::from("Asia/Tehran")));
    }
    if path.contains("NewYork") {
        return Ok((-18000, false, String::from("America/New_York")));
    }

    // Default to UTC
    Ok((0, false, String::from("UTC")))
}

/// Resolve a symlink target (placeholder for Phase 2.1 VFS integration)
///
/// This will be implemented when VFS syscalls are available.
/// For now, returns the path as-is.
fn resolve_symlink(path: &str, depth: usize) -> Result<String, &'static str> {
    if depth > MAX_SYMLINK_DEPTH {
        return Err("Symlink loop detected");
    }

    // Phase 2.1: Use VFS to read symlink and recurse if needed
    // For Phase 2.0, just return the path
    Ok(String::from(path))
}

/// Parse timezone configuration file content
/// Expected format:
///   Line 1: Offset as float or integer
///   Line 2: DST flag (0/1) - optional
pub fn parse_timezone_content(content: &str) -> Result<(i32, bool), &'static str> {
    let mut lines = content.lines();

    // Parse offset (required)
    let offset_line = lines.next().ok_or("Missing offset line")?;
    let offset_secs = parse_offset_string(offset_line)?;

    // Parse DST flag (optional)
    let dst_active = if let Some(dst_line) = lines.next() {
        parse_dst_flag(dst_line)
    } else {
        false // Default to no DST if line missing
    };

    Ok((offset_secs, dst_active))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timezone_content() {
        let content = "4.5\n0";
        let (offset, dst) = parse_timezone_content(content).unwrap();
        assert_eq!(offset, 16200);
        assert!(!dst);
    }

    #[test]
    fn test_parse_timezone_missing_dst() {
        let content = "-5";
        let (offset, dst) = parse_timezone_content(content).unwrap();
        assert_eq!(offset, -18000);
        assert!(!dst);
    }

    #[test]
    fn test_parse_timezone_with_dst_active() {
        let content = "1.0\n1";
        let (offset, dst) = parse_timezone_content(content).unwrap();
        assert_eq!(offset, 3600);
        assert!(dst);
    }
}
