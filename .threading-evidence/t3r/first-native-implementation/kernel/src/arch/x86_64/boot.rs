use crate::arch::x86_64::serial;

#[allow(dead_code)]
pub fn init() {
    serial::init();
}
