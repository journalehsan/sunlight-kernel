//! Source-root configuration and indexer settings.

use alloc::string::String;
use alloc::vec::Vec;

use wiseowl_memorydb::record::{MemoryScope, OwnerId};

use crate::error::IndexError;
use crate::hash::fnv1a64;
use crate::quotas::IndexQuotaConfig;

/// Stable root identifier (non-zero, restart-surviving when persisted).
pub type RootId = u64;

/// Allowed extension set (lowercase without leading dot, e.g. "md").
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct BoundedExtensionSet {
    pub extensions: Vec<String>,
}

impl BoundedExtensionSet {
    pub fn default_phase3() -> Self {
        Self {
            extensions: [
                "txt", "md", "rst", "toml", "json", "yaml", "yml", "csv", "log",
                // optional plain source
                "rs", "c", "h", "cpp", "hpp", "py", "js", "ts", "tsx", "jsx", "html", "css", "sh",
            ]
            .iter()
            .map(|s| String::from(*s))
            .collect(),
        }
    }

    pub fn contains(&self, ext: &str) -> bool {
        let lower = ext_to_lower(ext);
        self.extensions.iter().any(|e| e == &lower)
    }
}

fn ext_to_lower(ext: &str) -> String {
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

/// Explicit authorized indexing root.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct IndexRootConfig {
    pub root_id: RootId,
    /// Absolute path string (resolved at registration; never a developer home hard-code).
    pub path: String,
    pub scope: MemoryScope,
    pub owner: OwnerId,
    pub enabled: bool,
    pub recursive: bool,
    pub maximum_depth: u16,
    pub follow_symlinks: bool,
    pub stay_on_filesystem: bool,
    pub include_hidden: bool,
    pub maximum_file_size: u64,
    pub allowed_extensions: BoundedExtensionSet,
    /// Root available flag (cleared when mount disappears).
    pub available: bool,
}

impl IndexRootConfig {
    pub fn validate(&self, quotas: &IndexQuotaConfig) -> Result<(), IndexError> {
        if self.root_id == 0 {
            return Err(IndexError::InvalidValue("root_id zero"));
        }
        if self.path.is_empty() || self.path.len() as u16 > quotas.max_path_bytes {
            return Err(IndexError::PathRejected("path length"));
        }
        if self.path.contains('\0') {
            return Err(IndexError::PathRejected("nul in path"));
        }
        if self.maximum_depth == 0 || self.maximum_depth > quotas.max_traversal_depth {
            return Err(IndexError::InvalidValue("maximum_depth"));
        }
        if self.maximum_file_size == 0 || self.maximum_file_size > quotas.max_file_size_bytes {
            return Err(IndexError::InvalidValue("maximum_file_size"));
        }
        // Default: never follow symlinks unless explicitly enabled (still validated).
        Ok(())
    }
}

/// Top-level indexer configuration.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct IndexerConfig {
    pub quotas: IndexQuotaConfig,
    pub roots: Vec<IndexRootConfig>,
    /// When true, attempt to register ~/Documents as initial root if identity APIs allow.
    pub default_documents_root: bool,
    /// Parser / tokenizer / chunker versions (force re-index when bumped).
    pub parser_version: u32,
    pub tokenizer_id: u32,
    pub tokenizer_version: u32,
    pub chunking_id: u32,
    pub chunking_version: u32,
    /// Ignore-rule configuration generation (bump when global ignores change).
    pub ignore_config_version: u32,
    /// Background scan interval hint (nanoseconds). 0 = event/manual only.
    /// Host daemon uses this as a sleep between idle polls; never a busy loop.
    pub background_scan_interval_ns: u64,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            quotas: IndexQuotaConfig::default(),
            roots: Vec::new(),
            default_documents_root: true,
            parser_version: 1,
            tokenizer_id: WISEOWL_LEXICAL_V1_ID,
            tokenizer_version: 1,
            chunking_id: CHUNKING_ID_V1,
            chunking_version: 1,
            ignore_config_version: 1,
            // 5 minutes — conservative, not a busy poll.
            background_scan_interval_ns: 300_000_000_000,
        }
    }
}

/// WiseOwlLexicalV1 tokenizer identity.
pub const WISEOWL_LEXICAL_V1_ID: u32 = 1;
/// Chunking profile id for Phase 3 block-preserving chunker.
pub const CHUNKING_ID_V1: u32 = 1;
/// Plain text parser id.
pub const PARSER_PLAIN: u32 = 1;
/// Markdown / RST parser id.
pub const PARSER_MARKDOWN: u32 = 2;
/// JSON / TOML structured parser id.
pub const PARSER_STRUCTURED: u32 = 3;
/// CSV parser id.
pub const PARSER_CSV: u32 = 4;
/// YAML-as-text fallback parser id.
pub const PARSER_YAML_TEXT: u32 = 5;

/// Resolve a documents path under a home directory (no hard-coded developer home).
pub fn documents_path_under_home(home: &str) -> Result<String, IndexError> {
    if home.is_empty() || home.contains('\0') {
        return Err(IndexError::PathRejected("home"));
    }
    let mut p = String::from(home);
    if !p.ends_with('/') {
        p.push('/');
    }
    p.push_str("Documents");
    Ok(p)
}

/// Hash of root configuration for change detection.
pub fn root_config_hash(root: &IndexRootConfig) -> u64 {
    let mut buf = alloc::vec::Vec::new();
    buf.extend_from_slice(&root.root_id.to_le_bytes());
    buf.extend_from_slice(root.path.as_bytes());
    buf.push(root.scope.as_u8());
    buf.extend_from_slice(&root.owner.to_le_bytes());
    buf.push(root.enabled as u8);
    buf.push(root.recursive as u8);
    buf.extend_from_slice(&root.maximum_depth.to_le_bytes());
    buf.push(root.follow_symlinks as u8);
    buf.push(root.include_hidden as u8);
    fnv1a64(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_default_contains_md() {
        let e = BoundedExtensionSet::default_phase3();
        assert!(e.contains("md"));
        assert!(e.contains("MD"));
        assert!(!e.contains("pdf"));
        assert!(!e.contains("docx"));
    }

    #[test]
    fn documents_under_home() {
        let p = documents_path_under_home("/home/user").unwrap();
        assert_eq!(p, "/home/user/Documents");
    }
}
