//! Deterministic ignore rules for temporary and local ignore files.
//!
//! Grammar for `.wiseowlignore` (small subset):
//! - blank lines ignored
//! - `#` comments
//! - literal relative paths
//! - simple `*` and `?` wildcards (path-segment aware `*` does not cross `/`)
//! - trailing `/` means directory-only

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::IndexError;
use crate::hash::fnv1a64;
use crate::quotas::IndexQuotaConfig;

/// One ignore rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRule {
    pub pattern: String,
    pub directory_only: bool,
}

/// Compiled ignore set for a root.
#[derive(Debug, Clone, Default)]
pub struct IgnoreSet {
    pub rules: Vec<IgnoreRule>,
    /// Content hash of ignore sources for change detection.
    pub content_hash: u64,
}

impl IgnoreSet {
    /// Built-in temporary/editor/lock patterns (always active).
    pub fn builtin() -> Self {
        let patterns = [
            "*~",
            "*.tmp",
            "*.part",
            "*.swp",
            "*.swo",
            "*.lock",
            "*.bak",
            ".wiseowl-index/",
            ".wiseowl-memorydb/",
            "target/",
            ".git/",
            ".cache/",
        ];
        let mut rules = Vec::new();
        for p in patterns {
            let directory_only = p.ends_with('/');
            let pattern = p.trim_end_matches('/').to_string();
            rules.push(IgnoreRule {
                pattern,
                directory_only,
            });
        }
        let content_hash = fnv1a64(b"builtin-v1");
        Self {
            rules,
            content_hash,
        }
    }

    /// Parse `.wiseowlignore` body (UTF-8 text).
    pub fn parse_wiseowlignore(text: &str, quotas: &IndexQuotaConfig) -> Result<Self, IndexError> {
        let mut rules = Self::builtin().rules;
        let mut body_hash = crate::hash::Fnv1a64Hasher::new();
        body_hash.update(text.as_bytes());
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if rules.len() as u16 >= quotas.max_ignore_rules {
                return Err(IndexError::QuotaExceeded("ignore rules"));
            }
            let directory_only = line.ends_with('/');
            let pattern = line.trim_end_matches('/').to_string();
            if pattern.is_empty() || pattern.len() > 256 {
                return Err(IndexError::InvalidValue("ignore pattern"));
            }
            // No regex, no `**` recursion bombs.
            if pattern.contains("**") {
                return Err(IndexError::InvalidValue("** not supported"));
            }
            rules.push(IgnoreRule {
                pattern,
                directory_only,
            });
        }
        Ok(Self {
            rules,
            content_hash: body_hash.finish(),
        })
    }

    /// Return true if relative path should be ignored.
    pub fn is_ignored(&self, rel: &str, is_dir: bool) -> bool {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        for rule in &self.rules {
            if rule.directory_only && !is_dir {
                // Still ignore files under an ignored directory prefix.
                if path_matches_dir_prefix(rel, &rule.pattern) {
                    return true;
                }
                continue;
            }
            if glob_match(&rule.pattern, rel) || glob_match(&rule.pattern, name) {
                return true;
            }
            // Prefix directory match for patterns ending as path prefixes.
            if is_dir
                && (rel == rule.pattern || rel.starts_with(&alloc::format!("{}/", rule.pattern)))
            {
                return true;
            }
        }
        // Suffix-based temporary files (name ends with ~ etc.) already covered by patterns.
        if name.ends_with('~') {
            return true;
        }
        false
    }
}

fn path_matches_dir_prefix(rel: &str, dir_pat: &str) -> bool {
    if rel == dir_pat {
        return true;
    }
    let prefix = alloc::format!("{dir_pat}/");
    rel.starts_with(&prefix) || {
        // Also match if any path component equals dir_pat (e.g. .git)
        rel.split('/')
            .any(|c| c == dir_pat || glob_match(dir_pat, c))
    }
}

/// Simple glob: `*` = any chars except `/`, `?` = one char except `/`.
fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_rec(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_rec(pat: &[u8], text: &[u8]) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    while pi < pat.len() {
        match pat[pi] {
            b'*' => {
                // Match zero or more non-slash.
                pi += 1;
                if pi == pat.len() {
                    return !text[ti..].contains(&b'/');
                }
                while ti <= text.len() {
                    if glob_match_rec(&pat[pi..], &text[ti..]) {
                        return true;
                    }
                    if ti == text.len() || text[ti] == b'/' {
                        break;
                    }
                    ti += 1;
                }
                return false;
            }
            b'?' => {
                if ti >= text.len() || text[ti] == b'/' {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
            b => {
                if ti >= text.len() || text[ti] != b {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
        }
    }
    ti == text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_tmp_and_swp() {
        let ig = IgnoreSet::builtin();
        assert!(ig.is_ignored("notes.txt~", false));
        assert!(ig.is_ignored("file.tmp", false));
        assert!(ig.is_ignored("x.swp", false));
        assert!(ig.is_ignored("x.lock", false));
        assert!(!ig.is_ignored("notes.txt", false));
    }

    #[test]
    fn ignores_git_dir() {
        let ig = IgnoreSet::builtin();
        assert!(ig.is_ignored(".git", true));
        assert!(ig.is_ignored(".git/config", false));
    }

    #[test]
    fn wiseowlignore_literal() {
        let q = IndexQuotaConfig::default();
        let ig = IgnoreSet::parse_wiseowlignore("secrets/\n# comment\n*.bak\n", &q).unwrap();
        assert!(ig.is_ignored("secrets/a.txt", false));
        assert!(ig.is_ignored("old.bak", false));
    }

    #[test]
    fn glob_question() {
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(!glob_match("a*c", "a/b/c"));
    }
}
