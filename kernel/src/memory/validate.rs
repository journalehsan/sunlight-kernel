//! User-pointer validation for syscalls that accept raw pointer+length pairs
//! from user-space (e.g. shared-memory and read/write buffer arguments).

use crate::process::Process;
use x86_64::VirtAddr;

/// Start of the kernel's half of the address space. Any user-supplied pointer
/// at or above this address is either a forgery or a confused-deputy attempt
/// to make the kernel dereference kernel memory on the caller's behalf.
pub use crate::memory::user::USER_END_EXCLUSIVE as KERNEL_START;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtrError {
    /// Pointer (or end of range) lies at/above KERNEL_START.
    KernelAddress,
    /// `ptr + len` overflows u64.
    Overflow,
    /// Range starts in user-space but its end crosses into kernel-space.
    CrossesBoundary,
    /// Range is not (fully) mapped in the caller's address space.
    NotMapped,
}

/// Validate that a user-supplied pointer range is:
/// 1. Below KERNEL_START (no kernel memory access)
/// 2. Does not overflow (ptr + len wraps around)
/// 3. Is actually mapped in the calling process's address space
///
/// SAFETY: `hhdm_offset` must be the correct HHDM base for the running kernel.
pub unsafe fn validate_user_ptr(
    ptr: u64,
    len: u64,
    process: &Process,
    hhdm_offset: VirtAddr,
) -> Result<(), PtrError> {
    let len = usize::try_from(len).map_err(|_| PtrError::Overflow)?;
    let range = crate::memory::user::UserRange::new(ptr, len).map_err(|error| match error {
        crate::memory::user::UserMemoryError::Overflow => PtrError::Overflow,
        crate::memory::user::UserMemoryError::KernelRange
        | crate::memory::user::UserMemoryError::NonCanonical
        | crate::memory::user::UserMemoryError::InvalidAddress => PtrError::KernelAddress,
        _ => PtrError::NotMapped,
    })?;
    let mut scratch = [0u8; 256];
    let mut copied = 0usize;
    while copied < range.len() {
        let chunk = scratch.len().min(range.len() - copied);
        crate::memory::user::copy_from_process_bytes(
            process,
            hhdm_offset,
            range.start() + copied as u64,
            &mut scratch[..chunk],
        )
        .map_err(|_| PtrError::NotMapped)?;
        copied += chunk;
    }
    Ok(())
}
