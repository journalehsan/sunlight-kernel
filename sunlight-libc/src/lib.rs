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

pub fn getuid() -> u64 {
    unsafe { sys::syscall0(sys::SYS_GETUID) }
}

pub fn getgid() -> u64 {
    unsafe { sys::syscall0(sys::SYS_GETGID) }
}

pub fn getnice(pid: u64) -> Result<i8, Errno> {
    let ret = unsafe { sys::syscall1(sys::SYS_GETNICE, pid) };
    let value = sys::check(ret)? as i64;
    if (-10..=10).contains(&value) {
        Ok(value as i8)
    } else {
        Err(Errno::Failed)
    }
}

pub fn setnice(pid: u64, nice: i8) -> Result<i8, Errno> {
    let ret = unsafe { sys::syscall2(sys::SYS_SETNICE, pid, nice as i64 as u64) };
    let value = sys::check(ret)? as i64;
    if (-10..=10).contains(&value) {
        Ok(value as i8)
    } else {
        Err(Errno::Failed)
    }
}

/// Yield the CPU to the scheduler.
pub fn yield_now() {
    unsafe {
        sys::syscall0(sys::SYS_PROCESS_YIELD);
    }
}

/// One directory entry as returned by the ReadDir syscall (80 bytes,
/// layout shared with `sys_readdir` in the kernel).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DirEntry {
    pub name: [u8; 64],
    pub name_len: u8,
    pub file_type: u8,
    _pad: [u8; 6],
    pub size: u64,
}

pub const FT_FILE: u8 = 1;
pub const FT_DIR: u8 = 2;

impl DirEntry {
    pub const fn zeroed() -> Self {
        Self {
            name: [0; 64],
            name_len: 0,
            file_type: 0,
            _pad: [0; 6],
            size: 0,
        }
    }

    pub fn name_bytes(&self) -> &[u8] {
        &self.name[..(self.name_len as usize).min(64)]
    }
}

/// File metadata as returned by the StatPath syscall (24 bytes, layout
/// shared with `sys_stat_path` in the kernel).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Stat {
    pub size: u64,
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
    pub file_type: u8,
    _pad: u8,
    pub nlinks: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct SysInfo {
    pub total_ram_kb: u64,
    pub used_ram_kb: u64,
    pub uptime_secs: u64,
    pub unix_time: u64,
    pub swap_total_kb: u64,
    pub swap_used_kb: u64,
}

/// List a directory into `entries`; returns how many were filled.
pub fn read_dir(path: &[u8], entries: &mut [DirEntry]) -> Result<usize, Errno> {
    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;
    let ret = unsafe {
        sys::syscall3(
            sys::SYS_READDIR,
            path_ptr as u64,
            entries.as_mut_ptr() as u64,
            (entries.len() * core::mem::size_of::<DirEntry>()) as u64,
        )
    };
    sys::check(ret).map(|n| n as usize)
}

pub fn stat(path: &[u8]) -> Result<Stat, Errno> {
    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;
    let mut out = Stat {
        size: 0,
        uid: 0,
        gid: 0,
        mode: 0,
        file_type: 0,
        _pad: 0,
        nlinks: 0,
    };
    let ret = unsafe {
        sys::syscall3(
            sys::SYS_STAT,
            path_ptr as u64,
            (&mut out as *mut Stat) as u64,
            0,
        )
    };
    sys::check(ret).map(|_| out)
}

pub fn mkdir(path: &[u8], mode: u16) -> Result<(), Errno> {
    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;
    let ret = unsafe { sys::syscall3(sys::SYS_MKDIR, path_ptr as u64, mode as u64, 0) };
    sys::check(ret).map(|_| ())
}

pub fn sysinfo() -> Result<SysInfo, Errno> {
    let mut raw = [0u64; 6];
    let ret = unsafe { sys::syscall1(sys::SYS_SYSINFO, raw.as_mut_ptr() as u64) };
    sys::check(ret).map(|_| SysInfo {
        total_ram_kb: raw[0],
        used_ram_kb: raw[1],
        uptime_secs: raw[2],
        unix_time: raw[3],
        swap_total_kb: raw[4],
        swap_used_kb: raw[5],
    })
}

/// Create an anonymous pipe; returns (read_end, write_end).
pub fn pipe() -> Result<(Fd, Fd), Errno> {
    let mut fds = [0i32; 2];
    let ret = unsafe { sys::syscall1(sys::SYS_PIPE, fds.as_mut_ptr() as u64) };
    sys::check(ret).map(|_| (Fd(fds[0] as u32), Fd(fds[1] as u32)))
}

/// Spawn a new process running `path` (posix_spawn-style). `stdout` becomes
/// the child's fd 1 when given (e.g. a pipe write end). Returns the child pid.
pub fn spawn(path: &[u8], argv: &[&[u8]], stdout: Option<Fd>) -> Result<u64, Errno> {
    if argv.len() > MAX_ARGS {
        return Err(Errno::TooBig);
    }

    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;

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

    let stdout_arg = stdout.map_or(u64::MAX, |fd| fd.0 as u64);
    let ret = unsafe {
        sys::syscall3(sys::SYS_SPAWN, path_ptr as u64, ptrs.as_ptr() as u64, stdout_arg)
    };
    sys::check(ret)
}

/// Non-blocking wait: Ok(Some(code)) once the child exited, Ok(None) while
/// it is still running, Err for unknown pid.
pub fn try_waitpid(pid: u64) -> Result<Option<u64>, Errno> {
    let ret = unsafe { sys::syscall1(sys::SYS_WAITPID, pid) };
    match sys::check(ret) {
        Ok(code) => Ok(Some(code)),
        Err(Errno::Again) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Blocking wait built on `try_waitpid` + yield.
pub fn waitpid(pid: u64) -> Result<u64, Errno> {
    loop {
        match try_waitpid(pid)? {
            Some(code) => return Ok(code),
            None => yield_now(),
        }
    }
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
