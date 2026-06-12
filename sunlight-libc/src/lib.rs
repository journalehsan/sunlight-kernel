#![no_std]

//! Minimal userland libc for SunlightOS (the lucerna role in Luxos).
//!
//! Thin safe wrappers over the kernel's SYSCALL ABI: syscall number in `rax`,
//! arguments in `rdi`/`rsi`/`rdx`, return value in `rax`. `u64::MAX` means
//! error; `u64::MAX - 1` means try again (EAGAIN). Syscall numbers must stay
//! in sync with `SunlightSyscall` in `kernel/src/arch/x86_64/syscall.rs`.

pub mod sys;

pub use sys::{Errno, EAGAIN_RAW, ERR_RAW};

/// A userland file descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fd(pub u32);

pub const STDIN: Fd = Fd(0);
pub const STDOUT: Fd = Fd(1);
pub const STDERR: Fd = Fd(2);

/// Maximum path length accepted by kernel `read_user_cstr` callers.
pub const MAX_PATH: usize = 256;
/// Maximum argv entries the kernel reads in `sys_exec` (one slot is the NULL).
pub const MAX_ARGS: usize = 15;
const ARG_ARENA: usize = 1024;

/// Copy `bytes` into `buf` as a NUL-terminated C string.
fn cstr<'a>(buf: &'a mut [u8], bytes: &[u8]) -> Result<*const u8, Errno> {
    if bytes.len() + 1 > buf.len() || bytes.contains(&0) {
        return Err(Errno::Inval);
    }
    buf[..bytes.len()].copy_from_slice(bytes);
    buf[bytes.len()] = 0;
    Ok(buf.as_ptr())
}

/// Open a file by absolute path. Flags/mode are reserved (pass 0 in the ABI).
pub fn open(path: &[u8]) -> Result<Fd, Errno> {
    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;
    let ret = unsafe { sys::syscall3(sys::SYS_OPEN, path_ptr as u64, 0, 0) };
    sys::check(ret).map(|fd| Fd(fd as u32))
}

pub fn close(fd: Fd) -> Result<(), Errno> {
    let ret = unsafe { sys::syscall1(sys::SYS_CLOSE, fd.0 as u64) };
    sys::check(ret).map(|_| ())
}

pub fn read(fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
    let ret = unsafe {
        sys::syscall3(sys::SYS_READ, fd.0 as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
    };
    sys::check(ret).map(|n| n as usize)
}

pub fn write(fd: Fd, buf: &[u8]) -> Result<usize, Errno> {
    let ret = unsafe {
        sys::syscall3(sys::SYS_WRITE, fd.0 as u64, buf.as_ptr() as u64, buf.len() as u64)
    };
    sys::check(ret).map(|n| n as usize)
}

/// Replace the current process image. On success the kernel switches to the
/// new image at the next reschedule, so a `Ok(())` return means "accepted".
pub fn exec(path: &[u8], argv: &[&[u8]]) -> Result<(), Errno> {
    if argv.len() > MAX_ARGS {
        return Err(Errno::TooBig);
    }

    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;

    // argv strings packed into one arena, pointer table NULL-terminated.
    let mut arena = [0u8; ARG_ARENA];
    let mut ptrs = [core::ptr::null::<u8>(); MAX_ARGS + 1];
    let mut used = 0usize;
    for (i, arg) in argv.iter().enumerate() {
        let end = used + arg.len() + 1;
        if end > arena.len() || arg.contains(&0) {
            return Err(Errno::TooBig);
        }
        arena[used..used + arg.len()].copy_from_slice(arg);
        arena[end - 1] = 0;
        ptrs[i] = arena[used..].as_ptr();
        used = end;
    }

    let ret = unsafe {
        sys::syscall3(sys::SYS_EXEC, path_ptr as u64, ptrs.as_ptr() as u64, 0)
    };
    sys::check(ret).map(|_| ())
}

pub fn getpid() -> u64 {
    unsafe { sys::syscall0(sys::SYS_GETPID) }
}

pub fn exit(code: u64) -> ! {
    unsafe {
        sys::syscall1(sys::SYS_PROCESS_EXIT, code);
    }
    // The kernel never returns from ProcessExit; satisfy the type system.
    loop {
        core::hint::spin_loop();
    }
}
