//! thermalctl — CLI for sunlight-thermald.

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 32 * 1024] = [0; 32 * 1024];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

use core::fmt::Write;
use sunlight_ipc::{
    ipc_call, nameserver_lookup, system_identity, CoolingProfile, FanControlMode, FanLevel, IpcMsg,
    LeaseState, PowerProfile, SystemIdentityRecord, ThermalState, ThermaldMsg,
};

const MAX_ARGS: usize = 16;

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
    println!("thermalctl: PANIC");
    sunlight_ipc::ProcessExit::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let args = &storage[..count];

    let Some(cap) = nameserver_lookup("thermald") else {
        println!("thermalctl: thermald not running");
        sunlight_ipc::ProcessExit::exit(1);
    };

    let sub = args.get(1).copied().unwrap_or("status");
    let code = match sub {
        "status" => {
            print_status(cap);
            0
        }
        "sensors" => {
            print_sensors(cap);
            0
        }
        "identity" => {
            print_identity();
            0
        }
        "fans" => {
            print_fans(cap);
            0
        }
        "profile" => {
            if args.len() >= 3 && args[2] == "set" {
                if args.len() < 4 {
                    println!("Usage: thermalctl profile set <balanced|quiet|cool|performance>");
                    1
                } else {
                    do_set_profile(cap, args[3])
                }
            } else {
                print_profile(cap);
                0
            }
        }
        "auto" => do_auto(cap),
        "reset-defaults" | "reset" => do_reset(cap),
        _ => {
            print_usage();
            1
        }
    };

    sunlight_ipc::ProcessExit::exit(code);
}

fn print_usage() {
    println!("Usage: thermalctl <status|sensors|identity|fans|profile|auto|reset-defaults>");
    println!("  thermalctl status");
    println!("  thermalctl sensors");
    println!("  thermalctl identity");
    println!("  thermalctl fans");
    println!("  thermalctl profile");
    println!("  thermalctl profile set <balanced|quiet|cool|performance>");
    println!("  thermalctl auto");
    println!("  thermalctl reset-defaults");
}

fn profile_from_str(s: &str) -> Option<CoolingProfile> {
    match s {
        "balanced" | "Balanced" => Some(CoolingProfile::Balanced),
        "quiet" | "Quiet" => Some(CoolingProfile::Quiet),
        "cool" | "Cool" => Some(CoolingProfile::Cool),
        "performance" | "Performance" => Some(CoolingProfile::Performance),
        _ => None,
    }
}

struct StatusView {
    state: ThermalState,
    fan_mode: FanControlMode,
    profile: CoolingProfile,
    lease: LeaseState,
    temp_mc: Option<i32>,
    level: FanLevel,
    rpm: u32,
    power_req: PowerProfile,
    power_eff: PowerProfile,
    constraint: bool,
    on_ac: Option<bool>,
    errors: u32,
    model: u64,
    package_sensor: bool,
    sensor_count: u64,
}

fn fetch_status(cap: sunlight_ipc::CapabilityToken) -> Option<StatusView> {
    let reply = ipc_call(cap, IpcMsg::with_label(ThermaldMsg::GET_STATUS));
    if reply.label != ThermaldMsg::REPLY {
        return None;
    }
    let w0 = reply.words[0];
    let w1 = reply.words[1];
    let w2 = reply.words[2];
    let w3 = reply.words[3];
    let w4 = reply.words[4];
    let temp_bits = w1 as u32 as i32;
    let temp_mc = if temp_bits == i32::MIN {
        None
    } else {
        Some(temp_bits)
    };
    let on_ac = if (w3 & (1 << 16)) != 0 {
        Some((w3 & (1 << 17)) != 0)
    } else {
        None
    };
    Some(StatusView {
        state: ThermalState::from_u64(w0 & 0xff),
        fan_mode: FanControlMode::from_u64((w0 >> 8) & 0xff),
        profile: CoolingProfile::from_u64((w0 >> 16) & 0xff),
        lease: LeaseState::from_u64((w0 >> 24) & 0xff),
        temp_mc,
        level: FanLevel::from_u64(w2 & 0xff).unwrap_or(FanLevel::Level0),
        rpm: ((w2 >> 8) & 0xffff) as u32,
        power_req: PowerProfile::from_u64(w3 & 0xff),
        power_eff: PowerProfile::from_u64((w3 >> 8) & 0xff),
        constraint: (w3 & (1 << 24)) != 0,
        on_ac,
        errors: (w4 & 0xffff_ffff) as u32,
        model: (w4 >> 32) & 0xff,
        package_sensor: ((w4 >> 40) & 1) != 0,
        sensor_count: (w4 >> 48) & 0xff,
    })
}

fn fmt_temp(mc: Option<i32>) -> heapless::String<16> {
    let mut s = heapless::String::new();
    match mc {
        Some(t) => {
            let whole = t / 1000;
            let _ = write!(s, "{}°C", whole);
        }
        None => {
            let _ = s.push_str("Unavailable");
        }
    }
    s
}

