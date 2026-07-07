//! SunlightOS file utilities — busybox-style multi-call binary.
//!
//! `argv[0]` selects the applet (the PATH entries `/sunlight-utils/ls` etc.
//! all exec this binary); `sunlight-utils <applet> [args…]` also works.
//! no_std on top of sunlight-libc: all I/O goes through the kernel VFS
//! syscalls (Open/Read/Close/ReadDir/StatPath/Mkdir) added in Phase 6.5
//! Step 3.

#![no_std]
#![no_main]

use libc::{DirEntry, Errno, Fd, FT_DIR, STDOUT};
use sunlight_ipc::{
    get_time_utc, ipc_call_timeout, nameserver_lookup, query_display_metrics, DisplayMetrics,
    IpcMsg, TzMsg,
};
use sunlight_libc as libc;

const MAX_ARGS: usize = 16;
const MAX_DIR_ENTRIES: usize = 64;
const TIME_IPC_TIMEOUT_MS: u64 = 100;

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
        "kill" => cmd_kill(rest),
        "killall" => cmd_killall(rest, false),
        "pkill" => cmd_killall(rest, true),
        "free" => cmd_free(rest),
        "freezram" => cmd_freezram(rest),
        "nice" => cmd_nice(rest),
        "renice" => cmd_renice(rest),
        "pwd" => cmd_pwd(),
        "stat" => cmd_stat(rest),
        "file" => cmd_file(rest),
        "head" => cmd_head(rest),
        "wc" => cmd_wc(rest),
        "uname" => cmd_uname(rest),
        "touch" => cmd_touch(rest),
        "rm" => cmd_rm(rest),
        "rmdir" => cmd_rmdir(rest),
        "cp" => cmd_cp(rest),
        "mv" => cmd_mv(rest),
        "chmod" => cmd_chmod(rest),
        "chown" => cmd_chown(rest),
        "date" => cmd_date(rest),
        "find" | "sort" | "uniq" | "cut" | "tail" => {
            print2(applet, ": not implemented yet\n");
            1
        }
        "grep" => cmd_grep(rest),
        "display-status" => cmd_display_status(),
        _ => {
            print2(applet, ": applet not found\n");
            127
        }
    }
}

// ---------------------------------------------------------------------------
// Applets
// ---------------------------------------------------------------------------

fn cmd_display_status() -> i32 {
    let Some(display_ep) = nameserver_lookup("display_server") else {
        let _ = write_all(b"display_server: not running\n");
        return 1;
    };

    let metrics = query_display_metrics(display_ep).unwrap_or(DisplayMetrics::safe_fallback());
    let _ = write_all(b"resolution: ");
    print_u64(metrics.width_px as u64);
    let _ = write_all(b"x");
    print_u64(metrics.height_px as u64);
    let _ = write_all(b"\nstride_bytes: ");
    print_u64(metrics.stride_bytes as u64);
    let _ = write_all(b"\npixel_format: ");
    let _ = write_all(metrics.pixel_format.as_str().as_bytes());
    let _ = write_all(b"\nbackend: ");
    let _ = write_all(metrics.backend.as_str().as_bytes());
    let _ = write_all(b"\nscale: 1.0 (native)\nruntime_modesetting: unsupported\n");
    0
}

fn cmd_date(args: &[&str]) -> i32 {
    if !args.is_empty() {
        let _ = write_all(b"usage: date\n");
        return 2;
    }

    let mut dt = match query_tz_local_time() {
        Some(dt) => dt,
        None => {
            let (year, month, day, hour, minute, second) = decompose_unix(get_time_utc());
            DateTime {
                year,
                month,
                day,
                hour,
                minute,
                second,
                abbr: *b"UTC\0\0\0\0\0",
            }
        }
    };

    if !dt.is_valid() {
        let _ = write_all(b"date: invalid time source\n");
        return 1;
    }

    dt.write_date_cmd();
    let _ = write_all(b"\n");
    0
}

#[derive(Clone, Copy)]
struct DateTime {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    abbr: [u8; 8],
}

impl DateTime {
    fn is_valid(&self) -> bool {
        self.year >= 1970
            && self.month >= 1
            && self.month <= 12
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month)
            && self.hour < 24
            && self.minute < 60
            && self.second < 60
    }

    fn write_date_cmd(&mut self) {
        let wname = match weekday_from_ymd(self.year as i32, self.month, self.day) {
            0 => b"Sun",
            1 => b"Mon",
            2 => b"Tue",
            3 => b"Wed",
            4 => b"Thu",
            5 => b"Fri",
            _ => b"Sat",
        };
        let mname = match self.month {
            1 => b"Jan",
            2 => b"Feb",
            3 => b"Mar",
            4 => b"Apr",
            5 => b"May",
            6 => b"Jun",
            7 => b"Jul",
            8 => b"Aug",
            9 => b"Sep",
            10 => b"Oct",
            11 => b"Nov",
            _ => b"Dec",
        };

        let _ = write_all(wname);
        let _ = write_all(b" ");
        let _ = write_all(mname);
        let _ = write_all(b" ");
        write_two(self.day);
        let _ = write_all(b" ");
        write_two(self.hour);
        let _ = write_all(b":");
        write_two(self.minute);
        let _ = write_all(b":");
        write_two(self.second);
        let _ = write_all(b" ");
        let abbr_len = self
            .abbr
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(self.abbr.len());
        if abbr_len == 0 {
            let _ = write_all(b"UTC");
        } else {
            let _ = write_all(&self.abbr[..abbr_len]);
        }
        let _ = write_all(b" ");
        write_four(self.year);
    }
}

