#![no_std]
#![cfg_attr(not(test), no_main)]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_reply_and_try_recv, kill,
    monotonic_millis, nameserver_lookup, nameserver_register, process_is_alive, process_yield,
    session_consume_auth_grant, session_query_process, validate_session_caller, IpcMsg, MezzoMsg,
    ServiceCapability, SessionAction, SessionComponentRole, SessionComponentState, SessionGeneration,
    SessionId, SessionKind, SessionMsg, SessionProcessCredentials, SessionState, SpawnMsg,
    SpawnRequest, SESSION_CALLER_TTY_SERVICE, SESSION_ENDPOINT,
    SESSION_PROTOCOL_VERSION,
};
use sunlight_sessiond::{
    parse_manifest, ComponentExitReason, ManifestComponent, ManifestParseError, SessionManifest,
    SessionRecord,
};

const MANIFEST_PATH: &[u8] = b"/etc/sunlight/sessions/sunlight-desktop.toml";
const MANIFEST_MAX_BYTES: usize = 2048;
const USERNAME_MAX: usize = 12;
const COMPONENT_SHELL_ID: u64 = 1;
const LOGOUT_GRACE_MS: u64 = 3_000;

fn write_log(text: &str) {
    debug_log(text);
}

fn manifest_component<'a>(manifest: &'a SessionManifest) -> Option<&'a ManifestComponent> {
    manifest
        .components
        .iter()
        .find(|component| component.role == SessionComponentRole::Shell)
}

#[derive(Clone, Copy)]
struct SessionStats {
    sessions_created: u32,
    sessions_started: u32,
    sessions_running: u32,
    sessions_degraded: u32,
    sessions_failed: u32,
    sessions_stopped: u32,
    session_create_failures: u32,
    login_handoffs: u32,
    login_handoff_failures: u32,
    components_launched: u32,
    components_ready: u32,
    component_readiness_timeouts: u32,
    component_exits: u32,
    component_crashes: u32,
    component_restarts: u32,
    component_restart_exhaustions: u32,
    logout_requests: u32,
    logout_completions: u32,
    logout_timeouts: u32,
    stale_session_requests: u32,
    unauthorized_session_requests: u32,
}

impl SessionStats {
    const fn new() -> Self {
        Self {
            sessions_created: 0,
            sessions_started: 0,
            sessions_running: 0,
            sessions_degraded: 0,
            sessions_failed: 0,
            sessions_stopped: 0,
            session_create_failures: 0,
            login_handoffs: 0,
            login_handoff_failures: 0,
            components_launched: 0,
            components_ready: 0,
            component_readiness_timeouts: 0,
            component_exits: 0,
            component_crashes: 0,
            component_restarts: 0,
            component_restart_exhaustions: 0,
            logout_requests: 0,
            logout_completions: 0,
            logout_timeouts: 0,
            stale_session_requests: 0,
            unauthorized_session_requests: 0,
        }
    }
}

struct ActiveSession {
    record: SessionRecord,
    username: [u8; USERNAME_MAX],
    username_len: usize,
    request_id: u64,
    shell_ready_deadline_ms: u64,
    shell_stop_deadline_ms: u64,
    last_ready_at_ms: u64,
    restart_window_started_ms: u64,
}

impl ActiveSession {
    fn username(&self) -> &[u8] {
        &self.username[..self.username_len]
    }
}

struct ServiceState {
    manifest: Result<SessionManifest, ManifestParseError>,
    active: Option<ActiveSession>,
    last_closed: Option<SessionRecord>,
    next_session_id: u64,
    next_generation: u64,
    stats: SessionStats,
}

impl ServiceState {
    fn new() -> Self {
        Self {
            manifest: load_manifest(),
            active: None,
            last_closed: None,
            next_session_id: 1,
            next_generation: 1,
            stats: SessionStats::new(),
        }
    }
}

