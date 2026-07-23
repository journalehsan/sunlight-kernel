use crate::{path, FsError, RamFs};
use sunlight_block::{BlockDevice, NullDevice};
use sunlight_fat::{Fat32, MAX_NAME_83};

pub const MAX_MOUNTS: usize = 8;
/// Maximum file-name length reported by `read_dir`.
pub const VFS_NAME_MAX: usize = 64;
/// Open-file slots per mounted FAT volume.
pub const FAT_MAX_HANDLES: usize = 16;

// Open handles are private backend capabilities, not indexes that callers may
// safely retain after close.  Keep the slot in the low bits and tag each new
// incarnation with a generation.  The VFS mount packing reserves 24 bits for
// the local handle, leaving 18 generation bits with the six bits required for
// the current (at most 32) backend slots.
pub const HANDLE_SLOT_BITS: u32 = 6;
const HANDLE_SLOT_MASK: u32 = (1 << HANDLE_SLOT_BITS) - 1;
const HANDLE_GENERATION_MAX: u32 = (1 << (24 - HANDLE_SLOT_BITS)) - 1;

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

pub(crate) fn next_handle_generation(generation: u32) -> u32 {
    if generation >= HANDLE_GENERATION_MAX {
        1
    } else {
        generation + 1
    }
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
    /// Reduce an open regular file to length zero.  This is intentionally a
    /// narrow primitive used by `O_TRUNC`; it does not add a public ftruncate
    /// ABI.
    fn truncate(&mut self, handle: FileHandle) -> Result<(), FsError>;
    fn close(&mut self, handle: FileHandle) -> Result<(), FsError>;
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
}

// ---------------------------------------------------------------------------
// FAT32 adapter
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct FatOpen {
    first_cluster: u32,
    size: u32,
    generation: u32,
}

/// Read-only `FileSystem` adapter over a [`Fat32`] volume: adds the open-file
/// handle table the raw driver doesn't track. FAT carries no ownership, so
/// stats report root-owned 755 entries.
pub struct FatFs<D: BlockDevice> {
    fat: Fat32<D>,
    handles: [Option<FatOpen>; FAT_MAX_HANDLES],
    generations: [u32; FAT_MAX_HANDLES],
}

impl<D: BlockDevice> FatFs<D> {
    pub fn new(fat: Fat32<D>) -> Self {
        Self {
            fat,
            handles: [None; FAT_MAX_HANDLES],
            generations: [0; FAT_MAX_HANDLES],
        }
    }

    fn handle_slot(&self, handle: FileHandle) -> Result<FatOpen, FsError> {
        let (idx, generation) = split_local_handle(handle)?;
        self.handles
            .get(idx)
            .and_then(|slot| slot.filter(|open| open.generation == generation))
            .ok_or(FsError::BadHandle)
    }
}

impl<D: BlockDevice> FileSystem for FatFs<D> {
    fn open(&mut self, path: &str) -> Result<FileHandle, FsError> {
        path::validate_absolute(path)?;
        let stat = self
            .fat
            .stat_path(path.as_bytes())
            .ok_or(FsError::NotFound)?;
        if stat.is_dir {
            return Err(FsError::IsDir);
        }
        for (idx, slot) in self.handles.iter_mut().enumerate() {
            if slot.is_none() {
                let generation = next_handle_generation(self.generations[idx]);
                self.generations[idx] = generation;
                *slot = Some(FatOpen {
                    first_cluster: stat.first_cluster,
                    size: stat.size,
                    generation,
                });
                return Ok(make_local_handle(idx, generation));
            }
        }
        Err(FsError::TooManyOpenFiles)
    }

    fn create_file(
        &mut self,
        _path: &str,
        _uid: u32,
        _gid: u32,
        _mode: u16,
    ) -> Result<FileHandle, FsError> {
        Err(FsError::ReadOnlyFilesystem)
    }

