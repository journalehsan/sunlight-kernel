//! Linux syscall translation layer (Helios).
//!
//! Translates x86_64 Linux syscall numbers to SunlightOS native syscalls.
//! Supports static musl binaries with minimal dependencies.
//!
//! # Implementation note
//! This is a "thin" compat layer that maps Linux syscall numbers to native
//! SunlightOS equivalents. Full POSIX compliance is not a goal — only enough
//! to run simple statically-linked musl binaries.

#![no_std]

/// Translate Linux x86_64 syscall number to SunlightOS native syscall.
///
/// Returns the native syscall number if translation succeeds,
/// or an error code (negative) if unsupported.
///
/// # Arguments
/// - `linux_nr`: Linux x86_64 syscall number
///
/// # Returns
/// - Native SunlightOS syscall number (0-99)
/// - Negative value (error code) if unsupported
pub fn translate_syscall(linux_nr: u64) -> i64 {
    match linux_nr {
        // Tier 1: minimal I/O (needed for all musl programs)
        0 => 42,    // read → SunlightOS Read(42)
        1 => 43,    // write → SunlightOS Write(43)
        2 => 40,    // open → SunlightOS Open(40)
        3 => 41,    // close → SunlightOS Close(41)
        60 => -1,   // exit(code) → special handling (process termination)
        231 => -1,  // exit_group(code) → special handling

        // Tier 2: file descriptor operations
        5 => 48,    // fstat → SunlightOS Fstat(48)
        8 => 44,    // lseek → SunlightOS Lseek(44)
        32 => 32,   // dup2 → SunlightOS Dup2(46) [note: number differs]

        // Tier 3: process management
        39 => 33,   // getpid → SunlightOS Getpid(33)
        57 => 30,   // fork → SunlightOS Fork(30)
        59 => 31,   // execve → SunlightOS Exec(31)
        61 => 32,   // wait4 → SunlightOS Waitpid(32)

        // Tier 4: memory management
        9 => 50,    // mmap → SunlightOS Mmap(50)
        11 => 51,   // munmap → SunlightOS Munmap(51)
        12 => -2,   // brk → special handling (not implemented yet)
        10 => 52,   // mprotect → SunlightOS Mprotect(52)

        // Tier 5: signals
        13 => 72,   // kill → SunlightOS Kill(72)
        14 => 71,   // sigprocmask → SunlightOS Sigprocmask(71)

        // Process information
        4 => 48,    // stat → Fstat (approximation)
        6 => 48,    // lstat → Fstat (approximation)

        // Default: unsupported
        _ => -38,   // ENOSYS
    }
}

/// Helper to determine if a syscall requires special handling.
///
/// These syscalls need kernel-side post-processing after translation
/// (e.g., exit needs to terminate the process, not just return).
pub fn needs_special_handling(linux_nr: u64) -> bool {
    matches!(linux_nr, 60 | 231 | 12)  // exit, exit_group, brk
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_syscalls_translate() {
        assert_eq!(translate_syscall(1), 43);   // write
        assert_eq!(translate_syscall(0), 42);   // read
        assert_eq!(translate_syscall(60), -1);  // exit (special)
    }

    #[test]
    fn unsupported_returns_enosys() {
        assert_eq!(translate_syscall(999), -38);
    }
}
