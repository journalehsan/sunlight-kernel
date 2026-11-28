//! Owning directory references in the service mounted namespace. Paths supplied to `pin` are trusted policy,
//! never decoded object numbers or application cwd strings.
use alloc::sync::Arc;
use spin::Mutex;
use sunlight_fs::{BlockDevice, NullDevice};
use sunlight_fs::{FileHandle, FileStat, FileSystem, FileType, FsError, Tmpfs, Vfs};

/// Owns the backend independently of a service's mount-table lifetime. Each pin
/// owns generation-tagged backend handles; rename does not change their object.
/// The separate boot share-page adapter is not part of this mount table.
pub struct DirectoryNamespace<D: BlockDevice = NullDevice> {
    filesystem: Backend<D>,
}

/// Root and relative starting directory acquired in one serialized operation.
/// Handles and backend access stay private so callers cannot close/reuse them.
/// Store this value in `retention::Table`; Drop closes both handles exactly once.
pub struct DirectoryPair<D: BlockDevice = NullDevice> {
    filesystem: Backend<D>,
    root: FileHandle,
    directory: FileHandle,
}

enum Backend<D: BlockDevice> {
    Isolated(Arc<Mutex<Tmpfs>>),
    Mounted(Arc<Mutex<Vfs<D>>>),
}

impl<D: BlockDevice> Clone for Backend<D> {
    fn clone(&self) -> Self {
        match self {
            Self::Isolated(fs) => Self::Isolated(fs.clone()),
            Self::Mounted(fs) => Self::Mounted(fs.clone()),
        }
    }
}

impl DirectoryNamespace {
    /// Isolated fixtures keep their small backend rather than allocating a full
    /// mount table. Production provisioning uses `mounted` and the shared VFS.
    pub fn new(filesystem: Tmpfs) -> Result<Self, FsError> {
        Ok(Self {
            filesystem: Backend::Isolated(
                Arc::try_new(Mutex::new(filesystem)).map_err(|_| FsError::NoMemory)?,
            ),
        })
    }

    pub fn mounted(filesystem: Vfs) -> Result<Self, FsError> {
        Self::mount_owner(filesystem)
    }
}

impl<D: BlockDevice> DirectoryNamespace<D> {
    pub fn mount_owner(filesystem: Vfs<D>) -> Result<Self, FsError> {
        Ok(Self {
            filesystem: Backend::Mounted(
                Arc::try_new(Mutex::new(filesystem)).map_err(|_| FsError::NoMemory)?,
            ),
        })
    }

    pub fn shared(&self) -> Arc<Mutex<Vfs<D>>> {
        match &self.filesystem {
            Backend::Mounted(fs) => Arc::clone(fs),
            Backend::Isolated(_) => panic!("isolated fixture has no mounted VFS"),
        }
    }

    #[cfg(test)]
    fn isolated(&self) -> &Arc<Mutex<Tmpfs>> {
        match &self.filesystem {
            Backend::Isolated(fs) => fs,
            _ => panic!("expected isolated fixture"),
        }
    }

    /// Resolve canonical filesystem-local policy paths, rejecting escapes and
    /// non-directories before ownership is returned. This does not grant search
    /// permission, bind a caller/scope, or implement request path traversal.
    pub fn pin(&self, root: &str, directory: &str) -> Result<DirectoryPair<D>, FsError> {
        canonical(root)?;
        canonical(directory)?;
        if root != "/"
            && directory != root
            && !directory
                .strip_prefix(root)
                .is_some_and(|suffix| suffix.starts_with('/'))
        {
            return Err(FsError::PermissionDenied);
        }
        let (root, directory) = match &self.filesystem {
            Backend::Isolated(fs) => pin_pair(&mut *fs.lock(), root, directory)?,
            Backend::Mounted(fs) => {
                let mut fs = fs.lock();
                let root = open_mounted_directory(&mut fs, root)?;
                match open_mounted_directory(&mut fs, directory) {
                    Ok(directory) => (root, directory),
                    Err(error) => {
                        fs.close(root).expect("private root handle remains owned");
                        return Err(error);
                    }
                }
            }
        };
        Ok(DirectoryPair {
            filesystem: self.filesystem.clone(),
            root,
            directory,
        })
    }
}

