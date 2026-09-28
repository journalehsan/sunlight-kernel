//! Linux x86-64 raw clone flags and the supported pthread-style thread subset.
//!
//! Constants are from Linux `include/uapi/linux/sched.h`. The low byte is the
//! exit signal, rather than a clone resource-sharing flag.

pub const CSIGNAL: u64 = 0x0000_00ff;
pub const CLONE_VM: u64 = 0x0000_0100;
pub const CLONE_FS: u64 = 0x0000_0200;
pub const CLONE_FILES: u64 = 0x0000_0400;
pub const CLONE_SIGHAND: u64 = 0x0000_0800;
pub const CLONE_THREAD: u64 = 0x0001_0000;
pub const CLONE_SYSVSEM: u64 = 0x0004_0000;
pub const CLONE_SETTLS: u64 = 0x0008_0000;
pub const CLONE_PARENT_SETTID: u64 = 0x0010_0000;
pub const CLONE_CHILD_CLEARTID: u64 = 0x0020_0000;
pub const CLONE_DETACHED: u64 = 0x0040_0000;

/// Flags which a complete shared-process Helios thread implementation must
/// implement individually. CLONE_DETACHED is ignored for legacy clone(2).
pub const THREAD_FLAGS: u64 = CLONE_VM
    | CLONE_FS
    | CLONE_FILES
    | CLONE_SIGHAND
    | CLONE_THREAD
    | CLONE_SYSVSEM
    | CLONE_SETTLS
    | CLONE_PARENT_SETTID
    | CLONE_CHILD_CLEARTID
    | CLONE_DETACHED;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneFlagError {
    InvalidCombination,
    Unsupported,
}

/// Validate the flags independently; do not whitelist one pthread bitmask.
/// This only recognizes thread groups whose VM, files, and FS contexts share
/// ownership. A clone lacking these flags needs process-copy semantics.
pub fn validate_thread_flags(flags: u64) -> Result<(), CloneFlagError> {
    if flags & CLONE_SIGHAND != 0 && flags & CLONE_VM == 0
        || flags & CLONE_THREAD != 0 && flags & CLONE_SIGHAND == 0
        || flags & CLONE_THREAD != 0 && flags & CSIGNAL != 0
    {
        return Err(CloneFlagError::InvalidCombination);
    }
    if flags & !THREAD_FLAGS != 0
        || flags & (CLONE_THREAD | CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND)
            != CLONE_THREAD | CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND
    {
        return Err(CloneFlagError::Unsupported);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_flags_decode_to_linux_thread_resources() {
        assert_eq!(THREAD_FLAGS, 0x7d0f00);
        assert_eq!(THREAD_FLAGS & CSIGNAL, 0);
    }

    #[test]
    fn validation_accepts_independent_optional_thread_features() {
        let base = CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD;
        assert_eq!(validate_thread_flags(base), Ok(()));
        for bit in [
            CLONE_SYSVSEM,
            CLONE_SETTLS,
            CLONE_PARENT_SETTID,
            CLONE_CHILD_CLEARTID,
            CLONE_DETACHED,
        ] {
            assert_eq!(validate_thread_flags(base | bit), Ok(()));
        }
    }

    #[test]
    fn validation_rejects_broken_dependencies_and_exit_signals() {
        assert_eq!(
            validate_thread_flags(CLONE_SIGHAND),
            Err(CloneFlagError::InvalidCombination)
        );
        assert_eq!(
            validate_thread_flags(CLONE_VM | CLONE_THREAD),
            Err(CloneFlagError::InvalidCombination)
        );
        assert_eq!(
            validate_thread_flags(THREAD_FLAGS | 17),
            Err(CloneFlagError::InvalidCombination)
        );
    }

    #[test]
    fn validation_rejects_unsupported_process_and_namespace_flags() {
        assert_eq!(
            validate_thread_flags(THREAD_FLAGS | 0x2000_0000),
            Err(CloneFlagError::Unsupported)
        );
        assert_eq!(
            validate_thread_flags(CLONE_VM | CLONE_FS),
            Err(CloneFlagError::Unsupported)
        );
        assert_eq!(
            validate_thread_flags(THREAD_FLAGS & !CLONE_FILES),
            Err(CloneFlagError::Unsupported)
        );
    }
}
