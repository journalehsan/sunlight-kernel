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
        authorize: impl FnOnce(&str) -> Result<(), FsError>,
    ) -> Result<(), FsError> {
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
        authorize(resolved.as_str())?;
        self.mkdir(resolved.as_str(), caller.uid, caller.gid, mode & 0o777)
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
        if !read && !write {
            return Err(FsError::InvalidPath);
        }
        let (resolved, metadata) = self.resolve_beneath(root, cwd, path, caller)?;
        if metadata.stat.file_type != FileType::File {
            return Err(FsError::IsDir);
        }
        if (read && !check_permission(&metadata.stat, &caller, PermCheck::Read))
            || (write && !check_permission(&metadata.stat, &caller, PermCheck::Write))
        {
            return Err(FsError::PermissionDenied);
        }
        self.open(resolved.as_str())
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
        if metadata.stat.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        if !check_permission(&metadata.stat, &caller, PermCheck::Execute) {
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
            let parent = self.lookup_metadata(current.as_str())?;
            if parent.stat.file_type != FileType::Directory {
                return Err(FsError::NotDir);
            }
            if !check_permission(&parent.stat, &caller, PermCheck::Execute) {
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
                }
            }
        }
        let result = self.lookup_metadata(current.as_str())?;
        if path.ends_with('/') && result.stat.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        Ok((current, result))
    }

    pub(super) fn lookup_anchor(&mut self, handle: FileHandle) -> Result<LookupPath, FsError> {
        let (mount_idx, local_handle) = self.unpack_handle(handle)?;
        let mount = self.mounts[mount_idx].as_mut().ok_or(FsError::BadHandle)?;
        // FatOpen stores a historical spelling. Never treat it as an anchor.
        let FsNode::Ram(fs) = &mut mount.fs else {
            return Err(FsError::Unsupported);
        };
        if fs.fstat_handle(local_handle)?.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        let id = fs
            .handle_object_id(local_handle)?
            .ok_or(FsError::Unsupported)?;
        let mut bytes = [0; LOOKUP_PATH_MAX];
        let len = fs.handle_path(local_handle, &mut bytes)?;
        let local = core::str::from_utf8(&bytes[..len]).map_err(|_| FsError::InvalidPath)?;
        if fs.object_id(local)? != Some(id) {
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
            || self.mounts[visible_mount]
                .as_ref()
                .ok_or(FsError::BadHandle)?
                .fs
                .object_id(visible_local)?
                != Some(id)
        {
            return Err(FsError::BadHandle);
        }
        Ok(absolute)
    }

    pub(super) fn lookup_metadata(&mut self, path: &str) -> Result<ObjectMetadata, FsError> {
        let (index, local) = self.resolve_mount(path)?;
        let mount = self.mounts[index].as_mut().ok_or(FsError::NotFound)?;
        let (id, stat) = match &mut mount.fs {
            FsNode::Ram(fs) => (fs.object_id(local)?.ok_or(FsError::Unsupported)?, fs.stat(local)?),
            #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
            FsNode::Fat(fs) if mount.device_attachment.is_some() && local == "/" => (0, fs.stat(local)?),
            _ => return Err(FsError::Unsupported),
        };
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
