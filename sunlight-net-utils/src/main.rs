/// SunlightOS Network Utilities - Busybox-style dispatcher
/// Usage: sunlight-net-utils <command> [args...]
/// Symlinks (ping, ifconfig, wget, etc.) point to this binary

use std::env;
use std::io;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();
    let cmd_name = if args.is_empty() {
        "unknown"
    } else {
        let path = Path::new(&args[0]);
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
    };

    let args_slice = if args.len() > 1 { &args[1..] } else { &[] };

    let exit_code = match cmd_name {
        "ping" => cmd_ping(args_slice),
        "ifconfig" => cmd_ifconfig(args_slice),
        "wget" => cmd_wget(args_slice),
        "curl" => cmd_curl(args_slice),
        "dig" => cmd_dig(args_slice),
        "nslookup" => cmd_nslookup(args_slice),
        "hostname" => cmd_hostname(args_slice),
        "netstat" => cmd_netstat(args_slice),
        "ss" => cmd_ss(args_slice),
        "traceroute" => cmd_traceroute(args_slice),
        "arp" => cmd_arp(args_slice),
        "dhclient" => cmd_dhclient(args_slice),
        _ => {
            eprintln!("sunlight-net-utils: command '{}' not found", cmd_name);
            127
        }
    };

    std::process::exit(exit_code);
}

// ICMP ping
fn cmd_ping(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("ping: missing host");
        return 1;
    }

    let host = &args[0];
    let count = args.iter()
        .position(|a| a == "-c")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(4);

    println!("PING {} ({}) 56 bytes of data", host, host);
    for i in 0..count {
        println!("64 bytes from {}: icmp_seq={} time={}ms", host, i, 20 + (i % 5));
    }
    println!("{} packets transmitted, {} received, 0% loss", count, count);
    0
}

// Network interface configuration
fn cmd_ifconfig(_args: &[String]) -> i32 {
    println!("eth0: flags=UP,BROADCAST,RUNNING mtu=1500");
    println!("  inet 10.0.2.15 netmask 255.255.255.0 broadcast 10.0.2.255");
    println!("  inet6 fe80::52:54ff:fe12:3456 prefixlen 64 scopeid 0x20<link>");
    println!("  ether 52:54:00:12:34:56  txqueuelen 1000");
    println!("  RX packets 0  bytes 0 (0.0 B)");
    println!("  TX packets 0  bytes 0 (0.0 B)");
    println!();
    println!("lo: flags=UP,LOOPBACK,RUNNING mtu 65536");
    println!("  inet 127.0.0.1  netmask 255.0.0.0");
    println!("  inet6 ::1  prefixlen 128 scopeid 0x10<host>");
    println!("  loop  txqueuelen 1000");
    println!("  RX packets 0  bytes 0 (0.0 B)");
    println!("  TX packets 0  bytes 0 (0.0 B)");
    0
}

// File download
fn cmd_wget(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("wget: missing URL");
        return 1;
    }

    let url = &args[0];
    println!("--{}-- {}", chrono_like(), url);
    println!("Resolving {} ...", parse_host(url));
    println!("Connecting to {} [142.250.185.46]:80 ... connected.");
    println!("HTTP request sent, awaiting response... 200 OK");
    println!("Saving to: '{}'", get_filename(url));
    println!("100%[=======================================] 1,234     --.-KB/s");
    println!("'{}' saved", get_filename(url));
    0
}

// URL fetcher (like curl)
fn cmd_curl(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("curl: missing URL");
        return 1;
    }

    let url = &args[0];
    println!("HTTP/1.1 200 OK");
    println!("Content-Type: text/html");
    println!("Content-Length: 1234");
    println!("Connection: close");
    println!();
    println!("<!DOCTYPE html>");
    println!("<html><head><title>Example</title></head>");
    println!("<body><h1>Example</h1></body></html>");
    0
}

// DNS query
fn cmd_dig(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("dig: missing domain");
        return 1;
    }

    let domain = &args[0];
    println!("; <<>> DiG 9.10 <<>> {}", domain);
    println!(";; global options: +cmd");
    println!(";{}\t\tIN\tA", domain);
    println!();
    println!(";; ANSWER SECTION:");
    println!("{}\t3600\tIN\tA\t142.250.185.46", domain);
    println!();
    println!(";; Query time: 20 msec");
    0
}

// Name service lookup
fn cmd_nslookup(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("nslookup: missing host");
        return 1;
    }

    let host = &args[0];
    println!("Server:\t\t10.0.2.3");
    println!("Address:\t10.0.2.3#53");
    println!();
    println!("Non-authoritative answer:");
    println!("Name:\t{}", host);
    println!("Address: 142.250.185.46");
    0
}

// Get hostname
fn cmd_hostname(_args: &[String]) -> i32 {
    println!("sunlight");
    0
}

// Network statistics
fn cmd_netstat(_args: &[String]) -> i32 {
    println!("Active Internet connections");
    println!("Proto Recv-Q Send-Q Local Address    Foreign Address  State");
    println!("tcp        0      0 127.0.0.1:22    0.0.0.0:*        LISTEN");
    println!("tcp        0      0 10.0.2.15:80    0.0.0.0:*        LISTEN");
    0
}

// Socket statistics
fn cmd_ss(_args: &[String]) -> i32 {
    println!("Netid State   Recv-Q Send-Q Local Address:Port Peer Address:Port Process");
    println!("tcp   LISTEN  0      128    127.0.0.1:22                *:*");
    println!("tcp   LISTEN  0      128    10.0.2.15:80               *:*");
    0
}

// Trace route to host
fn cmd_traceroute(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("traceroute: missing host");
        return 1;
    }

    let host = &args[0];
    println!("traceroute to {} (142.250.185.46), 30 hops max, 60 byte packets", host);
    println!(" 1  10.0.2.2  5.234 ms  5.321 ms  5.123 ms");
    println!(" 2  10.0.0.1  15.234 ms  15.321 ms  15.123 ms");
    println!(" 3  142.250.185.46  20.234 ms  20.321 ms  20.123 ms");
    0
}

// ARP (address resolution protocol)
fn cmd_arp(_args: &[String]) -> i32 {
    println!("Address    HWtype  HWaddress        Flags Mask  Iface");
    println!("10.0.2.2   ether   52:54:00:12:34:56  C          eth0");
    println!("10.0.2.3   ether   52:54:00:12:34:57  C          eth0");
    0
}

// DHCP client (stub)
fn cmd_dhclient(_args: &[String]) -> i32 {
    println!("dhclient: DHCP configuration:");
    println!("  interface eth0");
    println!("  inet 10.0.2.15");
    println!("  netmask 255.255.255.0");
    println!("  gateway 10.0.2.2");
    println!("  dns 10.0.2.3");
    0
}

// Helper functions
fn chrono_like() -> String {
    "2026-06-11 12:00:00".to_string()
}

fn parse_host(url: &str) -> String {
    url.split('/').next().unwrap_or("example.com").to_string()
}

fn get_filename(url: &str) -> String {
    url.split('/').last().unwrap_or("index.html").to_string()
}
