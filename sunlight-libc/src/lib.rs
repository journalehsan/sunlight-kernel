#![no_std]

//! Minimal userland libc for SunlightOS (the lucerna role in Luxos).
//!
//! Thin safe wrappers over the kernel's SYSCALL ABI: syscall number in `rax`,
//! arguments in `rdi`/`rsi`/`rdx`, return value in `rax`. `u64::MAX` means
//! error; `u64::MAX - 1` means try again (EAGAIN). Syscall numbers must stay
//! in sync with `SunlightSyscall` in `kernel/src/arch/x86_64/syscall.rs`.

// ── Core syscall layer ───────────────────────────────────────────────────────
pub mod rand;
pub mod sys;

// ── libc extension modules ───────────────────────────────────────────────────

/// Phase 1 allocator: static bump allocator + C ABI malloc/free/realloc/calloc.
/// Enable `#[global_allocator]` for the `alloc` crate via the `global-alloc` feature.
pub mod alloc;
/// Program startup ABI documentation and raw argv helpers.
pub mod crt0;
/// Environment variable access via the SysV envp pointer.
pub mod env;
/// POSIX errno storage and `__errno_location()`.
pub mod errno;
/// File descriptor helpers: `lseek`, `fstat`, `isatty`.
pub mod fd;
/// Launch-trace argv parsing for GUI apps.
pub mod launch_trace;
/// Memory utility functions (memcpy, memmove, memset, memcmp).
pub mod mem;
/// POSIX memory mapping wrappers (`mmap`, `munmap`).
pub mod mman;
/// Power management: shutdown/reboot via the kernel `PowerCtl` syscall.
pub mod power;
/// Canonical user-space app launch resolution and tracing.
pub mod sun_exec;
/// Global file-open resolver (extension MIME + default app associations).
pub mod sun_open;
pub mod secret_store;
/// Strict, fail-closed configuration loader for the future SSH daemon.
pub mod ssh_config;
/// Native thread spawning.
pub mod thread;
/// Minimal time support: `clock_gettime` backed by the kernel clock syscall.
pub mod time;
/// Thread-Local Storage bootstrap: `Tcb` layout + `init_tls()`.
pub mod tls;

pub use rand::{getrandom, GRND_NONCRYPTO};
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
pub const O_RDONLY: u64 = 0x0;
pub const O_WRONLY: u64 = 0x1;
pub const O_RDWR: u64 = 0x2;
pub const O_CREAT: u64 = 0x40;
pub const O_EXCL: u64 = 0x80;
pub const O_TRUNC: u64 = 0x200;
pub const O_APPEND: u64 = 0x400;
pub const O_NOFOLLOW: u64 = 0x0002_0000;
pub const O_CLOEXEC: u64 = 0x0008_0000;
pub const SEEK_SET: i32 = 0;
pub const SEEK_CUR: i32 = 1;
pub const SEEK_END: i32 = 2;

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

pub fn open_with_flags_mode(path: &[u8], flags: u64, mode: u16) -> Result<Fd, Errno> {
    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;
    let ret = unsafe { sys::syscall3(sys::SYS_OPEN, path_ptr as u64, flags, mode as u64) };
    sys::check(ret).map(|fd| Fd(fd as u32))
}

pub fn open_with_flags(path: &[u8], flags: u64) -> Result<Fd, Errno> {
    open_with_flags_mode(path, flags, 0)
}

pub fn create(path: &[u8]) -> Result<Fd, Errno> {
    open_with_flags_mode(path, O_WRONLY | O_CREAT, 0o644)
}

pub fn close(fd: Fd) -> Result<(), Errno> {
    let ret = unsafe { sys::syscall1(sys::SYS_CLOSE, fd.0 as u64) };
    sys::check(ret).map(|_| ())
}

pub fn read(fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
    let ret = unsafe {
        sys::syscall3(
            sys::SYS_READ,
            fd.0 as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        )
    };
    sys::check(ret).map(|n| n as usize)
}

pub fn write(fd: Fd, buf: &[u8]) -> Result<usize, Errno> {
    let ret = unsafe {
        sys::syscall3(
            sys::SYS_WRITE,
            fd.0 as u64,
            buf.as_ptr() as u64,
            buf.len() as u64,
        )
    };
    sys::check(ret).map(|n| n as usize)
}

pub fn lseek(fd: Fd, offset: i64, whence: i32) -> Result<u64, Errno> {
    let ret = unsafe { sys::syscall3(sys::SYS_LSEEK, fd.0 as u64, offset as u64, whence as u64) };
    sys::check(ret)
}

