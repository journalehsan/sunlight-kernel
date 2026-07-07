use crate::vfs::{mode, FileHandle, FileStat, FileSystem, FileType, VfsDirEntry};
use crate::{path, FsError};
use alloc::vec::Vec;

pub const RAMFS_MAX_HANDLES: usize = 32;
pub const RAMFS_MAX_ENTRIES: usize = 512;

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

    fn parent_is_dir(&self, path: &str) -> Result<bool, FsError> {
        let parent = parent_path(path)?;
        if parent == "/" {
            return Ok(true);
        }
        let idx = self.entry_idx(parent)?;
        Ok(self.is_dir(idx))
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

    fn create_file(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<FileHandle, FsError> {
        path::validate_absolute(path)?;
        if self.entry_idx(path).is_ok() {
            return self.open(path);
        }
        if !self.parent_is_dir(path)? {
            return Err(FsError::NotDir);
        }
        self.dynamic.push(DynamicEntry {
            path: Vec::from(path.as_bytes()),
            data: Vec::new(),
            uid,
            gid,
            mode: mode::S_IFREG | mode,
            is_dir: false,
        });
        self.alloc_handle(self.all_entry_count() - 1)
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

    fn fstat_handle(&mut self, handle: FileHandle) -> Result<FileStat, FsError> {
        let idx = self.handle_entry_idx(handle)?;
        let ft = if self.is_dir(idx) {
            FileType::Directory
        } else {
            FileType::File
        };
        Ok(FileStat {
            file_type: ft,
            size: if ft == FileType::Directory {
                0
            } else {
                self.entry_data_len(idx)
            },
            uid: self.entry_uid(idx),
            gid: self.entry_gid(idx),
            mode: self.entry_mode(idx),
            nlinks: if ft == FileType::Directory { 2 } else { 1 },
        })
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
        if !self.parent_is_dir(path)? {
            return Err(FsError::NotDir);
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

    fn unlink(&mut self, path: &str) -> Result<(), FsError> {
        path::validate_absolute(path)?;
        let entry_idx = self.entry_idx(path)?;
        if self.is_dir(entry_idx) {
            return Err(FsError::IsDir);
        }
        if entry_idx < self.entries.len() {
            return Err(FsError::ReadOnlyFilesystem);
        }
        let dyn_idx = entry_idx - self.entries.len();
        // Invalidate any open handles pointing at this entry or at entries
        // that shifted past it.
        for slot in self.handles.iter_mut() {
            if let Some(h) = *slot {
                if h == entry_idx {
                    *slot = None;
                } else if h > entry_idx {
                    *slot = Some(h - 1);
                }
            }
        }
        self.dynamic.remove(dyn_idx);
        Ok(())
    }

    fn rename(&mut self, old: &str, new: &str) -> Result<(), FsError> {
        path::validate_absolute(old)?;
        path::validate_absolute(new)?;
        let entry_idx = self.entry_idx(old)?;
        if entry_idx < self.entries.len() {
            return Err(FsError::ReadOnlyFilesystem);
        }
        let dyn_idx = entry_idx - self.entries.len();
        // If destination exists (and is a file), remove it first.
        if let Ok(dst_idx) = self.entry_idx(new) {
            if self.is_dir(dst_idx) {
                return Err(FsError::IsDir);
            }
            if dst_idx >= self.entries.len() {
                let ddyn = dst_idx - self.entries.len();
                for slot in self.handles.iter_mut() {
                    if let Some(h) = *slot {
                        if h == dst_idx {
                            *slot = None;
                        } else if h > dst_idx {
                            *slot = Some(h - 1);
                        }
                    }
                }
                self.dynamic.remove(ddyn);
                // dyn_idx may have shifted if dst came before src in the vec
                let dyn_idx = self.entry_idx(old)? - self.entries.len();
                self.dynamic[dyn_idx].path = Vec::from(new.as_bytes());
                return Ok(());
            }
            return Err(FsError::ReadOnlyFilesystem);
        }
        self.dynamic[dyn_idx].path = Vec::from(new.as_bytes());
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

/// The conventional per-user directory structure that the OS guarantees to
/// exist inside every home directory (`/root` and each `/home/<user>`).
///
/// This is an OS-level responsibility: the kernel/filesystem seeds it at boot
/// for `/root` and the default user (see `INITRAMFS`), and the user-creation
/// path (`sunshell` `useradd`) reproduces it for every new account. The file
/// manager (`sunlight-files`) deliberately does NOT create these; it only
/// assumes they exist and surfaces an error if navigation fails.
pub const STANDARD_HOME_DIRS: &[&str] = &[
    "Desktop",
    "Documents",
    "Downloads",
    "Pictures",
    "Music",
    "Videos",
];

pub static INITRAMFS: &[RamEntry] = &[
    // Directories
    RamEntry::dir("/", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc/sunlight", 0, 0, mode::DIR_755),
    RamEntry::dir("/bin", 0, 0, mode::DIR_755),

    // -- Standard user home directory layout (OS responsibility) ------------
    // These directories are seeded by the OS so every home directory ships
    // with the conventional folder structure. This is intentionally done here
    // (and in the user-creation path), NOT in the file manager: the file
    // manager only navigates existing folders and must never be responsible
    // for creating the standard home layout.
    //
    // Keep this list in sync with `STANDARD_HOME_DIRS` below.
    RamEntry::dir("/root", 0, 0, mode::DIR_700),
    // Root's standard folders (uid 0).
    RamEntry::dir("/root/Desktop", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/Documents", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/Downloads", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/Pictures", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/Music", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/Videos", 0, 0, mode::DIR_700),

    RamEntry::dir("/home", 0, 0, mode::DIR_755),
    RamEntry::dir("/home/user", 1000, 1000, mode::DIR_755),
    // Default unprivileged user's standard folders (uid/gid 1000).
    RamEntry::dir("/home/user/Desktop", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Documents", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Downloads", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Pictures", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Music", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Videos", 1000, 1000, mode::DIR_755),
    // -- End standard home directory layout ---------------------------------

    RamEntry::dir("/tmp", 0, 0, mode::DIR_1777),
    RamEntry::dir("/run", 0, 0, mode::DIR_755),
    RamEntry::dir("/state", 0, 0, mode::DIR_755),
    RamEntry::dir("/state/sunlight-kv", 0, 0, mode::DIR_700),
    RamEntry::dir("/state/sunlight-tls", 0, 0, mode::DIR_700),
    RamEntry::dir("/state/sunlight-uac", 0, 0, mode::DIR_700),
    RamEntry::dir("/state/capability-broker", 0, 0, mode::DIR_700),
    RamEntry::dir("/srv", 0, 0, mode::DIR_755),
    RamEntry::dir("/srv/http", 0, 0, mode::DIR_755),
    RamEntry::dir("/var", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/lib", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/log", 0, 0, mode::DIR_755),
    RamEntry::dir("/system", 0, 0, mode::DIR_755),
    RamEntry::dir("/system/share", 0, 0, mode::DIR_755),
    RamEntry::dir("/system/share/wallpapers", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/wallpapers", 0, 0, mode::DIR_755),
    // SunlightOS icon theme directory structure.
    // Icons are embedded directly in service binaries (same approach as the wallpaper).
    // This directory tree is provided so future VFS-based icon loading works at:
    //   /var/sunlightos/icons/SunlightOS/{category}/{size}/{name}.tga
    RamEntry::dir("/var/sunlightos/icons", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/apps", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/places", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/mimetypes", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/devices", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/actions", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/status", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/preferences", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/system/share/wallpapers/default.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper.tga"),
    ),
    RamEntry::file(
        "/system/share/wallpapers/dark.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/sunlight-login-background.tga"),
    ),
    // Vortex Shell desktop wallpaper (TGA type-2, 1672×941, 24 bpp BGR).
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper.tga"),
    ),
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
    // Large file for shared-memory IPC read test (>48 bytes triggers shm path)
    RamEntry::file("/etc/large_test", 0, 0, mode::FILE_644, &[b'A'; 2048]),
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
    // /etc/hosts for DNS resolver (hosts first, then hardcoded fallback)
    RamEntry::file(
        "/etc/hosts",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../etc/hosts"),
    ),
    RamEntry::file(
        "/etc/resolv.conf",
        0,
        0,
        mode::FILE_644,
        b"# Generated by SunlightOS resolved.\n# Do not edit this file directly unless you know what you are doing.\nnameserver 208.67.222.222\nnameserver 208.67.220.220\n",
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
    RamEntry::file(
        "/srv/http/index.html",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/solar.html"),
    ),
    RamEntry::file("/bin/sshl", 0, 0, mode::FILE_755, b"#!/sunlight/sunshell\n"),
    RamEntry::file("/bin/sh", 0, 0, mode::FILE_755, b"#!/sunlight/builtin-sh\n"),
    // POSIX-style applet stubs in /bin so PATH resolution behaves like a
    // normal Unix layout. Kernel spawn maps these paths to the embedded
    // sunlight-utils / sunlight-net-utils multi-call ELFs.
    RamEntry::file(
        "/bin/ls",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/cat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/cp",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/mv",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/rm",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/mkdir",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/rmdir",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/touch",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/find",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/grep",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/head",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/tail",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/wc",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/sort",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/uniq",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/cut",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/file",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/stat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/pwd",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/date",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/whoami",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/id",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/uname",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/echo",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/nice",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/renice",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/free",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/freezram",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/kill",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/killall",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/pkill",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file("/bin/top", 0, 0, mode::FILE_755, b"#!/sunlight/top\n"),
    RamEntry::file(
        "/bin/sunlightctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlightctl\n",
    ),
    RamEntry::file(
        "/bin/devicectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/devicectl\n",
    ),
    RamEntry::file(
        "/bin/networkctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/networkctl\n",
    ),
    RamEntry::file(
        "/bin/resolvectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/resolvectl\n",
    ),
    RamEntry::file(
        "/bin/powerctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/powerctl\n",
    ),
    RamEntry::file(
        "/bin/nicectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/nicectl\n",
    ),
    RamEntry::file(
        "/bin/capabilityctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/capabilityctl\n",
    ),
    RamEntry::file("/bin/runas", 0, 0, mode::FILE_755, b"#!/sunlight/runas\n"),
    RamEntry::file("/bin/fetch", 0, 0, mode::FILE_755, b"#!/sunlight/fetch\n"),
    RamEntry::file(
        "/bin/sunlight-sunsay",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-sunsay\n",
    ),
    RamEntry::file("/bin/z", 0, 0, mode::FILE_755, b"#!/sunlight/z\n"),
    RamEntry::file("/bin/dict", 0, 0, mode::FILE_755, b"#!/sunlight/dict\n"),
    RamEntry::file(
        "/bin/hangman",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/hangman\n",
    ),
    // GUI Eyes Tracker demo client
    RamEntry::file("/bin/eyes", 0, 0, mode::FILE_755, b"#!/sunlight/eyes\n"),
    RamEntry::file("/bin/sunlight-runner", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-runner\n"),
    RamEntry::file("/bin/sun-exec", 0, 0, mode::FILE_755, b"#!/sunlight/sun-exec\n"),
    RamEntry::file("/bin/sun-open", 0, 0, mode::FILE_755, b"#!/sunlight/sun-open\n"),
    // GUI Terminal emulator
    RamEntry::file("/bin/sunlight-terminal", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-terminal\n"),
    // GUI Task Monitor
    RamEntry::file("/bin/sunlight-tasks", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-tasks\n"),
    // SunLight-Bench: CPU/multi-core performance benchmark
    RamEntry::file("/bin/sunbench", 0, 0, mode::FILE_755, b"#!/sunlight/sunbench\n"),
    // GUI calculator client
    RamEntry::file("/bin/calculator", 0, 0, mode::FILE_755, b"#!/sunlight/calculator\n"),
    // GUI file manager client
    RamEntry::file(
        "/bin/sunlight-files",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-files\n",
    ),
    RamEntry::file(
        "/bin/light-lens",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/light-lens\n",
    ),
    RamEntry::file(
        "/bin/sunlight-edit",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-edit\n",
    ),
    RamEntry::file(
        "/bin/sunlight-text",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-text\n",
    ),
    // System Preferences (Control Panel)
    RamEntry::file("/bin/control-panel", 0, 0, mode::FILE_755, b"#!/sunlight/control-panel\n"),
    RamEntry::file(
        "/bin/cpufeat",
        0,
        0,
        mode::FILE_755,
        b"#!/cpu-utils/cpufeat\n",
    ),
    RamEntry::file(
        "/bin/hello-linux",
        0,
        0,
        mode::FILE_755,
        b"#!/helios/hello-linux\n",
    ),
    RamEntry::file(
        "/bin/note",
        0,
        0,
        mode::FILE_755,
        b"#!/helios/note\n",
    ),
    RamEntry::file(
        "/bin/sunlight-kvctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-kvctl\n",
    ),
    RamEntry::file(
        "/bin/certificatectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/certificatectl\n",
    ),
    RamEntry::file(
        "/bin/sunlight-clip",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-clip\n",
    ),
    RamEntry::file(
        "/bin/sunlight-clipman",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-clipman\n",
    ),
    // sunlight-sm binary stub (real ELF embedded in kernel for spawn; entry for FS visibility/stat)
    RamEntry::file(
        "/sbin/sunlight-sm",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-sm\n",
    ),
    RamEntry::file(
        "/bin/ping",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/ifconfig",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/wget",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/curl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/dig",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/nslookup",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/hostname",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/netstat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/ss",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/traceroute",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/arp",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/dhclient",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    // Backward-compat applet paths kept for existing docs/scripts.
    RamEntry::dir("/sunlight-utils", 0, 0, mode::DIR_755),
    RamEntry::dir("/sunlight-net-utils", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/sunlight-utils/ls",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/cat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/cp",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/mv",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/rm",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/mkdir",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/rmdir",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/touch",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/find",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/grep",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/head",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/tail",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/wc",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/sort",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/uniq",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/cut",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/file",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/stat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/pwd",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/date",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/whoami",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/id",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/uname",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/nice",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/renice",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/free",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/freezram",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/ping",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/ifconfig",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/wget",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/curl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/dig",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/nslookup",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/hostname",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/netstat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    // timezone refactor support files (Phase TZ)
    RamEntry::dir("/etc/zoneinfo", 0, 0, mode::DIR_755),
    RamEntry::dir("/usr/share/zoneinfo", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc/sunlight/services", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/etc/localtime",
        0,
        0,
        mode::FILE_644,
        b"{\
\"id\":\"UTC\",\
\"display_name\":\"Coordinated Universal Time\",\
\"utc_offset_hours\":0,\
\"utc_offset_minutes\":0,\
\"dst_offset_minutes\":0,\
\"dst_start_month\":0,\
\"dst_end_month\":0\
}",
    ),
    RamEntry::file(
        "/etc/zoneinfo/Asia/Tehran.txt",
        0,
        0,
        mode::FILE_644,
        b"id=Asia/Tehran\ndisplay_name=Iran Standard Time\n\
utc_offset=+03:30\ndst=none (abolished 2022)\n",
    ),
    RamEntry::file(
        "/etc/zoneinfo/UTC.txt",
        0,
        0,
        mode::FILE_644,
        b"id=UTC\ndisplay_name=Coordinated Universal Time\n\
utc_offset=+00:00\ndst=none\n",
    ),
    RamEntry::file(
        "/usr/share/zoneinfo/zones.csv",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/Timezones.csv"),
    ),
    RamEntry::file(
        "/etc/sunlight/services/timezone.service",
        0,
        0,
        mode::FILE_644,
        b"[Unit]\nDescription=SunlightOS Timezone Service\n\
After=vfs.service\nRequires=vfs.service\n\n\
[Service]\nType=simple\nExecStart=/usr/sbin/timezone_service\n\
Restart=on-failure\nRestartSec=3\nUser=root\n\
StandardOutput=journal\nStandardError=journal\n\n\
[Install]\nWantedBy=sunlight.target\n",
    ),
    // tzctl client (standalone binary path + shell builtin alias)
    RamEntry::file(
        "/usr/bin/tzctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/tzctl\n",
    ),
    RamEntry::file("/usr/bin/top", 0, 0, mode::FILE_755, b"#!/sunlight/top\n"),
    RamEntry::file(
        "/usr/bin/devicectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/devicectl\n",
    ),
    RamEntry::file(
        "/usr/bin/networkctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/networkctl\n",
    ),
    RamEntry::file(
        "/usr/bin/resolvectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/resolvectl\n",
    ),
    RamEntry::file(
        "/usr/bin/powerctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/powerctl\n",
    ),
    RamEntry::file(
        "/usr/bin/fetch",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/fetch\n",
    ),
    RamEntry::file(
        "/usr/bin/hangman",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/hangman\n",
    ),
    // GUI Eyes Tracker demo client (usr path)
    RamEntry::file("/usr/bin/eyes", 0, 0, mode::FILE_755, b"#!/sunlight/eyes\n"),
    RamEntry::file(
        "/usr/bin/sunlight-runner",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-runner\n",
    ),
    RamEntry::file(
        "/usr/bin/sun-exec",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sun-exec\n",
    ),
    RamEntry::file(
        "/usr/bin/sun-open",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sun-open\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-terminal",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-terminal\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-tasks",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-tasks\n",
    ),
    RamEntry::file(
        "/usr/bin/sunbench",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunbench\n",
    ),
    RamEntry::file(
        "/usr/bin/calculator",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/calculator\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-files",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-files\n",
    ),
    RamEntry::file(
        "/usr/bin/light-lens",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/light-lens\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-edit",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-edit\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-text",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-text\n",
    ),
    // System Preferences (Control Panel)
    RamEntry::file(
        "/usr/bin/control-panel",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/control-panel\n",
    ),
    RamEntry::file(
        "/usr/bin/cpufeat",
        0,
        0,
        mode::FILE_755,
        b"#!/cpu-utils/cpufeat\n",
    ),
    RamEntry::file(
        "/usr/bin/hello-linux",
        0,
        0,
        mode::FILE_755,
        b"#!/helios/hello-linux\n",
    ),
    RamEntry::file(
        "/usr/bin/note",
        0,
        0,
        mode::FILE_755,
        b"#!/helios/note\n",
    ),
    // sunlight-thumbd: asynchronous thumbnail daemon.
    RamEntry::file(
        "/usr/bin/sunlight-thumbd",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-thumbd\n",
    ),
    RamEntry::file(
        "/sbin/sunlight-thumbd",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-thumbd\n",
    ),
    // ── Sample pictures ────────────────────────────────────────────────────
    // Installed under /usr/share/sunlightos/sample-pictures/ (system-wide).
    // Also visible to users under ~/Pictures/Sample Pictures/ (real directory,
    // same file data — no extra memory cost since data pointers are shared).
    RamEntry::dir("/usr/share", 0, 0, mode::DIR_755),
    RamEntry::dir("/usr/share/sunlightos", 0, 0, mode::DIR_755),
    // Login background image (TGA type-2).  The tty_server also embeds this
    // image at compile time; the VFS path is provided for other consumers.
    RamEntry::dir("/usr/share/sunlightos/backgrounds", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/usr/share/sunlightos/backgrounds/login-background.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/sunlight-login-background.tga"),
    ),
    RamEntry::dir("/usr/share/sunlightos/sample-pictures", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/01_solar_blossom.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/01_solar_blossom.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/02_amber_dunes.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/02_amber_dunes.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/03_blue_garden.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/03_blue_garden.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/04_lantern_jellyfish.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/04_lantern_jellyfish.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/05_sleepy_koala.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/05_sleepy_koala.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/06_sunrise_lighthouse.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/06_sunrise_lighthouse.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/07_penguin_walk.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/07_penguin_walk.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/08_orange_tulips.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/08_orange_tulips.simg"),
    ),
    // ~/Pictures/Sample Pictures/ for root (same file data, separate dir entry).
    RamEntry::dir("/root/Pictures/Sample Pictures", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/01_solar_blossom.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/01_solar_blossom.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/02_amber_dunes.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/02_amber_dunes.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/03_blue_garden.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/03_blue_garden.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/04_lantern_jellyfish.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/04_lantern_jellyfish.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/05_sleepy_koala.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/05_sleepy_koala.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/06_sunrise_lighthouse.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/06_sunrise_lighthouse.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/07_penguin_walk.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/07_penguin_walk.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/08_orange_tulips.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/08_orange_tulips.simg"),
    ),
    // ~/Pictures/Sample Pictures/ for the default unprivileged user.
    RamEntry::dir("/home/user/Pictures/Sample Pictures", 1000, 1000, mode::DIR_755),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/01_solar_blossom.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/01_solar_blossom.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/02_amber_dunes.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/02_amber_dunes.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/03_blue_garden.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/03_blue_garden.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/04_lantern_jellyfish.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/04_lantern_jellyfish.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/05_sleepy_koala.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/05_sleepy_koala.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/06_sunrise_lighthouse.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/06_sunrise_lighthouse.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/07_penguin_walk.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/07_penguin_walk.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/08_orange_tulips.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/08_orange_tulips.simg"),
    ),
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
    fn create_file_creates_and_opens_file() {
        let mut fs = RamFs::new(DIR_ENTRIES);
        let handle = fs.create_file("/etc/new", 1000, 1000, 0o644).unwrap();
        assert_eq!(fs.write(handle, 0, b"ok"), Ok(2));
        let stat = fs.stat("/etc/new").unwrap();
        assert_eq!(stat.file_type, FileType::File);
        assert_eq!(stat.uid, 1000);
        assert_eq!(stat.mode, mode::S_IFREG | 0o644);
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
