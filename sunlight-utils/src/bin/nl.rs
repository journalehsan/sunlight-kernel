#![no_std]
#![no_main]

use sunlight_libc::{exit, write_all, MAX_ARGS, STDERR};
use sunlight_utils::nl::{self, NativeIo};

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = write_all(STDERR, b"nl: panic\n");
    exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut args = [&[][..]; MAX_ARGS + 1];
    let count =
        unsafe { sunlight_utils::native::collect_bytes(argc, argv, &mut args) }.unwrap_or(0);
    let mut io = NativeIo;
    exit(nl::run(nl::user_args(&args[..count]), &mut io) as u64);
}
