//! sunlightctl - Control interface for sunlightd

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 16384] = [0; 16384];
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

use sunlight_ipc::{ipc_call, nameserver_lookup, CapabilityToken, IpcMsg};

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

// ── IPC opcodes (must match sunlightd/src/ipc.rs) ────────────────────────────
const OP_START: u64 = 1;
const OP_STOP: u64 = 2;
const OP_RESTART: u64 = 3;
const OP_ENABLE: u64 = 5;
const OP_DISABLE: u64 = 6;
const OP_STATUS: u64 = 10;
const OP_LIST: u64 = 11;

const REPLY_OK: u64 = 1;
const REPLY_NOP: u64 = 2;
const REPLY_TIMEOUT: u64 = 3;

// Detail kinds (must match sunlightd/src/supervisor.rs)
const DETAIL_NONE: u32 = 0;
const DETAIL_SPAWN: u32 = 1;
const DETAIL_IDENTITY: u32 = 2;
const DETAIL_STARTUP: u32 = 3;
const DETAIL_RESTART_LIMIT: u32 = 4;
const DETAIL_NOT_FOUND: u32 = 5;
const DETAIL_ALREADY_RUNNING: u32 = 6;
const DETAIL_ALREADY_STOPPED: u32 = 7;
const DETAIL_STOP_TIMEOUT: u32 = 8;
const DETAIL_EXEC_NOT_FOUND: u32 = 9;
const DETAIL_EXEC_DENIED: u32 = 10;
const DETAIL_EXEC_LOAD: u32 = 11;
const DETAIL_SPAWN_NOMEM: u32 = 12;
const DETAIL_EXITED: u32 = 13;
const DETAIL_IN_PROGRESS: u32 = 14;
const DETAIL_TRANSITION_BUSY: u32 = 15;
const DETAIL_RESTART_ABORTED: u32 = 16;
const DETAIL_KILL_FAILED: u32 = 17;
const DETAIL_TERMINATION_UNCONFIRMED: u32 = 18;

const STATE_STOPPED: u32 = 0;
const STATE_STARTING: u32 = 1;
const STATE_RUNNING: u32 = 2;
const STATE_FAILED: u32 = 3;
const STATE_RESTARTING: u32 = 4;
const STATE_STOPPING: u32 = 5;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn pack_unit_name(msg: &mut IpcMsg, name: &str) {
    let bytes = name.as_bytes();
    for i in 0..4 {
        let mut word: u64 = 0;
        for j in 0..8 {
            let idx = i * 8 + j;
            if idx < bytes.len() {
                word |= (bytes[idx] as u64) << (j * 8);
            }
        }
        msg.words[i] = word;
    }
}

fn state_str(state: u32) -> &'static str {
    match state {
        STATE_STOPPED => "stopped",
        STATE_STARTING => "starting",
        STATE_RUNNING => "running",
        STATE_FAILED => "failed",
        STATE_RESTARTING => "restarting",
        STATE_STOPPING => "stopping",
        _ => "unknown",
    }
}

fn last_op_str(op: u8) -> &'static str {
    match op {
        1 => "start",
        2 => "stop",
        3 => "restart",
        _ => "none",
    }
}

fn detail_str(kind: u32) -> &'static str {
    match kind {
        DETAIL_NONE => "none",
        DETAIL_SPAWN => "spawn-failed",
        DETAIL_IDENTITY => "identity-resolve-failed",
        DETAIL_STARTUP => "startup-failed",
        DETAIL_RESTART_LIMIT => "restart-limit",
        DETAIL_NOT_FOUND => "not-found",
        DETAIL_ALREADY_RUNNING => "already-running",
        DETAIL_ALREADY_STOPPED => "already-stopped",
        DETAIL_STOP_TIMEOUT => "stop-timeout",
        DETAIL_EXEC_NOT_FOUND => "executable-not-found",
        DETAIL_EXEC_DENIED => "permission-denied",
        DETAIL_EXEC_LOAD => "exec-load-failed",
        DETAIL_SPAWN_NOMEM => "spawn-out-of-memory",
        DETAIL_EXITED => "process-exited",
        DETAIL_IN_PROGRESS => "in-progress",
        DETAIL_TRANSITION_BUSY => "transition-busy",
        DETAIL_RESTART_ABORTED => "restart-aborted",
        DETAIL_KILL_FAILED => "kill-failed",
        DETAIL_TERMINATION_UNCONFIRMED => "termination-unconfirmed",
        _ => "unknown",
    }
}

