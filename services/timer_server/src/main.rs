#![no_std]
#![no_main]

use sunlight_ipc::{debug_log, ipc_recv, TimerMessage};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[timer] Timer server started");
    debug_log("[timer] Listening for tick events...");

    let mut tick_count: u64 = 0;
    loop {
        // Block until kernel sends a tick IPC message
        let msg = ipc_recv();
        if msg.tag == TimerMessage::TICK {
            tick_count += 1;
            if tick_count % 100 == 0 {
                debug_log("[timer] 100 ticks elapsed");
                debug_log("══════════════════════════════════════");
                debug_log("[SunlightOS] Phase 2 OK");
                debug_log("══════════════════════════════════════");
            }
        }
    }
}
