use crate::vfs::{
    make_local_handle, mode, next_handle_generation, split_local_handle, FileHandle, FileStat,
    FileSystem, FileType, VfsDirEntry,
};
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
    handles: [Option<RamOpen>; RAMFS_MAX_HANDLES],
    generations: [u32; RAMFS_MAX_HANDLES],
    /// Mutable data copies for static entries. Indexed by entry index.
    buffers: [Option<Vec<u8>>; RAMFS_MAX_ENTRIES],
    /// Dynamic entries created at runtime.
    dynamic: Vec<DynamicEntry>,
}

#[derive(Clone, Copy)]
struct RamOpen {
    entry_idx: usize,
    generation: u32,
}

impl RamFs {
    pub fn new(entries: &'static [RamEntry]) -> Self {
        Self {
            entries,
            handles: [None; RAMFS_MAX_HANDLES],
            generations: [0; RAMFS_MAX_HANDLES],
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
                let generation = next_handle_generation(self.generations[idx]);
                self.generations[idx] = generation;
                *slot = Some(RamOpen {
                    entry_idx,
                    generation,
                });
                return Ok(make_local_handle(idx, generation));
            }
        }
        Err(FsError::TooManyOpenFiles)
    }

    fn handle_entry_idx(&self, handle: FileHandle) -> Result<usize, FsError> {
        let (idx, generation) = split_local_handle(handle)?;
        self.handles
            .get(idx)
            .and_then(|slot| slot.filter(|open| open.generation == generation))
            .map(|open| open.entry_idx)
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

    fn create_file_exclusive(
        &mut self,
        path: &str,
        uid: u32,
        gid: u32,
        mode: u16,
    ) -> Result<FileHandle, FsError> {
        path::validate_absolute(path)?;
        if self.entry_idx(path).is_ok() {
            return Err(FsError::AlreadyExists);
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
        let end = offset.checked_add(buf.len()).ok_or(FsError::Io)?;
        if end > new_data.len() {
            new_data.resize(end, 0);
        }
        new_data[offset..end].copy_from_slice(buf);
        self.set_entry_data(entry_idx, new_data);
        Ok(buf.len())
    }

    fn truncate(&mut self, handle: FileHandle) -> Result<(), FsError> {
        let entry_idx = self.handle_entry_idx(handle)?;
        self.set_entry_data(entry_idx, Vec::new());
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
            if let Some(open) = slot.as_mut() {
                if open.entry_idx == entry_idx {
                    *slot = None;
                } else if open.entry_idx > entry_idx {
                    open.entry_idx -= 1;
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
            if dst_idx < self.entries.len() {
                let source = self.dynamic.remove(dyn_idx);
                self.buffers[dst_idx] = Some(source.data);
                for slot in self.handles.iter_mut() {
                    if let Some(open) = slot.as_mut() {
                        if open.entry_idx == entry_idx {
                            open.entry_idx = dst_idx;
                        } else if open.entry_idx > entry_idx {
                            open.entry_idx -= 1;
                        }
                    }
                }
                return Ok(());
            }
            if dst_idx >= self.entries.len() {
                let ddyn = dst_idx - self.entries.len();
                for slot in self.handles.iter_mut() {
                    if let Some(open) = slot.as_mut() {
                        if open.entry_idx == dst_idx {
                            *slot = None;
                        } else if open.entry_idx > dst_idx {
                            open.entry_idx -= 1;
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

static CAT_BIG_BYTES: [u8; 513] = [b'x'; 513];

static HEAD_MULTILINE: &[u8] = b"line1\nline2\nline3\nline4\nline5\n\
line6\nline7\nline8\nline9\nline10\n\
line11\nline12\nline13\nline14\nline15\n\
line16\nline17\nline18\nline19\nline20\n";

static HEAD_ONELINE: &[u8] = b"just one line\n";

static CMP_IDENTICAL: &[u8] = b"hello world\nthis is a test file\n";

static CMP_DIFF_A: &[u8] = b"hello world\nthis is file a\n";

static CMP_DIFF_B: &[u8] = b"hello world\nthis is file b\n";

// Phase 2B.3 test fixtures — wc, cut, fold, expand
static WC_TEXT: &[u8] = b"hello world\nline two\n  line three  \n";
static WC_EMPTY: &[u8] = b"";
static WC_NO_NL: &[u8] = b"no newline here";
static WC_SPACES: &[u8] = b"   \t  \n";
static WC_UTF8: &[u8] = &[0x68, 0xC3, 0xA9, 0x6C, 0x6C, 0x6F, 0x0A, 0x77, 0xC3, 0xB6, 0x72, 0x6C, 0x64, 0x0A];

static CUT_DELIM: &[u8] = b"a:b:c:d\n1:2:3:4\n";
static CUT_NO_DELIM: &[u8] = b"no delimiters here\n";
static CUT_BYTES: &[u8] = b"abcdefghij\n";

static FOLD_LONG: &[u8] = b"this is a very long line that should be folded at the specified width\n";
static FOLD_EXACT: &[u8] = b"1234567890\n";
static FOLD_OVER: &[u8] = b"12345678901\n";
static FOLD_TABS: &[u8] = b"hello\tworld\t!\n";

static EXPAND_TABS: &[u8] = b"a\tb\tc\n";
static EXPAND_CONSECUTIVE: &[u8] = b"\t\tx\n";
static EXPAND_NO_NL: &[u8] = b"a\tb";

pub static INITRAMFS: &[RamEntry] = &[
    // Directories
    RamEntry::dir("/", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc/sunlight", 0, 0, mode::DIR_755),
    RamEntry::dir("/bin", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/ChronosDosShell.sunapp", 0, 0, mode::DIR_755),
    RamEntry::dir(
        "/Applications/ChronosDosShell.sunapp/Program",
        0,
        0,
        mode::DIR_755,
    ),
    RamEntry::dir(
        "/Applications/ChronosDosShell.sunapp/Program/TESTS",
        0,
        0,
        mode::DIR_755,
    ),
    RamEntry::dir(
        "/Applications/ChronosDosShell.sunapp/Resources",
        0,
        0,
        mode::DIR_755,
    ),
    RamEntry::dir(
        "/Applications/ChronosDosShell.sunapp/Licenses",
        0,
        0,
        mode::DIR_755,
    ),

    RamEntry::dir("/Applications/SunlightMines.sunapp", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/SunlightMines.sunapp/Program", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/SunlightMines.sunapp/Resources", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/SunlightMines.sunapp/Defaults", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/SunlightMines.sunapp/Licenses", 0, 0, mode::DIR_755),

    RamEntry::dir("/Applications/ChronosFileLab.sunapp", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/ChronosFileLab.sunapp/Program", 0, 0, mode::DIR_755),
    RamEntry::dir(
        "/Applications/ChronosFileLab.sunapp/Dependencies",
        0,
        0,
        mode::DIR_755,
    ),
    RamEntry::dir("/Applications/ChronosFileLab.sunapp/Resources", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/ChronosFileLab.sunapp/Defaults", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/ChronosFileLab.sunapp/Licenses", 0, 0, mode::DIR_755),

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
    RamEntry::dir("/root/.config", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/.config/sunlight", 0, 0, mode::DIR_700),
    RamEntry::dir("/tests", 0, 0, mode::DIR_755),
    RamEntry::dir("/home/user", 1000, 1000, mode::DIR_755),
    // Default unprivileged user's standard folders (uid/gid 1000).
    RamEntry::dir("/home/user/Desktop", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Documents", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Downloads", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Pictures", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Music", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Videos", 1000, 1000, mode::DIR_755),
    // -- End standard home directory layout ---------------------------------

    RamEntry::file("/tests/cat-empty", 0, 0, mode::FILE_644, b""),
    RamEntry::file("/tests/cat-hello", 0, 0, mode::FILE_644, b"hello from cat\n"),
    RamEntry::file("/tests/cat-nonewline", 0, 0, mode::FILE_644, b"nonewline"),
    RamEntry::file("/tests/cat-big", 0, 0, mode::FILE_644, &CAT_BIG_BYTES),
    RamEntry::file("/tests/ho", 0, 0, mode::FILE_644, HEAD_ONELINE),
    RamEntry::file("/tests/hm", 0, 0, mode::FILE_644, HEAD_MULTILINE),
    RamEntry::file("/tests/ia", 0, 0, mode::FILE_644, CMP_IDENTICAL),
    RamEntry::file("/tests/ib", 0, 0, mode::FILE_644, CMP_IDENTICAL),
    RamEntry::file("/tests/da", 0, 0, mode::FILE_644, CMP_DIFF_A),
    RamEntry::file("/tests/db", 0, 0, mode::FILE_644, CMP_DIFF_B),
    // Phase 2B.3: wc, cut, fold, expand
    RamEntry::file("/tests/wc-text", 0, 0, mode::FILE_644, WC_TEXT),
    RamEntry::file("/tests/wc-empty", 0, 0, mode::FILE_644, WC_EMPTY),
    RamEntry::file("/tests/wc-nonl", 0, 0, mode::FILE_644, WC_NO_NL),
    RamEntry::file("/tests/wc-spaces", 0, 0, mode::FILE_644, WC_SPACES),
    RamEntry::file("/tests/wc-utf8", 0, 0, mode::FILE_644, WC_UTF8),
    RamEntry::file("/tests/cut-delim", 0, 0, mode::FILE_644, CUT_DELIM),
    RamEntry::file("/tests/cut-nodelim", 0, 0, mode::FILE_644, CUT_NO_DELIM),
    RamEntry::file("/tests/cut-bytes", 0, 0, mode::FILE_644, CUT_BYTES),
    RamEntry::file("/tests/fold-long", 0, 0, mode::FILE_644, FOLD_LONG),
    RamEntry::file("/tests/fold-exact", 0, 0, mode::FILE_644, FOLD_EXACT),
    RamEntry::file("/tests/fold-over", 0, 0, mode::FILE_644, FOLD_OVER),
    RamEntry::file("/tests/fold-tabs", 0, 0, mode::FILE_644, FOLD_TABS),
    RamEntry::file("/tests/expand-tabs", 0, 0, mode::FILE_644, EXPAND_TABS),
    RamEntry::file("/tests/expand-cons", 0, 0, mode::FILE_644, EXPAND_CONSECUTIVE),
    RamEntry::file("/tests/expand-nonl", 0, 0, mode::FILE_644, EXPAND_NO_NL),

    RamEntry::dir("/tmp", 0, 0, mode::DIR_1777),
    RamEntry::dir("/run", 0, 0, mode::DIR_755),
    RamEntry::dir("/state", 0, 0, mode::DIR_755),
    RamEntry::dir("/state/sunlightd", 0, 0, mode::DIR_700),
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
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/apps/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/apps/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/apps/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/places/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/places/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/places/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/mimetypes/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/mimetypes/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/mimetypes/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/devices/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/devices/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/devices/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/actions/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/actions/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/actions/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/status/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/status/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/status/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/preferences/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/preferences/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/preferences/16", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/var/sunlightos/icons/SunlightOS/apps/48/sunlight-runner.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/icons/SunlightOS/apps/48/system-run.tga"),
    ),
    RamEntry::file(
        "/var/sunlightos/icons/SunlightOS/apps/48/runner.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/icons/SunlightOS/apps/48/system-run.tga"),
    ),
    RamEntry::file(
        "/var/sunlightos/icons/SunlightOS/apps/48/generic-app.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/icons/SunlightOS/apps/48/applications-system.tga"),
    ),
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
    // Locally bundled wallpapers available in Wallpaper Settings.
    // Converted from docs/images/wallpaper{1..6}.{jpg,png} to render-ready
    // TGA type-2 24 bpp BGR (the desktop renderer is TGA-only).
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper1.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper1.tga"),
    ),
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper2.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper2.tga"),
    ),
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper3.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper3.tga"),
    ),
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper4.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper4.tga"),
    ),
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper5.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper5.tga"),
    ),
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper6.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper6.tga"),
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
    // Locale foundation (Phase locale prep for Calendar)
    RamEntry::file(
        "/etc/locale.conf",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../etc/locale.conf"),
    ),
    RamEntry::file(
        "/etc/locale.gen",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../etc/locale.gen"),
    ),
    RamEntry::file(
        "/root/.config/sunlight/desktop.toml",
        0,
        0,
        mode::FILE_644,
        b"[desktop]\nwallpaper = \"/var/sunlightos/wallpapers/wallpaper.tga\"\nwallpaper_mode = \"cover\"\n",
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
        "/etc/sunlight/ssh.toml",
        0,
        0,
        mode::FILE_644,
        br#"listen_address = "0.0.0.0"
port = 22
host_key_file = "/etc/sunlight/ssh_host_ed25519_key"
password_authentication = true
max_auth_attempts = 3
max_connections = 8
max_sessions_per_connection = 1
login_timeout_seconds = 30
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
        "/bin/fold",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/expand",
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
        "/bin/true",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/false",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/basename",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/dirname",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/cmp",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/cksum",
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
        "/bin/sunlight-hwinfo",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-hwinfo\n",
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
        "/bin/thermalctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/thermalctl\n",
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
    // Session lock recovery CLI (mezzo daemon is init-launched at /sbin/mezzo).
    RamEntry::file(
        "/bin/mezzoctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/mezzoctl\n",
    ),
    RamEntry::file(
        "/usr/bin/mezzoctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/mezzoctl\n",
    ),
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
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Manifest.toml",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Manifest.toml"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/SUNSH.EXE",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/AUTOEXEC.BAT",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/AUTOEXEC.BAT"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/CHILD.BAT",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/CHILD.BAT"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/MIDTERM.BAT",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/MIDTERM.BAT"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/TESTS/VGALAB.COM",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/VGALAB.COM"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/TESTS/SUNPAINT.COM",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/SUNPAINT.COM"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/TESTS/SUNMINE.EXE",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/SUNMINE.EXE"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Resources/icon.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Resources/icon.tga"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Licenses/README.txt",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Licenses/README.txt"),
    ),

    RamEntry::file(
        "/Applications/SunlightMines.sunapp/Manifest.toml",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/Manifest.toml"),
    ),
    RamEntry::file(
        "/Applications/SunlightMines.sunapp/Program/SUNMINE.EXE",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/Program/SUNMINE.EXE"),
    ),
    RamEntry::file(
        "/Applications/SunlightMines.sunapp/Resources/icon.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/Resources/icon.tga"),
    ),
    RamEntry::file(
        "/Applications/SunlightMines.sunapp/Resources/icon-large.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/Resources/icon-large.tga"),
    ),
    RamEntry::file(
        "/Applications/SunlightMines.sunapp/README.md",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/README.md"),
    ),
    RamEntry::file(
        "/Applications/SunlightMines.sunapp/Licenses/README.txt",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/Licenses/README.txt"),
    ),

    RamEntry::file(
        "/Applications/ChronosFileLab.sunapp/Manifest.toml",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosFileLab.sunapp/Manifest.toml"),
    ),
    RamEntry::file(
        "/Applications/ChronosFileLab.sunapp/Program/FILELAB.COM",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosFileLab.sunapp/Program/FILELAB.COM"),
    ),
    RamEntry::file(
        "/Applications/ChronosFileLab.sunapp/Program/README.TXT",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosFileLab.sunapp/Program/README.TXT"),
    ),
    RamEntry::file(
        "/Applications/ChronosFileLab.sunapp/Program/FILELAB.ASM",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosFileLab.sunapp/Program/FILELAB.ASM"),
    ),
    RamEntry::file(
        "/Applications/ChronosFileLab.sunapp/Resources/icon.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosFileLab.sunapp/Resources/icon.tga"),
    ),
    RamEntry::file("/bin/sun-open", 0, 0, mode::FILE_755, b"#!/sunlight/sun-open\n"),
    // GUI Terminal emulator
    RamEntry::file("/bin/sunlight-terminal", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-terminal\n"),
    // Chronos: native DOS compatibility application.
    RamEntry::file("/bin/sunlight-chronos", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-chronos\n"),
    // GUI Task Monitor
    RamEntry::file("/bin/sunlight-tasks", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-tasks\n"),
    // SunLight-Bench: CPU/multi-core performance benchmark
    RamEntry::file("/bin/sunbench", 0, 0, mode::FILE_755, b"#!/sunlight/sunbench\n"),
    // GUI calculator client
    RamEntry::file("/bin/calculator", 0, 0, mode::FILE_755, b"#!/sunlight/calculator\n"),
    // Developer widget gallery (not Control Panel / Lock Screen)
    RamEntry::file(
        "/bin/widget-gallery",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/widget-gallery\n",
    ),
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
    RamEntry::file(
        "/bin/sunlight-writer",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-writer\n",
    ),
    // Silicon Echoes: 1993: native graphical narrative game.
    RamEntry::file(
        "/bin/silicon-echoes",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/silicon-echoes\n",
    ),
    // Sunlight Calendar: graphical calendar client
    RamEntry::file(
        "/bin/sunlight-calendar",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-calendar\n",
    ),
    // Sunlight Reminders: personal tasks and reminders client
    RamEntry::file(
        "/bin/sunlight-reminders",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-reminders\n",
    ),
    RamEntry::file(
        "/bin/sunlight-devices",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-devices\n",
    ),
    RamEntry::file(
        "/bin/rappid-rabbit",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/rappid-rabbit\n",
    ),
    RamEntry::file(
        "/bin/sunlight-api-lab",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-api-lab\n",
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
    RamEntry::file(
        "/bin/emoji-picker",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/emoji-picker\n",
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
        "/sunlight-utils/fold",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/expand",
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
        "/sunlight-utils/true",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/false",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/basename",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/dirname",
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
    RamEntry::file(
        "/usr/bin/tzutils",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/tzutils\n",
    ),
    RamEntry::file(
        "/etc/sunlight/ntp.conf",
        0,
        0,
        mode::FILE_644,
        b"# Optional explicit NTP servers (one per line).\n\
# When present, these override regional pool selection.\n\
# server 0.pool.ntp.org\n",
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
        "/usr/bin/sunlight-hwinfo",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-hwinfo\n",
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
        "/usr/bin/thermalctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/thermalctl\n",
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
        "/usr/bin/sunlight-chronos",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-chronos\n",
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
        "/usr/bin/widget-gallery",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/widget-gallery\n",
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
    RamEntry::file(
        "/usr/bin/sunlight-writer",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-writer\n",
    ),
    // Silicon Echoes: 1993: native graphical narrative game.
    RamEntry::file(
        "/usr/bin/silicon-echoes",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/silicon-echoes\n",
    ),
    // Sunlight Calendar: graphical calendar client
    RamEntry::file(
        "/usr/bin/sunlight-calendar",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-calendar\n",
    ),
    // Sunlight Reminders: personal tasks and reminders client
    RamEntry::file(
        "/usr/bin/sunlight-reminders",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-reminders\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-devices",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-devices\n",
    ),
    RamEntry::file(
        "/usr/bin/rappid-rabbit",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/rappid-rabbit\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-api-lab",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-api-lab\n",
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
    // MiniType fonts (standalone .mtf) for dynamic font loader / future OS image use.
    // Generated via assets/fonts/minitype/generate.sh (and sun-font/build.rs).
    // See docs/MINITYPE_FONTS.md
    RamEntry::dir("/usr/share/sunlightos/fonts", 0, 0, mode::DIR_755),
    RamEntry::dir("/usr/share/sunlightos/fonts/minitype", 0, 0, mode::DIR_755),

    // Material Symbols font (for shell panel icons, future dynamic loaders).
    // Bundled from local system font; see assets/fonts/material-symbols/
    RamEntry::dir("/usr/share/fonts", 0, 0, mode::DIR_755),
    RamEntry::dir("/usr/share/fonts/material-symbols", 0, 0, mode::DIR_755),
    // Font file is present in source (assets/fonts/material-symbols/) and would be
    // installed to runtime image in a full build. For kernel ramfs size we include
    // a marker (real .ttf can be added when dynamic font loading lands).
    RamEntry::file(
        "/usr/share/fonts/material-symbols/MaterialSymbolsOutlined.ttf",
        0,
        0,
        mode::FILE_644,
        b"MaterialSymbolsOutlined (bundled; see assets/fonts/material-symbols/)\0",
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_11.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_11.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_13.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_13.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_16.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_16.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_medium_13.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_medium_13.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_semibold_13.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_semibold_13.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_title_18.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_title_18.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_mono_regular_14.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_mono_regular_14.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_mono_medium_14.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_mono_medium_14.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_serif_regular_16.mtf",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_serif_regular_16.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_emoji_16.mtf",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_emoji_16.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_emoji_manifest.txt",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_emoji_manifest.txt"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/material_icons_16.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/material_icons_16.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/material_icons_24.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/material_icons_24.mtf"),
    ),
    RamEntry::dir("/usr/share/licenses", 0, 0, mode::DIR_755),
    RamEntry::dir("/usr/share/licenses/openmoji", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/usr/share/licenses/openmoji/LICENSE.txt",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../licenses/openmoji/LICENSE.txt"),
    ),
    RamEntry::file(
        "/usr/share/licenses/openmoji/ATTRIBUTION.txt",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../licenses/openmoji/ATTRIBUTION.txt"),
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

    // ── Welcome document ──────────────────────────────────────────────────
    // Text asset copied into the Documents folder for the default accounts.
    // /home/user/ serves as the home template; later the installer will copy
    // from equivalent template locations when provisioning real users.
    // A system copy is provided under /usr/share for reference/reuse.
    RamEntry::dir("/usr/share/sunlightos/documents", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/usr/share/sunlightos/documents/Welcome to SunlightOS.txt",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/documents/Welcome to SunlightOS.txt"),
    ),
    // Root's Documents (seeded read-only by the OS).
    RamEntry::file(
        "/root/Documents/Welcome to SunlightOS.txt",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/documents/Welcome to SunlightOS.txt"),
    ),
    // Default user's Documents (template home, uid/gid 1000).
    RamEntry::file(
        "/home/user/Documents/Welcome to SunlightOS.txt",
        1000, 1000, mode::FILE_644,
        include_bytes!("../../assets/documents/Welcome to SunlightOS.txt"),
    ),

    // ── Why SunlightOS Exists document ──────────────────────────────────────
    // Identity and philosophy document seeded alongside Welcome.
    // Same deployment pattern: system reference + both default home accounts.
    RamEntry::file(
        "/usr/share/sunlightos/documents/Why SunlightOS Exists.txt",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/documents/Why SunlightOS Exists.txt"),
    ),
    RamEntry::file(
        "/root/Documents/Why SunlightOS Exists.txt",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/documents/Why SunlightOS Exists.txt"),
    ),
    RamEntry::file(
        "/home/user/Documents/Why SunlightOS Exists.txt",
        1000, 1000, mode::FILE_644,
        include_bytes!("../../assets/documents/Why SunlightOS Exists.txt"),
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
    fn reused_slot_rejects_the_previous_handle_incarnation() {
        let mut fs = RamFs::new(TEST_ENTRIES);
        let first = fs.open("/bin/sh").unwrap();
        fs.close(first).unwrap();
        let second = fs.open("/etc/motd").unwrap();

        assert_ne!(first, second);
        assert_eq!(fs.read(first, 0, &mut [0; 1]), Err(FsError::BadHandle));
        assert_eq!(fs.close(first), Err(FsError::BadHandle));
        assert!(fs.read(second, 0, &mut [0; 1]).is_ok());
    }

    #[test]
    fn truncate_keeps_the_handle_valid_and_discards_old_contents() {
        let mut fs = RamFs::new(TEST_ENTRIES);
        let handle = fs.open("/bin/sh").unwrap();
        fs.truncate(handle).unwrap();

        let mut one = [0u8; 1];
        assert_eq!(fs.read(handle, 0, &mut one), Ok(0));
        assert_eq!(fs.write(handle, 0, b"x"), Ok(1));
        assert_eq!(fs.read(handle, 0, &mut one), Ok(1));
        assert_eq!(one, [b'x']);
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
    fn rename_dynamic_file_replaces_static_file_contents() {
        let mut fs = RamFs::new(DIR_ENTRIES);
        let handle = fs
            .create_file("/etc/motd.tmp", 0, 0, mode::FILE_644)
            .unwrap();
        fs.write(handle, 0, b"updated\n").unwrap();
        fs.close(handle).unwrap();

        assert_eq!(fs.rename("/etc/motd.tmp", "/etc/motd"), Ok(()));
        assert_eq!(fs.open("/etc/motd.tmp"), Err(FsError::NotFound));

        let handle = fs.open("/etc/motd").unwrap();
        let mut buf = [0u8; 16];
        let read = fs.read(handle, 0, &mut buf).unwrap();
        assert_eq!(&buf[..read], b"updated\n");
    }

    #[test]
    fn exclusive_create_never_reopens_or_truncates_existing_file() {
        let mut fs = RamFs::new(DIR_ENTRIES);
        let original = fs
            .create_file_exclusive("/etc/secret", 0, 0, mode::FILE_600)
            .unwrap();
        fs.write(original, 0, b"retained").unwrap();
        fs.close(original).unwrap();

        assert_eq!(
            fs.create_file_exclusive("/etc/secret", 0, 0, mode::FILE_600),
            Err(FsError::AlreadyExists)
        );
        let handle = fs.open("/etc/secret").unwrap();
        let mut bytes = [0u8; 16];
        let len = fs.read(handle, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[..len], b"retained");
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

    #[test]
    fn welcome_document_is_present_in_default_homes() {
        // Verify the welcome document asset is wired into root and the
        // default user template under Documents, and that Documents dirs
        // exist (they are declared earlier in INITRAMFS).
        let has_root_welcome = INITRAMFS
            .iter()
            .any(|e| e.path == "/root/Documents/Welcome to SunlightOS.txt" && !e.is_dir);
        let has_user_welcome = INITRAMFS
            .iter()
            .any(|e| e.path == "/home/user/Documents/Welcome to SunlightOS.txt" && !e.is_dir);
        let has_documents_root = INITRAMFS
            .iter()
            .any(|e| e.path == "/root/Documents" && e.is_dir);
        let has_documents_user = INITRAMFS
            .iter()
            .any(|e| e.path == "/home/user/Documents" && e.is_dir);

        assert!(has_root_welcome, "missing /root/Documents welcome file");
        assert!(
            has_user_welcome,
            "missing /home/user/Documents welcome file"
        );
        assert!(has_documents_root, "missing /root/Documents dir entry");
        assert!(has_documents_user, "missing /home/user/Documents dir entry");

        // Also spot-check content by opening via a fresh RamFs.
        let mut fs = RamFs::new(INITRAMFS);
        let h = fs
            .open("/home/user/Documents/Welcome to SunlightOS.txt")
            .unwrap();
        let mut buf = [0u8; 80];
        let n = fs.read(h, 0, &mut buf).unwrap();
        // Should start with the ASCII title box.
        assert!(n > 0);
        assert_eq!(&buf[..10], b"+---------");
        // Verify text from the header/intro is present (full content is the
        // asset bytes; we just sanity-check a few early distinctive words).
        let text = core::str::from_utf8(&buf[..n]).unwrap_or("");
        assert!(text.contains("Welcome to SunlightOS") || text.contains("A bright"));
    }

    #[test]
    fn chronos_command_stubs_are_present_in_initramfs() {
        for path in ["/bin/sunlight-chronos", "/usr/bin/sunlight-chronos"] {
            let entry = INITRAMFS
                .iter()
                .find(|entry| entry.path == path)
                .unwrap_or_else(|| panic!("missing {path}"));
            assert!(!entry.is_dir);
            assert_eq!(entry.mode, mode::FILE_755);
            assert_eq!(entry.data, b"#!/sunlight/sunlight-chronos\n");
        }
    }

    #[test]
    fn chronos_dos_shell_bundle_is_present_in_initramfs() {
        for path in [
            "/Applications/ChronosDosShell.sunapp/Manifest.toml",
            "/Applications/ChronosDosShell.sunapp/Program/SUNSH.EXE",
            "/Applications/ChronosDosShell.sunapp/Program/TESTS/VGALAB.COM",
            "/Applications/ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM",
            "/Applications/ChronosDosShell.sunapp/Program/TESTS/SUNPAINT.COM",
            "/Applications/ChronosDosShell.sunapp/Resources/icon.tga",
        ] {
            assert!(
                INITRAMFS
                    .iter()
                    .any(|entry| entry.path == path && !entry.is_dir),
                "missing {path}"
            );
        }
    }

    #[test]
    fn why_sunlightos_exists_document_is_present_in_default_homes() {
        // Verify the "Why SunlightOS Exists" identity document is seeded
        // into the same locations as the Welcome document.
        let has_root_copy = INITRAMFS
            .iter()
            .any(|e| e.path == "/root/Documents/Why SunlightOS Exists.txt" && !e.is_dir);
        let has_user_copy = INITRAMFS
            .iter()
            .any(|e| e.path == "/home/user/Documents/Why SunlightOS Exists.txt" && !e.is_dir);
        let has_system_copy = INITRAMFS.iter().any(|e| {
            e.path == "/usr/share/sunlightos/documents/Why SunlightOS Exists.txt" && !e.is_dir
        });

        assert!(
            has_root_copy,
            "missing /root/Documents/Why SunlightOS Exists.txt"
        );
        assert!(
            has_user_copy,
            "missing /home/user/Documents/Why SunlightOS Exists.txt"
        );
        assert!(
            has_system_copy,
            "missing /usr/share/sunlightos/documents/Why SunlightOS Exists.txt"
        );

        // Spot-check content via a fresh RamFs.
        let mut fs = RamFs::new(INITRAMFS);
        let h = fs
            .open("/home/user/Documents/Why SunlightOS Exists.txt")
            .unwrap();
        let mut buf = [0u8; 80];
        let n = fs.read(h, 0, &mut buf).unwrap();
        assert!(n > 0);
        assert_eq!(&buf[..10], b"+---------");
        let text = core::str::from_utf8(&buf[..n]).unwrap_or("");
        assert!(text.contains("SunlightOS exists") || text.contains("future-first"));
    }

    #[test]
    fn locale_foundation_files_present_in_initramfs() {
        // Verify /etc/locale.conf and /etc/locale.gen are registered for boot.
        let has_locale_conf = INITRAMFS
            .iter()
            .any(|e| e.path == "/etc/locale.conf" && !e.is_dir);
        let has_locale_gen = INITRAMFS
            .iter()
            .any(|e| e.path == "/etc/locale.gen" && !e.is_dir);
        assert!(has_locale_conf, "missing /etc/locale.conf in INITRAMFS");
        assert!(has_locale_gen, "missing /etc/locale.gen in INITRAMFS");

        // Spot-check content is the expected default.
        let mut fs = RamFs::new(INITRAMFS);
        let h = fs.open("/etc/locale.conf").unwrap();
        let mut buf = [0u8; 256];
        let n = fs.read(h, 0, &mut buf).unwrap();
        let text = core::str::from_utf8(&buf[..n]).unwrap_or("");
        assert!(text.contains("LANG=en_US.UTF-8"));
        assert!(text.contains("LC_TIME=en_US.UTF-8"));

        let h2 = fs.open("/etc/locale.gen").unwrap();
        let mut buf2 = [0u8; 64];
        let n2 = fs.read(h2, 0, &mut buf2).unwrap();
        let text2 = core::str::from_utf8(&buf2[..n2]).unwrap_or("");
        assert!(text2.contains("C.UTF-8"));
        assert!(text2.contains("en_US.UTF-8"));
    }

    #[test]
    fn sun_emoji_assets_and_license_are_present_in_initramfs() {
        for path in [
            "/usr/share/sunlightos/fonts/minitype/sunlight_emoji_16.mtf",
            "/usr/share/sunlightos/fonts/minitype/sunlight_emoji_manifest.txt",
            "/usr/share/licenses/openmoji/LICENSE.txt",
            "/usr/share/licenses/openmoji/ATTRIBUTION.txt",
        ] {
            assert!(
                INITRAMFS
                    .iter()
                    .any(|entry| entry.path == path && !entry.is_dir),
                "missing {path} in INITRAMFS"
            );
        }
    }
}