impl<D: BlockDevice> DirectoryPair<D> {
    pub fn cwd(&self) -> Result<alloc::string::String, FsError> {
        let mut bytes = [0; 4096];
        let len = match &self.filesystem {
            Backend::Mounted(fs) => fs
                .lock()
                .cwd_beneath(self.root, self.directory, &mut bytes)?,
            Backend::Isolated(_) => return Err(FsError::Unsupported),
        };
        let text = core::str::from_utf8(&bytes[..len]).map_err(|_| FsError::InvalidPath)?;
        let mut cwd = alloc::string::String::new();
        cwd.try_reserve_exact(len).map_err(|_| FsError::NoMemory)?;
        cwd.push_str(text);
        Ok(cwd)
    }

    /// Prepare a replacement from retained objects, with caller search checks.
    /// Existing pairs and leases keep their own handles until explicitly dropped.
    pub fn change_directory(
        &self,
        path: &str,
        caller: sunlight_fs::Credential,
    ) -> Result<Self, FsError> {
        let (root, directory) = match &self.filesystem {
            Backend::Mounted(fs) => {
                fs.lock()
                    .pin_directory_beneath(self.root, self.directory, path, caller)?
            }
            Backend::Isolated(_) => return Err(FsError::Unsupported),
        };
        Ok(Self {
            filesystem: self.filesystem.clone(),
            root,
            directory,
        })
    }

    /// Trusted root-policy narrowing. Reset cwd to the new root so no retained
    /// relative anchor can reach the old namespace outside that root.
    pub fn change_root(
        &self,
        path: &str,
        caller: sunlight_fs::Credential,
    ) -> Result<Self, FsError> {
        let (root, directory) = match &self.filesystem {
            Backend::Mounted(fs) => {
                fs.lock()
                    .pin_root_beneath(self.root, self.directory, path, caller)?
            }
            Backend::Isolated(_) => return Err(FsError::Unsupported),
        };
        Ok(Self {
            filesystem: self.filesystem.clone(),
            root,
            directory,
        })
    }

    /// Traverse under the same mounted namespace lock used by mutation and pin
    /// lifetime. Isolated retention fixtures do not supply a mount identity.
    pub fn stat(
        &self,
        path: &str,
        caller: sunlight_fs::Credential,
    ) -> Result<sunlight_fs::vfs::ObjectMetadata, FsError> {
        match &self.filesystem {
            Backend::Mounted(fs) => fs
                .lock()
                .stat_beneath(self.root, self.directory, path, caller),
            Backend::Isolated(_) => Err(FsError::Unsupported),
        }
    }
}

impl<Device: BlockDevice> DirectoryPair<Device> {
    #[cfg(feature = "vfs-c1e2-test")]
    pub(crate) fn mutate(
        &self,
        request: sunlight_ipc::vfs_rpc::file_transport::Request<'_>,
    ) -> Result<(), FsError> {
        use sunlight_fs::{Credential, FsOperation};
        use sunlight_ipc::vfs_rpc::file_transport as api;
        let Backend::Mounted(fs) = &self.filesystem else {
            return Err(FsError::Unsupported);
        };
        let caller = Credential { uid: request.uid, gid: request.gid };
        let authorize = |fs: &Vfs<Device>, path: &str, op| {
            authorize_write(fs, path, caller, op, request.arg1 == 1)
        };
        let paths = core::str::from_utf8(request.payload).map_err(|_| FsError::InvalidPath)?;
        let mut fs = fs.lock();
        match request.op {
            api::MKDIR => fs.mkdir_beneath(
                self.root, self.directory, paths, caller, request.arg0 as u16,
                |fs, path| authorize(fs, path, FsOperation::Mkdir),
            ),
            api::UNLINK => fs.unlink_beneath(
                self.root, self.directory, paths, caller,
                |fs, path| authorize(fs, path, FsOperation::Delete),
            ),
            api::RENAME => {
                let (old, new) = paths.split_once('\0').ok_or(FsError::InvalidPath)?;
                fs.rename_beneath(
                    self.root, self.directory, old, new, caller, request.arg0 != 0,
                    |fs, old, new| {
                        authorize(fs, old, FsOperation::Delete)?;
                        authorize(fs, new, FsOperation::Create)
                    },
                )
            }
            _ => Err(FsError::Unsupported),
        }
    }

