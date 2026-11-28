//! Retained-anchor metadata lookup. The owner must serialize this operation
//! with namespace mutation (the service holds its mounted-VFS mutex throughout).
use super::*;
use crate::{check_permission, Credential, PermCheck};

/// Stable identity within one live mount incarnation. Tmpfs inode IDs never
/// reuse within that incarnation; the implicit root is encoded as object 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub stat: FileStat,
    pub mount_id: u64,
    pub mount_generation: u64,
    pub object_id: u64,
    pub object_generation: u64,
}

#[derive(Clone, Copy, Default)]
pub struct OpenOptions {
    pub read: bool,
    pub write: bool,
    pub create_mode: Option<u16>,
    pub exclusive: bool,
    pub truncate: bool,
}

const LOOKUP_PATH_MAX: usize = crate::tmpfs::TMPFS_PATH_MAX;

/// Fixed backing matters for services whose allocator cannot reclaim temporary
/// heap allocations. Every append checks capacity before writing.
#[derive(Clone)]
pub(super) struct LookupPath {
    bytes: [u8; LOOKUP_PATH_MAX],
    len: usize,
}
impl LookupPath {
    fn new() -> Self {
        Self {
            bytes: [0; LOOKUP_PATH_MAX],
            len: 0,
        }
    }
    pub(super) fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).expect("only UTF-8 components appended")
    }
    fn append(&mut self, value: &str) -> Result<(), FsError> {
        let end = self
            .len
            .checked_add(value.len())
            .filter(|n| *n <= LOOKUP_PATH_MAX)
            .ok_or(FsError::InvalidPath)?;
        self.bytes[self.len..end].copy_from_slice(value.as_bytes());
        self.len = end;
        Ok(())
    }
    fn pop_parent(&mut self) {
        self.len = self.as_str().rfind('/').unwrap_or(0).max(1);
    }
}

