#![no_std]
#![no_main]

use sunlight_ipc::{
    ipc_call, nameserver_lookup, IpcMsg, SessionComponentRole, SessionComponentState, SessionKind,
    SessionMsg, SessionState, SESSION_ENDPOINT,
};

fn stdout_write(text: &str) {
    let _ = sunlight_libc::write(sunlight_libc::STDOUT, text.as_bytes());
}

macro_rules! println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut line = heapless::String::<256>::new();
        let _ = write!(&mut line, $($arg)*);
        stdout_write(&line);
        stdout_write("\n");
    }};
}

unsafe fn arg<'a>(argc: u64, argv: *const *const u8, index: usize) -> &'a str {
    if index >= argc as usize || argv.is_null() {
        return "";
    }
    let pointer = *argv.add(index);
    if pointer.is_null() {
        return "";
    }
    let mut length = 0usize;
    while length < 128 && *pointer.add(length) != 0 {
        length += 1;
    }
    core::str::from_utf8(core::slice::from_raw_parts(pointer, length)).unwrap_or("")
}

fn lookup_uid_name(uid: u32) -> heapless::String<32> {
    let mut out = heapless::String::<32>::new();
    if let Ok(fd) = sunlight_libc::open(b"/etc/passwd") {
        let mut buf = [0u8; 1024];
        if let Ok(read) = sunlight_libc::read(fd, &mut buf) {
            if let Ok(text) = core::str::from_utf8(&buf[..read]) {
                for line in text.lines() {
                    let mut parts = line.split(':');
                    let Some(name) = parts.next() else { continue };
                    let _ = parts.next();
                    let Some(uid_part) = parts.next() else { continue };
                    if uid_part.parse::<u32>().ok() == Some(uid) {
                        let _ = out.push_str(name);
                        let _ = sunlight_libc::close(fd);
                        return out;
                    }
                }
            }
        }
        let _ = sunlight_libc::close(fd);
    }
    use core::fmt::Write;
    let _ = write!(&mut out, "uid:{}", uid);
    out
}

fn session_kind(raw: u64) -> &'static str {
    match SessionKind::from_u64(raw) {
        Some(SessionKind::Desktop) => "desktop",
        Some(SessionKind::SafeDesktop) => "safe-desktop",
        None => "unknown",
    }
}

fn session_state(raw: u64) -> &'static str {
    match SessionState::from_u64(raw) {
        Some(SessionState::Created) => "created",
        Some(SessionState::Preparing) => "preparing",
        Some(SessionState::StartingRequiredComponents) => "starting",
        Some(SessionState::Running) => "running",
        Some(SessionState::Degraded) => "degraded",
        Some(SessionState::Locking) => "locking",
        Some(SessionState::Locked) => "locked",
        Some(SessionState::Stopping) => "stopping",
        Some(SessionState::Stopped) => "stopped",
        Some(SessionState::Failed) => "failed",
        None => "unknown",
    }
}

fn component_role(raw: u64) -> &'static str {
    match SessionComponentRole::from_u64(raw) {
        Some(SessionComponentRole::Shell) => "shell",
        Some(SessionComponentRole::StartupApplication) => "startup-application",
        Some(SessionComponentRole::SessionService) => "session-service",
        Some(SessionComponentRole::WelcomeApplication) => "welcome-application",
        None => "unknown",
    }
}

fn component_state(raw: u64) -> &'static str {
    match SessionComponentState::from_u64(raw) {
        Some(SessionComponentState::Pending) => "pending",
        Some(SessionComponentState::Starting) => "starting",
        Some(SessionComponentState::Ready) => "ready",
        Some(SessionComponentState::Running) => "running",
        Some(SessionComponentState::RestartPending) => "restart-pending",
        Some(SessionComponentState::Stopping) => "stopping",
        Some(SessionComponentState::Exited) => "exited",
        Some(SessionComponentState::Failed) => "failed",
        Some(SessionComponentState::Disabled) => "disabled",
        None => "unknown",
    }
}

fn session_ep() -> Option<sunlight_ipc::CapabilityToken> {
    nameserver_lookup(SESSION_ENDPOINT)
}

fn list_or_status(status_only: bool) -> i32 {
    let Some(ep) = session_ep() else {
        println!("session service unavailable");
        return 1;
    };
    let reply = ipc_call(ep, IpcMsg::with_label(SessionMsg::SESSION_LIST).word(0, 0));
    if reply.label != SessionMsg::REPLY {
        println!("no sessions");
        return 1;
    }
    let uid = reply.words[2] as u32;
    let state = (reply.words[2] >> 32) & 0xff;
    let kind = (reply.words[2] >> 40) & 0xff;
    let ready = (reply.words[2] >> 48) & 0xff;
    let components = (reply.words[2] >> 56) & 0xff;
    let user = lookup_uid_name(uid);
    if status_only {
        let components_reply = ipc_call(
            ep,
            IpcMsg::with_label(SessionMsg::SESSION_GET_COMPONENTS)
                .word(0, reply.words[0])
                .word(1, reply.words[1]),
        );
        let shell_pid = if components_reply.label == SessionMsg::REPLY {
            components_reply.words[1]
        } else {
            0
        };
        println!("session_id={}", reply.words[0]);
        println!("generation={}", reply.words[1]);
        println!("user={}", user);
        println!("kind={}", session_kind(kind));
        println!("state={}", session_state(state));
        println!("required_ready={}", ready);
        println!("component_count={}", components);
        println!("shell_pid={}", shell_pid);
        return 0;
    }
    println!(
        "{} {} {} {}",
        reply.words[0],
        reply.words[1],
        user,
        session_state(state)
    );
    0
}

