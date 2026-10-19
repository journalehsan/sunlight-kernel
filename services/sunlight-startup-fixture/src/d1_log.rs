//! Single bounded serial record per assertion, including panic locations.
use core::fmt::{self, Write};
pub fn log(args: fmt::Arguments<'_>) {
    struct Record {
        bytes: [u8; 256],
        len: usize,
    }
    impl Write for Record {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            let end = self
                .len
                .checked_add(text.len())
                .filter(|end| *end <= self.bytes.len())
                .ok_or(fmt::Error)?;
            self.bytes[self.len..end].copy_from_slice(text.as_bytes());
            self.len = end;
            Ok(())
        }
    }
    let mut record = Record {
        bytes: [0; 256],
        len: 0,
    };
    let _ = record.write_fmt(args);
    if let Ok(text) = core::str::from_utf8(&record.bytes[..record.len]) {
        sunlight_ipc::debug_log(text);
    }
}
