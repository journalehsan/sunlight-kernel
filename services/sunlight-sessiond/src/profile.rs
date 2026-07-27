//! Per-user Session Profile contracts, plan resolution, and bundle eligibility.
//!
//! Host-testable pure logic. Persistence and IPC live in the sessiond binary.

use core::fmt::Write;
use heapless::{String, Vec};

use crate::{ManifestComponent, SessionManifest, MAX_APP_ID};

/// Maximum optional Startup Apps a user may configure.
pub const MAX_STARTUP_ENTRIES: usize = 16;
/// Required components + optional startup apps in a resolved plan.
pub const MAX_SESSION_COMPONENTS: usize = 20;
/// Maximum eligible apps returned in one catalog page.
pub const MAX_ELIGIBLE_CATALOG: usize = 32;
/// Profile format version for this phase.
pub const SESSION_PROFILE_FORMAT_VERSION: u16 = 1;
/// Plan format version for this phase.
pub const SESSION_PLAN_FORMAT_VERSION: u16 = 1;
/// Canonical reorder spacing (0, 1, 2, …).
pub const ORDER_STEP: u16 = 1;
/// Protected shell bundle id — never an optional startup entry.
pub const PROTECTED_SHELL_APP_ID: &str = "org.sunlight.vortex-shell";
/// Stable system release generation used by FirstLoginAfterSystemUpgrade.
/// Not a per-build timestamp; bump only on intentional onboarding generations.
pub const DEFAULT_SYSTEM_RELEASE_GENERATION: u32 = 1;

pub const MAX_DISPLAY_NAME: usize = 48;
pub const MAX_VERSION: usize = 24;
pub const MAX_ICON_REF: usize = 64;
pub const MAX_PUBLISHER: usize = 48;
pub const MAX_LAUNCH_PATH: usize = 96;
pub const MAX_BUNDLE_DIR: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionProfileId(u64);

impl SessionProfileId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupEntryId(u64);

impl StartupEntryId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionPlanId(u64);

impl SessionPlanId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StartupPolicy {
    EveryLogin = 1,
    FirstLoginOnly = 2,
    FirstLoginAfterSystemUpgrade = 3,
    Disabled = 4,
}

impl StartupPolicy {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::EveryLogin),
            2 => Some(Self::FirstLoginOnly),
            3 => Some(Self::FirstLoginAfterSystemUpgrade),
            4 => Some(Self::Disabled),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EveryLogin => "every-login",
            Self::FirstLoginOnly => "first-login-only",
            Self::FirstLoginAfterSystemUpgrade => "first-login-after-system-upgrade",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "every-login" | "EveryLogin" => Some(Self::EveryLogin),
            "first-login-only" | "FirstLoginOnly" => Some(Self::FirstLoginOnly),
            "first-login-after-system-upgrade" | "FirstLoginAfterSystemUpgrade" => {
                Some(Self::FirstLoginAfterSystemUpgrade)
            }
            "disabled" | "Disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StartupLaunchPhase {
    AfterShellReady = 1,
}

/// How one-time startup policies complete for an optional app.
///
/// `ProcessSuccess` (default): successful spawn/short-lived exit completes
/// FirstLogin* policies (Session Configuration Phase 1 fixtures).
///
/// `AppReported`: launch alone never completes the policy; the app must send
/// `SESSION_STARTUP_COMPLETE` after the user finishes onboarding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StartupCompletionMode {
    ProcessSuccess = 1,
    AppReported = 2,
}

impl StartupCompletionMode {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::ProcessSuccess),
            2 => Some(Self::AppReported),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProcessSuccess => "process-success",
            Self::AppReported => "app-reported",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "process-success" | "ProcessSuccess" => Some(Self::ProcessSuccess),
            "app-reported" | "AppReported" | "wizard-finished" | "WizardFinished" => {
                Some(Self::AppReported)
            }
            _ => None,
        }
    }
}

impl StartupLaunchPhase {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::AfterShellReady),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AfterShellReady => "after-shell-ready",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum OptionalComponentRestartPolicy {
    Never = 1,
    OnFailureOnce = 2,
}

impl OptionalComponentRestartPolicy {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Never),
            2 => Some(Self::OnFailureOnce),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::OnFailureOnce => "on-failure-once",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StartupLaunchResult {
    Ready = 1,
    RunningWithoutReadinessProtocol = 2,
    SkippedDisabled = 3,
    SkippedPolicyComplete = 4,
    SkippedUnavailable = 5,
    SkippedInvalidBundle = 6,
    SkippedDuplicateInstance = 7,
    FailedToLaunch = 8,
    ReadinessTimeout = 9,
    ExitedBeforeReady = 10,
}

