//! Linux x86_64 ABI constants used at the Helios compatibility boundary.
//!
//! These values are the Linux userspace contract. They must not be confused
//! with native SunlightOS syscall numbers, native error sentinels, or
//! similarly named kernel structs.

/// Encode a Linux errno as the `syscall` return value (`-errno`).
pub const fn errno_result(errno: u32) -> u64 {
    0u64.wrapping_sub(errno as u64)
}

pub const EPERM: u32 = 1;
pub const ENOENT: u32 = 2;
pub const ESRCH: u32 = 3;
pub const EIO: u32 = 5;
pub const ENXIO: u32 = 6;
pub const E2BIG: u32 = 7;
pub const EBADF: u32 = 9;
pub const EAGAIN: u32 = 11;
pub const ENOMEM: u32 = 12;
pub const EACCES: u32 = 13;
pub const EFAULT: u32 = 14;
pub const EBUSY: u32 = 16;
pub const EEXIST: u32 = 17;
pub const ENODEV: u32 = 19;
pub const ENOTDIR: u32 = 20;
pub const EISDIR: u32 = 21;
pub const EINVAL: u32 = 22;
pub const ENFILE: u32 = 23;
pub const EMFILE: u32 = 24;
pub const ENOTTY: u32 = 25;
pub const ESPIPE: u32 = 29;
pub const ENAMETOOLONG: u32 = 36;
pub const ENOSYS: u32 = 38;
pub const EPIPE: u32 = 32;
pub const ERANGE: u32 = 34;
pub const EAFNOSUPPORT: u32 = 97;

/// Native SunlightOS syscall error sentinels. These occupy `u64::MAX` down to
/// `u64::MAX - 9` and must never be returned to a Linux process unchanged:
/// musl interprets `-1` as EPERM, `-2` as ENOENT, `-3` as ESRCH, etc.
pub const NATIVE_GENERIC: u64 = u64::MAX;
pub const NATIVE_EAGAIN: u64 = u64::MAX - 1;
pub const NATIVE_ENOENT: u64 = u64::MAX - 2;
pub const NATIVE_EACCES: u64 = u64::MAX - 3;
pub const NATIVE_EBADF: u64 = u64::MAX - 4;
pub const NATIVE_EINVAL: u64 = u64::MAX - 5;
pub const NATIVE_EISDIR: u64 = u64::MAX - 6;
pub const NATIVE_ENOTDIR: u64 = u64::MAX - 7;
pub const NATIVE_EIO: u64 = u64::MAX - 8;
pub const NATIVE_ERANGE: u64 = u64::MAX - 9;

/// Translate a native SunlightOS syscall result into a Linux `-errno` result.
/// Success values (including user pointers and file descriptors) pass through.
/// Already-encoded Linux errno values (`-(11..=132)`) also pass through because
/// they sit below the native sentinel window.
pub const fn from_native_result(result: u64) -> u64 {
    match result {
        NATIVE_GENERIC => errno_result(EPERM),
        NATIVE_EAGAIN => errno_result(EAGAIN),
        NATIVE_ENOENT => errno_result(ENOENT),
        NATIVE_EACCES => errno_result(EACCES),
        NATIVE_EBADF => errno_result(EBADF),
        NATIVE_EINVAL => errno_result(EINVAL),
        NATIVE_EISDIR => errno_result(EISDIR),
        NATIVE_ENOTDIR => errno_result(ENOTDIR),
        NATIVE_EIO => errno_result(EIO),
        NATIVE_ERANGE => errno_result(ERANGE),
        other => other,
    }
}

