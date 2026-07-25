//! Native Tier 0 `echo` command using the Sunlight libc userland path.

#![no_std]
#![no_main]

use sunlight_libc::{crt0, exit, write_all, MAX_ARGS, STDERR, STDOUT};
use sunlight_utils::echo;

const MAX_ARG_LENGTH: usize = 256;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = write_all(STDERR, b"echo: panic\n");
    exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut args = [""; MAX_ARGS + 1];
    let count = unsafe { crt0::collect_utf8_args(argc, argv, &mut args, MAX_ARG_LENGTH) };

    let mut write = |bytes: &[u8]| write_all(STDOUT, bytes).map_err(|_| ());
    // argv[0] names the executable; echo only receives user arguments.
    let user_args = echo::user_args(&args[..count]);
    let status = echo::run(user_args, &mut write);
    exit(status as u64);
}
