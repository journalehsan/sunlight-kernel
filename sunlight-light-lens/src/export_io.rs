//! Export I/O boundary. Host tests exercise the same save state machine with
//! host files; native publication continues to use SunlightOS rename APIs.
#[cfg(not(test))]
pub(super) use sunlight_libc::{
    close, open_with_flags_mode, rename, rename_no_replace, unlink, write_all,
};
#[cfg(not(test))]
pub(super) fn pid() -> u64 {
    sunlight_ipc::getpid()
}

#[cfg(test)]
mod host {
    use std::{
        fs::{self, File, OpenOptions},
        io::Write,
        os::{
            fd::{FromRawFd, IntoRawFd},
            unix::ffi::OsStrExt,
        },
        path::Path,
    };
    use sunlight_libc::{Errno, Fd};
    std::thread_local! { pub static FAIL_WRITE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) }; }
    fn path(bytes: &[u8]) -> &Path {
        Path::new(std::ffi::OsStr::from_bytes(bytes))
    }
    pub fn pid() -> u64 {
        std::process::id() as u64
    }
    pub fn open_with_flags_mode(bytes: &[u8], _flags: u64, _mode: u16) -> Result<Fd, Errno> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path(bytes))
            .map(|f| Fd(f.into_raw_fd() as u32))
            .map_err(|_| Errno::Failed)
    }
    pub fn close(fd: Fd) -> Result<(), Errno> {
        drop(unsafe { File::from_raw_fd(fd.0 as i32) });
        Ok(())
    }
    pub fn unlink(bytes: &[u8]) -> Result<(), Errno> {
        fs::remove_file(path(bytes)).map_err(|_| Errno::Failed)
    }
    pub fn write_all(fd: Fd, bytes: &[u8]) -> Result<(), Errno> {
        if FAIL_WRITE.get() {
            return Err(Errno::Failed);
        }
        let mut file = core::mem::ManuallyDrop::new(unsafe { File::from_raw_fd(fd.0 as i32) });
        file.write_all(bytes).map_err(|_| Errno::Failed)
    }
    pub fn rename(old: &[u8], new: &[u8]) -> Result<(), Errno> {
        fs::rename(path(old), path(new)).map_err(|_| Errno::Failed)
    }
    pub fn rename_no_replace(old: &[u8], new: &[u8]) -> Result<(), Errno> {
        // Same-directory hard link protects an existing host destination.
        fs::hard_link(path(old), path(new)).map_err(|_| Errno::Failed)?;
        unlink(old)
    }
}
#[cfg(test)]
pub(super) use host::*;
