//! Thread-Local Storage bootstrap for SunlightOS.
//!
//! Call `init_tls()` at the very start of `_start`, before any C-ABI libc
//! calls that may write errno. Each thread receives one mapping that holds the
//! process TLS image followed by the Thread Control Block. FS_BASE points at
//! the TCB so that `fs:0` == `self_ptr`, `fs:8` == `errno` and `fs:16` holds
//! the optional pthread control block. Native Rust std key slots start at
//! `fs:32` and are part of the same allocated and initialized TCB.
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
    _key_padding: usize,
    // ABI shared with tools/native-std/overlay/thread_local/key/sunlightos.rs.
    // Reserve the whole table after FS_BASE, including for static C TLS images.
    std_keys: [*mut u8; 256],
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
fn tls_block_size(image: TlsImage) -> usize {
    align_up(image.mem_len, image.align)
}

fn thread_pointer_align(image: TlsImage) -> usize {
    TCB_ALIGN.max(image.align)
}

/// Mapping size required for a thread's TLS image and TCB.
pub fn area_size() -> usize {
    area_size_for(tls_image())
}

fn area_size_for(image: TlsImage) -> usize {
    align_up(
        tls_block_size(image) + TCB_SIZE + thread_pointer_align(image),
        4096,
    )
}

/// Lay out the TLS image and TCB inside `area`, returning the thread pointer.
///
/// # Safety
/// `area` must designate `size` writable bytes that stay mapped for the
/// lifetime of the thread.
pub unsafe fn prepare_area(area: *mut u8, size: usize) -> Option<*mut Tcb> {
    prepare_image(area, size, tls_image())
}

unsafe fn prepare_image(area: *mut u8, size: usize, image: TlsImage) -> Option<*mut Tcb> {
    let block_size = tls_block_size(image);
    if area.is_null() || size < block_size + TCB_SIZE {
        return None;
    }
    let tcb_addr = (area as usize + size - TCB_SIZE) & !(thread_pointer_align(image) - 1);
    if tcb_addr < area as usize + block_size {
        return None;
    }
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
        _key_padding: 0,
        std_keys: [core::ptr::null_mut(); 256],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcb_preserves_native_std_and_pthread_offsets() {
        assert_eq!(core::mem::offset_of!(Tcb, self_ptr), 0);
        assert_eq!(core::mem::offset_of!(Tcb, errno), 8);
        assert_eq!(core::mem::offset_of!(Tcb, pthread), 16);
        assert_eq!(core::mem::offset_of!(Tcb, std_keys), 32);
    }

    #[test]
    fn static_tls_and_std_keys_share_mapping_without_overlap() {
        #[repr(align(8192))]
        struct Storage([u8; 32768]);
        let data = [0x5a; 73];
        for align in [1, 64, 4096, 8192] {
            let image = TlsImage {
                data: data.as_ptr(),
                data_len: data.len(),
                mem_len: 5000,
                align,
            };
            let size = area_size_for(image);
            let mut storage = Storage([0xa5; 32768]);
            let base = storage.0.as_mut_ptr();
            let tcb = unsafe { prepare_image(base, size, image).unwrap() };
            let offset = tcb as usize - base as usize;
            let block = tls_block_size(image);
            assert_eq!(tcb as usize % thread_pointer_align(image), 0);
            assert!(offset >= block);
            assert!(offset + TCB_SIZE <= size);
            assert_eq!(
                &storage.0[offset - block..offset - block + data.len()],
                &data
            );
            assert!(storage.0[offset - block + data.len()..offset]
                .iter()
                .all(|b| *b == 0));
            assert!(unsafe { (*tcb).std_keys.iter().all(|p| p.is_null()) });
            assert!(storage.0[size..].iter().all(|b| *b == 0xa5));
        }
    }

    #[test]
    fn native_std_key_table_fits_and_starts_empty() {
        // std's key backend uses all 256 pointer slots at FS_BASE + 32.
        // Poison the storage so initialization cannot rely on mmap zeroing.
        #[repr(align(4096))]
        struct Page([u8; 4096]);
        let mut page = Page([0xa5; 4096]);
        let base = page.0.as_mut_ptr();
        let tcb = unsafe { prepare_area(base, page.0.len()).unwrap() };
        let table_end = tcb as usize + 32 + 256 * core::mem::size_of::<*mut u8>();
        assert!(
            table_end <= base as usize + page.0.len(),
            "std TLS keys exceed mapping"
        );
        let table =
            unsafe { core::slice::from_raw_parts((tcb as usize + 32) as *const *mut u8, 256) };
        assert!(table.iter().all(|value| value.is_null()));
    }
}
