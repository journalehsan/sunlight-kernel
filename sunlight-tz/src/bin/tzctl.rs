#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 64 * 1024] = [0; 64 * 1024];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() { return core::ptr::null_mut(); }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

use sunlight_ipc::{
    debug_log, ipc_call, nameserver_lookup, IpcMsg, TzMsg,
};

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! { loop {} }

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[tzctl] standalone tz client starting");
    if let Some(cap) = nameserver_lookup("tz") {
        // default action: list first few
        for i in 0..4u64 {
            let req = IpcMsg::with_label(TzMsg::LIST_ZONES).word(0, i);
            let r = ipc_call(cap, req);
            if r.label == TzMsg::REPLY && r.words[0] != 0xFFFF_FFFF {
                debug_log("[tzctl] zone row received");
            } else { break; }
        }
        debug_log("[tzctl] done (list)");
    } else {
        debug_log("[tzctl] tz service not found");
    }
    // In real use sunshell builtin is preferred; this bin exits.
    sunlight_ipc::process_exit::ProcessExit::exit(0);
}
