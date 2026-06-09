use crate::vfs::{mode, DirIter, FileHandle, FileStat, FileSystem, FileType};
use crate::{path, FsError};

pub const RAMFS_MAX_HANDLES: usize = 32;

pub struct RamEntry {
    pub path:   &'static str,
    pub data:   &'static [u8],
    pub uid:    u32,
    pub gid:    u32,
    pub mode:   u16,
    pub is_dir: bool,
}

impl RamEntry {
    pub const fn file(
        path: &'static str,
        uid: u32,
        gid: u32,
        file_mode: u16,
        data: &'static [u8],
    ) -> Self {
        Self { path, data, uid, gid, mode: file_mode, is_dir: false }
    }

    pub const fn dir(path: &'static str, uid: u32, gid: u32, dir_mode: u16) -> Self {
        Self { path, data: b"", uid, gid, mode: dir_mode, is_dir: true }
    }
}

pub struct RamFs {
    entries: &'static [RamEntry],
    handles: [Option<usize>; RAMFS_MAX_HANDLES],
}

impl RamFs {
    pub const fn new(entries: &'static [RamEntry]) -> Self {
        Self {
            entries,
            handles: [const { None }; RAMFS_MAX_HANDLES],
        }
    }

    fn entry_idx(&self, path: &str) -> Result<usize, FsError> {
        path::validate_absolute(path)?;
        self.entries
            .iter()
            .position(|entry| entry.path == path)
            .ok_or(FsError::NotFound)
    }

    fn alloc_handle(&mut self, entry_idx: usize) -> Result<FileHandle, FsError> {
        for (idx, slot) in self.handles.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(entry_idx);
                return Ok(FileHandle((idx + 1) as u32));
            }
        }
        Err(FsError::TooManyOpenFiles)
    }

    fn handle_entry_idx(&self, handle: FileHandle) -> Result<usize, FsError> {
        let idx = handle.0.checked_sub(1).ok_or(FsError::BadHandle)? as usize;
        self.handles
            .get(idx)
            .and_then(|slot| *slot)
            .ok_or(FsError::BadHandle)
    }
}

impl FileSystem for RamFs {
    fn open(&mut self, path: &str) -> Result<FileHandle, FsError> {
        let entry_idx = self.entry_idx(path)?;
        if self.entries[entry_idx].is_dir {
            return Err(FsError::IsDir);
        }
        self.alloc_handle(entry_idx)
    }

    fn read(
        &mut self,
        handle: FileHandle,
        offset: usize,
        buf: &mut [u8],
    ) -> Result<usize, FsError> {
        let entry_idx = self.handle_entry_idx(handle)?;
        let data = self.entries[entry_idx].data;
        if offset >= data.len() {
            return Ok(0);
        }
        let src = &data[offset..];
        let len = src.len().min(buf.len());
        buf[..len].copy_from_slice(&src[..len]);
        Ok(len)
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
        let entry_idx = self.entry_idx(path)?;
        let e = &self.entries[entry_idx];
        Ok(FileStat {
            file_type: if e.is_dir { FileType::Directory } else { FileType::File },
            size: e.data.len(),
            uid: e.uid,
            gid: e.gid,
            mode: e.mode,
            nlinks: if e.is_dir { 2 } else { 1 },
        })
    }

    fn readdir(&mut self, path: &str) -> Result<DirIter, FsError> {
        path::validate_absolute(path)?;
        Err(FsError::Unsupported)
    }
}