// Linux x86_64 syscall numbers (asm/unistd_64.h).
pub const SYS_READ: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_STAT: u64 = 4;
pub const SYS_FSTAT: u64 = 5;
pub const SYS_LSTAT: u64 = 6;
pub const SYS_POLL: u64 = 7;
pub const SYS_LSEEK: u64 = 8;
pub const SYS_MMAP: u64 = 9;
pub const SYS_MPROTECT: u64 = 10;
pub const SYS_MUNMAP: u64 = 11;
pub const SYS_BRK: u64 = 12;
pub const SYS_RT_SIGACTION: u64 = 13;
pub const SYS_RT_SIGPROCMASK: u64 = 14;
pub const SYS_IOCTL: u64 = 16;
pub const SYS_PREAD64: u64 = 17;
pub const SYS_PWRITE64: u64 = 18;
pub const SYS_WRITEV: u64 = 20;
pub const SYS_ACCESS: u64 = 21;
pub const SYS_PIPE: u64 = 22;
pub const SYS_SCHED_YIELD: u64 = 24;
pub const SYS_DUP: u64 = 32;
pub const SYS_DUP2: u64 = 33;
pub const SYS_NANOSLEEP: u64 = 35;
pub const SYS_GETPID: u64 = 39;
pub const SYS_SOCKET: u64 = 41;
pub const SYS_SOCKETPAIR: u64 = 53;
pub const SYS_CLONE: u64 = 56;
pub const SYS_FORK: u64 = 57;
pub const SYS_VFORK: u64 = 58;
pub const SYS_EXECVE: u64 = 59;
pub const SYS_EXIT: u64 = 60;
pub const SYS_WAIT4: u64 = 61;
pub const SYS_KILL: u64 = 62;
pub const SYS_UNAME: u64 = 63;
pub const SYS_FCNTL: u64 = 72;
pub const SYS_GETCWD: u64 = 79;
pub const SYS_CHDIR: u64 = 80;
pub const SYS_RENAME: u64 = 82;
pub const SYS_MKDIR: u64 = 83;
pub const SYS_UNLINK: u64 = 87;
pub const SYS_READLINK: u64 = 89;
pub const SYS_GETTIMEOFDAY: u64 = 96;
pub const SYS_GETUID: u64 = 102;
pub const SYS_GETGID: u64 = 104;
pub const SYS_GETEUID: u64 = 107;
pub const SYS_GETEGID: u64 = 108;
pub const SYS_GETPPID: u64 = 110;
pub const SYS_SIGALTSTACK: u64 = 131;
pub const SYS_ARCH_PRCTL: u64 = 158;
pub const SYS_GETTID: u64 = 186;
pub const SYS_GETDENTS64: u64 = 217;
pub const SYS_TKILL: u64 = 200;
pub const SYS_FUTEX: u64 = 202;
pub const SYS_SET_TID_ADDRESS: u64 = 218;
pub const SYS_CLOCK_GETTIME: u64 = 228;
pub const SYS_EXIT_GROUP: u64 = 231;
pub const SYS_EPOLL_WAIT: u64 = 232;
pub const SYS_EPOLL_CTL: u64 = 233;
pub const SYS_EPOLL_CREATE: u64 = 213;
pub const SYS_OPENAT: u64 = 257;
pub const SYS_MKDIRAT: u64 = 258;
pub const SYS_NEWFSTATAT: u64 = 262;
pub const SYS_UNLINKAT: u64 = 263;
pub const SYS_RENAMEAT: u64 = 264;
pub const SYS_READLINKAT: u64 = 267;
pub const SYS_FACCESSAT: u64 = 269;
pub const SYS_EPOLL_PWAIT: u64 = 281;
pub const SYS_EPOLL_CREATE1: u64 = 291;
pub const SYS_DUP3: u64 = 292;
pub const SYS_PIPE2: u64 = 293;
pub const SYS_GETRANDOM: u64 = 318;
pub const SYS_RSEQ: u64 = 334;
pub const SYS_SET_ROBUST_LIST: u64 = 273;

// Native SunlightOS syscall numbers used by the translator.
pub const SUN_PROCESS_EXIT: i64 = 20;
pub const SUN_PROCESS_YIELD: i64 = 21;
pub const SUN_FORK: i64 = 30;
pub const SUN_EXEC: i64 = 31;
pub const SUN_WAITPID: i64 = 32;
pub const SUN_GETPID: i64 = 33;
pub const SUN_GETPPID: i64 = 34;
pub const SUN_GETUID: i64 = 35;
pub const SUN_GETGID: i64 = 36;
pub const SUN_OPEN: i64 = 40;
pub const SUN_CLOSE: i64 = 41;
pub const SUN_READ: i64 = 42;
pub const SUN_WRITE: i64 = 43;
pub const SUN_LSEEK: i64 = 44;
pub const SUN_DUP: i64 = 45;
pub const SUN_DUP2: i64 = 46;
pub const SUN_PIPE: i64 = 47;
pub const SUN_FSTAT: i64 = 48;
pub const SUN_FCNTL: i64 = 49;
pub const SUN_MUNMAP: i64 = 51;
pub const SUN_MPROTECT: i64 = 52;
pub const SUN_CHDIR: i64 = 63;
pub const SUN_GETCWD: i64 = 64;
pub const SUN_UNLINK: i64 = 65;
pub const SUN_RENAME: i64 = 66;
pub const SUN_KILL: i64 = 72;