fn load_manifest() -> Result<SessionManifest, ManifestParseError> {
    let fd = sunlight_libc::open(MANIFEST_PATH).map_err(|_| ManifestParseError::MissingField)?;
    let mut bytes = [0u8; MANIFEST_MAX_BYTES];
    let read = sunlight_libc::read(fd, &mut bytes).map_err(|_| ManifestParseError::MissingField)?;
    let _ = sunlight_libc::close(fd);
    parse_manifest(&bytes[..read])
}

fn error(code: u64) -> IpcMsg {
    IpcMsg::with_label(SessionMsg::ERROR).word(0, code)
}

fn parse_username(words2: u64, words3: u64) -> ([u8; USERNAME_MAX], usize) {
    let meta = (words3 & 0xffff_ffff) as u32;
    let len = ((meta >> 24) & 0xff) as usize;
    let mut username = [0u8; USERNAME_MAX];
    let first = words2.to_le_bytes();
    let second = ((words3 >> 32) as u32).to_le_bytes();
    username[..8].copy_from_slice(&first);
    username[8..12].copy_from_slice(&second);
    (username, len.min(USERNAME_MAX))
}

fn create_reply(record: &SessionRecord) -> IpcMsg {
    IpcMsg::with_label(SessionMsg::REPLY)
        .word(0, record.session_id.get())
        .word(1, record.generation.get())
        .word(
            2,
            record.state as u64 | ((record.kind as u64) << 8),
        )
        .word(
            3,
            record
                .shell_component()
                .and_then(|component| component.process_id)
                .unwrap_or(0),
        )
}

fn establish_lock_session(username: &[u8], uid: u32, gid: u32) -> bool {
    let Some(mezzo) = nameserver_lookup("mezzo") else {
        return false;
    };
    let mut message = IpcMsg::with_label(MezzoMsg::SESSION_ESTABLISH_TRUSTED)
        .word(0, (uid as u64) | ((gid as u64) << 32));
    for (index, byte) in username.iter().copied().take(24).enumerate() {
        let word_index = 1 + index / 8;
        message.words[word_index] |= (byte as u64) << ((index % 8) * 8);
    }
    message.word_count = 4;
    ipc_call(mezzo, message).label == MezzoMsg::REPLY
}

fn session_summary_reply(record: &SessionRecord) -> IpcMsg {
    IpcMsg::with_label(SessionMsg::REPLY)
        .word(0, record.session_id.get())
        .word(1, record.generation.get())
        .word(
            2,
            (record.uid as u64)
                | ((record.state as u64) << 32)
                | ((record.kind as u64) << 40)
                | ((record.required_ready_count() as u64) << 48)
                | ((record.components.len() as u64) << 56),
        )
        .word(3, record.started_at.unwrap_or(record.created_at))
}

fn component_reply(record: &SessionRecord) -> IpcMsg {
    let Some(component) = record.shell_component() else {
        return error(SessionMsg::ERR_NOT_FOUND);
    };
    IpcMsg::with_label(SessionMsg::REPLY)
        .word(0, component.component_id.get())
        .word(1, component.process_id.unwrap_or(0))
        .word(2, component.process_generation.unwrap_or(0))
        .word(
            3,
            (component.state as u64)
                | ((component.role as u64) << 8)
                | ((component.required as u64) << 16)
                | ((component.launch_count as u64) << 24)
                | ((component.restart_count as u64) << 40)
                | ((component.last_exit_reason.as_u8() as u64) << 56),
        )
}

fn health_reply(state: &ServiceState) -> IpcMsg {
    let active_sessions = u64::from(state.active.is_some());
    let active_components = state
        .active
        .as_ref()
        .and_then(|session| session.record.shell_component())
        .and_then(|component| component.process_id)
        .is_some() as u64;
    let manifest_ok = u64::from(state.manifest.is_ok());
    IpcMsg::with_label(SessionMsg::REPLY)
        .word(
            0,
            manifest_ok | (active_sessions << 8) | (active_components << 16),
        )
        .word(
            1,
            (state.stats.sessions_created as u64)
                | ((state.stats.sessions_started as u64) << 32),
        )
        .word(
            2,
            (state.stats.sessions_failed as u64)
                | ((state.stats.sessions_stopped as u64) << 32),
        )
        .word(
            3,
            (state.stats.components_launched as u64)
                | ((state.stats.component_restarts as u64) << 32),
        )
}

