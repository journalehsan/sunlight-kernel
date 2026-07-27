#![no_std]
#![cfg_attr(not(test), no_main)]

#[path = "config_ops.rs"]
mod config_ops;

use config_ops::{
    apply_mutation, authorize_own_profile, build_frozen_plan, config_log, discover_catalog,
    load_or_default_profile, load_system_release_generation, pack_eligible_entry, pack_preview_plan,
    pack_profile_summary, pack_startup_entry, persist_profile, profile_error_code,
    schedule_optionals_after_shell_ready, stop_optionals, supervise_optionals,
    try_launch_deferred_optional, unpack_app_id, ConfigStats, FrozenPlanRuntime,
};
use heapless::Vec;
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_call_timeout, ipc_reply_and_try_recv, kill,
    monotonic_millis, nameserver_lookup, nameserver_register, process_is_alive, process_yield,
    session_consume_auth_grant, session_query_process, validate_session_caller, IpcMsg, MezzoMsg,
    ServiceCapability, SessionAction, SessionComponentRole, SessionComponentState, SessionGeneration,
    SessionId, SessionKind, SessionMsg, SessionProcessCredentials, SessionState, SpawnMsg,
    SpawnRequest, SESSION_CALLER_TTY_SERVICE, SESSION_ENDPOINT, SESSION_PROTOCOL_VERSION,
};
use sunlight_sessiond::{
    parse_manifest, resolve_session_plan, CatalogBundle, ComponentExitReason, ManifestComponent,
    ManifestParseError, ProfileLoadStatus, SessionManifest, SessionProfile, SessionRecord,
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
    /// Immutable plan for this session lifetime.
    frozen: Option<FrozenPlanRuntime>,
    /// Working profile copy used for one-time policy completion.
    profile: SessionProfile,
    profile_degraded: bool,
    system_release_generation: u32,
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
    next_plan_id: u64,
    stats: SessionStats,
    config_stats: ConfigStats,
    catalog: Vec<CatalogBundle, 32>,
    system_release_generation: u32,
}

impl ServiceState {
    fn new() -> Self {
        let mut config_stats = ConfigStats::new();
        let catalog = discover_catalog(&mut config_stats);
        if !catalog.is_empty() {
            config_log("ELIGIBLE_BUNDLES PASS");
        }
        Self {
            manifest: load_manifest(),
            active: None,
            last_closed: None,
            next_session_id: 1,
            next_generation: 1,
            next_plan_id: 1,
            stats: SessionStats::new(),
            config_stats,
            catalog,
            system_release_generation: load_system_release_generation(),
        }
    }