fn inspect(session_id: u64) -> i32 {
    let Some(ep) = session_ep() else {
        println!("session service unavailable");
        return 1;
    };
    let reply = ipc_call(
        ep,
        IpcMsg::with_label(SessionMsg::SESSION_GET)
            .word(0, session_id)
            .word(1, 0),
    );
    if reply.label != SessionMsg::REPLY {
        println!("session not found");
        return 1;
    }
    let uid = reply.words[2] as u32;
    let state = (reply.words[2] >> 32) & 0xff;
    let kind = (reply.words[2] >> 40) & 0xff;
    println!("session_id={}", reply.words[0]);
    println!("generation={}", reply.words[1]);
    println!("user={}", lookup_uid_name(uid));
    println!("kind={}", session_kind(kind));
    println!("state={}", session_state(state));
    println!("timestamp_ms={}", reply.words[3]);
    0
}

fn components(session_id: u64) -> i32 {
    let Some(ep) = session_ep() else {
        println!("session service unavailable");
        return 1;
    };
    let reply = ipc_call(
        ep,
        IpcMsg::with_label(SessionMsg::SESSION_GET_COMPONENTS).word(0, session_id),
    );
    if reply.label != SessionMsg::REPLY {
        println!("component list unavailable");
        return 1;
    }
    let packed = reply.words[3];
    println!("component_id={}", reply.words[0]);
    println!("pid={}", reply.words[1]);
    println!("generation={}", reply.words[2]);
    println!("role={}", component_role((packed >> 8) & 0xff));
    println!("state={}", component_state(packed & 0xff));
    println!("required={}", ((packed >> 16) & 1) != 0);
    println!("launch_count={}", (packed >> 24) & 0xffff);
    println!("restart_count={}", (packed >> 40) & 0xffff);
    println!("last_exit_reason={}", (packed >> 56) & 0xff);
    0
}

fn session_action(session_id: u64, action: u64) -> i32 {
    let Some(ep) = session_ep() else {
        println!("session service unavailable");
        return 1;
    };
    let get = ipc_call(
        ep,
        IpcMsg::with_label(SessionMsg::SESSION_GET)
            .word(0, session_id)
            .word(1, 0),
    );
    if get.label != SessionMsg::REPLY {
        println!("session not found");
        return 1;
    }
    let reply = ipc_call(
        ep,
        IpcMsg::with_label(SessionMsg::SESSION_ACTION)
            .word(0, session_id)
            .word(1, get.words[1])
            .word(2, action),
    );
    if reply.label != SessionMsg::REPLY {
        println!("action failed");
        return 1;
    }
    println!("ok");
    0
}

fn health() -> i32 {
    let Some(ep) = session_ep() else {
        println!("session service unavailable");
        return 1;
    };
    let reply = ipc_call(ep, IpcMsg::with_label(SessionMsg::SESSION_GET_HEALTH));
    if reply.label != SessionMsg::REPLY {
        println!("health unavailable");
        return 1;
    }
    println!("manifest_ok={}", (reply.words[0] & 1) != 0);
    println!("active_sessions={}", (reply.words[0] >> 8) & 0xff);
    println!("active_components={}", (reply.words[0] >> 16) & 0xff);
    println!("sessions_created={}", reply.words[1] as u32);
    println!("sessions_started={}", (reply.words[1] >> 32) as u32);
    println!("sessions_failed={}", reply.words[2] as u32);
    println!("sessions_stopped={}", (reply.words[2] >> 32) as u32);
    println!("components_launched={}", reply.words[3] as u32);
    println!("component_restarts={}", (reply.words[3] >> 32) as u32);
    0
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let cmd = unsafe { arg(argc, argv, 1) };
    let target = unsafe { arg(argc, argv, 2) };
    let status = match cmd {
        "status" => list_or_status(true),
        "list" => list_or_status(false),
        "inspect" => inspect(target.parse::<u64>().unwrap_or(0)),
        "components" => components(target.parse::<u64>().unwrap_or(0)),
        "restart-shell" => session_action(target.parse::<u64>().unwrap_or(0), 4),
        "logout" => session_action(target.parse::<u64>().unwrap_or(0), 3),
        "health" => health(),
        _ => {
            println!("usage: sunlight-sessionctl status|list|inspect <id>|components <id>|restart-shell <id>|logout <id>|health");
            2
        }
    };
    sunlight_libc::exit(status as u64);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sunlight_libc::exit(1)
}