pub static INITRAMFS: &[RamEntry] = &[
    // Directories
    RamEntry::dir("/",                 0,    0,    mode::DIR_755),
    RamEntry::dir("/etc",              0,    0,    mode::DIR_755),
    RamEntry::dir("/etc/sunlight",     0,    0,    mode::DIR_755),
    RamEntry::dir("/bin",              0,    0,    mode::DIR_755),
    RamEntry::dir("/root",             0,    0,    mode::DIR_700),
    RamEntry::dir("/home",             0,    0,    mode::DIR_755),
    RamEntry::dir("/home/user",        1000, 1000, mode::DIR_755),
    RamEntry::dir("/tmp",              0,    0,    mode::DIR_1777),
    RamEntry::dir("/var",              0,    0,    mode::DIR_755),
    RamEntry::dir("/var/log",          0,    0,    mode::DIR_755),

    // System config files (world-readable)
    RamEntry::file("/etc/passwd",  0, 0, mode::FILE_644,
        include_bytes!("../etc/passwd")),
    RamEntry::file("/etc/group",   0, 0, mode::FILE_644,
        include_bytes!("../etc/group")),
    RamEntry::file("/etc/shadow",  0, 0, mode::FILE_600,
        include_bytes!("../etc/shadow")),
    RamEntry::file("/etc/motd",    0, 0, mode::FILE_644,
        b"Welcome to SunlightOS\n"),
    RamEntry::file("/etc/hostname",0, 0, mode::FILE_644,
        b"sunlight\n"),
    RamEntry::file("/etc/sunlight/session.toml", 0, 0, mode::FILE_644,
        br#"
[default]
mode = "terminal"

[terminal]
shell = "/bin/sh"
initial_tabs = 1
theme = "sunlight-dark"

[multi_user]
enabled = false
max_ttys = 6
"#),
    RamEntry::file("/bin/sh",      0, 0, mode::FILE_755,
        b"#!/sunlight/builtin-sh\n"),
];

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_ENTRIES: &[RamEntry] = &[
        RamEntry::file("/etc/motd", 0, 0, mode::FILE_644, b"Welcome to SunlightOS\n"),
        RamEntry::file("/bin/sh",   0, 0, mode::FILE_755, b"shell"),
    ];

    #[test]
    fn open_and_read_whole_file() {
        let mut fs = RamFs::new(TEST_ENTRIES);
        let handle = fs.open("/etc/motd").unwrap();
        let mut buf = [0u8; 32];

        let read = fs.read(handle, 0, &mut buf).unwrap();

        assert_eq!(read, b"Welcome to SunlightOS\n".len());
        assert_eq!(&buf[..read], b"Welcome to SunlightOS\n");
    }

    #[test]
    fn read_respects_offset_and_buffer_size() {
        let mut fs = RamFs::new(TEST_ENTRIES);
        let handle = fs.open("/etc/motd").unwrap();
        let mut buf = [0u8; 8];

        let read = fs.read(handle, 11, &mut buf).unwrap();

        assert_eq!(read, 8);
        assert_eq!(&buf, b"Sunlight");
    }

    #[test]
    fn read_past_end_returns_zero() {
        let mut fs = RamFs::new(TEST_ENTRIES);
        let handle = fs.open("/bin/sh").unwrap();
        let mut buf = [0u8; 4];

        assert_eq!(fs.read(handle, 99, &mut buf), Ok(0));
    }

    #[test]
    fn stat_reports_file_size_and_permissions() {
        let mut fs = RamFs::new(TEST_ENTRIES);

        assert_eq!(
            fs.stat("/bin/sh"),
            Ok(FileStat {
                file_type: FileType::File,
                size: 5,
                uid: 0,
                gid: 0,
                mode: mode::FILE_755,
                nlinks: 1,
            })
        );
    }

    #[test]
    fn missing_file_returns_not_found() {
        let mut fs = RamFs::new(TEST_ENTRIES);

        assert_eq!(fs.open("/missing"), Err(FsError::NotFound));
    }

    #[test]
    fn invalid_path_returns_invalid_path() {
        let mut fs = RamFs::new(TEST_ENTRIES);

        assert_eq!(fs.open("etc/motd"), Err(FsError::InvalidPath));
    }

    #[test]
    fn close_rejects_stale_handle() {
        let mut fs = RamFs::new(TEST_ENTRIES);
        let handle = fs.open("/bin/sh").unwrap();

        assert_eq!(fs.close(handle), Ok(()));
        assert_eq!(fs.close(handle), Err(FsError::BadHandle));
    }

    #[test]
    fn too_many_open_files_is_reported() {
        let mut fs = RamFs::new(TEST_ENTRIES);
        for _ in 0..RAMFS_MAX_HANDLES {
            fs.open("/bin/sh").unwrap();
        }

        assert_eq!(fs.open("/bin/sh"), Err(FsError::TooManyOpenFiles));
    }

    static DIR_ENTRIES: &[RamEntry] = &[
        RamEntry::dir("/", 0, 0, mode::DIR_755),
        RamEntry::dir("/etc", 0, 0, mode::DIR_755),
        RamEntry::file("/etc/motd", 0, 0, mode::FILE_644, b"hello\n"),
    ];

    #[test]
    fn open_dir_returns_isdir() {
        let mut fs = RamFs::new(DIR_ENTRIES);
        assert_eq!(fs.open("/etc"), Err(FsError::IsDir));
    }

    #[test]
    fn stat_dir_returns_directory_type() {
        let mut fs = RamFs::new(DIR_ENTRIES);
        let stat = fs.stat("/etc").unwrap();
        assert_eq!(stat.file_type, FileType::Directory);
        assert_eq!(stat.mode, mode::DIR_755);
        assert_eq!(stat.nlinks, 2);
    }
}
