//! Session Configuration Phase 1 runtime helpers for sunlight-sessiond.
//! Profile persistence, trusted catalog discovery, optional launch, and IPC packing.

use core::fmt::Write;
use heapless::{String, Vec};
use sunlight_ipc::{
    debug_log, ipc_call_timeout, nameserver_lookup, process_is_alive, session_query_process, IpcMsg,
    ServiceCapability, SessionComponentState, SessionMsg, SessionProcessCredentials, SpawnMsg,
    SpawnRequest,
};
use sunlight_sessiond::{
    default_profile, deserialize_profile, list_eligible, mark_policy_success, normalize_orders,
    parse_bundle_session_manifest, profile_add_app, profile_checksum, profile_move,
    profile_remove_app, profile_reset, profile_set_enabled, profile_set_policy, resolve_session_plan,
    serialize_profile, validate_bundle_id, validate_profile, CatalogBundle, OptionalComponentRestartPolicy,
    ProfileError, ProfileLoadStatus, ResolvedComponentKind, ResolvedSessionPlan, SessionManifest,
    SessionProfile, StartupAvailability, StartupLaunchResult, StartupPolicy, DEFAULT_SYSTEM_RELEASE_GENERATION,
    MAX_ELIGIBLE_CATALOG, MAX_LAUNCH_PATH, MAX_SESSION_COMPONENTS, MAX_STARTUP_ENTRIES,
    PROFILE_BLOB_MAX, PROTECTED_SHELL_APP_ID,
};

/// Profiles live under `/root/.config/sunlight/` because sessiond currently
/// runs as root User actor and VFS allows root home-tree writes without a
/// separate service-state capability grant.
const PROFILE_DIR: &[u8] = b"/root/.config/sunlight";
const RELEASE_GEN_PATH: &[u8] = b"/etc/sunlight/release-generation";
const APPS_ROOT: &[u8] = b"/Applications";
const CATALOG_MAX_SCAN: usize = 24;
const OPTIONAL_READY_TIMEOUT_MS: u64 = 5_000;
const SPAWN_OPTIONAL_TIMEOUT_MS: u64 = 8_000;
/// Let Vortex Shell paint the desktop before optional Startup Apps spawn.
/// Immediate spawn races the shell for display focus and can look like a
/// "no desktop" flash. Matches ordinary sun-exec app launch timing.
/// After Shell Ready, wait for Vortex first paint + session activation before
/// spawning Welcome. Too short races a black framebuffer; ~2.5s is comfortable.
pub const DESKTOP_SETTLE_MS: u64 = 2_500;
/// Minimum gap between consecutive optional app spawns (stagger).
pub const OPTIONAL_STAGGER_MS: u64 = 250;

pub fn config_log(marker: &str) {
    // Single serial line so ISO gates can match the full marker string.
    let mut line = heapless::String::<96>::new();
    let _ = line.push_str("[SESSION-CONFIG] ");
    let _ = line.push_str(marker);
    let _ = line.push('\n');
    debug_log(line.as_str());
}

#[derive(Clone, Copy)]
pub struct ConfigStats {
    pub profiles_loaded: u32,
    pub profiles_missing: u32,
    pub profiles_corrupt: u32,
    pub profile_updates: u32,
    pub profile_update_conflicts: u32,
    pub profile_resets: u32,
    pub eligible_bundles_discovered: u32,
    pub invalid_bundles_skipped: u32,
    pub startup_entries_resolved: u32,
    pub startup_entries_skipped_disabled: u32,
    pub startup_entries_skipped_policy: u32,
    pub startup_entries_skipped_unavailable: u32,
    pub startup_apps_launched: u32,
    pub startup_apps_ready: u32,
    pub startup_apps_failed: u32,
    pub startup_apps_readiness_timeout: u32,
    pub startup_apps_duplicate_skipped: u32,
    pub first_login_completions: u32,
    pub upgrade_login_completions: u32,
    pub resolved_plans_created: u32,
    pub resolved_plan_failures: u32,
}

