#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 256 * 1024] = [0; 256 * 1024];
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

use sunlight_ipc::{
    debug_log, endpoint_create, get_time_utc, ipc_recv, ipc_reply_and_wait, nameserver_register,
    IpcMsg, TimeMsg,
};

mod ntp;
mod state;

use state::TimeState;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[timed] Time daemon started");

    // Initialize TimeState with default values (pure UTC mode)
    let mut time_state = TimeState::new();

    // Timezone handling has moved to timezone_service ("tz").
    // timed is now a pure UTC time source.
    debug_log("[timed] UTC mode — timezone handled by timezone_service");

    // Get initial UTC time from kernel
    time_state.utc_epoch = get_time_utc();
    debug_log("[timed] Initial UTC time acquired");

    // Create IPC endpoint and register with nameserver
    let ep = endpoint_create();
    nameserver_register("timed", ep);
    debug_log("[timed] Registered as 'timed' with init nameserver");

    debug_log("[timed] Entering main IPC loop");

    let mut msg = ipc_recv(ep);
    loop {
        // Update current UTC time
        time_state.utc_epoch = get_time_utc();

        let reply = match msg.label {
            TimeMsg::GET_TIME => {
                // Return current UTC time in word 0
                IpcMsg::with_label(TimeMsg::REPLY).word(0, time_state.utc_epoch)
            }
            TimeMsg::GET_STATE => {
                // Back-compat: timezone logic moved to timezone_service ("tz").
                // Always report offset=0, dst=false.
                IpcMsg::with_label(TimeMsg::REPLY)
                    .word(0, time_state.utc_epoch)
                    .word(1, 0u64)   // offset_secs — always 0 (UTC)
                    .word(2, 0u64)   // dst_active  — always false
            }
            TimeMsg::SET_TIMEZONE => {
                // No-op: timezone is managed by timezone_service, not timed.
                debug_log("[timed] SET_TIMEZONE ignored — use timezone_service");
                IpcMsg::with_label(TimeMsg::REPLY)
            }
            TimeMsg::SYNC_NTP => {
                // Placeholder for NTP sync (Phase 2.2)
                debug_log("[timed] NTP sync requested (Phase 2.2)");
                IpcMsg::with_label(TimeMsg::REPLY)
            }
            TimeMsg::GET_UTC => {
                // Alias for GET_TIME (clarity for new callers)
                IpcMsg::with_label(TimeMsg::REPLY).word(0, time_state.utc_epoch)
            }
            _ => IpcMsg::with_label(TimeMsg::ERROR),
        };

        msg = ipc_reply_and_wait(ep, reply);
    }
}
