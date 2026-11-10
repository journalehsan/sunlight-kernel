//! Path-backed C directory streams, using the production native enumeration
//! and permission checks. Only stable directories, canonical absolute paths
//! and "." are supported. No descriptor or concurrent mutation contract.
//! Callers provide valid C strings/live streams and serialize each stream.
use crate::{errno, sys::Errno, DirEntry, MAX_PATH};
use core::{ffi::c_int, ptr};

#[repr(C)]
pub struct CDirent {
    pub d_name: [u8; 65],
}

pub struct SunlightDir {
    path: [u8; MAX_PATH],
    len: usize,
    offset: usize,
    ready: bool,
    eof: bool,
    entry: CDirent,
}

#[derive(Debug, PartialEq)]
enum Error {
    Local(c_int),
    Native(Errno),
}
impl From<Errno> for Error {
    fn from(error: Errno) -> Self {
        Self::Native(error)
    }
}
fn fail(error: Error) {
    match error {
        Error::Local(code) => errno::set_errno(code),
        Error::Native(error) => errno::set_from_errno(error),
    }
}

fn canonical(path: &[u8]) -> bool {
    path.starts_with(b"/")
        && (path.len() == 1
            || path[1..]
                .split(|&ch| ch == b'/')
                .all(|part| !part.is_empty() && part != b"." && part != b".."))
        && !path.contains(&0)
}

unsafe fn prepare_with(
    path: *const u8,
    cwd: impl FnOnce(&mut [u8; MAX_PATH]) -> Result<usize, Errno>,
    stat: impl FnOnce(&[u8]) -> Result<u8, Errno>,
    read: impl FnOnce(&[u8], &mut [DirEntry], usize) -> Result<usize, Errno>,
) -> Result<SunlightDir, Error> {
    if path.is_null() {
        return Err(Error::Local(errno::EFAULT));
    }
    let mut state = SunlightDir {
        path: [0; MAX_PATH],
        len: 0,
        offset: 0,
        ready: false,
        eof: false,
        entry: CDirent { d_name: [0; 65] },
    };
    while state.len < MAX_PATH && *path.add(state.len) != 0 {
        state.path[state.len] = *path.add(state.len);
        state.len += 1;
    }
    if state.len == MAX_PATH {
        return Err(Error::Local(errno::E2BIG));
    }
    if state.len == 0 {
        return Err(Error::Local(errno::ENOENT));
    }
    if &state.path[..state.len] == b"." {
        state.path.fill(0);
        state.len = cwd(&mut state.path)?;
        if state.len == 0 || state.len >= MAX_PATH || state.path[state.len] != 0 {
            return Err(Error::Local(errno::EIO));
        }
    }
    if !canonical(&state.path[..state.len]) {
        return Err(Error::Local(errno::ENOSYS));
    }
    if stat(&state.path[..state.len])? != crate::FT_DIR {
        return Err(Error::Local(errno::ENOTDIR));
    }
    // Check read permission and cache the first real entry at open time. An
    // empty directory is a successful stream, distinct from enumeration error.
    state.fill_with(read)?;
    Ok(state)
}

impl SunlightDir {
    fn fill_with(
        &mut self,
        read: impl FnOnce(&[u8], &mut [DirEntry], usize) -> Result<usize, Errno>,
    ) -> Result<(), Error> {
        let mut record = [DirEntry::zeroed()];
        let count = read(&self.path[..self.len], &mut record, self.offset)?;
        match count {
            0 => self.eof = true,
            1 => {
                let len = record[0].name_len as usize;
                if len == 0
                    || len > 64
                    || record[0].name[..len].contains(&0)
                    || record[0].name[..len].contains(&b'/')
                {
                    return Err(Error::Local(errno::EIO));
                }
                let next = self.offset.checked_add(1).ok_or(Error::Local(errno::EIO))?;
                self.entry.d_name.fill(0);
                self.entry.d_name[..len].copy_from_slice(&record[0].name[..len]);
                self.offset = next;
                self.ready = true;
            }
            _ => return Err(Error::Local(errno::EIO)),
        }
        Ok(())
    }

    fn next_with(
        &mut self,
        read: impl FnOnce(&[u8], &mut [DirEntry], usize) -> Result<usize, Errno>,
    ) -> *mut CDirent {
        if !self.ready && !self.eof {
            if let Err(error) = self.fill_with(read) {
                fail(error);
                return ptr::null_mut();
            }
        }
        if self.ready {
            self.ready = false;
            &mut self.entry
        } else {
            // EOF must not change errno. No synthetic dot entries are inserted.
            ptr::null_mut()
        }
    }
}