fn model_name(tag: u64) -> &'static str {
    match tag {
        1 => "Generic",
        2 => "ThinkPad T440p",
        3 => "ThinkPad T480",
        _ => "Unknown",
    }
}

fn print_device_line() {
    if let Some(id) = system_identity() {
        let mfr = SystemIdentityRecord::field_str(&id.manufacturer);
        let prod = SystemIdentityRecord::field_str(&id.product_name);
        if !mfr.is_empty() || !prod.is_empty() {
            println!("Device: {} {}", mfr, prod);
            return;
        }
    }
    println!("Device: Unknown");
}

fn print_status(cap: sunlight_ipc::CapabilityToken) {
    let Some(st) = fetch_status(cap) else {
        println!("thermalctl: failed to get status");
        return;
    };

    print_device_line();
    if let Some(id) = system_identity() {
        if id.smbios_major != 0 {
            println!("SMBIOS: {}.{}", id.smbios_major, id.smbios_minor);
        }
    }
    println!("Thermal state: {}", st.state.as_str());
    if matches!(st.state, ThermalState::Unavailable) && st.temp_mc.is_none() {
        // Explicit Unavailable — never imply Normal without a controlling sensor.
        println!("CPU temperature: Unavailable");
        if (st.errors & 0x1) != 0 {
            println!("Reason: No thermal telemetry / no valid controlling sensor");
        } else if (st.errors & 0x2) != 0 {
            println!("Reason: Sensor stale");
        } else {
            println!("Reason: No valid controlling sensor");
        }
    } else {
        let temp = fmt_temp(st.temp_mc);
        if st.temp_mc.is_some() {
            if st.package_sensor {
                println!("CPU package maximum: {}", temp.as_str());
            } else {
                println!("CPU maximum: {} (maximum core temperature)", temp.as_str());
            }
        } else {
            println!("CPU temperature: Unavailable");
        }
    }

    let mut valid = 0u32;
    let mut stale = 0u32;
    // Best-effort recount via LIST_SENSORS
    for i in 0..st.sensor_count.max(1) {
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(ThermaldMsg::LIST_SENSORS).word(0, i),
        );
        if reply.label != ThermaldMsg::REPLY {
            break;
        }
        let status = reply.words[4] & 0xff;
        if status == 1 {
            valid += 1;
        } else if status == 4 {
            stale += 1;
        }
        let total = reply.words[3];
        if i + 1 >= total {
            break;
        }
    }
    println!("Sensors: {} valid, {} stale", valid, stale);

    match st.fan_mode {
        FanControlMode::FirmwareAuto => println!("Fan: Firmware Auto"),
        FanControlMode::Unavailable => println!("Fan: Unavailable"),
        other => println!("Fan: {}", other.as_str()),
    }
    if st.rpm > 0 {
        println!("Fan RPM: {}", st.rpm);
    } else {
        println!("Fan RPM: Unavailable");
    }
    println!("Managed control: unavailable");
    println!("  Disabled — safe EC backend not implemented");
    println!("Cooling profile: {}", st.profile.as_str());
    println!(
        "Power policy: requested {} / effective {}",
        st.power_req.as_str(),
        st.power_eff.as_str()
    );
    if st.constraint {
        println!("Thermal constraint: active");
    }
    println!("Lease: {}", st.lease.as_str());
    println!("Hardware model tag: {}", model_name(st.model));
    match st.on_ac {
        Some(true) => println!("Power source: AC"),
        Some(false) => println!("Power source: Battery"),
        None => println!("Power source: unknown"),
    }
}

fn sensor_class_name(class: u8) -> &'static str {
    match class {
        1 => "CPU package",
        2 => "Core",
        3 => "Logical CPU",
        _ => "Sensor",
    }
}

fn sensor_source_name(src: u8) -> &'static str {
    match src {
        1 => "Intel DTS",
        2 => "ACPI",
        3 => "EC",
        4 => "Mock",
        _ => "Unknown",
    }
}

fn sensor_status_name(st: u8) -> &'static str {
    match st {
        1 => "Valid",
        2 => "Unavailable",
        3 => "Unsupported",
        4 => "Stale",
        5 => "Invalid",
        6 => "HardwareError",
        _ => "Unavailable",
    }
}

fn print_sensors(cap: sunlight_ipc::CapabilityToken) {
    println!("Sensors:");
    let mut any = false;
    for i in 0..32u64 {
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(ThermaldMsg::LIST_SENSORS).word(0, i),
        );
        if reply.label != ThermaldMsg::REPLY {
            if i == 0 {
                println!("  (none / unavailable)");
            }
            break;
        }
        any = true;
        let id = reply.words[0];
        let valid = reply.words[1] != 0;
        let temp_bits = reply.words[2] as u32 as i32;
        let total = reply.words[3];
        let meta = reply.words[4];
        let status = (meta & 0xff) as u8;
        let class = ((meta >> 8) & 0xff) as u8;
        let source = ((meta >> 16) & 0xff) as u8;
        let label = ((meta >> 24) & 0xff) as u8;

        let name = sensor_class_name(class);
        if valid {
            println!(
                "  [{}] {} {}: {}°C  source={} status={}",
                i,
                name,
                label,
                temp_bits / 1000,
                sensor_source_name(source),
                sensor_status_name(status)
            );
        } else {
            println!(
                "  [{}] {} {}: {}  source={} (not 0°C)",
                i,
                name,
                label,
                sensor_status_name(status),
                sensor_source_name(source)
            );
        }
        let _ = id;
        if i + 1 >= total {
            break;
        }
    }
    if !any {
        println!("  (none)");
    }
}