fn pack_request_meta(kind: SessionKind, username_len: usize) -> u32 {
    (SESSION_PROTOCOL_VERSION as u32)
        | ((kind as u32) << 16)
        | ((username_len.min(USERNAME_MAX) as u32) << 24)
}

fn find_session<'a>(
    state: &'a ServiceState,
    session_id: u64,
) -> Option<&'a SessionRecord> {
    if let Some(active) = state.active.as_ref() {
        if active.record.session_id.get() == session_id {
            return Some(&active.record);
        }
    }
    state.last_closed.as_ref().filter(|record| record.session_id.get() == session_id)
}

fn find_session_mut<'a>(
    state: &'a mut ServiceState,
    session_id: u64,
) -> Option<&'a mut ActiveSession> {
    state
        .active
        .as_mut()
        .filter(|active| active.record.session_id.get() == session_id)
}

fn transition_to_running(session: &mut ActiveSession, now: u64) -> bool {
    if session.record.transition(SessionState::Running).is_ok() {
        session.record.started_at = Some(now);
        write_log("[SESSION-FOUNDATION] SHELL_READY PASS\n");
        return true;
    }
    false
}

fn spawn_shell(
    manifest: &SessionManifest,
    session: &mut ActiveSession,
    now: u64,
) -> Result<(), u64> {
    let Some(spawn_cap) = nameserver_lookup("spawn") else {
        return Err(SessionMsg::ERR_INVALID_STATE);
    };
    let req = SpawnRequest::new("/bin/sunlight-vortex-shell", "sunlight-vortex-shell")
        .with_identity(session.record.uid, session.record.gid)
        .with_service_caps(ServiceCapability::UserSession.bit());
    let mut msg = IpcMsg::empty();
    req.pack_into(&mut msg);
    let reply = ipc_call(spawn_cap, msg);
    if reply.label != SpawnMsg::REPLY {
        return Err(SessionMsg::ERR_INVALID_STATE);
    }
    let pid = reply.words[0];
    let Some(info) = session_query_process(pid) else {
        return Err(SessionMsg::ERR_INVALID_STATE);
    };
    let timeout = manifest_component(manifest)
        .map(|component| component.readiness_timeout_seconds as u64 * 1_000)
        .unwrap_or(10_000);
    if let Some(component) = session.record.shell_component_mut() {
        component.process_id = Some(pid);
        component.process_generation = Some(info.generation);
        component.state = SessionComponentState::Starting;
        component.launch_count = component.launch_count.saturating_add(1);
    }
    session.record.state = SessionState::StartingRequiredComponents;
    session.shell_ready_deadline_ms = now.saturating_add(timeout);
    write_log("[SESSION-FOUNDATION] SHELL_STARTED PASS\n");
    Ok(())
}

fn begin_stop(session: &mut ActiveSession, now: u64) {
    session.record.state = SessionState::Stopping;
    session.shell_stop_deadline_ms = now.saturating_add(LOGOUT_GRACE_MS);
    if let Some(component) = session.record.shell_component_mut() {
        component.state = SessionComponentState::Stopping;
        if let Some(pid) = component.process_id {
            let _ = kill(pid, 15);
        }
    }
}

fn finalize_stop(state: &mut ServiceState) {
    if let Some(active) = state.active.take() {
        let record = active.record;
        state.last_closed = Some(record);
        state.stats.sessions_stopped = state.stats.sessions_stopped.saturating_add(1);
        state.stats.logout_completions = state.stats.logout_completions.saturating_add(1);
        write_log("[SESSION-FOUNDATION] LOGOUT PASS\n");
    }
}

