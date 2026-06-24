//! resolvectl — CLI for resolved v0 (DNS resolver configuration).

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
    ipc_call, nameserver_lookup, pack_ipv4, unpack_ipv4, DnsSource, IpcMsg, ResolvedMsg,
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
    println!("resolvectl: PANIC");
    sunlight_ipc::ProcessExit::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let args = &storage[..count];

    let Some(cap) = nameserver_lookup("resolved") else {
        println!("resolvectl: resolved not running");
        sunlight_ipc::ProcessExit::exit(1);
    };

    let code = match args.get(1).copied().unwrap_or("status") {
        "status" => {
            print_status(cap);
            0
        }
        "servers" => {
            print_servers(cap);
            0
        }
        "set" => {
            if args.len() < 3 {
                println!("Usage: resolvectl set <ip> [ip ...]");
                1
            } else {
                do_set(cap, &args[2..])
            }
        }
        "default" => do_default(cap),
        "render" => {
            print_render(cap);
            0
        }
        "query" => {
            if args.len() < 3 {
                println!("Usage: resolvectl query <name>");
                1
            } else {
                do_query(cap, args[2])
            }
        }
        "flush-cache" | "flush_cache" => {
            let _ = ipc_call(cap, IpcMsg::with_label(ResolvedMsg::FLUSH_CACHE));
            println!("cache flushed (v0 stub)");
            0
        }
        "json" => {
            print_json(cap);
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
    println!("resolvectl status|servers|set <ip...>|default|render|query <name>|flush-cache|json");
}

unsafe fn collect_args(argc: u64, argv: *const *const u8, out: &mut [&str]) -> usize {
    let mut n = 0usize;
    let c = argc as usize;
    for i in 0..c.min(out.len()) {
        let p = *argv.add(i);
        if p.is_null() {
            break;
        }
        let mut len = 0;
        while *p.add(len) != 0 {
            len += 1;
        }
        let s = core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, len));
        out[n] = s;
        n += 1;
    }
    n
}

fn source_str(s: DnsSource, iface: &str) -> alloc::string::String {
    match s {
        DnsSource::Dhcp => {
            if iface.is_empty() {
                "Dhcp".into()
            } else {
                alloc::format!("Dhcp {}", iface)
            }
        }
        DnsSource::Static => {
            if iface.is_empty() {
                "Static".into()
            } else {
                alloc::format!("Static {}", iface)
            }
        }
        DnsSource::SystemDefault => "SystemDefault".into(),
        DnsSource::Vpn => "Vpn".into(),
        DnsSource::Manual => "Manual".into(),
        _ => "Unknown".into(),
    }
}

fn print_status(cap: sunlight_ipc::CapabilityToken) {
    println!("DNS Servers:");
    print_servers_inner(cap);
    println!("");
    println!("resolv.conf:");
    println!("  /etc/resolv.conf generated");
}

fn print_servers(cap: sunlight_ipc::CapabilityToken) {
    print_servers_inner(cap);
}

fn print_servers_inner(cap: sunlight_ipc::CapabilityToken) {
    let mut idx = 0u64;
    loop {
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(ResolvedMsg::GET_SERVER).word(0, idx),
        );
        let Some(s) = sunlight_ipc::unpack_dns_server_summary(&reply) else {
            break;
        };
        let name = if s.iface != 0 {
            name_from_packed(s.iface)
        } else {
            alloc::string::String::new()
        };
        let src = source_str(s.source, &name);
        println!(
            "  {}.{}.{}.{}   {}",
            s.address[0], s.address[1], s.address[2], s.address[3], src
        );
        idx += 1;
        if idx as u16 >= s.total && s.total > 0 {
            break;
        }
    }
    if idx == 0 {
        println!("  (none)");
    }
}

