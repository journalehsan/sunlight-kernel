//! networkctl — CLI for networkd v0.

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 64 * 1024] = [0; 64 * 1024];
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
    ipc_call, nameserver_lookup, pack_ipv4, pack_short_name, AdminState, DnsSource, InterfaceKind,
    IpConfigMode, IpcMsg, LinkState, NetworkdMsg, ResolvedMsg,
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
    println!("networkctl: PANIC");
    sunlight_ipc::ProcessExit::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let args = &storage[..count];

    let Some(cap) = nameserver_lookup("networkd") else {
        println!("networkctl: networkd not running");
        sunlight_ipc::ProcessExit::exit(1);
    };

    let code = match args.get(1).copied().unwrap_or("list") {
        "list" => {
            print_list(cap);
            0
        }
        "status" => {
            if args.len() >= 3 {
                print_status(cap, args[2]);
                0
            } else {
                print_status_all(cap);
                0
            }
        }
        "up" | "enable" => {
            if args.len() < 3 {
                println!("Usage: networkctl up <iface>");
                1
            } else {
                do_enable(cap, args[2], true)
            }
        }
        "down" | "disable" => {
            if args.len() < 3 {
                println!("Usage: networkctl down <iface>");
                1
            } else {
                do_enable(cap, args[2], false)
            }
        }
        "dhcp" => {
            if args.len() < 3 {
                println!("Usage: networkctl dhcp <iface>");
                1
            } else {
                do_dhcp(cap, args[2])
            }
        }
        "static" => {
            // networkctl static <iface> <a.b.c.d/prefix> [gw x.y.z.w] [dns a,b]
            if args.len() < 4 {
                println!("Usage: networkctl static <iface> <addr/prefix> [gw <gw>] [dns <d1,d2>]");
                1
            } else {
                do_static(cap, &args[2..])
            }
        }
        "priority" => {
            if args.len() < 4 {
                println!("Usage: networkctl priority <iface> <value>");
                1
            } else {
                do_priority(cap, args[2], args[3])
            }
        }
        "json" => {
            print_json(cap);
            0
        }
        "refresh" => {
            do_refresh(cap);
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
    println!("networkctl list|status [iface]|up <i>|down <i>|dhcp <i>|static <i> <ip/p> [gw g] [dns d1,d2]|priority <i> <n>|json|refresh");
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
        // Very small stack string; we transmute to &str for lifetime of _start
        let s = core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, len));
        out[n] = s;
        n += 1;
    }
    n
}

fn print_list(cap: sunlight_ipc::CapabilityToken) {
    println!("IFACE  KIND        ADMIN    LINK      MODE   ADDRESS         GATEWAY     PRIO  DEF");
    let mut idx = 0u64;
    loop {
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(NetworkdMsg::LIST_INTERFACES).word(0, idx),
        );
        let Some(s) = sunlight_ipc::unpack_iface_summary(&reply) else {
            break;
        };
        let kind_s = s.kind.label();
        let admin_s = if matches!(s.admin, AdminState::Enabled) {
            "enabled"
        } else {
            "disabled"
        };
        let link_s = match s.link {
            LinkState::Up | LinkState::Carrier => "carrier",
            LinkState::Down => "down",
            LinkState::NoCarrier => "nocarrier",
            _ => "unknown",
        };
        let mode_s = match s.mode {
            IpConfigMode::Dhcp => "dhcp",
            IpConfigMode::Static => "static",
            _ => "none",
        };
        let mut addr_buf = heapless::String::<32>::new();
        if s.addr == [0, 0, 0, 0] {
            let _ = addr_buf.push_str("-");
        } else {
            let _ = write!(
                &mut addr_buf,
                "{}.{}.{}.{}/{}",
                s.addr[0], s.addr[1], s.addr[2], s.addr[3], s.prefix
            );
        }
        let mut gw_buf = heapless::String::<32>::new();
        if s.gw == [0, 0, 0, 0] {
            let _ = gw_buf.push_str("-");
        } else {
            let _ = write!(
                &mut gw_buf,
                "{}.{}.{}.{}",
                s.gw[0], s.gw[1], s.gw[2], s.gw[3]
            );
        }
        let name = name_from_packed(s.name);
        let def = if s.is_default { "yes" } else { "no" };
        println!(
            "{:<6} {:<10} {:<8} {:<9} {:<6} {:<15} {:<11} {:>4}  {}",
            name, kind_s, admin_s, link_s, mode_s, addr_buf, gw_buf, s.priority, def
        );
        idx += 1;
        if idx as u16 >= s.total && s.total > 0 {
            break;
        }
    }
}