pub fn fstat(fd: Fd) -> Result<Stat, Errno> {
    let mut out = Stat::zeroed();
    let ret = unsafe { sys::syscall2(sys::SYS_FSTAT, fd.0 as u64, (&mut out as *mut Stat) as u64) };
    sys::check(ret).map(|_| out)
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

    let ret = unsafe { sys::syscall3(sys::SYS_EXEC, path_ptr as u64, ptrs.as_ptr() as u64, 0) };
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

impl Stat {
    pub const fn zeroed() -> Self {
        Self {
            size: 0,
            uid: 0,
            gid: 0,
            mode: 0,
            file_type: 0,
            _pad: 0,
            nlinks: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SysInfo {
    pub total_ram_kb: u64,
    pub used_ram_kb: u64,
    pub uptime_secs: u64,
    pub unix_time: u64,
    pub swap_total_kb: u64,
    pub swap_used_kb: u64,
    pub swap_compressed_kb: u64,
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
            core::mem::size_of_val(entries) as u64,
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

pub fn unlink(path: &[u8]) -> Result<(), Errno> {
    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;
    let ret = unsafe { sys::syscall1(sys::SYS_UNLINK, path_ptr as u64) };
    sys::check(ret).map(|_| ())
}

pub fn rename(old: &[u8], new: &[u8]) -> Result<(), Errno> {
    let mut old_buf = [0u8; MAX_PATH];
    let mut new_buf = [0u8; MAX_PATH];
    let old_ptr = cstr(&mut old_buf, old)?;
    let new_ptr = cstr(&mut new_buf, new)?;
    let ret = unsafe { sys::syscall2(sys::SYS_RENAME, old_ptr as u64, new_ptr as u64) };
    sys::check(ret).map(|_| ())
}

pub fn chmod(path: &[u8], mode: u16) -> Result<(), Errno> {
    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;
    let ret = unsafe { sys::syscall2(sys::SYS_CHMOD, path_ptr as u64, mode as u64) };
    sys::check(ret).map(|_| ())
}

pub fn chown(path: &[u8], uid: u32, gid: u32) -> Result<(), Errno> {
    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;
    let ret = unsafe { sys::syscall3(sys::SYS_CHOWN, path_ptr as u64, uid as u64, gid as u64) };
    sys::check(ret).map(|_| ())
}

pub(crate) fn secret_create_temp(path: &[u8], mode: u16) -> Result<Fd, Errno> {
    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;
    let ret = unsafe {
        sys::syscall2(
            sys::SYS_SECRET_CREATE,
            path_ptr as u64,
            mode as u64,
        )
    };
    sys::check(ret).map(|fd| Fd(fd as u32))
}

pub(crate) fn secret_publish(
    temporary: &[u8],
    destination: &[u8],
    mode: u16,
    replace: bool,
) -> Result<(), Errno> {
    let mut temporary_buf = [0u8; MAX_PATH];
    let mut destination_buf = [0u8; MAX_PATH];
    let temporary_ptr = cstr(&mut temporary_buf, temporary)?;
    let destination_ptr = cstr(&mut destination_buf, destination)?;
    let ret = unsafe {
        sys::syscall4(
            sys::SYS_SECRET_PUBLISH,
            temporary_ptr as u64,
            destination_ptr as u64,
            mode as u64,
            replace as u64,
        )
    };
    sys::check(ret).map(|_| ())
}

pub(crate) fn secret_remove_temp(path: &[u8]) -> Result<(), Errno> {
    let mut path_buf = [0u8; MAX_PATH];
    let path_ptr = cstr(&mut path_buf, path)?;
    let ret = unsafe { sys::syscall1(sys::SYS_SECRET_REMOVE_TEMP, path_ptr as u64) };
    sys::check(ret).map(|_| ())
}

pub fn sysinfo() -> Result<SysInfo, Errno> {
    let mut raw = [0u64; 7];
    let ret = unsafe { sys::syscall1(sys::SYS_SYSINFO, raw.as_mut_ptr() as u64) };
    sys::check(ret).map(|_| SysInfo {
        total_ram_kb: raw[0],
        used_ram_kb: raw[1],
        uptime_secs: raw[2],
        unix_time: raw[3],
        swap_total_kb: raw[4],
        swap_used_kb: raw[5],
        swap_compressed_kb: raw[6],
    })
}

/// Swap out a caller-owned anonymous range through the normal MM lifecycle.
pub fn freezram_fill(
    start_address: *mut u8,
    requested_pages: u64,
) -> Result<sunlight_ipc::swap_policy::FillDiagnostics, Errno> {
    let mut result = sunlight_ipc::swap_policy::FillDiagnostics::default();
    let ret = unsafe {
        sys::syscall6(
            sys::SYS_SWAPCTL,
            0,
            start_address as u64,
            requested_pages,
            (&mut result as *mut sunlight_ipc::swap_policy::FillDiagnostics) as u64,
            core::mem::size_of::<sunlight_ipc::swap_policy::FillDiagnostics>() as u64,
            0,
        )
    };
    sys::check(ret).map(|_| result)
}

/// Create an anonymous pipe; returns (read_end, write_end).
pub fn pipe() -> Result<(Fd, Fd), Errno> {
    let mut fds = [0i32; 2];
    let ret = unsafe { sys::syscall1(sys::SYS_PIPE, fds.as_mut_ptr() as u64) };
    sys::check(ret).map(|_| (Fd(fds[0] as u32), Fd(fds[1] as u32)))
}

/// Create a pseudo-terminal pair by asking `pty_server` for a fresh session.
///
/// Returns `(master_cap, slave_cap)`:
/// - `master_cap` is used by the GUI terminal emulator.
/// - `slave_cap` is the endpoint `sshl` should attach to as its stdio.
///
/// The current IPC transport can carry two capability tokens in the reply
/// message, which is enough for the PTY broker to hand out both ends without
/// a second round trip.
pub fn openpty() -> Result<(sunlight_ipc::CapabilityToken, sunlight_ipc::CapabilityToken), Errno> {
    let Some(pty_cap) = sunlight_ipc::nameserver_lookup_timeout("pty", 100) else {
        return Err(Errno::Again);
    };

    let req = sunlight_ipc::IpcMsg::with_label(sunlight_ipc::PtyMsg::CREATE).word(
        0,
        sunlight_ipc::PtyMsg::FLAG_CANONICAL | sunlight_ipc::PtyMsg::FLAG_ECHO,
    );
    let reply = match sunlight_ipc::ipc_call_timeout(pty_cap, req, 1000) {
        Ok(reply) => reply,
        Err(
            sunlight_ipc::IpcCallError::Timeout
            | sunlight_ipc::IpcCallError::QueueFull
            | sunlight_ipc::IpcCallError::Cancelled,
        ) => return Err(Errno::Again),
        Err(sunlight_ipc::IpcCallError::PeerClosed) => return Err(Errno::Failed),
        Err(_) => return Err(Errno::Failed),
    };

    if reply.label != sunlight_ipc::PtyMsg::REPLY
        || reply.cap_count < 2
        || reply.caps[0] == sunlight_ipc::CapabilityToken::INVALID
        || reply.caps[1] == sunlight_ipc::CapabilityToken::INVALID
    {
        return Err(Errno::Failed);
    }

    Ok((reply.caps[0], reply.caps[1]))
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
        sys::syscall3(
            sys::SYS_SPAWN,
            path_ptr as u64,
            ptrs.as_ptr() as u64,
            stdout_arg,
        )
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

pub fn kill(pid: u64, sig: u32) -> Result<(), Errno> {
    let ret = unsafe { sys::syscall2(sys::SYS_KILL, pid, sig as u64) };
    sys::check(ret).map(|_| ())
}

pub fn map_telemetry() -> Result<*const u8, Errno> {
    let ret = unsafe { sys::syscall0(sys::SYS_MAP_TELEMETRY) };
    sys::check(ret).map(|addr| addr as *const u8)
}

/// Write all bytes in `buf` to `fd`, looping over partial writes.
pub fn write_all(fd: Fd, mut buf: &[u8]) -> Result<(), Errno> {
    while !buf.is_empty() {
        let n = write(fd, buf)?;
        if n == 0 {
            return Err(Errno::Failed);
        }
        buf = &buf[n..];
    }
    Ok(())
}

/// Create all missing directory components of `path` (create_dir_all semantics).
/// Failures on intermediate components are silently ignored (they likely exist).
pub fn mkdir_recursive(path: &[u8]) -> Result<(), Errno> {
    if path.is_empty() {
        return Err(Errno::Inval);
    }
    let mut i = 1usize;
    while i <= path.len() {
        if i == path.len() || path[i] == b'/' {
            let _ = mkdir(&path[..i], 0o755);
        }
        i += 1;
    }
    Ok(())
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