    pub(crate) fn open_file(
        &self,
        path: &str,
        caller: sunlight_fs::Credential,
        options: sunlight_fs::vfs::OpenOptions,
        storage_admin: bool,
    ) -> Result<crate::open::File<Device>, FsError> {
        match &self.filesystem {
            Backend::Mounted(fs) => {
                let handle = fs.lock().open_file_options_beneath(
                    self.root,
                    self.directory,
                    path,
                    caller,
                    options,
                    |fs, path| authorize_write(fs, path, caller, sunlight_fs::FsOperation::Create, storage_admin),
                )?;
                Ok(crate::open::File::new(Arc::clone(fs), handle))
            }
            Backend::Isolated(_) => Err(FsError::Unsupported),
        }
    }
}

fn authorize_write<D: BlockDevice>(
    fs: &Vfs<D>,
    path: &str,
    caller: sunlight_fs::Credential,
    op: sunlight_fs::FsOperation,
    storage_admin: bool,
) -> Result<(), FsError> {
    let decision = sunlight_fs::can_write(
        sunlight_fs::Actor::User { uid: caller.uid, name: "" }, path, op, None, false,
    );
    if decision.allowed || (storage_admin && fs.is_runtime_path(path)) {
        Ok(())
    } else {
        Err(decision.error.unwrap_or(FsError::PermissionDenied))
    }
}

impl<D: BlockDevice> DirectoryPair<D> {
    /// Observe the retained objects under one backend lock, without pathname
    /// relookup. IDs are backend-local diagnostics, never wire lease identities.
    pub fn metadata(&self) -> Result<[(u64, FileStat); 2], FsError> {
        match &self.filesystem {
            Backend::Isolated(fs) => {
                let mut fs = fs.lock();
                let mut observe = |handle| {
                    let id = fs.handle_object_id(handle)?.ok_or(FsError::Unsupported)?;
                    Ok((id, fs.fstat_handle(handle)?))
                };
                Ok([observe(self.root)?, observe(self.directory)?])
            }
            Backend::Mounted(fs) => {
                let mut fs = fs.lock();
                let mut observe = |handle| {
                    let id = fs.handle_object_id(handle)?.ok_or(FsError::Unsupported)?;
                    Ok((id, fs.fstat_handle(handle)?))
                };
                Ok([observe(self.root)?, observe(self.directory)?])
            }
        }
    }
}

impl<D: BlockDevice> Drop for DirectoryPair<D> {
    fn drop(&mut self) {
        match &self.filesystem {
            Backend::Isolated(fs) => {
                let mut fs = fs.lock();
                fs.close(self.directory)
                    .expect("private directory handle remains owned");
                fs.close(self.root)
                    .expect("private root handle remains owned");
            }
            Backend::Mounted(fs) => {
                let mut fs = fs.lock();
                fs.close(self.directory)
                    .expect("private directory handle remains owned");
                fs.close(self.root)
                    .expect("private root handle remains owned");
            }
        }
    }
}

fn pin_pair(
    fs: &mut Tmpfs,
    root: &str,
    directory: &str,
) -> Result<(FileHandle, FileHandle), FsError> {
    let root = open_directory(fs, root)?;
    match open_directory(fs, directory) {
        Ok(directory) => Ok((root, directory)),
        Err(error) => {
            fs.close(root).expect("private root handle remains owned");
            Err(error)
        }
    }
}

fn open_directory(fs: &mut Tmpfs, path: &str) -> Result<FileHandle, FsError> {
    if fs.stat(path)?.file_type != FileType::Directory {
        return Err(FsError::NotDir);
    }
    fs.open(path)
}

fn open_mounted_directory<D: BlockDevice>(
    fs: &mut Vfs<D>,
    path: &str,
) -> Result<FileHandle, FsError> {
    if fs.stat(path)?.file_type != FileType::Directory {
        return Err(FsError::NotDir);
    }
    fs.open(path)
}

fn canonical(path: &str) -> Result<(), FsError> {
    if !path.starts_with('/')
        || path.contains('\0')
        || (path != "/"
            && path
                .split('/')
                .skip(1)
                .any(|part| part.is_empty() || part == "." || part == ".."))
    {
        return Err(FsError::InvalidPath);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
pub mod attachment;