fn print_next_action_for_timeout(unit: &str, pid: u32) {
    println!("   Termination is unconfirmed; service was NOT marked stopped.");
    println!(
        "   Next: sunlightctl status {}  (inspect state)",
        unit
    );
    if pid != 0 {
        println!(
            "   Next: kill -9 {}  (force, after kill-semantics audit)",
            pid
        );
    }
    println!(
        "   Next: sunlightctl restart {}  (only after process is confirmed dead)",
        unit
    );
}

// ── Commands ──────────────────────────────────────────────────────────────────

fn cmd_list(cap: CapabilityToken) {
    println!("{:<18} {:<10} {:<10} {}", "NAME", "STATE", "ENABLED", "PID");

    let mut idx: u64 = 0;
    loop {
        let mut msg = IpcMsg::empty();
        msg.label = OP_LIST;
        msg.words[0] = idx;

        let reply = ipc_call(cap, msg);
        if reply.label != REPLY_OK {
            break;
        }

        // Decode words[0..4] (transport-safe encoding set by sunlightd ListEntry::pack)
        let total = (reply.words[0] & 0xFFFF_FFFF) as usize;
        let state = ((reply.words[0] >> 32) & 0xFF) as u32;
        let enabled = ((reply.words[0] >> 40) & 0x01) != 0;
        let _restarts = ((reply.words[0] >> 48) & 0xFF) as u32;
        let pid = reply.words[1] as u32;

        // Name packed into words[2..4] (16 bytes)
        let mut name = heapless::String::<32>::new();
        for &word in &[reply.words[2], reply.words[3]] {
            for j in 0..8u64 {
                let byte = ((word >> (j * 8)) & 0xFF) as u8;
                if byte == 0 {
                    break;
                }
                let _ = name.push(byte as char);
            }
            if name.len() > 0 && name.as_bytes()[name.len() - 1] == 0 {
                break;
            }
        }

        let enabled_s = if enabled { "enabled" } else { "disabled" };

        let mut pid_s = heapless::String::<16>::new();
        if state == STATE_RUNNING
            || state == STATE_STARTING
            || state == STATE_STOPPING
        {
            use core::fmt::Write;
            let _ = write!(&mut pid_s, "{}", pid);
        } else {
            let _ = pid_s.push('-');
        }

        println!(
            "{:<18} {:<10} {:<10} {}",
            name,
            state_str(state),
            enabled_s,
            pid_s
        );

        idx += 1;
        if total == 0 || idx >= total as u64 {
            break;
        }
    }
}

fn cmd_status(cap: CapabilityToken, unit: &str) {
    let mut msg = IpcMsg::empty();
    msg.label = OP_STATUS;
    pack_unit_name(&mut msg, unit);

    let reply = ipc_call(cap, msg);
    if reply.label != REPLY_OK {
        println!(
            "ERROR: Service '{}' not found or sunlightd unavailable",
            unit
        );
        return;
    }

    // New 4-word status packing (register-IPC safe).
    let state = (reply.words[0] & 0xff) as u32;
    let detail_kind = ((reply.words[0] >> 8) & 0xff) as u32;
    let restarts = ((reply.words[0] >> 16) & 0xffff) as u32;
    let enabled = ((reply.words[0] >> 32) & 1) != 0;
    let unconfirmed = ((reply.words[0] >> 33) & 1) != 0;
    let last_op = ((reply.words[0] >> 40) & 0xff) as u8;
    let stop_timed_out = ((reply.words[0] >> 48) & 1) != 0;
    let pid = reply.words[1] as u32;
    let since = reply.words[2];
    let detail_value = reply.words[3] as u32;

    let healthy = state == STATE_RUNNING && detail_kind == DETAIL_NONE && !unconfirmed;

    println!("● {}.service", unit);
    println!("   Active:   {}", state_str(state));
    println!(
        "   Enabled:  {}",
        if enabled { "enabled" } else { "disabled" }
    );
    if state == STATE_STARTING || state == STATE_RUNNING || state == STATE_STOPPING {
        println!("   PID:      {}", pid);
    }
    if since != 0 {
        println!("   SinceMs:  {}", since);
    }
    if !healthy {
        println!("   LastOp:   {}", last_op_str(last_op));
        if detail_kind != DETAIL_NONE {
            println!("   Result:   {}", detail_str(detail_kind));
        }
        if detail_value != 0 || state == STATE_FAILED {
            println!("   Detail:   {}", detail_value);
        }
        if unconfirmed {
            println!("   Term:     requested, not confirmed");
        }
        if stop_timed_out {
            println!("   Timeout:  stop deadline expired");
            print_next_action_for_timeout(unit, pid);
        }
    }
    println!("   Restarts: {}", restarts);
}

