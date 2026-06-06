use crate::{serial_println};
use crate::arch::x86_64::serial;

pub fn init() {
    serial::init();
    serial_println!("[SunlightOS] Kernel booting...");
    serial_println!("[SunlightOS]   arch: x86_64");
    serial_println!("[SunlightOS]   serial: COM1 initialized");
    serial_println!("[SunlightOS] Phase 0 OK — serial output working");
}
