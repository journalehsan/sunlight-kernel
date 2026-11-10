//! Fixed ET_EXEC main-program namespace; no relocation or shared-object loading.
use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicUsize, Ordering};

static REFERENCES: AtomicUsize = AtomicUsize::new(0);
static MAIN: u8 = 0;
extern "C" {
    static __sl_exports_start: u8;
    static __sl_exports_end: u8;
    static __sl_text_start: u8;
    static __sl_text_end: u8;
}
fn main_handle() -> *mut c_void {
    core::ptr::addr_of!(MAIN) as *mut c_void
}
#[no_mangle]
pub extern "C" fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void {
    if !path.is_null() || flags != 2 {
        return core::ptr::null_mut();
    }
    if REFERENCES
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
        .is_err()
    {
        return core::ptr::null_mut();
    }
    main_handle()
}
#[no_mangle]
pub extern "C" fn dlclose(handle: *mut c_void) -> c_int {
    if handle != main_handle() {
        return -1;
    }
    if REFERENCES
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
        .is_ok()
    {
        0
    } else {
        -1
    }
}
/// Like other C string APIs, name must denote readable NUL-terminated storage.
#[no_mangle]
pub unsafe extern "C" fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void {
    if handle != main_handle() || REFERENCES.load(Ordering::Acquire) == 0 || name.is_null() {
        return core::ptr::null_mut();
    }
    let start = core::ptr::addr_of!(__sl_exports_start) as usize;
    let end = core::ptr::addr_of!(__sl_exports_end) as usize;
    // This ABI is emitted by the native ET_EXEC linker only (load bias zero).
    let Some(size) = end.checked_sub(start) else {
        return core::ptr::null_mut();
    };
    if start % 8 != 0 || size == 0 || size % 32 != 0 || size > 4096 {
        return core::ptr::null_mut();
    }
    for offset in (0..size).step_by(32) {
        let entry = (start + offset) as *const u8;
        let mut equal = false;
        for i in 0..24 {
            let expected = *entry.add(i);
            if expected != *name.add(i) as u8 {
                break;
            }
            if expected == 0 {
                equal = i != 0;
                break;
            }
        }
        if equal {
            let address = *(entry.add(24) as *const usize);
            if address >= core::ptr::addr_of!(__sl_text_start) as usize
                && address < core::ptr::addr_of!(__sl_text_end) as usize
            {
                return address as *mut c_void;
            }
            return core::ptr::null_mut();
        }
    }
    core::ptr::null_mut()
}
