#![no_std]
#![no_main]

use sunlight_libc::{exit, write_all, MAX_ARGS, STDERR};
use sunlight_utils::expand::{self, NativeIo};

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = write_all(STDERR, b"expand: panic\n");
    exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut pointers = [core::ptr::null::<u8>(); MAX_ARGS + 1];
    let count = unsafe { sunlight_libc::crt0::collect_raw_args(argc, argv, &mut pointers) };
    let mut args = [&[][..]; MAX_ARGS + 1];
    for (index, slot) in args.iter_mut().enumerate().take(count) {
        let ptr = pointers[index];
        let len = unsafe { sunlight_libc::crt0::cstr_len(ptr, 256) };
        *slot = unsafe { core::slice::from_raw_parts(ptr, len) };
    }

    let mut io = NativeIo;
    let status = expand::run(expand::user_args(&args[..count]), &mut io);
    exit(status as u64);
}
