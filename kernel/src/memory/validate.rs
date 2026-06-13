//! User-pointer validation for syscalls that accept raw pointer+length pairs
//! from user-space (e.g. shared-memory and read/write buffer arguments).

use crate::process::Process;
use x86_64::structures::paging::{Page, Size4KiB};
use x86_64::VirtAddr;

/// Start of the kernel's half of the address space. Any user-supplied pointer
/// at or above this address is either a forgery or a confused-deputy attempt
/// to make the kernel dereference kernel memory on the caller's behalf.
pub const KERNEL_START: u64 = 0xFFFF_8000_0000_0000;

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
    // Rule 1: must be in user-space
    if ptr >= KERNEL_START {
        crate::serial_println!(
            "[SEC] WARN: pid={} passed kernel ptr {:#x}",
            process.pid, ptr
        );
        return Err(PtrError::KernelAddress);
    }

    // Rule 2: must not overflow
    let end = ptr.checked_add(len).ok_or(PtrError::Overflow)?;

    // Rule 3: end must also be in user-space
    if end > KERNEL_START {
        return Err(PtrError::CrossesBoundary);
    }

    // Rule 4: range must be mapped (check page table), one page at a time
    if len > 0 {
        let first_page = ptr & !0xFFF;
        let last_page = (end - 1) & !0xFFF;
        let mut page_addr = first_page;
        loop {
            let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(page_addr))
                .map_err(|_| PtrError::NotMapped)?;
            if process.address_space.lookup_phys(page, hhdm_offset).is_none() {
                crate::serial_println!(
                    "[SEC] WARN: pid={} ptr {:#x} not mapped",
                    process.pid, ptr
                );
                return Err(PtrError::NotMapped);
            }
            if page_addr == last_page {
                break;
            }
            page_addr += 0x1000;
        }
    }

    Ok(())
}
