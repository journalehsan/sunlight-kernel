//! Path validation and symlink-safe relative path handling.
//!
//! Capability model note: Phase 3 path security is **strict string + root-prefix
//! validation**. SunlightOS filesystem directory-capability delegation is not yet
//! a general API; the weaker mechanism is:
//! 1. roots must be explicitly registered by an authorized caller
//! 2. every open re-validates the path stays under the authorized root
//! 3. symlink following is disabled by default
//! 4. `..` components and absolute escapes are rejected

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::IndexError;
use crate::quotas::IndexQuotaConfig;

/// Normalize path separators and reject dangerous patterns without resolving symlinks.
pub fn normalize_relative_components(rel: &str) -> Result<String, IndexError> {
    if rel.is_empty() {
        return Ok(String::new());
    }
    if rel.contains('\0') {
        return Err(IndexError::PathRejected("nul"));
    }
    // Reject absolute and drive-style paths as relative components.
    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(IndexError::PathRejected("absolute relative path"));
    }
    let mut out: Vec<&str> = Vec::new();
    for part in rel.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(IndexError::PathRejected("parent traversal"));
        }
        // Reject Windows device-ish names lightly.
        if part.contains(':') {
            return Err(IndexError::PathRejected("colon in component"));
        }
        out.push(part);
    }
    Ok(out.join("/"))
}

/// Join root absolute path with a validated relative path.
pub fn join_under_root(root: &str, rel: &str) -> Result<String, IndexError> {
    let rel_n = normalize_relative_components(rel)?;
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return Err(IndexError::PathRejected("empty root"));
    }
    if rel_n.is_empty() {
        return Ok(root.to_string());
    }
    let mut full = String::with_capacity(root.len() + 1 + rel_n.len());
    full.push_str(root);
    full.push('/');
    full.push_str(&rel_n);
    // Double-check no escape after join (string-level).
    if !path_is_under_root(root, &full) {
        return Err(IndexError::PathRejected("escaped root"));
    }
    Ok(full)
}

/// String-level check that `candidate` equals root or is under `root/`.
pub fn path_is_under_root(root: &str, candidate: &str) -> bool {
    let root = root.trim_end_matches('/');
    if candidate == root {
        return true;
    }
    let mut prefix = String::from(root);
    prefix.push('/');
    candidate.starts_with(&prefix)
}

/// Reject hidden path components unless include_hidden.
pub fn has_hidden_component(rel: &str) -> bool {
    for part in rel.split('/') {
        if part.starts_with('.') && part != "." && part != ".." {
            return true;
        }
    }
    false
}

/// Extract lowercase extension without dot from a relative path.
pub fn extension_of(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.starts_with('.') {
        // Dotfile without further extension: no extension.
        if !name[1..].contains('.') {
            return None;
        }
    }
    let dot = name.rfind('.')?;
    if dot == 0 || dot + 1 >= name.len() {
        return None;
    }
    let ext = &name[dot + 1..];
    if ext.is_empty() || ext.len() > 16 {
        return None;
    }
    Some(ext_lower(ext))
}

fn ext_lower(ext: &str) -> String {
    let mut s = String::with_capacity(ext.len());
    for b in ext.bytes() {
        s.push(if (b'A'..=b'Z').contains(&b) {
            (b + 32) as char
        } else {
            b as char
        });
    }
    s
}

/// Validate a discovered path against root policy before open.
pub fn validate_discovered_path(
    root_path: &str,
    rel: &str,
    depth: u16,
    max_depth: u16,
    include_hidden: bool,
    quotas: &IndexQuotaConfig,
) -> Result<String, IndexError> {
    if depth > max_depth {
        return Err(IndexError::PathRejected("depth"));
    }
    if rel.len() as u16 > quotas.max_relative_path_bytes {
        return Err(IndexError::PathRejected("relative path length"));
    }
    if !include_hidden && has_hidden_component(rel) {
        return Err(IndexError::PathRejected("hidden"));
    }
    join_under_root(root_path, rel)
}

/// Depth of a relative path (0 for file in root).
pub fn relative_depth(rel: &str) -> u16 {
    if rel.is_empty() {
        return 0;
    }
    rel.bytes().filter(|&b| b == b'/').count() as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_traversal() {
        assert!(normalize_relative_components("../etc/passwd").is_err());
        assert!(normalize_relative_components("a/../../b").is_err());
    }

    #[test]
    fn rejects_absolute_relative() {
        assert!(normalize_relative_components("/etc/passwd").is_err());
    }

    #[test]
    fn join_stays_under_root() {
        let j = join_under_root("/home/u/Documents", "notes/a.txt").unwrap();
        assert_eq!(j, "/home/u/Documents/notes/a.txt");
        assert!(path_is_under_root("/home/u/Documents", &j));
    }

    #[test]
    fn hidden_detection() {
        assert!(has_hidden_component(".git/config"));
        assert!(has_hidden_component("a/.cache/x"));
        assert!(!has_hidden_component("a/b.txt"));
    }

    #[test]
    fn extension() {
        assert_eq!(extension_of("foo.MD").as_deref(), Some("md"));
        assert_eq!(extension_of("noext").as_deref(), None);
    }
}