// Negative translator codes: kernel maps these to internal Helios handlers.
pub const SHIM_BRK: i64 = -2;
pub const SHIM_ARCH_PRCTL: i64 = -3;
pub const SHIM_SET_TID_ADDRESS: i64 = -4;
pub const SHIM_SET_ROBUST_LIST: i64 = -5;
pub const SHIM_RSEQ: i64 = -6;
pub const SHIM_POLL: i64 = -7;
pub const SHIM_RT_SIGACTION: i64 = -8;
pub const SHIM_RT_SIGPROCMASK: i64 = -9;
pub const SHIM_TKILL: i64 = -10;
pub const SHIM_MMAP: i64 = -11;
pub const SHIM_SIGALTSTACK: i64 = -12;
pub const SHIM_IOCTL: i64 = -13;
pub const SHIM_WRITEV: i64 = -14;
pub const SHIM_OPENAT: i64 = -15;
pub const SHIM_GETRANDOM: i64 = -16;
pub const SHIM_VFORK: i64 = -17;
pub const SHIM_CLONE: i64 = -18;
pub const SHIM_CLOCK_GETTIME: i64 = -19;
pub const SHIM_RENAMEAT: i64 = -20;
pub const SHIM_NANOSLEEP: i64 = -21;
pub const SHIM_EPOLL_CREATE: i64 = -22;
pub const SHIM_EPOLL_CTL: i64 = -23;
pub const SHIM_EPOLL_WAIT: i64 = -24;
pub const SHIM_PIPE2: i64 = -25;
pub const SHIM_SOCKETPAIR: i64 = -26;
pub const SHIM_UNLINKAT: i64 = -27;
pub const SHIM_NEWFSTATAT: i64 = -28;
pub const SHIM_WAIT4: i64 = -29;
pub const SHIM_UNAME: i64 = -30;
pub const SHIM_GETTIMEOFDAY: i64 = -31;
pub const SHIM_DUP3: i64 = -32;
pub const SHIM_PREAD64: i64 = -33;
pub const SHIM_PWRITE64: i64 = -34;
pub const SHIM_GETDENTS64: i64 = -35;
pub const SHIM_FACCESSAT: i64 = -36;
pub const SHIM_MKDIRAT: i64 = -37;
pub const SHIM_ENOSYS: i64 = -38;
pub const SHIM_READLINKAT: i64 = -39;

pub const MAP_PRIVATE: u64 = 0x02;
pub const MAP_FIXED: u64 = 0x10;
pub const MAP_ANONYMOUS: u64 = 0x20;
pub const MAP_FIXED_NOREPLACE: u64 = 0x10_0000;
pub const MAP_STACK: u64 = 0x20_000;

pub const PROT_READ: u32 = 0x1;
pub const PROT_WRITE: u32 = 0x2;
pub const PROT_EXEC: u32 = 0x4;

pub const O_RDONLY: u64 = 0;
pub const O_WRONLY: u64 = 1;
pub const O_RDWR: u64 = 2;
pub const O_ACCMODE: u64 = 3;
pub const O_CREAT: u64 = 0x40;
pub const O_EXCL: u64 = 0x80;
pub const O_NOCTTY: u64 = 0x100;
pub const O_TRUNC: u64 = 0x200;
pub const O_APPEND: u64 = 0x400;
pub const O_NONBLOCK: u64 = 0x800;
pub const O_DIRECTORY: u64 = 0x1_0000;
pub const O_NOFOLLOW: u64 = 0x2_0000;
pub const O_CLOEXEC: u64 = 0x0008_0000;
pub const GRND_NONBLOCK: u64 = 0x1;

/// Linux open(2) flags Helios accepts. Unknown bits are rejected rather than
/// silently ignored. O_NOCTTY and O_NOFOLLOW are accepted as no-ops: Sunlight
/// does not assign a controlling TTY on open, and the VFS has no symlinks.
pub const OPEN_SUPPORTED_FLAGS: u64 = O_ACCMODE
    | O_CREAT
    | O_EXCL
    | O_NOCTTY
    | O_TRUNC
    | O_APPEND
    | O_NONBLOCK
    | O_DIRECTORY
    | O_NOFOLLOW
    | O_CLOEXEC;

pub const F_OK: u64 = 0;
pub const X_OK: u64 = 1;
pub const W_OK: u64 = 2;
pub const R_OK: u64 = 4;
pub const ACCESS_OK_MASK: u64 = R_OK | W_OK | X_OK;

pub const CLOCK_REALTIME: i32 = 0;
pub const CLOCK_MONOTONIC: i32 = 1;

pub const DT_UNKNOWN: u8 = 0;
pub const DT_DIR: u8 = 4;
pub const DT_REG: u8 = 8;

