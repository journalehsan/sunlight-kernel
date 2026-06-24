//! devicectl — tiny CLI for the deviced v0 IPC protocol.

#![no_std]
#![no_main]

use sunlight_ipc::{ipc_call, nameserver_lookup, DevicedMsg, DriverKind, DriverState, IpcMsg};

const MAX_ARGS: usize = 8;

fn stdout_write(s: &str) {
    let mut data = s.as_bytes();
    while !data.is_empty() {
        match sunlight_libc::write(sunlight_libc::STDOUT, data) {
            Ok(n) if n > 0 => data = &data[n..],
            _ => break,
        }
    }
}

macro_rules! println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<512>::new();
        let _ = write!(&mut buf, $($arg)*);
        stdout_write(&buf);
        stdout_write("\n");
    }};
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    println!("devicectl: PANIC");
    sunlight_ipc::ProcessExit::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let args = &storage[..count];

    let Some(cap) = nameserver_lookup("deviced") else {
        println!("devicectl: deviced not running");
        sunlight_ipc::ProcessExit::exit(1);
    };

    let code = match args.get(1).copied().unwrap_or("list") {
        "list" | "drivers" => {
            print_drivers(cap, false);
            0
        }
        "devices" => {
            print_devices(cap);
            0
        }
        "json" => {
            print_drivers(cap, true);
            0
        }
        "status" => {
            if args.len() < 3 {
                print_usage();
                1
            } else {
                print_status(cap, args[2])
            }
        }
        _ => {
            print_usage();
            1
        }
    };

    sunlight_ipc::ProcessExit::exit(code);
}

fn print_usage() {
    println!("Usage: devicectl <list|drivers|devices|status|json> [name-or-id]");
}

fn print_drivers(cap: sunlight_ipc::CapabilityToken, json: bool) {
    if json {
        println!("[");
    } else {
        println!("DRIVER   KIND      PID   STATE      CAPS");
    }

    let mut idx = 0u64;
    let mut first = true;
    loop {
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(DevicedMsg::LIST_DRIVERS).word(0, idx),
        );
        if reply.label != DevicedMsg::REPLY {
            break;
        }
        if json {
            if !first {
                println!(",");
            }
            print_driver_json(&reply);
            first = false;
        } else {
            print_driver_row(&reply);
        }
        idx += 1;
        if idx >= total_from_driver_reply(&reply) {
            break;
        }
    }

    if json {
        println!("]");
    }
}

fn print_devices(cap: sunlight_ipc::CapabilityToken) {
    println!("DEVICE   KIND      DRIVER  STATE");
    let mut idx = 0u64;
    loop {
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(DevicedMsg::LIST_DEVICES).word(0, idx),
        );
        if reply.label != DevicedMsg::REPLY {
            break;
        }
        let name = ShortName(reply.words[1]);
        let kind = device_kind_str(reply.words[3] & 0xff);
        let state = state_str((reply.words[3] >> 8) & 0xff);
        println!("{:<8} {:<9} {:<7} {}", name, kind, reply.words[2], state);
        idx += 1;
        if idx >= ((reply.words[3] >> 32) & 0xff) {
            break;
        }
    }
}

fn print_status(cap: sunlight_ipc::CapabilityToken, key: &str) -> i32 {
    let packed = parse_u64(key).unwrap_or_else(|| pack_short_name(key));
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(DevicedMsg::GET_DRIVER).word(0, packed),
    );
    if reply.label != DevicedMsg::REPLY {
        println!("devicectl: driver not found: {}", key);
        return 1;
    }
    let caps = caps_from_driver_reply(&reply);
    println!("Driver: {}", ShortName(reply.words[1]));
    println!("ID: {}", reply.words[0]);
    println!("PID: {}", reply.words[2]);
    println!("Kind: {}", kind_str(kind_from_driver_reply(&reply) as u64));
    println!(
        "State: {}",
        state_str(state_from_driver_reply(&reply) as u64)
    );
    println!("Caps: {}", Caps(caps));
    0
}

fn print_driver_row(reply: &IpcMsg) {
    println!(
        "{:<8} {:<9} {:<5} {:<10} {}",
        ShortName(reply.words[1]),
        kind_str(kind_from_driver_reply(reply) as u64),
        reply.words[2],
        state_str(state_from_driver_reply(reply) as u64),
        Caps(caps_from_driver_reply(reply))
    );
}

fn print_driver_json(reply: &IpcMsg) {
    println!(
        "  {{\"id\":{},\"name\":\"{}\",\"kind\":\"{}\",\"pid\":{},\"state\":\"{}\",\"caps\":\"{}\"}}",
        reply.words[0],
        ShortName(reply.words[1]),
        kind_str(kind_from_driver_reply(reply) as u64),
        reply.words[2],
        state_str(state_from_driver_reply(reply) as u64),
        Caps(caps_from_driver_reply(reply))
    );
}

fn kind_from_driver_reply(reply: &IpcMsg) -> DriverKind {
    DriverKind::from_u64(reply.words[3] & 0xff)
}

