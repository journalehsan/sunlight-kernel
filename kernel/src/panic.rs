use crate::serial_println;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[KERNEL PANIC] {}", info);
    loop {
        core::arch::x86_64::_mm_pause();
    }
}