pub const F_DUPFD: u64 = 0;
pub const F_GETFD: u64 = 1;
pub const F_SETFD: u64 = 2;
pub const F_GETFL: u64 = 3;
pub const F_SETFL: u64 = 4;
pub const F_DUPFD_CLOEXEC: u64 = 1030;
pub const FD_CLOEXEC: u64 = 1;

pub const AT_FDCWD: i32 = -100;
pub const AT_SYMLINK_NOFOLLOW: u64 = 0x100;
pub const AT_REMOVEDIR: u64 = 0x200;
pub const AT_EACCESS: u64 = 0x200;
pub const AT_EMPTY_PATH: u64 = 0x1000;

/// faccessat flags with a native-grounded meaning. AT_EACCESS is accepted as a
/// no-op because Sunlight stores a single uid/gid with no saved/effective split.
pub const FACCESSAT_SUPPORTED_FLAGS: u64 = AT_SYMLINK_NOFOLLOW | AT_EACCESS;

pub const AT_NULL: u64 = 0;
pub const AT_PHDR: u64 = 3;
pub const AT_PHENT: u64 = 4;
pub const AT_PHNUM: u64 = 5;
pub const AT_PAGESZ: u64 = 6;
pub const AT_ENTRY: u64 = 9;
pub const AT_UID: u64 = 11;
pub const AT_EUID: u64 = 12;
pub const AT_GID: u64 = 13;
pub const AT_EGID: u64 = 14;
pub const AT_SECURE: u64 = 23;
pub const AT_RANDOM: u64 = 25;
pub const AT_EXECFN: u64 = 31;

pub const STAT_SIZE: usize = 144;
pub const TIMESPEC_SIZE: usize = 16;
pub const TIMEVAL_SIZE: usize = 16;
pub const TIMEZONE_SIZE: usize = 8;
pub const UTSNAME_FIELD: usize = 65;
pub const UTSNAME_SIZE: usize = UTSNAME_FIELD * 6;
pub const DIRENT64_HEADER: usize = 19;
pub const POLLFD_SIZE: usize = 8;
pub const IOVEC_SIZE: usize = 16;
pub const TERMIOS_SIZE: usize = 60;
pub const WINSIZE_SIZE: usize = 8;

/// Linux x86_64 `struct winsize`. This is an ABI-only value; native terminal
/// geometry is translated field-by-field at the syscall boundary.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinuxWinsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

impl LinuxWinsize {
    pub const fn new(ws_row: u16, ws_col: u16, ws_xpixel: u16, ws_ypixel: u16) -> Self {
        Self {
            ws_row,
            ws_col,
            ws_xpixel,
            ws_ypixel,
        }
    }

    pub fn to_ne_bytes(self) -> [u8; WINSIZE_SIZE] {
        let mut wire = [0u8; WINSIZE_SIZE];
        wire[0..2].copy_from_slice(&self.ws_row.to_ne_bytes());
        wire[2..4].copy_from_slice(&self.ws_col.to_ne_bytes());
        wire[4..6].copy_from_slice(&self.ws_xpixel.to_ne_bytes());
        wire[6..8].copy_from_slice(&self.ws_ypixel.to_ne_bytes());
        wire
    }
}
pub const EPOLL_EVENT_SIZE: usize = 12;
pub const SIGACTION_SIZE: usize = 32;
pub const STACK_T_SIZE: usize = 24;

const _: () = assert!(STAT_SIZE == 144);
const _: () = assert!(TIMESPEC_SIZE == 16);
const _: () = assert!(TIMEVAL_SIZE == 16);
const _: () = assert!(UTSNAME_SIZE == 390);
const _: () = assert!(POLLFD_SIZE == 8);
const _: () = assert!(IOVEC_SIZE == 16);
const _: () = assert!(TERMIOS_SIZE == 60);
const _: () = assert!(WINSIZE_SIZE == 8);
const _: () = assert!(core::mem::size_of::<LinuxWinsize>() == WINSIZE_SIZE);

#[cfg(test)]
mod winsize_tests {
    use super::*;

    #[test]
    fn linux_geometry_probe_has_exact_x86_64_layout() {
        let wire = LinuxWinsize::new(61, 173, 1384, 976).to_ne_bytes();
        assert_eq!(u16::from_ne_bytes([wire[0], wire[1]]), 61);
        assert_eq!(u16::from_ne_bytes([wire[2], wire[3]]), 173);
        assert_eq!(u16::from_ne_bytes([wire[4], wire[5]]), 1384);
        assert_eq!(u16::from_ne_bytes([wire[6], wire[7]]), 976);
    }
}
const _: () = assert!(EPOLL_EVENT_SIZE == 12);

