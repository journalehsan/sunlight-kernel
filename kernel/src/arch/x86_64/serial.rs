use core::fmt;
use uart_16550::SerialPort;
use spin::Mutex;

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

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    let mut writer = SerialWriter(SERIAL.lock());
    writer.write_fmt(args).unwrap();
}
