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

const MAX_ARGS: usize = 16;

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
    let (applet, _rest) = match args.split_first() {
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
        "ping" | "ifconfig" | "wget" | "curl" | "dig" | "nslookup" | "netstat" | "ss"
        | "traceroute" | "arp" | "dhclient" => {
            print2(applet, ": net_server IPC for spawned processes arrives in Step 4\n");
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
