//! Native POSIX-oriented `dirname` command.

#![no_std]
#![no_main]

use sunlight_libc::{exit, write_all, MAX_ARGS, STDERR, STDOUT};
use sunlight_utils::{dirname, native};

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = write_all(STDERR, b"dirname: panic\n");
    exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut args = [&[][..]; MAX_ARGS + 1];
    let status = match unsafe { native::collect_bytes(argc, argv, &mut args) } {
        Ok(count) => dirname::run(
            native::user_args(&args[..count]),
            &mut |bytes| write_all(STDOUT, bytes).map_err(|_| ()),
            &mut |bytes| write_all(STDERR, bytes).map_err(|_| ()),
        ),
        Err(_) => {
            let _ = write_all(STDERR, b"dirname: argument list too long\n");
            2
        }
    };
    exit(status as u64);
}