impl<D: BlockDevice> Vfs<D> {
    /// Stat relative to retained root/cwd, never a saved policy pathname. Both
    /// anchor path recovery and inode/mount authentication happen under the same
    /// exclusive VFS borrow as traversal. No syscall or callback may interleave.
    /// Backends without stable object identities fail closed.
    pub fn stat_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
    ) -> Result<ObjectMetadata, FsError> {
        self.resolve_beneath(root, cwd, path, caller)
            .map(|(_, metadata)| metadata)
    }

    /// Create a directory under authenticated anchors. The namespace owner
    /// supplies its write policy for the resolved, mount-visible path.
    pub fn mkdir_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
        mode: u16,
        authorize: impl FnOnce(&Self, &str) -> Result<(), FsError>,
    ) -> Result<(), FsError> {
        let resolved = self.mutation_leaf(root, cwd, path, caller)?;
        authorize(self, resolved.as_str())?;
        self.mkdir(resolved.as_str(), caller.uid, caller.gid, mode & 0o777)
    }

    pub fn unlink_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
        authorize: impl FnOnce(&Self, &str) -> Result<(), FsError>,
    ) -> Result<(), FsError> {
        let resolved = self.mutation_leaf(root, cwd, path, caller)?;
        authorize(self, resolved.as_str())?;
        let metadata = self.lookup_metadata(resolved.as_str())?;
        if metadata.stat.file_type == FileType::Directory {
            return Err(FsError::IsDir);
        }
        if path.ends_with('/') {
            return Err(FsError::NotDir);
        }
        self.unlink(resolved.as_str())
    }

    pub fn rename_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        old: &str,
        new: &str,
        caller: Credential,
        no_replace: bool,
        authorize: impl FnOnce(&Self, &str, &str) -> Result<(), FsError>,
    ) -> Result<(), FsError> {
        self.rename_between(root, (cwd, old), (cwd, new), caller, no_replace, authorize)
    }

    pub fn rename_between(
        &mut self,
        root: FileHandle,
        (source_dir, old): (FileHandle, &str),
        (target_dir, new): (FileHandle, &str),
        caller: Credential,
        no_replace: bool,
        authorize: impl FnOnce(&Self, &str, &str) -> Result<(), FsError>,
    ) -> Result<(), FsError> {
        let source = self.mutation_leaf(root, source_dir, old, caller)?;
        let target = self.mutation_leaf(root, target_dir, new, caller)?;
        let source = source.as_str();
        let target = target.as_str();
        authorize(self, source, target)?;
        let metadata = self.lookup_metadata(source)?;
        if (old.ends_with('/') || new.ends_with('/'))
            && metadata.stat.file_type != FileType::Directory
        {
            return Err(FsError::NotDir);
        }
        if new.ends_with('/') {
            match self.lookup_metadata(target) {
                Ok(entry) if entry.stat.file_type != FileType::Directory => {
                    return Err(FsError::NotDir);
                }
                Ok(_) | Err(FsError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        if no_replace {
            self.rename_no_replace(source, target)
        } else {
            self.rename(source, target)
        }
    }

    fn mutation_leaf(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
    ) -> Result<LookupPath, FsError> {
        if path.is_empty() || path.len() > LOOKUP_PATH_MAX || path.as_bytes().contains(&0) {
            return Err(FsError::InvalidPath);
        }
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            self.resolve_beneath(root, cwd, path, caller)?;
            return Err(FsError::AlreadyExists);
        }
        let (parent, leaf) = match trimmed.rsplit_once('/') {
            Some(("", leaf)) => ("/", leaf),
            Some((parent, leaf)) => (parent, leaf),
            None => (".", trimmed),
        };
        let (mut resolved, metadata) = self.resolve_beneath(root, cwd, parent, caller)?;
        if metadata.stat.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        if !check_permission(&metadata.stat, &caller, PermCheck::Execute) {
            return Err(FsError::PermissionDenied);
        }
        if matches!(leaf, "." | "..") {
            return Err(FsError::AlreadyExists);
        }
        if !check_permission(&metadata.stat, &caller, PermCheck::Write) {
            return Err(FsError::PermissionDenied);
        }
        if resolved.as_str() != "/" {
            resolved.append("/")?;
        }
        resolved.append(leaf)?;
        Ok(resolved)
    }

    /// Keep directory-descriptor traversal within the mounted Downloads tree.
    /// Walking only the parents also permits an absent final creation target.
    pub fn check_downloads_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
    ) -> Result<(), FsError> {
        if self.downloads_authority != super::DownloadsAuthority::Mounted {
            return Err(FsError::Unsupported);
        }
        let relative = !path.starts_with('/');
        let resolved = self.walk_beneath(root, cwd, path, caller, |fs, parent, _| {
            if relative && !crate::path::service_downloads(parent) {
                return Err(FsError::Unsupported);
            }
            fs.lookup_metadata(parent).map(|metadata| metadata.stat)
        })?;
        if !crate::path::service_downloads(resolved.as_str()) {
            return Err(FsError::Unsupported);
        }
        Ok(())
    }


    /// Walk from an authenticated Downloads cwd up to the first legacy component.
    /// The returned path includes the unwalked suffix for the legacy owner.
    pub fn downloads_exit(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
    ) -> Result<Option<alloc::string::String>, FsError> {
        if self.downloads_authority != super::DownloadsAuthority::Mounted
            || self.lookup_anchor(root)?.as_str() != "/"
            || !crate::path::service_downloads(self.lookup_anchor(cwd)?.as_str())
        {
            return Err(FsError::PermissionDenied);
        }
        let absolute = path.starts_with('/');
        if absolute && !crate::path::service_downloads(path) {
            return Ok(Some(copy_boundary_path("", path)?));
        }
        let mut exit = None;
        let mut entered = !absolute;
        let result = self.walk_beneath(root, cwd, path, caller, |fs, parent, suffix| {
            let owned = crate::path::service_downloads(parent);
            if entered && !owned {
                exit = Some(copy_boundary_path(parent, suffix)?);
                return Err(FsError::Unsupported);
            }
            entered |= owned;
            fs.lookup_metadata(parent).map(|metadata| metadata.stat)
        });
        match (result, exit) {
            (_, Some(path)) => Ok(Some(path)),
            (Ok(final_path), None) if !crate::path::service_downloads(final_path.as_str()) => {
                Ok(Some(copy_boundary_path(final_path.as_str(), "")?))
            }
            (Ok(_), None) => Ok(None),
            (Err(error), None) => Err(error),
        }
    }

    /// Verify that traversal ends on the mounted authority and does not leave
    /// Downloads after entering it. Hold the same borrow through pinning.
    pub fn pin_downloads_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
    ) -> Result<(FileHandle, FileHandle), FsError> {
        if self.downloads_authority != super::DownloadsAuthority::Mounted {
            return Err(FsError::Unsupported);
        }
        let mut entered = false;
        let resolved = self.walk_beneath(root, cwd, path, caller, |fs, parent, _| {
            let owned = crate::path::service_downloads(parent);
            if entered && !owned {
                return Err(FsError::Unsupported);
            }
            entered |= owned;
            fs.lookup_metadata(parent).map(|metadata| metadata.stat)
        })?;
        if !crate::path::service_downloads(resolved.as_str()) {
            return Err(FsError::Unsupported);
        }
        let stat = self.lookup_metadata(resolved.as_str())?.stat;
        self.pin_resolved_directory(root, resolved, stat, caller, false)
    }

    /// Open a searchable directory for a read-only directory description.
    pub fn open_directory_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
    ) -> Result<FileHandle, FsError> {
        let (resolved, metadata) = self.resolve_beneath(root, cwd, path, caller)?;
        if metadata.stat.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        if !check_permission(&metadata.stat, &caller, PermCheck::Read) {
            return Err(FsError::PermissionDenied);
        }
        self.open(resolved.as_str())
    }

    /// Open an existing regular file under retained anchors. Search, access
    /// checks and handle acquisition share this exclusive borrow. The returned
    /// handle owns the inode independently of subsequent unlink or rename.
    pub fn open_file_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
        read: bool,
        write: bool,
    ) -> Result<FileHandle, FsError> {
        self.open_file_options_beneath(
            root, cwd, path, caller,
            OpenOptions { read, write, ..OpenOptions::default() },
            |_, _| Ok(()),
        )
    }

    /// Resolve, optionally create, and truncate under the namespace owner's
    /// exclusive borrow. A created name survives later descriptor failures.
    pub fn open_file_options_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
        options: OpenOptions,
        authorize: impl FnOnce(&Self, &str) -> Result<(), FsError>,
    ) -> Result<FileHandle, FsError> {
        if !options.read && !options.write
            || options.exclusive && options.create_mode.is_none()
            || options.truncate && !options.write
        {
            return Err(FsError::InvalidPath);
        }
        let handle = match self.resolve_beneath(root, cwd, path, caller) {
            Ok((resolved, metadata)) => {
                if options.exclusive {
                    return Err(FsError::AlreadyExists);
                }
                if metadata.stat.file_type != FileType::File {
                    return Err(FsError::IsDir);
                }
                if (options.read && !check_permission(&metadata.stat, &caller, PermCheck::Read))
                    || (options.write && !check_permission(&metadata.stat, &caller, PermCheck::Write))
                {
                    return Err(FsError::PermissionDenied);
                }
                if options.truncate {
                    authorize(self, resolved.as_str())?;
                }
                self.open(resolved.as_str())?
            }
            Err(FsError::NotFound) if options.create_mode.is_some() && !path.ends_with('/') => {
                let resolved = self.mutation_leaf(root, cwd, path, caller)?;
                authorize(self, resolved.as_str())?;
                self.create_file_exclusive(
                    resolved.as_str(),
                    caller.uid,
                    caller.gid,
                    options.create_mode.unwrap() & 0o777,
                )?
            }
            Err(error) => return Err(error),
        };
        if options.truncate {
            if let Err(error) = self.truncate(handle) {
                self.close(handle)?;
                return Err(error);
            }
        }
        Ok(handle)
    }

    /// Acquire an independently owned root/cwd pair for a cwd transition.
    /// Traversal, final search permission, and both opens share one exclusive
    /// VFS borrow. Failure closes any provisional handle; old anchors stay live.
    pub fn pin_directory_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
    ) -> Result<(FileHandle, FileHandle), FsError> {
        self.pin_pair_beneath(root, cwd, path, caller, false)
    }

    /// Narrow the retained root to a searched directory and reset cwd to it.
    /// Both returned handles are independently owned; the previous cwd cannot
    /// survive outside the new confinement boundary.
    pub fn pin_root_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
    ) -> Result<(FileHandle, FileHandle), FsError> {
        self.pin_pair_beneath(root, cwd, path, caller, true)
    }

    /// Recover cwd relative to the retained root, authenticating both anchors
    /// under the same borrow as the containment check.
    pub fn cwd_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        output: &mut [u8],
    ) -> Result<usize, FsError> {
        let root = self.lookup_anchor(root)?;
        let cwd = self.lookup_anchor(cwd)?;
        if !beneath(root.as_str(), cwd.as_str()) {
            return Err(FsError::PermissionDenied);
        }
        let relative = if root.as_str() == cwd.as_str() {
            "/"
        } else if root.as_str() == "/" {
            cwd.as_str()
        } else {
            &cwd.as_str()[root.len..]
        };
        if output.len() < relative.len() {
            return Err(FsError::InvalidPath);
        }
        output[..relative.len()].copy_from_slice(relative.as_bytes());
        Ok(relative.len())
    }

    fn pin_pair_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
        replace_root: bool,
    ) -> Result<(FileHandle, FileHandle), FsError> {
        let (resolved, metadata) = self.resolve_beneath(root, cwd, path, caller)?;
        self.pin_resolved_directory(root, resolved, metadata.stat, caller, replace_root)
    }

    fn pin_resolved_directory(
        &mut self,
        root: FileHandle,
        resolved: LookupPath,
        stat: FileStat,
        caller: Credential,
        replace_root: bool,
    ) -> Result<(FileHandle, FileHandle), FsError> {
        if stat.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        if !check_permission(&stat, &caller, PermCheck::Execute) {
            return Err(FsError::PermissionDenied);
        }
        let root_path = if replace_root {
            resolved.clone()
        } else {
            self.lookup_anchor(root)?
        };
        let new_root = self.open(root_path.as_str())?;
        match self.open(resolved.as_str()) {
            Ok(directory) => Ok((new_root, directory)),
            Err(error) => {
                self.close(new_root)
                    .expect("provisional root remains owned");
                Err(error)
            }
        }
    }

    fn resolve_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
    ) -> Result<(LookupPath, ObjectMetadata), FsError> {
        let current = self.walk_beneath(root, cwd, path, caller, |fs, parent, _| {
            fs.lookup_metadata(parent).map(|metadata| metadata.stat)
        })?;
        let result = self.lookup_metadata(current.as_str())?;
        if path.ends_with('/') && result.stat.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        Ok((current, result))
    }

    pub(super) fn walk_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
        mut stat_parent: impl FnMut(&mut Self, &str, &str) -> Result<FileStat, FsError>,
    ) -> Result<LookupPath, FsError> {
        if path.is_empty() || path.len() > LOOKUP_PATH_MAX || path.as_bytes().contains(&0) {
            return Err(FsError::InvalidPath);
        }
        let root_path = self.lookup_anchor(root)?;
        let mut current = if path.starts_with('/') {
            root_path.clone()
        } else {
            self.lookup_anchor(cwd)?
        };
        if !beneath(root_path.as_str(), current.as_str()) {
            return Err(FsError::PermissionDenied);
        }
        for component in path.split('/').filter(|part| !part.is_empty()) {
            let start = component.as_ptr() as usize - path.as_ptr() as usize;
            let parent = stat_parent(self, current.as_str(), &path[start..])?;
            if parent.file_type != FileType::Directory {
                return Err(FsError::NotDir);
            }
            if !check_permission(&parent, &caller, PermCheck::Execute) {
                return Err(FsError::PermissionDenied);
            }
            match component {
                "." => {}
                ".." => {
                    if current.as_str() != root_path.as_str() {
                        current.pop_parent();
                    }
                }
                name => {
                    if current.as_str() != "/" {
                        current.append("/")?;
                    }
                    current.append(name)?;
                    self.canonicalize_fat(&mut current)?;
                }
            }
        }
        Ok(current)
    }

    pub(super) fn lookup_anchor(&mut self, handle: FileHandle) -> Result<LookupPath, FsError> {
        let (mount_idx, local_handle) = self.unpack_handle(handle)?;
        let mount = self.mounts[mount_idx].as_mut().ok_or(FsError::BadHandle)?;
        let fs = &mut mount.fs;
        if fs.fstat_handle(local_handle)?.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        let id = fs
            .handle_object_id(local_handle)?
            .ok_or(FsError::Unsupported)?;
        let mut bytes = [0; LOOKUP_PATH_MAX];
        let len = fs.handle_path(local_handle, &mut bytes)?;
        let local = core::str::from_utf8(&bytes[..len]).map_err(|_| FsError::InvalidPath)?;
        if Self::path_id(fs, local)? != id {
            return Err(FsError::BadHandle);
        }
        let mut absolute = LookupPath::new();
        let prefix = if mount.path == "/" {
            ""
        } else {
            mount.path.as_str()
        };
        let suffix = if local == "/" && !prefix.is_empty() {
            ""
        } else {
            local
        };
        absolute.append(prefix)?;
        absolute.append(suffix)?;
        // A new overmount must not silently replace a retained root or cwd.
        let (visible_mount, visible_local) = self.resolve_mount(absolute.as_str())?;
        if visible_mount != mount_idx
            || Self::path_id(
                &mut self.mounts[visible_mount].as_mut().ok_or(FsError::BadHandle)?.fs,
                visible_local,
            )? != id
        {
            return Err(FsError::BadHandle);
        }
        Ok(absolute)
    }

    fn path_id(fs: &mut FsNode<D>, local: &str) -> Result<u64, FsError> {
        match fs {
            FsNode::Ram(fs) => fs.object_id(local)?.ok_or(FsError::Unsupported),
            FsNode::Fat(fs) if fs.stable_objects => fs.stable_id(local),
            _ => Err(FsError::Unsupported),
        }
    }

    fn canonicalize_fat(&mut self, current: &mut LookupPath) -> Result<(), FsError> {
        let (index, local) = self.resolve_mount(current.as_str())?;
        let mount = self.mounts[index].as_mut().ok_or(FsError::NotFound)?;
        if let FsNode::Fat(fs) = &mut mount.fs {
            if fs.stable_objects && local != "/" {
                let mut bytes = [0; 128];
                let (len, _) = fs.canonical_location(local, &mut bytes)?;
                current.len -= local.len();
                current.append(core::str::from_utf8(&bytes[..len]).map_err(|_| FsError::InvalidPath)?)?;
            }
        }
        Ok(())
    }

    pub(super) fn lookup_metadata(&mut self, path: &str) -> Result<ObjectMetadata, FsError> {
        let (index, local) = self.resolve_mount(path)?;
        let mount = self.mounts[index].as_mut().ok_or(FsError::NotFound)?;
        let id = Self::path_id(&mut mount.fs, local)?;
        let stat = mount.fs.stat(local)?;
        Ok(ObjectMetadata {
            stat,
            mount_id: index as u64 + 1,
            mount_generation: u64::from(mount.tag) / MAX_MOUNTS as u64 + 1,
            object_id: id.checked_add(1).ok_or(FsError::Unsupported)?,
            object_generation: 1,
        })
    }
}

fn beneath(root: &str, path: &str) -> bool {
    root == "/"
        || path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(test)]
#[path = "anchored_tests.rs"]
mod tests;

fn copy_boundary_path(parent: &str, suffix: &str) -> Result<alloc::string::String, FsError> {
    let separator = !parent.is_empty() && parent != "/" && !suffix.is_empty();
    let len = parent.len() + usize::from(separator) + suffix.len();
    if len > LOOKUP_PATH_MAX {
        return Err(FsError::InvalidPath);
    }
    let mut path = alloc::string::String::new();
    path.try_reserve_exact(len).map_err(|_| FsError::NoMemory)?;
    path.push_str(parent);
    if separator {
        path.push('/');
    }
    path.push_str(suffix);
    Ok(path)
}
