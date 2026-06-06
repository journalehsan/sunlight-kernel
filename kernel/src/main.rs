#![no_std]
#![no_main]
#![deny(warnings)]

mod arch;
mod panic;

use core::arch::asm;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    arch::x86_64::boot::init();

    // Halt loop
    loop {
        unsafe {
            asm!("hlt");
        }
    }
}
