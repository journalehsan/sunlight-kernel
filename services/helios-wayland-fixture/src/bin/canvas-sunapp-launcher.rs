#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use sunlight_ipc::{debug_log, launch_trace::LaunchSource, monotonic_millis, nameserver_lookup, ProcessExit};
use sunlight_libc::sun_exec::{self, LaunchRequest};

const BUNDLE: &[u8] = b"/Applications/SunlightCanvas.sunapp";

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[CANVAS-SUNAPP] launcher panic\n");
    ProcessExit::exit(1)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let start = monotonic_millis();
    while nameserver_lookup("display_server").is_none() {
        if monotonic_millis().wrapping_sub(start) >= 30_000 {
            debug_log("[CANVAS-SUNAPP] display unavailable\n");
            ProcessExit::exit(1);
        }
        let delay = sunlight_libc::time::Timespec { tv_sec: 0, tv_nsec: 100_000_000 };
        unsafe { sunlight_libc::time::nanosleep(&delay, core::ptr::null_mut()) };
    }
    debug_log("[CANVAS-SUNAPP] launch /Applications/SunlightCanvas.sunapp env_keys=[] endpoint=/sunlight/native-unix/wayland-fixture\n");
    let result = sun_exec::launch_with_environment(
        LaunchRequest {
            trace: sun_exec::next_cli_trace(LaunchSource::Boot),
            source: LaunchSource::Boot,
            command: BUNDLE,
            args: &[],
            require_display: true,
        },
        Some(b""),
    );
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            debug_log(&format!("[CANVAS-SUNAPP] launch failed: {error:?}\n"));
            ProcessExit::exit(1);
        }
    };
    let path = core::str::from_utf8(result.path()).unwrap();
    debug_log(&format!("[CANVAS-SUNAPP] spawned pid={} executable={path} argv=[{path}]\n", result.pid));
    let status = sunlight_libc::waitpid(result.pid);
    debug_log(&format!("[CANVAS-SUNAPP] waitpid pid={} status={status:?}\n", result.pid));
    ProcessExit::exit(i32::from(status != Ok(0)))
}
