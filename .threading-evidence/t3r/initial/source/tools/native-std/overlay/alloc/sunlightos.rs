use super::*;
use crate::sys::sunlightos::{sl_std_alloc, sl_std_free};
#[stable(feature = "alloc_system_type", since = "1.28.0")]
unsafe impl GlobalAlloc for System {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { sl_std_alloc(layout.size(), layout.align()) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { sl_std_free(ptr, layout.size(), layout.align()) }
    }
}
