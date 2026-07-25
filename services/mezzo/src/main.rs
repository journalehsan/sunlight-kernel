#![no_std]
#![no_main]

use mezzo::LockSession;
use sunlight_ipc::{
    consume_lock_auth_grant, endpoint_create, ipc_call_timeout, ipc_reply_and_try_recv,
    monotonic_millis, nameserver_lookup_timeout, nameserver_register, process_is_alive,
    process_yield, validate_lock_caller, CapabilityToken, IpcMsg, LockState, MezzoMsg, SgpMsg,
    LOCK_CALLER_AUTHENTICATED_TTY, LOCK_CALLER_TTY_SERVICE, LOCK_SESSION_USERNAME_MAX,
};

const SUNLIGHTD_START: u64 = 1;
const SUNLIGHTD_RESTART: u64 = 3;
const SUNLIGHTD_STATUS: u64 = 10;
const SUNLIGHTD_OK: u64 = 1;
const PRESENTER_UNIT: &str = "vortex-lock-presenter";
const AUTO_RESTART_LIMIT: u32 = 2;
const CONTROL_TIMEOUT_MS: u64 = 1_000;
const PRESENTER_START_TIMEOUT_MS: u64 = 3_000;
const PRESENTER_TRANSITION_TIMEOUT_MS: u64 = 5_000;
const AUTO_RESTART_BACKOFF_MS: u64 = 500;

fn pack_name(message: &mut IpcMsg, name: &str) {
    for (index, byte) in name.bytes().take(32).enumerate() {
        message.words[index / 8] |= (byte as u64) << ((index % 8) * 8);
    }
    message.word_count = 4;
}

fn sunlightd_request(operation: u64) -> Option<IpcMsg> {
    let cap = nameserver_lookup_timeout("sunlightd", CONTROL_TIMEOUT_MS)?;
    let mut message = IpcMsg::with_label(operation);
    pack_name(&mut message, PRESENTER_UNIT);
    ipc_call_timeout(cap, message, CONTROL_TIMEOUT_MS).ok()
}

fn presenter_pid() -> Option<u64> {
    let reply = sunlightd_request(SUNLIGHTD_STATUS)?;
    if reply.label != SUNLIGHTD_OK {
        return None;
    }
    let state = reply.words[0] & 0xff;
    let pid = matches!(state, 1 | 2 | 5).then_some(reply.words[1])?;
    (pid != 0 && process_is_alive(pid)).then_some(pid)
}

fn start_presenter(restart: bool) -> Option<u64> {
    let operation = if restart {
        SUNLIGHTD_RESTART
    } else {
        SUNLIGHTD_START
    };
    let reply = sunlightd_request(operation)?;
    if reply.label != SUNLIGHTD_OK && reply.label != 2 {
        return None;
    }
    let deadline = monotonic_millis().saturating_add(PRESENTER_START_TIMEOUT_MS);
    while monotonic_millis() < deadline {
        if let Some(pid) = presenter_pid() {
            return Some(pid);
        }
        process_yield();
    }
    None
}

fn display_enter(generation: u64) -> Option<CapabilityToken> {
    let display = nameserver_lookup_timeout("display_server", CONTROL_TIMEOUT_MS)?;
    let reply = ipc_call_timeout(
        display,
        IpcMsg::with_label(SgpMsg::LOCK_ENTER).word(0, generation),
        CONTROL_TIMEOUT_MS,
    )
    .ok()?;
    (reply.label == SgpMsg::REPLY && reply.words[0] == generation).then_some(reply.caps[0])
}

fn display_presenter(
    operation: u64,
    generation: u64,
    window_id: u64,
    authority: CapabilityToken,
) -> bool {
    let Some(display) = nameserver_lookup_timeout("display_server", CONTROL_TIMEOUT_MS) else {
        return false;
    };
    ipc_call_timeout(
        display,
        IpcMsg::with_label(operation)
            .word(0, generation)
            .word(1, window_id)
            .with_cap(0, authority),
        CONTROL_TIMEOUT_MS,
    )
    .is_ok_and(|reply| reply.label == SgpMsg::REPLY)
}

fn unpack_username(message: &IpcMsg) -> Option<[u8; LOCK_SESSION_USERNAME_MAX]> {
    let mut username = [0u8; LOCK_SESSION_USERNAME_MAX];
    for word in 0..4 {
        username[word * 8..word * 8 + 8].copy_from_slice(&message.words[word].to_le_bytes());
    }
    let len = username
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(username.len());
    (len != 0).then_some(username)
}

