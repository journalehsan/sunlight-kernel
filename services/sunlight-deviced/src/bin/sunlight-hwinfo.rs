#![no_std]
#![no_main]

use sunlight_ipc::{nameserver_lookup, ProcessExit};

const MAX_ARGS: usize = 4;

fn stdout_write(value: &str) {
    let mut bytes = value.as_bytes();
    while !bytes.is_empty() {
        match sunlight_libc::write(sunlight_libc::STDOUT, bytes) {
            Ok(written) if written > 0 => bytes = &bytes[written..],
            _ => break,
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    stdout_write("sunlight-hwinfo: PANIC\n");
    ProcessExit::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let args = &storage[..count];
    let mut verbose = false;
    let mut requested = None;
    for argument in args.iter().skip(1) {
        if *argument == "--verbose" {
            verbose = true;
        } else if requested.is_none() {
            requested = Some(*argument);
        } else {
            stdout_write("Usage: sunlight-hwinfo [--verbose] [device-id]\n");
            ProcessExit::exit(1);
        }
    }
    let Some(capability) = nameserver_lookup("deviced") else {
        stdout_write("sunlight-hwinfo: deviced not running\n");
        ProcessExit::exit(1);
    };
    ProcessExit::exit(
        if sunlight_deviced::print_inventory(capability, verbose, requested, stdout_write) {
            0
        } else {
            1
        },
    );
}

unsafe fn collect_args<'a>(argc: u64, argv: *const *const u8, out: &mut [&'a str]) -> usize {
    if argv.is_null() {
        return 0;
    }
    let mut count = 0usize;
    for index in 0..(argc as usize).min(out.len()) {
        let pointer = *argv.add(index);
        if pointer.is_null() {
            break;
        }
        let mut length = 0usize;
        while length < 256 && *pointer.add(length) != 0 {
            length += 1;
        }
        out[count] =
            core::str::from_utf8(core::slice::from_raw_parts(pointer, length)).unwrap_or("");
        count += 1;
    }
    count
}
