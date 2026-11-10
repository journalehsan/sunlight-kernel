#![no_std]
#![no_main]
use sunlight_libc as libc;
#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    libc::exit(101)
}
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, envp: *const *const u8) -> ! {
    libc::env::init(envp);
    let mut pointers = [core::ptr::null(); libc::MAX_ARGS + 1];
    let count = unsafe { libc::crt0::collect_raw_args(argc, argv, &mut pointers) };
    let mut args = [""; libc::MAX_ARGS + 1];
    for i in 0..count {
        let len = unsafe { libc::crt0::cstr_len(pointers[i], libc::MAX_PATH) };
        let bytes = unsafe { core::slice::from_raw_parts(pointers[i], len) };
        args[i] = match core::str::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => libc::exit(2),
        };
    }
    libc::exit(sunlight_utils::mktemp::run(args[..count].get(1..).unwrap_or(&[])) as u64)
}
