//! Thread-Local Storage bootstrap for SunlightOS.
//!
//! Call `init_tls()` at the very start of `_start`, before any C-ABI libc
//! calls that may write errno. Each thread receives one mapping that holds the
//! process TLS image followed by the Thread Control Block. FS_BASE points at
//! the TCB so that `fs:0` == `self_ptr`, `fs:8` == `errno` and `fs:16` holds
//! the optional pthread control block.
//!
//! Native x86-64 uses TLS variant II: static TLS variables live at negative
//! offsets from the thread pointer. The linker script exports the image bounds
//! and the segment alignment; the block size is the image size rounded up to
//! that alignment, matching the linker's `@TPOFF` values.

use crate::mman::{mmap, MAP_ANONYMOUS, MAP_PRIVATE, PROT_READ, PROT_WRITE};
use crate::sys::{syscall1, SYS_SET_FS_BASE};

/// Thread Control Block layout required by the SysV AMD64 ABI.
/// `fs:0` must be a self-pointer; `fs:8` is our per-thread errno.
#[repr(C)]
pub struct Tcb {
    pub self_ptr: *mut Tcb,
    pub errno: i32,
    pub _reserved: u32,
    /// pthread control block for threads created by `pthread_create`.
    pub pthread: *mut u8,
}

pub const TCB_SIZE: usize = core::mem::size_of::<Tcb>();
/// Minimum alignment for the thread pointer. The linker's TLS segment
/// alignment raises it when a C `_Thread_local` variable is more aligned.
const TCB_ALIGN: usize = 64;

#[cfg(target_os = "sunlightos")]
extern "C" {
    static __tls_tdata_start: u8;
    static __tls_tdata_end: u8;
    static __tls_tbss_end: u8;
    static __tls_align: u8;
}

/// Static TLS image exported by the native linker scripts.
#[derive(Clone, Copy)]
struct TlsImage {
    data: *const u8,
    data_len: usize,
    mem_len: usize,
    align: usize,
}

#[cfg(target_os = "sunlightos")]
fn tls_image() -> TlsImage {
    // Absolute linker symbols: taking their address yields the assigned value.
    unsafe {
        TlsImage {
            data: &__tls_tdata_start,
            data_len: &__tls_tdata_end as *const u8 as usize
                - &__tls_tdata_start as *const u8 as usize,
            mem_len: &__tls_tbss_end as *const u8 as usize
                - &__tls_tdata_start as *const u8 as usize,
            align: &__tls_align as *const u8 as usize,
        }
    }
}

#[cfg(not(target_os = "sunlightos"))]
fn tls_image() -> TlsImage {
    TlsImage {
        data: core::ptr::null(),
        data_len: 0,
        mem_len: 0,
        align: 1,
    }
}

fn align_up(value: usize, align: usize) -> usize {
    let align = align.max(1);
    (value + align - 1) & !(align - 1)
}

/// Size of the TLS block as seen by `@TPOFF` references.
fn tls_block_size() -> usize {
    let image = tls_image();
    align_up(image.mem_len, image.align)
}

fn thread_pointer_align() -> usize {
    TCB_ALIGN.max(tls_image().align)
}

/// Mapping size required for a thread's TLS image and TCB.
pub fn area_size() -> usize {
    align_up(tls_block_size() + TCB_SIZE + thread_pointer_align(), 4096)
}

/// Lay out the TLS image and TCB inside `area`, returning the thread pointer.
///
/// # Safety
/// `area` must designate `size` writable bytes that stay mapped for the
/// lifetime of the thread.
pub unsafe fn prepare_area(area: *mut u8, size: usize) -> Option<*mut Tcb> {
    let block_size = tls_block_size();
    if area.is_null() || size < block_size + TCB_SIZE {
        return None;
    }
    let tcb_addr = (area as usize + size - TCB_SIZE) & !(thread_pointer_align() - 1);
    if tcb_addr < area as usize + block_size {
        return None;
    }
    let image = tls_image();
    let dest = tcb_addr - block_size;
    if image.data_len != 0 {
        core::ptr::copy_nonoverlapping(image.data, dest as *mut u8, image.data_len);
    }
    core::ptr::write_bytes(
        (dest + image.data_len) as *mut u8,
        0,
        block_size - image.data_len,
    );
    let tcb = tcb_addr as *mut Tcb;
    tcb.write(Tcb {
        self_ptr: tcb,
        errno: 0,
        _reserved: 0,
        pthread: core::ptr::null_mut(),
    });
    Some(tcb)
}

/// Initialize TLS for the current thread. Must be called before any
/// C-ABI libc function that reads or writes errno.
///
/// # Safety
/// Must be called exactly once per thread, before any errno-touching code.
pub unsafe fn init_tls() -> Result<(), ()> {
    let size = area_size();
    let ptr = mmap(
        core::ptr::null_mut(),
        size,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS,
        -1,
        0,
    )
    .map_err(|_| ())?;

    let tcb = prepare_area(ptr, size).ok_or(())?;

    let ret = syscall1(SYS_SET_FS_BASE, tcb as u64);
    if ret != 0 {
        return Err(());
    }

    crate::errno::mark_tls_ready();
    Ok(())
}

/// Return the calling thread's TCB, or null if TLS is not initialized.
///
/// # Safety
/// Only valid after `init_tls`/`prepare_area` has installed FS_BASE.
#[cfg(target_arch = "x86_64")]
pub unsafe fn current_tcb() -> *mut Tcb {
    let tcb: *mut Tcb;
    core::arch::asm!(
        "mov {}, qword ptr fs:[0]",
        out(reg) tcb,
        options(nostack, preserves_flags, readonly)
    );
    tcb
}

#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn current_tcb() -> *mut Tcb {
    core::ptr::null_mut()
}

/// pthread control block associated with the calling thread, if any.
///
/// # Safety
/// Same precondition as `current_tcb`.
pub unsafe fn pthread_control() -> *mut u8 {
    let tcb = current_tcb();
    if tcb.is_null() {
        core::ptr::null_mut()
    } else {
        (*tcb).pthread
    }
}

/// Attach a pthread control block to the calling thread.
///
/// # Safety
/// Same precondition as `current_tcb`.
pub unsafe fn set_pthread_control(control: *mut u8) {
    let tcb = current_tcb();
    if !tcb.is_null() {
        (*tcb).pthread = control;
    }
}
