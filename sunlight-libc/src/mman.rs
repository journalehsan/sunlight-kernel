//! POSIX memory management (`mmap`/`munmap`).

use crate::sys::{check, syscall2, syscall6, Errno, SYS_MMAP, SYS_MUNMAP};

pub const PROT_NONE: u32 = 0x0;
pub const PROT_READ: u32 = 0x1;
pub const PROT_WRITE: u32 = 0x2;
pub const PROT_EXEC: u32 = 0x4;

pub const MAP_SHARED: u32 = 0x01;
pub const MAP_PRIVATE: u32 = 0x02;
pub const MAP_FIXED: u32 = 0x10;
pub const MAP_ANONYMOUS: u32 = 0x20;

pub const MAP_FAILED: *mut u8 = !0 as *mut u8;

const PAGE_SIZE: usize = 4096;
const SUPPORTED_PROT: u32 = PROT_READ | PROT_WRITE | PROT_EXEC;
const SUPPORTED_FLAGS: u32 = MAP_PRIVATE | MAP_FIXED | MAP_ANONYMOUS;

/// Validate the deliberately narrow native mapping contract before entering
/// the kernel.  Native SunlightOS currently implements only private,
/// anonymous mappings; accepting a descriptor, offset, shared flag, or an
/// ordinary address hint here would falsely imply backed/shared or hint
/// semantics that the kernel does not provide.
fn validate_mmap_request(
    addr: *mut u8,
    length: usize,
    prot: u32,
    flags: u32,
    fd: i32,
    offset: u64,
) -> Result<usize, Errno> {
    if length == 0
        || prot & !SUPPORTED_PROT != 0
        || prot == PROT_NONE
        // x86_64 cannot make write-only or execute-only user mappings.  Do
        // not silently widen either request to readable memory.
        || prot & PROT_READ == 0
        || prot & PROT_WRITE != 0 && prot & PROT_EXEC != 0
        || flags & !SUPPORTED_FLAGS != 0
        || flags & (MAP_PRIVATE | MAP_ANONYMOUS) != (MAP_PRIVATE | MAP_ANONYMOUS)
        || fd != -1
        || offset != 0
    {
        return Err(Errno::Inval);
    }

    if flags & MAP_FIXED != 0 {
        if addr.is_null() || (addr as usize) & (PAGE_SIZE - 1) != 0 {
            return Err(Errno::Inval);
        }
    } else if !addr.is_null() {
        // Hints have never been placed by the native mapper.  Reject rather
        // than ignore a supplied address and return an unrelated range.
        return Err(Errno::Inval);
    }

    length
        .checked_add(PAGE_SIZE - 1)
        .map(|rounded| rounded & !(PAGE_SIZE - 1))
        .ok_or(Errno::Inval)
}

pub fn mmap(
    addr: *mut u8,
    length: usize,
    prot: u32,
    flags: u32,
    fd: i32,
    offset: u64,
) -> Result<*mut u8, Errno> {
    let rounded_length = validate_mmap_request(addr, length, prot, flags, fd, offset)?;
    let ret = unsafe {
        syscall6(
            SYS_MMAP,
            addr as u64,
            length as u64,
            prot as u64,
            flags as u64,
            fd as u64,
            offset,
        )
    };
    let mapped = check(ret)? as *mut u8;
    let mapped_addr = mapped as usize;
    if mapped.is_null()
        || mapped_addr & (PAGE_SIZE - 1) != 0
        || mapped_addr.checked_add(rounded_length).is_none()
    {
        return Err(Errno::Failed);
    }
    Ok(mapped)
}

pub fn munmap(addr: *mut u8, length: usize) -> Result<(), Errno> {
    if addr.is_null() || (addr as usize) & (PAGE_SIZE - 1) != 0 {
        return Err(Errno::Inval);
    }
    let _ = length
        .checked_add(PAGE_SIZE - 1)
        .map(|rounded| rounded & !(PAGE_SIZE - 1))
        .filter(|rounded| *rounded != 0)
        .ok_or(Errno::Inval)?;
    let ret = unsafe { syscall2(SYS_MUNMAP, addr as u64, length as u64) };
    check(ret).map(|_| ())
}