fn print_identity() {
    // Public identity only — never print serial/UUID.
    let Some(id) = system_identity() else {
        // Fall back to thermald GET_IDENTITY partial.
        let Some(cap) = nameserver_lookup("thermald") else {
            println!("identity: unavailable");
            return;
        };
        let reply = ipc_call(cap, IpcMsg::with_label(ThermaldMsg::GET_IDENTITY));
        if reply.label != ThermaldMsg::REPLY {
            println!("identity: unavailable");
            return;
        }
        let major = reply.words[0] & 0xff;
        let minor = (reply.words[0] >> 8) & 0xff;
        println!("SMBIOS: {}.{}", major, minor);
        println!("(full strings require system_identity syscall)");
        return;
    };
    println!(
        "Manufacturer: {}",
        SystemIdentityRecord::field_str(&id.manufacturer)
    );
    println!(
        "Product: {}",
        SystemIdentityRecord::field_str(&id.product_name)
    );
    println!(
        "Product version: {}",
        SystemIdentityRecord::field_str(&id.product_version)
    );
    println!(
        "Board: {} {}",
        SystemIdentityRecord::field_str(&id.board_manufacturer),
        SystemIdentityRecord::field_str(&id.board_product)
    );
    println!(
        "BIOS: {} {}",
        SystemIdentityRecord::field_str(&id.bios_vendor),
        SystemIdentityRecord::field_str(&id.bios_version)
    );
    println!("SMBIOS: {}.{}", id.smbios_major, id.smbios_minor);
    println!("Confidence: {}", id.identity_confidence);
    // Explicitly do not print serial number or UUID.
}

fn print_fans(cap: sunlight_ipc::CapabilityToken) {
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(ThermaldMsg::LIST_COOLING).word(0, 0),
    );
    if reply.label != ThermaldMsg::REPLY {
        println!("thermalctl: no cooling devices");
        return;
    }
    let mode = FanControlMode::from_u64(reply.words[1]);
    let level = FanLevel::from_u64(reply.words[2]).unwrap_or(FanLevel::Level0);
    let rpm = reply.words[3];
    println!("Cooling devices:");
    println!("  [0] Fan mode={}", mode.as_str());
    println!("      requested_level={}", level as u64);
    if rpm > 0 {
        println!("      rpm={}", rpm);
    } else {
        println!("      rpm=Unavailable");
    }
    println!("  Managed fan control: Disabled — safe EC backend not implemented");
}

fn print_profile(cap: sunlight_ipc::CapabilityToken) {
    let reply = ipc_call(cap, IpcMsg::with_label(ThermaldMsg::GET_PROFILE));
    if reply.label != ThermaldMsg::REPLY {
        println!("thermalctl: profile unavailable");
        return;
    }
    let p = CoolingProfile::from_u64(reply.words[0]);
    println!("Cooling profile: {}", p.as_str());
    if p == CoolingProfile::Balanced {
        println!("  (Recommended default — verified T440p curve when managed)");
    }
}

fn do_set_profile(cap: sunlight_ipc::CapabilityToken, name: &str) -> i32 {
    let Some(p) = profile_from_str(name) else {
        println!("thermalctl: unknown profile '{}'", name);
        println!("valid: balanced, quiet, cool, performance");
        return 1;
    };
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(ThermaldMsg::SET_PROFILE).word(0, p as u64),
    );
    if reply.label != ThermaldMsg::REPLY {
        println!("thermalctl: set failed");
        return 1;
    }
    println!("cooling profile: {}", p.as_str());
    0
}

fn do_auto(cap: sunlight_ipc::CapabilityToken) -> i32 {
    let reply = ipc_call(cap, IpcMsg::with_label(ThermaldMsg::FORCE_FIRMWARE_AUTO));
    if reply.label != ThermaldMsg::REPLY {
        println!("thermalctl: auto failed");
        return 1;
    }
    println!("fan control: FirmwareAuto requested");
    0
}

fn do_reset(cap: sunlight_ipc::CapabilityToken) -> i32 {
    let reply = ipc_call(cap, IpcMsg::with_label(ThermaldMsg::RESET_SAFE_DEFAULTS));
    if reply.label != ThermaldMsg::REPLY {
        println!("thermalctl: reset failed");
        return 1;
    }
    println!("safe defaults restored");
    0
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
