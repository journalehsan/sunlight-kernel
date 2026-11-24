#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use sha2::{Digest, Sha256};
use sunlight_ipc::{debug_log, launch_trace::LaunchSource, ProcessExit};
use sunlight_libc::sun_exec::{self, LaunchRequest};

const BUNDLE: &[u8] = b"/Applications/SunlightCanvas.sunapp";

#[path = "../desktop.rs"]
mod desktop;

fn installed_digest() -> Result<Sha256, sunlight_libc::Errno> {
    let fd = sunlight_libc::open(b"/Applications/SunlightCanvas.sunapp/Program/sunlight-ui-wayland")?;
    let result = (|| {
        let mut hash = Sha256::new();
        let mut buffer = [0; 4096];
        loop {
            let count = sunlight_libc::read(fd, &mut buffer)?;
            if count == 0 {
                return Ok(hash);
            }
            hash.update(&buffer[..count]);
        }
    })();
    let closed = sunlight_libc::close(fd);
    result.and_then(|hash| closed.map(|()| hash))
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[CANVAS-SUNAPP] launcher panic\n");
    ProcessExit::exit(1)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let expected = env!("SUNLIGHT_CANVAS_EXPECTED_SHA256");
    let observed = match installed_digest() {
        Ok(hash) => format!("{:x}", hash.finalize()),
        Err(error) => {
            debug_log(&format!("[CANVAS-SUNAPP] artifact read failed: {error:?}\n"));
            ProcessExit::exit(1)
        }
    };
    debug_log(&format!("[CANVAS-SUNAPP] expected sha256={expected}\n"));
    debug_log(&format!("[CANVAS-SUNAPP] installed sha256={observed}\n"));
    if observed != expected {
        debug_log("[CANVAS-SUNAPP] artifact digest mismatch\n");
        ProcessExit::exit(1);
    }
    if !desktop::wait_for_desktop(120_000) {
        debug_log("[CANVAS-SUNAPP] desktop unavailable\n");
        ProcessExit::exit(1);
    }
    debug_log("[CANVAS-SUNAPP] desktop ready for spawn\n");
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