/// Packed linux_dirent64 record size: 8+8+2+1+name+NUL, rounded up to 8.
pub const fn dirent64_reclen(name_len: usize) -> usize {
    let unaligned = DIRENT64_HEADER + name_len + 1;
    (unaligned + 7) & !7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errno_encoding_is_negative_linux_errno() {
        assert_eq!(errno_result(ENOSYS) as i64, -38);
        assert_eq!(errno_result(ENOENT) as i64, -2);
        assert_eq!(errno_result(EAGAIN) as i64, -11);
        assert_eq!(errno_result(EACCES) as i64, -13);
        assert_eq!(errno_result(EBADF) as i64, -9);
        assert_eq!(errno_result(EINVAL) as i64, -22);
        assert_eq!(errno_result(EFAULT) as i64, -14);
    }

    #[test]
    fn native_sentinels_do_not_masquerade_as_linux_errno() {
        // Before translation, native ENOENT is -3 (ESRCH to musl).
        assert_eq!(NATIVE_ENOENT as i64, -3);
        assert_eq!(from_native_result(NATIVE_ENOENT) as i64, -2);
        assert_eq!(from_native_result(NATIVE_EAGAIN) as i64, -11);
        assert_eq!(from_native_result(NATIVE_EACCES) as i64, -13);
        assert_eq!(from_native_result(NATIVE_EBADF) as i64, -9);
        assert_eq!(from_native_result(NATIVE_EINVAL) as i64, -22);
        assert_eq!(from_native_result(NATIVE_EISDIR) as i64, -21);
        assert_eq!(from_native_result(NATIVE_ENOTDIR) as i64, -20);
        assert_eq!(from_native_result(NATIVE_EIO) as i64, -5);
        assert_eq!(from_native_result(NATIVE_ERANGE) as i64, -34);
        assert_eq!(from_native_result(NATIVE_GENERIC) as i64, -1);
        assert_eq!(from_native_result(7), 7);
        // Already-Linux EAGAIN must not be rewritten.
        assert_eq!(
            from_native_result(errno_result(EAGAIN)),
            errno_result(EAGAIN)
        );
        assert_eq!(
            from_native_result(errno_result(ENOSYS)),
            errno_result(ENOSYS)
        );
    }

    #[test]
    fn linux_syscall_numbers_match_x86_64() {
        assert_eq!(SYS_READ, 0);
        assert_eq!(SYS_WRITE, 1);
        assert_eq!(SYS_OPEN, 2);
        assert_eq!(SYS_CLOSE, 3);
        assert_eq!(SYS_STAT, 4);
        assert_eq!(SYS_FSTAT, 5);
        assert_eq!(SYS_LSTAT, 6);
        assert_eq!(SYS_POLL, 7);
        assert_eq!(SYS_DUP, 32);
        assert_eq!(SYS_DUP2, 33);
        assert_eq!(SYS_FORK, 57);
        assert_eq!(SYS_VFORK, 58);
        assert_eq!(SYS_CLONE, 56);
        assert_eq!(SYS_WAIT4, 61);
        assert_eq!(SYS_GETPPID, 110);
        assert_eq!(SYS_GETTID, 186);
        assert_eq!(SYS_FUTEX, 202);
        assert_eq!(SYS_OPENAT, 257);
        assert_eq!(SYS_GETRANDOM, 318);
        assert_eq!(SYS_UNAME, 63);
        assert_eq!(SYS_MKDIR, 83);
        assert_eq!(SYS_DUP3, 292);
        assert_eq!(SYS_PREAD64, 17);
        assert_eq!(SYS_PWRITE64, 18);
        assert_eq!(SYS_ACCESS, 21);
        assert_eq!(SYS_GETTIMEOFDAY, 96);
        assert_eq!(SYS_GETUID, 102);
        assert_eq!(SYS_GETGID, 104);
        assert_eq!(SYS_GETEUID, 107);
        assert_eq!(SYS_GETEGID, 108);
        assert_eq!(SYS_GETDENTS64, 217);
        assert_eq!(SYS_MKDIRAT, 258);
        assert_eq!(SYS_READLINKAT, 267);
        assert_eq!(SYS_FACCESSAT, 269);
        assert_eq!(SYS_READLINK, 89);
        assert_eq!(UTSNAME_SIZE, 390);
        assert_eq!(dirent64_reclen(1), 24);
        assert_eq!(dirent64_reclen(4), 24);
        assert_eq!(dirent64_reclen(5), 32);
    }
}