unsafe fn allocate_with(
    state: SunlightDir,
    allocate: impl FnOnce(usize) -> *mut u8,
) -> *mut SunlightDir {
    let stream = allocate(core::mem::size_of::<SunlightDir>()).cast::<SunlightDir>();
    if stream.is_null() {
        errno::set_errno(errno::ENOMEM);
    } else {
        stream.write(state);
    }
    stream
}

/// # Safety
/// Non-NULL path must be readable through its NUL or MAX_PATH bytes.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn opendir(path: *const u8) -> *mut SunlightDir {
    match prepare_with(
        path,
        crate::getcwd,
        |path| crate::stat(path).map(|stat| stat.file_type),
        crate::read_dir_from,
    ) {
        Ok(state) => allocate_with(state, |size| crate::alloc::malloc(size)),
        Err(error) => {
            fail(error);
            ptr::null_mut()
        }
    }
}

/// # Safety
/// Non-NULL stream must be live, exclusively accessed and returned by opendir.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn readdir(stream: *mut SunlightDir) -> *mut CDirent {
    match stream.as_mut() {
        Some(state) => state.next_with(crate::read_dir_from),
        None => {
            errno::set_errno(errno::EBADF);
            ptr::null_mut()
        }
    }
}

/// # Safety
/// Non-NULL stream must be a live, exclusively owned opendir result. It is
/// invalid after this call; NULL is diagnosed, arbitrary/dangling pointers are not.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn closedir(stream: *mut SunlightDir) -> c_int {
    if stream.is_null() {
        errno::set_errno(errno::EBADF);
        return -1;
    }
    crate::alloc::free(stream.cast());
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &[u8], out: &mut [DirEntry]) -> Result<usize, Errno> {
        assert_eq!(out.len(), 1);
        out[0].name[..name.len()].copy_from_slice(name);
        out[0].name_len = name.len() as u8;
        Ok(1)
    }
    unsafe fn state(path: &[u8]) -> Result<SunlightDir, Error> {
        prepare_with(
            path.as_ptr(),
            |_| panic!("cwd"),
            |_| Ok(crate::FT_DIR),
            |_, _, _| Ok(0),
        )
    }

    #[test]
    fn dot_uses_cwd_and_caches_a_real_entry_without_errno_changes() {
        errno::set_errno(123);
        let mut dir = unsafe {
            prepare_with(
                b".\0".as_ptr(),
                |buf| {
                    buf[..5].copy_from_slice(b"/tmp\0");
                    Ok(4)
                },
                |path| {
                    assert_eq!(path, b"/tmp");
                    Ok(crate::FT_DIR)
                },
                |path, out, offset| {
                    assert_eq!(path, b"/tmp");
                    assert_eq!(offset, 0);
                    entry(b"real", out)
                },
            )
            .unwrap()
        };
        let first = dir.next_with(|_, _, _| panic!("prefetch reread"));
        assert_eq!(unsafe { &(&(*first).d_name)[..5] }, b"real\0");
        assert!(dir
            .next_with(|_, _, offset| {
                assert_eq!(offset, 1);
                Ok(0)
            })
            .is_null());
        assert!(dir.next_with(|_, _, _| panic!("EOF reread")).is_null());
        assert_eq!(errno::get_errno(), 123);
    }

    #[test]
    fn sequential_offsets_cross_64_and_names_have_space_for_a_terminator() {
        let mut dir = unsafe { state(b"/tmp\0").unwrap() };
        dir.eof = false;
        for offset in 0..130 {
            let result = dir.next_with(|_, out, actual| {
                assert_eq!(actual, offset);
                entry(&[b'x'; 64], out)
            });
            assert!(!result.is_null());
            assert_eq!(unsafe { (*result).d_name[64] }, 0);
        }
        assert_eq!(dir.offset, 130);
    }

    #[test]
    fn path_bounds_and_unsupported_semantics_fail_before_native_access() {
        for (path, expected) in [
            (&b"\0"[..], errno::ENOENT),
            (&b"relative\0"[..], errno::ENOSYS),
            (&b"/tmp/../x\0"[..], errno::ENOSYS),
            (&b"/tmp/.\0"[..], errno::ENOSYS),
            (&b"/tmp/\0"[..], errno::ENOSYS),
            (&b"//tmp\0"[..], errno::ENOSYS),
            (&[b'x'; MAX_PATH][..], errno::E2BIG),
        ] {
            let result = unsafe {
                prepare_with(
                    path.as_ptr(),
                    |_| panic!("cwd"),
                    |_| panic!("stat"),
                    |_, _, _| panic!("read"),
                )
            };
            assert!(matches!(result, Err(Error::Local(code)) if code == expected));
        }
        let mut max = [b'x'; MAX_PATH];
        max[0] = b'/';
        max[MAX_PATH - 1] = 0;
        assert!(unsafe { state(&max) }.is_ok());
        assert!(unsafe { state(b"/\0") }.is_ok());
        assert!(unsafe { opendir(ptr::null()) }.is_null());
        assert_eq!(errno::get_errno(), errno::EFAULT);
    }

    #[test]
    fn native_errors_non_directory_and_invalid_cwd_are_not_success() {
        for error in [Errno::NoEntry, Errno::Access, Errno::Failed] {
            let result = unsafe {
                prepare_with(
                    b"/tmp\0".as_ptr(),
                    |_| panic!("cwd"),
                    |_| Err(error),
                    |_, _, _| panic!("read"),
                )
            };
            assert!(matches!(result, Err(Error::Native(e)) if e == error));
            let result = unsafe {
                prepare_with(
                    b"/tmp\0".as_ptr(),
                    |_| panic!("cwd"),
                    |_| Ok(crate::FT_DIR),
                    |_, _, _| Err(error),
                )
            };
            assert!(matches!(result, Err(Error::Native(e)) if e == error));
        }
        let result = unsafe {
            prepare_with(
                b"/file\0".as_ptr(),
                |_| panic!("cwd"),
                |_| Ok(crate::FT_FILE),
                |_, _, _| panic!("read"),
            )
        };
        assert!(matches!(result, Err(Error::Local(errno::ENOTDIR))));
        for len in [0, MAX_PATH, usize::MAX, 4] {
            let result = unsafe {
                prepare_with(
                    b".\0".as_ptr(),
                    |buf| {
                        buf.fill(b'x');
                        Ok(len)
                    },
                    |_| panic!("stat"),
                    |_, _, _| panic!("read"),
                )
            };
            assert!(matches!(result, Err(Error::Local(errno::EIO))));
        }
    }

    #[test]
    fn empty_streams_allocation_failure_and_close() {
        let dir = unsafe { state(b"/\0").unwrap() };
        assert!(unsafe { allocate_with(dir, |_| ptr::null_mut()) }.is_null());
        assert_eq!(errno::get_errno(), errno::ENOMEM);
        let dir = unsafe { state(b"/\0").unwrap() };
        let stream = unsafe { allocate_with(dir, |size| crate::alloc::malloc(size)) };
        assert!(!stream.is_null());
        errno::set_errno(123);
        assert!(unsafe { readdir(stream) }.is_null());
        assert_eq!(unsafe { closedir(stream) }, 0);
        assert_eq!(errno::get_errno(), 123);
        assert!(unsafe { readdir(ptr::null_mut()) }.is_null());
        assert_eq!(errno::get_errno(), errno::EBADF);
        assert_eq!(unsafe { closedir(ptr::null_mut()) }, -1);
        assert_eq!(errno::get_errno(), errno::EBADF);
    }

    #[test]
    fn refill_errors_bad_records_and_overflow_leave_the_offset_unchanged() {
        let mut dir = unsafe { state(b"/\0").unwrap() };
        dir.eof = false;
        assert!(dir.next_with(|_, _, _| Err(Errno::Access)).is_null());
        assert_eq!(errno::get_errno(), errno::EACCES);
        for bad in 0..5 {
            assert!(dir
                .next_with(|_, out, _| {
                    if bad == 0 {
                        return Ok(2);
                    }
                    out[0].name_len = match bad {
                        1 => 0,
                        2 => 65,
                        _ => 1,
                    };
                    out[0].name[0] = if bad == 3 { 0 } else { b'/' };
                    Ok(1)
                })
                .is_null());
            assert_eq!(errno::get_errno(), errno::EIO);
            assert_eq!(dir.offset, 0);
        }
        dir.offset = usize::MAX;
        assert!(dir.next_with(|_, out, _| entry(b"real", out)).is_null());
        assert_eq!(dir.offset, usize::MAX);
        assert_eq!(errno::get_errno(), errno::EIO);
    }

    #[test]
    fn layouts_cwd_failure_and_native_error_mapping() {
        assert_eq!(core::mem::size_of::<CDirent>(), 65);
        assert_eq!(core::mem::align_of::<CDirent>(), 1);
        assert_eq!(core::mem::size_of::<DirEntry>(), 80);
        for (error, code) in [
            (Errno::NoEntry, errno::ENOENT),
            (Errno::Access, errno::EACCES),
            (Errno::Failed, errno::EIO),
        ] {
            let result = unsafe {
                prepare_with(
                    b".\0".as_ptr(),
                    |_| Err(error),
                    |_| panic!("stat after failed cwd"),
                    |_, _, _| panic!("read"),
                )
            };
            match result {
                Err(e) => fail(e),
                Ok(_) => panic!("failed cwd became success"),
            }
            assert_eq!(errno::get_errno(), code);
        }
    }
}
