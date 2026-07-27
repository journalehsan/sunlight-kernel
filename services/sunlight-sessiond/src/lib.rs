#![no_std]

#[cfg(test)]
extern crate std;

pub mod profile;

pub use profile::*;

use heapless::{String, Vec};
use sunlight_ipc::{
    SessionComponentId, SessionComponentRole, SessionComponentState, SessionGeneration, SessionId,
    SessionKind, SessionLaunchPolicy, SessionRestartPolicy, SessionState,
};

/// Required system components + optional startup apps.
pub const MAX_COMPONENTS: usize = profile::MAX_SESSION_COMPONENTS;
pub const MAX_MANIFEST_ID: usize = 48;
pub const MAX_MANIFEST_NAME: usize = 48;
pub const MAX_COMPONENT_NAME: usize = 32;
pub const MAX_APP_ID: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestParseError {
    InvalidUtf8,
    UnknownField,
    InvalidValue,
    MissingField,
    DuplicateComponentId,
    DuplicateShellRole,
    MissingShell,
    TooManyComponents,
    UnsupportedVersion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestComponent {
    pub id: String<MAX_COMPONENT_NAME>,
    pub app_id: String<MAX_APP_ID>,
    pub role: SessionComponentRole,
    pub required: bool,
    pub enabled: bool,
    pub launch_policy: SessionLaunchPolicy,
    pub restart_policy: SessionRestartPolicy,
    pub restart_limit: u16,
    pub restart_window_seconds: u16,
    pub readiness_timeout_seconds: u16,
    pub order: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionManifest {
    pub format_version: u16,
    pub id: String<MAX_MANIFEST_ID>,
    pub name: String<MAX_MANIFEST_NAME>,
    pub kind: SessionKind,
    pub components: Vec<ManifestComponent, MAX_COMPONENTS>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentExitReason {
    None = 0,
    Stopped = 1,
    Exited = 2,
    Crashed = 3,
    ReadinessTimeout = 4,
    RestartExhausted = 5,
}

impl ComponentExitReason {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionComponentRuntime {
    pub component_id: SessionComponentId,
    pub app_id: String<MAX_APP_ID>,
    pub role: SessionComponentRole,
    pub required: bool,
    pub process_id: Option<u64>,
    pub process_generation: Option<u64>,
    pub state: SessionComponentState,
    pub launch_count: u16,
    pub restart_count: u16,
    pub last_exit_reason: ComponentExitReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionRuntimeSnapshot {
    pub session_id: SessionId,
    pub generation: SessionGeneration,
    pub user_id: u32,
    pub kind: SessionKind,
    pub state: SessionState,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub component_count: u16,
    pub required_components_ready: u16,
    pub restart_count: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionError {
    InvalidTransition,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRecord {
    pub session_id: SessionId,
    pub generation: SessionGeneration,
    pub uid: u32,
    pub gid: u32,
    pub kind: SessionKind,
    pub state: SessionState,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub restart_count: u16,
    pub components: Vec<SessionComponentRuntime, MAX_COMPONENTS>,
}

impl SessionRecord {
    pub fn new(
        session_id: SessionId,
        generation: SessionGeneration,
        uid: u32,
        gid: u32,
        kind: SessionKind,
        created_at: u64,
        manifest: &SessionManifest,
    ) -> Self {
        let mut components = Vec::new();
        let mut index = 0u64;
        for component in manifest.components.iter() {
            index += 1;
            let _ = components.push(SessionComponentRuntime {
                component_id: SessionComponentId::new(index).unwrap(),
                app_id: component.app_id.clone(),
                role: component.role,
                required: component.required,
                process_id: None,
                process_generation: None,
                state: if component.enabled {
                    SessionComponentState::Pending
                } else {
                    SessionComponentState::Disabled
                },
                launch_count: 0,
                restart_count: 0,
                last_exit_reason: ComponentExitReason::None,
            });
        }
        Self {
            session_id,
            generation,
            uid,
            gid,
            kind,
            state: SessionState::Created,
            created_at,
            started_at: None,
            restart_count: 0,
            components,
        }
    }

    pub fn transition(&mut self, next: SessionState) -> Result<(), TransitionError> {
        let valid = matches!(
            (self.state, next),
            (SessionState::Created, SessionState::Preparing)
                | (SessionState::Preparing, SessionState::StartingRequiredComponents)
                | (SessionState::StartingRequiredComponents, SessionState::Running)
                | (SessionState::StartingRequiredComponents, SessionState::Degraded)
                | (SessionState::Running, SessionState::Degraded)
                | (SessionState::Running, SessionState::Stopping)
                | (SessionState::Degraded, SessionState::StartingRequiredComponents)
                | (SessionState::Degraded, SessionState::Failed)
                | (SessionState::Failed, SessionState::Stopping)
                | (SessionState::Locking, SessionState::Locked)
                | (SessionState::Locked, SessionState::Running)
                | (SessionState::Stopping, SessionState::Stopped)
        );
        if !valid {
            return Err(TransitionError::InvalidTransition);
        }
        self.state = next;
        Ok(())
    }

    pub fn shell_component_mut(&mut self) -> Option<&mut SessionComponentRuntime> {
        self.components
            .iter_mut()
            .find(|component| component.role == SessionComponentRole::Shell)
    }

    pub fn shell_component(&self) -> Option<&SessionComponentRuntime> {
        self.components
            .iter()
            .find(|component| component.role == SessionComponentRole::Shell)
    }

    pub fn required_ready_count(&self) -> u16 {
        self.components
            .iter()
            .filter(|component| {
                component.required
                    && matches!(
                        component.state,
                        SessionComponentState::Ready | SessionComponentState::Running
                    )
            })
            .count() as u16
    }

    pub fn snapshot(&self) -> SessionRuntimeSnapshot {
        SessionRuntimeSnapshot {
            session_id: self.session_id,
            generation: self.generation,
            user_id: self.uid,
            kind: self.kind,
            state: self.state,
            created_at: self.created_at,
            started_at: self.started_at,
            component_count: self.components.len() as u16,
            required_components_ready: self.required_ready_count(),
            restart_count: self.restart_count,
        }
    }
}

fn parse_bool(value: &str) -> Result<bool, ManifestParseError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ManifestParseError::InvalidValue),
    }
}

fn parse_u16(value: &str) -> Result<u16, ManifestParseError> {
    value
        .parse::<u16>()
        .map_err(|_| ManifestParseError::InvalidValue)
}

fn parse_string<const N: usize>(value: &str) -> Result<String<N>, ManifestParseError> {
    let trimmed = value.trim();
    if !(trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2) {
        return Err(ManifestParseError::InvalidValue);
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let mut out = String::new();
    out.push_str(inner)
        .map_err(|_| ManifestParseError::InvalidValue)?;
    Ok(out)
}

fn parse_kind(value: &str) -> Result<SessionKind, ManifestParseError> {
    match parse_string::<16>(value)?.as_str() {
        "desktop" => Ok(SessionKind::Desktop),
        "safe-desktop" => Ok(SessionKind::SafeDesktop),
        _ => Err(ManifestParseError::InvalidValue),
    }
}

fn parse_role(value: &str) -> Result<SessionComponentRole, ManifestParseError> {
    match parse_string::<24>(value)?.as_str() {
        "shell" => Ok(SessionComponentRole::Shell),
        "startup-application" => Ok(SessionComponentRole::StartupApplication),
        "session-service" => Ok(SessionComponentRole::SessionService),
        "welcome-application" => Ok(SessionComponentRole::WelcomeApplication),
        _ => Err(ManifestParseError::InvalidValue),
    }
}

fn parse_launch_policy(value: &str) -> Result<SessionLaunchPolicy, ManifestParseError> {
    match parse_string::<24>(value)?.as_str() {
        "session-start" => Ok(SessionLaunchPolicy::SessionStart),
        "on-demand" => Ok(SessionLaunchPolicy::OnDemand),
        "disabled" => Ok(SessionLaunchPolicy::Disabled),
        _ => Err(ManifestParseError::InvalidValue),
    }
}

fn parse_restart_policy(value: &str) -> Result<SessionRestartPolicy, ManifestParseError> {
    match parse_string::<24>(value)?.as_str() {
        "never" => Ok(SessionRestartPolicy::Never),
        "on-failure" => Ok(SessionRestartPolicy::OnFailure),
        "always" => Ok(SessionRestartPolicy::Always),
        _ => Err(ManifestParseError::InvalidValue),
    }
}

pub fn parse_manifest(bytes: &[u8]) -> Result<SessionManifest, ManifestParseError> {
    let text = core::str::from_utf8(bytes).map_err(|_| ManifestParseError::InvalidUtf8)?;
    let mut manifest = SessionManifest {
        format_version: 0,
        id: String::new(),
        name: String::new(),
        kind: SessionKind::Desktop,
        components: Vec::new(),
    };
    let mut in_component = false;
    let mut current: Option<ManifestComponent> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[components]]" {
            if let Some(component) = current.take() {
                manifest
                    .components
                    .push(component)
                    .map_err(|_| ManifestParseError::TooManyComponents)?;
            }
            current = Some(ManifestComponent {
                id: String::new(),
                app_id: String::new(),
                role: SessionComponentRole::Shell,
                required: false,
                enabled: false,
                launch_policy: SessionLaunchPolicy::SessionStart,
                restart_policy: SessionRestartPolicy::OnFailure,
                restart_limit: 0,
                restart_window_seconds: 0,
                readiness_timeout_seconds: 0,
                order: 0,
            });
            in_component = true;
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            return Err(ManifestParseError::InvalidValue);
        };
        let key = raw_key.trim();
        let value = raw_value.trim();
        if in_component {
            let component = current.as_mut().ok_or(ManifestParseError::InvalidValue)?;
            match key {
                "id" => {
                    component.id = parse_string(value)?;
                    let mut app_id = String::new();
                    app_id
                        .push_str(component.id.as_str())
                        .map_err(|_| ManifestParseError::InvalidValue)?;
                    component.app_id = app_id;
                }
                "role" => component.role = parse_role(value)?,
                "required" => component.required = parse_bool(value)?,
                "enabled" => component.enabled = parse_bool(value)?,
                "launch_policy" => component.launch_policy = parse_launch_policy(value)?,
                "restart_policy" => component.restart_policy = parse_restart_policy(value)?,
                "restart_limit" => component.restart_limit = parse_u16(value)?,
                "restart_window_seconds" => component.restart_window_seconds = parse_u16(value)?,
                "readiness_timeout_seconds" => {
                    component.readiness_timeout_seconds = parse_u16(value)?
                }
                "order" => component.order = parse_u16(value)?,
                _ => return Err(ManifestParseError::UnknownField),
            }
            continue;
        }
        match key {
            "format_version" => manifest.format_version = parse_u16(value)?,
            "id" => manifest.id = parse_string(value)?,
            "name" => manifest.name = parse_string(value)?,
            "kind" => manifest.kind = parse_kind(value)?,
            _ => return Err(ManifestParseError::UnknownField),
        }
    }

    if let Some(component) = current.take() {
        manifest
            .components
            .push(component)
            .map_err(|_| ManifestParseError::TooManyComponents)?;
    }

    if manifest.format_version != 1 {
        return Err(ManifestParseError::UnsupportedVersion);
    }
    if manifest.id.is_empty() || manifest.name.is_empty() {
        return Err(ManifestParseError::MissingField);
    }

    let mut shell_count = 0usize;
    for (index, component) in manifest.components.iter().enumerate() {
        if component.id.is_empty() || component.app_id.is_empty() {
            return Err(ManifestParseError::MissingField);
        }
        if component.restart_limit > 16 || component.readiness_timeout_seconds == 0 {
            return Err(ManifestParseError::InvalidValue);
        }
        for other in manifest.components.iter().skip(index + 1) {
            if other.id == component.id {
                return Err(ManifestParseError::DuplicateComponentId);
            }
        }
        if component.role == SessionComponentRole::Shell {
            shell_count += 1;
        }
    }
    if shell_count == 0 {
        return Err(ManifestParseError::MissingShell);
    }
    if shell_count > 1 {
        return Err(ManifestParseError::DuplicateShellRole);
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::format;

    const VALID: &str = r#"
format_version = 1
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
"#;

    #[test]
    fn ids_reject_zero() {
        assert!(SessionId::new(0).is_none());
        assert!(SessionGeneration::new(0).is_none());
        assert!(SessionComponentId::new(0).is_none());
    }

    #[test]
    fn parse_valid_manifest() {
        let manifest = parse_manifest(VALID.as_bytes()).unwrap();
        assert_eq!(manifest.format_version, 1);
        assert_eq!(manifest.components.len(), 1);
        assert_eq!(manifest.components[0].restart_limit, 8);
    }

    #[test]
    fn duplicate_component_is_rejected() {
        let invalid = format!("{VALID}\n[[components]]\nid = \"org.sunlight.vortex-shell\"\nrole = \"startup-application\"\nrequired = false\nenabled = true\nlaunch_policy = \"session-start\"\nrestart_policy = \"never\"\nrestart_limit = 1\nrestart_window_seconds = 30\nreadiness_timeout_seconds = 10\norder = 1\n");
        assert_eq!(
            parse_manifest(invalid.as_bytes()).unwrap_err(),
            ManifestParseError::DuplicateComponentId
        );
    }

    #[test]
    fn missing_shell_is_rejected() {
        let invalid = r#"
format_version = 1
id = "org.sunlight.session.desktop"
name = "Sunlight Desktop"
kind = "desktop"
"#;
        assert_eq!(
            parse_manifest(invalid.as_bytes()).unwrap_err(),
            ManifestParseError::MissingShell
        );
    }

    #[test]
    fn invalid_transition_is_rejected() {
        let manifest = parse_manifest(VALID.as_bytes()).unwrap();
        let mut record = SessionRecord::new(
            SessionId::new(1).unwrap(),
            SessionGeneration::new(1).unwrap(),
            1000,
            1000,
            SessionKind::Desktop,
            1,
            &manifest,
        );
        assert_eq!(
            record.transition(SessionState::Running),
            Err(TransitionError::InvalidTransition)
        );
    }
}
