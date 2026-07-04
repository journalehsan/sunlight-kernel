#![no_std]
#![no_main]

use sunlight_ipc::{debug_log, launch_trace::LaunchSource, process_yield, ProcessExit};
use sunlight_libc::{crt0, sun_exec, sun_open};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[sun-open] panic\n");
    loop {
        process_yield();
    }
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut raw = [core::ptr::null::<u8>(); sunlight_libc::MAX_ARGS];
    let count = unsafe { crt0::collect_raw_args(argc, argv, &mut raw) };
    if count < 2 {
        debug_log("sun-open: usage: sun-open <path>\n");
        ProcessExit::exit(2);
    }

    let path_len = unsafe { crt0::cstr_len(raw[1], sunlight_libc::MAX_PATH) };
    let path = unsafe { core::slice::from_raw_parts(raw[1], path_len) };

    let trace = sun_exec::next_cli_trace(LaunchSource::Unknown);
    match sun_open::open_path(trace, LaunchSource::Unknown, path) {
        Ok(result) => {
            debug_log("sun-open: spawned pid=");
            debug_log_u64(result.pid);
            debug_log("\n");
            ProcessExit::exit(0);
        }
        Err(err) => {
            debug_log("sun-open: ");
            debug_log_error(err);
            debug_log("\n");
            ProcessExit::exit(1);
        }
    }
}

fn debug_log_error(err: sun_open::OpenError) {
    match err {
        sun_open::OpenError::MissingPath => debug_log("missing path"),
        sun_open::OpenError::PathTooLong => debug_log("path too long"),
        sun_open::OpenError::NoAssociation => {
            debug_log("No application is registered for this file type")
        }
        sun_open::OpenError::InvalidDesktopEntry => debug_log("invalid desktop entry"),
        sun_open::OpenError::LaunchFailed(launch_err) => {
            debug_log("launch failed: ");
            debug_log_launch_error(launch_err);
        }
    }
}

fn debug_log_launch_error(err: sun_exec::LaunchError) {
    match err {
        sun_exec::LaunchError::AppNotFound => debug_log("app not found"),
        sun_exec::LaunchError::InvalidCommand => debug_log("invalid command"),
        sun_exec::LaunchError::SpawnFailed(_) => debug_log("spawn failed"),
        sun_exec::LaunchError::PermissionDenied => debug_log("permission denied"),
        sun_exec::LaunchError::DisplayUnavailable => debug_log("display/session unavailable"),
        sun_exec::LaunchError::TooManyArgs => debug_log("too many arguments"),
        sun_exec::LaunchError::ArgTooLong => debug_log("argument too long"),
    }
}

fn debug_log_u64(mut value: u64) {
    let mut buf = [0u8; 20];
    let mut idx = buf.len();
    if value == 0 {
        debug_log("0");
        return;
    }
    while value != 0 {
        idx -= 1;
        buf[idx] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    debug_log(core::str::from_utf8(&buf[idx..]).unwrap_or("0"));
}
