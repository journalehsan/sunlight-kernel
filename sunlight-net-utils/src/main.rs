//! SunlightOS network utilities — busybox-style multi-call binary.
//!
//! `argv[0]` selects the applet (PATH entries under /sunlight-net-utils all
//! exec this binary). no_std on top of sunlight-libc. Applets that need the
//! net_server IPC backend report so and exit non-zero until Phase 6.5 Step 4
//! wires spawned processes to the network stack; hostname/dnsdomainname work
//! today through the kernel VFS.

#![no_std]
#![no_main]

use sunlight_libc as libc;
use libc::{Errno, STDOUT};
use sunlight_ipc::{CapabilityToken, IpcMsg, ipc_call, nameserver_lookup};

const MAX_ARGS: usize = 16;
const NET_LABEL_GETIP: u64 = 10;
const NET_LABEL_PING: u64 = 11;
const NET_LABEL_RESOLVE: u64 = 9;  // DNS lookup(hostname) -> packed ip in word(0) or 0 on failure

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = write_all(b"sunlight-net-utils: panic\n");
    libc::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let code = run(&storage[..count]);
    libc::exit(code as u64);
}

/// SAFETY: argc/argv come from the kernel's SysV stack marshalling.
unsafe fn collect_args<'a>(argc: u64, argv: *const *const u8, out: &mut [&'a str]) -> usize {
    if argv.is_null() {
        return 0;
    }
    let mut count = 0;
    for i in 0..(argc as usize).min(out.len()) {
        let ptr = *argv.add(i);
        if ptr.is_null() {
            break;
        }
        let mut len = 0;
        while len < 256 && *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(ptr, len);
        out[count] = core::str::from_utf8(slice).unwrap_or("");
        count += 1;
    }
    count
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn run(args: &[&str]) -> i32 {
    let (applet, rest) = match args.split_first() {
        Some((first, rest)) => {
            let name = basename(first);
            if name == "sunlight-net-utils" {
                match rest.split_first() {
                    Some((sub, subrest)) => (*sub, subrest),
                    None => {
                        let _ = write_all(b"usage: sunlight-net-utils <applet> [args...]\n");
                        return 2;
                    }
                }
            } else {
                (name, rest)
            }
        }
        None => return 2,
    };

    match applet {
        "hostname" => cmd_hostname(),
        "dnsdomainname" => {
            let _ = write_all(b"local\n");
            0
        }
        "ping" => cmd_ping(rest),
        "ifconfig" => cmd_ifconfig(),
        "wget" | "curl" | "dig" | "nslookup" | "netstat" | "ss" | "traceroute" | "arp"
        | "dhclient" => {
            print2(applet, ": not implemented yet\n");
            1
        }
        _ => {
            print2(applet, ": applet not found\n");
            127
        }
    }
}

/// Print /etc/hostname (the file already ends in a newline).
fn cmd_hostname() -> i32 {
    let fd = match libc::open(b"/etc/hostname") {
        Ok(fd) => fd,
        Err(_) => {
            let _ = write_all(b"hostname: cannot read /etc/hostname\n");
            return 1;
        }
    };
    let mut buf = [0u8; 128];
    loop {
        match libc::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let _ = write_all(&buf[..n]);
            }
            Err(Errno::Again) => libc::yield_now(),
            Err(_) => break,
        }
    }
    let _ = libc::close(fd);
    0
}

fn cmd_ifconfig() -> i32 {
    let Some(net_cap) = nameserver_lookup("net") else {
        let _ = write_all(b"ifconfig: net service unavailable\n");
        return 1;
    };
    let reply = ipc_call(net_cap, IpcMsg::with_label(NET_LABEL_GETIP));
    if reply.label != NET_LABEL_GETIP {
        let _ = write_all(b"ifconfig: getip failed\n");
        return 1;
    }

    let ip = unpack_ipv4(reply.words[0]);
    let prefix = (reply.words[1] as u8).min(32);
    let gw = unpack_ipv4(reply.words[2]);
    let dns = unpack_ipv4(reply.words[3]);

    let _ = write_all(b"eth0: UP\n  inet ");
    print_ipv4(ip);
    let _ = write_all(b"/");
    print_u64(prefix as u64);
    let _ = write_all(b"\n  gateway ");
    print_ipv4(gw);
    let _ = write_all(b"\n  dns ");
    print_ipv4(dns);
    let _ = write_all(b"\n");
    0
}

