//! SunlightOS file utilities — busybox-style multi-call binary.
//!
//! `argv[0]` selects the applet (the PATH entries `/sunlight-utils/ls` etc.
//! all exec this binary); `sunlight-utils <applet> [args…]` also works.
//! no_std on top of sunlight-libc: all I/O goes through the kernel VFS
//! syscalls (Open/Read/Close/ReadDir/StatPath/Mkdir) added in Phase 6.5
//! Step 3.

#![no_std]
#![no_main]

use sunlight_libc as libc;
use libc::{DirEntry, Errno, Fd, STDOUT, FT_DIR};

const MAX_ARGS: usize = 16;
const MAX_DIR_ENTRIES: usize = 64;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = write_all(b"sunlight-utils: panic\n");
    libc::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    debug_log_start(storage.first().copied().unwrap_or(""));
    let code = run(&storage[..count]);
    libc::exit(code as u64);
}

/// Borrow argv strings out of the exec-time stack arena.
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
            if name == "sunlight-utils" {
                match rest.split_first() {
                    Some((sub, subrest)) => (*sub, subrest),
                    None => {
                        let _ = write_all(b"usage: sunlight-utils <applet> [args...]\n");
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
        "ls" => cmd_ls(rest),
        "cat" => cmd_cat(rest),
        "mkdir" => cmd_mkdir(rest),
        "echo" => cmd_echo(rest),
        "whoami" => cmd_whoami(),
        "id" => cmd_id(rest),
        "pwd" => cmd_pwd(),
        "stat" => cmd_stat(rest),
        "file" => cmd_file(rest),
        "head" => cmd_head(rest),
        "wc" => cmd_wc(rest),
        "uname" => {
            let _ = write_all(b"SunlightOS x86_64\n");
            0
        }
        "touch" | "rm" | "rmdir" | "cp" | "mv" | "chmod" | "chown" => {
            print2(applet, ": filesystem is read-only from utils (Step 4)\n");
            1
        }
        "find" | "grep" | "sort" | "uniq" | "cut" | "tail" | "date" => {
            print2(applet, ": not implemented yet\n");
            1
        }
        _ => {
            print2(applet, ": applet not found\n");
            127
        }
    }
}

// ---------------------------------------------------------------------------
// Applets
// ---------------------------------------------------------------------------

fn cmd_ls(args: &[&str]) -> i32 {
    let path = args.first().copied().unwrap_or("/");
    debug_log2("[UTILS] ls start path=", path);
    let mut entries = [DirEntry::zeroed(); MAX_DIR_ENTRIES];
    match libc::read_dir(path.as_bytes(), &mut entries) {
        Ok(n) => {
            debug_log_u64("[UTILS] ls entries=", n as u64);
            for entry in &entries[..n] {
                debug_log_bytes("[UTILS] ls write name=", entry.name_bytes());
                let _ = write_all(entry.name_bytes());
                if entry.file_type == FT_DIR {
                    let _ = write_all(b"/");
                }
                debug_log_static("[UTILS] ls write newline");
                let _ = write_all(b"\n");
            }
            0
        }
        Err(_) => {
            print2("ls: cannot access ", path);
            let _ = write_all(b"\n");
            1
        }
    }
}

fn cmd_cat(args: &[&str]) -> i32 {
    if args.is_empty() {
        let _ = write_all(b"cat: missing file operand\n");
        return 1;
    }
    for path in args {
        let fd = match libc::open(path.as_bytes()) {
            Ok(fd) => fd,
            Err(_) => {
                print2("cat: cannot open ", path);
                let _ = write_all(b"\n");
                return 1;
            }
        };
        let mut buf = [0u8; 512];
        loop {
            match read_retry(fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = write_all(&buf[..n]);
                }
                Err(_) => {
                    let _ = libc::close(fd);
                    print2("cat: read error on ", path);
                    let _ = write_all(b"\n");
                    return 1;
                }
            }
        }
        let _ = libc::close(fd);
    }
    0
}

fn cmd_mkdir(args: &[&str]) -> i32 {
    if args.is_empty() {
        let _ = write_all(b"mkdir: missing operand\n");
        return 1;
    }
    for path in args {
        if libc::mkdir(path.as_bytes(), 0o755).is_err() {
            print2("mkdir: cannot create directory ", path);
            let _ = write_all(b"\n");
            return 1;
        }
    }
    0
}

fn cmd_echo(args: &[&str]) -> i32 {
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            let _ = write_all(b" ");
        }
        let _ = write_all(arg.as_bytes());
    }
    let _ = write_all(b"\n");
    0
}