    fn create_file_exclusive(
        &mut self,
        _path: &str,
        _uid: u32,
        _gid: u32,
        _mode: u16,
    ) -> Result<FileHandle, FsError> {
        Err(FsError::ReadOnlyFilesystem)
    }

    fn read(
        &mut self,
        handle: FileHandle,
        offset: usize,
        buf: &mut [u8],
    ) -> Result<usize, FsError> {
        let open = self.handle_slot(handle)?;
        self.fat
            .read_at(open.first_cluster, open.size, offset, buf)
            .ok_or(FsError::Io)
    }

    fn write(
        &mut self,
        _handle: FileHandle,
        _offset: usize,
        _buf: &[u8],
    ) -> Result<usize, FsError> {
        Err(FsError::Unsupported)
    }

    fn truncate(&mut self, _handle: FileHandle) -> Result<(), FsError> {
        Err(FsError::ReadOnlyFilesystem)
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

    fn fstat_handle(&mut self, handle: FileHandle) -> Result<FileStat, FsError> {
        let open = self.handle_slot(handle)?;
        Ok(FileStat {
            file_type: FileType::File,
            size: open.size as usize,
            uid: 0,
            gid: 0,
            mode: mode::FILE_755,
            nlinks: 1,
        })
    }

    fn stat(&mut self, path: &str) -> Result<FileStat, FsError> {
        path::validate_absolute(path)?;
        let stat = self
            .fat
            .stat_path(path.as_bytes())
            .ok_or(FsError::NotFound)?;
        Ok(if stat.is_dir {
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
        })
    }

    fn mkdir(&mut self, _path: &str, _uid: u32, _gid: u32, _mode: u16) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    fn chmod(&mut self, _path: &str, _mode: u16) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    fn chown(&mut self, _path: &str, _uid: u32, _gid: u32) -> Result<(), FsError> {
        Err(FsError::Unsupported)
    }

    fn unlink(&mut self, _path: &str) -> Result<(), FsError> {
        Err(FsError::ReadOnlyFilesystem)
    }

    fn rename(&mut self, _old: &str, _new: &str) -> Result<(), FsError> {
        Err(FsError::ReadOnlyFilesystem)
    }

    fn read_dir(
        &mut self,
        path: &str,
        f: &mut dyn FnMut(&VfsDirEntry) -> bool,
    ) -> Result<(), FsError> {
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
}

pub struct Mount<D: BlockDevice = NullDevice> {
    path: &'static str,
    fs: FsNode<D>,
}

pub struct Vfs<D: BlockDevice = NullDevice> {
    mounts: [Option<Mount<D>>; MAX_MOUNTS],
    count: usize,
}

impl<D: BlockDevice> Vfs<D> {
    pub const fn new() -> Self {
        Self {
            mounts: [const { None }; MAX_MOUNTS],
            count: 0,
        }
    }

    /// Mount a filesystem at `path`. Path resolution picks the
    /// longest-prefix mount, so nested mounts shadow their parents.
    pub fn mount(&mut self, path: &'static str, fs: FsNode<D>) -> Result<(), FsError> {
        path::validate_absolute(path)?;
        if self.count >= MAX_MOUNTS {
            return Err(FsError::TooManyOpenFiles);
        }
        if self.mounts.iter().flatten().any(|mount| mount.path == path) {
            return Err(FsError::InvalidPath);
        }

        self.mounts[self.count] = Some(Mount { path, fs });
        self.count += 1;
        Ok(())
    }

    pub fn mount_ramfs(&mut self, path: &'static str, fs: RamFs) -> Result<(), FsError> {
        self.mount(path, FsNode::Ram(fs))
    }

    pub fn mount_fat(&mut self, path: &'static str, fat: Fat32<D>) -> Result<(), FsError> {
        self.mount(path, FsNode::Fat(FatFs::new(fat)))
    }

    pub fn open(&mut self, path: &str) -> Result<FileHandle, FsError> {
        let (mount_idx, local_path) = self.resolve_mount(path)?;
        let handle = self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .open(local_path)?;
        Ok(pack_handle(mount_idx, handle))
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
        Ok(pack_handle(mount_idx, handle))
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
        Ok(pack_handle(mount_idx, handle))
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
        let (mount_idx, local_handle) = unpack_handle(handle)?;
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
        let (mount_idx, local_handle) = unpack_handle(handle)?;
        self.mounts
            .get_mut(mount_idx)
            .and_then(Option::as_mut)
            .ok_or(FsError::BadHandle)?
            .fs
            .write(local_handle, offset, buf)
    }

    pub fn truncate(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let (mount_idx, local_handle) = unpack_handle(handle)?;
        self.mounts
            .get_mut(mount_idx)
            .and_then(Option::as_mut)
            .ok_or(FsError::BadHandle)?
            .fs
            .truncate(local_handle)
    }

    pub fn close(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let (mount_idx, local_handle) = unpack_handle(handle)?;
        self.mounts
            .get_mut(mount_idx)
            .and_then(Option::as_mut)
            .ok_or(FsError::BadHandle)?
            .fs
            .close(local_handle)
    }

    /// Return metadata for an open handle.  Used by `sys_fstat` and
    /// `sys_lseek(SEEK_END)` so they can avoid a path round-trip.
    pub fn fstat_handle(&mut self, handle: FileHandle) -> Result<FileStat, FsError> {
        let (mount_idx, local_handle) = unpack_handle(handle)?;
        self.mounts
            .get_mut(mount_idx)
            .and_then(Option::as_mut)
            .ok_or(FsError::BadHandle)?
            .fs
            .fstat_handle(local_handle)
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
        let (mount_idx, local_path) = self.resolve_mount(path)?;
        self.mounts[mount_idx]
            .as_mut()
            .ok_or(FsError::NotFound)?
            .fs
            .unlink(local_path)
    }

    pub fn rename(&mut self, old_path: &str, new_path: &str) -> Result<(), FsError> {
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
        fs.rename(old_local, new_local)?;
        validate_private_stat(fs.stat(new_local)?, uid, gid, mode)
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

    fn resolve_mount<'a>(&self, path: &'a str) -> Result<(usize, &'a str), FsError> {
        path::validate_absolute(path)?;
        let mut best: Option<(usize, usize, &'a str)> = None;
        for (idx, mount) in self.mounts.iter().enumerate() {
            let Some(mount) = mount else {
                continue;
            };
            let Some(local_path) = path::strip_mount(path, mount.path) else {
                continue;
            };
            let len = mount.path.len();
            if best.map_or(true, |(_, best_len, _)| len > best_len) {
                best = Some((idx, len, local_path));
            }
        }
        best.map(|(idx, _, local)| (idx, local))
            .ok_or(FsError::NotFound)
    }

    fn resolve_mount_for_create<'a>(&self, path: &'a str) -> Result<(usize, &'a str), FsError> {
        path::validate_absolute(path)?;
        let parent = parent_path(path)?;
        let (mount_idx, _) = self.resolve_mount(parent)?;
        let mount = self.mounts[mount_idx].as_ref().ok_or(FsError::NotFound)?;
        path::strip_mount(path, mount.path)
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
    FileHandle(((mount_idx as u32) << 24) | (local.0 & 0x00ff_ffff))
}

fn unpack_handle(handle: FileHandle) -> Result<(usize, FileHandle), FsError> {
    let mount_idx = (handle.0 >> 24) as usize;
    let local = handle.0 & 0x00ff_ffff;
    if local == 0 {
        return Err(FsError::BadHandle);
    }
    Ok((mount_idx, FileHandle(local)))
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
        // FAT volume is read-only.
        let handle = vfs.open("/mnt/disk/HELLO.TXT").unwrap();
        assert_eq!(vfs.write(handle, 0, b"x"), Err(FsError::Unsupported));
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