fn query_tz_local_time() -> Option<DateTime> {
    let tz = nameserver_lookup("tz")?;
    let reply = ipc_call_timeout(
        tz,
        IpcMsg::with_label(TzMsg::GET_LOCAL_TIME),
        TIME_IPC_TIMEOUT_MS,
    )
    .ok()?;
    if reply.label != TzMsg::REPLY {
        return None;
    }
    let word = reply.words[0];
    let mut abbr = [0u8; 8];
    let abbr_word = reply.words[3];
    for (idx, slot) in abbr.iter_mut().enumerate() {
        *slot = ((abbr_word >> (idx * 8)) & 0xff) as u8;
    }
    Some(DateTime {
        year: ((word >> 48) & 0xffff) as u16,
        month: ((word >> 40) & 0xff) as u8,
        day: ((word >> 32) & 0xff) as u8,
        hour: ((word >> 24) & 0xff) as u8,
        minute: ((word >> 16) & 0xff) as u8,
        second: ((word >> 8) & 0xff) as u8,
        abbr,
    })
}

fn decompose_unix(ts: u64) -> (u16, u8, u8, u8, u8, u8) {
    let days = (ts / 86_400) as i64;
    let secs = ts % 86_400;
    let (year, month, day) = civil_from_days(days);
    (
        year as u16,
        month,
        day,
        (secs / 3600) as u8,
        ((secs % 3600) / 60) as u8,
        (secs % 60) as u8,
    )
}