fn do_set(cap: sunlight_ipc::CapabilityToken, ips: &[&str]) -> i32 {
    let mut msg = IpcMsg::with_label(ResolvedMsg::SET_SERVERS);
    let mut wi = 0usize;
    for ip in ips {
        if let Some(a) = parse_ipv4(ip.trim()) {
            msg = msg.word(wi, pack_ipv4(a));
            wi += 1;
            if wi >= 4 {
                break;
            }
        }
    }
    let reply = ipc_call(cap, msg);
    if reply.label == ResolvedMsg::REPLY {
        println!("DNS servers set (manual)");
        0
    } else {
        println!("resolvectl: set failed");
        1
    }
}

fn do_default(cap: sunlight_ipc::CapabilityToken) -> i32 {
    let reply = ipc_call(cap, IpcMsg::with_label(ResolvedMsg::USE_SYSTEM_DEFAULT));
    if reply.label == ResolvedMsg::REPLY {
        println!("using system default DNS");
        0
    } else {
        println!("resolvectl: default failed");
        1
    }
}

fn print_render(cap: sunlight_ipc::CapabilityToken) {
    // Ask for packed servers and format like /etc/resolv.conf
    println!("# Generated by SunlightOS resolved.");
    let mut idx = 0u64;
    let mut any = false;
    loop {
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(ResolvedMsg::GET_SERVER).word(0, idx),
        );
        let Some(s) = sunlight_ipc::unpack_dns_server_summary(&reply) else {
            break;
        };
        println!(
            "nameserver {}.{}.{}.{}",
            s.address[0], s.address[1], s.address[2], s.address[3]
        );
        any = true;
        idx += 1;
        if idx as u16 >= s.total && s.total > 0 {
            break;
        }
    }
    if !any {
        // fallback text
        println!("nameserver 208.67.222.222");
        println!("nameserver 208.67.220.220");
    }
}

fn do_query(cap: sunlight_ipc::CapabilityToken, name: &str) -> i32 {
    // v0: resolved does not own full resolution. Ask politely; net path wins.
    let reply = ipc_call(cap, IpcMsg::with_label(ResolvedMsg::RESOLVE_NAME));
    if reply.label == ResolvedMsg::REPLY && reply.words[0] != 0 {
        let ip = unpack_ipv4(reply.words[0]);
        println!("{} -> {}.{}.{}.{}", name, ip[0], ip[1], ip[2], ip[3]);
        0
    } else {
        println!(
            "{}: resolution delegated to net (use host/ping/fetch)",
            name
        );
        0
    }
}

fn print_json(cap: sunlight_ipc::CapabilityToken) {
    println!("[");
    let mut idx = 0u64;
    let mut first = true;
    loop {
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(ResolvedMsg::GET_SERVER).word(0, idx),
        );
        let Some(s) = sunlight_ipc::unpack_dns_server_summary(&reply) else {
            break;
        };
        if !first {
            println!(",");
        }
        first = false;
        let name = if s.iface != 0 {
            name_from_packed(s.iface)
        } else {
            "".into()
        };
        let src_s = match s.source {
            DnsSource::Dhcp => "Dhcp",
            DnsSource::Static => "Static",
            DnsSource::SystemDefault => "SystemDefault",
            DnsSource::Vpn => "Vpn",
            DnsSource::Manual => "Manual",
            _ => "Unknown",
        };
        println!("  {{ \"address\": \"{}.{}.{}.{}\", \"source\": \"{}\", \"iface\": \"{}\", \"priority\": {} }}",
            s.address[0], s.address[1], s.address[2], s.address[3],
            src_s, name, s.priority);
        idx += 1;
        if s.total > 0 && idx as u16 >= s.total {
            break;
        }
    }
    println!("\n]");
}

fn name_from_packed(p: u64) -> alloc::string::String {
    let mut b = [0u8; 8];
    for i in 0..8 {
        b[i] = ((p >> (i * 8)) & 0xff) as u8;
    }
    let len = b.iter().position(|&x| x == 0).unwrap_or(8);
    alloc::string::String::from_utf8_lossy(&b[..len]).into_owned()
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for i in 0..4 {
        let p = parts.next()?;
        out[i] = p.parse::<u8>().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}
