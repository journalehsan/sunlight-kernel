//! Unprivileged inventory; never requests mutation or legacy privileged queries.
#![no_std]
#![no_main]
use core::fmt::{self, Write};
use sunlight_libc::{crt0, exit, write_all, STDOUT};
struct Output;
impl Write for Output {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        write_all(STDOUT, text.as_bytes()).map_err(|_| fmt::Error)
    }
}
#[cfg(feature = "storage-test")]
fn list(json: bool) -> Result<(), i64> {
    struct Capture {
        bytes: [u8; 8192],
        len: usize,
    }
    impl Write for Capture {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            if self.len + s.len() > self.bytes.len() {
                return Err(fmt::Error);
            }
            self.bytes[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
            self.len += s.len();
            Ok(())
        }
    }
    let mut capture = Capture {
        bytes: [0; 8192],
        len: 0,
    };
    sunlight_utils::storage_inventory::inventory(&mut capture, json)?;
    write_all(STDOUT, &capture.bytes[..capture.len]).map_err(|_| -5)?;
    // Test-only lossless capture: the TTY's normal serial command log truncates stdout.
    for chunk in capture.bytes[..capture.len].chunks(32) {
        let prefix = if json {
            "[LSBLK-JSON-HEX] "
        } else {
            "[LSBLK-TABLE-HEX] "
        };
        let mut line = [0u8; 96];
        line[..prefix.len()].copy_from_slice(prefix.as_bytes());
        let digits = b"0123456789abcdef";
        for (i, b) in chunk.iter().enumerate() {
            line[prefix.len() + i * 2] = digits[(b >> 4) as usize];
            line[prefix.len() + i * 2 + 1] = digits[(b & 15) as usize];
        }
        sunlight_ipc::debug_log(
            core::str::from_utf8(&line[..prefix.len() + chunk.len() * 2]).map_err(|_| -5)?,
        );
    }
    Ok(())
}
#[cfg(not(feature = "storage-test"))]
fn list(json: bool) -> Result<(), i64> {
    sunlight_utils::storage_inventory::inventory(&mut Output, json)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    exit(101)
}
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut pointers = [core::ptr::null(); 4];
    let count = unsafe { crt0::collect_raw_args(argc, argv, &mut pointers) };
    let mut args = [&[][..]; 4];
    for i in 0..count {
        args[i] =
            unsafe { core::slice::from_raw_parts(pointers[i], crt0::cstr_len(pointers[i], 256)) };
    }
    let args = if args.first().is_some_and(|a| a.ends_with(b"lsblk")) {
        &args[1..count]
    } else {
        &args[..count]
    };
    let json = match args {
        [] => false,
        [b"--json"] => true,
        _ => {
            let _ = writeln!(Output, "usage: lsblk [--json]");
            exit(1)
        }
    };
    match list(json) {
        Ok(()) => exit(0),
        Err(e) => {
            let _ = writeln!(Output, "lsblk: error {e}");
            exit(1)
        }
    }
}
