use crate::{path, FsError, RamFs};
use alloc::{collections::BTreeMap, string::String};
use sunlight_block::{BlockDevice, NullDevice};
use sunlight_fat::{Fat32, FatError, MAX_NAME_83};

#[cfg(test)]
mod storage_tests;

mod lookup;
mod mountinfo;
pub use lookup::{LegacyCwd, BackendCapabilities, Metadata, MetadataSource, ResolvedPath, RESOLVE_PATH_MAX};

pub const MAX_MOUNTS: usize = 8;
/// Maximum file-name length reported by `read_dir`.
pub const VFS_NAME_MAX: usize = 64;
/// Open-file slots per mounted FAT volume.
pub const FAT_MAX_HANDLES: usize = 16;

// Open handles are private backend capabilities, not indexes that callers may
// safely retain after close.  Keep the slot in the low bits and tag each new
// incarnation with a generation. The kernel fd table reserves two high bits
// for handle kind. VFS uses 22 local bits plus 8 mount-incarnation bits,
// leaving 16 local generation bits with six bits for backend slots.
pub const HANDLE_SLOT_BITS: u32 = 6;
const HANDLE_SLOT_MASK: u32 = (1 << HANDLE_SLOT_BITS) - 1;
const HANDLE_GENERATION_MAX: u32 = (1 << (22 - HANDLE_SLOT_BITS)) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileHandle(pub u32);

pub(crate) fn make_local_handle(slot: usize, generation: u32) -> FileHandle {
    debug_assert!(slot < (HANDLE_SLOT_MASK as usize));
    debug_assert!((1..=HANDLE_GENERATION_MAX).contains(&generation));
    FileHandle((generation << HANDLE_SLOT_BITS) | (slot as u32 + 1))
}

pub(crate) fn split_local_handle(handle: FileHandle) -> Result<(usize, u32), FsError> {
    let slot = handle.0 & HANDLE_SLOT_MASK;
    let generation = handle.0 >> HANDLE_SLOT_BITS;
    if slot == 0 || generation == 0 {
        return Err(FsError::BadHandle);
    }
    Ok(((slot - 1) as usize, generation))
}

