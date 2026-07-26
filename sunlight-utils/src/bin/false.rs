//! Native POSIX-oriented `false` command.

#![no_std]
#![no_main]

use sunlight_libc::{exit, write_all, MAX_ARGS, STDERR};
use sunlight_utils::{false_cmd, native};

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = write_all(STDERR, b"false: panic\n");
    exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut args = [&[][..]; MAX_ARGS + 1];
    let status = match unsafe { native::collect_bytes(argc, argv, &mut args) } {
        Ok(count) => false_cmd::run(native::user_args(&args[..count])),
        Err(_) => {
            let _ = write_all(STDERR, b"false: argument list too long\n");
            2
        }
    };
    exit(status as u64);
}
