//! Raw syscall plumbing: numbers and inline-assembly stubs.

// Mirror of `SunlightSyscall` (kernel/src/arch/x86_64/syscall.rs).
pub const SYS_PROCESS_EXIT: u64 = 20;
pub const SYS_PROCESS_YIELD: u64 = 21;
pub const SYS_FORK: u64 = 30;
pub const SYS_EXEC: u64 = 31;
pub const SYS_WAITPID: u64 = 32;
pub const SYS_GETPID: u64 = 33;
pub const SYS_GETUID: u64 = 35;
pub const SYS_GETGID: u64 = 36;
pub const SYS_SPAWN: u64 = 39;
pub const SYS_OPEN: u64 = 40;
pub const SYS_CLOSE: u64 = 41;
pub const SYS_READ: u64 = 42;
pub const SYS_WRITE: u64 = 43;
pub const SYS_LSEEK: u64 = 44;
pub const SYS_PIPE: u64 = 47;
pub const SYS_FSTAT: u64 = 48;
pub const SYS_MMAP: u64 = 50;
pub const SYS_MUNMAP: u64 = 51;
pub const SYS_MPROTECT: u64 = 52;
pub const SYS_MREMAP: u64 = 53;
pub const SYS_READDIR: u64 = 60;
pub const SYS_STAT: u64 = 61;
pub const SYS_MKDIR: u64 = 62;
pub const SYS_UNLINK: u64 = 65;
pub const SYS_RENAME: u64 = 66;
pub const SYS_CHMOD: u64 = 67;
pub const SYS_CHOWN: u64 = 68;
pub const SYS_SECRET_CREATE: u64 = 69;
pub const SYS_KILL: u64 = 72;
pub const SYS_SECRET_PUBLISH: u64 = 75;
pub const SYS_SECRET_REMOVE_TEMP: u64 = 76;
pub const SYS_SYSINFO: u64 = 82;
pub const SYS_SETNICE: u64 = 83;
pub const SYS_GETNICE: u64 = 84;
pub const SYS_SWAPCTL: u64 = 85;
pub const SYS_GET_ENTROPY: u64 = 87;
pub const SYS_SECURE_ENTROPY_READY: u64 = 89;
pub const SYS_CLOCK_GETTIME: u64 = 88;
pub const SYS_MAP_TELEMETRY: u64 = 95;
pub const SYS_GRANT_CAPABILITY: u64 = 100;
pub const SYS_DEBUG_LOG: u64 = 99;
pub const SYS_SET_FS_BASE: u64 = 101;
pub const SYS_MINT_AUTH_SESSION_GRANT: u64 = 102;

/// Raw error return from the kernel.
pub const ERR_RAW: u64 = u64::MAX;
/// Raw "try again" return from the kernel.
pub const EAGAIN_RAW: u64 = u64::MAX - 1;

/// Largest count that can be represented by the public `ssize_t` result.
/// Native SunlightOS is currently x86_64, but keep this tied to the Rust ABI
/// rather than assuming every future libc target has a 64-bit `usize`.
pub const MAX_IO_COUNT: usize = isize::MAX as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Errno {
    /// Generic kernel failure (the ABI does not carry a code yet).
    Failed,
    /// Operation would block; retry.
    Again,
    /// Invalid argument built in userspace (bad string, embedded NUL).
    Inval,
    /// Argument list or path too large for the fixed marshalling buffers.
    TooBig,
}

pub fn check(ret: u64) -> Result<u64, Errno> {
    match ret {
        ERR_RAW => Err(Errno::Failed),
        EAGAIN_RAW => Err(Errno::Again),
        n => Ok(n),
    }
}

/// Decode a byte-count syscall result without allowing a malformed kernel or
/// service result to escape as a larger Rust slice length.  Raw read/write are
/// deliberately single-shot: a short non-error count is progress and is
/// returned to the caller unchanged.
pub fn check_io_count(ret: u64, requested: usize) -> Result<usize, Errno> {
    if requested > MAX_IO_COUNT {
        return Err(Errno::TooBig);
    }
    let count = check(ret)?;
    let count = usize::try_from(count).map_err(|_| Errno::Failed)?;
    if count > requested || count > MAX_IO_COUNT {
        return Err(Errno::Failed);
    }
    Ok(count)
}
/// # Safety
/// SYSCALL clobbers rcx (return RIP) and r11 (RFLAGS); the kernel preserves
/// the remaining GPRs by saving a full frame on entry.
#[inline]
pub unsafe fn syscall0(n: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        in("rax") n,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}
/// # Safety
#[inline]
pub unsafe fn syscall1(n: u64, a1: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        in("rax") n,
        in("rdi") a1,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}
/// # Safety
#[inline]
pub unsafe fn syscall2(n: u64, a1: u64, a2: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        in("rax") n,
        in("rdi") a1,
        in("rsi") a2,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}
/// # Safety
#[inline]
pub unsafe fn syscall3(n: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        in("rax") n,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

/// # Safety
/// The SysV AMD64 syscall ABI passes the fourth argument in r10.
#[inline]
pub unsafe fn syscall4(n: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        in("rax") n,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

/// # Safety
/// SYSCALL clobbers rcx (return RIP) and r11 (RFLAGS); the kernel preserves
/// the remaining GPRs by saving a full frame on entry.
/// The SysV AMD64 syscall ABI passes the 4th argument in r10, not rcx.
#[inline]
pub unsafe fn syscall6(n: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        in("rax") n,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        in("r8") a5,
        in("r9") a6,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_sentinel_is_never_a_successful_count() {
        assert_eq!(check(ERR_RAW), Err(Errno::Failed));
        assert_eq!(check(EAGAIN_RAW), Err(Errno::Again));
        assert_eq!(check(17), Ok(17));
    }

    #[test]
    fn io_counts_reject_impossible_or_truncated_results() {
        assert_eq!(check_io_count(0, 0), Ok(0));
        assert_eq!(check_io_count(3, 3), Ok(3));
        assert_eq!(check_io_count(4, 3), Err(Errno::Failed));
        assert_eq!(check_io_count(ERR_RAW, 3), Err(Errno::Failed));
        assert_eq!(check_io_count(EAGAIN_RAW, 3), Err(Errno::Again));
    }
}