fn civil_from_days(mut z: i64) -> (i32, u8, u8) {
    z += 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = (yoe + era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = (mp + if mp < 10 { 3 } else { -9 }) as u8;
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

fn weekday_from_ymd(year: i32, month: u8, day: u8) -> u8 {
    let mut yy = year;
    let mut mm = month as i32;
    if mm <= 2 {
        yy -= 1;
        mm += 12;
    }
    let c = yy / 100;
    let k = yy % 100;
    let w = (day as i32 + (13 * (mm + 1) / 5) + k + (k / 4) + (c / 4) + 5 * c) % 7;
    ((w + 6) % 7) as u8
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn write_two(value: u8) {
    let _ = write_all(&[b'0' + value / 10, b'0' + value % 10]);
}

fn write_four(value: u16) {
    let _ = write_all(&[
        b'0' + ((value / 1000) % 10) as u8,
        b'0' + ((value / 100) % 10) as u8,
        b'0' + ((value / 10) % 10) as u8,
        b'0' + (value % 10) as u8,
    ]);
}

fn cmd_ls(args: &[&str]) -> i32 {
    let mut long_format = false;
    let mut show_all = false;
    let mut classify = false;
    let mut path = "/";
    for arg in args {
        if arg.starts_with('-') && arg.len() > 1 {
            for &b in arg.as_bytes().iter().skip(1) {
                match b {
                    b'l' => long_format = true,
                    b'a' => show_all = true,
                    b'F' => classify = true,
                    _ => {}
                }
            }
        } else {
            path = arg;
        }
    }
    debug_log2("[UTILS] ls start path=", path);
    let mut entries = [DirEntry::zeroed(); MAX_DIR_ENTRIES];
    match libc::read_dir(path.as_bytes(), &mut entries) {
        Ok(n) => {
            let mut shown = 0u64;
            debug_log_u64("[UTILS] ls entries=", n as u64);
            for entry in &entries[..n] {
                let name = entry.name_bytes();
                if !show_all && name.first() == Some(&b'.') {
                    continue;
                }
                if long_format {
                    let _ = write_all(if entry.file_type == FT_DIR {
                        b"drwxr-xr-x " as &[u8]
                    } else {
                        b"-rw-r--r-- "
                    });
                    print_u64(entry.size);
                    let _ = write_all(b" ");
                }
                debug_log_bytes("[UTILS] ls write name=", entry.name_bytes());
                let _ = write_all(entry.name_bytes());
                if entry.file_type == FT_DIR && (classify || !long_format) {
                    let _ = write_all(b"/");
                }
                debug_log_static("[UTILS] ls write newline");
                let _ = write_all(b"\n");
                shown += 1;
            }
            debug_log_u64("[UTILS] ls shown=", shown);
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

const TELEMETRY_MAGIC: u64 = 0x5355_4E4C_5449_4D45;
const TELEMETRY_MAX_PROCS: usize = 64;
const SIGTERM: u32 = 15;
const SIGKILL: u32 = 9;

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
struct TelemetryProcessStat {
    pid: u32,
    ppid: u32,
    state: u8,
    _pad: [u8; 3],
    name: [u8; 32],
    cpu_ticks: u64,
    mem_pages: u32,
    _pad2: u32,
}

#[repr(C)]
struct TelemetryPage {
    magic: u64,
    version: u32,
    sequence: u32,
    uptime_secs: u64,
    total_ram_kb: u64,
    used_ram_kb: u64,
    zram_orig_kb: u64,
    zram_comp_kb: u64,
    net_rx_bytes: u64,
    net_tx_bytes: u64,
    tick_hz: u32,
    cpu_count: u8,
    _pad: [u8; 3],
    sample_time_ns: u64,
    proc_count: u32,
    procs: [TelemetryProcessStat; TELEMETRY_MAX_PROCS],
}

fn telemetry_page() -> Option<&'static TelemetryPage> {
    let ptr = libc::map_telemetry().ok()? as *const TelemetryPage;
    if ptr.is_null() {
        return None;
    }
    let page = unsafe { &*ptr };
    (page.magic == TELEMETRY_MAGIC).then_some(page)
}

fn parse_signal(args: &[&str]) -> Result<(u32, usize), i32> {
    if args.is_empty() {
        return Ok((SIGTERM, 0));
    }
    let sig = match args[0] {
        "-9" | "-KILL" | "-SIGKILL" => Some(SIGKILL),
        "-15" | "-TERM" | "-SIGTERM" => Some(SIGTERM),
        _ => None,
    };
    match sig {
        Some(signal) => Ok((signal, 1)),
        None => Ok((SIGTERM, 0)),
    }
}

fn cmd_kill(args: &[&str]) -> i32 {
    let (signal, start) = match parse_signal(args) {
        Ok(v) => v,
        Err(code) => return code,
    };
    if start >= args.len() {
        let _ = write_all(b"kill: missing pid\n");
        return 1;
    }
    let Some(pid) = parse_u64(args[start]) else {
        let _ = write_all(b"kill: invalid pid\n");
        return 1;
    };
    if libc::kill(pid, signal).is_err() {
        let _ = write_all(b"kill: signal delivery failed\n");
        return 1;
    }
    0
}

fn cmd_killall(args: &[&str], substring: bool) -> i32 {
    let (signal, start) = match parse_signal(args) {
        Ok(v) => v,
        Err(code) => return code,
    };
    if start >= args.len() {
        let msg: &[u8] = if substring {
            b"pkill: missing pattern\n"
        } else {
            b"killall: missing name\n"
        };
        let _ = write_all(msg);
        return 1;
    }

    let pattern = args[start].as_bytes();
    let Some(page) = telemetry_page() else {
        let _ = write_all(b"killall: telemetry unavailable\n");
        return 1;
    };

    let count = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(page.proc_count)) as usize }
        .min(TELEMETRY_MAX_PROCS);
    let mut matched = 0usize;
    let mut delivered = 0usize;
    for i in 0..count {
        let stat = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(page.procs[i])) };
        if stat.pid == 0 || stat.state == 3 {
            continue;
        }
        let name = process_name_bytes(&stat.name);
        let is_match = if substring {
            contains_bytes(name, pattern)
        } else {
            name == pattern
        };
        if !is_match {
            continue;
        }
        matched += 1;
        if libc::kill(stat.pid as u64, signal).is_ok() {
            delivered += 1;
        }
    }

    if matched == 0 {
        let msg: &[u8] = if substring {
            b"pkill: no matching process\n"
        } else {
            b"killall: no matching process\n"
        };
        let _ = write_all(msg);
        return 1;
    }
    if delivered != matched {
        let _ = write_all(b"killall: partial delivery failure\n");
        return 1;
    }
    0
}

fn process_name_bytes(name: &[u8; 32]) -> &[u8] {
    let mut len = 0usize;
    while len < name.len() && name[len] != 0 {
        len += 1;
    }
    &name[..len]
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    let last = haystack.len() - needle.len();
    let mut i = 0usize;
    while i <= last {
        if &haystack[i..i + needle.len()] == needle {
            return true;
        }
        i += 1;
    }
    false
}

fn cmd_touch(args: &[&str]) -> i32 {
    if args.is_empty() {
        let _ = write_all(b"touch: missing file operand\n");
        return 1;
    }
    for path in args {
        match libc::create(path.as_bytes()) {
            Ok(fd) => {
                let _ = libc::close(fd);
            }
            Err(_) => {
                print2("touch: cannot touch ", path);
                let _ = write_all(b": permission denied or read-only filesystem\n");
                return 1;
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Progress bar helper
// ---------------------------------------------------------------------------

/// Print a simple progress bar: `####---------- 40%`  (bar_width=10 chars)
fn print_progress(done: u64, total: u64) {
    const W: usize = 40;
    let pct = if total == 0 {
        100u64
    } else {
        done * 100 / total
    };
    let filled = if total == 0 {
        W
    } else {
        (done * W as u64 / total) as usize
    }
    .min(W);
    let _ = write_all(b"\r[");
    for _ in 0..filled {
        let _ = write_all(b"#");
    }
    for _ in filled..W {
        let _ = write_all(b"-");
    }
    let _ = write_all(b"] ");
    print_u64(pct);
    let _ = write_all(b"%");
}

// ---------------------------------------------------------------------------
// rm / rmdir
// ---------------------------------------------------------------------------

fn cmd_rm(args: &[&str]) -> i32 {
    if args.is_empty() {
        let _ = write_all(b"rm: missing operand\n");
        return 1;
    }
    let mut recursive = false;
    let mut paths: [&str; MAX_ARGS] = [""; MAX_ARGS];
    let mut path_count = 0usize;

    for arg in args {
        if *arg == "-r" || *arg == "-rf" || *arg == "-R" {
            recursive = true;
        } else if arg.starts_with('-') {
            for &b in arg.as_bytes().iter().skip(1) {
                match b {
                    b'r' | b'R' => recursive = true,
                    b'f' => {}
                    _ => {
                        let _ = write_all(b"rm: invalid option\n");
                        return 1;
                    }
                }
            }
        } else if path_count < MAX_ARGS {
            paths[path_count] = arg;
            path_count += 1;
        }
    }

    if path_count == 0 {
        let _ = write_all(b"rm: missing operand\n");
        return 1;
    }

    let mut code = 0i32;
    for &path in &paths[..path_count] {
        code |= rm_path(path, recursive);
    }
    code
}

fn rm_path(path: &str, recursive: bool) -> i32 {
    match libc::stat(path.as_bytes()) {
        Ok(st) => {
            if st.file_type == libc::FT_DIR {
                if !recursive {
                    print2("rm: cannot remove '", path);
                    let _ = write_all(b"': Is a directory\n");
                    return 1;
                }
                rm_dir_recursive(path)
            } else {
                match libc::unlink(path.as_bytes()) {
                    Ok(()) => 0,
                    Err(_) => {
                        print2("rm: cannot remove '", path);
                        let _ = write_all(b"': Permission denied\n");
                        1
                    }
                }
            }
        }
        Err(_) => {
            print2("rm: cannot remove '", path);
            let _ = write_all(b"': No such file or directory\n");
            1
        }
    }
}

fn rm_dir_recursive(path: &str) -> i32 {
    let mut entries = [libc::DirEntry::zeroed(); MAX_DIR_ENTRIES];
    let n = match libc::read_dir(path.as_bytes(), &mut entries) {
        Ok(n) => n,
        Err(_) => {
            print2("rm: cannot read dir '", path);
            let _ = write_all(b"'\n");
            return 1;
        }
    };

    let mut code = 0i32;
    for entry in &entries[..n] {
        let name = entry.name_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        let mut child = [0u8; 256];
        let child_path = join_path(&mut child, path, name);
        let child_str = match core::str::from_utf8(child_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        code |= rm_path(child_str, true);
    }

    if code == 0 {
        match libc::unlink(path.as_bytes()) {
            Ok(()) => {}
            Err(_) => {
                print2("rm: cannot remove dir '", path);
                let _ = write_all(b"'\n");
                code = 1;
            }
        }
    }
    code
}

fn cmd_rmdir(args: &[&str]) -> i32 {
    if args.is_empty() {
        let _ = write_all(b"rmdir: missing operand\n");
        return 1;
    }
    let mut code = 0i32;
    for path in args {
        match libc::stat(path.as_bytes()) {
            Ok(st) if st.file_type == libc::FT_DIR => match libc::unlink(path.as_bytes()) {
                Ok(()) => {}
                Err(_) => {
                    print2("rmdir: failed to remove '", path);
                    let _ = write_all(b"'\n");
                    code = 1;
                }
            },
            Ok(_) => {
                print2("rmdir: '", path);
                let _ = write_all(b"' is not a directory\n");
                code = 1;
            }
            Err(_) => {
                print2("rmdir: '", path);
                let _ = write_all(b"': No such file or directory\n");
                code = 1;
            }
        }
    }
    code
}

// ---------------------------------------------------------------------------
// cp / mv
// ---------------------------------------------------------------------------

fn cmd_cp(args: &[&str]) -> i32 {
    if args.len() < 2 {
        let _ = write_all(b"cp: missing destination file operand\n");
        return 1;
    }
    let src = args[args.len() - 2];
    let dst = args[args.len() - 1];
    cp_file(src, dst, true)
}

fn cp_file(src: &str, dst: &str, show_progress: bool) -> i32 {
    let src_fd = match libc::open(src.as_bytes()) {
        Ok(fd) => fd,
        Err(_) => {
            print2("cp: cannot open '", src);
            let _ = write_all(b"'\n");
            return 1;
        }
    };

    // Get size for progress bar
    let file_size = match libc::fstat(src_fd) {
        Ok(st) => st.size as u64,
        Err(_) => 0,
    };

    let dst_fd = match libc::create(dst.as_bytes()) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = libc::close(src_fd);
            print2("cp: cannot create '", dst);
            let _ = write_all(b"': Permission denied\n");
            return 1;
        }
    };

    let mut buf = [0u8; 512];
    let mut total_copied = 0u64;
    let mut dst_offset = 0usize;
    let mut code = 0i32;

    loop {
        match read_retry(src_fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = &buf[..n];
                match libc::write(dst_fd, chunk) {
                    Ok(_) => {}
                    Err(_) => {
                        let _ = write_all(b"\ncp: write error\n");
                        code = 1;
                        break;
                    }
                }
                // Advance dst offset (simplified: sequential write)
                dst_offset += n;
                total_copied += n as u64;
                if show_progress && file_size > 0 {
                    print_progress(total_copied, file_size);
                }
            }
            Err(_) => {
                let _ = write_all(b"\ncp: read error\n");
                code = 1;
                break;
            }
        }
    }

    let _ = libc::close(src_fd);
    let _ = libc::close(dst_fd);

    if show_progress && file_size > 0 {
        print_progress(total_copied, file_size.max(total_copied));
        let _ = write_all(b"\n");
    }

    let _ = dst_offset; // suppress warning
    code
}

fn cmd_mv(args: &[&str]) -> i32 {
    if args.len() < 2 {
        let _ = write_all(b"mv: missing destination file operand\n");
        return 1;
    }
    let src = args[args.len() - 2];
    let dst = args[args.len() - 1];

    // Try rename first (fast path, same mount).
    if libc::rename(src.as_bytes(), dst.as_bytes()).is_ok() {
        return 0;
    }

    // Fall back to copy + delete.
    let code = cp_file(src, dst, true);
    if code == 0 {
        if libc::unlink(src.as_bytes()).is_err() {
            print2("mv: warning: could not remove source '", src);
            let _ = write_all(b"'\n");
        }
    }
    code
}

// ---------------------------------------------------------------------------
// chmod / chown
// ---------------------------------------------------------------------------

fn cmd_chmod(args: &[&str]) -> i32 {
    let [mode_s, path] = args else {
        let _ = write_all(b"usage: chmod MODE FILE\n");
        return 2;
    };
    let Some(mode) = parse_octal(mode_s) else {
        let _ = write_all(b"chmod: invalid mode\n");
        return 1;
    };
    match libc::chmod(path.as_bytes(), mode as u16) {
        Ok(()) => 0,
        Err(_) => {
            print2("chmod: cannot change mode of '", path);
            let _ = write_all(b"': Permission denied\n");
            1
        }
    }
}

fn cmd_chown(args: &[&str]) -> i32 {
    let [owner_s, path] = args else {
        let _ = write_all(b"usage: chown OWNER[:GROUP] FILE\n");
        return 2;
    };
    // Parse "uid" or "uid:gid"
    let (uid_s, gid_s) = match owner_s.find(':') {
        Some(idx) => (&owner_s[..idx], Some(&owner_s[idx + 1..])),
        None => (*owner_s, None),
    };
    let Some(uid) = parse_u64(uid_s) else {
        let _ = write_all(b"chown: invalid user\n");
        return 1;
    };
    let gid = match gid_s {
        Some(s) => match parse_u64(s) {
            Some(g) => g,
            None => {
                let _ = write_all(b"chown: invalid group\n");
                return 1;
            }
        },
        None => uid, // default gid = uid
    };
    match libc::chown(path.as_bytes(), uid as u32, gid as u32) {
        Ok(()) => 0,
        Err(_) => {
            print2("chown: cannot change owner of '", path);
            let _ = write_all(b"': Permission denied\n");
            1
        }
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Build `parent/name` into `buf`. Returns the filled slice.
fn join_path<'a>(buf: &'a mut [u8], parent: &str, name: &[u8]) -> &'a [u8] {
    let mut pos = 0usize;
    let pb = parent.as_bytes();
    let plen = pb.len().min(buf.len().saturating_sub(2));
    buf[..plen].copy_from_slice(&pb[..plen]);
    pos += plen;
    if pos < buf.len() && buf.get(pos.saturating_sub(1)) != Some(&b'/') {
        buf[pos] = b'/';
        pos += 1;
    }
    let nlen = name.len().min(buf.len().saturating_sub(pos + 1));
    buf[pos..pos + nlen].copy_from_slice(&name[..nlen]);
    pos += nlen;
    if pos < buf.len() {
        buf[pos] = 0;
    }
    &buf[..pos]
}

fn parse_octal(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut out = 0u64;
    for &b in s.as_bytes() {
        if b < b'0' || b > b'7' {
            return None;
        }
        out = out.checked_mul(8)?.checked_add((b - b'0') as u64)?;
    }
    Some(out)
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

fn cmd_nice(args: &[&str]) -> i32 {
    match args {
        [] => match libc::getnice(0) {
            Ok(nice) => {
                print_i64(nice as i64);
                let _ = write_all(b"\n");
                0
            }
            Err(_) => {
                let _ = write_all(b"nice: failed to get current nice\n");
                1
            }
        },
        ["-n", n] => {
            let Some(value) = parse_i64(n) else {
                let _ = write_all(b"nice: invalid nice value\n");
                return 1;
            };
            let Ok(requested) = i8::try_from(value) else {
                let _ = write_all(b"nice: invalid nice value\n");
                return 1;
            };
            match libc::setnice(0, requested) {
                Ok(applied) => {
                    let _ = write_all(b"nice: set to ");
                    print_i64(applied as i64);
                    let _ = write_all(b"\n");
                    0
                }
                Err(_) => {
                    let _ = write_all(b"nice: permission denied or failed\n");
                    1
                }
            }
        }
        _ => {
            let _ = write_all(b"usage: nice [-n N]\n");
            2
        }
    }
}

fn cmd_renice(args: &[&str]) -> i32 {
    let [nice_s, pid_s] = args else {
        let _ = write_all(b"usage: renice N PID\n");
        return 2;
    };

    let Some(nice) = parse_i64(nice_s) else {
        let _ = write_all(b"renice: invalid nice value\n");
        return 1;
    };
    let Ok(requested) = i8::try_from(nice) else {
        let _ = write_all(b"renice: invalid nice value\n");
        return 1;
    };
    let Some(pid) = parse_u64(pid_s) else {
        let _ = write_all(b"renice: invalid pid\n");
        return 1;
    };

    match libc::setnice(pid, requested) {
        Ok(applied) => {
            let _ = write_all(b"renice: pid ");
            print_u64(pid);
            let _ = write_all(b" now ");
            print_i64(applied as i64);
            let _ = write_all(b"\n");
            0
        }
        Err(_) => {
            let _ = write_all(b"renice: permission denied or failed\n");
            1
        }
    }
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

// ---------------------------------------------------------------------------
// High-performance grep (memchr SIMD, fixed 64KB buffer, no per-line alloc)
// ---------------------------------------------------------------------------

fn cmd_grep(args: &[&str]) -> i32 {
    if args.is_empty() {
        let _ = write_all(b"grep: missing pattern\n");
        return 2;
    }

    // First arg is pattern (literal bytes). Remaining are optional files.
    let pattern = args[0].as_bytes();
    let files = &args[1..];

    let finder = memchr::memmem::Finder::new(pattern);
    let mut found_any = false;

    if files.is_empty() {
        // Read from stdin (fd 0). Critical for pipelines: cmd | grep pat
        if let Ok(code) = grep_fd(libc::Fd(0), &finder, &mut found_any) {
            return code;
        } else {
            return 1;
        }
    }

    let mut overall = 0i32;
    for &path in files {
        let fd = match libc::open(path.as_bytes()) {
            Ok(f) => f,
            Err(_) => {
                print2("grep: cannot open ", path);
                let _ = write_all(b"\n");
                overall = 1;
                continue;
            }
        };
        let mut local_found = false;
        match grep_fd(fd, &finder, &mut local_found) {
            Ok(_) => {
                if local_found {
                    found_any = true;
                }
            }
            Err(_) => {
                overall = 1;
            }
        }
        let _ = libc::close(fd);
    }

    if overall != 0 {
        overall
    } else if found_any {
        0
    } else {
        1
    }
}

/// Core fast grep over an open fd using 64KiB buffer + tail carry + memchr.
fn grep_fd(fd: libc::Fd, finder: &memchr::memmem::Finder, found_any: &mut bool) -> Result<i32, ()> {
    const BUFSZ: usize = 64 * 1024;
    let mut buffer = [0u8; BUFSZ];
    let mut tail_len = 0usize;

    loop {
        let bytes_read = match read_retry(fd, &mut buffer[tail_len..]) {
            Ok(n) => n,
            Err(_) => break,
        };
        if bytes_read == 0 {
            break; // EOF
        }

        let end = tail_len + bytes_read;
        let chunk = &buffer[..end];

        let mut last_line_end = 0usize;

        while let Some(nl_rel) = memchr::memchr(b'\n', &chunk[last_line_end..]) {
            let line_start = last_line_end;
            let line_end = last_line_end + nl_rel;
            let line = &chunk[line_start..line_end];

            if finder.find(line).is_some() {
                // Write the matching line + newline
                let _ = write_all(line);
                let _ = write_all(b"\n");
                *found_any = true;
            }

            last_line_end = line_end + 1;
        }

        // Carry over the incomplete final line (no trailing \n yet)
        tail_len = end - last_line_end;
        if tail_len > 0 {
            // memmove the tail to start of buffer
            if last_line_end > 0 {
                buffer.copy_within(last_line_end..end, 0);
            }
        }
    }

    // Handle final tail without newline
    if tail_len > 0 {
        let tail = &buffer[..tail_len];
        if finder.find(tail).is_some() {
            let _ = write_all(tail);
            let _ = write_all(b"\n");
            *found_any = true;
        }
    }

    Ok(0)
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

#[derive(Clone, Copy)]
enum FreeUnit {
    Human,
    MB,
    GB,
}

fn cmd_free(args: &[&str]) -> i32 {
    let mut unit = FreeUnit::MB;

    for arg in args {
        match *arg {
            "-h" | "--human-readable" => unit = FreeUnit::Human,
            "-m" => unit = FreeUnit::MB,
            "-g" => unit = FreeUnit::GB,
            _ => {
                let _ = write_all(b"usage: free [-h|-m|-g]\n");
                return 2;
            }
        }
    }

    let info = match libc::sysinfo() {
        Ok(s) => s,
        Err(_) => {
            let _ = write_all(b"free: sysinfo failed\n");
            return 1;
        }
    };

    let total_kb = info.total_ram_kb;
    let used_kb = info.used_ram_kb.min(total_kb);
    let free_kb = total_kb.saturating_sub(used_kb);
    let swap_total_kb = info.swap_total_kb;
    let swap_used_kb = info.swap_used_kb.min(info.swap_total_kb);
    let swap_free_kb = swap_total_kb.saturating_sub(swap_used_kb);

    let (hdr1, hdr2, hdr3) = match unit {
        FreeUnit::Human => ("total", "used", "free"),
        FreeUnit::MB => ("total(MB)", "used(MB)", "free(MB)"),
        FreeUnit::GB => ("total(GB)", "used(GB)", "free(GB)"),
    };

    let _ = write_all(b"              ");
    let _ = write_all(hdr1.as_bytes());
    let _ = write_all(b"    ");
    let _ = write_all(hdr2.as_bytes());
    let _ = write_all(b"    ");
    let _ = write_all(hdr3.as_bytes());
    let _ = write_all(b"\n");

    let _ = write_all(b"Mem:        ");
    write_unit(total_kb, unit);
    let _ = write_all(b"    ");
    write_unit(used_kb, unit);
    let _ = write_all(b"    ");
    write_unit(free_kb, unit);
    let _ = write_all(b"\n");

    let _ = write_all(b"Swap:       ");
    write_unit(swap_total_kb, unit);
    let _ = write_all(b"    ");
    write_unit(swap_used_kb, unit);
    let _ = write_all(b"    ");
    write_unit(swap_free_kb, unit);
    let _ = write_all(b"\n");

    if swap_used_kb > 0 {
        let compressed_kb = info.swap_compressed_kb.max(1);
        let ratio_x10 = (swap_used_kb * 10) / compressed_kb;
        let _ = write_all(b"  compressed: ");
        write_unit(compressed_kb, unit);
        let _ = write_all(b" (ratio ");
        print_u64(ratio_x10 / 10);
        let _ = write_all(b".");
        print_u64(ratio_x10 % 10);
        let _ = write_all(b"x)\n");
    }
    0
}

/// Demo/verification command for the ZRAM swap pipeline: writes synthetic
/// compressed pages into ZRAM (`freezram [n]`, default 16) or verifies and
/// discards a prior fill (`freezram verify`), printing swap usage before and
/// after so live activity is visible.
fn cmd_freezram(args: &[&str]) -> i32 {
    match args {
        ["verify"] => match libc::freezram_verify() {
            Ok(n) => {
                let _ = write_all(b"freezram: verified ");
                print_u64(n);
                let _ = write_all(b" page(s)\n");
                print_swap_used("after verify: ");
                0
            }
            Err(_) => {
                let _ = write_all(b"freezram: verify failed (read/decompress error)\n");
                1
            }
        },
        [] | [_] => {
            let n = match args {
                [n_str] => match parse_u64(n_str) {
                    Some(n) => n,
                    None => {
                        let _ = write_all(b"usage: freezram [n] | freezram verify\n");
                        return 2;
                    }
                },
                _ => 16,
            };

            print_swap_used("before fill: ");
            let written = libc::freezram_fill(n);
            let _ = write_all(b"freezram: wrote ");
            print_u64(written);
            let _ = write_all(b" page(s)\n");
            print_swap_used("after fill:  ");
            0
        }
        _ => {
            let _ = write_all(b"usage: freezram [n] | freezram verify\n");
            2
        }
    }
}

fn print_swap_used(label: &str) {
    let _ = write_all(label.as_bytes());
    match libc::sysinfo() {
        Ok(info) => {
            print_human(info.swap_used_kb);
            let _ = write_all(b" used / ");
            print_human(info.swap_total_kb);
            let _ = write_all(b" total swap\n");
        }
        Err(_) => {
            let _ = write_all(b"sysinfo failed\n");
        }
    }
}

fn write_unit(kb: u64, unit: FreeUnit) {
    match unit {
        FreeUnit::MB => print_u64(kb / 1024),
        FreeUnit::GB => print_u64(kb / (1024 * 1024)),
        FreeUnit::Human => print_human(kb),
    }
}

fn print_human(kb: u64) {
    if kb >= 1024 * 1024 {
        print_scaled(kb, 1024 * 1024, b'G');
    } else if kb >= 1024 {
        print_scaled(kb, 1024, b'M');
    } else {
        print_u64(kb);
        let _ = write_all(b"K");
    }
}

fn print_scaled(value: u64, base: u64, suffix: u8) {
    let integer = value / base;
    let frac = ((value % base) * 10) / base;
    print_u64(integer);
    let _ = write_all(b".");
    print_u64(frac);
    let _ = write_all(&[suffix]);
}

fn cmd_uname(args: &[&str]) -> i32 {
    let mut show_kernel_name = false;
    let mut show_nodename = false;
    let mut show_kernel_release = false;
    let mut show_kernel_version = false;
    let mut show_machine = false;
    let mut show_processor = false;
    let mut show_hw_platform = false;
    let mut show_operating_system = false;
    let mut show_all = false;

    if args.is_empty() {
        show_kernel_name = true;
    } else {
        for arg in args {
            if *arg == "--help" {
                return uname_help();
            }
            if *arg == "--version" {
                return uname_version();
            }
            if let Some(long) = arg.strip_prefix("--") {
                match long {
                    "all" => {
                        show_kernel_name = true;
                        show_nodename = true;
                        show_kernel_release = true;
                        show_kernel_version = true;
                        show_machine = true;
                        if processor_name().is_some() {
                            show_processor = true;
                        }
                        if hardware_platform_name().is_some() {
                            show_hw_platform = true;
                        }
                        show_operating_system = true;
                        show_all = true;
                    }
                    "kernel-name" => show_kernel_name = true,
                    "nodename" => show_nodename = true,
                    "kernel-release" => show_kernel_release = true,
                    "kernel-version" => show_kernel_version = true,
                    "machine" => show_machine = true,
                    "processor" => show_processor = true,
                    "hardware-platform" => show_hw_platform = true,
                    "operating-system" => show_operating_system = true,
                    _ => {
                        let _ = write_all(b"uname: invalid option -- ");
                        let _ = write_all(arg.as_bytes());
                        let _ = write_all(b"\nTry 'uname --help' for more information.\n");
                        return 1;
                    }
                }
                continue;
            }

            if arg.starts_with('-') && arg.len() > 1 {
                for &b in arg.as_bytes().iter().skip(1) {
                    match b {
                        b'a' => {
                            show_kernel_name = true;
                            show_nodename = true;
                            show_kernel_release = true;
                            show_kernel_version = true;
                            show_machine = true;
                            if processor_name().is_some() {
                                show_processor = true;
                            }
                            if hardware_platform_name().is_some() {
                                show_hw_platform = true;
                            }
                            show_operating_system = true;
                            show_all = true;
                        }
                        b's' => show_kernel_name = true,
                        b'n' => show_nodename = true,
                        b'r' => show_kernel_release = true,
                        b'v' => show_kernel_version = true,
                        b'm' => show_machine = true,
                        b'p' => show_processor = true,
                        b'i' => show_hw_platform = true,
                        b'o' => show_operating_system = true,
                        _ => {
                            let _ = write_all(b"uname: invalid option -- ");
                            let _ = write_all(&[b]);
                            let _ = write_all(b"\nTry 'uname --help' for more information.\n");
                            return 1;
                        }
                    }
                }
            } else {
                let _ = write_all(b"uname: extra operand ");
                let _ = write_all(arg.as_bytes());
                let _ = write_all(b"\nTry 'uname --help' for more information.\n");
                return 1;
            }
        }
    }

    let mut first = true;
    if show_kernel_name {
        write_uname_field(&mut first, kernel_name().as_bytes());
    }
    if show_nodename {
        let mut host = [0u8; 64];
        let n = nodename_bytes(&mut host);
        write_uname_field(&mut first, &host[..n]);
    }
    if show_kernel_release {
        write_uname_field(&mut first, kernel_release().as_bytes());
    }
    if show_kernel_version {
        write_uname_field(&mut first, kernel_version().as_bytes());
    }
    if show_machine {
        write_uname_field(&mut first, machine_name().as_bytes());
    }
    if show_processor {
        let value = processor_name().unwrap_or("unknown");
        write_uname_field(&mut first, value.as_bytes());
    }
    if show_hw_platform {
        let value = hardware_platform_name().unwrap_or("unknown");
        write_uname_field(&mut first, value.as_bytes());
    }
    if show_operating_system {
        write_uname_field(&mut first, operating_system().as_bytes());
    }
    if show_all {
        if let Some(page) = telemetry_page() {
            let used_mb = page.used_ram_kb / 1024;
            let total_mb = page.total_ram_kb / 1024;
            let mut b1 = [0u8; 20];
            let mut b2 = [0u8; 20];
            let _ = write_all(b" [RAM: ");
            let _ = write_all(write_u64_dec(used_mb, &mut b1));
            let _ = write_all(b"MB/");
            let _ = write_all(write_u64_dec(total_mb, &mut b2));
            let _ = write_all(b"MB - microkernel stays lean!]");
        }
    }
    let _ = write_all(b"\n");
    0
}

fn write_u64_dec<'a>(mut v: u64, buf: &'a mut [u8; 20]) -> &'a [u8] {
    if v == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut tmp = [0u8; 20];
    let mut n = 0usize;
    while v > 0 && n < tmp.len() {
        tmp[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    &buf[..n]
}

fn write_uname_field(first: &mut bool, value: &[u8]) {
    if !*first {
        let _ = write_all(b" ");
    }
    let _ = write_all(value);
    *first = false;
}

fn uname_help() -> i32 {
    let _ = write_all(
        b"Usage: uname [OPTION]...\n\
Print certain system information.  With no OPTION, same as -s.\n\
\n\
  -a, --all                print all information, in the following order,\n\
                             except omit -p and -i if unknown\n\
  -s, --kernel-name        print the kernel name\n\
  -n, --nodename           print the network node hostname\n\
  -r, --kernel-release     print the kernel release\n\
  -v, --kernel-version     print the kernel version\n\
  -m, --machine            print the machine hardware name\n\
  -p, --processor          print the processor type (non-portable)\n\
  -i, --hardware-platform  print the hardware platform (non-portable)\n\
  -o, --operating-system   print the operating system\n\
      --help               display this help and exit\n\
      --version            output version information and exit\n",
    );
    0
}

fn uname_version() -> i32 {
    let _ = write_all(b"uname (sunlight-utils) ");
    let _ = write_all(kernel_release().as_bytes());
    let _ = write_all(b"\n");
    let _ = write_all(b"target: ");
    let _ = write_all(machine_name().as_bytes());
    let _ = write_all(b"\n");
    if let Some(source_ident) = option_env!("COOKBOOK_SOURCE_IDENT") {
        if !source_ident.is_empty() {
            let _ = write_all(b"source: ");
            let _ = write_all(source_ident.as_bytes());
            let _ = write_all(b"\n");
        }
    }
    0
}

fn kernel_name() -> &'static str {
    "SunlightOS"
}

fn operating_system() -> &'static str {
    "SunlightOS"
}

fn machine_name() -> &'static str {
    option_env!("TARGET")
        .and_then(|t| t.split('-').next())
        .unwrap_or("x86_64")
}

fn processor_name() -> Option<&'static str> {
    None
}

fn hardware_platform_name() -> Option<&'static str> {
    None
}

fn kernel_release() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn kernel_version() -> &'static str {
    if let Some(source_ident) = option_env!("COOKBOOK_SOURCE_IDENT") {
        if !source_ident.is_empty() {
            return source_ident;
        }
    }
    "SunlightOS build"
}

