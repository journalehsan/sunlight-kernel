use crate::{path, FsError, RamFs};

pub const MAX_MOUNTS: usize = 8;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirIter;

pub trait FileSystem {
    fn open(&mut self, path: &str) -> Result<FileHandle, FsError>;
    fn read(&mut self, handle: FileHandle, offset: usize, buf: &mut [u8])
        -> Result<usize, FsError>;
    fn close(&mut self, handle: FileHandle) -> Result<(), FsError>;
    fn stat(&mut self, path: &str) -> Result<FileStat, FsError>;
    fn readdir(&mut self, path: &str) -> Result<DirIter, FsError>;
}

pub enum FsNode {
    Ram(RamFs),
}

impl FileSystem for FsNode {
    fn open(&mut self, path: &str) -> Result<FileHandle, FsError> {
        match self {
            Self::Ram(fs) => fs.open(path),
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
        }
    }

    fn close(&mut self, handle: FileHandle) -> Result<(), FsError> {
        match self {
            Self::Ram(fs) => fs.close(handle),
        }
    }

    fn stat(&mut self, path: &str) -> Result<FileStat, FsError> {
        match self {
            Self::Ram(fs) => fs.stat(path),
        }
    }

    fn readdir(&mut self, path: &str) -> Result<DirIter, FsError> {
        match self {
            Self::Ram(fs) => fs.readdir(path),
        }
    }
}

pub struct Mount {
    path: &'static str,
    fs: FsNode,
}

pub struct Vfs {
    mounts: [Option<Mount>; MAX_MOUNTS],
    count: usize,
}

impl Vfs {
    pub const fn new() -> Self {
        Self {
            mounts: [const { None }; MAX_MOUNTS],
            count: 0,
        }
    }

    pub fn mount_ramfs(&mut self, path: &'static str, fs: RamFs) -> Result<(), FsError> {
        path::validate_absolute(path)?;
        if self.count >= MAX_MOUNTS {
            return Err(FsError::TooManyOpenFiles);
        }
        if self.mounts.iter().flatten().any(|mount| mount.path == path) {
            return Err(FsError::InvalidPath);
        }

        self.mounts[self.count] = Some(Mount {
            path,
            fs: FsNode::Ram(fs),
        });
        self.count += 1;
        Ok(())
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

impl Default for Vfs {
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
        let mut vfs = Vfs::new();
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
        let mut vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();
        vfs.mount_ramfs("/boot", RamFs::new(BOOT_ENTRIES)).unwrap();

        let handle = vfs.open("/boot/HELLO.TXT").unwrap();
        let mut buf = [0u8; 16];
        let read = vfs.read(handle, 0, &mut buf).unwrap();

        assert_eq!(&buf[..read], b"boot volume\n");
    }

    #[test]
    fn reports_missing_file_from_resolved_mount() {
        let mut vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();

        assert_eq!(vfs.open("/missing"), Err(FsError::NotFound));
    }

    #[test]
    fn rejects_bad_global_handle() {
        let mut vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();

        assert_eq!(
            vfs.read(FileHandle(0), 0, &mut [0u8; 8]),
            Err(FsError::BadHandle)
        );
    }

    #[test]
    fn rejects_duplicate_mount() {
        let mut vfs = Vfs::new();
        vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)).unwrap();

        assert_eq!(
            vfs.mount_ramfs("/", RamFs::new(ROOT_ENTRIES)),
            Err(FsError::InvalidPath)
        );
    }
}