/// Retire exhausted slots rather than resurrecting an older capability.
pub(crate) fn next_handle_generation(generation: u32) -> Option<u32> {
    generation
        .checked_add(1)
        .filter(|next| *next <= HANDLE_GENERATION_MAX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
}

/// Unix permission-bit constants.
pub mod mode {
    pub const S_IRUSR: u16 = 0o400;
    pub const S_IWUSR: u16 = 0o200;
    pub const S_IXUSR: u16 = 0o100;
    pub const S_IRGRP: u16 = 0o040;
    pub const S_IWGRP: u16 = 0o020;
    pub const S_IXGRP: u16 = 0o010;
    pub const S_IROTH: u16 = 0o004;
    pub const S_IWOTH: u16 = 0o002;
    pub const S_IXOTH: u16 = 0o001;

    pub const S_IFDIR: u16 = 0o040_000;
    pub const S_IFREG: u16 = 0o100_000;

    pub const DIR_755: u16 = S_IFDIR | 0o755;
    pub const FILE_644: u16 = S_IFREG | 0o644;
    pub const FILE_600: u16 = S_IFREG | 0o600;
    pub const FILE_755: u16 = S_IFREG | 0o755;
    pub const FILE_700: u16 = S_IFREG | 0o700;
    pub const DIR_700: u16 = S_IFDIR | 0o700;
    pub const DIR_1777: u16 = S_IFDIR | 0o1777;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub file_type: FileType,
    pub size: usize,
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
    pub nlinks: u32,
}

/// One directory entry reported through `read_dir`. Fixed-size so listing
/// never allocates.
#[derive(Clone, Copy, Debug)]
pub struct VfsDirEntry {
    pub name: [u8; VFS_NAME_MAX],
    pub name_len: u8,
    pub file_type: FileType,
    pub size: usize,
}

impl VfsDirEntry {
    pub fn from_bytes(name: &[u8], file_type: FileType, size: usize) -> Self {
        let len = name.len().min(VFS_NAME_MAX);
        let mut buf = [0u8; VFS_NAME_MAX];
        buf[..len].copy_from_slice(&name[..len]);
        Self {
            name: buf,
            name_len: len as u8,
            file_type,
            size,
        }
    }

    pub fn name_bytes(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }

    pub fn name(&self) -> &str {
        core::str::from_utf8(self.name_bytes()).unwrap_or("?")
    }
}

pub trait FileSystem {
    /// Provenance of the native subset, independent of POSIX compatibility.
    fn capabilities(&self) -> BackendCapabilities;
    fn path_capabilities(&self, _path: &str) -> BackendCapabilities {
        self.capabilities()
    }
    fn handle_capabilities(&self, _handle: FileHandle) -> BackendCapabilities {
        self.capabilities()
    }
    fn object_id(&self, _path: &str) -> Result<Option<u64>, FsError> {
        Ok(None)
    }
    fn handle_object_id(&self, _handle: FileHandle) -> Result<Option<u64>, FsError> {
        Ok(None)
    }
    fn open(&mut self, path: &str) -> Result<FileHandle, FsError>;
    fn create_file(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<FileHandle, FsError>;
    /// Create a new regular file only when no directory entry already exists.
    ///
    /// This is distinct from `create_file`, whose historical behaviour opens
    /// an existing file.  Security-sensitive callers need the failure to be
    /// decided while the filesystem is locked, not after a path-level check.
    fn create_file_exclusive(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<FileHandle, FsError>;
    fn read(&mut self, handle: FileHandle, offset: usize, buf: &mut [u8])
        -> Result<usize, FsError>;
    fn write(&mut self, handle: FileHandle, offset: usize, buf: &[u8]) -> Result<usize, FsError>;
    /// Reserve backing storage for an expected final file size without
    /// changing the visible length. Filesystems that do not keep file data in
    /// the kernel heap may treat this as an advisory no-op.
    fn reserve(&mut self, _handle: FileHandle, _size: usize) -> Result<(), FsError> {
        Ok(())
    }
    /// Reduce an open regular file to length zero.  This is intentionally a
    /// narrow primitive used by `O_TRUNC`; it does not add a public ftruncate
    /// ABI.
    fn truncate(&mut self, handle: FileHandle) -> Result<(), FsError>;
    fn close(&mut self, handle: FileHandle) -> Result<(), FsError>;
    /// Force file content and its metadata to stable storage.
    fn sync_file(&mut self, handle: FileHandle) -> Result<(), FsError>;
    /// Force directory entries and allocation metadata reachable at `path` to
    /// stable storage.
    fn sync_dir(&mut self, path: &str) -> Result<(), FsError>;
    /// Return metadata for an open handle without a path round-trip.
    /// Used by `sys_fstat` and `sys_lseek(SEEK_END)`.
    fn fstat_handle(&mut self, handle: FileHandle) -> Result<FileStat, FsError>;
    fn stat(&mut self, path: &str) -> Result<FileStat, FsError>;
    fn mkdir(&mut self, path: &str, uid: u32, gid: u32, mode: u16) -> Result<(), FsError>;
    fn chmod(&mut self, path: &str, mode: u16) -> Result<(), FsError>;
    fn chown(&mut self, path: &str, uid: u32, gid: u32) -> Result<(), FsError>;
    fn unlink(&mut self, path: &str) -> Result<(), FsError>;
    fn rename(&mut self, old: &str, new: &str) -> Result<(), FsError>;
    /// Call `f` once per entry in the directory at `path`; `f` returns false
    /// to stop early. Non-allocating: entries are built on the stack.
    fn read_dir(
        &mut self,
        path: &str,
        f: &mut dyn FnMut(&VfsDirEntry) -> bool,
    ) -> Result<(), FsError>;
    /// Copy the filesystem-local path of an open handle into `buf`.
    /// Returns the number of bytes written (not NUL-terminated).
    fn handle_path(&self, handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError>;
}

// ---------------------------------------------------------------------------
// FAT32 adapter
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct FatOpen {
    entry_location: (u64, u16),
    object_id: Option<u64>,
    revoked: bool,
    first_cluster: u32,
    size: u32,
    generation: u32,
    is_dir: bool,
    path_len: u8,
    path: [u8; 128],
}

/// Exclusive mutable FAT adapter. Handles authenticate an entry location and
/// are revoked before namespace retirement. Unlinked access returns BadHandle;
/// this is deliberately weaker than POSIX retained-open-file semantics.
/// An uncertain mutation failure poisons the adapter rather than allowing a
/// possibly aliased cluster chain to authorize later I/O. Close still succeeds.
pub struct FatFs<D: BlockDevice> {
    fat: Fat32<D>,
    user_storage: bool,
    poisoned: bool,
    handles: [Option<FatOpen>; FAT_MAX_HANDLES],
    generations: [u32; FAT_MAX_HANDLES],
    stable_objects: bool,
    identities: BTreeMap<(u64, u16), u64>,
    next_identity: u64,
}

impl<D: BlockDevice> FatFs<D> {
    pub fn new_attachment(fat: Fat32<D>) -> Self {
        let mut backend = Self::new(fat);
        backend.stable_objects = true;
        backend
    }

    pub fn new(fat: Fat32<D>) -> Self {
        Self {
            fat,
            user_storage: false,
            poisoned: false,
            handles: [None; FAT_MAX_HANDLES],
            generations: [0; FAT_MAX_HANDLES],
            stable_objects: false,
            identities: BTreeMap::new(),
            next_identity: 1,
        }
    }

    fn intern(&mut self, location: (u64, u16)) -> Result<u64, FsError> {
        if location == (0, 0) {
            return Ok(0);
        }
        if let Some(&id) = self.identities.get(&location) {
            return Ok(id);
        }
        let id = self.next_identity;
        self.next_identity = id.checked_add(1).ok_or(FsError::Busy)?;
        self.identities.insert(location, id);
        Ok(id)
    }

    fn canonical_location(&mut self, path: &str, output: &mut [u8]) -> Result<(usize, (u64, u16)), FsError> {
        self.ensure_healthy()?;
        path::validate_absolute(path)?;
        self.fat.locate_path(path.as_bytes(), output).map_err(Self::map_error)
    }

    fn stable_id(&mut self, path: &str) -> Result<u64, FsError> {
        let (_, location) = self.canonical_location(path, &mut [0; 128])?;
        self.intern(location)
    }

    fn pinned(&self, location: (u64, u16)) -> bool {
        self.handles.iter().flatten().any(|open| open.entry_location == location)
    }

    fn rename_stable(&mut self, old: &str, new: &str) -> Result<(), FsError> {
        if old == "/" || new == "/" || new.len() > 128 {
            return Err(FsError::InvalidPath);
        }
        let mut old_path = [0; 128];
        let (old_len, source) = self.canonical_location(old, &mut old_path)?;
        let destination = match self.canonical_location(new, &mut [0; 128]) {
            Ok((_, location)) if location == source => return Ok(()),
            Ok((_, location)) if self.pinned(location) => return Err(FsError::Busy),
            Ok((_, location)) => Some(location),
            Err(FsError::NotFound) => None,
            Err(error) => return Err(error),
        };
        let directory = self.stat(old)?.file_type == FileType::Directory;
        let mut new_parent = [0; 128];
        let (parent_len, _) = self.canonical_location(parent_path(new)?, &mut new_parent)?;
        let canonical_old = core::str::from_utf8(&old_path[..old_len]).map_err(|_| FsError::InvalidPath)?;
        if directory {
            if destination.is_some() || parent_path(canonical_old)?.as_bytes() != &new_parent[..parent_len] {
                return Err(FsError::Unsupported);
            }
            for open in self.handles.iter().flatten() {
                let len = open.path_len as usize;
                if open.path[..len].starts_with(&old_path[..old_len])
                    && open.path.get(old_len) == Some(&b'/')
                    && len - old_len + new.len() - parent_path(new)?.len() + parent_len > open.path.len()
                {
                    return Err(FsError::InvalidPath);
                }
            }
        } else if destination.is_some() && self.stat(new)?.file_type != FileType::File {
            return Err(FsError::IsDir);
        }
        let result = self.fat.rename_path(old.as_bytes(), new.as_bytes());
        self.mutation(result)?;
        let mut new_path = [0; 128];
        let (new_len, location) = self.canonical_location(new, &mut new_path).inspect_err(|_| self.poisoned = true)?;
        if let Some(destination) = destination {
            self.identities.remove(&destination);
        }
        if let Some(id) = self.identities.remove(&source) {
            self.identities.insert(location, id);
        }
        for open in self.handles.iter_mut().flatten() {
            if open.entry_location == source {
                open.entry_location = location;
                open.path = new_path;
                open.path_len = new_len as u8;
            } else if directory {
                let len = open.path_len as usize;
                if open.path[..len].starts_with(&old_path[..old_len])
                    && open.path.get(old_len) == Some(&b'/')
                {
                    open.path.copy_within(old_len..len, new_len);
                    open.path[..new_len].copy_from_slice(&new_path[..new_len]);
                    open.path_len = (len - old_len + new_len) as u8;
                }
            }
        }
        Ok(())
    }

    fn handle_slot(&self, handle: FileHandle) -> Result<FatOpen, FsError> {
        let (idx, generation) = split_local_handle(handle)?;
        self.handles
            .get(idx)
            .and_then(|slot| {
                slot.filter(|open| open.generation == generation && !open.revoked && !self.poisoned)
            })
            .ok_or(FsError::BadHandle)
    }

    fn ensure_healthy(&self) -> Result<(), FsError> {
        if self.poisoned {
            Err(FsError::Io)
        } else {
            Ok(())
        }
    }

    fn revoke_entry(&mut self, location: (u64, u16)) {
        for open in self.handles.iter_mut().flatten() {
            if open.entry_location == location {
                open.revoked = true;
            }
        }
    }

    fn mutation<T>(&mut self, result: Result<T, FatError>) -> Result<T, FsError> {
        result.map_err(|error| {
            // Raw FAT mutations are not transactional. Even an error may have
            // published an alias or released part of a chain. Never retry I/O
            // through that uncertain state within this mount incarnation.
            self.poisoned = true;
            Self::map_error(error)
        })
    }

    fn authenticated_open(&mut self, handle: FileHandle) -> Result<FatOpen, FsError> {
        let mut open = self.handle_slot(handle)?;
        let path = &open.path[..open.path_len as usize];
        let location = self.fat.entry_location(path).map_err(Self::map_error)?;
        if location != open.entry_location {
            return Err(FsError::BadHandle);
        }
        if let Some(id) = open.object_id {
            if location != (0, 0) && self.identities.get(&location) != Some(&id) {
                return Err(FsError::BadHandle);
            }
        }
        let stat = self.fat.stat_path(path).ok_or(FsError::Io)?;
        open.first_cluster = stat.first_cluster;
        open.size = stat.size;
        Ok(open)
    }

    fn map_error(error: FatError) -> FsError {
        match error {
            FatError::NotFound => FsError::NotFound,
            FatError::AlreadyExists => FsError::AlreadyExists,
            FatError::NotDirectory => FsError::NotDir,
            FatError::IsDirectory => FsError::IsDir,
            FatError::InvalidName => FsError::InvalidPath,
            FatError::NoSpace | FatError::Io => FsError::Io,
            FatError::NotEmpty => FsError::OperationNotPermitted,
        }
    }

    fn user_metadata(&self, path: &str, mut stat: FileStat) -> FileStat {
        if self.user_storage {
            if let Some(owner) = path
                .strip_prefix("/users/")
                .and_then(|p| p.split('/').next())
            {
                if !owner.is_empty()
                    && (owner == "0" || !owner.starts_with('0'))
                    && owner.bytes().all(|b| b.is_ascii_digit())
                {
                    if let Ok(uid) = owner.parse::<u32>() {
                        stat.uid = uid;
                        stat.gid = uid;
                        stat.mode = if stat.file_type == FileType::Directory {
                            0o700
                        } else {
                            0o600
                        };
                    }
                }
            }
        }
        stat
    }
}

impl<D: BlockDevice> FileSystem for FatFs<D> {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::FAT
    }

    fn handle_object_id(&self, handle: FileHandle) -> Result<Option<u64>, FsError> {
        Ok(self.handle_slot(handle)?.object_id)
    }

    fn open(&mut self, path: &str) -> Result<FileHandle, FsError> {
        self.ensure_healthy()?;
        path::validate_absolute(path)?;
        let stat = self
            .fat
            .stat_path(path.as_bytes())
            .ok_or(FsError::NotFound)?;
        let path_bytes = path.as_bytes();
        if path_bytes.len() > 128 {
            return Err(FsError::InvalidPath);
        }
        let mut stored = [0; 128];
        let (len, entry_location, object_id) = if self.stable_objects {
            let (len, location) = self.canonical_location(path, &mut stored)?;
            (len, location, Some(self.intern(location)?))
        } else {
            stored[..path_bytes.len()].copy_from_slice(path_bytes);
            (path_bytes.len(), self.fat.entry_location(path_bytes).map_err(Self::map_error)?, None)
        };
        for (idx, slot) in self.handles.iter_mut().enumerate() {
            if slot.is_none() {
                let Some(generation) = next_handle_generation(self.generations[idx]) else {
                    continue;
                };
                self.generations[idx] = generation;
                *slot = Some(FatOpen {
                    entry_location,
                    object_id,
                    revoked: false,
                    first_cluster: stat.first_cluster,
                    size: stat.size,
                    generation,
                    is_dir: stat.is_dir,
                    path_len: len as u8,
                    path: stored,
                });
                return Ok(make_local_handle(idx, generation));
            }
        }
        Err(FsError::TooManyOpenFiles)
    }

    fn create_file(
        &mut self,
        path: &str,
        _uid: u32,
        _gid: u32,
        _mode: u16,
    ) -> Result<FileHandle, FsError> {
        match self.open(path) {
            Ok(handle) => Ok(handle),
            Err(FsError::NotFound) => {
                let result = self.fat.create_file_path(path.as_bytes());
                self.mutation(result)?;
                self.open(path)
            }
            Err(error) => Err(error),
        }
    }

    fn create_file_exclusive(
        &mut self,
        path: &str,
        _uid: u32,
        _gid: u32,
        _mode: u16,
    ) -> Result<FileHandle, FsError> {
        self.ensure_healthy()?;
        path::validate_absolute(path)?;
        if self.fat.stat_path(path.as_bytes()).is_some() {
            return Err(FsError::AlreadyExists);
        }
        let result = self.fat.create_file_path(path.as_bytes());
        self.mutation(result)?;
        self.open(path)
    }

    fn read(
        &mut self,
        handle: FileHandle,
        offset: usize,
        buf: &mut [u8],
    ) -> Result<usize, FsError> {
        let open = self.authenticated_open(handle)?;
        if open.is_dir {
            return Err(FsError::IsDir);
        }
        self.fat
            .read_at(open.first_cluster, open.size, offset, buf)
            .ok_or(FsError::Io)
    }

    fn write(&mut self, handle: FileHandle, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        let open = self.authenticated_open(handle)?;
        if open.is_dir {
            return Err(FsError::IsDir);
        }
        let path = &open.path[..open.path_len as usize];
        let result = self.fat.write_path(path, offset, buf);
        let written = self.mutation(result)?;
        let stat = self.fat.stat_path(path).ok_or(FsError::Io)?;
        let (index, generation) = split_local_handle(handle)?;
        let slot = self.handles.get_mut(index).ok_or(FsError::BadHandle)?;
        let current = slot.as_mut().ok_or(FsError::BadHandle)?;
        if current.generation != generation {
            return Err(FsError::BadHandle);
        }
        current.first_cluster = stat.first_cluster;
        current.size = stat.size;
        Ok(written)
    }

    fn truncate(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let open = self.authenticated_open(handle)?;
        if open.is_dir {
            return Err(FsError::IsDir);
        }
        let path = &open.path[..open.path_len as usize];
        let result = self.fat.truncate_path(path);
        self.mutation(result)?;
        let (index, generation) = split_local_handle(handle)?;
        let current = self.handles[index].as_mut().ok_or(FsError::BadHandle)?;
        if current.generation != generation {
            return Err(FsError::BadHandle);
        }
        current.first_cluster = 0;
        current.size = 0;
        Ok(())
    }

    fn close(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let (idx, generation) = split_local_handle(handle)?;
        let slot = self.handles.get_mut(idx).ok_or(FsError::BadHandle)?;
        if slot
            .as_ref()
            .map(|open| open.generation != generation)
            .unwrap_or(true)
        {
            return Err(FsError::BadHandle);
        }
        *slot = None;
        Ok(())
    }

    fn sync_file(&mut self, handle: FileHandle) -> Result<(), FsError> {
        self.handle_slot(handle)?;
        self.fat.sync().map_err(|error| match error {
            sunlight_block::BlockError::Unsupported => FsError::Unsupported,
            _ => FsError::Io,
        })
    }

    fn sync_dir(&mut self, path: &str) -> Result<(), FsError> {
        self.ensure_healthy()?;
        let stat = self
            .fat
            .stat_path(path.as_bytes())
            .ok_or(FsError::NotFound)?;
        if !stat.is_dir {
            return Err(FsError::NotDir);
        }
        self.fat.sync().map_err(|error| match error {
            sunlight_block::BlockError::Unsupported => FsError::Unsupported,
            _ => FsError::Io,
        })
    }

    fn fstat_handle(&mut self, handle: FileHandle) -> Result<FileStat, FsError> {
        let open = self.authenticated_open(handle)?;
        let path = core::str::from_utf8(&open.path[..open.path_len as usize])
            .map_err(|_| FsError::InvalidPath)?;
        Ok(self.user_metadata(
            path,
            if open.is_dir {
                FileStat {
                    file_type: FileType::Directory,
                    size: 0,
                    uid: 0,
                    gid: 0,
                    mode: mode::DIR_755,
                    nlinks: 2,
                }
            } else {
                FileStat {
                    file_type: FileType::File,
                    size: open.size as usize,
                    uid: 0,
                    gid: 0,
                    mode: mode::FILE_755,
                    nlinks: 1,
                }
            },
        ))
    }

    fn stat(&mut self, path: &str) -> Result<FileStat, FsError> {
        self.ensure_healthy()?;
        path::validate_absolute(path)?;
        let stat = self
            .fat
            .stat_path(path.as_bytes())
            .ok_or(FsError::NotFound)?;
        Ok(self.user_metadata(
            path,
            if stat.is_dir {
                FileStat {
                    file_type: FileType::Directory,
                    size: 0,
                    uid: 0,
                    gid: 0,
                    mode: mode::DIR_755,
                    nlinks: 2,
                }
            } else {
                FileStat {
                    file_type: FileType::File,
                    size: stat.size as usize,
                    uid: 0,
                    gid: 0,
                    mode: mode::FILE_755,
                    nlinks: 1,
                }
            },
        ))
    }

    fn mkdir(&mut self, path: &str, _uid: u32, _gid: u32, _mode: u16) -> Result<(), FsError> {
        self.ensure_healthy()?;
        path::validate_absolute(path)?;
        if self.fat.stat_path(path.as_bytes()).is_some() {
            return Err(FsError::AlreadyExists);
        }
        let result = self.fat.mkdir_path(path.as_bytes());
        self.mutation(result)
    }

    fn chmod(&mut self, _path: &str, _mode: u16) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    fn chown(&mut self, _path: &str, _uid: u32, _gid: u32) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    fn unlink(&mut self, path: &str) -> Result<(), FsError> {
        self.ensure_healthy()?;
        path::validate_absolute(path)?;
        if self.stable_objects {
            let (_, location) = self.canonical_location(path, &mut [0; 128])?;
            if self.pinned(location) {
                return Err(FsError::Busy);
            }
            if self.stat(path)?.file_type == FileType::Directory {
                return Err(FsError::IsDir);
            }
            let result = self.fat.unlink_path(path.as_bytes());
            self.mutation(result)?;
            self.identities.remove(&location);
            return Ok(());
        }
        let location = self
            .fat
            .entry_location(path.as_bytes())
            .map_err(Self::map_error)?;
        // Revoke every alias before the first backend mutation, including when
        // unlink subsequently fails. Slots remain owned until valid close.
        self.revoke_entry(location);
        let result = self.fat.unlink_path(path.as_bytes());
        self.mutation(result)
    }

    fn rename(&mut self, old: &str, new: &str) -> Result<(), FsError> {
        self.ensure_healthy()?;
        path::validate_absolute(old)?;
        path::validate_absolute(new)?;
        if self.stable_objects {
            return self.rename_stable(old, new);
        }
        if new.len() > 128 {
            return Err(FsError::InvalidPath);
        }
        let source = self
            .fat
            .entry_location(old.as_bytes())
            .map_err(Self::map_error)?;
        if let Ok(destination) = self.fat.entry_location(new.as_bytes()) {
            // Raw rename-to-self deletes its own entry; handle all aliases here.
            if source == destination {
                return Ok(());
            }
            self.revoke_entry(destination);
        }
        let is_dir = self
            .fat
            .stat_path(old.as_bytes())
            .ok_or(FsError::NotFound)?
            .is_dir;
        if is_dir {
            // No retained directory-relative resolver exists yet. Do not leave
            // textual descendant handles available for future namespace reuse.
            for open in self.handles.iter_mut().flatten() {
                if open.entry_location != source {
                    open.revoked = true;
                }
            }
        }
        let result = self.fat.rename_path(old.as_bytes(), new.as_bytes());
        self.mutation(result)?;
        let location = match self.fat.entry_location(new.as_bytes()) {
            Ok(location) => location,
            Err(error) => {
                self.poisoned = true;
                return Err(Self::map_error(error));
            }
        };
        for open in self.handles.iter_mut().flatten() {
            if !open.revoked && open.entry_location == source {
                open.entry_location = location;
                open.path.fill(0);
                open.path[..new.len()].copy_from_slice(new.as_bytes());
                open.path_len = new.len() as u8;
            }
        }
        Ok(())
    }

    fn read_dir(
        &mut self,
        path: &str,
        f: &mut dyn FnMut(&VfsDirEntry) -> bool,
    ) -> Result<(), FsError> {
        self.ensure_healthy()?;
        path::validate_absolute(path)?;
        let stat = self
            .fat
            .stat_path(path.as_bytes())
            .ok_or(FsError::NotFound)?;
        if !stat.is_dir {
            return Err(FsError::NotDir);
        }
        debug_assert!(MAX_NAME_83 <= VFS_NAME_MAX);
        self.fat
            .read_dir_raw(path.as_bytes(), &mut |name, is_dir, size| {
                let file_type = if is_dir {
                    FileType::Directory
                } else {
                    FileType::File
                };
                f(&VfsDirEntry::from_bytes(name, file_type, size as usize))
            })
            .ok_or(FsError::Io)?;
        Ok(())
    }

    fn handle_path(&self, handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError> {
        let open = self.handle_slot(handle)?;
        let len = open.path_len as usize;
        if buf.len() < len {
            return Err(FsError::InvalidPath);
        }
        buf[..len].copy_from_slice(&open.path[..len]);
        Ok(len)
    }
}

// ---------------------------------------------------------------------------
// Mount table
// ---------------------------------------------------------------------------

/// A concrete filesystem behind a mount point. Enum dispatch keeps the VFS
/// free of trait objects (no vtables, works without alloc).
pub enum FsNode<D: BlockDevice = NullDevice> {
    Ram(RamFs),
    Fat(FatFs<D>),
}

impl<D: BlockDevice> FileSystem for FsNode<D> {
    fn capabilities(&self) -> BackendCapabilities {
        match self {
            Self::Ram(fs) => fs.capabilities(),
            Self::Fat(fs) => fs.capabilities(),
        }
    }
    fn path_capabilities(&self, path: &str) -> BackendCapabilities {
        match self {
            Self::Ram(fs) => fs.path_capabilities(path),
            Self::Fat(fs) => fs.path_capabilities(path),
        }
    }
    fn handle_capabilities(&self, handle: FileHandle) -> BackendCapabilities {
        match self {
            Self::Ram(fs) => fs.handle_capabilities(handle),
            Self::Fat(fs) => fs.handle_capabilities(handle),
        }
    }
    fn object_id(&self, path: &str) -> Result<Option<u64>, FsError> {
        match self {
            Self::Ram(fs) => fs.object_id(path),
            Self::Fat(fs) => fs.object_id(path),
        }
    }
    fn handle_object_id(&self, handle: FileHandle) -> Result<Option<u64>, FsError> {
        match self {
            Self::Ram(fs) => fs.handle_object_id(handle),
            Self::Fat(fs) => fs.handle_object_id(handle),
        }
    }

    fn open(&mut self, path: &str) -> Result<FileHandle, FsError> {
        match self {
            Self::Ram(fs) => fs.open(path),
            Self::Fat(fs) => fs.open(path),
        }
    }

    fn create_file(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<FileHandle, FsError> {
        match self {
            Self::Ram(fs) => fs.create_file(path, uid, gid, mode),
            Self::Fat(fs) => fs.create_file(path, uid, gid, mode),
        }
    }

    fn create_file_exclusive(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<FileHandle, FsError> {
        match self {
            Self::Ram(fs) => fs.create_file_exclusive(path, uid, gid, mode),
            Self::Fat(fs) => fs.create_file_exclusive(path, uid, gid, mode),
        }
    }

    fn read(
        &mut self,
        handle: FileHandle,
        offset: usize,
        buf: &mut [u8],
    ) -> Result<usize, FsError> {
        match self {
            Self::Ram(fs) => fs.read(handle, offset, buf),
            Self::Fat(fs) => fs.read(handle, offset, buf),
        }
    }

    fn write(&mut self, handle: FileHandle, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        match self {
            Self::Ram(fs) => fs.write(handle, offset, buf),
            Self::Fat(fs) => fs.write(handle, offset, buf),
        }
    }

    fn reserve(&mut self, handle: FileHandle, size: usize) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.reserve(handle, size),
            Self::Fat(fs) => fs.reserve(handle, size),
        }
    }

    fn truncate(&mut self, handle: FileHandle) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.truncate(handle),
            Self::Fat(fs) => fs.truncate(handle),
        }
    }

    fn close(&mut self, handle: FileHandle) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.close(handle),
            Self::Fat(fs) => fs.close(handle),
        }
    }

    fn sync_file(&mut self, handle: FileHandle) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.sync_file(handle),
            Self::Fat(fs) => fs.sync_file(handle),
        }
    }

    fn sync_dir(&mut self, path: &str) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.sync_dir(path),
            Self::Fat(fs) => fs.sync_dir(path),
        }
    }

    fn fstat_handle(&mut self, handle: FileHandle) -> Result<FileStat, FsError> {
        match self {
            Self::Ram(fs) => fs.fstat_handle(handle),
            Self::Fat(fs) => fs.fstat_handle(handle),
        }
    }

    fn stat(&mut self, path: &str) -> Result<FileStat, FsError> {
        match self {
            Self::Ram(fs) => fs.stat(path),
            Self::Fat(fs) => fs.stat(path),
        }
    }

    fn mkdir(&mut self, path: &str, uid: u32, gid: u32, mode: u16) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.mkdir(path, uid, gid, mode),
            Self::Fat(fs) => fs.mkdir(path, uid, gid, mode),
        }
    }

    fn chmod(&mut self, path: &str, mode: u16) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.chmod(path, mode),
            Self::Fat(fs) => fs.chmod(path, mode),
        }
    }

    fn chown(&mut self, path: &str, uid: u32, gid: u32) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.chown(path, uid, gid),
            Self::Fat(fs) => fs.chown(path, uid, gid),
        }
    }

    fn unlink(&mut self, path: &str) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.unlink(path),
            Self::Fat(fs) => fs.unlink(path),
        }
    }

    fn rename(&mut self, old: &str, new: &str) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.rename(old, new),
            Self::Fat(fs) => fs.rename(old, new),
        }
    }

    fn read_dir(
        &mut self,
        path: &str,
        f: &mut dyn FnMut(&VfsDirEntry) -> bool,
    ) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.read_dir(path, f),
            Self::Fat(fs) => fs.read_dir(path, f),
        }
    }

    fn handle_path(&self, handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError> {
        match self {
            Self::Ram(fs) => fs.handle_path(handle, buf),
            Self::Fat(fs) => fs.handle_path(handle, buf),
        }
    }
}