fn nodename_bytes(out: &mut [u8]) -> usize {
    let fd = match libc::open(b"/etc/hostname") {
        Ok(fd) => fd,
        Err(_) => return copy_into(out, b"sunlight"),
    };

    let mut buf = [0u8; 128];
    let read = read_retry(fd, &mut buf).unwrap_or(0);
    let _ = libc::close(fd);
    if read == 0 {
        return copy_into(out, b"sunlight");
    }

    let mut end = 0usize;
    while end < read {
        let b = buf[end];
        if b == b'\n' || b == b'\r' {
            break;
        }
        end += 1;
    }

    if end == 0 {
        copy_into(out, b"sunlight")
    } else {
        copy_into(out, &buf[..end])
    }
}

fn copy_into(dst: &mut [u8], src: &[u8]) -> usize {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    n
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
    let _ =
        unsafe { libc::sys::syscall2(libc::sys::SYS_DEBUG_LOG, msg.as_ptr() as u64, pos as u64) };
}

fn debug_log_static(s: &str) {
    let _ =
        unsafe { libc::sys::syscall2(libc::sys::SYS_DEBUG_LOG, s.as_ptr() as u64, s.len() as u64) };
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
    let _ =
        unsafe { libc::sys::syscall2(libc::sys::SYS_DEBUG_LOG, msg.as_ptr() as u64, pos as u64) };
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
        msg[pos] = if b.is_ascii_graphic() || b == b' ' {
            b
        } else {
            b'?'
        };
        pos += 1;
    }
    let _ =
        unsafe { libc::sys::syscall2(libc::sys::SYS_DEBUG_LOG, msg.as_ptr() as u64, pos as u64) };
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

fn print_i64(v: i64) {
    if v < 0 {
        let _ = write_all(b"-");
        print_u64(v.unsigned_abs());
    } else {
        print_u64(v as u64);
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

fn parse_i64(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix('-') {
        let value = parse_u64(rest)?;
        if value > i64::MAX as u64 {
            return None;
        }
        Some(-(value as i64))
    } else {
        let value = parse_u64(s)?;
        if value > i64::MAX as u64 {
            return None;
        }
        Some(value as i64)
    }
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
