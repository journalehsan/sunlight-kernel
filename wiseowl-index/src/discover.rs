//! Bounded filesystem discovery under authorized roots.
//!
//! Host implementation uses std::fs. Native uses sunlight-libc when available.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::config::IndexRootConfig;
use crate::error::IndexError;
use crate::ignore::IgnoreSet;
use crate::path_security::{
    extension_of, relative_depth, validate_discovered_path,
};
use crate::quotas::IndexQuotaConfig;

/// A discovered candidate file (relative path + metadata prefilter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    pub relative_path: String,
    pub size_bytes: u64,
    pub modified_at_ns: Option<u64>,
    pub file_identity: Option<crate::source::FileIdentity>,
    pub extension: String,
    pub depth: u16,
}

/// Scan cursor for resumable discovery (deterministic lexicographic order).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct ScanCursor {
    pub root_id: u64,
    /// Next relative path to continue after (exclusive lower bound).
    pub after_relative_path: String,
    pub directories_visited: u32,
    pub files_inspected: u32,
    pub exhausted: bool,
}

/// Discovery budget snapshot.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscoverBudget {
    pub directories_visited: u32,
    pub files_inspected: u32,
}

/// List files under a root with bounds (host).
#[cfg(feature = "host")]
pub fn discover_files_host(
    root: &IndexRootConfig,
    ignore: &IgnoreSet,
    quotas: &IndexQuotaConfig,
    cursor: &ScanCursor,
    budget: &mut DiscoverBudget,
) -> Result<(Vec<DiscoveredFile>, ScanCursor), IndexError> {
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    if !root.enabled {
        return Ok((
            Vec::new(),
            ScanCursor {
                root_id: root.root_id,
                exhausted: true,
                ..cursor.clone()
            },
        ));
    }

    let root_path = Path::new(&root.path);
    if !root_path.is_dir() {
        return Err(IndexError::RootUnavailable);
    }

    let mut stack: Vec<(PathBuf, String, u16)> = Vec::new();
    stack.push((root_path.to_path_buf(), String::new(), 0));

    let mut found = Vec::new();
    let mut next_cursor = cursor.clone();
    next_cursor.root_id = root.root_id;
    next_cursor.exhausted = false;

    // Collect then sort for deterministic order; process with cursor filter.
    let mut all_paths: Vec<(String, PathBuf, u16)> = Vec::new();

    while let Some((dir, rel, depth)) = stack.pop() {
        if budget.directories_visited >= quotas.max_directories_per_scan {
            break;
        }
        budget.directories_visited = budget.directories_visited.saturating_add(1);

        let read = fs::read_dir(&dir).map_err(|_| IndexError::Io("readdir"))?;
        let mut entries: Vec<_> = read
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for ent in entries {
            let name = ent.file_name();
            let name_str = name.to_string_lossy();
            if name_str == "." || name_str == ".." {
                continue;
            }
            let child_rel = if rel.is_empty() {
                name_str.to_string()
            } else {
                alloc::format!("{rel}/{name_str}")
            };

            // Symlink policy: do not follow by default.
            let ft = ent.file_type().map_err(|_| IndexError::Io("file_type"))?;
            if ft.is_symlink() && !root.follow_symlinks {
                continue;
            }

            let is_dir = ft.is_dir() || (ft.is_symlink() && root.follow_symlinks && ent.path().is_dir());
            if ignore.is_ignored(&child_rel, is_dir) {
                continue;
            }

            if is_dir {
                if !root.recursive {
                    continue;
                }
                let next_depth = depth.saturating_add(1);
                if next_depth > root.maximum_depth {
                    continue;
                }
                if !root.include_hidden && name_str.starts_with('.') {
                    continue;
                }
                stack.push((ent.path(), child_rel, next_depth));
            } else if ft.is_file() || (ft.is_symlink() && root.follow_symlinks) {
                all_paths.push((child_rel, ent.path(), depth));
            }
        }
        // Process stack in reverse sorted order for DFS lexicographic-ish;
        // we sort all_paths later.
        stack.sort_by(|a, b| b.1.cmp(&a.1)); // pop small first → reverse
    }

    all_paths.sort_by(|a, b| a.0.cmp(&b.0));

    for (rel, path, depth) in all_paths {
        if !cursor.after_relative_path.is_empty() && rel.as_str() <= cursor.after_relative_path.as_str()
        {
            continue;
        }
        if budget.files_inspected >= quotas.max_files_inspected_per_scan {
            next_cursor.after_relative_path = rel;
            next_cursor.directories_visited = budget.directories_visited;
            next_cursor.files_inspected = budget.files_inspected;
            return Ok((found, next_cursor));
        }
        budget.files_inspected = budget.files_inspected.saturating_add(1);

        if let Err(_) = validate_discovered_path(
            &root.path,
            &rel,
            relative_depth(&rel),
            root.maximum_depth,
            root.include_hidden,
            quotas,
        ) {
            continue;
        }

        let ext = match extension_of(&rel) {
            Some(e) if root.allowed_extensions.contains(&e) => e,
            _ => continue,
        };

        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue, // disappeared
        };
        if !meta.is_file() {
            continue;
        }
        let size = meta.len();
        if size > root.maximum_file_size {
            // Still surface for failure tracking in scan layer via oversized flag
            found.push(DiscoveredFile {
                relative_path: rel.clone(),
                size_bytes: size,
                modified_at_ns: mtime_ns(&meta),
                file_identity: Some(crate::source::FileIdentity {
                    device: meta.dev(),
                    inode: meta.ino(),
                }),
                extension: ext,
                depth,
            });
            next_cursor.after_relative_path = rel;
            continue;
        }

        found.push(DiscoveredFile {
            relative_path: rel.clone(),
            size_bytes: size,
            modified_at_ns: mtime_ns(&meta),
            file_identity: Some(crate::source::FileIdentity {
                device: meta.dev(),
                inode: meta.ino(),
            }),
            extension: ext,
            depth,
        });
        next_cursor.after_relative_path = rel;
    }

    next_cursor.exhausted = true;
    next_cursor.directories_visited = budget.directories_visited;
    next_cursor.files_inspected = budget.files_inspected;
    Ok((found, next_cursor))
}