fn name_from_packed(p: u64) -> alloc::string::String {
    let mut b = [0u8; 8];
    for i in 0..8 {
        b[i] = ((p >> (i * 8)) & 0xff) as u8;
    }
    let len = b.iter().position(|&x| x == 0).unwrap_or(8);
    alloc::string::String::from_utf8_lossy(&b[..len]).into_owned()
}

fn print_status_all(cap: sunlight_ipc::CapabilityToken) {
    print_list(cap);
    let info = sunlight_ipc::net_device_info().unwrap_or_default();
    if !info.present {
        println!("Ethernet backend: unavailable");
        println!("VMXNET3 stage: {}", info.vmxnet3_stage.label());
        println!(
            "VMXNET3 error: {} (failed_stage={} detail={:#x})",
            sunlight_ipc::Vmxnet3ErrorCode::from_u64(info.error).label(),
            info.vmxnet3_failure_stage.label(),
            info.vmxnet3_error_detail
        );
    } else {
        println!(
            "backend: {} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} mtu={} carrier={}",
            info.backend
                .map(|backend| backend.driver_name())
                .unwrap_or("unknown"),
            info.mac[0],
            info.mac[1],
            info.mac[2],
            info.mac[3],
            info.mac[4],
            info.mac[5],
            info.mtu,
            if info.link_up { "yes" } else { "no" }
        );
        println!(
            "vmxnet3: resets={} activations={} irq={} polls={}",
            info.device_resets, info.device_activations, info.interrupts, info.polls
        );
        println!(
            "tx: requests={} submitted={} bytes={} full={} notify={} complete={} errors={}",
            info.tx_requests,
            info.tx_submitted,
            info.tx_bytes,
            info.tx_ring_full,
            info.tx_notifications,
            info.tx_completed,
            info.tx_errors
        );
        println!(
            "rx: posted={} complete={} delivered={} bytes={} dropped={} errors={}",
            info.rx_buffers_posted,
            info.rx_completed,
            info.rx_delivered,
            info.rx_bytes,
            info.rx_dropped,
            info.rx_errors
        );
        println!(
            "dhcp: discover={} offer={} request={} ack={} timeout={} installed={}",
            info.dhcp_discovers,
            info.dhcp_offers,
            info.dhcp_requests,
            info.dhcp_acks,
            info.dhcp_timeouts,
            info.dhcp_leases_installed
        );
    }
}

