use crate::vfs::{mode, FileHandle, FileStat, FileSystem, FileType, VfsDirEntry};
use crate::{path, FsError};
use alloc::vec::Vec;

pub const RAMFS_MAX_HANDLES: usize = 32;
pub const RAMFS_MAX_ENTRIES: usize = 128;

pub struct RamEntry {
    pub path: &'static str,
    pub data: &'static [u8],
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
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
        Self {
            path,
            data,
            uid,
            gid,
            mode: file_mode,
            is_dir: false,
        }
    }

    pub const fn dir(path: &'static str, uid: u32, gid: u32, dir_mode: u16) -> Self {
        Self {
            path,
            data: b"",
            uid,
            gid,
            mode: dir_mode,
            is_dir: true,
        }
    }
}

/// A dynamic entry created at runtime (e.g., by mkdir or write).
struct DynamicEntry {
    path: Vec<u8>,
    data: Vec<u8>,
    uid: u32,
    gid: u32,
    mode: u16,
    is_dir: bool,
}

pub struct RamFs {
    entries: &'static [RamEntry],
    handles: [Option<usize>; RAMFS_MAX_HANDLES],
    /// Mutable data copies for static entries. Indexed by entry index.
    buffers: [Option<Vec<u8>>; RAMFS_MAX_ENTRIES],
    /// Dynamic entries created at runtime.
    dynamic: Vec<DynamicEntry>,
}

impl RamFs {
    pub fn new(entries: &'static [RamEntry]) -> Self {
        Self {
            entries,
            handles: [None; RAMFS_MAX_HANDLES],
            buffers: [const { None }; RAMFS_MAX_ENTRIES],
            dynamic: Vec::new(),
        }
    }

    fn all_entry_count(&self) -> usize {
        self.entries.len() + self.dynamic.len()
    }

    fn entry_idx(&self, path: &str) -> Result<usize, FsError> {
        path::validate_absolute(path)?;
        if let Some(idx) = self.entries.iter().position(|e| e.path == path) {
            return Ok(idx);
        }
        if let Some(idx) = self
            .dynamic
            .iter()
            .position(|e| core::str::from_utf8(&e.path).ok() == Some(path))
        {
            return Ok(self.entries.len() + idx);
        }
        Err(FsError::NotFound)
    }

    fn is_dir(&self, idx: usize) -> bool {
        if idx < self.entries.len() {
            self.entries[idx].is_dir
        } else {
            self.dynamic[idx - self.entries.len()].is_dir
        }
    }

    fn entry_mode(&self, idx: usize) -> u16 {
        if idx < self.entries.len() {
            self.entries[idx].mode
        } else {
            self.dynamic[idx - self.entries.len()].mode
        }
    }

    fn entry_uid(&self, idx: usize) -> u32 {
        if idx < self.entries.len() {
            self.entries[idx].uid
        } else {
            self.dynamic[idx - self.entries.len()].uid
        }
    }

    fn entry_gid(&self, idx: usize) -> u32 {
        if idx < self.entries.len() {
            self.entries[idx].gid
        } else {
            self.dynamic[idx - self.entries.len()].gid
        }
    }

    fn entry_data(&self, idx: usize) -> &[u8] {
        if idx < self.entries.len() {
            self.buffers[idx]
                .as_deref()
                .unwrap_or(self.entries[idx].data)
        } else {
            &self.dynamic[idx - self.entries.len()].data
        }
    }

    fn entry_data_len(&self, idx: usize) -> usize {
        if idx < self.entries.len() {
            self.buffers[idx]
                .as_ref()
                .map(|v| v.len())
                .unwrap_or(self.entries[idx].data.len())
        } else {
            self.dynamic[idx - self.entries.len()].data.len()
        }
    }

    fn set_entry_data(&mut self, idx: usize, data: Vec<u8>) {
        if idx < self.entries.len() {
            self.buffers[idx] = Some(data);
        } else {
            self.dynamic[idx - self.entries.len()].data = data;
        }
    }

    fn set_entry_mode(&mut self, idx: usize, mode: u16) {
        if idx < self.entries.len() {
            // Static entries are immutable for mode; ignore or we could add a buffer for metadata
        } else {
            self.dynamic[idx - self.entries.len()].mode = mode;
        }
    }

    fn set_entry_owner(&mut self, idx: usize, uid: u32, gid: u32) {
        if idx < self.entries.len() {
            // Static entries are immutable for owner
        } else {
            self.dynamic[idx - self.entries.len()].uid = uid;
            self.dynamic[idx - self.entries.len()].gid = gid;
        }
    }

