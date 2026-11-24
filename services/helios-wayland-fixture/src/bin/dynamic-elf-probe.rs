#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use sunlight_ipc::{debug_log, launch_trace::LaunchSource, nameserver_lookup, ProcessExit};
use sunlight_libc::sun_exec::{self, LaunchRequest};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[DYNAMIC-ELF] launcher panic\n");
    ProcessExit::exit(1)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let start = sunlight_ipc::monotonic_millis();
    while nameserver_lookup("vfs").is_none() || nameserver_lookup("display_server").is_none() {
        if sunlight_ipc::monotonic_millis().wrapping_sub(start) >= 30_000 {
            debug_log("[DYNAMIC-ELF] launcher services unavailable\n");
            ProcessExit::exit(1);
        }
        let delay = sunlight_libc::time::Timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        };
        unsafe { sunlight_libc::time::nanosleep(&delay, core::ptr::null_mut()) };
    }
    #[cfg(feature = "dynamic-elf-relocation-fixture")]
    let mut completed = [0; 4];
    for (_index, bundle) in [
        "/Applications/elf-pie.sunapp",
        "/Applications/elf-exec.sunapp",
        "/Applications/elf-static.sunapp",
        #[cfg(feature = "dynamic-elf-relocation-fixture")]
        "/Applications/elf-relocation.sunapp",
    ].into_iter().enumerate() {
        debug_log(&format!("[DYNAMIC-ELF] launch {bundle}\n"));
        match sun_exec::launch(LaunchRequest::new(
            sun_exec::next_cli_trace(LaunchSource::Boot),
            LaunchSource::Boot,
            bundle.as_bytes(),
        )) {
            Ok(child) => {
                let status = sunlight_libc::waitpid(child.pid);
                debug_log(&format!("[DYNAMIC-ELF] pid={} waitpid={status:?}\n", child.pid));
                if status != Ok(0) {
                    ProcessExit::exit(1);
                }
                #[cfg(feature = "dynamic-elf-relocation-fixture")]
                for &old in &completed[.._index] {
                    if old == 0 {
                        continue;
                    }
                    let stale = sunlight_libc::waitpid(old);
                    debug_log(&format!(
                        "[WAIT-REGRESSION] old={old} new={} stale={stale:?}\n",
                        child.pid
                    ));
                    if stale != Err(sunlight_libc::Errno::Failed) {
                        ProcessExit::exit(1);
                    }
                }
                #[cfg(feature = "dynamic-elf-relocation-fixture")]
                {
                    completed[_index] = child.pid;
                }
            }
            Err(error) => debug_log(&format!("[DYNAMIC-ELF] launch failed: {error:?}\n")),
        }
    }
    debug_log("[DYNAMIC-ELF] done\n");
    ProcessExit::exit(0)
}
