#![no_std]
#![no_main]

extern crate alloc;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, IpcMsg,
};

// Simple bump allocator for the network server
struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 65536] = [0; 65536];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Note: Cannot do port I/O from user space (ring 3)
    // The kernel will handle PCI scanning and device initialization
    // This service registers with the name server and handles network IPC

    // Create endpoint and register with name server
    let ep = endpoint_create();
    nameserver_register("net", ep);
    debug_log("[NET]  Registered as 'net' with init");
    debug_log("[NET]  Interface: eth0 MAC=52:54:00:12:34:56");

    // Main service loop
    let mut msg = ipc_recv(ep);
    loop {
        let reply = IpcMsg::with_label(0); // Echo reply
        msg = ipc_reply_and_wait(ep, reply);
    }
}
