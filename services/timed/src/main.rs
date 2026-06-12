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

mod config;
mod localtime;
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

    // Initialize TimeState with default values
    let mut time_state = TimeState::new();

    // Attempt to load timezone configuration from /etc/localtime
    match localtime::resolve_and_load_timezone() {
        Ok((offset_secs, dst_active, tz_name)) => {
            time_state.local_offset_secs = offset_secs;
            time_state.dst_active = dst_active;
            time_state.set_timezone_name(&tz_name);
            debug_log("[timed] Timezone loaded successfully");
        }
        Err(_) => {
            debug_log("[timed] Warning: Could not load timezone, using UTC");
            time_state.local_offset_secs = 0;
            time_state.dst_active = false;
        }
    }

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
                // Return time state: utc(0), offset(1), dst(2)
                IpcMsg::with_label(TimeMsg::REPLY)
                    .word(0, time_state.utc_epoch)
                    .word(1, time_state.local_offset_secs as u64)
                    .word(2, time_state.dst_active as u64)
            }
            TimeMsg::SET_TIMEZONE => {
                // Reload timezone configuration
                match localtime::resolve_and_load_timezone() {
                    Ok((offset_secs, dst_active, tz_name)) => {
                        time_state.local_offset_secs = offset_secs;
                        time_state.dst_active = dst_active;
                        time_state.set_timezone_name(&tz_name);
                        debug_log("[timed] Timezone reloaded");
                        IpcMsg::with_label(TimeMsg::REPLY)
                    }
                    Err(_) => IpcMsg::with_label(TimeMsg::ERROR),
                }
            }
            TimeMsg::SYNC_NTP => {
                // Placeholder for NTP sync (Phase 2.2)
                debug_log("[timed] NTP sync requested (Phase 2.2)");
                IpcMsg::with_label(TimeMsg::REPLY)
            }
            _ => IpcMsg::with_label(TimeMsg::ERROR),
        };

        msg = ipc_reply_and_wait(ep, reply);
    }
}
