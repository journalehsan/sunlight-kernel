//! powerctl — CLI for powerd v0.

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

use sunlight_ipc::{
    ipc_call, nameserver_lookup, CacheMode, EffectsMode, IpcMsg, PowerProfile, PowerdMsg, PrefetchMode,
    SchedulerBias,
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
    println!("powerctl: PANIC");
    sunlight_ipc::ProcessExit::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let args = &storage[..count];

    let Some(cap) = nameserver_lookup("powerd") else {
        println!("powerctl: powerd not running");
        sunlight_ipc::ProcessExit::exit(1);
    };

    let sub = args.get(1).copied().unwrap_or("status");
    let code = match sub {
        "status" => {
            print_status(cap);
            0
        }
        "profiles" | "list" => {
            print_profiles(cap);
            0
        }
        "set" => {
            if args.len() < 3 {
                println!("Usage: powerctl set <turbo|performance|balanced|low-power|stamina|custom|auto>");
                1
            } else if args[2].eq_ignore_ascii_case("auto") {
                println!("powerctl: to set Auto mode, run 'powerctl auto' (this also works)");
                do_auto(cap)
            } else {
                do_set(cap, args[2])
            }
        }
        "auto" => {
            do_auto(cap)
        }
        "policy" => {
            print_policy(cap);
            0
        }
        _ => {
            print_usage();
            1
        }
    };

    sunlight_ipc::ProcessExit::exit(code);
}

fn print_usage() {
    println!("Usage: powerctl <status|profiles|set|auto|policy> [profile]");
    println!("  powerctl status");
    println!("  powerctl profiles");
    println!("  powerctl set <turbo|performance|balanced|low-power|stamina|custom|auto>");
    println!("  powerctl auto");
    println!("  powerctl policy");
}

fn profile_from_str(s: &str) -> Option<PowerProfile> {
    match s {
        "turbo" | "Turbo" => Some(PowerProfile::Turbo),
        "performance" | "Performance" => Some(PowerProfile::Performance),
        "balanced" | "Balanced" => Some(PowerProfile::Balanced),
        "low-power" | "lowpower" | "LowPower" => Some(PowerProfile::LowPower),
        "stamina" | "Stamina" => Some(PowerProfile::Stamina),
        "custom" | "Custom" => Some(PowerProfile::Custom),
        _ => None,
    }
}

fn profile_str(p: PowerProfile) -> &'static str {
    p.as_str()
}

fn print_status(cap: sunlight_ipc::CapabilityToken) {
    let reply = ipc_call(cap, IpcMsg::with_label(PowerdMsg::GET_STATUS));
    if reply.label != PowerdMsg::REPLY {
        println!("powerctl: failed to get status");
        return;
    }
    let sel = PowerProfile::from_u64(reply.words[0]);
    let eff = PowerProfile::from_u64(reply.words[1]);
    // Minimal context decode for display
    let w2 = reply.words[2];
    let on_ac = if (w2 & 1) != 0 {
        Some((w2 & 2) != 0)
    } else {
        None
    };
    let battery = if (w2 & (1 << 2)) != 0 {
        Some(((w2 >> 3) & 0xff) as u8)
    } else {
        None
    };

    println!("Power:");
    println!("  selected:  {}", profile_str(sel));
    println!("  effective: {}", profile_str(eff));

    let ac_str = match on_ac {
        Some(true) => "on",
        Some(false) => "battery",
        None => "unknown",
    };
    println!("  AC:        {}", ac_str);

    if let Some(bp) = battery {
        println!("  Battery:   {}%", bp);
    } else {
        println!("  Battery:   unknown");
    }
}

fn print_profiles(cap: sunlight_ipc::CapabilityToken) {
    println!("Profiles:");
    let mut i = 0u64;
    loop {
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(PowerdMsg::LIST_PROFILES).word(0, i),
        );
        if reply.label != PowerdMsg::REPLY {
            break;
        }
        let tag = reply.words[0];
        let p = PowerProfile::from_u64(tag);
        println!("  {}", profile_str(p));
        i += 1;
        if i >= reply.words[1] {
            break;
        }
    }
}

fn do_set(cap: sunlight_ipc::CapabilityToken, name: &str) -> i32 {
    let p = match profile_from_str(name) {
        Some(p) => p,
        None => {
            println!("powerctl: unknown profile '{}'", name);
            println!("valid: turbo, performance, balanced, low-power, stamina, custom");
            println!("(for Auto mode: 'powerctl auto' or 'powerctl set auto')");
            return 1;
        }
    };
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(PowerdMsg::SET_PROFILE).word(0, p as u64),
    );
    if reply.label != PowerdMsg::REPLY {
        println!("powerctl: set failed");
        return 1;
    }
    // Print resulting status summary
    let sel = PowerProfile::from_u64(reply.words[0]);
    let eff = PowerProfile::from_u64(reply.words[1]);
    println!("selected: {}", profile_str(sel));
    if sel != eff {
        println!("effective: {}", profile_str(eff));
    }
    0
}

fn do_auto(cap: sunlight_ipc::CapabilityToken) -> i32 {
    let reply = ipc_call(cap, IpcMsg::with_label(PowerdMsg::SET_AUTO));
    if reply.label != PowerdMsg::REPLY {
        println!("powerctl: auto failed");
        return 1;
    }
    let sel = PowerProfile::from_u64(reply.words[0]);
    let eff = PowerProfile::from_u64(reply.words[1]);
    println!("selected: {}", profile_str(sel));
    println!("effective: {}", profile_str(eff));
    0
}

fn print_policy(cap: sunlight_ipc::CapabilityToken) {
    let reply = ipc_call(cap, IpcMsg::with_label(PowerdMsg::GET_POLICY));
    if reply.label != PowerdMsg::REPLY {
        println!("powerctl: policy unavailable");
        return;
    }
    let w0 = reply.words[0];
    let w1 = reply.words[1];
    let sel = PowerProfile::from_u64(w0 & 0xff);
    let eff = PowerProfile::from_u64((w0 >> 8) & 0xff);

    let cache = match CacheMode::from_u64(w1 & 0xff) {
        CacheMode::Minimal => "Minimal",
        CacheMode::Normal => "Normal",
        CacheMode::Aggressive => "Aggressive",
    };
    let prefetch = match PrefetchMode::from_u64((w1 >> 8) & 0xff) {
        PrefetchMode::Off => "Off",
        PrefetchMode::Light => "Light",
        PrefetchMode::Normal => "Normal",
        PrefetchMode::Aggressive => "Aggressive",
    };
    let effects = match EffectsMode::from_u64((w1 >> 16) & 0xff) {
        EffectsMode::Minimal => "Minimal",
        EffectsMode::Normal => "Normal",
        EffectsMode::Rich => "Rich",
    };
    let sched = match SchedulerBias::from_u64((w1 >> 24) & 0xff) {
        SchedulerBias::Battery => "Battery",
        SchedulerBias::Balanced => "Balanced",
        SchedulerBias::Interactive => "Interactive",
        SchedulerBias::Performance => "Performance",
    };
    let bg = if (w1 >> 32) & 1 != 0 { "allowed" } else { "limited" };

    println!("Power:");
    println!("  selected:  {}", profile_str(sel));
    println!("  effective: {}", profile_str(eff));
    println!("Policy:");
    println!("  cache:       {}", cache);
    println!("  prefetch:    {}", prefetch);
    println!("  effects:     {}", effects);
    println!("  scheduler:   {}", sched);
    println!("  background:  {}", bg);
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