impl ConfigStats {
    pub const fn new() -> Self {
        Self {
            profiles_loaded: 0,
            profiles_missing: 0,
            profiles_corrupt: 0,
            profile_updates: 0,
            profile_update_conflicts: 0,
            profile_resets: 0,
            eligible_bundles_discovered: 0,
            invalid_bundles_skipped: 0,
            startup_entries_resolved: 0,
            startup_entries_skipped_disabled: 0,
            startup_entries_skipped_policy: 0,
            startup_entries_skipped_unavailable: 0,
            startup_apps_launched: 0,
            startup_apps_ready: 0,
            startup_apps_failed: 0,
            startup_apps_readiness_timeout: 0,
            startup_apps_duplicate_skipped: 0,
            first_login_completions: 0,
            upgrade_login_completions: 0,
            resolved_plans_created: 0,
            resolved_plan_failures: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct OptionalRuntime {
    pub component_id: u64,
    pub app_id: String<32>,
    pub entry_id: Option<u64>,
    pub process_id: Option<u64>,
    pub process_generation: Option<u64>,
    pub state: SessionComponentState,
    pub launch_result: Option<StartupLaunchResult>,
    pub restart_policy: OptionalComponentRestartPolicy,
    pub restart_used: bool,
    pub single_instance: bool,
    pub completion_mode: sunlight_sessiond::StartupCompletionMode,
    pub launch_path: String<MAX_LAUNCH_PATH>,
    pub ready_deadline_ms: u64,
    pub order: u16,
    /// True after app-reported completion was accepted for this component.
    pub app_completion_recorded: bool,
}

pub struct FrozenPlanRuntime {
    pub plan: ResolvedSessionPlan,
    pub optionals: Vec<OptionalRuntime, MAX_SESSION_COMPONENTS>,
    pub next_optional_index: usize,
    pub optionals_started: bool,
    pub shell_first_logged: bool,
    pub ordering_logged: bool,
    /// Wall time (monotonic_ms) when optional apps may first spawn.
    /// 0 = not yet scheduled (shell not ready). After shell Ready, set to
    /// `now + DESKTOP_SETTLE_MS` so the desktop can finish first paint.
    pub optionals_not_before_ms: u64,
    /// Earliest time the next optional may be spawned (stagger).
    pub next_optional_not_before_ms: u64,
}

fn profile_path_for_uid(uid: u32, out: &mut [u8; 96]) -> usize {
    // /root/.config/sunlight/session-profile.v1.<uid>
    let prefix = b"/root/.config/sunlight/session-profile.v1.";
    out[..prefix.len()].copy_from_slice(prefix);
    let mut n = uid;
    let mut digits = [0u8; 10];
    let mut dlen = 0usize;
    if n == 0 {
        digits[0] = b'0';
        dlen = 1;
    } else {
        while n > 0 && dlen < digits.len() {
            digits[dlen] = b'0' + (n % 10) as u8;
            n /= 10;
            dlen += 1;
        }
    }
    for i in 0..dlen {
        out[prefix.len() + i] = digits[dlen - 1 - i];
    }
    prefix.len() + dlen
}

pub fn load_system_release_generation() -> u32 {
    let Ok(fd) = sunlight_libc::open(RELEASE_GEN_PATH) else {
        return DEFAULT_SYSTEM_RELEASE_GENERATION;
    };
    let mut buf = [0u8; 16];
    let Ok(n) = sunlight_libc::read(fd, &mut buf) else {
        let _ = sunlight_libc::close(fd);
        return DEFAULT_SYSTEM_RELEASE_GENERATION;
    };
    let _ = sunlight_libc::close(fd);
    let Ok(text) = core::str::from_utf8(&buf[..n]) else {
        return DEFAULT_SYSTEM_RELEASE_GENERATION;
    };
    text.trim()
        .parse::<u32>()
        .unwrap_or(DEFAULT_SYSTEM_RELEASE_GENERATION)
        .max(1)
}

pub fn load_or_default_profile(
    uid: u32,
    base_session_id: &str,
    now: u64,
    stats: &mut ConfigStats,
) -> (SessionProfile, ProfileLoadStatus) {
    let mut path = [0u8; 96];
    let plen = profile_path_for_uid(uid, &mut path);
    let Ok(fd) = sunlight_libc::open(&path[..plen]) else {
        stats.profiles_missing = stats.profiles_missing.saturating_add(1);
        let profile = default_profile(uid, base_session_id, now).unwrap_or_else(|_| {
            default_profile(uid, "org.sunlight.session.desktop", now).unwrap()
        });
        return (profile, ProfileLoadStatus::Missing);
    };
    let mut bytes = [0u8; PROFILE_BLOB_MAX];
    let Ok(n) = sunlight_libc::read(fd, &mut bytes) else {
        let _ = sunlight_libc::close(fd);
        stats.profiles_corrupt = stats.profiles_corrupt.saturating_add(1);
        let profile = default_profile(uid, base_session_id, now).unwrap();
        return (profile, ProfileLoadStatus::Corrupt);
    };
    let _ = sunlight_libc::close(fd);
    match deserialize_profile(&bytes[..n], uid) {
        Ok(profile) => {
            stats.profiles_loaded = stats.profiles_loaded.saturating_add(1);
            (profile, ProfileLoadStatus::Ok)
        }
        Err(_) => {
            stats.profiles_corrupt = stats.profiles_corrupt.saturating_add(1);
            let profile = default_profile(uid, base_session_id, now).unwrap();
            (profile, ProfileLoadStatus::Corrupt)
        }
    }
}

pub fn persist_profile(profile: &SessionProfile) -> Result<(), ProfileError> {
    let _ = sunlight_libc::mkdir_recursive(PROFILE_DIR);
    let mut path = [0u8; 96];
    let plen = profile_path_for_uid(profile.user_id, &mut path);
    let mut tmp = [0u8; 100];
    tmp[..plen].copy_from_slice(&path[..plen]);
    tmp[plen..plen + 4].copy_from_slice(b".tmp");
    let tlen = plen + 4;
    let mut blob = [0u8; PROFILE_BLOB_MAX];
    let mut live = profile.clone();
    live.checksum = profile_checksum(&live);
    let n = serialize_profile(&live, &mut blob)?;
    let fd = sunlight_libc::open_with_flags(
        &tmp[..tlen],
        sunlight_libc::O_WRONLY | sunlight_libc::O_CREAT | sunlight_libc::O_TRUNC,
    )
    .map_err(|_| ProfileError::InvalidString)?;
    let w = sunlight_libc::write_all(fd, &blob[..n]);
    let _ = sunlight_libc::close(fd);
    w.map_err(|_| ProfileError::InvalidString)?;
    sunlight_libc::rename(&tmp[..tlen], &path[..plen]).map_err(|_| ProfileError::InvalidString)?;
    Ok(())
}

fn push_str<const N: usize>(s: &str) -> String<N> {
    let mut out = String::new();
    let _ = out.push_str(s);
    out
}

fn try_load_bundle_dir(dir_name: &[u8], stats: &mut ConfigStats) -> Option<CatalogBundle> {
    if !dir_name.ends_with(b".sunapp") {
        return None;
    }
    let mut root = [0u8; 128];
    let prefix = b"/Applications/";
    if prefix.len() + dir_name.len() >= root.len() {
        return None;
    }
    root[..prefix.len()].copy_from_slice(prefix);
    root[prefix.len()..prefix.len() + dir_name.len()].copy_from_slice(dir_name);
    let rlen = prefix.len() + dir_name.len();
    let mut manifest_path = [0u8; 160];
    manifest_path[..rlen].copy_from_slice(&root[..rlen]);
    let suffix = b"/Manifest.toml";
    if rlen + suffix.len() >= manifest_path.len() {
        return None;
    }
    manifest_path[rlen..rlen + suffix.len()].copy_from_slice(suffix);
    let mlen = rlen + suffix.len();
    let Ok(fd) = sunlight_libc::open(&manifest_path[..mlen]) else {
        stats.invalid_bundles_skipped = stats.invalid_bundles_skipped.saturating_add(1);
        return None;
    };
    let mut bytes = [0u8; 4096];
    let Ok(n) = sunlight_libc::read(fd, &mut bytes) else {
        let _ = sunlight_libc::close(fd);
        stats.invalid_bundles_skipped = stats.invalid_bundles_skipped.saturating_add(1);
        return None;
    };
    let _ = sunlight_libc::close(fd);
    let Ok(text) = core::str::from_utf8(&bytes[..n]) else {
        stats.invalid_bundles_skipped = stats.invalid_bundles_skipped.saturating_add(1);
        return None;
    };
    let Ok(root_str) = core::str::from_utf8(&root[..rlen]) else {
        return None;
    };
    let parsed = match parse_bundle_session_manifest(text, root_str) {
        Ok(p) => p,
        Err(_) => {
            stats.invalid_bundles_skipped = stats.invalid_bundles_skipped.saturating_add(1);
            return None;
        }
    };
    if parsed.app_id.as_str() == PROTECTED_SHELL_APP_ID {
        return None;
    }
    if !parsed.startup_eligible {
        return None;
    }
    if parsed.launch_path.is_empty() || parsed.launch_path.len() >= 32 {
        // SpawnRequest path limit is 32 including NUL; require short launch paths.
        stats.invalid_bundles_skipped = stats.invalid_bundles_skipped.saturating_add(1);
        return None;
    }
    // Executable must exist.
    if sunlight_libc::stat(parsed.launch_path.as_bytes()).is_err() {
        stats.invalid_bundles_skipped = stats.invalid_bundles_skipped.saturating_add(1);
        return None;
    }
    stats.eligible_bundles_discovered = stats.eligible_bundles_discovered.saturating_add(1);
    if parsed.app_id.as_str() == "org.sunlight.welcome"
        || parsed.app_id.as_str() == "org.sunlight.wiseowl-welcome"
    {
        config_log("WELCOME_BUNDLE_DISCOVERED PASS");
    }
    Some(CatalogBundle {
        app_id: parsed.app_id,
        display_name: parsed.display_name,
        version: parsed.version,
        icon_reference: parsed.icon,
        publisher: None,
        default_policy: parsed.default_policy,
        default_enabled: parsed.default_enabled,
        single_instance: parsed.single_instance,
        completion_mode: parsed.completion_mode,
        availability: StartupAvailability::Available,
        launch_path: parsed.launch_path,
        bundle_dir: push_str(root_str),
        startup_eligible: true,
    })
}

pub fn discover_catalog(stats: &mut ConfigStats) -> Vec<CatalogBundle, MAX_ELIGIBLE_CATALOG> {
    let mut out: Vec<CatalogBundle, MAX_ELIGIBLE_CATALOG> = Vec::new();
    let mut entries: [sunlight_libc::DirEntry; CATALOG_MAX_SCAN] =
        core::array::from_fn(|_| sunlight_libc::DirEntry::zeroed());
    let Ok(count) = sunlight_libc::read_dir(APPS_ROOT, &mut entries) else {
        return out;
    };
    for entry in entries.iter().take(count) {
        let name = &entry.name[..entry.name_len as usize];
        if let Some(bundle) = try_load_bundle_dir(name, stats) {
            let _ = out.push(bundle);
        }
    }
    out
}

pub fn build_frozen_plan(
    manifest: &SessionManifest,
    profile: &SessionProfile,
    catalog: &[CatalogBundle],
    system_release_generation: u32,
    plan_id: u64,
    profile_degraded: bool,
    stats: &mut ConfigStats,
) -> FrozenPlanRuntime {
    let plan = resolve_session_plan(
        manifest,
        profile,
        catalog,
        1,
        catalog.len() as u64,
        system_release_generation,
        plan_id,
        profile_degraded,
    );
    stats.resolved_plans_created = stats.resolved_plans_created.saturating_add(1);
    let mut optionals: Vec<OptionalRuntime, MAX_SESSION_COMPONENTS> = Vec::new();
    for c in plan.components.iter() {
        if c.kind != ResolvedComponentKind::OptionalStartup {
            continue;
        }
        stats.startup_entries_resolved = stats.startup_entries_resolved.saturating_add(1);
        let mut app_id = heapless::String::<32>::new();
        if app_id.push_str(c.app_id.as_str()).is_err() {
            continue;
        }
        let mut launch_path = String::<MAX_LAUNCH_PATH>::new();
        if launch_path.push_str(c.launch_path.as_str()).is_err() {
            continue;
        }
        let _ = optionals.push(OptionalRuntime {
            component_id: c.component_id,
            app_id,
            entry_id: c.entry_id.map(|e| e.get()),
            process_id: None,
            process_generation: None,
            state: SessionComponentState::Pending,
            launch_result: None,
            restart_policy: c.restart_policy,
            restart_used: false,
            single_instance: c.single_instance,
            completion_mode: c.completion_mode,
            launch_path,
            ready_deadline_ms: 0,
            order: c.order,
            app_completion_recorded: false,
        });
    }
    let mut frozen = FrozenPlanRuntime {
        plan,
        optionals,
        next_optional_index: 0,
        optionals_started: false,
        shell_first_logged: false,
        ordering_logged: false,
        optionals_not_before_ms: 0,
        next_optional_not_before_ms: 0,
    };
    // Failsafe: if Welcome is catalog-available and pending in the profile but
    // missing from the frozen optionals (e.g. earlier resolve edge case), inject it.
    repair_welcome_optional(&mut frozen, profile, catalog, system_release_generation);
    frozen
}

/// Canonical Welcome bundle id (short).
pub const WELCOME_APP_ID: &str = "org.sunlight.welcome";
/// Legacy Phase 1 id (longer); still recognized for migration.
pub const WELCOME_APP_ID_LEGACY: &str = "org.sunlight.wiseowl-welcome";

fn is_welcome_app_id(id: &str) -> bool {
    id == WELCOME_APP_ID || id == WELCOME_APP_ID_LEGACY || id.starts_with("org.sunlight.wiseowl")
}

/// Ensure Welcome is scheduled when the catalog has a launchable bundle and
/// policy still wants a run. Uses catalog as source of truth for id + path.
fn repair_welcome_optional(
    frozen: &mut FrozenPlanRuntime,
    profile: &SessionProfile,
    catalog: &[CatalogBundle],
    system_generation: u32,
) {
    if frozen.optionals.iter().any(|o| is_welcome_app_id(o.app_id.as_str())) {
        return;
    }
    let Some(bundle) = catalog.iter().find(|b| is_welcome_app_id(b.app_id.as_str())) else {
        debug_log("[SESSION-CONFIG] welcome not in catalog\n");
        return;
    };
    if bundle.availability != StartupAvailability::Available
        || !bundle.startup_eligible
        || bundle.launch_path.is_empty()
        || bundle.launch_path.len() >= 32
    {
        debug_log("[SESSION-CONFIG] welcome catalog entry not launchable\n");
        return;
    }

    // Prefer profile entry for policy state; if missing/mismatched id, still
    // launch once when no completed generation is recorded for any welcome-like entry.
    let entry = profile
        .entries
        .iter()
        .find(|e| is_welcome_app_id(e.app_id.as_str()));
    let state = entry.and_then(|e| {
        profile
            .policy_state
            .iter()
            .find(|p| p.entry_id == e.entry_id)
    });
    let should = match entry {
        Some(e) => sunlight_sessiond::policy_should_launch(e, state, system_generation),
        None => {
            // No profile row: allow first auto-launch (seed should have added one).
            true
        }
    };
    if !should {
        debug_log("[SESSION-CONFIG] welcome policy complete; not repairing\n");
        return;
    }

    let mut app_id = heapless::String::<32>::new();
    if app_id.push_str(bundle.app_id.as_str()).is_err() {
        return;
    }
    let mut launch_path = String::<MAX_LAUNCH_PATH>::new();
    if launch_path
        .push_str(bundle.launch_path.as_str())
        .is_err()
    {
        return;
    }
    let next_id = frozen
        .optionals
        .iter()
        .map(|o| o.component_id)
        .chain(core::iter::once(1u64))
        .max()
        .unwrap_or(1)
        .saturating_add(1);
    let entry_id = entry.map(|e| e.entry_id.get());
    let order = entry.map(|e| e.order).unwrap_or(0);
    let restart = entry
        .map(|e| e.restart_policy)
        .unwrap_or(OptionalComponentRestartPolicy::Never);
    if frozen
        .optionals
        .push(OptionalRuntime {
            component_id: next_id,
            app_id,
            entry_id,
            process_id: None,
            process_generation: None,
            state: SessionComponentState::Pending,
            launch_result: None,
            restart_policy: restart,
            restart_used: false,
            single_instance: bundle.single_instance,
            completion_mode: bundle.completion_mode,
            launch_path,
            ready_deadline_ms: 0,
            order,
            app_completion_recorded: false,
        })
        .is_ok()
    {
        debug_log("[SESSION-CONFIG] welcome optional repaired into frozen plan\n");
        config_log("WELCOME_PLAN_REPAIRED PASS");
    }
}

/// After Shell Ready: schedule optional launches so the desktop paints first.
pub fn schedule_optionals_after_shell_ready(frozen: &mut FrozenPlanRuntime, now: u64) {
    if frozen.optionals_not_before_ms != 0 {
        return; // already scheduled
    }
    if frozen.optionals.is_empty() {
        debug_log("[SESSION-CONFIG] no optionals to schedule after shell ready\n");
        return;
    }
    frozen.optionals_not_before_ms = now.saturating_add(DESKTOP_SETTLE_MS);
    frozen.next_optional_not_before_ms = frozen.optionals_not_before_ms;
    if !frozen.shell_first_logged {
        config_log("SHELL_FIRST PASS");
        frozen.shell_first_logged = true;
    }
    debug_log("[SESSION-CONFIG] optionals deferred until desktop settle\n");
}

/// Launch at most one optional per call once the desktop settle deadline has passed.
/// Returns true if a launch attempt was made (success or fail).
pub fn try_launch_deferred_optional(
    frozen: &mut FrozenPlanRuntime,
    uid: u32,
    gid: u32,
    now: u64,
    stats: &mut ConfigStats,
) -> bool {
    if frozen.optionals.is_empty() {
        return false;
    }
    if frozen.optionals_not_before_ms == 0 {
        return false; // not scheduled yet (shell not ready)
    }
    if now < frozen.optionals_not_before_ms {
        return false;
    }
    if now < frozen.next_optional_not_before_ms {
        return false;
    }
    if frozen.next_optional_index >= frozen.optionals.len() {
        if !frozen.ordering_logged && frozen.optionals_started {
            config_log("ORDERING PASS");
            frozen.ordering_logged = true;
        }
        return false;
    }
    let launched = launch_next_optional(frozen, uid, gid, now, stats);
    if launched {
        frozen.next_optional_not_before_ms = now.saturating_add(OPTIONAL_STAGGER_MS);
    }
    launched
}

pub fn spawn_optional(
    path: &str,
    name: &str,
    uid: u32,
    gid: u32,
) -> Result<(u64, u64), u64> {
    let Some(spawn_cap) = nameserver_lookup("spawn") else {
        return Err(SessionMsg::ERR_INVALID_STATE);
    };
    let req = SpawnRequest::new(path, name)
        .with_identity(uid, gid)
        .with_service_caps(ServiceCapability::UserSession.bit());
    let mut msg = IpcMsg::empty();
    req.pack_into(&mut msg);
    let reply = match ipc_call_timeout(spawn_cap, msg, SPAWN_OPTIONAL_TIMEOUT_MS) {
        Ok(r) => r,
        Err(_) => return Err(SessionMsg::ERR_INVALID_STATE),
    };
    if reply.label != SpawnMsg::REPLY {
        return Err(SessionMsg::ERR_INVALID_STATE);
    }
    let pid = reply.words[0];
    let gen = session_query_process(pid)
        .map(|info| info.generation)
        .unwrap_or(1);
    Ok((pid, gen))
}

/// After a successful optional launch, complete first-login style policies.
pub fn complete_policy_after_success(
    profile: &mut SessionProfile,
    entry_id: Option<u64>,
    system_generation: u32,
    now: u64,
    stats: &mut ConfigStats,
) {
    let Some(eid) = entry_id else {
        return;
    };
    let Some(entry_id) = sunlight_sessiond::StartupEntryId::new(eid) else {
        return;
    };
    let before = profile
        .policy_state
        .iter()
        .find(|p| p.entry_id == entry_id)
        .cloned();
    mark_policy_success(profile, entry_id, system_generation, now);
    if let Some(after) = profile.policy_state.iter().find(|p| p.entry_id == entry_id) {
        if before
            .as_ref()
            .map(|b| !b.completed_first_login && after.completed_first_login)
            .unwrap_or(false)
        {
            stats.first_login_completions = stats.first_login_completions.saturating_add(1);
            config_log("FIRST_LOGIN_POLICY PASS");
        }
        if before
            .as_ref()
            .map(|b| b.completed_system_generation != after.completed_system_generation)
            .unwrap_or(false)
        {
            stats.upgrade_login_completions = stats.upgrade_login_completions.saturating_add(1);
        }
    }
    let _ = persist_profile(profile);
}

pub fn launch_next_optional(
    frozen: &mut FrozenPlanRuntime,
    uid: u32,
    gid: u32,
    now: u64,
    stats: &mut ConfigStats,
) -> bool {
    if frozen.next_optional_index >= frozen.optionals.len() {
        if !frozen.ordering_logged && frozen.optionals_started {
            config_log("ORDERING PASS");
            frozen.ordering_logged = true;
        }
        return false;
    }
    // shell_first is logged at schedule time so the desktop settles first.
    frozen.optionals_started = true;
    let idx = frozen.next_optional_index;
    frozen.next_optional_index += 1;
    // Single-instance: skip if same app_id already has a live process in this plan.
    let app = frozen.optionals[idx].app_id.clone();
    let single = frozen.optionals[idx].single_instance;
    if single {
        let duplicate = frozen.optionals.iter().enumerate().any(|(i, other)| {
            i < idx
                && other.app_id == app
                && other.process_id.map(process_is_alive).unwrap_or(false)
        });
        if duplicate {
            let opt = &mut frozen.optionals[idx];
            opt.launch_result = Some(StartupLaunchResult::SkippedDuplicateInstance);
            opt.state = SessionComponentState::Disabled;
            stats.startup_apps_duplicate_skipped =
                stats.startup_apps_duplicate_skipped.saturating_add(1);
            return true;
        }
    }
    let path_owned = frozen.optionals[idx].launch_path.clone();
    let path = path_owned.as_str();
    let is_welcome = is_welcome_app_id(frozen.optionals[idx].app_id.as_str());
    let name = if path.ends_with("su1") {
        "su1"
    } else if path.ends_with("su2") {
        "su2"
    } else if path.ends_with("welcome") {
        "welcome"
    } else {
        "startup-app"
    };
    match spawn_optional(path, name, uid, gid) {
        Ok((pid, gen)) => {
            let opt = &mut frozen.optionals[idx];
            opt.process_id = Some(pid);
            opt.process_generation = Some(gen);
            opt.ready_deadline_ms = now.saturating_add(OPTIONAL_READY_TIMEOUT_MS);
            opt.launch_result = Some(StartupLaunchResult::RunningWithoutReadinessProtocol);
            opt.state = SessionComponentState::Running;
            stats.startup_apps_launched = stats.startup_apps_launched.saturating_add(1);
            stats.startup_apps_ready = stats.startup_apps_ready.saturating_add(1);
            config_log("NEXT_LOGIN_LAUNCH PASS");
            if is_welcome {
                config_log("WELCOME_SESSION_ELIGIBLE PASS");
                // Shell-first is already logged; welcome is always after shell ready.
                debug_log("[WELCOME-WIZARD] BUNDLE_DISCOVERED PASS\n");
                debug_log("[WELCOME-WIZARD] SESSION_ELIGIBLE PASS\n");
                debug_log("[WELCOME-WIZARD] SHELL_READY_FIRST PASS\n");
            }
            true
        }
        Err(_) => {
            let opt = &mut frozen.optionals[idx];
            opt.launch_result = Some(StartupLaunchResult::FailedToLaunch);
            opt.state = SessionComponentState::Failed;
            stats.startup_apps_failed = stats.startup_apps_failed.saturating_add(1);
            config_log("OPTIONAL_FAILURE_ISOLATION PASS");
            true
        }
    }
}

pub fn supervise_optionals(
    frozen: &mut FrozenPlanRuntime,
    profile: &mut SessionProfile,
    system_generation: u32,
    now: u64,
    stats: &mut ConfigStats,
    stopping: bool,
) {
    for opt in frozen.optionals.iter_mut() {
        let Some(pid) = opt.process_id else {
            continue;
        };
        if !process_is_alive(pid) {
            opt.process_id = None;
            opt.process_generation = None;
            if stopping {
                opt.state = SessionComponentState::Exited;
                continue;
            }
            if opt.state == SessionComponentState::Running
                || opt.state == SessionComponentState::Starting
                || opt.state == SessionComponentState::Ready
            {
                // ProcessSuccess only: short-lived fixtures complete on exit.
                // AppReported apps must call SESSION_STARTUP_COMPLETE instead.
                if opt.completion_mode
                    == sunlight_sessiond::StartupCompletionMode::ProcessSuccess
                    && opt
                        .launch_result
                        .map(|r| r.is_success())
                        .unwrap_or(false)
                {
                    if let Some(eid) = opt.entry_id {
                        if let Some(entry_id) = sunlight_sessiond::StartupEntryId::new(eid) {
                            let before = profile
                                .policy_state
                                .iter()
                                .find(|p| p.entry_id == entry_id)
                                .cloned();
                            mark_policy_success(profile, entry_id, system_generation, now);
                            if let Some(after) = profile
                                .policy_state
                                .iter()
                                .find(|p| p.entry_id == entry_id)
                            {
                                if before
                                    .as_ref()
                                    .map(|b| !b.completed_first_login && after.completed_first_login)
                                    .unwrap_or(false)
                                {
                                    stats.first_login_completions =
                                        stats.first_login_completions.saturating_add(1);
                                    config_log("FIRST_LOGIN_POLICY PASS");
                                }
                                if before
                                    .as_ref()
                                    .map(|b| {
                                        b.completed_system_generation
                                            != after.completed_system_generation
                                    })
                                    .unwrap_or(false)
                                {
                                    stats.upgrade_login_completions =
                                        stats.upgrade_login_completions.saturating_add(1);
                                }
                            }
                            let _ = persist_profile(profile);
                        }
                    }
                }
                // Optional restart at most once.
                if !stopping
                    && opt.restart_policy == OptionalComponentRestartPolicy::OnFailureOnce
                    && !opt.restart_used
                    && opt.launch_result != Some(StartupLaunchResult::RunningWithoutReadinessProtocol)
                {
                    opt.restart_used = true;
                    opt.state = SessionComponentState::RestartPending;
                } else {
                    opt.state = SessionComponentState::Exited;
                }
            }
        } else if opt.state == SessionComponentState::Starting && now >= opt.ready_deadline_ms {
            opt.launch_result = Some(StartupLaunchResult::ReadinessTimeout);
            stats.startup_apps_readiness_timeout =
                stats.startup_apps_readiness_timeout.saturating_add(1);
            let _ = sunlight_ipc::kill(pid, 15);
            opt.state = SessionComponentState::Failed;
            // Do not complete first-login policy on timeout.
        }
    }
    // Process one restart if pending.
    if stopping {
        return;
    }
    for opt in frozen.optionals.iter_mut() {
        if opt.state == SessionComponentState::RestartPending {
            let path = opt.launch_path.as_str();
            let name = "startup-app";
            // Need uid/gid from caller — restart handled by outer with identity.
            let _ = (path, name);
        }
    }
}

pub fn stop_optionals(frozen: &mut FrozenPlanRuntime) {
    for opt in frozen.optionals.iter_mut() {
        if let Some(pid) = opt.process_id {
            let _ = sunlight_ipc::kill(pid, 15);
            opt.state = SessionComponentState::Stopping;
        }
    }
}

pub fn profile_error_code(err: ProfileError) -> u64 {
    match err {
        ProfileError::RevisionConflict => SessionMsg::ERR_PROFILE_REVISION,
        ProfileError::ChecksumFailure | ProfileError::UnsupportedVersion => {
            SessionMsg::ERR_PROFILE_CORRUPT
        }
        ProfileError::NotFound => SessionMsg::ERR_NOT_FOUND,
        ProfileError::IneligibleBundle | ProfileError::UnavailableBundle => {
            SessionMsg::ERR_INELIGIBLE
        }
        ProfileError::DuplicateBundleId | ProfileError::DuplicateEntryId => SessionMsg::ERR_DUPLICATE,
        ProfileError::TooManyEntries => SessionMsg::ERR_LIMIT,
        ProfileError::ShellAsOptional
        | ProfileError::ExecutablePathRejected
        | ProfileError::MalformedBundleId => SessionMsg::ERR_INVALID_ARGUMENT,
        ProfileError::WrongUser => SessionMsg::ERR_UNAUTHORIZED,
        _ => SessionMsg::ERR_INVALID_ARGUMENT,
    }
}

/// Pack app_id string into words[2] and words[3] (up to 16 bytes).
/// Pack up to 32 app-id bytes into `words[2..6]` (4 words × 8 bytes).
pub fn pack_app_id(msg: &mut IpcMsg, app_id: &str) {
    let bytes = app_id.as_bytes();
    let len = bytes.len().min(32);
    for w in 2..6 {
        msg.words[w] = 0;
    }
    for (i, b) in bytes.iter().take(len).enumerate() {
        let word = 2 + i / 8;
        let shift = (i % 8) * 8;
        msg.words[word] |= (*b as u64) << shift;
    }
    if msg.word_count < 6 {
        msg.word_count = 6;
    }
}

pub fn unpack_app_id(msg: &IpcMsg, len: usize) -> String<32> {
    let mut out = String::new();
    let len = len.min(32);
    for i in 0..len {
        let word = 2 + i / 8;
        let shift = (i % 8) * 8;
        if word >= IPC_MAX_WORDS_LOCAL {
            break;
        }
        let b = ((msg.words[word] >> shift) & 0xff) as u8;
        if b == 0 {
            break;
        }
        let _ = out.push(b as char);
    }
    out
}

const IPC_MAX_WORDS_LOCAL: usize = 8;

pub fn authorize_own_profile(
    caller: SessionProcessCredentials,
    target_uid: u32,
) -> bool {
    caller.uid == 0 || caller.uid == target_uid
}

pub fn apply_mutation(
    profile: &mut SessionProfile,
    op: u64,
    app_id: &str,
    policy_raw: u8,
    direction: i8,
    expected_revision: u64,
    now: u64,
    catalog: &[CatalogBundle],
    stats: &mut ConfigStats,
) -> Result<(), ProfileError> {
    match op {
        SessionMsg::SESSION_PROFILE_ADD_APP => {
            validate_bundle_id(app_id)?;
            let eligible = catalog.iter().any(|b| {
                b.app_id.as_str() == app_id
                    && b.startup_eligible
                    && b.availability == StartupAvailability::Available
            });
            if !eligible {
                return Err(ProfileError::IneligibleBundle);
            }
            let policy = StartupPolicy::from_u8(policy_raw).unwrap_or(StartupPolicy::EveryLogin);
            profile_add_app(profile, app_id, policy, expected_revision, now)?;
            stats.profile_updates = stats.profile_updates.saturating_add(1);
            config_log("ADD_APP PASS");
        }
        SessionMsg::SESSION_PROFILE_REMOVE_APP => {
            profile_remove_app(profile, app_id, expected_revision, now)?;
            stats.profile_updates = stats.profile_updates.saturating_add(1);
        }
        SessionMsg::SESSION_PROFILE_ENABLE_APP => {
            profile_set_enabled(profile, app_id, true, expected_revision, now)?;
            stats.profile_updates = stats.profile_updates.saturating_add(1);
        }
        SessionMsg::SESSION_PROFILE_DISABLE_APP => {
            profile_set_enabled(profile, app_id, false, expected_revision, now)?;
            stats.profile_updates = stats.profile_updates.saturating_add(1);
            config_log("DISABLE_APP PASS");
        }
        SessionMsg::SESSION_PROFILE_SET_POLICY => {
            let policy =
                StartupPolicy::from_u8(policy_raw).ok_or(ProfileError::InvalidPolicy)?;
            profile_set_policy(profile, app_id, policy, expected_revision, now)?;
            stats.profile_updates = stats.profile_updates.saturating_add(1);
        }
        SessionMsg::SESSION_PROFILE_REORDER => {
            profile_move(profile, app_id, direction, expected_revision, now)?;
            stats.profile_updates = stats.profile_updates.saturating_add(1);
        }
        SessionMsg::SESSION_PROFILE_RESET => {
            profile_reset(profile, expected_revision, now)?;
            stats.profile_resets = stats.profile_resets.saturating_add(1);
            config_log("RESET_DEFAULTS PASS");
        }
        _ => return Err(ProfileError::UnsupportedField),
    }
    normalize_orders(profile);
    persist_profile(profile)?;
    Ok(())
}

pub fn pack_profile_summary(profile: &SessionProfile, degraded: bool) -> IpcMsg {
    IpcMsg::with_label(SessionMsg::REPLY)
        .word(0, profile.revision)
        .word(1, profile.user_id as u64)
        .word(
            2,
            (profile.entries.len() as u64)
                | ((profile.format_version as u64) << 16)
                | ((degraded as u64) << 32)
                | ((profile.profile_id.get() & 0xffff) << 40),
        )
        .word(3, profile.updated_at)
}

pub fn pack_eligible_entry(index: u64, catalog: &[CatalogBundle], profile: &SessionProfile) -> IpcMsg {
    let list = list_eligible(catalog, profile);
    if index as usize >= list.len() {
        return IpcMsg::with_label(SessionMsg::ERROR).word(0, SessionMsg::ERR_NOT_FOUND);
    }
    let e = &list[index as usize];
    let mut msg = IpcMsg::with_label(SessionMsg::REPLY)
        .word(0, index)
        .word(
            1,
            (list.len() as u64)
                | ((e.default_policy as u64) << 16)
                | ((e.single_instance as u64) << 24)
                | ((e.currently_configured as u64) << 32)
                | ((e.availability as u64) << 40),
        );
    pack_app_id(&mut msg, e.app_id.as_str());
    // Overlay length into word1 high.
    msg.words[1] |= (e.app_id.len().min(32) as u64) << 48;
    msg
}

pub fn pack_startup_entry(index: u64, profile: &SessionProfile) -> IpcMsg {
    if index as usize >= profile.entries.len() {
        return IpcMsg::with_label(SessionMsg::ERROR).word(0, SessionMsg::ERR_NOT_FOUND);
    }
    // Present entries sorted by order.
    let mut indices: Vec<usize, MAX_STARTUP_ENTRIES> = Vec::new();
    for i in 0..profile.entries.len() {
        let _ = indices.push(i);
    }
    for i in 1..indices.len() {
        let mut j = i;
        while j > 0
            && profile.entries[indices[j]].order < profile.entries[indices[j - 1]].order
        {
            indices.swap(j - 1, j);
            j -= 1;
        }
    }
    let e = &profile.entries[indices[index as usize]];
    let mut msg = IpcMsg::with_label(SessionMsg::REPLY)
        .word(0, index)
        .word(
            1,
            (profile.entries.len() as u64)
                | ((e.enabled as u64) << 16)
                | ((e.policy as u64) << 24)
                | ((e.order as u64) << 32)
                | ((profile.revision & 0xffff) << 48),
        );
    pack_app_id(&mut msg, e.app_id.as_str());
    msg.words[1] = (profile.entries.len() as u64)
        | ((e.app_id.len().min(32) as u64) << 8)
        | ((e.enabled as u64) << 16)
        | ((e.policy as u64) << 24)
        | ((e.order as u64) << 32)
        | ((profile.revision & 0xffff) << 48);
    msg
}

/// Record app-reported onboarding completion for a live optional component.
///
/// Caller must be the process currently tracked for `app_id` in the frozen plan.
/// Returns true when policy state was updated.
pub fn complete_app_reported(
    frozen: &mut FrozenPlanRuntime,
    profile: &mut SessionProfile,
    caller_pid: u64,
    app_id: &str,
    system_generation: u32,
    now: u64,
    stats: &mut ConfigStats,
) -> Result<(), u64> {
    use sunlight_sessiond::StartupCompletionMode;
    // Accept exact id or welcome-family id (legacy / repaired entries).
    let Some(opt) = frozen.optionals.iter_mut().find(|o| {
        o.app_id.as_str() == app_id || (is_welcome_app_id(app_id) && is_welcome_app_id(o.app_id.as_str()))
    }) else {
        debug_log("[SESSION-CONFIG] complete: optional not found for app\n");
        return Err(SessionMsg::ERR_NOT_FOUND);
    };
    if opt.completion_mode != StartupCompletionMode::AppReported {
        // Welcome may have been scheduled with default ProcessSuccess if
        // catalog parse missed completion_mode — still accept the report.
        if !is_welcome_app_id(opt.app_id.as_str()) {
            return Err(SessionMsg::ERR_INVALID_ARGUMENT);
        }
    }
    if opt.process_id != Some(caller_pid) {
        // Still record completion if this pid is the live welcome process.
        if !is_welcome_app_id(opt.app_id.as_str())
            || !process_is_alive(caller_pid)
        {
            debug_log("[SESSION-CONFIG] complete: pid mismatch\n");
            return Err(SessionMsg::ERR_UNAUTHORIZED);
        }
        opt.process_id = Some(caller_pid);
    }
    if opt.app_completion_recorded {
        return Ok(());
    }
    let entry_id = opt.entry_id;
    opt.app_completion_recorded = true;
    complete_policy_after_success(profile, entry_id, system_generation, now, stats);
    config_log("APP_COMPLETION_RECORDED PASS");
    // Prove next login would skip for the same generation.
    if let Some(entry) = profile.entries.iter().find(|e| {
        entry_id
            .and_then(sunlight_sessiond::StartupEntryId::new)
            .map(|id| e.entry_id == id)
            .unwrap_or(false)
    }) {
        let state = profile
            .policy_state
            .iter()
            .find(|p| p.entry_id == entry.entry_id);
        if !sunlight_sessiond::policy_should_launch(entry, state, system_generation) {
            debug_log("[WELCOME-WIZARD] NO_REPEAT_AFTER_COMPLETION PASS\n");
        }
    }
    Ok(())
}

pub fn pack_preview_plan(plan: &ResolvedSessionPlan) -> IpcMsg {
    let optional_count = plan
        .components
        .iter()
        .filter(|c| c.kind == ResolvedComponentKind::OptionalStartup)
        .count();
    IpcMsg::with_label(SessionMsg::REPLY)
        .word(0, plan.plan_id.get())
        .word(1, plan.profile_revision)
        .word(
            2,
            (plan.components.len() as u64)
                | ((optional_count as u64) << 16)
                | ((plan.profile_degraded as u64) << 32),
        )
        .word(3, plan.plan_digest)
}
