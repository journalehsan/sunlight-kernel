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

fn session_ep() -> Option<sunlight_ipc::CapabilityToken> {
    nameserver_lookup(SESSION_ENDPOINT)
}

fn pack_app_id(msg: &mut IpcMsg, app_id: &str) {
    let bytes = app_id.as_bytes();
    let len = bytes.len().min(16);
    msg.words[2] = 0;
    msg.words[3] = 0;
    for (i, b) in bytes.iter().take(len).enumerate() {
        if i < 8 {
            msg.words[2] |= (*b as u64) << (i * 8);
        } else {
            msg.words[3] |= (*b as u64) << ((i - 8) * 8);
        }
    }
}

fn unpack_app_id(msg: &IpcMsg, len: usize) -> heapless::String<32> {
    let mut out = heapless::String::new();
    let len = len.min(16);
    for i in 0..len {
        let b = if i < 8 {
            ((msg.words[2] >> (i * 8)) & 0xff) as u8
        } else {
            ((msg.words[3] >> ((i - 8) * 8)) & 0xff) as u8
        };
        if b == 0 {
            break;
        }
        let _ = out.push(b as char);
    }
    out
}

fn policy_name(raw: u64) -> &'static str {
    match raw {
        1 => "every-login",
        2 => "first-login-only",
        3 => "first-login-after-system-upgrade",
        4 => "disabled",
        _ => "unknown",
    }
}

fn policy_from_str(s: &str) -> Option<u8> {
    match s {
        "every-login" => Some(1),
        "first-login-only" => Some(2),
        "first-login-after-system-upgrade" => Some(3),
        "disabled" => Some(4),
        _ => None,
    }
}

fn availability_name(raw: u64) -> &'static str {
    match raw {
        1 => "available",
        2 => "missing",
        3 => "invalid-manifest",
        4 => "unsupported-architecture",
        5 => "disabled-by-policy",
        6 => "incomplete-installation",
        _ => "unknown",
    }
}

fn mutate(op: u64, app_id: &str, expected_revision: u64, policy: u8, direction: u8) -> i32 {
    let Some(ep) = session_ep() else {
        println!("session service unavailable");
        return 1;
    };
    let mut msg = IpcMsg::with_label(op)
        .word(
            0,
            (app_id.len().min(16) as u64) << 32
                | ((policy as u64) << 40)
                | ((direction as u64) << 48),
        )
        .word(1, expected_revision);
    pack_app_id(&mut msg, app_id);
    let reply = ipc_call(ep, msg);
    if reply.label != SessionMsg::REPLY {
        println!("error={}", reply.words[0]);
        return 1;
    }
    println!("ok revision={}", reply.words[0]);
    0
}

fn profile_revision() -> Option<u64> {
    let ep = session_ep()?;
    let reply = ipc_call(ep, IpcMsg::with_label(SessionMsg::SESSION_PROFILE_GET).word(0, 0));
    if reply.label != SessionMsg::REPLY {
        return None;
    }
    Some(reply.words[0])
}

fn startup_list() -> i32 {
    let Some(ep) = session_ep() else {
        println!("session service unavailable");
        return 1;
    };
    let summary = ipc_call(ep, IpcMsg::with_label(SessionMsg::SESSION_PROFILE_GET).word(0, 0));
    if summary.label != SessionMsg::REPLY {
        println!("profile unavailable");
        return 1;
    }
    let revision = summary.words[0];
    let count = summary.words[2] & 0xffff;
    println!("profile_revision={}", revision);
    println!("entry_count={}", count);
    for index in 0..count {
        let reply = ipc_call(
            ep,
            IpcMsg::with_label(SessionMsg::SESSION_PROFILE_UPDATE)
                .word(0, 0)
                .word(1, index),
        );
        if reply.label != SessionMsg::REPLY {
            break;
        }
        let app_len = ((reply.words[1] >> 8) & 0xff) as usize;
        let enabled = ((reply.words[1] >> 16) & 1) != 0;
        let policy = (reply.words[1] >> 24) & 0xff;
        let order = (reply.words[1] >> 32) & 0xffff;
        let app = unpack_app_id(&reply, app_len);
        println!(
            "{} enabled={} policy={} order={}",
            app,
            enabled,
            policy_name(policy),
            order
        );
    }
    0
}

fn startup_eligible() -> i32 {
    let Some(ep) = session_ep() else {
        println!("session service unavailable");
        return 1;
    };
    for index in 0..32u64 {
        let reply = ipc_call(
            ep,
            IpcMsg::with_label(SessionMsg::SESSION_PROFILE_LIST_ELIGIBLE_APPS)
                .word(0, 0)
                .word(1, index),
        );
        if reply.label != SessionMsg::REPLY {
            if index == 0 {
                println!("(no eligible applications)");
            }
            break;
        }
        let total = reply.words[1] & 0xffff;
        let policy = (reply.words[1] >> 16) & 0xff;
        let configured = ((reply.words[1] >> 32) & 1) != 0;
        let availability = (reply.words[1] >> 40) & 0xff;
        let app_len = ((reply.words[1] >> 48) & 0xff) as usize;
        let app = unpack_app_id(&reply, app_len);
        println!(
            "{} configured={} policy={} availability={} ({}/{})",
            app,
            configured,
            policy_name(policy),
            availability_name(availability),
            index + 1,
            total
        );
        if index + 1 >= total {
            break;
        }
    }
    0
}