fn cmd_whoami() -> i32 {
    let uid = libc::getuid() as u32;
    let name = username_for_uid(uid);
    let _ = write_all(name.as_bytes());
    let _ = write_all(b"\n");
    0
}

fn cmd_id(args: &[&str]) -> i32 {
    if !args.is_empty() {
        let _ = write_all(b"id: user lookup by name not implemented\n");
        return 1;
    }
    let uid = libc::getuid() as u32;
    let gid = libc::getgid() as u32;
    let uname = username_for_uid(uid);
    let gname = groupname_for_gid(gid);
    let _ = write_all(b"uid=");
    print_u64(uid as u64);
    let _ = write_all(b"(");
    let _ = write_all(uname.as_bytes());
    let _ = write_all(b") gid=");
    print_u64(gid as u64);
    let _ = write_all(b"(");
    let _ = write_all(gname.as_bytes());
    let _ = write_all(b")\n");
    0
}

fn cmd_pwd() -> i32 {
    // No per-process cwd yet; every path is absolute.
    let _ = write_all(b"/\n");
    0
}

fn cmd_stat(args: &[&str]) -> i32 {
    let Some(path) = args.first() else {
        let _ = write_all(b"stat: missing operand\n");
        return 1;
    };
    match libc::stat(path.as_bytes()) {
        Ok(st) => {
            print2("  File: ", path);
            let _ = write_all(b"\n  Size: ");
            print_u64(st.size);
            let _ = write_all(b"\n  Type: ");
            let _ = write_all(if st.file_type == FT_DIR {
                b"directory" as &[u8]
            } else {
                b"regular file"
            });
            let _ = write_all(b"\n  Mode: 0o");
            print_octal(st.mode as u64 & 0o7777);
            let _ = write_all(b"  Uid: ");
            print_u64(st.uid as u64);
            let _ = write_all(b"  Gid: ");
            print_u64(st.gid as u64);
            let _ = write_all(b"\n");
            0
        }
        Err(_) => {
            print2("stat: cannot stat ", path);
            let _ = write_all(b"\n");
            1
        }
    }
}

fn cmd_file(args: &[&str]) -> i32 {
    let Some(path) = args.first() else {
        let _ = write_all(b"file: missing operand\n");
        return 1;
    };
    match libc::stat(path.as_bytes()) {
        Ok(st) => {
            print2(path, ": ");
            let _ = write_all(if st.file_type == FT_DIR {
                b"directory\n" as &[u8]
            } else {
                b"regular file\n"
            });
            0
        }
        Err(_) => {
            print2("file: cannot stat ", path);
            let _ = write_all(b"\n");
            1
        }
    }
}

fn cmd_head(args: &[&str]) -> i32 {
    let (limit, path) = match args {
        ["-n", n, path, ..] => (parse_u64(n).unwrap_or(10), *path),
        [path, ..] => (10, *path),
        [] => {
            let _ = write_all(b"head: missing file operand\n");
            return 1;
        }
    };
    let fd = match libc::open(path.as_bytes()) {
        Ok(fd) => fd,
        Err(_) => {
            print2("head: cannot open ", path);
            let _ = write_all(b"\n");
            return 1;
        }
    };
    let mut printed_lines = 0u64;
    let mut buf = [0u8; 512];
    'outer: loop {
        match read_retry(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for (i, &b) in buf[..n].iter().enumerate() {
                    if b == b'\n' {
                        printed_lines += 1;
                        if printed_lines >= limit {
                            let _ = write_all(&buf[..=i]);
                            break 'outer;
                        }
                    }
                }
                let _ = write_all(&buf[..n]);
            }
            Err(_) => break,
        }
    }
    let _ = libc::close(fd);
    0
}

fn cmd_wc(args: &[&str]) -> i32 {
    let Some(path) = args.first() else {
        let _ = write_all(b"wc: missing file operand\n");
        return 1;
    };
    let fd = match libc::open(path.as_bytes()) {
        Ok(fd) => fd,
        Err(_) => {
            print2("wc: cannot open ", path);
            let _ = write_all(b"\n");
            return 1;
        }
    };
    let (mut lines, mut words, mut bytes) = (0u64, 0u64, 0u64);
    let mut in_word = false;
    let mut buf = [0u8; 512];
    loop {
        match read_retry(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                bytes += n as u64;
                for &b in &buf[..n] {
                    if b == b'\n' {
                        lines += 1;
                    }
                    if b.is_ascii_whitespace() {
                        in_word = false;
                    } else if !in_word {
                        in_word = true;
                        words += 1;
                    }
                }
            }
            Err(_) => break,
        }
    }
    let _ = libc::close(fd);
    let _ = write_all(b" ");
    print_u64(lines);
    let _ = write_all(b" ");
    print_u64(words);
    let _ = write_all(b" ");
    print_u64(bytes);
    print2(" ", path);
    let _ = write_all(b"\n");
    0
}

