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
pub const SYS_PIPE: u64 = 47;
pub const SYS_READDIR: u64 = 60;
pub const SYS_STAT: u64 = 61;
pub const SYS_MKDIR: u64 = 62;
pub const SYS_SYSINFO: u64 = 82;
pub const SYS_SETNICE: u64 = 83;
pub const SYS_GETNICE: u64 = 84;
pub const SYS_SWAPCTL: u64 = 85;
pub const SYS_DEBUG_LOG: u64 = 99;

/// Raw error return from the kernel.
pub const ERR_RAW: u64 = u64::MAX;
/// Raw "try again" return from the kernel.
pub const EAGAIN_RAW: u64 = u64::MAX - 1;

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