fn decode_control_kind(reply: &IpcMsg) -> (u32, u32, bool) {
    let kind = reply.words[0] as u32;
    let pid = reply.words[1] as u32;
    let unconfirmed = reply.words[3] != 0;
    (kind, pid, unconfirmed)
}

fn cmd_start(cap: CapabilityToken, unit: &str) {
    let mut msg = IpcMsg::empty();
    msg.label = OP_START;
    pack_unit_name(&mut msg, unit);
    let reply = ipc_call(cap, msg);
    let (kind, pid, _) = decode_control_kind(&reply);
    match reply.label {
        REPLY_OK => {
            if pid != 0 {
                println!("Started {}.service (pid={})", unit, pid);
            } else {
                println!("Started {}.service", unit);
            }
        }
        REPLY_NOP if kind == DETAIL_ALREADY_RUNNING => {
            println!("{}.service is already running (pid={})", unit, pid);
        }
        _ => {
            println!(
                "ERROR: Failed to start '{}': {}",
                unit,
                detail_str(kind)
            );
        }
    }
}

fn cmd_stop(cap: CapabilityToken, unit: &str) {
    let mut msg = IpcMsg::empty();
    msg.label = OP_STOP;
    pack_unit_name(&mut msg, unit);
    let reply = ipc_call(cap, msg);
    let (kind, pid, unconfirmed) = decode_control_kind(&reply);
    match reply.label {
        REPLY_OK => {
            println!("Stopped {}.service", unit);
        }
        REPLY_NOP if kind == DETAIL_ALREADY_STOPPED => {
            println!("{}.service is already stopped", unit);
        }
        REPLY_TIMEOUT => {
            println!(
                "ERROR: Stop timed out for '{}' ({})",
                unit,
                detail_str(kind)
            );
            if unconfirmed {
                print_next_action_for_timeout(unit, pid);
            }
        }
        _ => {
            println!(
                "ERROR: Failed to stop '{}': {}",
                unit,
                detail_str(kind)
            );
        }
    }
}

fn cmd_restart(cap: CapabilityToken, unit: &str) {
    let mut msg = IpcMsg::empty();
    msg.label = OP_RESTART;
    pack_unit_name(&mut msg, unit);
    let reply = ipc_call(cap, msg);
    let (kind, pid, unconfirmed) = decode_control_kind(&reply);
    match reply.label {
        REPLY_OK => {
            if pid != 0 {
                println!("Restarted {}.service (pid={})", unit, pid);
            } else {
                println!("Restarted {}.service", unit);
            }
        }
        REPLY_TIMEOUT => {
            println!(
                "ERROR: Restart aborted for '{}': {} (old instance not confirmed dead)",
                unit,
                detail_str(kind)
            );
            if unconfirmed {
                print_next_action_for_timeout(unit, pid);
            }
        }
        _ => {
            println!(
                "ERROR: Failed to restart '{}': {}",
                unit,
                detail_str(kind)
            );
        }
    }
}

fn cmd_enable(cap: CapabilityToken, unit: &str, now: bool) {
    if unit.is_empty() {
        println!("ERROR: missing service name");
        return;
    }
    let mut msg = IpcMsg::empty();
    msg.label = OP_ENABLE;
    pack_unit_name(&mut msg, unit);
    let reply = ipc_call(cap, msg);
    match reply.label {
        REPLY_OK => println!("Enabled {}.service", unit),
        REPLY_NOP => println!("{}.service is already enabled", unit),
        _ => {
            println!("ERROR: Service '{}' not found", unit);
            return;
        }
    }
    if now {
        cmd_start(cap, unit);
    }
}

fn cmd_disable(cap: CapabilityToken, unit: &str, now: bool) {
    if unit.is_empty() {
        println!("ERROR: missing service name");
        return;
    }
    let mut msg = IpcMsg::empty();
    msg.label = OP_DISABLE;
    pack_unit_name(&mut msg, unit);
    let reply = ipc_call(cap, msg);
    match reply.label {
        REPLY_OK => println!("Disabled {}.service", unit),
        REPLY_NOP => println!("{}.service is already disabled", unit),
        _ => {
            println!("ERROR: Service '{}' not found", unit);
            return;
        }
    }
    if now {
        cmd_stop(cap, unit);
    }
}

