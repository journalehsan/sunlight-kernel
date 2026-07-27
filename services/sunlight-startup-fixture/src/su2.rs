//! Session Configuration Phase 1 test fixture — startup app two.
#![no_std]
#![no_main]

use sunlight_ipc::{debug_log, process_yield};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[SESSION-CONFIG] FIXTURE_SU2_STARTED PASS\n");
    let mut spins = 0u32;
    while spins < 200 {
        process_yield();
        spins = spins.saturating_add(1);
    }
    debug_log("[SESSION-CONFIG] FIXTURE_SU2_EXIT PASS\n");
    sunlight_libc::exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        process_yield();
    }
}