pub struct Mount<D: BlockDevice = NullDevice> {
    path: String,
    fs: FsNode<D>,
    tag: u8,
    source: u64,
    protected: bool,
    open_handles: usize,
    #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
    device_attachment: Option<attachment::DeviceState>,
}

pub struct MountInfo<'a> {
    pub id: u64,
    pub source: u64,
    pub protected: bool,
    pub path: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DownloadsAuthority {
    Legacy,
    Reserved,
    Mounted,
}

pub struct Vfs<D: BlockDevice = NullDevice> {
    mounts: [Option<Mount<D>>; MAX_MOUNTS],
    count: usize,
    downloads_authority: DownloadsAuthority,
    #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
    topology_revision: u64,
    // Upper handle byte: 3 slot bits and 5 incarnation bits. Retire a slot
    // instead of wrapping; closed handles must never alias a remounted volume.
    next_tags: [u16; MAX_MOUNTS],
}

impl<D: BlockDevice> Vfs<D> {
    pub const fn new() -> Self {
        Self {
            mounts: [const { None }; MAX_MOUNTS],
            count: 0,
            downloads_authority: DownloadsAuthority::Legacy,
            #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
            topology_revision: 0,
            next_tags: [0, 1, 2, 3, 4, 5, 6, 7],
        }
    }