fn print_status(cap: sunlight_ipc::CapabilityToken, key: &str) {
    let key_u = pack_short_name(key);
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(NetworkdMsg::GET_INTERFACE).word(0, key_u),
    );
    if let Some(s) = sunlight_ipc::unpack_iface_summary(&reply) {
        let name = name_from_packed(s.name);
        println!("{}:", name);
        println!("  id:        {}", s.id);
        let kind_s = s.kind.label();
        println!("  kind:      {}", kind_s);
        // driver: best-effort for v0
        let driver_s = match s.kind {
            InterfaceKind::VirtioNet => "virtio",
            InterfaceKind::Vmxnet3 => "vmxnet3",
            InterfaceKind::Ethernet => "ethernet",
            _ => "-",
        };
        println!("  driver:    {}", driver_s);
        if s.kind != InterfaceKind::Loopback {
            let info = sunlight_ipc::net_device_info().unwrap_or_default();
            if info.present && info.backend.map(|backend| backend.interface_kind()) == Some(s.kind)
            {
                println!(
                    "  mac:       {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    info.mac[0], info.mac[1], info.mac[2], info.mac[3], info.mac[4], info.mac[5]
                );
                println!("  mtu:       {}", info.mtu);
            }
        }
        let admin_s = if matches!(s.admin, AdminState::Enabled) {
            "Enabled"
        } else {
            "Disabled"
        };
        println!("  admin:     {}", admin_s);
        let link_s = match s.link {
            LinkState::Up | LinkState::Carrier => "Carrier",
            LinkState::Down => "Down",
            LinkState::NoCarrier => "NoCarrier",
            _ => "Unknown",
        };
        println!("  link:      {}", link_s);
        let mode_s = match s.mode {
            IpConfigMode::Dhcp => "Dhcp",
            IpConfigMode::Static => "Static",
            _ => "None",
        };
        println!("  mode:      {}", mode_s);
        if s.addr != [0; 4] {
            println!(
                "  address:   {}.{}.{}.{}/{}",
                s.addr[0], s.addr[1], s.addr[2], s.addr[3], s.prefix
            );
        } else {
            println!("  address:   -");
        }
        if s.gw != [0; 4] {
            println!(
                "  gateway:   {}.{}.{}.{}",
                s.gw[0], s.gw[1], s.gw[2], s.gw[3]
            );
        } else {
            println!("  gateway:   -");
        }
        let dns = if s.dns != [0; 4] {
            s.dns
        } else {
            dns_from_resolved_for(&name).unwrap_or([0; 4])
        };
        if dns != [0; 4] {
            println!("  dns:       {}.{}.{}.{}", dns[0], dns[1], dns[2], dns[3]);
        } else {
            println!("  dns:       -");
        }
        println!("  priority:  {}", s.priority);
        println!("  default:   {}", if s.is_default { "yes" } else { "no" });
    } else {
        println!("networkctl: interface '{}' not found", key);
    }
}

fn dns_from_resolved_for(iface: &str) -> Option<[u8; 4]> {
    let cap = nameserver_lookup("resolved")?;
    let mut idx = 0u64;
    let mut fallback = None;
    loop {
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(ResolvedMsg::GET_SERVER).word(0, idx),
        );
        let Some(server) = sunlight_ipc::unpack_dns_server_summary(&reply) else {
            break;
        };
        if fallback.is_none() {
            fallback = Some(server.address);
        }
        let server_iface = if server.iface != 0 {
            name_from_packed(server.iface)
        } else {
            alloc::string::String::new()
        };
        if server_iface == iface
            || (server_iface.is_empty()
                && matches!(server.source, DnsSource::SystemDefault | DnsSource::Manual))
        {
            return Some(server.address);
        }
        idx += 1;
        if server.total > 0 && idx as u16 >= server.total {
            break;
        }
    }
    fallback
}

fn do_enable(cap: sunlight_ipc::CapabilityToken, key: &str, en: bool) -> i32 {
    let op = if en {
        NetworkdMsg::ENABLE_INTERFACE
    } else {
        NetworkdMsg::DISABLE_INTERFACE
    };
    let key_u = pack_short_name(key);
    let reply = ipc_call(cap, IpcMsg::with_label(op).word(0, key_u));
    if reply.label == NetworkdMsg::REPLY {
        println!("{} {}", if en { "enabled" } else { "disabled" }, key);
        0
    } else {
        print_networkd_error("change", Some(key), reply.words[0]);
        1
    }
}

fn do_dhcp(cap: sunlight_ipc::CapabilityToken, key: &str) -> i32 {
    let key_u = pack_short_name(key);
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(NetworkdMsg::SET_DHCP).word(0, key_u),
    );
    if reply.label == NetworkdMsg::REPLY {
        println!("{} configured for dhcp", key);
        0
    } else {
        println!("networkctl: dhcp {} failed (err={})", key, reply.words[0]);
        1
    }
}

