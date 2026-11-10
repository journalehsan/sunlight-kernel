//! Native C stdio. All streams are unbuffered, including stdout/stderr. A
//! successful fflush means there are no userspace bytes pending, not fsync.
//! The per-stream lock covers an entire operation (including formatted output).
//! As in C, callers must supply live streams and valid pointers and must not
//! race fclose against another use. Host probes inject only the syscall layer.
use crate::{errno, sys::Errno, Fd};
use core::cell::UnsafeCell;
use core::ffi::{c_int, c_long, c_void, VaList};
use core::sync::atomic::{AtomicBool, Ordering};

pub const EOF: c_int = -1;
struct State {
    fd: Fd,
    readable: bool,
    writable: bool,
    eof: bool,
    error: bool,
    closed: bool,
}
#[repr(C)]
pub struct SunlightFile {
    lock: AtomicBool,
    state: UnsafeCell<State>,
}
// State is accessible only while holding this stream's lock.
unsafe impl Sync for SunlightFile {}
impl SunlightFile {
    const fn new(fd: u32, readable: bool, writable: bool) -> Self {
        Self {
            lock: AtomicBool::new(false),
            state: UnsafeCell::new(State {
                fd: Fd(fd),
                readable,
                writable,
                eof: false,
                error: false,
                closed: false,
            }),
        }
    }
}
static INPUT: SunlightFile = SunlightFile::new(0, true, false);
static OUTPUT: SunlightFile = SunlightFile::new(1, false, true);
static ERROR: SunlightFile = SunlightFile::new(2, false, true);
#[allow(non_upper_case_globals)]
#[cfg_attr(not(test), no_mangle)]
pub static mut stdin: *mut SunlightFile = &INPUT as *const _ as *mut _;
#[allow(non_upper_case_globals)]
#[cfg_attr(not(test), no_mangle)]
pub static mut stdout: *mut SunlightFile = &OUTPUT as *const _ as *mut _;
#[allow(non_upper_case_globals)]
#[cfg_attr(not(test), no_mangle)]
pub static mut stderr: *mut SunlightFile = &ERROR as *const _ as *mut _;

