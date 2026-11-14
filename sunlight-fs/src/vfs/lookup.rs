//! Existing-object lookup under the caller's exclusive VFS borrow. Canonical
//! names are observations, not object pins or authority for subsequent calls.
use super::*;
use crate::{check_permission, Credential, PermCheck};

/// Native path payload limit; syscall buffers include a further NUL byte.
pub const RESOLVE_PATH_MAX: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataSource {
    /// Backend stores and maintains this field.
    Stored,
    /// Mount policy supplies a value, not on-disk POSIX metadata.
    Policy,
    /// No representation or backend operation exists.
    Unsupported,
    /// Native representation does not currently expose a trustworthy value.
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub object_identity: MetadataSource,
    pub ownership_permissions: MetadataSource,
    pub timestamps: MetadataSource,
    pub symlinks: MetadataSource,
    pub posix_link_count: MetadataSource,
    pub pinned_metadata: MetadataSource,
}

impl BackendCapabilities {
    pub const TMPFS: Self = Self {
        object_identity: MetadataSource::Stored,
        ownership_permissions: MetadataSource::Stored,
        timestamps: MetadataSource::Unavailable,
        symlinks: MetadataSource::Unsupported,
        posix_link_count: MetadataSource::Unavailable,
        pinned_metadata: MetadataSource::Stored,
    };
    pub const FAT: Self = Self {
        object_identity: MetadataSource::Unavailable,
        ownership_permissions: MetadataSource::Policy,
        timestamps: MetadataSource::Unavailable,
        symlinks: MetadataSource::Unsupported,
        posix_link_count: MetadataSource::Unsupported,
        pinned_metadata: MetadataSource::Unavailable,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata {
    /// Existing native subset; consult capabilities before interpreting it as
    /// authoritative backend data. In particular, nlinks is not POSIX on dirs.
    pub native: FileStat,
    /// Mount incarnation, valid only within this VFS instance's lifetime.
    pub mount_id: u64,
    /// Backend object lifetime, never a handle slot or pathname hash. A local
    /// id of zero is valid; combine with mount_id. FAT exposes None.
    pub object_id: Option<u64>,
    pub capabilities: BackendCapabilities,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResolvedPath {
    pub path: String,
    pub metadata: Metadata,
}

impl<D: BlockDevice> Vfs<D> {
    pub fn metadata(&mut self, path: &str) -> Result<Metadata, FsError> {
        let (idx, local) = self.resolve_mount(path)?;
        let mount = self.mounts[idx].as_mut().ok_or(FsError::NotFound)?;
        Ok(Metadata {
            native: mount.fs.stat(local)?,
            mount_id: u64::from(mount.tag) + 1,
            object_id: mount.fs.object_id(local)?,
            capabilities: mount.fs.path_capabilities(local),
        })
    }

    /// No pathname relookup: tmpfs metadata/identity refer to the retained
    /// object even after unlink. FAT explicitly lacks that complete guarantee.
    pub fn metadata_handle(&mut self, handle: FileHandle) -> Result<Metadata, FsError> {
        let (idx, local) = self.unpack_handle(handle)?;
        let mount = self.mounts[idx].as_mut().ok_or(FsError::BadHandle)?;
        Ok(Metadata {
            native: mount.fs.fstat_handle(local)?,
            mount_id: u64::from(mount.tag) + 1,
            object_id: mount.fs.handle_object_id(local)?,
            capabilities: mount.fs.handle_capabilities(local),
        })
    }

    /// Resolve an existing object with search checks on each traversed parent.
    /// Uses the current global namespace and existing uid/gid/root policy.
    /// There are no native symlink objects yet; this API does not advertise
    /// follow/no-follow or directory-fd pinning. No mutation policy is bypassed.
    pub fn resolve_existing(
        &mut self,
        cwd: &str,
        input: &str,
        cred: &Credential,
    ) -> Result<ResolvedPath, FsError> {
        self.resolve_existing_authorized(cwd, input, cred, &mut |_| Ok(()))
    }

    /// Additional kernel capability restrictions are checked before each
    /// component metadata lookup, including components later removed by '..'.
    pub fn resolve_existing_authorized(
        &mut self,
        cwd: &str,
        input: &str,
        cred: &Credential,
        authorize: &mut dyn FnMut(&str) -> Result<(), FsError>,
    ) -> Result<ResolvedPath, FsError> {
        if input.is_empty() || input.len() > RESOLVE_PATH_MAX || input.contains('\0') {
            return Err(FsError::InvalidPath);
        }
        let mut path = String::new();
        path.try_reserve_exact(RESOLVE_PATH_MAX)
            .map_err(|_| FsError::NoMemory)?;
        path.push('/');
        if !input.starts_with('/') {
            if !cwd.starts_with('/') || cwd.len() > RESOLVE_PATH_MAX || cwd.contains('\0') {
                return Err(FsError::InvalidPath);
            }
            self.walk_components(&mut path, cwd, cred, authorize)?;
        }
        self.walk_components(&mut path, input, cred, authorize)?;
        authorize(&path)?;
        let metadata = self.metadata(&path)?;
        Ok(ResolvedPath { path, metadata })
    }

    fn search_directory(&mut self, path: &str, cred: &Credential) -> Result<(), FsError> {
        let stat = self.stat(path)?;
        if stat.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        if !check_permission(&stat, cred, PermCheck::Execute) {
            return Err(FsError::PermissionDenied);
        }
        Ok(())
    }

    fn walk_components(
        &mut self,
        path: &mut String,
        input: &str,
        cred: &Credential,
        authorize: &mut dyn FnMut(&str) -> Result<(), FsError>,
    ) -> Result<(), FsError> {
        authorize(path)?;
        for component in input.split('/').filter(|part| !part.is_empty()) {
            self.search_directory(path, cred)?;
            match component {
                "." => continue,
                ".." => {
                    path.truncate(path.rfind('/').unwrap_or(0).max(1));
                    continue;
                }
                _ => {}
            }
            let parent_len = path.len();
            let required = parent_len + usize::from(parent_len > 1) + component.len();
            if required > RESOLVE_PATH_MAX {
                return Err(FsError::InvalidPath);
            }
            if parent_len > 1 {
                path.push('/');
            }
            let name_start = path.len();
            path.push_str(component);
            let (idx, local) = self.resolve_mount(path)?;
            // Mounted roots use the mount table's spelling. FAT names need
            // backend spelling, not the user's case-insensitive lookup text.
            if local != "/" && matches!(self.mounts[idx].as_ref().unwrap().fs, FsNode::Fat(_)) {
                let local_parent = local
                    .rsplit_once('/')
                    .map(|(p, _)| if p.is_empty() { "/" } else { p })
                    .unwrap_or("/");
                let mut found = None;
                self.mounts[idx]
                    .as_mut()
                    .unwrap()
                    .fs
                    .read_dir(local_parent, &mut |entry| {
                        if entry
                            .name_bytes()
                            .eq_ignore_ascii_case(component.as_bytes())
                        {
                            found = Some(*entry);
                            false
                        } else {
                            true
                        }
                    })?;
                let entry = found.ok_or(FsError::NotFound)?;
                let name =
                    core::str::from_utf8(entry.name_bytes()).map_err(|_| FsError::Unsupported)?;
                path.truncate(name_start);
                path.push_str(name);
            }
            authorize(path)?;
            // Existence is checked before a later '..' can discard the name.
            self.stat(path)?;
        }
        if input.ends_with('/') {
            self.search_directory(path, cred)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ROOT: Credential = Credential { uid: 0, gid: 0 };
    const USER: Credential = Credential {
        uid: 1000,
        gid: 1000,
    };
    fn fixture() -> Vfs {
        static SEED: &[crate::RamEntry] = &[
            crate::RamEntry::dir("/", 0, 0, mode::DIR_755),
            crate::RamEntry::dir("/public", 0, 0, mode::DIR_755),
            crate::RamEntry::dir("/hidden", 0, 0, mode::DIR_700),
            crate::RamEntry::dir("/hidden/mounted", 0, 0, mode::DIR_755),
            crate::RamEntry::file("/hidden/secret", 0, 0, mode::FILE_644, b"secret"),
            crate::RamEntry::file("/public/file", 0, 0, mode::FILE_644, b"data"),
            crate::RamEntry::file("/public/ spaced ", 0, 0, mode::FILE_644, b"space"),
        ];
        let mut vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(SEED)).unwrap();
        vfs.mount_ramfs("/hidden/mounted", RamFs::new(&[])).unwrap();
        vfs
    }
    #[test]
    fn component_walk_checks_objects_before_dotdot() {
        let mut vfs = fixture();
        for name in ["/public/missing/../file", "/public/missing/.."] {
            assert_eq!(
                vfs.resolve_existing("/", name, &ROOT),
                Err(FsError::NotFound)
            );
        }
        for name in ["/public/file/..", "/public/file/.", "/public/file/"] {
            assert_eq!(vfs.resolve_existing("/", name, &ROOT), Err(FsError::NotDir));
        }
        assert_eq!(
            vfs.resolve_existing("/public", "./../public// spaced ", &USER)
                .unwrap()
                .path,
            "/public/ spaced "
        );
        assert_eq!(
            vfs.resolve_existing("bad-cwd", "/public/file", &USER)
                .unwrap()
                .path,
            "/public/file"
        );
        assert_eq!(
            vfs.resolve_existing("/public/file", "child", &ROOT),
            Err(FsError::NotDir)
        );
    }
    #[test]
    fn parent_search_and_capability_policy_cannot_be_erased() {
        let mut vfs = fixture();
        for name in ["/hidden/secret", "/hidden/../public", "/hidden/mounted"] {
            assert_eq!(
                vfs.resolve_existing("/", name, &USER),
                Err(FsError::PermissionDenied)
            );
        }
        assert_eq!(
            vfs.resolve_existing_authorized("/", "/public/../", &ROOT, &mut |name| {
                if name == "/public" {
                    Err(FsError::PermissionDenied)
                } else {
                    Ok(())
                }
            }),
            Err(FsError::PermissionDenied)
        );
        assert_eq!(
            vfs.resolve_existing("/", "/../../public", &USER)
                .unwrap()
                .path,
            "/public"
        );
    }
    #[test]
    fn metadata_is_object_backed_and_mount_qualified() {
        let mut vfs = fixture();
        let h = vfs
            .create_file("/public/live", 1000, 1000, mode::FILE_600)
            .unwrap();
        let before = vfs.metadata_handle(h).unwrap();
        assert_eq!(before, vfs.metadata("/public/live").unwrap());
        assert_eq!(before.capabilities.timestamps, MetadataSource::Unavailable);
        vfs.rename("/public/live", "/public/renamed").unwrap();
        assert_eq!(
            before.object_id,
            vfs.metadata("/public/renamed").unwrap().object_id
        );
        vfs.unlink("/public/renamed").unwrap();
        let replacement = vfs
            .create_file("/public/renamed", 1000, 1000, mode::FILE_600)
            .unwrap();
        assert_ne!(
            before.object_id,
            vfs.metadata_handle(replacement).unwrap().object_id
        );
        assert_eq!(before.object_id, vfs.metadata_handle(h).unwrap().object_id);
        assert_eq!(vfs.metadata_handle(h).unwrap().native.nlinks, 0);
        vfs.close(h).unwrap();
        assert_eq!(vfs.metadata_handle(h), Err(FsError::BadHandle));
        vfs.close(replacement).unwrap();
        let a = vfs.metadata("/").unwrap();
        let b = vfs.metadata("/hidden/mounted").unwrap();
        assert_ne!(a.mount_id, b.mount_id);
        assert_eq!(b.object_id, Some(0));
        assert_eq!(b.capabilities.ownership_permissions, MetadataSource::Policy);
    }
    #[test]
    fn rejects_invalid_and_oversized_paths() {
        let mut vfs = fixture();
        for name in ["", "bad\0path"] {
            assert_eq!(
                vfs.resolve_existing("/", name, &ROOT),
                Err(FsError::InvalidPath)
            );
        }
        let oversized = "a".repeat(RESOLVE_PATH_MAX + 1);
        assert_eq!(
            vfs.resolve_existing("/", &oversized, &ROOT),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            vfs.resolve_existing("relative", "name", &ROOT),
            Err(FsError::InvalidPath)
        );
    }
    #[test]
    fn exhausted_local_generation_never_resurrects_stale_handle() {
        let mut vfs = fixture();
        let stale = vfs.open("/public/file").unwrap();
        vfs.close(stale).unwrap();
        for _ in 0..super::super::HANDLE_GENERATION_MAX {
            let live = vfs.open("/public/file").unwrap();
            assert_eq!(vfs.metadata_handle(stale), Err(FsError::BadHandle));
            vfs.close(live).unwrap();
        }
    }
}
