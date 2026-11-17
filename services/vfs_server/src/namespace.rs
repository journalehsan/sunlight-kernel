//! Operations on the service's mounted namespace, shared by dispatch adapters.
//! Caller credentials must be authenticated by the adapter, never payload fields.
use sunlight_fs::{check_permission, Credential, FileStat, FileType, FsError, PermCheck, Vfs};

/// Maximum captured path supported by the initial mounted-namespace adapter.
/// Reject before traversal rather than silently truncating a caller's path.
pub const MAX_ROOT_PATH: usize = 255;

/// Metadata lookup in the live service root. Relative paths and alternate roots
/// require retained directory objects and are not supported by this adapter.
/// Check each traversed parent before interpreting `.` or `..`: lexical cleanup
/// alone would let a caller skip a denied or nonexistent directory.
pub fn stat_root(vfs: &mut Vfs, path: &str, caller: Credential) -> Result<FileStat, FsError> {
    if !path.starts_with('/') || path.len() > MAX_ROOT_PATH || path.as_bytes().contains(&0) {
        return Err(FsError::InvalidPath);
    }
    let mut current = heapless::String::<MAX_ROOT_PATH>::new();
    current.push('/').map_err(|_| FsError::InvalidPath)?;
    for component in path.split('/').filter(|part| !part.is_empty()) {
        search(vfs, current.as_str(), caller)?;
        match component {
            "." => {}
            ".." => {
                let parent = current.rfind('/').unwrap_or(0);
                current.truncate(parent.max(1));
            }
            name => {
                if current.len() > 1 {
                    current.push('/').map_err(|_| FsError::InvalidPath)?;
                }
                current.push_str(name).map_err(|_| FsError::InvalidPath)?;
            }
        }
    }
    let stat = vfs.stat(current.as_str())?;
    if path.ends_with('/') && stat.file_type != FileType::Directory {
        return Err(FsError::NotDir);
    }
    Ok(stat)
}

fn search(vfs: &mut Vfs, path: &str, caller: Credential) -> Result<(), FsError> {
    let stat = vfs.stat(path)?;
    if stat.file_type != FileType::Directory {
        return Err(FsError::NotDir);
    }
    if !check_permission(&stat, &caller, PermCheck::Execute) {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunlight_fs::Tmpfs;

    #[test]
    fn lookup_observes_the_live_service_namespace_and_search_permissions() {
        let mut vfs = Vfs::new();
        vfs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
        vfs.mkdir("/private", 42, 42, 0o700).unwrap();
        let handle = vfs
            .create_file_exclusive("/private/data", 42, 42, 0o000)
            .unwrap();
        vfs.write(handle, 0, b"live").unwrap();
        let owner = Credential { uid: 42, gid: 42 };
        assert_eq!(stat_root(&mut vfs, "/private/data", owner).unwrap().size, 4);
        assert_eq!(
            stat_root(&mut vfs, "/private/data", Credential { uid: 43, gid: 43 }),
            Err(FsError::PermissionDenied)
        );
        vfs.write(handle, 4, b"!").unwrap();
        assert_eq!(stat_root(&mut vfs, "/private/data", owner).unwrap().size, 5);
        vfs.close(handle).unwrap();
    }

    #[test]
    fn lookup_rejects_invalid_paths_and_file_parents() {
        let mut vfs = Vfs::new();
        vfs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
        let root = Credential { uid: 0, gid: 0 };
        for path in ["", "relative", "/bad\0path"] {
            assert_eq!(stat_root(&mut vfs, path, root), Err(FsError::InvalidPath));
        }
        let h = vfs.create_file_exclusive("/file", 0, 0, 0o644).unwrap();
        vfs.close(h).unwrap();
        assert_eq!(
            stat_root(&mut vfs, "/file/child", root),
            Err(FsError::NotDir)
        );
    }
}

#[cfg(test)]
mod traversal_tests {
    use super::*;
    use sunlight_fs::Tmpfs;

    #[test]
    fn traversal_checks_components_before_parent_navigation() {
        let mut vfs = Vfs::new();
        vfs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
        vfs.mkdir("/denied", 42, 42, 0o700).unwrap();
        vfs.mkdir("/visible", 43, 43, 0o755).unwrap();
        let caller = Credential { uid: 43, gid: 43 };
        assert_eq!(
            stat_root(&mut vfs, "/denied/../visible", caller),
            Err(FsError::PermissionDenied)
        );
        assert_eq!(
            stat_root(&mut vfs, "/missing/../visible", caller),
            Err(FsError::NotFound)
        );
        assert_eq!(
            stat_root(&mut vfs, "/visible/./../visible//", caller)
                .unwrap()
                .file_type,
            FileType::Directory
        );
        assert_eq!(stat_root(&mut vfs, "/../../", caller), vfs.stat("/"));
        let h = vfs.create_file_exclusive("/file", 43, 43, 0o644).unwrap();
        vfs.close(h).unwrap();
        for path in ["/file/", "/file/.", "/file/.."] {
            assert_eq!(stat_root(&mut vfs, path, caller), Err(FsError::NotDir));
        }
        let oversized = alloc::format!("/{}", "a".repeat(MAX_ROOT_PATH));
        assert_eq!(
            stat_root(&mut vfs, &oversized, caller),
            Err(FsError::InvalidPath)
        );
    }

    #[test]
    fn traversal_crosses_live_mounts_and_preserves_utf8_parents() {
        let mut vfs = Vfs::new();
        vfs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
        vfs.mkdir("/mounted", 0, 0, 0o755).unwrap();
        vfs.mount_tmpfs("/mounted", Tmpfs::new(&[])).unwrap();
        vfs.mkdir("/mounted/é", 0, 0, 0o755).unwrap();
        let caller = Credential { uid: 0, gid: 0 };
        assert_eq!(
            stat_root(&mut vfs, "/mounted/é/..", caller),
            vfs.stat("/mounted")
        );
        assert_eq!(stat_root(&mut vfs, "/mounted/../", caller), vfs.stat("/"));
    }
}
