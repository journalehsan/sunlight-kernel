use crate::{path, FsError, RamFs};
use sunlight_block::{BlockDevice, NullDevice};
use sunlight_fat::{Fat32, MAX_NAME_83};

pub const MAX_MOUNTS: usize = 8;
/// Maximum file-name length reported by `read_dir`.
pub const VFS_NAME_MAX: usize = 64;
/// Open-file slots per mounted FAT volume.
pub const FAT_MAX_HANDLES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileHandle(pub u32);

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

    pub const DIR_755:  u16 = S_IFDIR | 0o755;
    pub const FILE_644: u16 = S_IFREG | 0o644;
    pub const FILE_600: u16 = S_IFREG | 0o600;
    pub const FILE_755: u16 = S_IFREG | 0o755;
    pub const FILE_700: u16 = S_IFREG | 0o700;
    pub const DIR_700:  u16 = S_IFDIR | 0o700;
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
    fn read(&mut self, handle: FileHandle, offset: usize, buf: &mut [u8])
        -> Result<usize, FsError>;
    fn write(&mut self, handle: FileHandle, offset: usize, buf: &[u8])
        -> Result<usize, FsError>;
    fn close(&mut self, handle: FileHandle) -> Result<(), FsError>;
    fn stat(&mut self, path: &str) -> Result<FileStat, FsError>;
    fn mkdir(&mut self, path: &str, uid: u32, gid: u32, mode: u16) -> Result<(), FsError>;
    fn chmod(&mut self, path: &str, mode: u16) -> Result<(), FsError>;
    fn chown(&mut self, path: &str, uid: u32, gid: u32) -> Result<(), FsError>;
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
}

/// Read-only `FileSystem` adapter over a [`Fat32`] volume: adds the open-file
/// handle table the raw driver doesn't track. FAT carries no ownership, so
/// stats report root-owned 755 entries.
pub struct FatFs<D: BlockDevice> {
    fat: Fat32<D>,
    handles: [Option<FatOpen>; FAT_MAX_HANDLES],
}

impl<D: BlockDevice> FatFs<D> {
    pub fn new(fat: Fat32<D>) -> Self {
        Self {
            fat,
            handles: [None; FAT_MAX_HANDLES],
        }
    }

    fn handle_slot(&self, handle: FileHandle) -> Result<FatOpen, FsError> {
        let idx = handle.0.checked_sub(1).ok_or(FsError::BadHandle)? as usize;
        self.handles
            .get(idx)
            .and_then(|slot| *slot)
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
                *slot = Some(FatOpen {
                    first_cluster: stat.first_cluster,
                    size: stat.size,
                });
                return Ok(FileHandle((idx + 1) as u32));
            }
        }
        Err(FsError::TooManyOpenFiles)
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

    fn write(&mut self, _handle: FileHandle, _offset: usize, _buf: &[u8])
        -> Result<usize, FsError> {
        Err(FsError::Unsupported)
    }

    fn close(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let idx = handle.0.checked_sub(1).ok_or(FsError::BadHandle)? as usize;
        let slot = self.handles.get_mut(idx).ok_or(FsError::BadHandle)?;
        if slot.is_none() {
            return Err(FsError::BadHandle);
        }
        *slot = None;
        Ok(())
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

    fn write(
        &mut self,
        handle: FileHandle,
        offset: usize,
        buf: &[u8],
    ) -> Result<usize, FsError> {
        match self {
            Self::Ram(fs) => fs.write(handle, offset, buf),
            Self::Fat(fs) => fs.write(handle, offset, buf),
        }
    }

    fn close(&mut self, handle: FileHandle) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.close(handle),
            Self::Fat(fs) => fs.close(handle),
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

    pub fn close(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let (mount_idx, local_handle) = unpack_handle(handle)?;
        self.mounts
            .get_mut(mount_idx)
            .and_then(Option::as_mut)
            .ok_or(FsError::BadHandle)?
            .fs
            .close(local_handle)
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
        RamEntry::file("/etc/motd",    0, 0, mode::FILE_644, b"Welcome to SunlightOS\n"),
        RamEntry::file("/etc/passwd",  0, 0, mode::FILE_644, b"root:x:0:0:root:/root:/bin/sh\n"),
    ];

    static BOOT_ENTRIES: &[RamEntry] = &[
        RamEntry::file("/HELLO.TXT", 0, 0, mode::FILE_644, b"boot volume\n"),
    ];

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