/// Pack PRESENTER_HELLO into the four register-IPC words only.
///
/// Layout (register IPC drops words[4..]):
/// - word0: generation | (safe_mode << 63)
/// - word1: session_uid | (session_gid << 32)
/// - word2: username bytes [0..8]
/// - word3: username bytes [8..16]
///
/// Display name is truncated to 16 bytes; unlock auth uses uid/gid.
fn presenter_hello_reply(session: &LockSession, authority: CapabilityToken) -> IpcMsg {
    let mut name0 = [0u8; 8];
    let mut name1 = [0u8; 8];
    let copy_len = session.session_username_len.min(16);
    if copy_len > 0 {
        let first = copy_len.min(8);
        name0[..first].copy_from_slice(&session.session_username[..first]);
        if copy_len > 8 {
            name1[..copy_len - 8].copy_from_slice(&session.session_username[8..copy_len]);
        }
    }
    IpcMsg::with_label(MezzoMsg::REPLY)
        .word(
            0,
            session.generation | ((session.safe_mode as u64) << 63),
        )
        .word(
            1,
            (session.session_uid as u64) | ((session.session_gid as u64) << 32),
        )
        .word(2, u64::from_le_bytes(name0))
        .word(3, u64::from_le_bytes(name1))
        .with_cap(0, authority)
}

/// Status fits in four register words:
/// word0 = state | (recovery_attempts << 8) | (safe_mode << 16) | (last_failure_lo << 32)
/// word1 = generation
/// word2 = presenter_pid
/// word3 = presenter_generation | (last_presenter_failure << 32) when pid fits u32
fn status_reply(session: &LockSession) -> IpcMsg {
    let failure = session.last_presenter_failure.min(u32::MAX as u64) as u32;
    IpcMsg::with_label(MezzoMsg::REPLY)
        .word(
            0,
            (session.state as u64)
                | ((session.recovery_attempts as u64) << 8)
                | ((session.safe_mode as u64) << 16)
                | ((failure as u64) << 32),
        )
        .word(1, session.generation)
        .word(2, session.presenter_pid)
        .word(3, session.presenter_generation)
}

