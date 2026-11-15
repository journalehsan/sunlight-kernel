//! Owning directory references for an isolated tmpfs namespace. No transport or
//! runtime route is enabled here. Paths supplied to `pin` are trusted policy,
//! never decoded object numbers or application cwd strings.
use alloc::sync::Arc;
use spin::Mutex;
use sunlight_fs::{FileHandle, FileStat, FileSystem, FileType, FsError, Tmpfs};

/// Owns the backend independently of a service's mount-table lifetime. Each pin
/// owns generation-tagged backend handles; rename does not change their object.
/// FAT and the boot share page are deliberately unsupported by this adapter.
pub struct DirectoryNamespace {
    filesystem: Arc<Mutex<Tmpfs>>,
}

/// Root and relative starting directory acquired in one serialized operation.
/// Handles and backend access stay private so callers cannot close/reuse them.
/// Store this value in `retention::Table`; Drop closes both handles exactly once.
pub struct DirectoryPair {
    filesystem: Arc<Mutex<Tmpfs>>,
    root: FileHandle,
    directory: FileHandle,
}

impl DirectoryNamespace {
    pub fn new(filesystem: Tmpfs) -> Result<Self, FsError> {
        Ok(Self {
            filesystem: Arc::try_new(Mutex::new(filesystem)).map_err(|_| FsError::NoMemory)?,
        })
    }

    /// Resolve canonical filesystem-local policy paths, rejecting escapes and
    /// non-directories before ownership is returned. This does not grant search
    /// permission, bind a caller/scope, or implement request path traversal.
    pub fn pin(&self, root: &str, directory: &str) -> Result<DirectoryPair, FsError> {
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
        let mut fs = self.filesystem.lock();
        let root = open_directory(&mut fs, root)?;
        let directory = match open_directory(&mut fs, directory) {
            Ok(handle) => handle,
            Err(error) => {
                fs.close(root).expect("private root handle remains owned");
                return Err(error);
            }
        };
        Ok(DirectoryPair {
            filesystem: Arc::clone(&self.filesystem),
            root,
            directory,
        })
    }
}

impl DirectoryPair {
    /// Observe the retained objects under one backend lock, without pathname
    /// relookup. IDs are backend-local diagnostics, never wire lease identities.
    pub fn metadata(&self) -> Result<[(u64, FileStat); 2], FsError> {
        let mut fs = self.filesystem.lock();
        let mut observe = |handle| {
            let id = fs.handle_object_id(handle)?.ok_or(FsError::Unsupported)?;
            Ok((id, fs.fstat_handle(handle)?))
        };
        Ok([observe(self.root)?, observe(self.directory)?])
    }
}

impl Drop for DirectoryPair {
    fn drop(&mut self) {
        let mut fs = self.filesystem.lock();
        fs.close(self.directory)
            .expect("private directory handle remains owned");
        fs.close(self.root)
            .expect("private root handle remains owned");
    }
}

fn open_directory(fs: &mut Tmpfs, path: &str) -> Result<FileHandle, FsError> {
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
