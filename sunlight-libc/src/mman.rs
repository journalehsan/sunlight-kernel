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

pub fn mmap(
    addr: *mut u8,
    length: usize,
    prot: u32,
    flags: u32,
    fd: i32,
    offset: u64,
) -> Result<*mut u8, Errno> {
    if length == 0 {
        return Err(Errno::Inval);
    }
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
    check(ret).map(|ptr| ptr as *mut u8)
}

pub fn munmap(addr: *mut u8, length: usize) -> Result<(), Errno> {
    if length == 0 || (addr as u64) % 4096 != 0 {
        return Err(Errno::Inval);
    }
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
    match mmap(addr, length, prot as u32, flags as u32, fd, offset as u64) {
        Ok(ptr) => ptr,
        Err(e) => {
            crate::errno::set_from_errno(e);
            MAP_FAILED
        }
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
