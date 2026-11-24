#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use sunlight_ipc::{
    debug_log, launch_trace::LaunchSource, monotonic_millis, nameserver_lookup, ProcessExit,
};
use sunlight_libc::sun_exec::{self, LaunchRequest};

const BUNDLE: &[u8] = b"/Applications/Qalculate.sunapp";

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[QALCULATE-PROBE] launcher panic\n");
    ProcessExit::exit(1)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let start = monotonic_millis();
    while nameserver_lookup("display_server").is_none() {
        if monotonic_millis().wrapping_sub(start) >= 30_000 {
            debug_log("[QALCULATE-PROBE] display unavailable\n");
            ProcessExit::exit(1)
        }
        let delay = sunlight_libc::time::Timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        };
        unsafe { sunlight_libc::time::nanosleep(&delay, core::ptr::null_mut()) };
    }

    debug_log("[QALCULATE-PROBE] launch /Applications/Qalculate.sunapp\n");
    match sun_exec::launch(LaunchRequest::new(
        sun_exec::next_cli_trace(LaunchSource::Boot),
        LaunchSource::Boot,
        BUNDLE,
    )) {
        Ok(child) => {
            let status = sunlight_libc::waitpid(child.pid);
            debug_log(&format!("[QALCULATE-PROBE] child status={status:?}\n"));
            ProcessExit::exit(i32::from(status != Ok(0)))
        }
        Err(error) => {
            debug_log(&format!("[QALCULATE-PROBE] launch failed: {error:?}\n"));
            ProcessExit::exit(1)
        }
    }
}