fn do_static(cap: sunlight_ipc::CapabilityToken, args: &[&str]) -> i32 {
    // args[0]=iface, args[1]=addr/prefix , ...
    let iface = args[0];
    let addr_p = args[1];
    // parse addr/prefix
    let (addr_str, pfx_str) = match addr_p.split_once('/') {
        Some((a, p)) => (a, p),
        None => {
            println!("networkctl: address must be ip/prefix");
            return 1;
        }
    };
    let addr = match parse_ipv4(addr_str) {
        Some(a) => a,
        None => {
            println!("networkctl: bad address");
            return 1;
        }
    };
    let prefix: u8 = pfx_str.parse().unwrap_or(24);

    let mut gw = [0u8; 4];
    let mut dns_list: alloc::vec::Vec<[u8; 4]> = alloc::vec::Vec::new();
    let mut i = 2;
    while i + 1 < args.len() {
        match args[i] {
            "gw" => {
                gw = parse_ipv4(args[i + 1]).unwrap_or([0; 4]);
                i += 2;
            }
            "dns" => {
                // support "dns 1.1.1.1,8.8.8.8" or "dns 1.1.1.1"
                let val = args[i + 1];
                for part in val.split(',') {
                    if let Some(d) = parse_ipv4(part.trim()) {
                        dns_list.push(d);
                    }
                }
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }
    let dns0 = dns_list.get(0).copied().unwrap_or([0; 4]);
    let dns1 = dns_list.get(1).copied().unwrap_or([0; 4]);

    let key_u = pack_short_name(iface);
    let msg = IpcMsg::with_label(NetworkdMsg::SET_STATIC_IPV4)
        .word(0, key_u)
        .word(1, pack_ipv4(addr))
        .word(2, pack_ipv4(gw))
        .word(3, prefix as u64)
        .word(4, pack_ipv4(dns0))
        .word(5, pack_ipv4(dns1));
    let reply = ipc_call(cap, msg);
    if reply.label == NetworkdMsg::REPLY {
        let gw_disp = if gw != [0; 4] {
            alloc::format!("{}.{}.{}.{}", gw[0], gw[1], gw[2], gw[3])
        } else {
            "-".into()
        };
        let dns_disp = if dns0 != [0; 4] {
            if dns1 != [0; 4] {
                alloc::format!(
                    "{}.{}.{}.{}, {}.{}.{}.{}",
                    dns0[0],
                    dns0[1],
                    dns0[2],
                    dns0[3],
                    dns1[0],
                    dns1[1],
                    dns1[2],
                    dns1[3]
                )
            } else {
                alloc::format!("{}.{}.{}.{}", dns0[0], dns0[1], dns0[2], dns0[3])
            }
        } else {
            "-".into()
        };
        println!(
            "{} static {}/{} gw {} dns {}",
            iface, addr_str, prefix, gw_disp, dns_disp
        );
        println!("(stored in networkd; live stack reconfigure is v0.2 - new connections will see updated GETIP)");
        0
    } else {
        println!("networkctl: static failed");
        1
    }
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

fn do_priority(cap: sunlight_ipc::CapabilityToken, key: &str, val: &str) -> i32 {
    let prio: i32 = match val.parse() {
        Ok(v) => v,
        _ => {
            println!("networkctl: bad priority");
            return 1;
        }
    };
    let key_u = pack_short_name(key);
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(NetworkdMsg::SET_PRIORITY)
            .word(0, key_u)
            .word(1, prio as u64),
    );
    if reply.label == NetworkdMsg::REPLY {
        println!("{} priority -> {}", key, prio);
        0
    } else {
        println!("networkctl: priority failed");
        1
    }
}

fn print_json(cap: sunlight_ipc::CapabilityToken) {
    println!("[");
    let mut idx = 0u64;
    let mut first = true;
    loop {
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(NetworkdMsg::LIST_INTERFACES).word(0, idx),
        );
        let Some(s) = sunlight_ipc::unpack_iface_summary(&reply) else {
            break;
        };
        if !first {
            println!(",");
        }
        first = false;
        let name = name_from_packed(s.name);
        let kind_s = match s.kind {
            InterfaceKind::Loopback => "Loopback",
            InterfaceKind::Ethernet => "Ethernet",
            InterfaceKind::VirtioNet => "VirtioNet",
            InterfaceKind::Vmxnet3 => "VMXNET3",
            InterfaceKind::Wireless => "Wireless",
            InterfaceKind::Tunnel => "Tunnel",
            _ => "Unknown",
        };
        let admin_s = if matches!(s.admin, AdminState::Enabled) {
            "enabled"
        } else {
            "disabled"
        };
        let link_s = match s.link {
            LinkState::Up | LinkState::Carrier => "carrier",
            LinkState::Down => "down",
            LinkState::NoCarrier => "nocarrier",
            _ => "unknown",
        };
        let mode_s = match s.mode {
            IpConfigMode::Dhcp => "dhcp",
            IpConfigMode::Static => "static",
            _ => "none",
        };
        let addr_str = if s.addr != [0, 0, 0, 0] {
            alloc::format!(
                "{}.{}.{}.{}/{}",
                s.addr[0],
                s.addr[1],
                s.addr[2],
                s.addr[3],
                s.prefix
            )
        } else {
            "-".into()
        };
        let gw_str = if s.gw != [0, 0, 0, 0] {
            alloc::format!("{}.{}.{}.{}", s.gw[0], s.gw[1], s.gw[2], s.gw[3])
        } else {
            "-".into()
        };
        let driver_s = match s.kind {
            InterfaceKind::VirtioNet => "virtio",
            InterfaceKind::Vmxnet3 => "vmxnet3",
            _ => "-",
        };
        // dns_servers as simple array of the primary (v0); "-" if none
        let dns_str = if s.dns != [0, 0, 0, 0] {
            alloc::format!("[\"{}.{}.{}.{}\"]", s.dns[0], s.dns[1], s.dns[2], s.dns[3])
        } else {
            "[]".into()
        };
        println!("  {{ \"id\": {}, \"name\": \"{}\", \"kind\": \"{}\", \"admin_state\": \"{}\", \"link_state\": \"{}\", \"mode\": \"{}\", \"address\": \"{}\", \"prefix_len\": {}, \"gateway\": \"{}\", \"dns_servers\": {}, \"priority\": {}, \"default\": {}, \"driver\": \"{}\" }}",
            s.id, name,
            kind_s, admin_s, link_s, mode_s,
            addr_str, s.prefix,
            gw_str,
            dns_str,
            s.priority,
            if s.is_default { "true" } else { "false" },
            driver_s
        );
        idx += 1;
        if s.total > 0 && idx as u16 >= s.total {
            break;
        }
    }
    println!("\n]");
}

fn do_refresh(cap: sunlight_ipc::CapabilityToken) {
    let reply = ipc_call(cap, IpcMsg::with_label(NetworkdMsg::REFRESH));
    if reply.label == NetworkdMsg::REPLY {
        println!("refreshed ({} interface(s))", reply.words[1]);
    } else {
        print_networkd_error("refresh", None, reply.words[0]);
    }
}

fn print_networkd_error(action: &str, iface: Option<&str>, code: u64) {
    let detail = match code {
        NetworkdMsg::ERR_NOT_FOUND => "interface not found",
        NetworkdMsg::ERR_BAD_REQUEST => "invalid request",
        NetworkdMsg::ERR_NO_CARRIER => "link unavailable",
        NetworkdMsg::ERR_DEVICED_UNAVAILABLE => "device registry unavailable",
        NetworkdMsg::ERR_BACKEND_UNAVAILABLE => "network backend unavailable",
        NetworkdMsg::ERR_DEVICE_INITIALIZATION_FAILED => "device initialization failed",
        NetworkdMsg::ERR_INVALID_STATE => "invalid interface state",
        NetworkdMsg::ERR_PERMISSION_DENIED => "permission denied",
        NetworkdMsg::ERR_UNSUPPORTED_OPERATION => "unsupported operation",
        NetworkdMsg::ERR_IPC_FAILURE => "IPC failure",
        _ => "unknown error",
    };
    if code == NetworkdMsg::ERR_NOT_FOUND {
        if let Some(name) = iface {
            println!("networkctl: interface '{}' not found", name);
            return;
        }
    }
    if let Some(name) = iface {
        println!("networkctl: failed to {} '{}': {}", action, name, detail);
    } else {
        println!("networkctl: failed to {}: {}", action, detail);
    }
}