#[cfg(feature = "host")]
fn mtime_ns(meta: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    // mtime sec * 1e9 + nsec if available
    let secs = meta.mtime() as u64;
    let nsec = meta.mtime_nsec() as u64;
    Some(secs.saturating_mul(1_000_000_000).saturating_add(nsec))
}

/// In-memory tree discovery for unit tests (no real FS).
pub fn discover_from_listing(
    root: &IndexRootConfig,
    listing: &[(String, u64)],
    ignore: &IgnoreSet,
    quotas: &IndexQuotaConfig,
    cursor: &ScanCursor,
) -> Result<(Vec<DiscoveredFile>, ScanCursor), IndexError> {
    let mut files: Vec<_> = listing
        .iter()
        .filter(|(rel, _)| {
            if !cursor.after_relative_path.is_empty() && rel.as_str() <= cursor.after_relative_path.as_str() {
                return false;
            }
            if ignore.is_ignored(rel, false) {
                return false;
            }
            if let Some(ext) = extension_of(rel) {
                root.allowed_extensions.contains(&ext)
            } else {
                false
            }
        })
        .cloned()
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = Vec::new();
    let mut next = cursor.clone();
    next.root_id = root.root_id;
    for (i, (rel, size)) in files.iter().enumerate() {
        if out.len() as u32 >= quotas.max_files_inspected_per_scan {
            next.after_relative_path = rel.clone();
            next.exhausted = false;
            return Ok((out, next));
        }
        let ext = extension_of(rel).unwrap_or_default();
        out.push(DiscoveredFile {
            relative_path: rel.clone(),
            size_bytes: *size,
            modified_at_ns: None,
            file_identity: None,
            extension: ext,
            depth: relative_depth(rel),
        });
        next.after_relative_path = rel.clone();
        if i + 1 == files.len() {
            next.exhausted = true;
        }
    }
    if files.is_empty() {
        next.exhausted = true;
    }
    Ok((out, next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BoundedExtensionSet, IndexRootConfig};
    use wiseowl_memorydb::record::MemoryScope;

    fn root() -> IndexRootConfig {
        IndexRootConfig {
            root_id: 1,
            path: String::from("/tmp/docs"),
            scope: MemoryScope::User,
            owner: 1,
            enabled: true,
            recursive: true,
            maximum_depth: 8,
            follow_symlinks: false,
            stay_on_filesystem: true,
            include_hidden: false,
            maximum_file_size: 48 * 1024,
            allowed_extensions: BoundedExtensionSet::default_phase3(),
            available: true,
        }
    }

    #[test]
    fn listing_filters_and_cursor() {
        let r = root();
        let ig = IgnoreSet::builtin();
        let q = IndexQuotaConfig::default();
        let listing = vec![
            (String::from("a.txt"), 10),
            (String::from("b.tmp"), 10),
            (String::from("c.md"), 20),
            (String::from("d.pdf"), 20),
        ];
        let (files, cur) = discover_from_listing(&r, &listing, &ig, &q, &ScanCursor::default())
            .unwrap();
        assert!(files.iter().any(|f| f.relative_path == "a.txt"));
        assert!(files.iter().any(|f| f.relative_path == "c.md"));
        assert!(!files.iter().any(|f| f.relative_path == "b.tmp"));
        assert!(!files.iter().any(|f| f.relative_path == "d.pdf"));
        assert!(cur.exhausted);
    }
}