// ── C ABI ───────────────────────────────────────────────────────────────────

#[export_name = "mmap"]
pub unsafe extern "C" fn c_mmap(
    addr: *mut u8,
    length: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    offset: i64,
) -> *mut u8 {
    let result = if prot < 0 || flags < 0 || offset < 0 {
        Err(Errno::Inval)
    } else {
        mmap(addr, length, prot as u32, flags as u32, fd, offset as u64)
    };
    match result {
        Ok(ptr) => ptr,
        Err(e) => {
            crate::errno::set_from_errno(e);
            MAP_FAILED
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_request_validation_is_checked_and_fail_closed() {
        let flags = MAP_PRIVATE | MAP_ANONYMOUS;
        assert_eq!(
            validate_mmap_request(core::ptr::null_mut(), 1, PROT_READ, flags, -1, 0),
            Ok(PAGE_SIZE)
        );
        assert_eq!(
            validate_mmap_request(
                core::ptr::null_mut(),
                PAGE_SIZE + 1,
                PROT_READ | PROT_WRITE,
                flags,
                -1,
                0,
            ),
            Ok(PAGE_SIZE * 2)
        );
        assert_eq!(
            validate_mmap_request(core::ptr::null_mut(), usize::MAX, PROT_READ, flags, -1, 0),
            Err(Errno::Inval)
        );
        assert_eq!(
            validate_mmap_request(core::ptr::null_mut(), 0, PROT_READ, flags, -1, 0),
            Err(Errno::Inval)
        );
    }

    #[test]
    fn unsupported_mapping_and_protection_semantics_are_rejected() {
        let flags = MAP_PRIVATE | MAP_ANONYMOUS;
        assert_eq!(
            validate_mmap_request(core::ptr::null_mut(), PAGE_SIZE, PROT_WRITE, flags, -1, 0),
            Err(Errno::Inval)
        );
        assert_eq!(
            validate_mmap_request(core::ptr::null_mut(), PAGE_SIZE, PROT_EXEC, flags, -1, 0),
            Err(Errno::Inval)
        );
        assert_eq!(
            validate_mmap_request(
                core::ptr::null_mut(),
                PAGE_SIZE,
                PROT_READ,
                MAP_SHARED,
                -1,
                0,
            ),
            Err(Errno::Inval)
        );
        assert_eq!(
            validate_mmap_request(core::ptr::null_mut(), PAGE_SIZE, PROT_READ, flags, 3, 0),
            Err(Errno::Inval)
        );
        assert_eq!(
            validate_mmap_request(
                core::ptr::null_mut(),
                PAGE_SIZE,
                PROT_READ,
                flags,
                -1,
                PAGE_SIZE as u64,
            ),
            Err(Errno::Inval)
        );
    }

    #[test]
    fn fixed_mappings_require_an_aligned_non_null_address_and_hints_are_not_ignored() {
        let fixed = MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED;
        assert_eq!(
            validate_mmap_request(0x1000usize as *mut u8, PAGE_SIZE, PROT_READ, fixed, -1, 0),
            Ok(PAGE_SIZE)
        );
        assert_eq!(
            validate_mmap_request(0x1001usize as *mut u8, PAGE_SIZE, PROT_READ, fixed, -1, 0),
            Err(Errno::Inval)
        );
        assert_eq!(
            validate_mmap_request(
                0x2000usize as *mut u8,
                PAGE_SIZE,
                PROT_READ,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            ),
            Err(Errno::Inval)
        );
    }
}

#[export_name = "munmap"]
pub unsafe extern "C" fn c_munmap(addr: *mut u8, length: usize) -> i32 {
    match munmap(addr, length) {
        Ok(()) => 0,
        Err(e) => {
            crate::errno::set_from_errno(e);
            -1
        }
    }
}