    /// Fence the empty boot mountpoint before any legacy backend is installed.
    pub fn reserve_service_downloads(&mut self, seed: &[crate::RamEntry]) -> Result<(), FsError> {
        if self.count != 0 {
            return Err(FsError::Busy);
        }
        let mut entries = seed.iter().filter(|entry| path::service_downloads(entry.path));
        if !entries.next().is_some_and(|entry| {
            entry.path == path::SERVICE_DOWNLOADS && entry.is_dir
                && entry.uid == 0 && entry.gid == 0 && entry.mode == mode::DIR_700
        }) || entries.next().is_some() {
            return Err(FsError::InvalidPath);
        }
        self.downloads_authority = DownloadsAuthority::Reserved;
        Ok(())
    }

    fn check_service_reservation(&self, path: &str) -> Result<(), FsError> {
        if self.downloads_authority == DownloadsAuthority::Reserved && path::service_downloads(path) {
            return Err(FsError::Unsupported);
        }
        Ok(())
    }

    fn check_downloads_mountpoint(&self, path: &str) -> Result<(), FsError> {
        let target = crate::path::SERVICE_DOWNLOADS;
        if self.downloads_authority != DownloadsAuthority::Legacy
            && path::strip_mount(target, path).is_some()
        {
            return Err(FsError::Busy);
        }
        Ok(())
    }

    /// Install the service-owned mount before publishing any namespace leases.
    pub fn mount_service_downloads(&mut self) -> Result<(), FsError> {
        const ROOT: &[crate::RamEntry] = &[crate::RamEntry::dir("/", 0, 0, mode::DIR_700)];
        let target = path::SERVICE_DOWNLOADS;
        if self.open_handle_count() != 0 {
            return Err(FsError::Busy);
        }
        let stat = self.stat(target)?;
        if stat.file_type != FileType::Directory
            || stat.uid != 0
            || stat.gid != 0
            || stat.mode != mode::DIR_700
        {
            return Err(FsError::InvalidPath);
        }
        let mut empty = true;
        self.read_dir(target, &mut |_| {
            empty = false;
            false
        })?;
        if !empty {
            return Err(FsError::Busy);
        }
        self.mount_tmpfs(target, crate::Tmpfs::new(ROOT))?;
        self.downloads_authority = DownloadsAuthority::Mounted;
        Ok(())
    }

    /// Mount a filesystem at `path`. Path resolution picks the
    /// longest-prefix mount, so nested mounts shadow their parents.
    pub fn mount(&mut self, path: &str, fs: FsNode<D>) -> Result<(), FsError> {
        self.mount_with_metadata(path, fs, 0, true)
    }

    fn mount_with_metadata(
        &mut self,
        path: &str,
        fs: FsNode<D>,
        source: u64,
        protected: bool,
    ) -> Result<(), FsError> {
        self.install_mount(path, source, protected, || fs).map(|_| ())
    }

    fn install_mount(
        &mut self,
        path: &str,
        source: u64,
        protected: bool,
        backend: impl FnOnce() -> FsNode<D>,
    ) -> Result<usize, FsError> {
        path::validate_absolute(path)?;
        self.check_service_reservation(path)?;
        if self.downloads_authority != DownloadsAuthority::Legacy
            && path != "/"
            && (path::service_downloads(path)
                || path::strip_mount(path::SERVICE_DOWNLOADS, path).is_some())
        {
            return Err(FsError::Unsupported);
        }
        if self.count >= MAX_MOUNTS {
            return Err(FsError::TooManyOpenFiles);
        }
        if self.mounts.iter().flatten().any(|mount| mount.path == path) {
            return Err(FsError::InvalidPath);
        }

        let slot = self
            .mounts
            .iter()
            .enumerate()
            .position(|(i, m)| m.is_none() && self.next_tags[i] <= u8::MAX as u16)
            .ok_or(FsError::TooManyOpenFiles)?;
        let tag = self.next_tags[slot] as u8;
        let mut mount_path = String::new();
        mount_path
            .try_reserve_exact(path.len())
            .map_err(|_| FsError::NoMemory)?;
        mount_path.push_str(path);
        #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
        self.advance_topology()?;
        self.next_tags[slot] += MAX_MOUNTS as u16;
        self.mounts[slot] = Some(Mount {
            path: mount_path,
            fs: backend(),
            tag,
            source,
            protected,
            open_handles: 0,
            #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
            device_attachment: None,
        });
        self.count += 1;
        Ok(slot)
    }

    pub fn mount_tmpfs(&mut self, path: &str, fs: crate::Tmpfs) -> Result<(), FsError> {
        self.mount(path, FsNode::Ram(fs))
    }

