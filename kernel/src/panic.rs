use crate::serial_println;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[KERNEL PANIC] {}", info);
    loop {
        core::arch::x86_64::_mm_pause();
    }
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    serial_println!(
        "[OOM] Allocation of {} bytes (align={}) failed",
        layout.size(),
        layout.align()
    );
    loop {
        core::arch::x86_64::_mm_pause();
    }
}