    fn entry_path(&self, idx: usize) -> Option<&str> {
        if idx < self.entries.len() {
            Some(self.entries[idx].path)
        } else {
            core::str::from_utf8(&self.dynamic[idx - self.entries.len()].path).ok()
        }
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
        if self.is_dir(entry_idx) {
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
        let data = self.entry_data(entry_idx);
        if offset >= data.len() {
            return Ok(0);
        }
        let src = &data[offset..];
        let len = src.len().min(buf.len());
        buf[..len].copy_from_slice(&src[..len]);
        Ok(len)
    }

    fn write(&mut self, handle: FileHandle, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        let entry_idx = self.handle_entry_idx(handle)?;
        let current = self.entry_data(entry_idx);
        let mut new_data = Vec::new();
        if offset <= current.len() {
            new_data.extend_from_slice(&current[..offset]);
        } else {
            new_data.extend_from_slice(current);
            new_data.resize(offset, 0);
        }
        let end = offset + buf.len();
        if end > new_data.len() {
            new_data.resize(end, 0);
        }
        new_data[offset..end].copy_from_slice(buf);
        self.set_entry_data(entry_idx, new_data);
        Ok(buf.len())
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
        let ft = if self.is_dir(entry_idx) {
            FileType::Directory
        } else {
            FileType::File
        };
        let size = if ft == FileType::Directory {
            0
        } else {
            self.entry_data_len(entry_idx)
        };
        let nlinks = if ft == FileType::Directory { 2 } else { 1 };
        Ok(FileStat {
            file_type: ft,
            size,
            uid: self.entry_uid(entry_idx),
            gid: self.entry_gid(entry_idx),
            mode: self.entry_mode(entry_idx),
            nlinks,
        })
    }

    fn mkdir(&mut self, path: &str, uid: u32, gid: u32, mode: u16) -> Result<(), FsError> {
        path::validate_absolute(path)?;
        if self.entry_idx(path).is_ok() {
            return Err(FsError::InvalidPath);
        }
        self.dynamic.push(DynamicEntry {
            path: Vec::from(path.as_bytes()),
            data: Vec::new(),
            uid,
            gid,
            mode: mode::S_IFDIR | mode,
            is_dir: true,
        });
        Ok(())
    }

    fn chmod(&mut self, path: &str, mode: u16) -> Result<(), FsError> {
        let entry_idx = self.entry_idx(path)?;
        self.set_entry_mode(entry_idx, mode);
        Ok(())
    }

    fn chown(&mut self, path: &str, uid: u32, gid: u32) -> Result<(), FsError> {
        let entry_idx = self.entry_idx(path)?;
        self.set_entry_owner(entry_idx, uid, gid);
        Ok(())
    }

    fn read_dir(
        &mut self,
        path: &str,
        f: &mut dyn FnMut(&VfsDirEntry) -> bool,
    ) -> Result<(), FsError> {
        path::validate_absolute(path)?;
        if path != "/" {
            let idx = self.entry_idx(path)?;
            if !self.is_dir(idx) {
                return Err(FsError::NotDir);
            }
        }
        for idx in 0..self.all_entry_count() {
            let Some(entry_path) = self.entry_path(idx) else {
                continue;
            };
            let Some(name) = direct_child_name(entry_path, path) else {
                continue;
            };
            let entry = if self.is_dir(idx) {
                VfsDirEntry::from_bytes(name.as_bytes(), FileType::Directory, 0)
            } else {
                VfsDirEntry::from_bytes(name.as_bytes(), FileType::File, self.entry_data_len(idx))
            };
            if !f(&entry) {
                break;
            }
        }
        Ok(())
    }
}

/// If `entry_path` names a direct child of directory `dir`, return its name.
fn direct_child_name<'a>(entry_path: &'a str, dir: &str) -> Option<&'a str> {
    let rest = if dir == "/" {
        entry_path.strip_prefix('/')?
    } else {
        entry_path.strip_prefix(dir)?.strip_prefix('/')?
    };
    if rest.is_empty() || rest.contains('/') {
        None
    } else {
        Some(rest)
    }
}