fn maybe_restart_shell(state: &mut ServiceState, now: u64) {
    let manifest = match state.manifest.as_ref() {
        Ok(manifest) => manifest.clone(),
        Err(_) => return,
    };
    let component_spec = manifest_component(&manifest).cloned();
    let Some(component_spec) = component_spec else {
        return;
    };
    let Some(active) = state.active.as_mut() else {
        return;
    };
    let reset_window = now.saturating_sub(active.last_ready_at_ms)
        > (component_spec.restart_window_seconds as u64 * 1_000);
    let Some(component) = active.record.shell_component_mut() else {
        return;
    };
    if component.restart_count == 0 || reset_window {
        component.restart_count = 0;
    }
    let current_restart_count = component.restart_count;
    let exhausted = current_restart_count >= component_spec.restart_limit;
    if exhausted {
        component.last_exit_reason = ComponentExitReason::RestartExhausted;
    } else {
        component.restart_count = component.restart_count.saturating_add(1);
        component.state = SessionComponentState::RestartPending;
        active.record.restart_count = component.restart_count;
    }
    if reset_window {
        active.record.restart_count = 0;
        active.restart_window_started_ms = now;
    }
    if exhausted {
        active.record.state = SessionState::Failed;
        begin_stop(active, now);
        state.stats.sessions_failed = state.stats.sessions_failed.saturating_add(1);
        state.stats.component_restart_exhaustions =
            state.stats.component_restart_exhaustions.saturating_add(1);
        return;
    }
    active.record.state = SessionState::Degraded;
    let _ = spawn_shell(&manifest, active, now);
    state.stats.components_launched = state.stats.components_launched.saturating_add(1);
    state.stats.sessions_degraded = state.stats.sessions_degraded.saturating_add(1);
    state.stats.component_restarts = state.stats.component_restarts.saturating_add(1);
    write_log("[SESSION-FOUNDATION] SHELL_CRASH_RESTART PASS\n");
}

fn supervise(state: &mut ServiceState) {
    let now = monotonic_millis();
    let mut finalize = false;
    let mut do_restart = false;
    if let Some(active) = state.active.as_mut() {
        let session_state = active.record.state;
        if let Some(component) = active.record.shell_component_mut() {
            if let Some(pid) = component.process_id {
                if !process_is_alive(pid) {
                    state.stats.component_exits = state.stats.component_exits.saturating_add(1);
                    component.process_id = None;
                    component.process_generation = None;
                    if session_state == SessionState::Stopping {
                        component.state = SessionComponentState::Exited;
                        component.last_exit_reason = ComponentExitReason::Stopped;
                        active.record.state = SessionState::Stopped;
                        finalize = true;
                    } else {
                        component.state = SessionComponentState::Failed;
                        component.last_exit_reason = ComponentExitReason::Crashed;
                        state.stats.component_crashes =
                            state.stats.component_crashes.saturating_add(1);
                        do_restart = true;
                    }
                } else if component.state == SessionComponentState::Starting
                    && now >= active.shell_ready_deadline_ms
                {
                    state.stats.component_readiness_timeouts =
                        state.stats.component_readiness_timeouts.saturating_add(1);
                    let _ = kill(pid, 15);
                    component.state = SessionComponentState::Failed;
                    component.last_exit_reason = ComponentExitReason::ReadinessTimeout;
                } else if active.record.state == SessionState::Stopping
                    && now >= active.shell_stop_deadline_ms
                {
                    let _ = kill(pid, 9);
                    state.stats.logout_timeouts = state.stats.logout_timeouts.saturating_add(1);
                }
            }
        }
    }
    if do_restart {
        maybe_restart_shell(state, now);
    }
    if finalize {
        finalize_stop(state);
    }
}