fn print_usage() {
    println!("Usage: sunlightctl <command> [options]");
    println!("Commands:");
    println!("  list                       List all managed services");
    println!("  status <service>           Show detailed service status");
    println!("  start <service>            Start a service (even if disabled)");
    println!("  stop <service>             Stop a running service (bounded wait)");
    println!("  restart <service>          Stop then start a service");
    println!("  reboot <service>           Alias for restart");
    println!("  enable [--now] <service>   Mark service enabled for auto-start");
    println!("  disable [--now] <service>  Mark service disabled (no auto-start)");
    println!("Options:");
    println!("  --now  With enable/disable: also start/stop the service immediately");
    println!("Examples:");
    println!("  sunlightctl start solar");
    println!("  sunlightctl enable --now solar");
    println!("  sunlightctl disable --now solar");
}

// ── Argument parsing ──────────────────────────────────────────────────────────

/// Collect argv strings from kernel-supplied argc/argv.
///
/// # Safety
/// argc and argv must be the values provided by the kernel at process entry.
unsafe fn collect_args<'a>(argc: u64, argv: *const *const u8, out: &mut [&'a str]) -> usize {
    if argv.is_null() {
        return 0;
    }
    let count = (argc as usize).min(out.len());
    for i in 0..count {
        let ptr = *argv.add(i);
        if ptr.is_null() {
            return i;
        }
        let mut len = 0usize;
        while len < 256 && *ptr.add(len) != 0 {
            len += 1;
        }
        if let Ok(s) = core::str::from_utf8(core::slice::from_raw_parts(ptr, len)) {
            out[i] = s;
        } else {
            return i;
        }
    }
    count
}

/// Parse `[--now] <service>` or `<service> [--now]` from a slice of args.
/// Returns `(now_flag, service_name)`.
fn parse_now_service<'a>(args: &[&'a str]) -> (bool, &'a str) {
    match args {
        ["--now", svc] => (true, svc),
        [svc, "--now"] => (true, svc),
        [svc] => (false, svc),
        _ => (false, ""),
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut raw: [&str; 8] = [""; 8];
    let count = unsafe { collect_args(argc, argv, &mut raw) };
    let args = &raw[..count];

    // args[0] is the binary name; commands start at args[1]
    let cmd = if args.len() >= 2 { args[1] } else { "" };

    if cmd == "--help" || cmd == "-h" || cmd == "help" || cmd.is_empty() {
        print_usage();
        sunlight_libc::exit(0);
    }

    let cap = nameserver_lookup("sunlightd");
    let Some(cap) = cap else {
        println!("ERROR: sunlightd not found (is it running?)");
        sunlight_libc::exit(1);
    };

    match cmd {
        "list" => cmd_list(cap),
        "status" => {
            let unit = args.get(2).copied().unwrap_or("");
            if unit.is_empty() {
                println!("ERROR: sunlightctl status <service>");
            } else {
                cmd_status(cap, unit);
            }
        }
        "start" => {
            let unit = args.get(2).copied().unwrap_or("");
            if unit.is_empty() {
                println!("ERROR: sunlightctl start <service>");
            } else {
                cmd_start(cap, unit);
            }
        }
        "stop" => {
            let unit = args.get(2).copied().unwrap_or("");
            if unit.is_empty() {
                println!("ERROR: sunlightctl stop <service>");
            } else {
                cmd_stop(cap, unit);
            }
        }
        "restart" | "reboot" => {
            let unit = args.get(2).copied().unwrap_or("");
            if unit.is_empty() {
                println!("ERROR: sunlightctl restart <service>");
            } else {
                cmd_restart(cap, unit);
            }
        }
        "enable" => {
            let rest = if args.len() > 2 { &args[2..] } else { &[] };
            let (now, unit) = parse_now_service(rest);
            cmd_enable(cap, unit, now);
        }
        "disable" => {
            let rest = if args.len() > 2 { &args[2..] } else { &[] };
            let (now, unit) = parse_now_service(rest);
            cmd_disable(cap, unit, now);
        }
        _ => {
            println!("Unknown command: '{}'", cmd);
            print_usage();
            sunlight_libc::exit(1);
        }
    }

    sunlight_libc::exit(0);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    println!("sunlightctl: panic: {}", _info);
    sunlight_libc::exit(1);
}
