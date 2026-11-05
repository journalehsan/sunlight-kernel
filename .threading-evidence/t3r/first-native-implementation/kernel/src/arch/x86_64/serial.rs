use core::fmt;
use spin::Mutex;
use uart_16550::SerialPort;

const SERIAL_PORT: u16 = 0x3F8;

static SERIAL: Mutex<SerialPort> = Mutex::new(unsafe { SerialPort::new(SERIAL_PORT) });

pub fn init() {
    SERIAL.lock().init();
}

struct SerialWriter<'a>(spin::MutexGuard<'a, SerialPort>);

impl<'a> fmt::Write for SerialWriter<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            self.0.send(byte);
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::arch::x86_64::serial::_print(format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => ($crate::serial_print!("{}\n", format_args!($($arg)*)));
}

/// Non-blocking serial print for panic/OOM paths.
///
/// Never takes a blocking lock and never panics on format failure. If the
/// serial mutex is already held (including by this CPU), the message is
/// dropped rather than hanging the panic renderer.
#[macro_export]
macro_rules! serial_print_try {
    ($($arg:tt)*) => {
        $crate::arch::x86_64::serial::_try_print(format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! serial_println_try {
    () => ($crate::serial_print_try!("\n"));
    ($($arg:tt)*) => ($crate::serial_print_try!("{}\n", format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    x86_64::instructions::interrupts::without_interrupts(|| {
        let mut writer = SerialWriter(SERIAL.lock());
        // Normal path: keep prior behavior. Panic paths must use `_try_print`.
        let _ = writer.write_fmt(args);
    });
}

#[doc(hidden)]
pub fn _try_print(args: fmt::Arguments) {
    use core::fmt::Write;
    // Interrupts should already be off on the panic path; try_lock avoids
    // deadlock if this CPU (or another) already holds SERIAL.
    if let Some(serial) = SERIAL.try_lock() {
        let mut writer = SerialWriter(serial);
        let _ = writer.write_fmt(args);
    }
}
