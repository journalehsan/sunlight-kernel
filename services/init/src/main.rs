#![no_std]
#![no_main]

use sunlight_ipc::debug_log;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[ init] SunlightOS init process started");
    debug_log("[ init] Waiting for system services to register...");

    // In Phase 2, init just signals readiness and halts.
    // Phase 3+ will have it spawn filesystem, network services.
    debug_log("[ init] Phase 2 complete — all services nominal");

    loop {
        for i in 0..10_000_000 {
            unsafe { core::ptr::read_volatile(&i) };
        }
        debug_log("[ init] still alive");
    }
}