fn cmd_ping(args: &[&str]) -> i32 {
    let target = args.first().copied().unwrap_or("8.8.8.8");

    let Some(net_cap) = nameserver_lookup("net") else {
        let _ = write_all(b"ping: net service unavailable\n");
        return 1;
    };

    let ping_ip: [u8; 4];
    if let Some(ip) = parse_ipv4(target) {
        // Dotted-quad IP path — unchanged behavior
        ping_ip = ip;
        let _ = write_all(b"PING ");
        let _ = write_all(target.as_bytes());
        let _ = write_all(b"\n");
    } else if is_valid_hostname(target) {
        // Hostname path: resolve via NetOp::RESOLVE, then ping the resulting IP.
        // Print the special "PING name (ip) ..." header; "from" lines will use the IP.
        match resolve_via_net(net_cap, target) {
            Some(ip) => {
                ping_ip = ip;
                let _ = write_all(b"PING ");
                let _ = write_all(target.as_bytes());
                let _ = write_all(b" (");
                print_ipv4(ping_ip);
                let _ = write_all(b") 56 bytes of data.\n");
            }
            None => {
                let _ = write_all(b"ping: ");
                let _ = write_all(target.as_bytes());
                let _ = write_all(b": Name or service not known\n");
                return 1;
            }
        }
    } else {
        let _ = write_all(b"ping: only dotted-quad IPv4 is supported\n");
        return 2;
    }

    // Common ping execution path (uses the (possibly resolved) numeric IP for the actual NetOp ping).
    let req_count = 4u64;
    let reply = ipc_call(
        net_cap,
        IpcMsg::with_label(NET_LABEL_PING)
            .word(0, pack_ipv4(ping_ip))
            .word(1, req_count),
    );
    if reply.label != NET_LABEL_PING || reply.words[0] == 0 {
        let _ = write_all(b"ping: request failed\n");
        return 1;
    }

    let received = reply.words[1].min(req_count);
    let base_rtt = reply.words[2];
    for seq in 0..received {
        let _ = write_all(b"64 bytes from ");
        print_ipv4(ping_ip);  // always the dotted IP for the "from" lines (even when original target was hostname)
        let _ = write_all(b": icmp_seq=");
        print_u64(seq);
        let _ = write_all(b" time=");
        print_u64(base_rtt + seq);
        let _ = write_all(b"ms\n");
    }
    let _ = write_all(b"--- ");
    let _ = write_all(target.as_bytes());  // original target (name or IP) for the statistics header
    let _ = write_all(b" ping statistics ---\n");
    print_u64(req_count);
    let _ = write_all(b" packets transmitted, ");
    print_u64(received);
    let _ = write_all(b" received\n");
    if received > 0 { 0 } else { 1 }
}

fn write_all(mut data: &[u8]) -> Result<(), Errno> {
    while !data.is_empty() {
        match libc::write(STDOUT, data) {
            Ok(n) => data = &data[n.min(data.len())..],
            Err(Errno::Again) => libc::yield_now(),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn print2(a: &str, b: &str) {
    let _ = write_all(a.as_bytes());
    let _ = write_all(b.as_bytes());
}

fn print_u64(mut v: u64) {
    let mut digits = [0u8; 20];
    let mut n = 0usize;
    loop {
        digits[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    while n > 0 {
        n -= 1;
        let _ = write_all(&digits[n..n + 1]);
    }
}

fn parse_u8_dec(s: &str) -> Option<u8> {
    if s.is_empty() {
        return None;
    }
    let mut out = 0u16;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((b - b'0') as u16)?;
        if out > 255 {
            return None;
        }
    }
    Some(out as u8)
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut count = 0usize;
    for part in s.split('.') {
        if count >= 4 {
            return None;
        }
        out[count] = parse_u8_dec(part)?;
        count += 1;
    }
    if count == 4 {
        Some(out)
    } else {
        None
    }
}

/// Returns true for things that look like hostnames (have letters or '-', and otherwise valid chars).
/// Must be called only after parse_ipv4 returned None.
fn is_valid_hostname(s: &str) -> bool {
    if s.is_empty() || s.len() > 253 {
        return false;
    }
    let has_letter_or_hyphen = s.chars().any(|c| c.is_ascii_alphabetic() || c == '-');
    if !has_letter_or_hyphen {
        return false;
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// Call net_server over IPC (NetOp::RESOLVE) with hostname packed into the message.
/// Returns Some(ip) on success, None on "Name or service not known".
/// Hostname is sent as: words[0]=len, then 8-byte little-endian chunks in words[1..].
fn resolve_via_net(net_cap: CapabilityToken, hostname: &str) -> Option<[u8; 4]> {
    let bytes = hostname.as_bytes();
    let name_len = bytes.len().min(48);
    let mut msg = IpcMsg::with_label(NET_LABEL_RESOLVE).word(0, name_len as u64);
    let mut w_idx = 1usize;
    let mut b_idx = 0usize;
    while b_idx < name_len && w_idx < 8 {
        let mut w = 0u64;
        for j in 0..8 {
            if b_idx >= name_len { break; }
            w |= (bytes[b_idx] as u64) << (j * 8);
            b_idx += 1;
        }
        msg = msg.word(w_idx, w);
        w_idx += 1;
    }
    let reply = ipc_call(net_cap, msg);
    if reply.label != NET_LABEL_RESOLVE || reply.word_count == 0 || reply.words[0] == 0 {
        return None;
    }
    Some(unpack_ipv4(reply.words[0]))
}

fn pack_ipv4(ip: [u8; 4]) -> u64 {
    (ip[0] as u64)
        | ((ip[1] as u64) << 8)
        | ((ip[2] as u64) << 16)
        | ((ip[3] as u64) << 24)
}

fn unpack_ipv4(v: u64) -> [u8; 4] {
    [
        (v & 0xff) as u8,
        ((v >> 8) & 0xff) as u8,
        ((v >> 16) & 0xff) as u8,
        ((v >> 24) & 0xff) as u8,
    ]
}

fn print_ipv4(ip: [u8; 4]) {
    print_u64(ip[0] as u64);
    let _ = write_all(b".");
    print_u64(ip[1] as u64);
    let _ = write_all(b".");
    print_u64(ip[2] as u64);
    let _ = write_all(b".");
    print_u64(ip[3] as u64);
}
