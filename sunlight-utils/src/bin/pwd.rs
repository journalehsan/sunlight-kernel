//! Native `pwd` command using the Sunlight libc userland path.

#![no_std]
#![no_main]

use sunlight_libc::{crt0, exit, getcwd, write_all, MAX_ARGS, MAX_PATH, STDERR, STDOUT};
use sunlight_utils::pwd;

const MAX_ARG_LENGTH: usize = 256;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = write_all(STDERR, b"pwd: panic\n");
    exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut pointers = [core::ptr::null::<u8>(); MAX_ARGS + 1];
    let count = unsafe { crt0::collect_raw_args(argc, argv, &mut pointers) };
    let mut args = [&[][..]; MAX_ARGS + 1];
    for (index, slot) in args.iter_mut().enumerate().take(count) {
        let ptr = pointers[index];
        let len = unsafe { crt0::cstr_len(ptr, MAX_ARG_LENGTH) };
        *slot = unsafe { core::slice::from_raw_parts(ptr, len) };
    }

    let mut stdout = |bytes: &[u8]| write_all(STDOUT, bytes);
    let status = pwd::run(
        pwd::user_args(&args[..count]),
        &mut |buf: &mut [u8; MAX_PATH]| getcwd(buf),
        &mut stdout,
    );
    exit(status as u64);
}