struct Guard<'a>(&'a SunlightFile);
impl core::ops::Deref for Guard<'_> {
    type Target = State;
    fn deref(&self) -> &State {
        unsafe { &*self.0.state.get() }
    }
}
impl core::ops::DerefMut for Guard<'_> {
    fn deref_mut(&mut self) -> &mut State {
        unsafe { &mut *self.0.state.get() }
    }
}
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.lock.store(false, Ordering::Release);
    }
}
unsafe fn lock<'a>(file: *mut SunlightFile) -> Option<Guard<'a>> {
    let Some(file) = file.as_ref() else {
        errno::set_errno(errno::EINVAL);
        return None;
    };
    while file
        .lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        crate::yield_now();
    }
    let guard = Guard(file);
    if guard.closed {
        errno::set_errno(errno::EBADF);
        None
    } else {
        Some(guard)
    }
}
fn fail(state: &mut State, error: Errno) {
    state.error = true;
    errno::set_from_errno(error);
}
unsafe fn cbytes<'a>(text: *const u8) -> &'a [u8] {
    let mut len = 0;
    while *text.add(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(text, len)
}
// Do not use sl_std_write: its stderr write_all adapter cannot report partial
// progress on failure. Direct wrappers preserve both progress and errno here.
fn write_bytes(state: &mut State, bytes: &[u8]) -> usize {
    if !state.writable {
        fail(state, Errno::BadFd);
        return 0;
    }
    let mut done = 0;
    while done < bytes.len() {
        let end = bytes
            .len()
            .min(done.saturating_add(crate::sys::MAX_IO_COUNT));
        match crate::write(state.fd, &bytes[done..end]) {
            Ok(n) if n > 0 && n <= end - done => done += n,
            Ok(_) => {
                fail(state, Errno::Failed);
                break;
            }
            Err(e) => {
                fail(state, e);
                break;
            }
        }
    }
    done
}
fn read_bytes(state: &mut State, bytes: &mut [u8]) -> usize {
    if !state.readable {
        fail(state, Errno::BadFd);
        return 0;
    }
    if state.eof {
        return 0;
    }
    let mut done = 0;
    while done < bytes.len() {
        let end = bytes
            .len()
            .min(done.saturating_add(crate::sys::MAX_IO_COUNT));
        match crate::read(state.fd, &mut bytes[done..end]) {
            Ok(0) => {
                state.eof = true;
                break;
            }
            Ok(n) if n <= end - done => done += n,
            Ok(_) => {
                fail(state, Errno::Failed);
                break;
            }
            Err(e) => {
                fail(state, e);
                break;
            }
        }
    }
    done
}
fn mode_flags(mode: &[u8]) -> Option<(u64, bool, bool)> {
    let first = *mode.first()?;
    let mut plus = false;
    let mut binary = false;
    let mut exclusive = false;
    for &ch in &mode[1..] {
        match ch {
            b'+' if !plus => plus = true,
            b'b' if !binary => binary = true,
            b'x' if first == b'w' && !exclusive => exclusive = true,
            _ => return None,
        }
    }
    let flags = match first {
        b'r' => {
            if plus {
                crate::O_RDWR
            } else {
                crate::O_RDONLY
            }
        }
        b'w' => {
            (if plus { crate::O_RDWR } else { crate::O_WRONLY }) | crate::O_CREAT | crate::O_TRUNC
        }
        b'a' => {
            (if plus { crate::O_RDWR } else { crate::O_WRONLY }) | crate::O_CREAT | crate::O_APPEND
        }
        _ => return None,
    };
    Some((
        flags | if exclusive { crate::O_EXCL } else { 0 },
        first == b'r' || plus,
        first != b'r' || plus,
    ))
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fopen(path: *const u8, mode: *const u8) -> *mut SunlightFile {
    if path.is_null() || mode.is_null() {
        errno::set_errno(errno::EINVAL);
        return core::ptr::null_mut();
    }
    let Some((flags, read, write)) = mode_flags(cbytes(mode)) else {
        errno::set_errno(errno::EINVAL);
        return core::ptr::null_mut();
    };
    // Allocate first so allocation failure cannot truncate/create a file.
    let file = crate::alloc::malloc(core::mem::size_of::<SunlightFile>()).cast::<SunlightFile>();
    if file.is_null() {
        errno::set_errno(errno::ENOMEM);
        return file;
    }
    match crate::open_with_flags_mode(cbytes(path), flags, 0o666) {
        Ok(fd) => {
            file.write(SunlightFile::new(fd.0, read, write));
            file
        }
        Err(e) => {
            crate::alloc::free(file.cast());
            errno::set_from_errno(e);
            core::ptr::null_mut()
        }
    }
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn tmpfile() -> *mut SunlightFile {
    // /tmp is supplied by the native boot tmpfs. Do not redirect this to an
    // arbitrary environment path whose filesystem may lack open-unlink lifetime.
    // Reuse mkstemp's bounded exclusive creation; no name secrecy is promised.
    let file = crate::alloc::malloc(core::mem::size_of::<SunlightFile>()).cast::<SunlightFile>();
    if file.is_null() {
        errno::set_errno(errno::ENOMEM);
        return file;
    }
    let mut path = *b"/tmp/.sunlight-stdio.XXXXXX";
    let fd = match crate::temp::mkstemp(&mut path) {
        Ok(fd) => fd,
        Err(error) => {
            crate::alloc::free(file.cast());
            errno::set_from_errno(error);
            return core::ptr::null_mut();
        }
    };
    if let Err(error) = crate::unlink(&path) {
        // Close consumes the descriptor even on error. Preserve unlink's
        // error across both cleanup operations; never return a named stream.
        let _ = crate::close(fd);
        crate::alloc::free(file.cast());
        errno::set_from_errno(error);
        return core::ptr::null_mut();
    }
    file.write(SunlightFile::new(fd.0, true, true));
    file
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fclose(file: *mut SunlightFile) -> c_int {
    let Some(mut state) = lock(file) else {
        return EOF;
    };
    let result = crate::close(state.fd);
    // Native close consumes the descriptor even when backend cleanup fails.
    state.closed = true;
    drop(state);
    if !core::ptr::eq(file, &INPUT) && !core::ptr::eq(file, &OUTPUT) && !core::ptr::eq(file, &ERROR)
    {
        crate::alloc::free(file.cast());
    }
    match result {
        Ok(()) => 0,
        Err(e) => {
            errno::set_from_errno(e);
            EOF
        }
    }
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fread(
    ptr: *mut c_void,
    size: usize,
    count: usize,
    file: *mut SunlightFile,
) -> usize {
    if size == 0 || count == 0 {
        return 0;
    }
    let Some(mut state) = lock(file) else {
        return 0;
    };
    let Some(len) = size
        .checked_mul(count)
        .filter(|&n| n <= isize::MAX as usize)
    else {
        fail(&mut state, Errno::Inval);
        return 0;
    };
    if ptr.is_null() {
        fail(&mut state, Errno::Inval);
        return 0;
    }
    read_bytes(&mut state, core::slice::from_raw_parts_mut(ptr.cast(), len)) / size
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fwrite(
    ptr: *const c_void,
    size: usize,
    count: usize,
    file: *mut SunlightFile,
) -> usize {
    if size == 0 || count == 0 {
        return 0;
    }
    let Some(mut state) = lock(file) else {
        return 0;
    };
    let Some(len) = size
        .checked_mul(count)
        .filter(|&n| n <= isize::MAX as usize)
    else {
        fail(&mut state, Errno::Inval);
        return 0;
    };
    if ptr.is_null() {
        fail(&mut state, Errno::Inval);
        return 0;
    }
    write_bytes(&mut state, core::slice::from_raw_parts(ptr.cast(), len)) / size
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fflush(file: *mut SunlightFile) -> c_int {
    if file.is_null() {
        return 0;
    } // All streams are unbuffered.
    if lock(file).is_some() {
        0
    } else {
        EOF
    }
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fileno(file: *mut SunlightFile) -> c_int {
    lock(file).map_or(EOF, |state| state.fd.0 as c_int)
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fgetc(file: *mut SunlightFile) -> c_int {
    let Some(mut state) = lock(file) else {
        return EOF;
    };
    let mut byte = [0];
    if read_bytes(&mut state, &mut byte) == 1 {
        byte[0] as c_int
    } else {
        EOF
    }
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getc(file: *mut SunlightFile) -> c_int {
    fgetc(file)
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getchar() -> c_int {
    fgetc(stdin)
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fputc(ch: c_int, file: *mut SunlightFile) -> c_int {
    let Some(mut state) = lock(file) else {
        return EOF;
    };
    let byte = ch as u8;
    if write_bytes(&mut state, &[byte]) == 1 {
        byte as c_int
    } else {
        EOF
    }
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn putc(ch: c_int, file: *mut SunlightFile) -> c_int {
    fputc(ch, file)
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn putchar(ch: c_int) -> c_int {
    fputc(ch, stdout)
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fputs(text: *const u8, file: *mut SunlightFile) -> c_int {
    let Some(mut state) = lock(file) else {
        return EOF;
    };
    if text.is_null() {
        fail(&mut state, Errno::Inval);
        return EOF;
    }
    let bytes = cbytes(text);
    if write_bytes(&mut state, bytes) == bytes.len() && state.writable {
        0
    } else {
        EOF
    }
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn puts(text: *const u8) -> c_int {
    let Some(mut state) = lock(stdout) else {
        return EOF;
    };
    if text.is_null() {
        fail(&mut state, Errno::Inval);
        return EOF;
    }
    let bytes = cbytes(text);
    if write_bytes(&mut state, bytes) != bytes.len() {
        return EOF;
    }
    if write_bytes(&mut state, b"\n") == 1 {
        0
    } else {
        EOF
    }
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fgets(text: *mut u8, size: c_int, file: *mut SunlightFile) -> *mut u8 {
    let Some(mut state) = lock(file) else {
        return core::ptr::null_mut();
    };
    if text.is_null() || size <= 0 {
        fail(&mut state, Errno::Inval);
        return core::ptr::null_mut();
    }
    let mut used = 0;
    while used < size as usize - 1 {
        let mut byte = [0];
        if read_bytes(&mut state, &mut byte) == 0 {
            // EOF can coexist with an older error indicator. Only this
            // read failing (rather than reaching EOF) invalidates the line.
            if !state.eof || used == 0 {
                return core::ptr::null_mut();
            }
            break;
        }
        text.add(used).write(byte[0]);
        used += 1;
        if byte[0] == b'\n' {
            break;
        }
    }
    text.add(used).write(0);
    text
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn feof(file: *mut SunlightFile) -> c_int {
    lock(file).map_or(0, |s| s.eof as c_int)
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn ferror(file: *mut SunlightFile) -> c_int {
    lock(file).map_or(0, |s| s.error as c_int)
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn clearerr(file: *mut SunlightFile) {
    if let Some(mut s) = lock(file) {
        s.eof = false;
        s.error = false;
    }
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fseek(file: *mut SunlightFile, offset: c_long, whence: c_int) -> c_int {
    let Some(mut state) = lock(file) else {
        return -1;
    };
    if !(0..=2).contains(&whence) {
        fail(&mut state, Errno::Inval);
        return -1;
    }
    match crate::lseek(state.fd, offset, whence) {
        Ok(_) => {
            state.eof = false;
            0
        }
        Err(e) => {
            fail(&mut state, e);
            -1
        }
    }
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn ftell(file: *mut SunlightFile) -> c_long {
    let Some(mut state) = lock(file) else {
        return -1;
    };
    match crate::lseek(state.fd, 0, crate::SEEK_CUR) {
        Ok(n) if n <= c_long::MAX as u64 => n as c_long,
        Ok(_) => {
            fail(&mut state, Errno::TooBig);
            -1
        }
        Err(e) => {
            fail(&mut state, e);
            -1
        }
    }
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn rewind(file: *mut SunlightFile) {
    fseek(file, 0, crate::SEEK_SET);
    clearerr(file);
}

#[path = "stdio/format.rs"]
mod format;
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn vfprintf(
    file: *mut SunlightFile,
    fmt: *const u8,
    args: VaList<'_>,
) -> c_int {
    let Some(mut state) = lock(file) else {
        return -1;
    };
    if fmt.is_null() {
        fail(&mut state, Errno::Inval);
        return -1;
    }
    format::print(&mut state, cbytes(fmt), args)
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fprintf(file: *mut SunlightFile, fmt: *const u8, args: ...) -> c_int {
    vfprintf(file, fmt, args)
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn printf(fmt: *const u8, args: ...) -> c_int {
    vfprintf(stdout, fmt, args)
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn vprintf(fmt: *const u8, args: VaList<'_>) -> c_int {
    vfprintf(stdout, fmt, args)
}