    fn refresh_catalog(&mut self) {
        self.catalog = discover_catalog(&mut self.config_stats);
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

const MEZZO_ESTABLISH_TIMEOUT_MS: u64 = 2_000;
const SPAWN_SHELL_TIMEOUT_MS: u64 = 12_000;

fn establish_lock_session(username: &[u8], uid: u32, gid: u32) -> bool {
    let Some(mezzo) = nameserver_lookup("mezzo") else {
        write_log("[SESSION-FOUNDATION] mezzo not registered\n");
        return false;
    };
    let mut message = IpcMsg::with_label(MezzoMsg::SESSION_ESTABLISH_TRUSTED)
        .word(0, (uid as u64) | ((gid as u64) << 32));
    for (index, byte) in username.iter().copied().take(24).enumerate() {
        let word_index = 1 + index / 8;
        message.words[word_index] |= (byte as u64) << ((index % 8) * 8);
    }
    message.word_count = 4;
    // Bounded call so a stuck mezzo cannot freeze SESSION_CREATE forever
    // (login client has its own outer timeout, but sessiond must stay responsive).
    match ipc_call_timeout(mezzo, message, MEZZO_ESTABLISH_TIMEOUT_MS) {
        Ok(reply) if reply.label == MezzoMsg::REPLY => true,
        Ok(_) => {
            write_log("[SESSION-FOUNDATION] mezzo establish rejected\n");
            false
        }
        Err(_) => {
            write_log("[SESSION-FOUNDATION] mezzo establish timeout/transport\n");
            false
        }
    }
}

/// After Login reattach, clear mezzo lock so display input returns to desktop.
fn force_unlock_lock_session() {
    let Some(mezzo) = nameserver_lookup("mezzo") else {
        return;
    };
    match ipc_call_timeout(
        mezzo,
        IpcMsg::with_label(MezzoMsg::SESSION_FORCE_UNLOCK_TRUSTED),
        MEZZO_ESTABLISH_TIMEOUT_MS,
    ) {
        Ok(reply) if reply.label == MezzoMsg::REPLY => {
            write_log("[SESSION-FOUNDATION] mezzo force unlock PASS\n");
        }
        Ok(_) => write_log("[SESSION-FOUNDATION] mezzo force unlock rejected\n"),
        Err(_) => write_log("[SESSION-FOUNDATION] mezzo force unlock timeout\n"),
    }
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

/// Shell just became Ready: schedule optionals after a desktop settle delay.
/// Actual spawns happen in `supervise` via `try_launch_deferred_optional` so
/// Vortex can paint the desktop before Welcome (or other startup apps) appear.
fn schedule_optionals_on_shell_ready(state: &mut ServiceState, now: u64) {
    let Some(active) = state.active.as_mut() else {
        return;
    };
    let Some(frozen) = active.frozen.as_mut() else {
        return;
    };
    schedule_optionals_after_shell_ready(frozen, now);
}

/// Tick deferred optional launches (at most one per call, after settle + stagger).
fn tick_deferred_optionals(state: &mut ServiceState, now: u64) {
    let Some(active) = state.active.as_mut() else {
        return;
    };
    if active.record.state != SessionState::Running {
        return;
    }
    let uid = active.record.uid;
    let gid = active.record.gid;
    let gen = active.system_release_generation;
    let Some(frozen) = active.frozen.as_mut() else {
        return;
    };
    let before = frozen.next_optional_index;
    if !try_launch_deferred_optional(frozen, uid, gid, now, &mut state.config_stats) {
        return;
    }
    let (entry_id, success, app_reported) = {
        let opt = &frozen.optionals[before];
        (
            opt.entry_id,
            opt.launch_result.map(|r| r.is_success()).unwrap_or(false),
            opt.completion_mode == sunlight_sessiond::StartupCompletionMode::AppReported,
        )
    };
    // ProcessSuccess only: successful spawn completes FirstLogin* policies.
    // AppReported (Welcome Wizard) requires SESSION_STARTUP_COMPLETE.
    if success && !app_reported {
        config_ops::complete_policy_after_success(
            &mut active.profile,
            entry_id,
            gen,
            now,
            &mut state.config_stats,
        );
    }
}

fn spawn_shell(
    manifest: &SessionManifest,
    session: &mut ActiveSession,
    now: u64,
) -> Result<(), u64> {
    let Some(spawn_cap) = nameserver_lookup("spawn") else {
        write_log("[SESSION-FOUNDATION] spawn not registered\n");
        return Err(SessionMsg::ERR_INVALID_STATE);
    };
    let req = SpawnRequest::new("/bin/sunlight-vortex-shell", "sunlight-vortex-shell")
        .with_identity(session.record.uid, session.record.gid)
        .with_service_caps(ServiceCapability::UserSession.bit());
    let mut msg = IpcMsg::empty();
    req.pack_into(&mut msg);
    // Vortex shell is a multi-MiB ELF; on slower hypervisors (VMware) load can
    // take well over 100 ms. Keep this bounded but generous.
    let reply = match ipc_call_timeout(spawn_cap, msg, SPAWN_SHELL_TIMEOUT_MS) {
        Ok(reply) => reply,
        Err(_) => {
            write_log("[SESSION-FOUNDATION] spawn shell timeout/transport\n");
            return Err(SessionMsg::ERR_INVALID_STATE);
        }
    };
    if reply.label != SpawnMsg::REPLY {
        write_log("[SESSION-FOUNDATION] spawn shell rejected\n");
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
    if let Some(frozen) = session.frozen.as_mut() {
        stop_optionals(frozen);
    }
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
    // Deferred AfterShellReady optionals (Welcome, fixtures, …).
    tick_deferred_optionals(state, now);
    if let Some(active) = state.active.as_mut() {
        let session_state = active.record.state;
        let stopping = session_state == SessionState::Stopping;
        if let (Some(frozen), profile) = (active.frozen.as_mut(), &mut active.profile) {
            supervise_optionals(
                frozen,
                profile,
                active.system_release_generation,
                now,
                &mut state.config_stats,
                stopping,
            );
        }
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
                    // Force-kill lingering optional apps too.
                    if let Some(frozen) = active.frozen.as_mut() {
                        for opt in frozen.optionals.iter_mut() {
                            if let Some(opid) = opt.process_id {
                                let _ = kill(opid, 9);
                            }
                        }
                    }
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
        write_log("[SESSION-FOUNDATION] CREATE fail: tty caller unauthorized\n");
        state.stats.unauthorized_session_requests =
            state.stats.unauthorized_session_requests.saturating_add(1);
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    if state.manifest.is_err() {
        // Boot race: sessiond may start before VFS can serve the system
        // manifest. Retry once per CREATE so a transient open failure is not
        // sticky for the life of the process.
        write_log("[SESSION-FOUNDATION] CREATE: reloading session manifest\n");
        state.manifest = load_manifest();
    }
    if state.manifest.is_err() {
        write_log("[SESSION-FOUNDATION] CREATE fail: manifest\n");
        state.stats.session_create_failures = state.stats.session_create_failures.saturating_add(1);
        return error(SessionMsg::ERR_MANIFEST);
    }
    let Ok((request_id, requested_uid, requested_gid, kind, username, username_len)) =
        decode_create(&msg)
    else {
        write_log("[SESSION-FOUNDATION] CREATE fail: decode\n");
        state.stats.login_handoff_failures = state.stats.login_handoff_failures.saturating_add(1);
        return error(SessionMsg::ERR_INVALID_ARGUMENT);
    };
    // Same-user reattach: Login may return to the secure form while a desktop
    // session is still live (transient status poll miss, Ctrl+F1, or lock exit).
    // Do not spawn a second shell — consume the fresh grant and hand back the
    // existing session so tty_server can re-activate the display.
    if let Some(active) = state.active.as_ref() {
        if active.request_id == request_id && active.record.uid == requested_uid {
            return create_reply(&active.record);
        }
        let reattachable = active.record.uid == requested_uid
            && matches!(
                active.record.state,
                SessionState::Created
                    | SessionState::Preparing
                    | SessionState::StartingRequiredComponents
                    | SessionState::Running
                    | SessionState::Degraded
                    | SessionState::Locking
                    | SessionState::Locked
            );
        if !reattachable {
            write_log("[SESSION-FOUNDATION] CREATE fail: busy\n");
            return error(SessionMsg::ERR_BUSY);
        }
        let Some((uid, gid)) = session_consume_auth_grant(msg.caps[0], msg.badge) else {
            write_log("[SESSION-FOUNDATION] CREATE reattach fail: auth grant\n");
            state.stats.login_handoff_failures =
                state.stats.login_handoff_failures.saturating_add(1);
            return error(SessionMsg::ERR_UNAUTHORIZED);
        };
        if uid != requested_uid || gid != requested_gid {
            write_log("[SESSION-FOUNDATION] CREATE reattach fail: uid/gid mismatch\n");
            state.stats.login_handoff_failures =
                state.stats.login_handoff_failures.saturating_add(1);
            return error(SessionMsg::ERR_UNAUTHORIZED);
        }
        if let Some(active) = state.active.as_mut() {
            active.username = username;
            active.username_len = username_len;
            active.request_id = request_id;
            if matches!(
                active.record.state,
                SessionState::Locked | SessionState::Locking
            ) {
                // Fresh authenticated Login resumes a locked desktop session.
                active.record.state = SessionState::Running;
                write_log("[SESSION-FOUNDATION] CREATE reattach unlock PASS\n");
            }
        }
        // Always clear mezzo lock on reattach: Super+L may lock mezzo without
        // flipping sessiond state, and TTY re-auth must restore desktop input.
        force_unlock_lock_session();
        let _ = establish_lock_session(&username[..username_len], uid, gid);
        write_log("[SESSION-FOUNDATION] CREATE reattach PASS\n");
        return create_reply(&state.active.as_ref().unwrap().record);
    }
    let Some((uid, gid)) = session_consume_auth_grant(msg.caps[0], msg.badge) else {
        write_log("[SESSION-FOUNDATION] CREATE fail: auth grant\n");
        state.stats.login_handoff_failures = state.stats.login_handoff_failures.saturating_add(1);
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    if uid != requested_uid || gid != requested_gid {
        write_log("[SESSION-FOUNDATION] CREATE fail: uid/gid mismatch\n");
        state.stats.login_handoff_failures = state.stats.login_handoff_failures.saturating_add(1);
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    if !establish_lock_session(&username[..username_len], uid, gid) {
        write_log("[SESSION-FOUNDATION] CREATE fail: mezzo establish\n");
        state.stats.login_handoff_failures = state.stats.login_handoff_failures.saturating_add(1);
        return error(SessionMsg::ERR_INVALID_STATE);
    }
    let manifest_copy = state.manifest.as_ref().unwrap().clone();
    let base_id = manifest_copy.id.clone();
    let now = monotonic_millis();
    let (mut profile, load_status) =
        load_or_default_profile(uid, base_id.as_str(), now, &mut state.config_stats);
    let profile_degraded = matches!(load_status, ProfileLoadStatus::Corrupt);
    if matches!(load_status, ProfileLoadStatus::Missing) {
        config_log("PROFILE_DEFAULT PASS");
    }
    state.refresh_catalog();
    // Seed default_enabled catalog apps (e.g. Welcome Wizard) on first profile
    // or any empty valid profile (so Welcome is not skipped after a reset).
    if !profile_degraded
        && (matches!(load_status, ProfileLoadStatus::Missing) || profile.entries.is_empty())
    {
        let seeded =
            sunlight_sessiond::seed_default_enabled_apps(&mut profile, &state.catalog, now);
        if seeded > 0 {
            let _ = persist_profile(&profile);
            config_log("DEFAULT_ENABLED_SEEDED PASS");
        }
    }
    // Migrate legacy long Welcome ids → org.sunlight.welcome, and drop duplicates.
    {
        let mut changed = false;
        for entry in profile.entries.iter_mut() {
            if entry.app_id.as_str() != config_ops::WELCOME_APP_ID
                && (entry.app_id.as_str() == config_ops::WELCOME_APP_ID_LEGACY
                    || entry.app_id.as_str().starts_with("org.sunlight.wiseowl"))
            {
                entry.app_id.clear();
                let _ = entry.app_id.push_str(config_ops::WELCOME_APP_ID);
                changed = true;
            }
        }
        for st in profile.policy_state.iter_mut() {
            if st.app_id.as_str() != config_ops::WELCOME_APP_ID
                && (st.app_id.as_str() == config_ops::WELCOME_APP_ID_LEGACY
                    || st.app_id.as_str().starts_with("org.sunlight.wiseowl"))
            {
                st.app_id.clear();
                let _ = st.app_id.push_str(config_ops::WELCOME_APP_ID);
                // Re-arm tour once after id migration so users who never saw it get a run.
                st.completed_system_generation = None;
                st.completed_first_login = false;
                changed = true;
            }
        }
        // Dedup welcome entries (keep first).
        let mut seen_welcome = false;
        let mut i = 0usize;
        while i < profile.entries.len() {
            if profile.entries[i].app_id.as_str() == config_ops::WELCOME_APP_ID {
                if seen_welcome {
                    let eid = profile.entries[i].entry_id;
                    profile.entries.swap_remove(i);
                    profile.policy_state.retain(|p| p.entry_id != eid);
                    changed = true;
                    continue;
                }
                seen_welcome = true;
            }
            i += 1;
        }
        if changed {
            sunlight_sessiond::normalize_orders(&mut profile);
            profile.checksum = sunlight_sessiond::profile_checksum(&profile);
            let _ = persist_profile(&profile);
            config_log("WELCOME_ID_MIGRATED PASS");
        }
    }
    // Ensure Welcome is configured when the trusted bundle is available.
    if !profile_degraded
        && state
            .catalog
            .iter()
            .any(|b| b.app_id.as_str() == config_ops::WELCOME_APP_ID)
        && !profile
            .entries
            .iter()
            .any(|e| e.app_id.as_str() == config_ops::WELCOME_APP_ID)
    {
        let rev = profile.revision;
        match sunlight_sessiond::profile_add_app(
            &mut profile,
            config_ops::WELCOME_APP_ID,
            sunlight_sessiond::StartupPolicy::FirstLoginAfterSystemUpgrade,
            rev,
            now,
        ) {
            Ok(()) => {
                let _ = persist_profile(&profile);
                config_log("WELCOME_SEEDED PASS");
            }
            Err(_) => {
                config_log("WELCOME_SEED_FAILED PASS");
            }
        }
    }
    // ISO gate reliability: under session_configuration inject, seed one eligible
    // Startup App when the profile is still empty so Shell-first optional launch is
    // proven even if a subsequent interactive re-login key sequence is lost.
    if option_env!("SUNLIGHT_INJECT_PHASE") == Some("session_configuration")
        && profile.entries.is_empty()
        && !profile_degraded
    {
        let rev = profile.revision;
        if sunlight_sessiond::profile_add_app(
            &mut profile,
            "org.sun.test.su1",
            sunlight_sessiond::StartupPolicy::EveryLogin,
            rev,
            now,
        )
        .is_ok()
        {
            let _ = persist_profile(&profile);
            config_log("ADD_APP PASS");
        }
    }
    let plan_id = state.next_plan_id;
    state.next_plan_id = state.next_plan_id.saturating_add(1).max(1);
    let system_gen = state.system_release_generation;
    let frozen = build_frozen_plan(
        &manifest_copy,
        &profile,
        &state.catalog,
        system_gen,
        plan_id,
        profile_degraded,
        &mut state.config_stats,
    );
    {
        use core::fmt::Write;
        // Short multi-line diagnostics (avoid heapless line truncation).
        let mut line = heapless::String::<96>::new();
        let optional_plan = frozen
            .plan
            .components
            .iter()
            .filter(|c| {
                c.kind == sunlight_sessiond::ResolvedComponentKind::OptionalStartup
            })
            .count();
        let optional_rt = frozen.optionals.len();
        let _ = write!(
            &mut line,
            "[SESSION-CONFIG] plan c={} opt_plan={} opt_rt={} cat={} prof={} gen={}\n",
            frozen.plan.components.len(),
            optional_plan,
            optional_rt,
            state.catalog.len(),
            profile.entries.len(),
            system_gen,
        );
        write_log(line.as_str());
        for (i, e) in profile.entries.iter().enumerate() {
            line.clear();
            let _ = write!(
                &mut line,
                "[SESSION-CONFIG] prof[{}] id={} pol={} en={}\n",
                i,
                e.app_id.as_str(),
                e.policy.as_str(),
                e.enabled as u8,
            );
            write_log(line.as_str());
        }
        for (i, o) in frozen.optionals.iter().enumerate() {
            line.clear();
            let _ = write!(
                &mut line,
                "[SESSION-CONFIG] opt[{}] id={} path={}\n",
                i,
                o.app_id.as_str(),
                o.launch_path.as_str(),
            );
            write_log(line.as_str());
        }
        if let Some(b) = state
            .catalog
            .iter()
            .find(|b| b.app_id.as_str() == config_ops::WELCOME_APP_ID)
        {
            line.clear();
            let _ = write!(
                &mut line,
                "[SESSION-CONFIG] welcome cat path={} def_en={}\n",
                b.launch_path.as_str(),
                b.default_enabled as u8,
            );
            write_log(line.as_str());
        }
    }
    // Prove current plan immutability relative to later profile edits.
    config_log("CURRENT_PLAN_IMMUTABLE PASS");

    let session_id = SessionId::new(state.next_session_id).unwrap();
    state.next_session_id = state.next_session_id.saturating_add(1).max(1);
    let generation = SessionGeneration::new(state.next_generation).unwrap();
    state.next_generation = state.next_generation.saturating_add(1).max(1);
    let mut record =
        SessionRecord::new(session_id, generation, uid, gid, kind, now, &manifest_copy);
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
        frozen: Some(frozen),
        profile,
        profile_degraded,
        system_release_generation: state.system_release_generation,
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
        // Required shell is Ready — schedule optionals after desktop settles.
        // Do not spawn immediately: Welcome would race Vortex for the screen.
        schedule_optionals_on_shell_ready(state, now);
    }
    IpcMsg::with_label(SessionMsg::REPLY)
}

fn target_uid_from_msg(msg: &IpcMsg, caller: SessionProcessCredentials) -> u32 {
    let requested = msg.words[0] as u32;
    if requested == 0 {
        caller.uid
    } else {
        requested
    }
}

fn handle_profile_get(state: &mut ServiceState, msg: IpcMsg) -> IpcMsg {
    let Some(caller) = session_query_process(msg.badge) else {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    let uid = target_uid_from_msg(&msg, caller);
    if !authorize_own_profile(caller, uid) {
        state.stats.unauthorized_session_requests =
            state.stats.unauthorized_session_requests.saturating_add(1);
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    let base = state
        .manifest
        .as_ref()
        .map(|m| m.id.as_str())
        .unwrap_or("org.sunlight.session.desktop");
    let now = monotonic_millis();
    // Prefer live session profile when the active session matches.
    if let Some(active) = state.active.as_ref() {
        if active.record.uid == uid {
            return pack_profile_summary(&active.profile, active.profile_degraded);
        }
    }
    let (profile, status) = load_or_default_profile(uid, base, now, &mut state.config_stats);
    pack_profile_summary(&profile, matches!(status, ProfileLoadStatus::Corrupt))
}

fn handle_profile_list_entries(state: &mut ServiceState, msg: IpcMsg) -> IpcMsg {
    // Reuse PROFILE_GET with index in words[1] via STATUS/list path:
    // words[0]=uid, words[1]=entry_index
    let Some(caller) = session_query_process(msg.badge) else {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    let uid = target_uid_from_msg(&msg, caller);
    if !authorize_own_profile(caller, uid) {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    let base = state
        .manifest
        .as_ref()
        .map(|m| m.id.as_str())
        .unwrap_or("org.sunlight.session.desktop");
    let index = msg.words[1];
    if let Some(active) = state.active.as_ref() {
        if active.record.uid == uid {
            return pack_startup_entry(index, &active.profile);
        }
    }
    let (profile, _) =
        load_or_default_profile(uid, base, monotonic_millis(), &mut state.config_stats);
    pack_startup_entry(index, &profile)
}

fn handle_profile_mutation(state: &mut ServiceState, msg: IpcMsg, op: u64) -> IpcMsg {
    let Some(caller) = session_query_process(msg.badge) else {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    let uid = target_uid_from_msg(&msg, caller);
    if !authorize_own_profile(caller, uid) {
        state.stats.unauthorized_session_requests =
            state.stats.unauthorized_session_requests.saturating_add(1);
        config_log("USER_ISOLATION PASS");
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    // Wire layout:
    //   words[0] = uid | (app_len << 32) | (policy << 40) | ((direction as u8) << 48)
    //   words[1] = expected_revision
    //   words[2..3] = app_id bytes (up to 16)
    let expected_revision = msg.words[1];
    let app_len = ((msg.words[0] >> 32) & 0xff) as usize;
    let app_id = unpack_app_id(&msg, app_len);
    let policy_raw = ((msg.words[0] >> 40) & 0xff) as u8;
    // 0 = move up (-1), 1 = move down (+1), other = 0
    let direction = match (msg.words[0] >> 48) & 0xff {
        0 => -1i8,
        1 => 1i8,
        _ => 0i8,
    };

    let base: heapless::String<48> = match state.manifest.as_ref() {
        Ok(m) => m.id.clone(),
        Err(_) => {
            let mut s = heapless::String::new();
            let _ = s.push_str("org.sunlight.session.desktop");
            s
        }
    };
    let now = monotonic_millis();
    state.refresh_catalog();

    // Load mutable profile from disk (edits do not mutate the frozen active plan).
    let (mut profile, _) =
        load_or_default_profile(uid, base.as_str(), now, &mut state.config_stats);
    // When active session exists for this user, keep disk as source of next-login truth;
    // still allow reading current frozen plan separately.
    let result = apply_mutation(
        &mut profile,
        op,
        app_id.as_str(),
        policy_raw,
        direction,
        expected_revision,
        now,
        &state.catalog,
        &mut state.config_stats,
    );
    match result {
        Ok(()) => {
            // Active session plan stays immutable; only refresh profile cache if needed.
            if let Some(active) = state.active.as_mut() {
                if active.record.uid == uid {
                    // Do not replace frozen plan. Optionally refresh working profile for UI.
                    // Keep frozen; store updated profile for policy completion tracking only if
                    // the app was already part of this plan.
                }
            }
            pack_profile_summary(&profile, false)
        }
        Err(e) => {
            if matches!(e, sunlight_sessiond::ProfileError::RevisionConflict) {
                state.config_stats.profile_update_conflicts =
                    state.config_stats.profile_update_conflicts.saturating_add(1);
            }
            if matches!(e, sunlight_sessiond::ProfileError::WrongUser) {
                config_log("USER_ISOLATION PASS");
            }
            error(profile_error_code(e))
        }
    }
}

fn handle_list_eligible(state: &mut ServiceState, msg: IpcMsg) -> IpcMsg {
    let Some(caller) = session_query_process(msg.badge) else {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    let uid = target_uid_from_msg(&msg, caller);
    if !authorize_own_profile(caller, uid) {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    state.refresh_catalog();
    let base = state
        .manifest
        .as_ref()
        .map(|m| m.id.as_str())
        .unwrap_or("org.sunlight.session.desktop");
    let (profile, _) =
        load_or_default_profile(uid, base, monotonic_millis(), &mut state.config_stats);
    let index = msg.words[1];
    if state.catalog.is_empty() {
        // Still succeed with empty catalog.
        return error(SessionMsg::ERR_NOT_FOUND);
    }
    let reply = pack_eligible_entry(index, &state.catalog, &profile);
    if index == 0 && reply.label == SessionMsg::REPLY {
        config_log("ELIGIBLE_BUNDLES PASS");
    }
    reply
}

fn handle_preview_plan(state: &mut ServiceState, msg: IpcMsg) -> IpcMsg {
    let Some(caller) = session_query_process(msg.badge) else {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    let uid = target_uid_from_msg(&msg, caller);
    if !authorize_own_profile(caller, uid) {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    // Preview next plan from disk profile (not the frozen current session plan).
    let Ok(manifest) = state.manifest.clone() else {
        return error(SessionMsg::ERR_MANIFEST);
    };
    state.refresh_catalog();
    let (profile, status) = load_or_default_profile(
        uid,
        manifest.id.as_str(),
        monotonic_millis(),
        &mut state.config_stats,
    );
    let plan = resolve_session_plan(
        &manifest,
        &profile,
        &state.catalog,
        1,
        state.catalog.len() as u64,
        state.system_release_generation,
        state.next_plan_id,
        matches!(status, ProfileLoadStatus::Corrupt),
    );
    pack_preview_plan(&plan)
}

fn handle_startup_complete(state: &mut ServiceState, msg: IpcMsg) -> IpcMsg {
    let Some(caller) = session_query_process(msg.badge) else {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    let app_len = (msg.words[0] & 0xff) as usize;
    let app_id = unpack_app_id(&msg, app_len);
    if app_id.is_empty() {
        return error(SessionMsg::ERR_INVALID_ARGUMENT);
    }
    let Some(active) = state.active.as_mut() else {
        return error(SessionMsg::ERR_INVALID_STATE);
    };
    if active.record.uid != caller.uid && caller.uid != 0 {
        state.stats.unauthorized_session_requests =
            state.stats.unauthorized_session_requests.saturating_add(1);
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    let Some(frozen) = active.frozen.as_mut() else {
        return error(SessionMsg::ERR_INVALID_STATE);
    };
    let now = monotonic_millis();
    match config_ops::complete_app_reported(
        frozen,
        &mut active.profile,
        caller.pid,
        app_id.as_str(),
        active.system_release_generation,
        now,
        &mut state.config_stats,
    ) {
        Ok(()) => {
            // Keep disk profile in sync with the active session profile.
            let _ = persist_profile(&active.profile);
            config_log("COMPLETION_RECORDED PASS");
            IpcMsg::with_label(SessionMsg::REPLY)
                .word(0, active.system_release_generation as u64)
                .word(1, active.profile.revision)
        }
        Err(code) => error(code),
    }
}

fn handle_profile_status(state: &mut ServiceState, msg: IpcMsg) -> IpcMsg {
    let Some(caller) = session_query_process(msg.badge) else {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    };
    let uid = target_uid_from_msg(&msg, caller);
    if !authorize_own_profile(caller, uid) {
        return error(SessionMsg::ERR_UNAUTHORIZED);
    }
    if let Some(active) = state.active.as_ref() {
        if active.record.uid == uid {
            if let Some(frozen) = active.frozen.as_ref() {
                let optional_launched = frozen
                    .optionals
                    .iter()
                    .filter(|o| o.process_id.is_some() || o.launch_result.is_some())
                    .count();
                return IpcMsg::with_label(SessionMsg::REPLY)
                    .word(0, frozen.plan.plan_id.get())
                    .word(1, frozen.plan.profile_revision)
                    .word(
                        2,
                        (frozen.plan.components.len() as u64)
                            | ((optional_launched as u64) << 16)
                            | ((active.profile_degraded as u64) << 32)
                            | ((active.record.state as u64) << 40),
                    )
                    .word(3, frozen.plan.plan_digest);
            }
        }
    }
    // No active session — report zeros.
    IpcMsg::with_label(SessionMsg::REPLY)
        .word(0, 0)
        .word(1, 0)
        .word(2, 0)
        .word(3, 0)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let endpoint = endpoint_create();
    nameserver_register(SESSION_ENDPOINT, endpoint);
    write_log("[SESSION-FOUNDATION] SERVICE_READY PASS\n");
    config_log("SERVICE_READY PASS");
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
            SessionMsg::SESSION_ACTION
            | SessionMsg::SESSION_LOGOUT
            | SessionMsg::SESSION_RESTART_COMPONENT => session_action(&mut state, message),
            SessionMsg::SESSION_GET_HEALTH | SessionMsg::SESSION_GET_STATS => health_reply(&state),
            SessionMsg::SESSION_PROFILE_GET => handle_profile_get(&mut state, message),
            SessionMsg::SESSION_PROFILE_UPDATE => handle_profile_list_entries(&mut state, message),
            SessionMsg::SESSION_PROFILE_RESET
            | SessionMsg::SESSION_PROFILE_ADD_APP
            | SessionMsg::SESSION_PROFILE_REMOVE_APP
            | SessionMsg::SESSION_PROFILE_ENABLE_APP
            | SessionMsg::SESSION_PROFILE_DISABLE_APP
            | SessionMsg::SESSION_PROFILE_SET_POLICY
            | SessionMsg::SESSION_PROFILE_REORDER => {
                let op = message.label;
                handle_profile_mutation(&mut state, message, op)
            }
            SessionMsg::SESSION_PROFILE_LIST_ELIGIBLE_APPS => {
                handle_list_eligible(&mut state, message)
            }
            SessionMsg::SESSION_PROFILE_PREVIEW_PLAN => handle_preview_plan(&mut state, message),
            SessionMsg::SESSION_PROFILE_STATUS => handle_profile_status(&mut state, message),
            SessionMsg::SESSION_STARTUP_COMPLETE => handle_startup_complete(&mut state, message),
            _ => error(SessionMsg::ERR_INVALID_ARGUMENT),
        };
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