fn decode_create(msg: &IpcMsg) -> Result<(u64, u32, u32, SessionKind, [u8; USERNAME_MAX], usize), u64> {
    let version = (msg.words[3] & 0xffff) as u16;
    if version != SESSION_PROTOCOL_VERSION {
        return Err(SessionMsg::ERR_INVALID_VERSION);
    }
    let kind = SessionKind::from_u64(((msg.words[3] >> 16) & 0xff) as u64)
        .ok_or(SessionMsg::ERR_INVALID_ARGUMENT)?;
    let (username, username_len) = parse_username(msg.words[2], msg.words[3]);
    let uid = msg.words[1] as u32;
    let gid = (msg.words[1] >> 32) as u32;
    if username_len == 0 || username_len > USERNAME_MAX {
        return Err(SessionMsg::ERR_INVALID_ARGUMENT);
    }
    Ok((msg.words[0], uid, gid, kind, username, username_len))
}

fn create_session(state: &mut ServiceState, msg: IpcMsg) -> IpcMsg {
    state.stats.login_handoffs = state.stats.login_handoffs.saturating_add(1);
    if !validate_session_caller(msg.badge, SESSION_CALLER_TTY_SERVICE) {
        state.stats.unauthorized_session_requests =
            state.stats.unauthorized_session_requests.saturating_add(1);
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    if state.manifest.is_err() {
        state.stats.session_create_failures = state.stats.session_create_failures.saturating_add(1);
        return error(SessionMsg::ERR_MANIFEST);
    }
    let Ok((request_id, requested_uid, requested_gid, kind, username, username_len)) =
        decode_create(&msg)
    else {
        state.stats.login_handoff_failures = state.stats.login_handoff_failures.saturating_add(1);
        return error(SessionMsg::ERR_INVALID_ARGUMENT);
    };
    if let Some(active) = state.active.as_ref() {
        if active.request_id == request_id && active.record.uid == requested_uid {
            return create_reply(&active.record);
        }
        return error(SessionMsg::ERR_BUSY);
    }
    let Some((uid, gid)) = session_consume_auth_grant(msg.caps[0], msg.badge) else {
        state.stats.login_handoff_failures = state.stats.login_handoff_failures.saturating_add(1);
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    if uid != requested_uid || gid != requested_gid {
        state.stats.login_handoff_failures = state.stats.login_handoff_failures.saturating_add(1);
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    if !establish_lock_session(&username[..username_len], uid, gid) {
        state.stats.login_handoff_failures = state.stats.login_handoff_failures.saturating_add(1);
        return error(SessionMsg::ERR_INVALID_STATE);
    }
    let manifest = state.manifest.as_ref().unwrap();
    let manifest_copy = manifest.clone();
    let session_id = SessionId::new(state.next_session_id).unwrap();
    state.next_session_id = state.next_session_id.saturating_add(1).max(1);
    let generation = SessionGeneration::new(state.next_generation).unwrap();
    state.next_generation = state.next_generation.saturating_add(1).max(1);
    let now = monotonic_millis();
    let mut record = SessionRecord::new(session_id, generation, uid, gid, kind, now, manifest);
    let _ = record.transition(SessionState::Preparing);
    let _ = record.transition(SessionState::StartingRequiredComponents);
    let mut active = ActiveSession {
        record,
        username,
        username_len,
        request_id,
        shell_ready_deadline_ms: 0,
        shell_stop_deadline_ms: 0,
        last_ready_at_ms: 0,
        restart_window_started_ms: now,
    };
    if spawn_shell(&manifest_copy, &mut active, now).is_err() {
        active.record.state = SessionState::Failed;
        state.stats.sessions_failed = state.stats.sessions_failed.saturating_add(1);
    } else {
        state.stats.components_launched = state.stats.components_launched.saturating_add(1);
    }
    state.stats.sessions_created = state.stats.sessions_created.saturating_add(1);
    write_log("[SESSION-FOUNDATION] SESSION_CREATED PASS\n");
    state.active = Some(active);
    create_reply(&state.active.as_ref().unwrap().record)
}

fn authorize_session_control(
    state: &ServiceState,
    caller: SessionProcessCredentials,
    session_id: u64,
) -> bool {
    let Some(record) = find_session(state, session_id) else {
        return false;
    };
    caller.uid == 0 || caller.uid == record.uid
}

fn session_action(state: &mut ServiceState, msg: IpcMsg) -> IpcMsg {
    let session_id = msg.words[0];
    let generation = msg.words[1];
    let Some(caller) = session_query_process(msg.badge) else {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    if !authorize_session_control(state, caller, session_id) {
        state.stats.unauthorized_session_requests =
            state.stats.unauthorized_session_requests.saturating_add(1);
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    let Some(record) = find_session(state, session_id) else {
        state.stats.stale_session_requests = state.stats.stale_session_requests.saturating_add(1);
        return error(SessionMsg::ERR_STALE);
    };
    if generation != 0 && record.generation.get() != generation {
        state.stats.stale_session_requests = state.stats.stale_session_requests.saturating_add(1);
        return error(SessionMsg::ERR_STALE);
    }
    let Some(action) = SessionAction::from_u64(msg.words[2]) else {
        return error(SessionMsg::ERR_INVALID_ARGUMENT);
    };
    match action {
        SessionAction::Logout => {
            state.stats.logout_requests = state.stats.logout_requests.saturating_add(1);
            let Some(active) = find_session_mut(state, session_id) else {
                return error(SessionMsg::ERR_STALE);
            };
            begin_stop(active, monotonic_millis());
            IpcMsg::with_label(SessionMsg::REPLY)
        }
        SessionAction::RestartShell => {
            let Some(active) = find_session_mut(state, session_id) else {
                return error(SessionMsg::ERR_STALE);
            };
            if let Some(pid) = active.record.shell_component().and_then(|component| component.process_id)
            {
                let _ = kill(pid, 15);
            }
            IpcMsg::with_label(SessionMsg::REPLY)
        }
        SessionAction::Lock => {
            if let Some(mezzo) = nameserver_lookup("mezzo") {
                let _ = ipc_call(mezzo, IpcMsg::with_label(MezzoMsg::LOCK_ACTIVATE));
            }
            let Some(active) = find_session_mut(state, session_id) else {
                return error(SessionMsg::ERR_STALE);
            };
            active.record.state = SessionState::Locked;
            IpcMsg::with_label(SessionMsg::REPLY)
        }
        SessionAction::QueryStatus => {
            let Some(active) = find_session_mut(state, session_id) else {
                return error(SessionMsg::ERR_STALE);
            };
            session_summary_reply(&active.record)
        }
        SessionAction::UnlockCompleted => {
            let Some(active) = find_session_mut(state, session_id) else {
                return error(SessionMsg::ERR_STALE);
            };
            active.record.state = SessionState::Running;
            IpcMsg::with_label(SessionMsg::REPLY)
        }
    }
}

fn get_session(state: &mut ServiceState, msg: IpcMsg) -> IpcMsg {
    let session_id = msg.words[0];
    let generation = msg.words[1];
    let Some(record) = find_session(state, session_id) else {
        return error(SessionMsg::ERR_NOT_FOUND);
    };
    if generation != 0 && record.generation.get() != generation {
        state.stats.stale_session_requests = state.stats.stale_session_requests.saturating_add(1);
        return error(SessionMsg::ERR_STALE);
    }
    session_summary_reply(record)
}

fn list_session(state: &ServiceState, index: u64) -> IpcMsg {
    if index == 0 {
        if let Some(active) = state.active.as_ref() {
            return session_summary_reply(&active.record);
        }
        if let Some(closed) = state.last_closed.as_ref() {
            return session_summary_reply(closed);
        }
    }
    error(SessionMsg::ERR_NOT_FOUND)
}

fn get_components(state: &ServiceState, msg: IpcMsg) -> IpcMsg {
    let session_id = msg.words[0];
    let Some(record) = find_session(state, session_id) else {
        return error(SessionMsg::ERR_NOT_FOUND);
    };
    component_reply(record)
}

fn component_hello(state: &mut ServiceState, msg: IpcMsg) -> IpcMsg {
    if (msg.words[0] as u16) != SESSION_PROTOCOL_VERSION {
        return error(SessionMsg::ERR_INVALID_VERSION);
    }
    let Some(caller) = session_query_process(msg.badge) else {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    let Some(active) = state.active.as_mut() else {
        return error(SessionMsg::ERR_NOT_FOUND);
    };
    let Some(component) = active.record.shell_component_mut() else {
        return error(SessionMsg::ERR_NOT_FOUND);
    };
    if component.process_id != Some(caller.pid)
        || component.process_generation != Some(caller.generation)
        || msg.words[1] != caller.generation
    {
        return error(SessionMsg::ERR_STALE);
    }
    IpcMsg::with_label(SessionMsg::REPLY)
        .word(0, active.record.session_id.get())
        .word(1, active.record.generation.get())
        .word(2, COMPONENT_SHELL_ID)
}

fn component_ready(state: &mut ServiceState, msg: IpcMsg) -> IpcMsg {
    let Some(caller) = session_query_process(msg.badge) else {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    let now = monotonic_millis();
    let mut became_running = false;
    {
        let Some(active) = state.active.as_mut() else {
            return error(SessionMsg::ERR_NOT_FOUND);
        };
        if msg.words[0] != active.record.session_id.get()
            || msg.words[1] != active.record.generation.get()
        {
            return error(SessionMsg::ERR_STALE);
        }
        let Some(component) = active.record.shell_component_mut() else {
            return error(SessionMsg::ERR_NOT_FOUND);
        };
        if component.component_id.get() != msg.words[2]
            || component.process_id != Some(caller.pid)
            || component.process_generation != Some(caller.generation)
            || msg.words[3] != caller.generation
        {
            return error(SessionMsg::ERR_STALE);
        }
        component.state = SessionComponentState::Ready;
        active.last_ready_at_ms = now;
        became_running = transition_to_running(active, now);
    }
    state.stats.components_ready = state.stats.components_ready.saturating_add(1);
    if became_running {
        state.stats.sessions_started = state.stats.sessions_started.saturating_add(1);
        state.stats.sessions_running = state.stats.sessions_running.saturating_add(1);
    }
    IpcMsg::with_label(SessionMsg::REPLY)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let endpoint = endpoint_create();
    nameserver_register(SESSION_ENDPOINT, endpoint);
    write_log("[SESSION-FOUNDATION] SERVICE_READY PASS\n");
    let mut state = ServiceState::new();
    let mut reply = IpcMsg::empty();
    loop {
        supervise(&mut state);
        let Some(message) = ipc_reply_and_try_recv(endpoint, reply) else {
            reply = IpcMsg::empty();
            process_yield();
            continue;
        };
        reply = match message.label {
            SessionMsg::SESSION_CREATE => create_session(&mut state, message),
            SessionMsg::SESSION_GET => get_session(&mut state, message),
            SessionMsg::SESSION_LIST => list_session(&state, message.words[0]),
            SessionMsg::SESSION_GET_COMPONENTS => get_components(&state, message),
            SessionMsg::SESSION_COMPONENT_HELLO => component_hello(&mut state, message),
            SessionMsg::SESSION_COMPONENT_READY => component_ready(&mut state, message),
            SessionMsg::SESSION_ACTION | SessionMsg::SESSION_LOGOUT | SessionMsg::SESSION_RESTART_COMPONENT => {
                session_action(&mut state, message)
            }
            SessionMsg::SESSION_GET_HEALTH | SessionMsg::SESSION_GET_STATS => health_reply(&state),
            _ => error(SessionMsg::ERR_INVALID_ARGUMENT),
        };
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