fn state_from_driver_reply(reply: &IpcMsg) -> DriverState {
    DriverState::from_u64((reply.words[3] >> 8) & 0xff)
}

fn caps_from_driver_reply(reply: &IpcMsg) -> u64 {
    (reply.words[3] >> 40) & 0x00ff_ffff
}

fn total_from_driver_reply(reply: &IpcMsg) -> u64 {
    (reply.words[3] >> 32) & 0xff
}

fn kind_str(value: u64) -> &'static str {
    match DriverKind::from_u64(value) {
        DriverKind::Virtio => "Virtio",
        DriverKind::Keyboard => "Keyboard",
        DriverKind::Mouse => "Mouse",
        DriverKind::Network => "Network",
        DriverKind::Storage => "Storage",
        DriverKind::Block => "Block",
        DriverKind::Display => "Display",
        DriverKind::Audio => "Audio",
        DriverKind::Power => "Power",
        DriverKind::Unknown => "Unknown",
    }
}

fn device_kind_str(value: u64) -> &'static str {
    match sunlight_ipc::DeviceKind::from_u64(value) {
        sunlight_ipc::DeviceKind::Input => "Input",
        sunlight_ipc::DeviceKind::Network => "Network",
        sunlight_ipc::DeviceKind::Block => "Block",
        sunlight_ipc::DeviceKind::Display => "Display",
        sunlight_ipc::DeviceKind::Audio => "Audio",
        sunlight_ipc::DeviceKind::Bus => "Bus",
        sunlight_ipc::DeviceKind::Unknown => "Unknown",
    }
}

fn state_str(value: u64) -> &'static str {
    match DriverState::from_u64(value) {
        DriverState::Starting => "Starting",
        DriverState::Ready => "Ready",
        DriverState::Blocked => "Blocked",
        DriverState::Failed => "Failed",
        DriverState::Restarting => "Restarting",
        DriverState::Stopped => "Stopped",
    }
}

struct ShortName(u64);

impl core::fmt::Display for ShortName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for i in 0..8 {
            let b = ((self.0 >> (i * 8)) & 0xff) as u8;
            if b == 0 {
                break;
            }
            f.write_str(core::str::from_utf8(&[b]).unwrap_or("?"))?;
        }
        Ok(())
    }
}

struct Caps(u64);

impl core::fmt::Display for Caps {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut first = true;
        write_cap(
            f,
            &mut first,
            self.0,
            sunlight_ipc::DriverCaps::INPUT,
            "input",
        )?;
        write_cap(
            f,
            &mut first,
            self.0,
            sunlight_ipc::DriverCaps::KEYBOARD,
            "keyboard",
        )?;
        write_cap(
            f,
            &mut first,
            self.0,
            sunlight_ipc::DriverCaps::POINTER,
            "pointer",
        )?;
        write_cap(
            f,
            &mut first,
            self.0,
            sunlight_ipc::DriverCaps::RELATIVE_MOTION,
            "relative-motion",
        )?;
        write_cap(
            f,
            &mut first,
            self.0,
            sunlight_ipc::DriverCaps::VIRTIO,
            "virtio",
        )?;
        write_cap(f, &mut first, self.0, sunlight_ipc::DriverCaps::BUS, "bus")?;
        write_cap(
            f,
            &mut first,
            self.0,
            sunlight_ipc::DriverCaps::NETWORK,
            "net",
        )?;
        write_cap(
            f,
            &mut first,
            self.0,
            sunlight_ipc::DriverCaps::BLOCK,
            "block",
        )?;
        write_cap(
            f,
            &mut first,
            self.0,
            sunlight_ipc::DriverCaps::STORAGE,
            "storage",
        )?;
        write_cap(
            f,
            &mut first,
            self.0,
            sunlight_ipc::DriverCaps::DISPLAY,
            "display",
        )?;
        if first {
            f.write_str("-")?;
        }
        Ok(())
    }
}

fn write_cap(
    f: &mut core::fmt::Formatter<'_>,
    first: &mut bool,
    caps: u64,
    bit: u64,
    name: &str,
) -> core::fmt::Result {
    if caps & bit != 0 {
        if !*first {
            f.write_str(",")?;
        }
        f.write_str(name)?;
        *first = false;
    }
    Ok(())
}

fn parse_u64(s: &str) -> Option<u64> {
    s.parse::<u64>().ok()
}

fn pack_short_name(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut word = 0u64;
    let mut i = 0usize;
    while i < bytes.len().min(8) {
        word |= (bytes[i] as u64) << (i * 8);
        i += 1;
    }
    word
}

unsafe fn collect_args<'a>(argc: u64, argv: *const *const u8, out: &mut [&'a str]) -> usize {
    if argv.is_null() {
        return 0;
    }
    let mut count = 0usize;
    for i in 0..(argc as usize).min(out.len()) {
        let ptr = *argv.add(i);
        if ptr.is_null() {
            break;
        }
        let mut len = 0usize;
        while len < 256 && *ptr.add(len) != 0 {
            len += 1;
        }
        out[count] = core::str::from_utf8(core::slice::from_raw_parts(ptr, len)).unwrap_or("");
        count += 1;
    }
    count
}
