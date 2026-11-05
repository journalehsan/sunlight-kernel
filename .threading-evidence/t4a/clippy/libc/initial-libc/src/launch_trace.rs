//! Parse the optional `--sunlight-launch=...` trace argument passed to GUI
//! apps and store it in the shared launch-trace context.

use crate::crt0::collect_raw_args;
use sunlight_ipc::launch_trace::{self, LaunchTrace};

/// Parse the launch trace arg from `argv` and publish it for `Window::connect`
/// / app startup logging.
pub fn init_from_argv(argc: u64, argv: *const *const u8) {
    launch_trace::clear_current();
    let mut raw = [core::ptr::null::<u8>(); 16];
    let count = unsafe { collect_raw_args(argc, argv, &mut raw) };
    for i in 0..count {
        let ptr = raw[i];
        if ptr.is_null() {
            continue;
        }
        let len = unsafe { crate::crt0::cstr_len(ptr, 128) };
        let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
        if let Some(trace) = launch_trace::parse_launch_arg(bytes) {
            launch_trace::set_current(trace);
            return;
        }
    }
}

/// Clear any inherited launch trace context.
pub fn clear() {
    launch_trace::clear_current();
}

/// Return the current launch trace context if present.
pub fn current() -> Option<LaunchTrace> {
    launch_trace::current()
}