// ---------------------------------------------------------------------------
// Small I/O helpers (no alloc, retry on EAGAIN)
// ---------------------------------------------------------------------------

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

fn debug_log_start(argv0: &str) {
    let mut msg = [0u8; 128];
    let prefix = b"[UTILS] main() entered, argv[0]=";
    let mut pos = prefix.len();
    msg[..pos].copy_from_slice(prefix);
    let bytes = argv0.as_bytes();
    let copy = bytes.len().min(msg.len().saturating_sub(pos));
    msg[pos..pos + copy].copy_from_slice(&bytes[..copy]);
    pos += copy;
    let _ = unsafe { libc::sys::syscall2(libc::sys::SYS_DEBUG_LOG, msg.as_ptr() as u64, pos as u64) };
}

fn debug_log_static(s: &str) {
    let _ = unsafe {
        libc::sys::syscall2(libc::sys::SYS_DEBUG_LOG, s.as_ptr() as u64, s.len() as u64)
    };
}

fn debug_log2(prefix: &str, value: &str) {
    let mut msg = [0u8; 128];
    let p = prefix.as_bytes();
    let v = value.as_bytes();
    let p_len = p.len().min(msg.len());
    msg[..p_len].copy_from_slice(&p[..p_len]);
    let space = msg.len().saturating_sub(p_len);
    let v_len = v.len().min(space);
    msg[p_len..p_len + v_len].copy_from_slice(&v[..v_len]);
    let _ = unsafe {
        libc::sys::syscall2(
            libc::sys::SYS_DEBUG_LOG,
            msg.as_ptr() as u64,
            (p_len + v_len) as u64,
        )
    };
}

fn debug_log_u64(prefix: &str, value: u64) {
    let mut digits = [0u8; 20];
    let mut v = value;
    let mut dlen = 0usize;
    loop {
        digits[dlen] = b'0' + (v % 10) as u8;
        dlen += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let mut msg = [0u8; 128];
    let p = prefix.as_bytes();
    let mut pos = p.len().min(msg.len());
    msg[..pos].copy_from_slice(&p[..pos]);
    while dlen > 0 && pos < msg.len() {
        dlen -= 1;
        msg[pos] = digits[dlen];
        pos += 1;
    }
    let _ = unsafe { libc::sys::syscall2(libc::sys::SYS_DEBUG_LOG, msg.as_ptr() as u64, pos as u64) };
}

fn debug_log_bytes(prefix: &str, value: &[u8]) {
    let mut msg = [0u8; 128];
    let p = prefix.as_bytes();
    let mut pos = p.len().min(msg.len());
    msg[..pos].copy_from_slice(&p[..pos]);
    for &b in value {
        if pos >= msg.len() {
            break;
        }
        msg[pos] = if b.is_ascii_graphic() || b == b' ' { b } else { b'?' };
        pos += 1;
    }
    let _ = unsafe { libc::sys::syscall2(libc::sys::SYS_DEBUG_LOG, msg.as_ptr() as u64, pos as u64) };
}

fn read_retry(fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
    loop {
        match libc::read(fd, buf) {
            Err(Errno::Again) => libc::yield_now(),
            other => return other,
        }
    }
}

fn print2(a: &str, b: &str) {
    let _ = write_all(a.as_bytes());
    let _ = write_all(b.as_bytes());
}

fn print_u64(mut v: u64) {
    let mut digits = [0u8; 20];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
        if v == 0 {
            break;
        }
    }
    while n > 0 {
        n -= 1;
        let _ = write_all(&digits[n..n + 1]);
    }
}

fn print_octal(mut v: u64) {
    let mut digits = [0u8; 22];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (v % 8) as u8;
        v /= 8;
        n += 1;
        if v == 0 {
            break;
        }
    }
    while n > 0 {
        n -= 1;
        let _ = write_all(&digits[n..n + 1]);
    }
}

fn parse_u64(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut out = 0u64;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(out)
}

fn username_for_uid(uid: u32) -> &'static str {
    match uid {
        0 => "root",
        1000 => "user",
        1001 => "testuser",
        _ => "unknown",
    }
}

fn groupname_for_gid(gid: u32) -> &'static str {
    match gid {
        0 => "root",
        10 => "wheel",
        100 => "users",
        1000 => "user",
        1001 => "testuser",
        _ => "unknown",
    }
}