fn startup_preview() -> i32 {
    let Some(ep) = session_ep() else {
        println!("session service unavailable");
        return 1;
    };
    let reply = ipc_call(
        ep,
        IpcMsg::with_label(SessionMsg::SESSION_PROFILE_PREVIEW_PLAN).word(0, 0),
    );
    if reply.label != SessionMsg::REPLY {
        println!("preview unavailable");
        return 1;
    }
    println!("plan_id={}", reply.words[0]);
    println!("profile_revision={}", reply.words[1]);
    println!("component_count={}", reply.words[2] & 0xffff);
    println!("optional_count={}", (reply.words[2] >> 16) & 0xffff);
    println!("degraded={}", ((reply.words[2] >> 32) & 1) != 0);
    println!("plan_digest={:#x}", reply.words[3]);
    0
}

fn startup_status() -> i32 {
    let Some(ep) = session_ep() else {
        println!("session service unavailable");
        return 1;
    };
    let reply = ipc_call(
        ep,
        IpcMsg::with_label(SessionMsg::SESSION_PROFILE_STATUS).word(0, 0),
    );
    if reply.label != SessionMsg::REPLY {
        println!("status unavailable");
        return 1;
    }
    println!("plan_id={}", reply.words[0]);
    println!("profile_revision={}", reply.words[1]);
    println!("component_count={}", reply.words[2] & 0xffff);
    println!("optional_launched={}", (reply.words[2] >> 16) & 0xffff);
    println!("degraded={}", ((reply.words[2] >> 32) & 1) != 0);
    println!("session_state={}", (reply.words[2] >> 40) & 0xff);
    println!("plan_digest={:#x}", reply.words[3]);
    0
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

fn usage() {
    println!(
        "usage: sunlight-sessionctl status|list|inspect <id>|components <id>|restart-shell <id>|logout <id>|health"
    );
    println!(
        "       sunlight-sessionctl startup list|eligible|add <id>|remove <id>|enable <id>|disable <id>"
    );
    println!(
        "       sunlight-sessionctl startup policy <id> <policy>|move-up <id>|move-down <id>|reset|preview|status"
    );
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let cmd = unsafe { arg(argc, argv, 1) };
    let a2 = unsafe { arg(argc, argv, 2) };
    let a3 = unsafe { arg(argc, argv, 3) };
    let a4 = unsafe { arg(argc, argv, 4) };
    let status = if cmd == "startup" {
        match a2 {
            "list" => startup_list(),
            "eligible" => startup_eligible(),
            "preview" => startup_preview(),
            "status" => startup_status(),
            "reset" => match profile_revision() {
                Some(rev) => mutate(SessionMsg::SESSION_PROFILE_RESET, "", rev, 0, 0),
                None => {
                    println!("profile unavailable");
                    1
                }
            },
            "add" => match profile_revision() {
                Some(rev) => mutate(SessionMsg::SESSION_PROFILE_ADD_APP, a3, rev, 1, 0),
                None => {
                    println!("profile unavailable");
                    1
                }
            },
            "remove" => match profile_revision() {
                Some(rev) => mutate(SessionMsg::SESSION_PROFILE_REMOVE_APP, a3, rev, 0, 0),
                None => {
                    println!("profile unavailable");
                    1
                }
            },
            "enable" => match profile_revision() {
                Some(rev) => mutate(SessionMsg::SESSION_PROFILE_ENABLE_APP, a3, rev, 0, 0),
                None => {
                    println!("profile unavailable");
                    1
                }
            },
            "disable" => match profile_revision() {
                Some(rev) => mutate(SessionMsg::SESSION_PROFILE_DISABLE_APP, a3, rev, 0, 0),
                None => {
                    println!("profile unavailable");
                    1
                }
            },
            "policy" => match (profile_revision(), policy_from_str(a4)) {
                (Some(rev), Some(pol)) => {
                    mutate(SessionMsg::SESSION_PROFILE_SET_POLICY, a3, rev, pol, 0)
                }
                (None, _) => {
                    println!("profile unavailable");
                    1
                }
                (_, None) => {
                    println!("unknown policy");
                    2
                }
            },
            "move-up" => match profile_revision() {
                Some(rev) => mutate(SessionMsg::SESSION_PROFILE_REORDER, a3, rev, 0, 0),
                None => {
                    println!("profile unavailable");
                    1
                }
            },
            "move-down" => match profile_revision() {
                Some(rev) => mutate(SessionMsg::SESSION_PROFILE_REORDER, a3, rev, 0, 1),
                None => {
                    println!("profile unavailable");
                    1
                }
            },
            _ => {
                usage();
                2
            }
        }
    } else {
        match cmd {
            "status" => list_or_status(true),
            "list" => list_or_status(false),
            "inspect" => inspect(a2.parse::<u64>().unwrap_or(0)),
            "components" => components(a2.parse::<u64>().unwrap_or(0)),
            "restart-shell" => session_action(a2.parse::<u64>().unwrap_or(0), 4),
            "logout" => session_action(a2.parse::<u64>().unwrap_or(0), 3),
            "health" => health(),
            _ => {
                usage();
                2
            }
        }
    };
    sunlight_libc::exit(status as u64);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sunlight_libc::exit(1)
}
