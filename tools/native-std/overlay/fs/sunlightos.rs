// Adapted from Rust std fs/unsupported.rs (MIT OR Apache-2.0).
use crate::ffi::OsString;
use crate::fmt;
use crate::fs::TryLockError;
use crate::io::{self, BorrowedCursor, IoSlice, IoSliceMut, SeekFrom};
use crate::path::{Path, PathBuf};
pub use crate::sys::fs::common::Dir;
use crate::sys::time::SystemTime;
use crate::sys::{sunlightos::*, unsupported};

pub struct File(u32);

#[derive(Clone)]
pub struct FileAttr {
    size: u64,
    mode: u16,
    kind: u8,
}

pub struct ReadDir(!);

pub struct DirEntry(!);

#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct FileTimes {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePermissions(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileType(u8);

#[derive(Debug)]
pub struct DirBuilder {}

impl FileAttr {
    pub fn size(&self) -> u64 {
        self.size
    }
    pub fn perm(&self) -> FilePermissions {
        FilePermissions(self.mode)
    }
    pub fn file_type(&self) -> FileType {
        FileType(self.kind)
    }
    pub fn modified(&self) -> io::Result<SystemTime> {
        unsupported()
    }
    pub fn accessed(&self) -> io::Result<SystemTime> {
        unsupported()
    }
    pub fn created(&self) -> io::Result<SystemTime> {
        unsupported()
    }
}
impl FilePermissions {
    pub fn readonly(&self) -> bool {
        self.0 & 0o222 == 0
    }
    pub fn set_readonly(&mut self, readonly: bool) {
        if readonly {
            self.0 &= !0o222;
        } else {
            self.0 |= 0o222;
        }
    }
}

impl FileTimes {
    pub fn set_accessed(&mut self, _t: SystemTime) {}
    pub fn set_modified(&mut self, _t: SystemTime) {}
}

impl FileType {
    pub fn is_dir(&self) -> bool {
        self.0 == 1
    }
    pub fn is_file(&self) -> bool {
        self.0 == 0
    }
    pub fn is_symlink(&self) -> bool {
        self.0 == 2
    }
}

impl fmt::Debug for ReadDir {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
    }
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<io::Result<DirEntry>> {
        self.0
    }
}

impl DirEntry {
    pub fn path(&self) -> PathBuf {
        self.0
    }

    pub fn file_name(&self) -> OsString {
        self.0
    }

    pub fn metadata(&self) -> io::Result<FileAttr> {
        self.0
    }

    pub fn file_type(&self) -> io::Result<FileType> {
        self.0
    }
}

impl OpenOptions {
    pub fn new() -> Self {
        Self {
            read: false,
            write: false,
            append: false,
            truncate: false,
            create: false,
            create_new: false,
        }
    }
    pub fn read(&mut self, value: bool) {
        self.read = value;
    }
    pub fn write(&mut self, value: bool) {
        self.write = value;
    }
    pub fn append(&mut self, value: bool) {
        self.append = value;
    }
    pub fn truncate(&mut self, value: bool) {
        self.truncate = value;
    }
    pub fn create(&mut self, value: bool) {
        self.create = value;
    }
    pub fn create_new(&mut self, value: bool) {
        self.create_new = value;
    }
}