impl StartupLaunchResult {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Ready),
            2 => Some(Self::RunningWithoutReadinessProtocol),
            3 => Some(Self::SkippedDisabled),
            4 => Some(Self::SkippedPolicyComplete),
            5 => Some(Self::SkippedUnavailable),
            6 => Some(Self::SkippedInvalidBundle),
            7 => Some(Self::SkippedDuplicateInstance),
            8 => Some(Self::FailedToLaunch),
            9 => Some(Self::ReadinessTimeout),
            10 => Some(Self::ExitedBeforeReady),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::RunningWithoutReadinessProtocol => "running-without-readiness",
            Self::SkippedDisabled => "skipped-disabled",
            Self::SkippedPolicyComplete => "skipped-policy-complete",
            Self::SkippedUnavailable => "skipped-unavailable",
            Self::SkippedInvalidBundle => "skipped-invalid-bundle",
            Self::SkippedDuplicateInstance => "skipped-duplicate",
            Self::FailedToLaunch => "failed-to-launch",
            Self::ReadinessTimeout => "readiness-timeout",
            Self::ExitedBeforeReady => "exited-before-ready",
        }
    }

    pub const fn is_success(self) -> bool {
        matches!(
            self,
            Self::Ready | Self::RunningWithoutReadinessProtocol
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StartupAvailability {
    Available = 1,
    Missing = 2,
    InvalidManifest = 3,
    UnsupportedArchitecture = 4,
    DisabledByPolicy = 5,
    IncompleteInstallation = 6,
}

impl StartupAvailability {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Available),
            2 => Some(Self::Missing),
            3 => Some(Self::InvalidManifest),
            4 => Some(Self::UnsupportedArchitecture),
            5 => Some(Self::DisabledByPolicy),
            6 => Some(Self::IncompleteInstallation),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Missing => "missing",
            Self::InvalidManifest => "invalid-manifest",
            Self::UnsupportedArchitecture => "unsupported-architecture",
            Self::DisabledByPolicy => "disabled-by-policy",
            Self::IncompleteInstallation => "incomplete-installation",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ResolvedComponentKind {
    RequiredShell = 1,
    OptionalStartup = 2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupEntry {
    pub entry_id: StartupEntryId,
    pub app_id: String<MAX_APP_ID>,
    pub enabled: bool,
    pub policy: StartupPolicy,
    pub launch_phase: StartupLaunchPhase,
    pub order: u16,
    pub restart_policy: OptionalComponentRestartPolicy,
    pub added_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupPolicyState {
    pub entry_id: StartupEntryId,
    pub app_id: String<MAX_APP_ID>,
    pub completed_first_login: bool,
    pub completed_system_generation: Option<u32>,
    pub last_successful_start_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionProfile {
    pub format_version: u16,
    pub profile_id: SessionProfileId,
    pub user_id: u32,
    pub base_session_id: String<48>,
    pub revision: u64,
    pub entries: Vec<StartupEntry, MAX_STARTUP_ENTRIES>,
    pub policy_state: Vec<StartupPolicyState, MAX_STARTUP_ENTRIES>,
    pub created_at: u64,
    pub updated_at: u64,
    pub checksum: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSessionComponent {
    pub component_id: u64,
    pub app_id: String<MAX_APP_ID>,
    pub kind: ResolvedComponentKind,
    pub required: bool,
    pub launch_phase: StartupLaunchPhase,
    pub order: u16,
    pub entry_id: Option<StartupEntryId>,
    pub restart_policy: OptionalComponentRestartPolicy,
    pub launch_path: String<MAX_LAUNCH_PATH>,
    pub single_instance: bool,
    pub completion_mode: StartupCompletionMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSessionPlan {
    pub format_version: u16,
    pub plan_id: SessionPlanId,
    pub profile_revision: u64,
    pub system_manifest_revision: u64,
    pub bundle_catalog_generation: u64,
    pub components: Vec<ResolvedSessionComponent, MAX_SESSION_COMPONENTS>,
    pub plan_digest: u64,
    pub profile_degraded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EligibleStartupApplication {
    pub app_id: String<MAX_APP_ID>,
    pub display_name: String<MAX_DISPLAY_NAME>,
    pub version: String<MAX_VERSION>,
    pub icon_reference: Option<String<MAX_ICON_REF>>,
    pub publisher: Option<String<MAX_PUBLISHER>>,
    pub default_policy: StartupPolicy,
    pub single_instance: bool,
    pub currently_configured: bool,
    pub availability: StartupAvailability,
    pub launch_path: String<MAX_LAUNCH_PATH>,
    pub bundle_dir: String<MAX_BUNDLE_DIR>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileError {
    UnsupportedVersion,
    WrongUser,
    InvalidBaseSession,
    DuplicateEntryId,
    DuplicateBundleId,
    TooManyEntries,
    InvalidPolicy,
    InvalidLaunchPhase,
    InvalidRestartPolicy,
    InvalidOrder,
    InvalidString,
    ZeroIdentifier,
    MalformedBundleId,
    ShellAsOptional,
    ChecksumFailure,
    UnsupportedField,
    ExecutablePathRejected,
    NotFound,
    RevisionConflict,
    IneligibleBundle,
    UnavailableBundle,
    DisabledByPolicy,
    NotEnabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileLoadStatus {
    Ok,
    Missing,
    Corrupt,
}

/// FNV-1a 32-bit over profile fields that participate in integrity.
pub fn profile_checksum(profile: &SessionProfile) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    let mix = |h: &mut u32, b: u8| {
        *h ^= b as u32;
        *h = h.wrapping_mul(0x0100_0193);
    };
    let mix_u64 = |h: &mut u32, v: u64| {
        for i in 0..8 {
            mix(h, ((v >> (i * 8)) & 0xff) as u8);
        }
    };
    mix_u64(&mut h, profile.format_version as u64);
    mix_u64(&mut h, profile.profile_id.get());
    mix_u64(&mut h, profile.user_id as u64);
    for b in profile.base_session_id.as_bytes() {
        mix(&mut h, *b);
    }
    mix_u64(&mut h, profile.revision);
    mix_u64(&mut h, profile.entries.len() as u64);
    for e in profile.entries.iter() {
        mix_u64(&mut h, e.entry_id.get());
        for b in e.app_id.as_bytes() {
            mix(&mut h, *b);
        }
        mix(&mut h, e.enabled as u8);
        mix(&mut h, e.policy as u8);
        mix(&mut h, e.launch_phase as u8);
        mix_u64(&mut h, e.order as u64);
        mix(&mut h, e.restart_policy as u8);
        mix_u64(&mut h, e.added_at);
    }
    for p in profile.policy_state.iter() {
        mix_u64(&mut h, p.entry_id.get());
        for b in p.app_id.as_bytes() {
            mix(&mut h, *b);
        }
        mix(&mut h, p.completed_first_login as u8);
        mix_u64(
            &mut h,
            p.completed_system_generation.unwrap_or(0) as u64,
        );
        mix_u64(&mut h, p.last_successful_start_at.unwrap_or(0));
    }
    mix_u64(&mut h, profile.created_at);
    mix_u64(&mut h, profile.updated_at);
    h
}

pub fn default_profile(user_id: u32, base_session_id: &str, now: u64) -> Result<SessionProfile, ProfileError> {
    let mut base = String::new();
    base.push_str(base_session_id)
        .map_err(|_| ProfileError::InvalidString)?;
    let mut profile = SessionProfile {
        format_version: SESSION_PROFILE_FORMAT_VERSION,
        profile_id: SessionProfileId::new(1).unwrap(),
        user_id,
        base_session_id: base,
        revision: 1,
        entries: Vec::new(),
        policy_state: Vec::new(),
        created_at: now,
        updated_at: now,
        checksum: 0,
    };
    profile.checksum = profile_checksum(&profile);
    Ok(profile)
}

pub fn validate_bundle_id(id: &str) -> Result<(), ProfileError> {
    if id.is_empty() || id.len() > MAX_APP_ID {
        return Err(ProfileError::MalformedBundleId);
    }
    if id == PROTECTED_SHELL_APP_ID {
        return Err(ProfileError::ShellAsOptional);
    }
    // Reject anything that looks like a filesystem path or command string.
    if id.contains('/') || id.contains('\\') || id.contains(' ') || id.starts_with('.') {
        return Err(ProfileError::ExecutablePathRejected);
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(ProfileError::MalformedBundleId);
    }
    Ok(())
}

pub fn validate_profile(profile: &SessionProfile, expected_user: u32) -> Result<(), ProfileError> {
    if profile.format_version != SESSION_PROFILE_FORMAT_VERSION {
        return Err(ProfileError::UnsupportedVersion);
    }
    if profile.profile_id.get() == 0 {
        return Err(ProfileError::ZeroIdentifier);
    }
    if profile.user_id != expected_user {
        return Err(ProfileError::WrongUser);
    }
    if profile.base_session_id.is_empty() {
        return Err(ProfileError::InvalidBaseSession);
    }
    if profile.entries.len() > MAX_STARTUP_ENTRIES {
        return Err(ProfileError::TooManyEntries);
    }
    for (i, e) in profile.entries.iter().enumerate() {
        if e.entry_id.get() == 0 {
            return Err(ProfileError::ZeroIdentifier);
        }
        validate_bundle_id(e.app_id.as_str())?;
        if StartupPolicy::from_u8(e.policy as u8).is_none() {
            return Err(ProfileError::InvalidPolicy);
        }
        if e.launch_phase != StartupLaunchPhase::AfterShellReady {
            return Err(ProfileError::InvalidLaunchPhase);
        }
        if OptionalComponentRestartPolicy::from_u8(e.restart_policy as u8).is_none() {
            return Err(ProfileError::InvalidRestartPolicy);
        }
        for other in profile.entries.iter().skip(i + 1) {
            if other.entry_id == e.entry_id {
                return Err(ProfileError::DuplicateEntryId);
            }
            if other.app_id == e.app_id {
                return Err(ProfileError::DuplicateBundleId);
            }
        }
    }
    let expected = profile_checksum(profile);
    if profile.checksum != expected {
        return Err(ProfileError::ChecksumFailure);
    }
    Ok(())
}

fn next_entry_id(profile: &SessionProfile) -> StartupEntryId {
    let max = profile
        .entries
        .iter()
        .map(|e| e.entry_id.get())
        .max()
        .unwrap_or(0);
    StartupEntryId::new(max.saturating_add(1).max(1)).unwrap()
}

pub fn normalize_orders(profile: &mut SessionProfile) {
    // Sort by current order then entry_id, then assign 0..n-1.
    let mut indices: Vec<usize, MAX_STARTUP_ENTRIES> = Vec::new();
    for i in 0..profile.entries.len() {
        let _ = indices.push(i);
    }
    // Simple insertion sort on indices.
    for i in 1..indices.len() {
        let mut j = i;
        while j > 0 {
            let a = &profile.entries[indices[j - 1]];
            let b = &profile.entries[indices[j]];
            let less = b.order < a.order
                || (b.order == a.order && b.entry_id.get() < a.entry_id.get());
            if less {
                indices.swap(j - 1, j);
                j -= 1;
            } else {
                break;
            }
        }
    }
    for (order, idx) in indices.iter().enumerate() {
        profile.entries[*idx].order = order as u16;
    }
}

fn bump_revision(profile: &mut SessionProfile, now: u64) {
    profile.revision = profile.revision.saturating_add(1).max(1);
    profile.updated_at = now;
    profile.checksum = profile_checksum(profile);
}

fn ensure_revision(profile: &SessionProfile, expected: u64) -> Result<(), ProfileError> {
    if profile.revision != expected {
        Err(ProfileError::RevisionConflict)
    } else {
        Ok(())
    }
}

pub fn profile_add_app(
    profile: &mut SessionProfile,
    app_id: &str,
    policy: StartupPolicy,
    expected_revision: u64,
    now: u64,
) -> Result<(), ProfileError> {
    ensure_revision(profile, expected_revision)?;
    validate_bundle_id(app_id)?;
    if profile.entries.iter().any(|e| e.app_id.as_str() == app_id) {
        return Err(ProfileError::DuplicateBundleId);
    }
    if profile.entries.len() >= MAX_STARTUP_ENTRIES {
        return Err(ProfileError::TooManyEntries);
    }
    if policy == StartupPolicy::Disabled {
        return Err(ProfileError::InvalidPolicy);
    }
    let entry_id = next_entry_id(profile);
    let order = profile.entries.len() as u16;
    let mut id = String::<MAX_APP_ID>::new();
    id.push_str(app_id)
        .map_err(|_| ProfileError::InvalidString)?;
    let entry = StartupEntry {
        entry_id,
        app_id: id.clone(),
        enabled: true,
        policy,
        launch_phase: StartupLaunchPhase::AfterShellReady,
        order,
        restart_policy: OptionalComponentRestartPolicy::Never,
        added_at: now,
    };
    profile
        .entries
        .push(entry)
        .map_err(|_| ProfileError::TooManyEntries)?;
    let _ = profile.policy_state.push(StartupPolicyState {
        entry_id,
        app_id: id,
        completed_first_login: false,
        completed_system_generation: None,
        last_successful_start_at: None,
    });
    normalize_orders(profile);
    bump_revision(profile, now);
    Ok(())
}

pub fn profile_remove_app(
    profile: &mut SessionProfile,
    app_id: &str,
    expected_revision: u64,
    now: u64,
) -> Result<(), ProfileError> {
    ensure_revision(profile, expected_revision)?;
    let Some(pos) = profile.entries.iter().position(|e| e.app_id.as_str() == app_id) else {
        return Err(ProfileError::NotFound);
    };
    let entry_id = profile.entries[pos].entry_id;
    profile.entries.swap_remove(pos);
    if let Some(ppos) = profile
        .policy_state
        .iter()
        .position(|p| p.entry_id == entry_id)
    {
        profile.policy_state.swap_remove(ppos);
    }
    normalize_orders(profile);
    bump_revision(profile, now);
    Ok(())
}

pub fn profile_set_enabled(
    profile: &mut SessionProfile,
    app_id: &str,
    enabled: bool,
    expected_revision: u64,
    now: u64,
) -> Result<(), ProfileError> {
    ensure_revision(profile, expected_revision)?;
    let Some(entry) = profile.entries.iter_mut().find(|e| e.app_id.as_str() == app_id) else {
        return Err(ProfileError::NotFound);
    };
    entry.enabled = enabled;
    if !enabled {
        entry.policy = StartupPolicy::Disabled;
    } else if entry.policy == StartupPolicy::Disabled {
        entry.policy = StartupPolicy::EveryLogin;
    }
    bump_revision(profile, now);
    Ok(())
}

pub fn profile_set_policy(
    profile: &mut SessionProfile,
    app_id: &str,
    policy: StartupPolicy,
    expected_revision: u64,
    now: u64,
) -> Result<(), ProfileError> {
    ensure_revision(profile, expected_revision)?;
    let Some(entry) = profile.entries.iter_mut().find(|e| e.app_id.as_str() == app_id) else {
        return Err(ProfileError::NotFound);
    };
    entry.policy = policy;
    entry.enabled = policy != StartupPolicy::Disabled;
    // Reset first-login completion when policy is re-armed.
    if matches!(
        policy,
        StartupPolicy::FirstLoginOnly | StartupPolicy::FirstLoginAfterSystemUpgrade
    ) {
        if let Some(state) = profile
            .policy_state
            .iter_mut()
            .find(|p| p.entry_id == entry.entry_id)
        {
            if policy == StartupPolicy::FirstLoginOnly {
                state.completed_first_login = false;
            }
            if policy == StartupPolicy::FirstLoginAfterSystemUpgrade {
                state.completed_system_generation = None;
            }
        }
    }
    bump_revision(profile, now);
    Ok(())
}

pub fn profile_move(
    profile: &mut SessionProfile,
    app_id: &str,
    direction: i8,
    expected_revision: u64,
    now: u64,
) -> Result<(), ProfileError> {
    ensure_revision(profile, expected_revision)?;
    normalize_orders(profile);
    let Some(pos) = profile.entries.iter().position(|e| e.app_id.as_str() == app_id) else {
        return Err(ProfileError::NotFound);
    };
    let order = profile.entries[pos].order as isize;
    let target_order = order + direction as isize;
    if target_order < 0 || target_order as usize >= profile.entries.len() {
        // No-op at ends still consumes revision for deterministic clients.
        bump_revision(profile, now);
        return Ok(());
    }
    let Some(swap_pos) = profile
        .entries
        .iter()
        .position(|e| e.order as isize == target_order)
    else {
        bump_revision(profile, now);
        return Ok(());
    };
    let a = profile.entries[pos].order;
    let b = profile.entries[swap_pos].order;
    profile.entries[pos].order = b;
    profile.entries[swap_pos].order = a;
    normalize_orders(profile);
    bump_revision(profile, now);
    Ok(())
}

pub fn profile_reset(
    profile: &mut SessionProfile,
    expected_revision: u64,
    now: u64,
) -> Result<(), ProfileError> {
    ensure_revision(profile, expected_revision)?;
    profile.entries.clear();
    profile.policy_state.clear();
    bump_revision(profile, now);
    Ok(())
}

/// Mark one-time policies complete only after a successful startup result.
pub fn mark_policy_success(
    profile: &mut SessionProfile,
    entry_id: StartupEntryId,
    system_generation: u32,
    now: u64,
) {
    let Some(entry) = profile.entries.iter().find(|e| e.entry_id == entry_id) else {
        return;
    };
    let policy = entry.policy;
    let Some(state) = profile
        .policy_state
        .iter_mut()
        .find(|p| p.entry_id == entry_id)
    else {
        return;
    };
    state.last_successful_start_at = Some(now);
    match policy {
        StartupPolicy::FirstLoginOnly => {
            state.completed_first_login = true;
        }
        StartupPolicy::FirstLoginAfterSystemUpgrade => {
            state.completed_system_generation = Some(system_generation);
        }
        _ => {}
    }
    profile.checksum = profile_checksum(profile);
}

pub fn policy_should_launch(
    entry: &StartupEntry,
    state: Option<&StartupPolicyState>,
    system_generation: u32,
) -> bool {
    if !entry.enabled || entry.policy == StartupPolicy::Disabled {
        return false;
    }
    match entry.policy {
        StartupPolicy::EveryLogin => true,
        StartupPolicy::FirstLoginOnly => !state.map(|s| s.completed_first_login).unwrap_or(false),
        StartupPolicy::FirstLoginAfterSystemUpgrade => {
            state
                .and_then(|s| s.completed_system_generation)
                .map(|g| g != system_generation)
                .unwrap_or(true)
        }
        StartupPolicy::Disabled => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogBundle {
    pub app_id: String<MAX_APP_ID>,
    pub display_name: String<MAX_DISPLAY_NAME>,
    pub version: String<MAX_VERSION>,
    pub icon_reference: Option<String<MAX_ICON_REF>>,
    pub publisher: Option<String<MAX_PUBLISHER>>,
    pub default_policy: StartupPolicy,
    pub default_enabled: bool,
    pub single_instance: bool,
    pub completion_mode: StartupCompletionMode,
    pub availability: StartupAvailability,
    pub launch_path: String<MAX_LAUNCH_PATH>,
    pub bundle_dir: String<MAX_BUNDLE_DIR>,
    pub startup_eligible: bool,
}

pub fn list_eligible(
    catalog: &[CatalogBundle],
    profile: &SessionProfile,
) -> Vec<EligibleStartupApplication, MAX_ELIGIBLE_CATALOG> {
    let mut out: Vec<EligibleStartupApplication, MAX_ELIGIBLE_CATALOG> = Vec::new();
    // Collect eligible indices then sort by display name then app_id.
    let mut indices: Vec<usize, MAX_ELIGIBLE_CATALOG> = Vec::new();
    for (i, b) in catalog.iter().enumerate() {
        if b.startup_eligible && b.availability == StartupAvailability::Available {
            let _ = indices.push(i);
        }
    }
    for i in 1..indices.len() {
        let mut j = i;
        while j > 0 {
            let a = &catalog[indices[j - 1]];
            let b = &catalog[indices[j]];
            let less = b.display_name.as_str() < a.display_name.as_str()
                || (b.display_name.as_str() == a.display_name.as_str()
                    && b.app_id.as_str() < a.app_id.as_str());
            if less {
                indices.swap(j - 1, j);
                j -= 1;
            } else {
                break;
            }
        }
    }
    for idx in indices.iter() {
        let b = &catalog[*idx];
        let configured = profile
            .entries
            .iter()
            .any(|e| e.app_id.as_str() == b.app_id.as_str());
        let _ = out.push(EligibleStartupApplication {
            app_id: b.app_id.clone(),
            display_name: b.display_name.clone(),
            version: b.version.clone(),
            icon_reference: b.icon_reference.clone(),
            publisher: b.publisher.clone(),
            default_policy: b.default_policy,
            single_instance: b.single_instance,
            currently_configured: configured,
            availability: b.availability,
            launch_path: b.launch_path.clone(),
            bundle_dir: b.bundle_dir.clone(),
        });
    }
    out
}

pub fn resolve_session_plan(
    manifest: &SessionManifest,
    profile: &SessionProfile,
    catalog: &[CatalogBundle],
    system_manifest_revision: u64,
    bundle_catalog_generation: u64,
    system_release_generation: u32,
    plan_id: u64,
    profile_degraded: bool,
) -> ResolvedSessionPlan {
    let mut components: Vec<ResolvedSessionComponent, MAX_SESSION_COMPONENTS> = Vec::new();
    let mut next_id = 1u64;

    // 1–2. Required components from system manifest (shell first by order).
    let mut required: Vec<&ManifestComponent, 8> = Vec::new();
    for c in manifest.components.iter() {
        if c.required && c.enabled {
            let _ = required.push(c);
        }
    }
    for i in 1..required.len() {
        let mut j = i;
        while j > 0 && required[j].order < required[j - 1].order {
            required.swap(j - 1, j);
            j -= 1;
        }
    }
    for c in required.iter() {
        let mut app_id = String::new();
        let _ = app_id.push_str(c.app_id.as_str());
        let mut launch_path = String::new();
        // Shell is always vortex-shell binary.
        if c.role == sunlight_ipc::SessionComponentRole::Shell {
            let _ = launch_path.push_str("/bin/sunlight-vortex-shell");
        }
        let _ = components.push(ResolvedSessionComponent {
            component_id: next_id,
            app_id,
            kind: ResolvedComponentKind::RequiredShell,
            required: true,
            launch_phase: StartupLaunchPhase::AfterShellReady,
            order: c.order,
            entry_id: None,
            restart_policy: OptionalComponentRestartPolicy::Never,
            launch_path,
            single_instance: true,
            completion_mode: StartupCompletionMode::ProcessSuccess,
        });
        next_id = next_id.saturating_add(1);
    }

    // 3–9. Optional entries from profile when not degraded.
    if !profile_degraded {
        let mut optional: Vec<(u16, &StartupEntry), MAX_STARTUP_ENTRIES> = Vec::new();
        for e in profile.entries.iter() {
            let state = profile
                .policy_state
                .iter()
                .find(|p| p.entry_id == e.entry_id);
            if !policy_should_launch(e, state, system_release_generation) {
                continue;
            }
            let _ = optional.push((e.order, e));
        }
        // Sort: launch phase (all AfterShellReady), order, bundle id, entry id.
        for i in 1..optional.len() {
            let mut j = i;
            while j > 0 {
                let (oa, ea) = optional[j - 1];
                let (ob, eb) = optional[j];
                let less = ob < oa
                    || (ob == oa
                        && (eb.app_id.as_str() < ea.app_id.as_str()
                            || (eb.app_id.as_str() == ea.app_id.as_str()
                                && eb.entry_id.get() < ea.entry_id.get())));
                if less {
                    optional.swap(j - 1, j);
                    j -= 1;
                } else {
                    break;
                }
            }
        }
        for (_, e) in optional.iter() {
            let Some(bundle) = catalog.iter().find(|b| b.app_id.as_str() == e.app_id.as_str())
            else {
                // Missing bundle: skip launch, keep entry in profile (caller records status).
                continue;
            };
            if bundle.availability != StartupAvailability::Available || !bundle.startup_eligible {
                continue;
            }
            if bundle.launch_path.is_empty() {
                continue;
            }
            let mut app_id = String::<MAX_APP_ID>::new();
            if app_id.push_str(e.app_id.as_str()).is_err() {
                continue;
            }
            let mut launch_path = String::<MAX_LAUNCH_PATH>::new();
            if launch_path.push_str(bundle.launch_path.as_str()).is_err() {
                continue;
            }
            let _ = components.push(ResolvedSessionComponent {
                component_id: next_id,
                app_id,
                kind: ResolvedComponentKind::OptionalStartup,
                required: false,
                launch_phase: e.launch_phase,
                order: e.order,
                entry_id: Some(e.entry_id),
                restart_policy: e.restart_policy,
                launch_path,
                single_instance: bundle.single_instance,
                completion_mode: bundle.completion_mode,
            });
            next_id = next_id.saturating_add(1);
        }
    }

    let plan_id = SessionPlanId::new(plan_id.max(1)).unwrap();
    let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
    for c in components.iter() {
        for b in c.app_id.as_bytes() {
            digest ^= *b as u64;
            digest = digest.wrapping_mul(0x100_0000_01b3);
        }
        digest ^= c.order as u64;
        digest = digest.wrapping_mul(0x100_0000_01b3);
        digest ^= c.component_id;
        digest = digest.wrapping_mul(0x100_0000_01b3);
    }
    digest ^= profile.revision;
    digest ^= system_manifest_revision << 17;
    digest ^= bundle_catalog_generation << 7;

    ResolvedSessionPlan {
        format_version: SESSION_PLAN_FORMAT_VERSION,
        plan_id,
        profile_revision: profile.revision,
        system_manifest_revision,
        bundle_catalog_generation,
        components,
        plan_digest: digest,
        profile_degraded,
    }
}

/// Parse optional `[session]` fields and application identity from a `.sunapp` Manifest.toml.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedBundleSession {
    pub app_id: String<MAX_APP_ID>,
    pub display_name: String<MAX_DISPLAY_NAME>,
    pub version: String<MAX_VERSION>,
    pub icon: Option<String<MAX_ICON_REF>>,
    pub startup_eligible: bool,
    pub default_enabled: bool,
    pub default_policy: StartupPolicy,
    pub single_instance: bool,
    pub completion_mode: StartupCompletionMode,
    pub runtime_native: bool,
    pub launch_path: String<MAX_LAUNCH_PATH>,
}

fn toml_string_value(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        Some(&trimmed[1..trimmed.len() - 1])
    } else {
        None
    }
}

fn toml_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Lightweight TOML section/key reader for bundle manifests.
pub fn parse_bundle_session_manifest(
    text: &str,
    bundle_root: &str,
) -> Result<ParsedBundleSession, ProfileError> {
    let mut section = "";
    let mut app_id = String::<MAX_APP_ID>::new();
    let mut display_name = String::<MAX_DISPLAY_NAME>::new();
    let mut version = String::<MAX_VERSION>::new();
    let mut icon: Option<String<MAX_ICON_REF>> = None;
    let mut startup_eligible = false;
    let mut default_enabled = false;
    let mut default_policy = StartupPolicy::EveryLogin;
    let mut single_instance = true;
    let mut completion_mode = StartupCompletionMode::ProcessSuccess;
    let mut runtime = String::<16>::new();
    let mut entry_exec = String::<MAX_LAUNCH_PATH>::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let value = v.trim();
        match (section, key) {
            ("application", "id") => {
                let s = toml_string_value(value).ok_or(ProfileError::InvalidString)?;
                app_id.clear();
                app_id
                    .push_str(s)
                    .map_err(|_| ProfileError::InvalidString)?;
            }
            ("application", "name") => {
                let s = toml_string_value(value).ok_or(ProfileError::InvalidString)?;
                display_name.clear();
                display_name
                    .push_str(s)
                    .map_err(|_| ProfileError::InvalidString)?;
            }
            ("application", "version") => {
                let s = toml_string_value(value).ok_or(ProfileError::InvalidString)?;
                version.clear();
                let _ = version.push_str(s);
            }
            ("application", "icon") => {
                if let Some(s) = toml_string_value(value) {
                    let mut ic = String::new();
                    if ic.push_str(s).is_ok() {
                        icon = Some(ic);
                    }
                }
            }
            ("runtime", "type") => {
                if let Some(s) = toml_string_value(value) {
                    runtime.clear();
                    let _ = runtime.push_str(s);
                }
            }
            ("entry", "executable") => {
                if let Some(s) = toml_string_value(value) {
                    entry_exec.clear();
                    let _ = entry_exec.push_str(s);
                }
            }
            ("session", "startup_eligible") => {
                startup_eligible = toml_bool(value).unwrap_or(false);
            }
            ("session", "default_enabled") => {
                default_enabled = toml_bool(value).unwrap_or(false);
            }
            ("session", "default_policy") => {
                if let Some(s) = toml_string_value(value) {
                    default_policy = StartupPolicy::parse(s).unwrap_or(StartupPolicy::EveryLogin);
                }
            }
            ("session", "single_instance") => {
                single_instance = toml_bool(value).unwrap_or(true);
            }
            ("session", "completion_mode") => {
                if let Some(s) = toml_string_value(value) {
                    completion_mode =
                        StartupCompletionMode::parse(s).unwrap_or(StartupCompletionMode::ProcessSuccess);
                }
            }
            ("session", "launch_path") => {
                // System-only short spawn path for native session utilities.
                if let Some(s) = toml_string_value(value) {
                    entry_exec.clear();
                    let _ = entry_exec.push_str(s);
                }
            }
            _ => {}
        }
    }

    if app_id.is_empty() || display_name.is_empty() {
        return Err(ProfileError::InvalidString);
    }
    validate_bundle_id(app_id.as_str()).or_else(|e| {
        if e == ProfileError::ShellAsOptional {
            // Shell may appear in catalog as required, not eligible.
            Ok(())
        } else {
            Err(e)
        }
    })?;

    let runtime_native = runtime.as_str() == "native" || !entry_exec.is_empty() && entry_exec.starts_with('/');
    let mut launch_path = String::new();
    if entry_exec.starts_with('/') {
        let _ = launch_path.push_str(entry_exec.as_str());
    } else if runtime.as_str() == "native" && !entry_exec.is_empty() {
        // Relative to bundle Program/
        let mut path = String::<MAX_LAUNCH_PATH>::new();
        let _ = write!(&mut path, "{}/Program/{}", bundle_root, entry_exec.as_str());
        launch_path = path;
    } else if !entry_exec.is_empty() {
        // Chronos or other: not SpawnRequest-friendly; leave empty unless launch_path set.
        // launch_path from session.launch_path already handled into entry_exec when absolute.
    }

    Ok(ParsedBundleSession {
        app_id,
        display_name,
        version,
        icon,
        startup_eligible,
        default_enabled,
        default_policy,
        single_instance,
        completion_mode,
        runtime_native,
        launch_path,
    })
}

/// Seed catalog apps marked `default_enabled` into a fresh user profile.
///
/// Only adds entries that are not already configured. Used when a profile is
/// first created (Missing) so first-login onboarding can auto-launch without
/// Control Panel configuration.
pub fn seed_default_enabled_apps(
    profile: &mut SessionProfile,
    catalog: &[CatalogBundle],
    now: u64,
) -> u32 {
    let mut added = 0u32;
    for bundle in catalog.iter() {
        if !bundle.default_enabled || !bundle.startup_eligible {
            continue;
        }
        if bundle.availability != StartupAvailability::Available {
            continue;
        }
        if profile
            .entries
            .iter()
            .any(|e| e.app_id.as_str() == bundle.app_id.as_str())
        {
            continue;
        }
        let rev = profile.revision;
        if profile_add_app(
            profile,
            bundle.app_id.as_str(),
            bundle.default_policy,
            rev,
            now,
        )
        .is_ok()
        {
            added = added.saturating_add(1);
        }
    }
    added
}

/// Serialize profile to a compact binary blob for atomic persistence.
pub const PROFILE_MAGIC: [u8; 4] = *b"SPF1";
pub const PROFILE_BLOB_MAX: usize = 4096;

pub fn serialize_profile(profile: &SessionProfile, out: &mut [u8]) -> Result<usize, ProfileError> {
    if out.len() < 64 {
        return Err(ProfileError::InvalidString);
    }
    let mut cursor = 0usize;
    let put_u8 = |out: &mut [u8], c: &mut usize, v: u8| -> Result<(), ProfileError> {
        if *c >= out.len() {
            return Err(ProfileError::InvalidString);
        }
        out[*c] = v;
        *c += 1;
        Ok(())
    };
    let put_u16 = |out: &mut [u8], c: &mut usize, v: u16| -> Result<(), ProfileError> {
        put_u8(out, c, (v & 0xff) as u8)?;
        put_u8(out, c, (v >> 8) as u8)
    };
    let put_u32 = |out: &mut [u8], c: &mut usize, v: u32| -> Result<(), ProfileError> {
        for i in 0..4 {
            put_u8(out, c, ((v >> (i * 8)) & 0xff) as u8)?;
        }
        Ok(())
    };
    let put_u64 = |out: &mut [u8], c: &mut usize, v: u64| -> Result<(), ProfileError> {
        for i in 0..8 {
            put_u8(out, c, ((v >> (i * 8)) & 0xff) as u8)?;
        }
        Ok(())
    };
    let put_str = |out: &mut [u8], c: &mut usize, s: &str| -> Result<(), ProfileError> {
        if s.len() > 255 {
            return Err(ProfileError::InvalidString);
        }
        put_u8(out, c, s.len() as u8)?;
        for b in s.as_bytes() {
            put_u8(out, c, *b)?;
        }
        Ok(())
    };

    for b in PROFILE_MAGIC {
        put_u8(out, &mut cursor, b)?;
    }
    put_u16(out, &mut cursor, profile.format_version)?;
    put_u64(out, &mut cursor, profile.profile_id.get())?;
    put_u32(out, &mut cursor, profile.user_id)?;
    put_str(out, &mut cursor, profile.base_session_id.as_str())?;
    put_u64(out, &mut cursor, profile.revision)?;
    put_u64(out, &mut cursor, profile.created_at)?;
    put_u64(out, &mut cursor, profile.updated_at)?;
    put_u8(out, &mut cursor, profile.entries.len() as u8)?;
    for e in profile.entries.iter() {
        put_u64(out, &mut cursor, e.entry_id.get())?;
        put_str(out, &mut cursor, e.app_id.as_str())?;
        put_u8(out, &mut cursor, e.enabled as u8)?;
        put_u8(out, &mut cursor, e.policy as u8)?;
        put_u8(out, &mut cursor, e.launch_phase as u8)?;
        put_u16(out, &mut cursor, e.order)?;
        put_u8(out, &mut cursor, e.restart_policy as u8)?;
        put_u64(out, &mut cursor, e.added_at)?;
    }
    put_u8(out, &mut cursor, profile.policy_state.len() as u8)?;
    for p in profile.policy_state.iter() {
        put_u64(out, &mut cursor, p.entry_id.get())?;
        put_str(out, &mut cursor, p.app_id.as_str())?;
        put_u8(out, &mut cursor, p.completed_first_login as u8)?;
        put_u8(
            out,
            &mut cursor,
            p.completed_system_generation.is_some() as u8,
        )?;
        put_u32(
            out,
            &mut cursor,
            p.completed_system_generation.unwrap_or(0),
        )?;
        put_u8(
            out,
            &mut cursor,
            p.last_successful_start_at.is_some() as u8,
        )?;
        put_u64(out, &mut cursor, p.last_successful_start_at.unwrap_or(0))?;
    }
    // profile_checksum does not include the checksum field itself.
    let checksum = profile_checksum(profile);
    put_u32(out, &mut cursor, checksum)?;
    Ok(cursor)
}

pub fn deserialize_profile(bytes: &[u8], expected_user: u32) -> Result<SessionProfile, ProfileError> {
    if bytes.len() < 20 || bytes[0..4] != PROFILE_MAGIC {
        return Err(ProfileError::ChecksumFailure);
    }
    let mut c = 4usize;
    let take_u8 = |bytes: &[u8], c: &mut usize| -> Result<u8, ProfileError> {
        if *c >= bytes.len() {
            return Err(ProfileError::ChecksumFailure);
        }
        let v = bytes[*c];
        *c += 1;
        Ok(v)
    };
    let take_u16 = |bytes: &[u8], c: &mut usize| -> Result<u16, ProfileError> {
        let lo = take_u8(bytes, c)? as u16;
        let hi = take_u8(bytes, c)? as u16;
        Ok(lo | (hi << 8))
    };
    let take_u32 = |bytes: &[u8], c: &mut usize| -> Result<u32, ProfileError> {
        let mut v = 0u32;
        for i in 0..4 {
            v |= (take_u8(bytes, c)? as u32) << (i * 8);
        }
        Ok(v)
    };
    let take_u64 = |bytes: &[u8], c: &mut usize| -> Result<u64, ProfileError> {
        let mut v = 0u64;
        for i in 0..8 {
            v |= (take_u8(bytes, c)? as u64) << (i * 8);
        }
        Ok(v)
    };
    let take_str = |bytes: &[u8], c: &mut usize| -> Result<String<64>, ProfileError> {
        let len = take_u8(bytes, c)? as usize;
        if *c + len > bytes.len() {
            return Err(ProfileError::ChecksumFailure);
        }
        let s = core::str::from_utf8(&bytes[*c..*c + len]).map_err(|_| ProfileError::InvalidString)?;
        *c += len;
        let mut out = String::new();
        out.push_str(s).map_err(|_| ProfileError::InvalidString)?;
        Ok(out)
    };

    let format_version = take_u16(bytes, &mut c)?;
    let profile_id = SessionProfileId::new(take_u64(bytes, &mut c)?).ok_or(ProfileError::ZeroIdentifier)?;
    let user_id = take_u32(bytes, &mut c)?;
    let base = take_str(bytes, &mut c)?;
    let mut base_session_id = String::<48>::new();
    base_session_id
        .push_str(base.as_str())
        .map_err(|_| ProfileError::InvalidString)?;
    let revision = take_u64(bytes, &mut c)?;
    let created_at = take_u64(bytes, &mut c)?;
    let updated_at = take_u64(bytes, &mut c)?;
    let n_entries = take_u8(bytes, &mut c)? as usize;
    if n_entries > MAX_STARTUP_ENTRIES {
        return Err(ProfileError::TooManyEntries);
    }
    let mut entries = Vec::new();
    for _ in 0..n_entries {
        let entry_id = StartupEntryId::new(take_u64(bytes, &mut c)?).ok_or(ProfileError::ZeroIdentifier)?;
        let app = take_str(bytes, &mut c)?;
        let mut app_id = String::<MAX_APP_ID>::new();
        app_id
            .push_str(app.as_str())
            .map_err(|_| ProfileError::InvalidString)?;
        let enabled = take_u8(bytes, &mut c)? != 0;
        let policy = StartupPolicy::from_u8(take_u8(bytes, &mut c)?).ok_or(ProfileError::InvalidPolicy)?;
        let launch_phase = StartupLaunchPhase::from_u8(take_u8(bytes, &mut c)?)
            .ok_or(ProfileError::InvalidLaunchPhase)?;
        let order = take_u16(bytes, &mut c)?;
        let restart_policy = OptionalComponentRestartPolicy::from_u8(take_u8(bytes, &mut c)?)
            .ok_or(ProfileError::InvalidRestartPolicy)?;
        let added_at = take_u64(bytes, &mut c)?;
        entries
            .push(StartupEntry {
                entry_id,
                app_id,
                enabled,
                policy,
                launch_phase,
                order,
                restart_policy,
                added_at,
            })
            .map_err(|_| ProfileError::TooManyEntries)?;
    }
    let n_state = take_u8(bytes, &mut c)? as usize;
    let mut policy_state = Vec::new();
    for _ in 0..n_state {
        let entry_id = StartupEntryId::new(take_u64(bytes, &mut c)?).ok_or(ProfileError::ZeroIdentifier)?;
        let app = take_str(bytes, &mut c)?;
        let mut app_id = String::<MAX_APP_ID>::new();
        app_id
            .push_str(app.as_str())
            .map_err(|_| ProfileError::InvalidString)?;
        let completed_first_login = take_u8(bytes, &mut c)? != 0;
        let has_gen = take_u8(bytes, &mut c)? != 0;
        let gen = take_u32(bytes, &mut c)?;
        let has_last = take_u8(bytes, &mut c)? != 0;
        let last = take_u64(bytes, &mut c)?;
        policy_state
            .push(StartupPolicyState {
                entry_id,
                app_id,
                completed_first_login,
                completed_system_generation: if has_gen { Some(gen) } else { None },
                last_successful_start_at: if has_last { Some(last) } else { None },
            })
            .map_err(|_| ProfileError::TooManyEntries)?;
    }
    let checksum = take_u32(bytes, &mut c)?;
    let profile = SessionProfile {
        format_version,
        profile_id,
        user_id,
        base_session_id,
        revision,
        entries,
        policy_state,
        created_at,
        updated_at,
        checksum,
    };
    validate_profile(&profile, expected_user)?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_manifest;
    use std::format;
    use std::string::String as StdString;

    fn base_manifest() -> SessionManifest {
        parse_manifest(
            br#"format_version = 1
id = "org.sunlight.session.desktop"
name = "Sunlight Desktop"
kind = "desktop"

[[components]]
id = "org.sunlight.vortex-shell"
role = "shell"
required = true
enabled = true
launch_policy = "session-start"
restart_policy = "on-failure"
restart_limit = 8
restart_window_seconds = 120
readiness_timeout_seconds = 10
order = 0
"#,
        )
        .unwrap()
    }

    fn catalog_two() -> [CatalogBundle; 2] {
        let mut a = CatalogBundle {
            app_id: String::new(),
            display_name: String::new(),
            version: String::new(),
            icon_reference: None,
            publisher: None,
            default_policy: StartupPolicy::EveryLogin,
            default_enabled: false,
            single_instance: true,
            completion_mode: StartupCompletionMode::ProcessSuccess,
            availability: StartupAvailability::Available,
            launch_path: String::new(),
            bundle_dir: String::new(),
            startup_eligible: true,
        };
        let _ = a.app_id.push_str("org.sun.test.su1");
        let _ = a.display_name.push_str("Startup One");
        let _ = a.version.push_str("1.0.0");
        let _ = a.launch_path.push_str("/bin/su1");
        let _ = a.bundle_dir.push_str("/Applications/StartupOne.sunapp");
        let mut b = a.clone();
        b.app_id.clear();
        b.display_name.clear();
        let _ = b.app_id.push_str("org.sun.test.su2");
        let _ = b.display_name.push_str("Startup Two");
        let _ = b.launch_path.push_str("/bin/su2");
        let _ = b.bundle_dir.push_str("/Applications/StartupTwo.sunapp");
        [a, b]
    }

    #[test]
    fn ids_reject_zero() {
        assert!(SessionProfileId::new(0).is_none());
        assert!(StartupEntryId::new(0).is_none());
        assert!(SessionPlanId::new(0).is_none());
    }

    #[test]
    fn default_profile_valid() {
        let p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        validate_profile(&p, 1000).unwrap();
        assert_eq!(p.revision, 1);
        assert!(p.entries.is_empty());
    }

    #[test]
    fn wrong_user_rejected() {
        let p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        assert_eq!(validate_profile(&p, 1001), Err(ProfileError::WrongUser));
    }

    #[test]
    fn shell_as_optional_rejected() {
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        assert_eq!(
            profile_add_app(&mut p, PROTECTED_SHELL_APP_ID, StartupPolicy::EveryLogin, 1, 2),
            Err(ProfileError::ShellAsOptional)
        );
    }

    #[test]
    fn executable_path_rejected() {
        assert_eq!(
            validate_bundle_id("/bin/evil"),
            Err(ProfileError::ExecutablePathRejected)
        );
        assert_eq!(
            validate_bundle_id("foo bar"),
            Err(ProfileError::ExecutablePathRejected)
        );
        assert_eq!(
            validate_bundle_id("foo@bar"),
            Err(ProfileError::MalformedBundleId)
        );
    }

    #[test]
    fn add_remove_enable_policy_reorder() {
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        profile_add_app(
            &mut p,
            "org.sun.test.su1",
            StartupPolicy::EveryLogin,
            1,
            2,
        )
        .unwrap();
        assert_eq!(p.revision, 2);
        assert_eq!(p.entries.len(), 1);
        profile_add_app(
            &mut p,
            "org.sun.test.su2",
            StartupPolicy::EveryLogin,
            2,
            3,
        )
        .unwrap();
        assert_eq!(p.entries.len(), 2);
        // Duplicate add
        assert_eq!(
            profile_add_app(
                &mut p,
                "org.sun.test.su1",
                StartupPolicy::EveryLogin,
                3,
                4
            ),
            Err(ProfileError::DuplicateBundleId)
        );
        // Revision conflict
        assert_eq!(
            profile_add_app(
                &mut p,
                "org.example.other",
                StartupPolicy::EveryLogin,
                1,
                5
            ),
            Err(ProfileError::RevisionConflict)
        );
        profile_move(&mut p, "org.sun.test.su2", -1, 3, 6).unwrap();
        assert_eq!(p.entries.iter().find(|e| e.app_id.as_str() == "org.sun.test.su2").unwrap().order, 0);
        profile_set_enabled(&mut p, "org.sun.test.su1", false, 4, 7).unwrap();
        assert!(!p.entries.iter().find(|e| e.app_id.as_str() == "org.sun.test.su1").unwrap().enabled);
        profile_set_policy(
            &mut p,
            "org.sun.test.su2",
            StartupPolicy::FirstLoginOnly,
            5,
            8,
        )
        .unwrap();
        profile_remove_app(&mut p, "org.sun.test.su1", 6, 9).unwrap();
        assert_eq!(p.entries.len(), 1);
        profile_reset(&mut p, 7, 10).unwrap();
        assert!(p.entries.is_empty());
    }

    #[test]
    fn serialize_roundtrip() {
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 10).unwrap();
        profile_add_app(
            &mut p,
            "org.sun.test.su1",
            StartupPolicy::FirstLoginOnly,
            1,
            11,
        )
        .unwrap();
        let mut buf = [0u8; PROFILE_BLOB_MAX];
        let n = serialize_profile(&p, &mut buf).unwrap();
        let loaded = deserialize_profile(&buf[..n], 1000).unwrap();
        assert_eq!(loaded.revision, p.revision);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].app_id.as_str(), "org.sun.test.su1");
    }

    #[test]
    fn checksum_failure_detected() {
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        p.checksum ^= 0xffff;
        assert_eq!(validate_profile(&p, 1000), Err(ProfileError::ChecksumFailure));
    }

    #[test]
    fn plan_shell_first_and_ordering() {
        let manifest = base_manifest();
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        profile_add_app(&mut p, "org.sun.test.su1", StartupPolicy::EveryLogin, 1, 2).unwrap();
        profile_add_app(&mut p, "org.sun.test.su2", StartupPolicy::EveryLogin, 2, 3).unwrap();
        // Move two before one
        profile_move(&mut p, "org.sun.test.su2", -1, 3, 4).unwrap();
        let cat = catalog_two();
        let plan = resolve_session_plan(&manifest, &p, &cat, 1, 1, 1, 1, false);
        assert_eq!(plan.components.len(), 3);
        assert_eq!(plan.components[0].app_id.as_str(), PROTECTED_SHELL_APP_ID);
        assert!(plan.components[0].required);
        assert_eq!(plan.components[1].app_id.as_str(), "org.sun.test.su2");
        assert_eq!(plan.components[2].app_id.as_str(), "org.sun.test.su1");
    }

    #[test]
    fn plan_disabled_and_policy() {
        let manifest = base_manifest();
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        profile_add_app(&mut p, "org.sun.test.su1", StartupPolicy::EveryLogin, 1, 2).unwrap();
        profile_set_enabled(&mut p, "org.sun.test.su1", false, 2, 3).unwrap();
        let cat = catalog_two();
        let plan = resolve_session_plan(&manifest, &p, &cat, 1, 1, 1, 1, false);
        assert_eq!(plan.components.len(), 1); // shell only
    }

    #[test]
    fn first_login_only_completion() {
        let manifest = base_manifest();
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        profile_add_app(
            &mut p,
            "org.sun.test.su1",
            StartupPolicy::FirstLoginOnly,
            1,
            2,
        )
        .unwrap();
        let cat = catalog_two();
        let plan1 = resolve_session_plan(&manifest, &p, &cat, 1, 1, 1, 1, false);
        assert_eq!(plan1.components.len(), 2);
        let entry_id = p.entries[0].entry_id;
        mark_policy_success(&mut p, entry_id, 1, 100);
        // Checksum updated; revision unchanged by mark_policy_success
        p.checksum = profile_checksum(&p);
        let plan2 = resolve_session_plan(&manifest, &p, &cat, 1, 1, 1, 2, false);
        assert_eq!(plan2.components.len(), 1);
    }

    #[test]
    fn first_login_after_upgrade() {
        let manifest = base_manifest();
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        profile_add_app(
            &mut p,
            "org.sun.test.su1",
            StartupPolicy::FirstLoginAfterSystemUpgrade,
            1,
            2,
        )
        .unwrap();
        let cat = catalog_two();
        let plan1 = resolve_session_plan(&manifest, &p, &cat, 1, 1, 1, 1, false);
        assert_eq!(plan1.components.len(), 2);
        let entry_id = p.entries[0].entry_id;
        mark_policy_success(&mut p, entry_id, 1, 50);
        p.checksum = profile_checksum(&p);
        let plan2 = resolve_session_plan(&manifest, &p, &cat, 1, 1, 1, 2, false);
        assert_eq!(plan2.components.len(), 1);
        // New generation launches again
        let plan3 = resolve_session_plan(&manifest, &p, &cat, 1, 1, 2, 3, false);
        assert_eq!(plan3.components.len(), 2);
    }

    #[test]
    fn unavailable_bundle_skipped_entry_preserved() {
        let manifest = base_manifest();
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        profile_add_app(
            &mut p,
            "org.sun.test.su1",
            StartupPolicy::EveryLogin,
            1,
            2,
        )
        .unwrap();
        let empty: [CatalogBundle; 0] = [];
        let plan = resolve_session_plan(&manifest, &p, &empty, 1, 1, 1, 1, false);
        assert_eq!(plan.components.len(), 1);
        assert_eq!(p.entries.len(), 1);
    }

    #[test]
    fn corrupt_profile_plan_shell_only() {
        let manifest = base_manifest();
        let p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        let mut p2 = p.clone();
        profile_add_app(
            &mut p2,
            "org.sun.test.su1",
            StartupPolicy::EveryLogin,
            1,
            2,
        )
        .unwrap();
        let cat = catalog_two();
        let plan = resolve_session_plan(&manifest, &p2, &cat, 1, 1, 1, 1, true);
        assert_eq!(plan.components.len(), 1);
        assert!(plan.profile_degraded);
    }

    #[test]
    fn parse_bundle_session() {
        let text = r#"
[bundle]
format = 1

[application]
id = "org.sun.test.su1"
name = "Startup One"
version = "1.0.0"
icon = "Resources/icon.tga"

[runtime]
type = "native"

[session]
startup_eligible = true
default_enabled = false
default_policy = "every-login"
single_instance = true
launch_path = "/bin/su1"
"#;
        let parsed = parse_bundle_session_manifest(text, "/Applications/StartupOne.sunapp").unwrap();
        assert!(parsed.startup_eligible);
        assert_eq!(parsed.launch_path.as_str(), "/bin/su1");
        assert_eq!(parsed.default_policy, StartupPolicy::EveryLogin);
        assert_eq!(parsed.completion_mode, StartupCompletionMode::ProcessSuccess);
    }

    #[test]
    fn parse_app_reported_completion_mode() {
        let text = r#"
[application]
id = "org.sunlight.welcome"
name = "Welcome to SunlightOS"
version = "0.1.0"

[session]
startup_eligible = true
default_enabled = true
default_policy = "first-login-after-system-upgrade"
single_instance = true
completion_mode = "wizard-finished"
launch_path = "/bin/welcome"
"#;
        let parsed =
            parse_bundle_session_manifest(text, "/Applications/WiseOwlWelcome.sunapp").unwrap();
        assert!(parsed.default_enabled);
        assert_eq!(
            parsed.default_policy,
            StartupPolicy::FirstLoginAfterSystemUpgrade
        );
        assert_eq!(parsed.completion_mode, StartupCompletionMode::AppReported);
        assert_eq!(parsed.launch_path.as_str(), "/bin/welcome");
        assert_eq!(parsed.app_id.as_str(), "org.sunlight.welcome");
    }

    #[test]
    fn seed_default_enabled_adds_once() {
        let mut cat = catalog_two();
        cat[0].default_enabled = true;
        cat[0].default_policy = StartupPolicy::FirstLoginAfterSystemUpgrade;
        cat[0].completion_mode = StartupCompletionMode::AppReported;
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        let n = seed_default_enabled_apps(&mut p, &cat, 10);
        assert_eq!(n, 1);
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.entries[0].app_id.as_str(), "org.sun.test.su1");
        assert_eq!(
            p.entries[0].policy,
            StartupPolicy::FirstLoginAfterSystemUpgrade
        );
        // Second seed is a no-op.
        assert_eq!(seed_default_enabled_apps(&mut p, &cat, 11), 0);
        assert_eq!(p.entries.len(), 1);
    }

    #[test]
    fn welcome_manifest_seeds_and_resolves_optional() {
        let text = r#"
[bundle]
format = 1

[application]
id = "org.sunlight.welcome"
name = "Welcome to SunlightOS"
version = "0.1.0"
icon = "Resources/icon.tga"

[runtime]
type = "native"

[entry]
executable = "/bin/welcome"

[session]
startup_eligible = true
default_enabled = true
default_policy = "first-login-after-system-upgrade"
single_instance = true
completion_mode = "wizard-finished"
launch_path = "/bin/welcome"
"#;
        let parsed =
            parse_bundle_session_manifest(text, "/Applications/WiseOwlWelcome.sunapp").unwrap();
        assert!(parsed.default_enabled);
        assert_eq!(parsed.app_id.as_str(), "org.sunlight.welcome");
        assert_eq!(parsed.launch_path.as_str(), "/bin/welcome");
        assert_eq!(
            parsed.completion_mode,
            StartupCompletionMode::AppReported
        );
        assert_eq!(
            parsed.default_policy,
            StartupPolicy::FirstLoginAfterSystemUpgrade
        );

        let cat = CatalogBundle {
            app_id: parsed.app_id.clone(),
            display_name: parsed.display_name.clone(),
            version: parsed.version.clone(),
            icon_reference: parsed.icon.clone(),
            publisher: None,
            default_policy: parsed.default_policy,
            default_enabled: parsed.default_enabled,
            single_instance: parsed.single_instance,
            completion_mode: parsed.completion_mode,
            availability: StartupAvailability::Available,
            launch_path: parsed.launch_path.clone(),
            bundle_dir: {
                let mut s = String::new();
                let _ = s.push_str("/Applications/WiseOwlWelcome.sunapp");
                s
            },
            startup_eligible: true,
        };
        let catalog = [cat];
        let mut p = default_profile(0, "org.sunlight.session.desktop", 1).unwrap();
        let n = seed_default_enabled_apps(&mut p, &catalog, 10);
        assert_eq!(n, 1, "welcome must seed into empty profile");
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.entries[0].app_id.as_str(), "org.sunlight.welcome");
        assert!(p.entries[0].enabled);
        assert_eq!(
            p.entries[0].policy,
            StartupPolicy::FirstLoginAfterSystemUpgrade
        );

        let manifest = base_manifest();
        let plan = resolve_session_plan(&manifest, &p, &catalog, 1, 1, 1, 1, false);
        let optional = plan
            .components
            .iter()
            .filter(|c| c.kind == ResolvedComponentKind::OptionalStartup)
            .count();
        assert_eq!(
            optional, 1,
            "seeded welcome must appear as optional in plan, got components={:?}",
            plan.components
                .iter()
                .map(|c| c.app_id.as_str())
                .collect::<std::vec::Vec<_>>()
        );
        assert_eq!(
            plan.components[1].completion_mode,
            StartupCompletionMode::AppReported
        );
    }

    #[test]
    fn app_reported_policy_not_complete_until_mark() {
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        profile_add_app(
            &mut p,
            "org.sunlight.welcome",
            StartupPolicy::FirstLoginAfterSystemUpgrade,
            1,
            2,
        )
        .unwrap();
        let eid = p.entries[0].entry_id;
        // Launch eligibility remains until explicit mark.
        assert!(policy_should_launch(
            &p.entries[0],
            p.policy_state.iter().find(|s| s.entry_id == eid),
            1
        ));
        mark_policy_success(&mut p, eid, 1, 3);
        assert!(!policy_should_launch(
            &p.entries[0],
            p.policy_state.iter().find(|s| s.entry_id == eid),
            1
        ));
        // New system generation re-arms.
        assert!(policy_should_launch(
            &p.entries[0],
            p.policy_state.iter().find(|s| s.entry_id == eid),
            2
        ));
    }

    #[test]
    fn eligible_list_sorted() {
        let cat = catalog_two();
        let p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        let list = list_eligible(&cat, &p);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].display_name.as_str(), "Startup One");
        assert_eq!(list[1].display_name.as_str(), "Startup Two");
    }

    #[test]
    fn excessive_entries_rejected() {
        let mut p = default_profile(1000, "org.sunlight.session.desktop", 1).unwrap();
        for i in 0..MAX_STARTUP_ENTRIES {
            let id = StdString::from(format!("org.example.app{i}"));
            let rev = p.revision;
            profile_add_app(&mut p, &id, StartupPolicy::EveryLogin, rev, i as u64 + 2).unwrap();
        }
        let rev = p.revision;
        assert_eq!(
            profile_add_app(
                &mut p,
                "org.example.overflow",
                StartupPolicy::EveryLogin,
                rev,
                100
            ),
            Err(ProfileError::TooManyEntries)
        );
    }
}
