//! Isolated host proof for the production SMP timekeeper.

#![allow(dead_code)]

#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        let _ = core::format_args!($($arg)*);
    }};
}

#[path = "../kernel/src/timekeeping.rs"]
mod timekeeping;