impl File {
    pub fn open(path: &Path, opts: &OpenOptions) -> io::Result<Self> {
        if !opts.read && !opts.write && !opts.append {
            return Err(io::Error::from_raw_os_error(22));
        }
        if (opts.create || opts.create_new || opts.truncate) && !(opts.write || opts.append) {
            return Err(io::Error::from_raw_os_error(22));
        }
        let bytes = path.as_os_str().as_encoded_bytes();
        if bytes.contains(&0) {
            return Err(io::Error::from_raw_os_error(22));
        }
        let flags = match (opts.read, opts.write || opts.append) {
            (true, true) => 2,
            (false, true) => 1,
            _ => 0,
        } | if opts.append { 0x400 } else { 0 }
            | if opts.truncate { 0x200 } else { 0 }
            | if opts.create { 0x40 } else { 0 }
            | if opts.create_new { 0xc0 } else { 0 };
        cvt(unsafe { sl_std_open(bytes.as_ptr(), bytes.len(), flags) }).map(|fd| File(fd as u32))
    }
    pub fn file_attr(&self) -> io::Result<FileAttr> {
        let mut value = [0u64; 3];
        cvt(unsafe { sl_std_stat(self.0, value.as_mut_ptr()) } as isize)?;
        Ok(FileAttr {
            size: value[0],
            mode: value[1] as u16,
            kind: value[2] as u8,
        })
    }
    pub fn read(&self, bytes: &mut [u8]) -> io::Result<usize> {
        cvt(unsafe { sl_std_read(self.0, bytes.as_mut_ptr(), bytes.len()) })
    }
    pub fn write(&self, bytes: &[u8]) -> io::Result<usize> {
        cvt(unsafe { sl_std_write(self.0, bytes.as_ptr(), bytes.len()) })
    }
    pub fn read_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.read(
            bufs.iter_mut()
                .find(|b| !b.is_empty())
                .map_or(&mut [], |b| &mut **b),
        )
    }
    pub fn write_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.write(bufs.iter().find(|b| !b.is_empty()).map_or(&[], |b| &**b))
    }
    pub fn is_read_vectored(&self) -> bool {
        false
    }
    pub fn is_write_vectored(&self) -> bool {
        false
    }
    pub fn read_buf(&self, cursor: BorrowedCursor<'_>) -> io::Result<()> {
        crate::io::default_read_buf(|bytes| self.read(bytes), cursor)
    }
    pub fn flush(&self) -> io::Result<()> {
        Ok(())
    }
    pub fn seek(&self, position: SeekFrom) -> io::Result<u64> {
        let (offset, whence) = match position {
            SeekFrom::Start(value) => (
                i64::try_from(value).map_err(|_| io::Error::from_raw_os_error(22))?,
                0,
            ),
            SeekFrom::Current(value) => (value, 1),
            SeekFrom::End(value) => (value, 2),
        };
        let mut result = 0;
        cvt(unsafe { sl_std_seek(self.0, offset, whence, &mut result) } as isize)?;
        Ok(result)
    }
    pub fn tell(&self) -> io::Result<u64> {
        self.seek(SeekFrom::Current(0))
    }
    pub fn size(&self) -> Option<io::Result<u64>> {
        Some(self.file_attr().map(|a| a.size()))
    }
    pub fn fsync(&self) -> io::Result<()> {
        unsupported()
    }
    pub fn datasync(&self) -> io::Result<()> {
        unsupported()
    }
    pub fn lock(&self) -> io::Result<()> {
        unsupported()
    }
    pub fn lock_shared(&self) -> io::Result<()> {
        unsupported()
    }
    pub fn try_lock(&self) -> Result<(), TryLockError> {
        Err(TryLockError::Error(crate::sys::unsupported_err()))
    }
    pub fn try_lock_shared(&self) -> Result<(), TryLockError> {
        self.try_lock()
    }
    pub fn unlock(&self) -> io::Result<()> {
        unsupported()
    }
    pub fn truncate(&self, _size: u64) -> io::Result<()> {
        unsupported()
    }
    pub fn duplicate(&self) -> io::Result<File> {
        unsupported()
    }
    pub fn set_permissions(&self, _perm: FilePermissions) -> io::Result<()> {
        unsupported()
    }
    pub fn set_times(&self, _times: FileTimes) -> io::Result<()> {
        unsupported()
    }
}
impl Drop for File {
    fn drop(&mut self) {
        unsafe { sl_std_close(self.0) };
    }
}

impl DirBuilder {
    pub fn new() -> DirBuilder {
        DirBuilder {}
    }

    pub fn mkdir(&self, _p: &Path) -> io::Result<()> {
        unsupported()
    }
}

impl fmt::Debug for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("File").field(&self.0).finish()
    }
}

pub fn readdir(_p: &Path) -> io::Result<ReadDir> {
    unsupported()
}

pub fn unlink(_p: &Path) -> io::Result<()> {
    unsupported()
}

pub fn rename(_old: &Path, _new: &Path) -> io::Result<()> {
    unsupported()
}

pub fn set_perm(_p: &Path, perm: FilePermissions) -> io::Result<()> {
    let _ = perm;
    unsupported()
}

pub fn set_times(_p: &Path, _times: FileTimes) -> io::Result<()> {
    unsupported()
}

pub fn set_times_nofollow(_p: &Path, _times: FileTimes) -> io::Result<()> {
    unsupported()
}

pub fn rmdir(_p: &Path) -> io::Result<()> {
    unsupported()
}

pub fn remove_dir_all(_path: &Path) -> io::Result<()> {
    unsupported()
}

pub fn exists(path: &Path) -> io::Result<bool> {
    match stat(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn readlink(_p: &Path) -> io::Result<PathBuf> {
    unsupported()
}

pub fn symlink(_original: &Path, _link: &Path) -> io::Result<()> {
    unsupported()
}

pub fn link(_src: &Path, _dst: &Path) -> io::Result<()> {
    unsupported()
}

pub fn stat(path: &Path) -> io::Result<FileAttr> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    File::open(path, &opts)?.file_attr()
}

pub fn lstat(_p: &Path) -> io::Result<FileAttr> {
    unsupported()
}

pub fn canonicalize(_p: &Path) -> io::Result<PathBuf> {
    unsupported()
}

pub fn copy(_from: &Path, _to: &Path) -> io::Result<u64> {
    unsupported()
}