fn error(code: u64) -> IpcMsg {
    IpcMsg::with_label(MezzoMsg::ERROR).word(0, code)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let endpoint = endpoint_create();
    nameserver_register("mezzo", endpoint);
    let mut session = LockSession::new();
    let mut display_authority = CapabilityToken::INVALID;
    let mut expected_presenter_pid = 0u64;
    let mut last_auto_restart_ms = 0u64;
    let mut reply = IpcMsg::empty();

    loop {
        let now = monotonic_millis();
        if session.presenter_pid != 0 && !process_is_alive(session.presenter_pid) {
            let failed_pid = session.presenter_pid;
            if session.presenter_failed(failed_pid) {
                sunlight_ipc::debug_log(
                    "[MEZZO] presenter failed; locked fallback remains active\n",
                );
            }
        }
        if session.transition_expired(now, expected_presenter_pid) {
            if let Some(authority) = display_enter(session.generation) {
                display_authority = authority;
            }
            expected_presenter_pid = 0;
            last_auto_restart_ms = now;
            sunlight_ipc::debug_log("[MEZZO] presenter transition timed out; fallback active\n");
        }
        if session.state == LockState::LockedFallback
            && session.recovery_attempts < AUTO_RESTART_LIMIT
            && now
                >= last_auto_restart_ms.saturating_add(
                    AUTO_RESTART_BACKOFF_MS.saturating_mul(session.recovery_attempts as u64 + 1),
                )
        {
            session.begin_recovery(session.safe_mode, now, PRESENTER_TRANSITION_TIMEOUT_MS);
            if let Some(authority) = display_enter(session.generation) {
                display_authority = authority;
                expected_presenter_pid = start_presenter(true).unwrap_or(0);
            } else {
                expected_presenter_pid = 0;
            }
            last_auto_restart_ms = now;
            if expected_presenter_pid == 0 {
                session.fallback(0);
            }
        }

        let Some(message) = ipc_reply_and_try_recv(endpoint, reply) else {
            reply = IpcMsg::empty();
            process_yield();
            continue;
        };
        reply = match message.label {
            MezzoMsg::SESSION_ESTABLISH => {
                if !validate_lock_caller(message.badge, LOCK_CALLER_TTY_SERVICE) {
                    error(MezzoMsg::ERR_UNAUTHORIZED)
                } else if let Some(username) = unpack_username(&message) {
                    let username_len = username
                        .iter()
                        .position(|byte| *byte == 0)
                        .unwrap_or(username.len());
                    if let Some((uid, gid)) =
                        consume_lock_auth_grant(message.caps[0], message.badge)
                    {
                        if session.establish_session(uid, gid, &username[..username_len]) {
                            status_reply(&session)
                        } else {
                            error(MezzoMsg::ERR_BUSY)
                        }
                    } else {
                        error(MezzoMsg::ERR_UNAUTHORIZED)
                    }
                } else {
                    error(MezzoMsg::ERR_UNAUTHORIZED)
                }
            }
            MezzoMsg::LOCK_ACTIVATE => {
                sunlight_ipc::debug_log("[MEZZO] LOCK_ACTIVATE\n");
                match session.enter(monotonic_millis(), PRESENTER_TRANSITION_TIMEOUT_MS) {
                    Some(generation) => {
                        if let Some(authority) = display_enter(generation) {
                            display_authority = authority;
                            expected_presenter_pid = start_presenter(false).unwrap_or(0);
                            if expected_presenter_pid == 0 {
                                sunlight_ipc::debug_log(
                                    "[MEZZO] presenter start failed; locked fallback\n",
                                );
                                session.fallback(0);
                            } else {
                                sunlight_ipc::debug_log("[MEZZO] presenter start requested\n");
                            }
                            status_reply(&session)
                        } else {
                            sunlight_ipc::debug_log(
                                "[MEZZO] display LOCK_ENTER failed; remaining unlocked\n",
                            );
                            session.state = LockState::Unlocked;
                            error(MezzoMsg::ERR_START_FAILED)
                        }
                    }
                    None if session.state == LockState::Unlocked => {
                        sunlight_ipc::debug_log("[MEZZO] LOCK_ACTIVATE: no session established\n");
                        error(MezzoMsg::ERR_NO_SESSION)
                    }
                    None => status_reply(&session),
                }
            }
            MezzoMsg::LOCK_STATUS => status_reply(&session),
            MezzoMsg::LOCK_RECOVER => {
                if !validate_lock_caller(message.badge, LOCK_CALLER_AUTHENTICATED_TTY) {
                    error(MezzoMsg::ERR_UNAUTHORIZED)
                } else if session.state == LockState::Unlocked {
                    error(MezzoMsg::ERR_NOT_LOCKED)
                } else {
                    let safe = message.words[0] & MezzoMsg::RECOVER_SAFE != 0;
                    session.begin_recovery(
                        safe,
                        monotonic_millis(),
                        PRESENTER_TRANSITION_TIMEOUT_MS,
                    );
                    expected_presenter_pid =
                        if let Some(authority) = display_enter(session.generation) {
                            display_authority = authority;
                            start_presenter(true).unwrap_or(0)
                        } else {
                            0
                        };
                    if expected_presenter_pid == 0 {
                        session.fallback(0);
                        error(MezzoMsg::ERR_START_FAILED)
                    } else {
                        status_reply(&session)
                    }
                }
            }
            MezzoMsg::PRESENTER_HELLO => {
                let pid = message.badge;
                if pid != expected_presenter_pid
                    || !session.register_presenter(session.generation, pid)
                {
                    error(MezzoMsg::ERR_STALE)
                } else {
                    presenter_hello_reply(&session, display_authority)
                }
            }
            MezzoMsg::PRESENTER_READY => {
                let generation = message.words[0];
                if !session.presenter_ready(generation, message.badge) {
                    error(MezzoMsg::ERR_STALE)
                } else {
                    status_reply(&session)
                }
            }
            MezzoMsg::AUTHENTICATE => {
                let generation = message.words[0];
                let pid = message.badge;
                if !session.begin_authentication(generation, pid) {
                    error(MezzoMsg::ERR_STALE)
                } else if let Some((uid, gid)) = consume_lock_auth_grant(message.caps[0], pid) {
                    if !session.authentication_identity_matches(uid, gid) {
                        session.authentication_failed(generation, pid);
                        error(MezzoMsg::ERR_UNAUTHORIZED)
                    } else if session.leave(generation, pid)
                        && display_presenter(SgpMsg::LOCK_LEAVE, generation, 0, display_authority)
                    {
                        session.finish_leave();
                        display_authority = CapabilityToken::INVALID;
                        expected_presenter_pid = 0;
                        status_reply(&session)
                    } else {
                        if let Some(authority) = display_enter(generation) {
                            display_authority = authority;
                        }
                        session.fallback(pid);
                        error(MezzoMsg::ERR_BUSY)
                    }
                } else {
                    session.authentication_failed(generation, pid);
                    error(MezzoMsg::ERR_UNAUTHORIZED)
                }
            }
            _ => error(MezzoMsg::ERR_UNAUTHORIZED),
        };
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        process_yield();
    }
}