    /// Compatibility spelling for older callers and fstab entries.
    pub fn mount_ramfs(&mut self, path: &str, fs: RamFs) -> Result<(), FsError> {
        self.mount_tmpfs(path, fs)
    }

    pub fn mount_fat(&mut self, path: &str, fat: Fat32<D>) -> Result<(), FsError> {
        let mut fs = FatFs::new(fat);
        fs.user_storage = path == "/state";
        self.mount(path, FsNode::Fat(fs))
    }

    /// Record the registry identity on a protected boot mount.
    pub fn set_mount_source(&mut self, path: &str, source: u64) -> Result<(), FsError> {
        let index = self
            .mounts
            .iter()
            .position(|m| m.as_ref().is_some_and(|m| m.path == path))
            .ok_or(FsError::NotFound)?;
        let mount = self.mounts[index].as_ref().unwrap();
        if mount.source == source {
            return Ok(());
        }
        #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
        {
            if !mount.protected && matches!(mount.fs, FsNode::Ram(_)) {
                return Err(FsError::Unsupported);
            }
            self.advance_topology()?;
        }
        let mount = self.mounts[index].as_mut().unwrap();
        mount.source = source;
        Ok(())
    }

    pub fn mount_info(&self, index: usize) -> Option<MountInfo<'_>> {
        let mount = self.mounts.iter().flatten().nth(index)?;
        Some(MountInfo {
            id: u64::from(mount.tag) + 1,
            source: mount.source,
            protected: mount.protected,
            path: &mount.path,
        })
    }

    /// Resolve a previously authorized mount incarnation without another
    /// pathname lookup. An unpublished or poisoned count is unavailable.
    pub fn fat_capacity(&self, mount_id: u64) -> Result<(u32, u32, u32), FsError> {
        let mount = self.mounts.iter().flatten()
            .find(|m| u64::from(m.tag) + 1 == mount_id)
            .ok_or(FsError::NotFound)?;
        match &mount.fs {
            FsNode::Fat(fs) if !fs.poisoned => fs.fat.capacity().ok_or(FsError::Unsupported),
            FsNode::Fat(_) => Err(FsError::Io),
            FsNode::Ram(_) => Err(FsError::Unsupported),
        }
    }

    /// Number of backend opens still owned by VFS clients, across all mounts.
    pub fn open_handle_count(&self) -> usize {
        self.mounts
            .iter()
            .flatten()
            .map(|mount| mount.open_handles)
            .sum()
    }

    /// A narrow runtime namespace keeps root, boot, state and service state
    /// policy outside the authority conveyed by an ordinary mount request.
    pub fn validate_runtime_target(&mut self, path: &str, source: u64) -> Result<(), FsError> {
        self.validate_attachment_target(path, Some(source))
    }

    fn validate_attachment_target(&mut self, path: &str, source: Option<u64>) -> Result<(), FsError> {
        path::validate_absolute(path)?;
        if path.len() >= 128 || path.contains("//") {
            return Err(FsError::InvalidPath);
        }
        if path != "/mnt" && !path.starts_with("/mnt/") {
            return Err(FsError::OperationNotPermitted);
        }
        if source == Some(0) {
            return Err(FsError::NotFound);
        }
        if self
            .mounts
            .iter()
            .flatten()
            .any(|m| m.path == path || Some(m.source) == source)
        {
            return Err(FsError::AlreadyExists);
        }
        // Do not stack mounts or shadow a child mount.
        if self.mounts.iter().flatten().any(|m| {
            m.path != "/"
                && (path::strip_mount(path, &m.path).is_some()
                    || path::strip_mount(&m.path, path).is_some())
        }) {
            return Err(FsError::Busy);
        }
        let stat = self.stat(path)?;
        if stat.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        if stat.uid != 0 || stat.gid != 0 || stat.mode & 0o022 != 0 {
            return Err(FsError::InsecureMetadata);
        }
        let mut occupied = false;
        self.read_dir(path, &mut |_| {
            occupied = true;
            false
        })?;
        if occupied {
            return Err(FsError::Busy);
        }
        Ok(())
    }

    /// Call under the same lock as all other VFS access. No filesystem may
    /// escape this lock; open handles pin the mount until their final close.
    pub fn mount_runtime_fat(
        &mut self,
        path: &str,
        source: u64,
        fat: Fat32<D>,
    ) -> Result<(), FsError> {
        self.validate_runtime_target(path, source)?;
        self.mount_with_metadata(path, FsNode::Fat(FatFs::new(fat)), source, false)
    }

    pub fn is_runtime_path(&self, path: &str) -> bool {
        self.resolve_mount(path)
            .ok()
            .and_then(|(i, _)| self.mounts[i].as_ref())
            .is_some_and(|m| !m.protected)
    }

    pub fn unmount(&mut self, id: u64) -> Result<(), FsError> {
        let index = self
            .mounts
            .iter()
            .position(|m| m.as_ref().is_some_and(|m| u64::from(m.tag) + 1 == id))
            .ok_or(FsError::NotFound)?;
        #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
        let revision = self.next_topology_revision()?;
        let mount = self.mounts[index].as_mut().ok_or(FsError::NotFound)?;
        #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
        if mount.device_attachment.is_some() {
            return Err(FsError::OperationNotPermitted);
        }
        if mount.protected {
            return Err(FsError::OperationNotPermitted);
        }
        if mount.open_handles != 0 {
            return Err(FsError::Busy);
        }
        #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
        let volatile = matches!(mount.fs, FsNode::Ram(_));
        #[cfg(target_os = "none")]
        let volatile = false;
        // Keep the complete mount/cache installed on any barrier failure.
        if !volatile {
            mount.fs.sync_dir("/")?;
        }
        self.mounts[index] = None;
        self.count -= 1;
        #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
        {
            self.topology_revision = revision;
        }
        Ok(())
    }

    pub fn open(&mut self, path: &str) -> Result<FileHandle, FsError> {
        let (mount_idx, local_path) = self.resolve_mount(path)?;
        let handle = self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .open(local_path)?;
        let mount = self.mounts[mount_idx].as_mut().ok_or(FsError::BadHandle)?;
        mount.open_handles += 1;
        Ok(pack_handle(mount.tag as usize, handle))
    }

    pub fn create_file(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<FileHandle, FsError> {
        let (mount_idx, local_path) = self.resolve_mount_for_create(path)?;
        let handle = self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .create_file(local_path, uid, gid, mode)?;
        let mount = self.mounts[mount_idx].as_mut().ok_or(FsError::BadHandle)?;
        mount.open_handles += 1;
        Ok(pack_handle(mount.tag as usize, handle))
    }

    /// Exclusively create a file on the destination mount.
    pub fn create_file_exclusive(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<FileHandle, FsError> {
        let (mount_idx, local_path) = self.resolve_mount_for_create(path)?;
        let handle = self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .create_file_exclusive(local_path, uid, gid, mode)?;
        let mount = self.mounts[mount_idx].as_mut().ok_or(FsError::BadHandle)?;
        mount.open_handles += 1;
        Ok(pack_handle(mount.tag as usize, handle))
    }

    /// Exclusively create a private regular file after validating the parent
    /// directory while this VFS is locked.
    pub fn create_private_file_exclusive(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<FileHandle, FsError> {
        validate_private_parent(self, path)?;
        self.create_file_exclusive(path, uid, gid, mode)
    }

    pub fn read(
        &mut self,
        handle: FileHandle,
        offset: usize,
        buf: &mut [u8],
    ) -> Result<usize, FsError> {
        let (mount_idx, local_handle) = self.unpack_handle(handle)?;
        self.mounts
            .get_mut(mount_idx)
            .and_then(Option::as_mut)
            .ok_or(FsError::BadHandle)?
            .fs
            .read(local_handle, offset, buf)
    }

    pub fn write(
        &mut self,
        handle: FileHandle,
        offset: usize,
        buf: &[u8],
    ) -> Result<usize, FsError> {
        let (mount_idx, local_handle) = self.unpack_handle(handle)?;
        self.mounts
            .get_mut(mount_idx)
            .and_then(Option::as_mut)
            .ok_or(FsError::BadHandle)?
            .fs
            .write(local_handle, offset, buf)
    }

    pub fn reserve(&mut self, handle: FileHandle, size: usize) -> Result<(), FsError> {
        let (mount_idx, local_handle) = self.unpack_handle(handle)?;
        self.mounts
            .get_mut(mount_idx)
            .and_then(Option::as_mut)
            .ok_or(FsError::BadHandle)?
            .fs
            .reserve(local_handle, size)
    }

    pub fn truncate(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let (mount_idx, local_handle) = self.unpack_handle(handle)?;
        self.mounts
            .get_mut(mount_idx)
            .and_then(Option::as_mut)
            .ok_or(FsError::BadHandle)?
            .fs
            .truncate(local_handle)
    }

    pub fn close(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let (mount_idx, local_handle) = self.unpack_handle(handle)?;
        let mount = self.mounts[mount_idx].as_mut().ok_or(FsError::BadHandle)?;
        mount.fs.close(local_handle)?;
        mount.open_handles -= 1;
        Ok(())
    }

    pub fn sync_file(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let (mount_idx, local_handle) = self.unpack_handle(handle)?;
        self.mounts
            .get_mut(mount_idx)
            .and_then(Option::as_mut)
            .ok_or(FsError::BadHandle)?
            .fs
            .sync_file(local_handle)
    }

    pub fn sync_dir(&mut self, path: &str) -> Result<(), FsError> {
        let (mount_idx, local_path) = self.resolve_mount(path)?;
        self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .sync_dir(local_path)
    }

    /// Return metadata for an open handle.  Used by `sys_fstat` and
    /// `sys_lseek(SEEK_END)` so they can avoid a path round-trip.
    pub fn fstat_handle(&mut self, handle: FileHandle) -> Result<FileStat, FsError> {
        let (mount_idx, local_handle) = self.unpack_handle(handle)?;
        self.mounts
            .get_mut(mount_idx)
            .and_then(Option::as_mut)
            .ok_or(FsError::BadHandle)?
            .fs
            .fstat_handle(local_handle)
    }

    /// Backend-local identity of a retained object. The mount identity must be
    /// carried separately; object numbers alone are not namespace authority.
    pub fn handle_object_id(&self, handle: FileHandle) -> Result<Option<u64>, FsError> {
        let (mount_idx, local_handle) = self.unpack_handle(handle)?;
        self.mounts[mount_idx]
            .as_ref()
            .ok_or(FsError::BadHandle)?
            .fs
            .handle_object_id(local_handle)
    }

    pub fn stat(&mut self, path: &str) -> Result<FileStat, FsError> {
        let (mount_idx, local_path) = self.resolve_mount(path)?;
        self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .stat(local_path)
    }

    pub fn mkdir(&mut self, path: &str, uid: u32, gid: u32, mode: u16) -> Result<(), FsError> {
        let (mount_idx, local_path) = self.resolve_mount(path)?;
        self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .mkdir(local_path, uid, gid, mode)
    }

    pub fn chmod(&mut self, path: &str, mode: u16) -> Result<(), FsError> {
        let (mount_idx, local_path) = self.resolve_mount(path)?;
        self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .chmod(local_path, mode)
    }

    pub fn chown(&mut self, path: &str, uid: u32, gid: u32) -> Result<(), FsError> {
        let (mount_idx, local_path) = self.resolve_mount(path)?;
        self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .chown(local_path, uid, gid)
    }

    pub fn unlink(&mut self, path: &str) -> Result<(), FsError> {
        self.check_service_reservation(path)?;
        self.check_downloads_mountpoint(path)?;
        #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
        self.check_attachment_ancestor(path)?;
        let (mount_idx, local_path) = self.resolve_mount(path)?;
        self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .unlink(local_path)
    }

    /// Must be called under the same VFS lock as rename so collisions cannot race.
    pub fn rename_no_replace(&mut self, old_path: &str, new_path: &str) -> Result<(), FsError> {
        match self.stat(new_path) {
            Ok(_) => return Err(FsError::AlreadyExists),
            Err(FsError::NotFound) => {}
            Err(error) => return Err(error),
        }
        self.rename(old_path, new_path)
    }

    pub fn rename(&mut self, old_path: &str, new_path: &str) -> Result<(), FsError> {
        self.check_service_reservation(old_path)?;
        self.check_service_reservation(new_path)?;
        self.check_downloads_mountpoint(old_path)?;
        self.check_downloads_mountpoint(new_path)?;
        #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
        {
            self.check_attachment_ancestor(old_path)?;
            self.check_attachment_ancestor(new_path)?;
        }
        let (old_mount, old_local) = self.resolve_mount(old_path)?;
        let (new_mount, new_local) = self.resolve_mount_for_create(new_path)?;
        if old_mount != new_mount {
            return Err(FsError::Unsupported);
        }
        self.mounts[old_mount]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .rename(old_local, new_local)
    }

    /// Atomically publish a regular private file within one mounted filesystem.
    ///
    /// The caller supplies the metadata contract used for both the staged
    /// source and, for replacement, the previous destination.  Validation and
    /// rename execute while this VFS instance is locked by the kernel, so a
    /// path swap cannot be interposed between target validation and publish.
    pub fn publish_private(
        &mut self,
        old_path: &str,
        new_path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
        replace: bool,
    ) -> Result<(), FsError> {
        let (fs, old_local, new_local) = self.prepare_private_publication(
            old_path, new_path, uid, gid, mode, replace,
        )?;
        fs.rename(old_local, new_local)?;
        validate_private_stat(fs.stat(new_local)?, uid, gid, mode)
    }

    /// The service serializes the prepared rename through the copied reply.
    /// Its root tmpfs can commit without allocating or replacing a pinned
    /// destination inode after the kernel accepts the reply.
    pub fn publish_private_if(
        &mut self, old_path: &str, new_path: &str, uid: u32, gid: u32,
        mode: u16, replace: bool, acknowledge: impl FnOnce() -> bool,
    ) -> Result<(), FsError> {
        if old_path == new_path {
            return Err(FsError::InvalidPath);
        }
        let (fs, old_local, new_local) = self.prepare_private_publication(
            old_path, new_path, uid, gid, mode, replace,
        )?;
        match fs {
            FsNode::Ram(fs) => fs.rename_private_if(old_local, new_local, acknowledge),
            FsNode::Fat(_) => Err(FsError::Unsupported),
        }
    }

    fn prepare_private_publication<'a>(
        &mut self, old_path: &'a str, new_path: &'a str, uid: u32, gid: u32,
        mode: u16, replace: bool,
    ) -> Result<(&mut FsNode<D>, &'a str, &'a str), FsError> {
        if parent_path(old_path)? != parent_path(new_path)? {
            return Err(FsError::Unsupported);
        }
        validate_private_parent(self, old_path)?;
        let (old_mount, old_local) = self.resolve_mount(old_path)?;
        let (new_mount, new_local) = self.resolve_mount_for_create(new_path)?;
        if old_mount != new_mount {
            return Err(FsError::Unsupported);
        }

        let fs = &mut self.mounts[old_mount].as_mut().ok_or(FsError::NotFound)?.fs;
        let source = fs.stat(old_local)?;
        validate_private_stat(source, uid, gid, mode)?;

        match fs.stat(new_local) {
            Ok(_destination) if !replace => return Err(FsError::AlreadyExists),
            Ok(destination) => validate_private_stat(destination, uid, gid, mode)?,
            Err(FsError::NotFound) if replace => return Err(FsError::NotFound),
            Err(FsError::NotFound) => {}
            Err(error) => return Err(error),
        }
        Ok((fs, old_local, new_local))
    }

    pub fn read_dir(
        &mut self,
        path: &str,
        f: &mut dyn FnMut(&VfsDirEntry) -> bool,
    ) -> Result<(), FsError> {
        let (mount_idx, local_path) = self.resolve_mount(path)?;
        self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .read_dir(local_path, f)
    }

    /// Absolute VFS path of an open handle. Used by directory-fd resolution
    /// and getdents64. Backend path lifetime differs: tmpfs follows the entry's
    /// current name, while FAT retains the spelling captured at open time.
    /// This is not an authoritative canonical-path or authorization interface.
    pub fn handle_path(&self, handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError> {
        let (mount_idx, local_handle) = self.unpack_handle(handle)?;
        let mount = self
            .mounts
            .get(mount_idx)
            .and_then(Option::as_ref)
            .ok_or(FsError::BadHandle)?;
        let mut local = [0u8; 256];
        let local_len = mount.fs.handle_path(local_handle, &mut local)?;
        let local_path =
            core::str::from_utf8(&local[..local_len]).map_err(|_| FsError::InvalidPath)?;
        let absolute = if mount.path == "/" {
            alloc::string::String::from(local_path)
        } else if local_path == "/" {
            alloc::string::String::from(mount.path.as_str())
        } else {
            alloc::format!("{}{}", mount.path, local_path)
        };
        if buf.len() < absolute.len() {
            return Err(FsError::InvalidPath);
        }
        buf[..absolute.len()].copy_from_slice(absolute.as_bytes());
        Ok(absolute.len())
    }

    fn unpack_handle(&self, handle: FileHandle) -> Result<(usize, FileHandle), FsError> {
        let tag = (handle.0 >> 22) as u8;
        let index = (tag as usize) % MAX_MOUNTS;
        let local = handle.0 & 0x003f_ffff;
        let mount = self.mounts[index].as_ref().ok_or(FsError::BadHandle)?;
        if local == 0 || mount.tag != tag {
            return Err(FsError::BadHandle);
        }
        Ok((index, FileHandle(local)))
    }

    fn resolve_mount<'a>(&self, path: &'a str) -> Result<(usize, &'a str), FsError> {
        path::validate_absolute(path)?;
        self.check_service_reservation(path)?;
        let mut best: Option<(usize, usize, &'a str)> = None;
        for (idx, mount) in self.mounts.iter().enumerate() {
            let Some(mount) = mount else {
                continue;
            };
            let Some(local_path) = path::strip_mount(path, &mount.path) else {
                continue;
            };
            let len = mount.path.len();
            if best.map_or(true, |(_, best_len, _)| len > best_len) {
                best = Some((idx, len, local_path));
            }
        }
        let (index, _, local) = best.ok_or(FsError::NotFound)?;
        #[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
        if self.mounts[index]
            .as_ref()
            .unwrap()
            .device_attachment
            .as_ref()
            .is_some_and(attachment::DeviceState::closed)
        {
            return Err(FsError::Busy);
        }
        Ok((index, local))
    }

    fn resolve_mount_for_create<'a>(&self, path: &'a str) -> Result<(usize, &'a str), FsError> {
        path::validate_absolute(path)?;
        self.check_service_reservation(path)?;
        let parent = parent_path(path)?;
        let (mount_idx, _) = self.resolve_mount(parent)?;
        let mount = self.mounts[mount_idx].as_ref().ok_or(FsError::NotFound)?;
        path::strip_mount(path, &mount.path)
            .map(|local| (mount_idx, local))
            .ok_or(FsError::NotFound)
    }
}

fn validate_private_parent<D: BlockDevice>(vfs: &mut Vfs<D>, path: &str) -> Result<(), FsError> {
    let parent = parent_path(path)?;
    let stat = vfs.stat(parent)?;
    if stat.file_type != FileType::Directory {
        return Err(FsError::NotDir);
    }
    if stat.uid != 0 || stat.gid != 0 || stat.mode & 0o022 != 0 {
        return Err(FsError::InsecureMetadata);
    }
    Ok(())
}

fn validate_private_stat(stat: FileStat, uid: u32, gid: u32, mode: u16) -> Result<(), FsError> {
    if stat.file_type != FileType::File {
        return Err(FsError::UnexpectedType);
    }
    if stat.uid != uid || stat.gid != gid || stat.mode != (mode::S_IFREG | mode) || stat.nlinks != 1
    {
        return Err(FsError::InsecureMetadata);
    }
    Ok(())
}

fn parent_path(path: &str) -> Result<&str, FsError> {
    if path == "/" {
        return Err(FsError::InvalidPath);
    }
    match path.rfind('/') {
        Some(0) => Ok("/"),
        Some(idx) => Ok(&path[..idx]),
        None => Err(FsError::InvalidPath),
    }
}

impl<D: BlockDevice> Default for Vfs<D> {
    fn default() -> Self {
        Self::new()
    }
}

fn pack_handle(mount_idx: usize, local: FileHandle) -> FileHandle {
    FileHandle(((mount_idx as u32) << 22) | (local.0 & 0x003f_ffff))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RamEntry;
    use alloc::vec;
    use alloc::vec::Vec;
    use sunlight_block::MemDisk;
    use sunlight_fat::testimg::FatImageBuilder;

    use crate::vfs::mode;

    static ROOT_ENTRIES: &[RamEntry] = &[
        RamEntry::file(
            "/etc/motd",
            0,
            0,
            mode::FILE_644,
            b"Welcome to SunlightOS\n",
        ),
        RamEntry::file(
            "/etc/passwd",
            0,
            0,
            mode::FILE_644,
            b"root:x:0:0:root:/root:/bin/sh\n",
        ),
    ];

    static BOOT_ENTRIES: &[RamEntry] = &[RamEntry::file(
        "/HELLO.TXT",
        0,
        0,
        mode::FILE_644,
        b"boot volume\n",
    )];

    static DOWNLOADS_SEED: &[RamEntry] = &[
        RamEntry::dir("/root", 0, 0, mode::DIR_700),
        RamEntry::dir(path::SERVICE_DOWNLOADS, 0, 0, mode::DIR_700),
        RamEntry::file("/root/boot", 0, 0, mode::FILE_644, b"boot"),
    ];

    #[test]
    fn downloads_reservation_blocks_legacy_producers_before_first_mount() {
        const FILE: &str = "/root/Downloads/file";
        let mut legacy: Vfs = Vfs::new();
        legacy.reserve_service_downloads(DOWNLOADS_SEED).unwrap();
        legacy.mount_tmpfs("/", crate::Tmpfs::new(DOWNLOADS_SEED)).unwrap();
        assert_eq!(legacy.open(path::SERVICE_DOWNLOADS), Err(FsError::Unsupported));
        assert_eq!(legacy.create_file(FILE, 0, 0, mode::FILE_644), Err(FsError::Unsupported));
        assert_eq!(legacy.mkdir("/root/Downloads/dir", 0, 0, mode::DIR_700), Err(FsError::Unsupported));
        assert_eq!(legacy.rename("/root/boot", FILE), Err(FsError::Unsupported));
        assert_eq!(legacy.mount_tmpfs("/root", crate::Tmpfs::new(&[])), Err(FsError::Unsupported));
        assert_eq!(legacy.rename("/root", "/moved"), Err(FsError::Busy));
        assert_eq!(legacy.unlink("/root"), Err(FsError::Busy));
        assert_eq!(legacy.reserve_service_downloads(DOWNLOADS_SEED), Err(FsError::Busy));
        let handle = legacy.open("/root/boot").unwrap();
        let mut data = [0; 4];
        assert_eq!(legacy.read(handle, 0, &mut data), Ok(4));
        assert_eq!(&data, b"boot");
        legacy.close(handle).unwrap();
    }

    #[test]
    fn downloads_reservation_rejects_a_nonempty_seed() {
        let seed = [
            RamEntry::dir(path::SERVICE_DOWNLOADS, 0, 0, mode::DIR_700),
            RamEntry::file("/root/Downloads/data", 0, 0, mode::FILE_644, b"preserve"),
        ];
        let mut legacy: Vfs = Vfs::new();
        assert_eq!(legacy.reserve_service_downloads(&seed), Err(FsError::InvalidPath));
    }

    #[test]
    fn downloads_mount_refuses_live_references_and_existing_data() {
        const FILE: &str = "/root/Downloads/existing";
        let mut service: Vfs = Vfs::new();
        service.mount_tmpfs("/", crate::Tmpfs::new(DOWNLOADS_SEED)).unwrap();
        let cwd = service.open(path::SERVICE_DOWNLOADS).unwrap();
        assert_eq!(service.mount_service_downloads(), Err(FsError::Busy));
        service.close(cwd).unwrap();
        let handle = service.create_file(FILE, 0, 0, mode::FILE_644).unwrap();
        service.write(handle, 0, b"preserve").unwrap();
        service.close(handle).unwrap();
        assert_eq!(service.mount_service_downloads(), Err(FsError::Busy));
        let handle = service.open(FILE).unwrap();
        let mut contents = [0; 8];
        assert_eq!(service.read(handle, 0, &mut contents), Ok(8));
        assert_eq!(&contents, b"preserve");
        service.close(handle).unwrap();
        assert_eq!(service.open_handle_count(), 0);
    }

    #[test]
    fn downloads_mount_preserves_open_file_across_rename() {
        const FILE: &str = "/root/Downloads/file";
        const MOVED: &str = "/root/Downloads/moved";
        let mut service: Vfs = Vfs::new();
        service.mount_tmpfs("/", crate::Tmpfs::new(DOWNLOADS_SEED)).unwrap();
        service.mount_service_downloads().unwrap();
        assert_eq!(service.mount_tmpfs("/root", crate::Tmpfs::new(&[])), Err(FsError::Unsupported));
        assert_eq!(service.rename("/root", "/moved"), Err(FsError::Busy));
        assert_eq!(service.unlink(path::SERVICE_DOWNLOADS), Err(FsError::Busy));
        let baseline = service.open_handle_count();
        let root = service.open("/").unwrap();
        let cwd = service.open(path::SERVICE_DOWNLOADS).unwrap();
        let actor = crate::Credential { uid: 0, gid: 0 };
        let owner = service.stat_beneath(root, cwd, ".", actor).unwrap().mount_id;
        assert_ne!(owner, service.stat_beneath(root, root, "/root", actor).unwrap().mount_id);
        service.mkdir_beneath(root, cwd, "new-dir", actor, 0o700, |_, _| Ok(())).unwrap();
        assert_eq!(service.stat_beneath(root, cwd, "new-dir", actor).unwrap().mount_id, owner);
        let handle = service.open_file_options_beneath(
            root, cwd, "file", actor,
            OpenOptions { read: true, write: true, create_mode: Some(0o644), ..OpenOptions::default() },
            |_, _| Ok(()),
        ).unwrap();
        service.write(handle, 0, b"one").unwrap();
        assert_eq!(service.rename(FILE, "/root/moved"), Err(FsError::Unsupported));
        service.rename_beneath(root, cwd, "file", "moved", actor, false, |_, _, _| Ok(())).unwrap();
        assert_eq!(service.stat(FILE), Err(FsError::NotFound));
        assert_eq!(service.stat(MOVED).unwrap().size, 3);
        let mut data = [0; 3];
        assert_eq!(service.read(handle, 0, &mut data), Ok(3));
        assert_eq!(&data, b"one");
        service.close(handle).unwrap();
        service.close(cwd).unwrap();
        service.close(root).unwrap();
        assert_eq!(service.open_handle_count(), baseline);
    }

    #[test]
    fn no_replace_move_preserves_existing_target_and_source() {
        let mut vfs: Vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(&[])).unwrap();
        vfs.mkdir("/source", 0, 0, 0o755).unwrap();
        vfs.mkdir("/target", 0, 0, 0o755).unwrap();
        assert_eq!(
            vfs.rename_no_replace("/source", "/target"),
            Err(FsError::AlreadyExists)
        );
        assert!(vfs.stat("/source").is_ok());
        assert!(vfs.stat("/target").is_ok());
        vfs.rename_no_replace("/source", "/moved").unwrap();
        assert!(vfs.stat("/source").is_err());
        assert!(vfs.stat("/moved").is_ok());
    }

    #[test]
    fn handle_paths_preserve_mount_prefix_and_validate_lifetime() {
        let mut vfs: Vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();
        vfs.mount_ramfs("/boot", RamFs::new(BOOT_ENTRIES)).unwrap();
        let root = vfs.open("/etc/motd").unwrap();
        let mounted = vfs.open("/boot/HELLO.TXT").unwrap();
        let directory = vfs.open("/boot").unwrap();
        let mut buf = [0u8; 64];
        for (handle, expected) in [
            (root, "/etc/motd"),
            (mounted, "/boot/HELLO.TXT"),
            (directory, "/boot"),
        ] {
            let len = vfs.handle_path(handle, &mut buf).unwrap();
            assert_eq!(&buf[..len], expected.as_bytes());
        }
        let mut short = [0xa5; 4];
        assert_eq!(
            vfs.handle_path(mounted, &mut short),
            Err(FsError::InvalidPath)
        );
        assert_eq!(short, [0xa5; 4]);
        vfs.close(mounted).unwrap();
        assert_eq!(vfs.handle_path(mounted, &mut buf), Err(FsError::BadHandle));
        vfs.close(root).unwrap();
        vfs.close(directory).unwrap();
    }

    #[test]
    fn routes_root_mount_open_read_stat() {
        let mut vfs: Vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();

        let stat = vfs.stat("/etc/motd").unwrap();
        assert_eq!(stat.size, b"Welcome to SunlightOS\n".len());

        let handle = vfs.open("/etc/motd").unwrap();
        let mut buf = [0u8; 24];
        let read = vfs.read(handle, 0, &mut buf).unwrap();
        assert_eq!(&buf[..read], b"Welcome to SunlightOS\n");
        assert_eq!(vfs.close(handle), Ok(()));
    }

    #[test]
    fn ramfs_never_claims_file_or_directory_durability() {
        let mut vfs: Vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();

        let handle = vfs.open("/etc/motd").unwrap();
        assert_eq!(vfs.sync_file(handle), Err(FsError::Unsupported));
        vfs.mkdir("/durable", 0, 0, 0o700).unwrap();
        assert_eq!(vfs.sync_dir("/durable"), Err(FsError::Unsupported));
    }

    #[test]
    fn chooses_longest_matching_mount() {
        let mut vfs: Vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();
        vfs.mount_ramfs("/boot", RamFs::new(BOOT_ENTRIES)).unwrap();

        let handle = vfs.open("/boot/HELLO.TXT").unwrap();
        let mut buf = [0u8; 16];
        let read = vfs.read(handle, 0, &mut buf).unwrap();

        assert_eq!(&buf[..read], b"boot volume\n");
    }

    #[test]
    fn reports_missing_file_from_resolved_mount() {
        let mut vfs: Vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();

        assert_eq!(vfs.open("/missing"), Err(FsError::NotFound));
    }

    #[test]
    fn rejects_bad_global_handle() {
        let mut vfs: Vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();

        assert_eq!(
            vfs.read(FileHandle(0), 0, &mut [0u8; 8]),
            Err(FsError::BadHandle)
        );
    }

    #[test]
    fn private_publish_create_if_absent_keeps_winning_file() {
        static PRIVATE_ENTRIES: &[RamEntry] = &[
            RamEntry::dir("/", 0, 0, mode::DIR_755),
            RamEntry::dir("/etc", 0, 0, mode::DIR_755),
            RamEntry::dir("/etc/sunlight", 0, 0, mode::DIR_755),
        ];
        let mut vfs: Vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(PRIVATE_ENTRIES)).unwrap();

        let first = vfs
            .create_private_file_exclusive("/etc/sunlight/.key.tmp.first", 0, 0, 0o600)
            .unwrap();
        vfs.write(first, 0, b"first").unwrap();
        vfs.close(first).unwrap();
        vfs.publish_private(
            "/etc/sunlight/.key.tmp.first",
            "/etc/sunlight/key",
            0,
            0,
            0o600,
            false,
        )
        .unwrap();

        let second = vfs
            .create_private_file_exclusive("/etc/sunlight/.key.tmp.second", 0, 0, 0o600)
            .unwrap();
        vfs.write(second, 0, b"second").unwrap();
        vfs.close(second).unwrap();
        assert_eq!(
            vfs.publish_private(
                "/etc/sunlight/.key.tmp.second",
                "/etc/sunlight/key",
                0,
                0,
                0o600,
                false,
            ),
            Err(FsError::AlreadyExists)
        );

        let winner = vfs.open("/etc/sunlight/key").unwrap();
        let mut bytes = [0u8; 8];
        let len = vfs.read(winner, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[..len], b"first");
        assert_eq!(
            vfs.stat("/etc/sunlight/.key.tmp.second").unwrap().mode,
            mode::FILE_600
        );
    }

    #[test]
    fn private_replace_rejects_bad_destination_without_touching_old_bytes() {
        static PRIVATE_ENTRIES: &[RamEntry] = &[
            RamEntry::dir("/", 0, 0, mode::DIR_755),
            RamEntry::dir("/etc", 0, 0, mode::DIR_755),
            RamEntry::dir("/etc/sunlight", 0, 0, mode::DIR_755),
        ];
        let mut vfs: Vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(PRIVATE_ENTRIES)).unwrap();
        let old = vfs
            .create_private_file_exclusive("/etc/sunlight/key", 0, 0, 0o600)
            .unwrap();
        vfs.write(old, 0, b"old").unwrap();
        vfs.close(old).unwrap();
        vfs.chmod("/etc/sunlight/key", 0o644).unwrap();

        let replacement = vfs
            .create_private_file_exclusive("/etc/sunlight/.key.tmp.next", 0, 0, 0o600)
            .unwrap();
        vfs.write(replacement, 0, b"new").unwrap();
        vfs.close(replacement).unwrap();
        assert_eq!(
            vfs.publish_private(
                "/etc/sunlight/.key.tmp.next",
                "/etc/sunlight/key",
                0,
                0,
                0o600,
                true,
            ),
            Err(FsError::InsecureMetadata)
        );

        let old = vfs.open("/etc/sunlight/key").unwrap();
        let mut bytes = [0u8; 8];
        let len = vfs.read(old, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[..len], b"old");
    }

    #[test]
    fn rejects_duplicate_mount() {
        let mut vfs: Vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();

        assert_eq!(
            vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)),
            Err(FsError::InvalidPath)
        );
    }

    fn boot_image() -> Vec<u8> {
        let mut builder = FatImageBuilder::new(1);
        builder.add_file(builder.root(), "HELLO.TXT", b"fat volume\n");
        let utils = builder.add_dir(builder.root(), "UTILS");
        builder.add_file(utils, "LS.ELF", b"\x7fELF fake binary");
        builder.build()
    }

    #[test]
    fn state_disk_user_files_have_private_uid_metadata() {
        let mut image = boot_image();
        let fat = Fat32::mount(MemDisk::new(&mut image)).unwrap();
        let mut vfs: Vfs<MemDisk> = Vfs::new();
        static ROOT: &[RamEntry] = &[RamEntry::dir("/state", 0, 0, mode::DIR_755)];
        vfs.mount_ramfs("/", RamFs::new(ROOT)).unwrap();
        vfs.mount_fat("/state", fat).unwrap();
        vfs.mkdir("/state/users", 0, 0, 0o755).unwrap();
        vfs.mkdir("/state/users/1000", 1000, 1000, 0o700).unwrap();
        let handle = vfs
            .create_file_exclusive("/state/users/1000/voice.wav", 1000, 1000, 0o600)
            .unwrap();
        for stat in [
            vfs.stat("/state/users/1000/voice.wav").unwrap(),
            vfs.fstat_handle(handle).unwrap(),
        ] {
            assert_eq!(stat.uid, 1000);
            assert_eq!(stat.mode, 0o600);
            assert!(crate::permission::check_permission(
                &stat,
                &crate::permission::Credential {
                    uid: 1000,
                    gid: 1000
                },
                crate::permission::PermCheck::Read
            ));
            assert!(!crate::permission::check_permission(
                &stat,
                &crate::permission::Credential {
                    uid: 1001,
                    gid: 1001
                },
                crate::permission::PermCheck::Read
            ));
        }
        let dir = vfs.stat("/state/users/1000").unwrap();
        assert_eq!(dir.mode, 0o700);
    }

    #[test]
    fn mounts_fat32_volume_at_directory() {
        let mut image = boot_image();
        let fat = Fat32::mount(MemDisk::new(&mut image)).expect("fat mount");

        let mut vfs: Vfs<MemDisk> = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();
        vfs.mount_fat("/mnt/disk", fat).unwrap();

        // Files on both mounts resolve through one namespace.
        let stat = vfs.stat("/mnt/disk/HELLO.TXT").unwrap();
        assert_eq!(stat.file_type, FileType::File);
        assert_eq!(stat.size, 11);

        let handle = vfs.open("/mnt/disk/UTILS/LS.ELF").unwrap();
        let mut buf = [0u8; 32];
        let read = vfs.read(handle, 0, &mut buf).unwrap();
        assert_eq!(&buf[..read], b"\x7fELF fake binary");
        assert_eq!(vfs.close(handle), Ok(()));

        // RamFs root still resolves.
        assert!(vfs.open("/etc/motd").is_ok());
        // FAT volume supports mutation and a stable-media barrier.
        let handle = vfs.open("/mnt/disk/HELLO.TXT").unwrap();
        assert_eq!(vfs.write(handle, 0, b"x"), Ok(1));
        assert_eq!(vfs.sync_file(handle), Ok(()));
        assert_eq!(vfs.sync_dir("/mnt/disk"), Ok(()));
    }

    #[test]
    fn read_dir_lists_fat_mount() {
        let mut image = boot_image();
        let fat = Fat32::mount(MemDisk::new(&mut image)).expect("fat mount");

        let mut vfs: Vfs<MemDisk> = Vfs::new();
        vfs.mount_fat("/mnt/disk", fat).unwrap();

        let mut names: Vec<(Vec<u8>, FileType)> = Vec::new();
        vfs.read_dir("/mnt/disk", &mut |entry| {
            names.push((entry.name_bytes().to_vec(), entry.file_type));
            true
        })
        .unwrap();

        assert_eq!(
            names,
            vec![
                (b"HELLO.TXT".to_vec(), FileType::File),
                (b"UTILS".to_vec(), FileType::Directory),
            ]
        );

        // Early termination via the callback.
        let mut seen = 0;
        vfs.read_dir("/mnt/disk", &mut |_| {
            seen += 1;
            false
        })
        .unwrap();
        assert_eq!(seen, 1);
    }
}

mod anchored;
#[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
mod attachment;
#[cfg(any(not(target_os = "none"), feature = "attachment-transactions"))]
pub use attachment::{AttachmentCapacity, AttachmentRow, DeviceWithdrawal};
pub use anchored::{ObjectMetadata, OpenOptions};
