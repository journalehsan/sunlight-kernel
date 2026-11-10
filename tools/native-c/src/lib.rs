#![no_std]

// Package the canonical libc, including std_runtime's _start, for C consumers.
// There is no alternate startup, allocator, or syscall implementation here.
extern crate sunlight_libc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { sunlight_libc::std_runtime::sl_std_exit(101) }
}