pub static INITRAMFS: &[RamEntry] = &[
    // Directories
    RamEntry::dir("/", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc/sunlight", 0, 0, mode::DIR_755),
    RamEntry::dir("/bin", 0, 0, mode::DIR_755),
    RamEntry::dir("/root", 0, 0, mode::DIR_700),
    RamEntry::dir("/home", 0, 0, mode::DIR_755),
    RamEntry::dir("/tmp", 0, 0, mode::DIR_1777),
    RamEntry::dir("/var", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/log", 0, 0, mode::DIR_755),
    // System config files (world-readable)
    RamEntry::file(
        "/etc/passwd",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../etc/passwd"),
    ),
    RamEntry::file(
        "/etc/group",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../etc/group"),
    ),
    RamEntry::file(
        "/etc/shadow",
        0,
        0,
        mode::FILE_600,
        include_bytes!("../etc/shadow"),
    ),
    RamEntry::file(
        "/etc/motd",
        0,
        0,
        mode::FILE_644,
        b"Welcome to SunlightOS\n",
    ),
    RamEntry::file("/etc/hostname", 0, 0, mode::FILE_644, b"sunlight\n"),
    RamEntry::file(
        "/etc/fstab",
        0,
        0,
        mode::FILE_644,
        b"# device    mountpoint   type         options\n\
/dev/sda1   /boot        bootfs       defaults\n\
/dev/ram0   /            ramfs        defaults\n",
    ),
    RamEntry::file(
        "/etc/sunlight/session.toml",
        0,
        0,
        mode::FILE_644,
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
"#,
    ),
    RamEntry::file("/bin/sshl", 0, 0, mode::FILE_755, b"#!/sunlight/sunshell\n"),
    RamEntry::file("/bin/sh", 0, 0, mode::FILE_755, b"#!/sunlight/builtin-sh\n"),
    // Phase 6.5 Step 2: applet stubs so $PATH resolution can find the
    // sunlight-utils / sunlight-net-utils binaries. Step 3 replaces these
    // with real ELF loading from the VFS.
    RamEntry::dir("/sunlight-utils", 0, 0, mode::DIR_755),
    RamEntry::dir("/sunlight-net-utils", 0, 0, mode::DIR_755),
    RamEntry::file("/sunlight-utils/ls", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/cat", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/cp", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/mv", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/rm", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/mkdir", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/rmdir", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/touch", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/find", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/grep", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/head", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/tail", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/wc", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/sort", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/uniq", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/cut", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/file", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/stat", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/pwd", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/date", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-net-utils/ping", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-net-utils\n"),
    RamEntry::file("/sunlight-net-utils/ifconfig", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-net-utils\n"),
    RamEntry::file("/sunlight-net-utils/wget", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-net-utils\n"),
    RamEntry::file("/sunlight-net-utils/curl", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-net-utils\n"),
    RamEntry::file("/sunlight-net-utils/dig", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-net-utils\n"),
    RamEntry::file("/sunlight-net-utils/nslookup", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-net-utils\n"),
    RamEntry::file("/sunlight-net-utils/hostname", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-net-utils\n"),
    RamEntry::file("/sunlight-net-utils/netstat", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-net-utils\n"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::borrow::ToOwned;

    static TEST_ENTRIES: &[RamEntry] = &[
        RamEntry::file(
            "/etc/motd",
            0,
            0,
            mode::FILE_644,
            b"Welcome to SunlightOS\n",
        ),
        RamEntry::file("/bin/sh", 0, 0, mode::FILE_755, b"shell"),
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

    #[test]
    fn write_extends_file() {
        let mut fs = RamFs::new(TEST_ENTRIES);
        let handle = fs.open("/bin/sh").unwrap();
        assert_eq!(fs.write(handle, 0, b"newdata"), Ok(7));
        let mut buf = [0u8; 16];
        let n = fs.read(handle, 0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"newdata");
    }

    #[test]
    fn mkdir_creates_directory() {
        let mut fs = RamFs::new(TEST_ENTRIES);
        assert_eq!(fs.mkdir("/newdir", 0, 0, 0o755), Ok(()));
        let stat = fs.stat("/newdir").unwrap();
        assert_eq!(stat.file_type, FileType::Directory);
        assert_eq!(stat.mode, mode::S_IFDIR | 0o755);
    }

    #[test]
    fn read_dir_lists_direct_children_only() {
        let mut fs = RamFs::new(DIR_ENTRIES);

        let mut root_names = Vec::new();
        fs.read_dir("/", &mut |entry| {
            root_names.push((entry.name().to_owned(), entry.file_type));
            true
        })
        .unwrap();
        // "/etc/motd" is not a direct child of "/".
        assert_eq!(
            root_names,
            std::vec![("etc".to_owned(), FileType::Directory)]
        );

        let mut etc_names = Vec::new();
        fs.read_dir("/etc", &mut |entry| {
            etc_names.push((entry.name().to_owned(), entry.size));
            true
        })
        .unwrap();
        assert_eq!(etc_names, std::vec![("motd".to_owned(), 6)]);
    }

    #[test]
    fn read_dir_includes_dynamic_entries_and_rejects_files() {
        let mut fs = RamFs::new(DIR_ENTRIES);
        fs.mkdir("/etc/sunlight", 0, 0, 0o755).unwrap();

        let mut names = Vec::new();
        fs.read_dir("/etc", &mut |entry| {
            names.push(entry.name().to_owned());
            true
        })
        .unwrap();
        assert_eq!(names, std::vec!["motd".to_owned(), "sunlight".to_owned()]);

        assert_eq!(
            fs.read_dir("/etc/motd", &mut |_| true),
            Err(FsError::NotDir)
        );
        assert_eq!(
            fs.read_dir("/missing", &mut |_| true),
            Err(FsError::NotFound)
        );
    }
}
