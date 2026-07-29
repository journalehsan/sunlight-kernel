//! Authoritative session attestation and readiness evidence boundary.
//!
//! This module deliberately contains no GUI transport, planner, executor, or
//! launcher entry point.  It converts facts supplied by the session authority
//! and capability-authenticated lifecycle sources into opaque values consumed
//! by the GUI bridge and outcome observer.  GUI payloads never enter here.

use heapless::{Deque, String, Vec};

use crate::action_intent::{ActionTarget, RequestedBy, SessionId, TypedIdentifier};
use crate::confirmation::AuthorityTime;
use crate::executor::{ExecutionId, ExecutionResult, LaunchCorrelationToken};
use crate::gui_bridge::{
    CorrelatedGuiReadinessEvidence, GuiReadinessEvidenceId, ReadinessCorrelationError,
    TrustedGuiReadinessKind, TrustedReadinessCorrelation, TrustedReadinessSource,
    VerifiedGraphicalSession,
};
use crate::outcome::{EvidenceId, ObservationEvidence, ObservationEvidenceKind, TrustedSourceKind};

pub const MAX_TRUSTED_GUI_ATTESTATIONS: usize = 32;
pub const MAX_TRUSTED_READINESS_EVENTS: usize = 64;
pub const MAX_TRUSTED_SOURCE_SEQUENCES: usize = 32;
pub const TRUSTED_SESSION_PROTOCOL_VERSION: u16 = 1;
pub const SESSION_AUTHORITY_PROOF_LIFETIME_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WiseOwlLiveActionAvailability {
    DisabledByPolicy,
    SessionAuthorityUnavailable,
    ConsoleNotAttested,
    DisplayLifecycleUnavailable,
    ControlPanelLifecycleUnavailable,
    ReadyForApplicationActions,
    ReadyForSettingsActions,
    FullyReady,
}

/// Informational readiness only. The executor never accepts this value as an
/// authorization capability, and v1 policy remains disabled for live actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WiseOwlAuthorityConnections {
    pub session_authority: bool,
    pub console_attested: bool,
    pub display_lifecycle: bool,
    pub control_panel_lifecycle: bool,
    pub actions_disabled_by_policy: bool,
}

impl WiseOwlAuthorityConnections {
    pub const fn availability(self) -> WiseOwlLiveActionAvailability {
        if self.actions_disabled_by_policy {
            return WiseOwlLiveActionAvailability::DisabledByPolicy;
        }
        if !self.session_authority {
            return WiseOwlLiveActionAvailability::SessionAuthorityUnavailable;
        }
        if !self.console_attested {
            return WiseOwlLiveActionAvailability::ConsoleNotAttested;
        }
        match (self.display_lifecycle, self.control_panel_lifecycle) {
            (false, _) => WiseOwlLiveActionAvailability::DisplayLifecycleUnavailable,
            (true, false) => WiseOwlLiveActionAvailability::ControlPanelLifecycleUnavailable,
            (true, true) => WiseOwlLiveActionAvailability::FullyReady,
        }
    }

    pub const fn application_availability(self) -> WiseOwlLiveActionAvailability {
        match self.availability() {
            WiseOwlLiveActionAvailability::ControlPanelLifecycleUnavailable
                if self.display_lifecycle =>
            {
                WiseOwlLiveActionAvailability::ReadyForApplicationActions
            }
            other => other,
        }
    }

    pub const fn settings_availability(self) -> WiseOwlLiveActionAvailability {
        if self.actions_disabled_by_policy {
            return WiseOwlLiveActionAvailability::DisabledByPolicy;
        }
        if !self.session_authority {
            return WiseOwlLiveActionAvailability::SessionAuthorityUnavailable;
        }
        if !self.console_attested {
            return WiseOwlLiveActionAvailability::ConsoleNotAttested;
        }
        if self.control_panel_lifecycle {
            WiseOwlLiveActionAvailability::ReadyForSettingsActions
        } else {
            WiseOwlLiveActionAvailability::ControlPanelLifecycleUnavailable
        }
    }
}

#[cfg(feature = "sunlightos")]
pub fn materialize_kernel_session_proof(
    proof: sunlight_ipc::SessionAuthorityProof,
    now: u64,
) -> Result<VerifiedGraphicalSession, SessionAttestationError> {
    let validated = sunlight_ipc::wiseowl_consume_session_authority_proof(proof)
        .ok_or(SessionAttestationError::Revoked)?;
    let session_id = SessionId(validated.session_id);
    let requester = RequestedBy::User(validated.caller_uid);
    VerifiedGraphicalSession::from_authority(
        validated.caller_process_generation,
        session_id,
        requester,
        "en",
        validated.session_generation,
        validated.session_generation,
        validated.session_generation,
        validated.session_generation,
        sunlight_ipc::current_process_generation(),
        now,
        now.saturating_add(SESSION_AUTHORITY_PROOF_LIFETIME_MS),
    )
    .ok_or(SessionAttestationError::CapacityExhausted)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionAttestationId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GuiClientInstanceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionGeneration(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuthorityGeneration(pub u64);

/// The only source classes that can obtain a readiness capability.  There is
/// intentionally no GUI source class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedLifecycleSource {
    DisplayServer,
    ApplicationRegistry,
    ControlPanel,
    ProcessLifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAttestationError {
    UnsupportedProtocol,
    NoActiveGraphicalSession,
    CallerNotRegisteredGui,
    CallerPidMismatch,
    SessionNotActive,
    Expired,
    Revoked,
    AuthorityRestarted,
    CapacityExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedReadinessError {
    UnauthorizedSource,
    SourceGenerationChanged,
    DuplicateOrOutOfOrder,
    QueueFull,
    Correlation(ReadinessCorrelationError),
}

/// Session facts supplied only by the authoritative session manager.  It is
/// crate-private on purpose: neither the GUI nor `wiseowl-braind` can turn a
/// payload session id or caller pid into a trusted session.
#[derive(Debug, Clone)]
pub(crate) struct AuthoritativeGraphicalSession {
    pub session_id: SessionId,
    pub requester: RequestedBy,
    pub requester_pid: ProcessId,
    pub gui_instance_id: GuiClientInstanceId,
    pub desktop_process_id: ProcessId,
    pub locale: String<16>,
    pub session_generation: SessionGeneration,
    pub runtime_snapshot_generation: u64,
    pub application_registry_generation: u64,
    pub settings_registry_generation: u64,
    pub active: bool,
}

#[derive(Debug, Clone, Copy)]
struct IssuedAttestation {
    id: SessionAttestationId,
    session_id: SessionId,
    requester: RequestedBy,
    requester_pid: ProcessId,
    gui_instance_id: GuiClientInstanceId,
    session_generation: SessionGeneration,
    authority_generation: AuthorityGeneration,
    expires_at: u64,
    revoked: bool,
}

/// The daemon-side mirror of the authoritative graphical-session service.
/// Only a sessiond/display-session adapter inside this crate can install the
/// current record.  Attestation cannot be created from IPC payload values.
pub struct TrustedGraphicalSessionAuthority<const N: usize = MAX_TRUSTED_GUI_ATTESTATIONS> {
    active: Option<AuthoritativeGraphicalSession>,
    issued: Deque<IssuedAttestation, N>,
    next_id: u64,
    generation: AuthorityGeneration,
}

impl<const N: usize> TrustedGraphicalSessionAuthority<N> {
    pub const fn new() -> Self {
        Self {
            active: None,
            issued: Deque::new(),
            next_id: 1,
            generation: AuthorityGeneration(1),
        }
    }

    /// This is the sole production installation point.  It is intentionally
    /// unavailable to the GUI bridge, coordinator, planner, and executor.
    pub(crate) fn install_active_session(&mut self, session: AuthoritativeGraphicalSession) {
        self.revoke_all();
        self.active = Some(session);
    }

    pub(crate) fn attest_registered_gui_client(
        &mut self,
        caller_pid: ProcessId,
        client_instance_id: GuiClientInstanceId,
        protocol_version: u16,
        now: u64,
        expires_at: u64,
    ) -> Result<VerifiedGraphicalSession, SessionAttestationError> {
        if protocol_version != TRUSTED_SESSION_PROTOCOL_VERSION {
            return Err(SessionAttestationError::UnsupportedProtocol);
        }
        let active = self
            .active
            .as_ref()
            .ok_or(SessionAttestationError::NoActiveGraphicalSession)?;
        if !active.active {
            return Err(SessionAttestationError::SessionNotActive);
        }
        if active.requester_pid != caller_pid {
            return Err(SessionAttestationError::CallerPidMismatch);
        }
        if active.gui_instance_id != client_instance_id {
            return Err(SessionAttestationError::CallerNotRegisteredGui);
        }
        if expires_at <= now {
            return Err(SessionAttestationError::Expired);
        }
        if self.issued.is_full() {
            return Err(SessionAttestationError::CapacityExhausted);
        }
        let id = SessionAttestationId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        let verified = VerifiedGraphicalSession::from_authority(
            id.0,
            active.session_id,
            active.requester,
            active.locale.as_str(),
            active.runtime_snapshot_generation,
            active.application_registry_generation,
            active.settings_registry_generation,
            active.session_generation.0,
            self.generation.0,
            now,
            expires_at,
        )
        .ok_or(SessionAttestationError::CapacityExhausted)?;
        self.issued
            .push_back(IssuedAttestation {
                id,
                session_id: active.session_id,
                requester: active.requester,
                requester_pid: active.requester_pid,
                gui_instance_id: active.gui_instance_id,
                session_generation: active.session_generation,
                authority_generation: self.generation,
                expires_at,
                revoked: false,
            })
            .map_err(|_| SessionAttestationError::CapacityExhausted)?;
        Ok(verified)
    }

    pub fn validate(
        &self,
        verified: &VerifiedGraphicalSession,
        caller_pid: ProcessId,
        client_instance_id: GuiClientInstanceId,
        now: u64,
    ) -> Result<(), SessionAttestationError> {
        let issued = self
            .issued
            .iter()
            .find(|entry| entry.id.0 == verified.attestation_id())
            .ok_or(SessionAttestationError::Revoked)?;
        let active = self
            .active
            .as_ref()
            .ok_or(SessionAttestationError::NoActiveGraphicalSession)?;
        if issued.revoked || now > issued.expires_at || now > verified.expires_at() {
            return Err(SessionAttestationError::Expired);
        }
        if issued.authority_generation != self.generation
            || verified.authority_generation() != self.generation.0
        {
            return Err(SessionAttestationError::AuthorityRestarted);
        }
        if issued.requester_pid != caller_pid || issued.gui_instance_id != client_instance_id {
            return Err(SessionAttestationError::CallerPidMismatch);
        }
        if !active.active
            || active.session_id != issued.session_id
            || active.requester != issued.requester
            || active.session_generation != issued.session_generation
            || verified.session_id() != active.session_id
            || verified.requester() != active.requester
            || verified.session_generation() != active.session_generation.0
        {
            return Err(SessionAttestationError::Revoked);
        }
        Ok(())
    }

    pub(crate) fn revoke_session(&mut self, session_id: SessionId) {
        for entry in self.issued.iter_mut() {
            if entry.session_id == session_id {
                entry.revoked = true;
            }
        }
        if self
            .active
            .as_ref()
            .is_some_and(|entry| entry.session_id == session_id)
        {
            self.active = None;
        }
    }

    pub(crate) fn revoke_gui_process(&mut self, pid: ProcessId) {
        for entry in self.issued.iter_mut() {
            if entry.requester_pid == pid {
                entry.revoked = true;
            }
        }
    }

    pub(crate) fn restart_authority(&mut self) {
        self.revoke_all();
        self.active = None;
        self.generation = AuthorityGeneration(self.generation.0.saturating_add(1));
    }

    fn revoke_all(&mut self) {
        for entry in self.issued.iter_mut() {
            entry.revoked = true;
        }
    }
}

impl<const N: usize> Default for TrustedGraphicalSessionAuthority<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// An unforgeable capability retained by the trusted lifecycle adapter.  Its
/// private nonce makes it impossible for the GUI to register a source or send
/// a readiness event through the public type system.
#[derive(Debug)]
pub struct TrustedReadinessSourceCapability {
    source: TrustedLifecycleSource,
    nonce: u64,
    generation: u64,
}

#[derive(Debug, Clone)]
struct SourceSequence {
    source: TrustedLifecycleSource,
    generation: u64,
    last_sequence: u64,
}

/// Daemon-facing ingress for trusted lifecycle sources.  It owns both source
/// authentication and the existing exact execution/session/target correlation
/// table.  A GUI has no constructor for its capabilities or evidence tokens.
pub struct TrustedReadinessIngress<
    const EXECUTIONS: usize = 16,
    const SOURCES: usize = MAX_TRUSTED_SOURCE_SEQUENCES,
    const EVENTS: usize = MAX_TRUSTED_READINESS_EVENTS,
> {
    correlation: TrustedReadinessCorrelation<EXECUTIONS>,
    source_sequences: Vec<SourceSequence, SOURCES>,
    events: Deque<CorrelatedGuiReadinessEvidence, EVENTS>,
    next_nonce: u64,
}

impl<const EXECUTIONS: usize, const SOURCES: usize, const EVENTS: usize>
    TrustedReadinessIngress<EXECUTIONS, SOURCES, EVENTS>
{
    pub const fn new() -> Self {
        Self {
            correlation: TrustedReadinessCorrelation::new(),
            source_sequences: Vec::new(),
            events: Deque::new(),
            next_nonce: 1,
        }
    }

    pub(crate) fn issue_source_capability(
        &mut self,
        source: TrustedLifecycleSource,
        generation: u64,
    ) -> TrustedReadinessSourceCapability {
        let nonce = self.next_nonce;
        self.next_nonce = self.next_nonce.saturating_add(1);
        TrustedReadinessSourceCapability {
            source,
            nonce,
            generation,
        }
    }

    pub(crate) fn register_execution(
        &mut self,
        execution_id: ExecutionId,
        session_id: SessionId,
        target_id: TypedIdentifier,
        token: LaunchCorrelationToken,
        settings: bool,
    ) -> Result<(), TrustedReadinessError> {
        self.correlation
            .register_execution(execution_id, session_id, target_id, token, settings)
            .map_err(TrustedReadinessError::Correlation)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ingest(
        &mut self,
        capability: &TrustedReadinessSourceCapability,
        evidence_id: GuiReadinessEvidenceId,
        source_identity: TypedIdentifier,
        execution_id: ExecutionId,
        token: LaunchCorrelationToken,
        session_id: SessionId,
        target_id: TypedIdentifier,
        sequence: u64,
        timestamp: u64,
        kind: TrustedGuiReadinessKind,
    ) -> Result<(), TrustedReadinessError> {
        if capability.nonce == 0 {
            return Err(TrustedReadinessError::UnauthorizedSource);
        }
        let source = lifecycle_source(capability.source);
        let entry = if let Some(entry) = self
            .source_sequences
            .iter_mut()
            .find(|entry| entry.source == capability.source)
        {
            if entry.generation != capability.generation {
                return Err(TrustedReadinessError::SourceGenerationChanged);
            }
            entry
        } else {
            self.source_sequences
                .push(SourceSequence {
                    source: capability.source,
                    generation: capability.generation,
                    last_sequence: 0,
                })
                .map_err(|_| TrustedReadinessError::QueueFull)?;
            self.source_sequences.last_mut().unwrap()
        };
        if sequence <= entry.last_sequence {
            return Err(TrustedReadinessError::DuplicateOrOutOfOrder);
        }
        if self.events.is_full() {
            return Err(TrustedReadinessError::QueueFull);
        }
        let evidence = self
            .correlation
            .ingest(
                evidence_id,
                source,
                source_identity,
                execution_id,
                token,
                session_id,
                target_id,
                capability.generation,
                sequence,
                timestamp,
                kind,
            )
            .map_err(TrustedReadinessError::Correlation)?;
        entry.last_sequence = sequence;
        self.events
            .push_back(evidence)
            .map_err(|_| TrustedReadinessError::QueueFull)
    }

    pub(crate) fn next_event(&mut self) -> Option<CorrelatedGuiReadinessEvidence> {
        self.events.pop_front()
    }

    /// Converts only ingress-accepted evidence into an observer envelope.  It
    /// is crate-private so neither the GUI nor a general IPC client can make
    /// observer evidence from an arbitrary title, PID, or log line.
    pub(crate) fn into_observer_evidence(
        evidence: CorrelatedGuiReadinessEvidence,
        execution: &ExecutionResult,
    ) -> Option<ObservationEvidence> {
        if evidence.execution_id != execution.execution_id()
            || evidence.session_id != execution.session_id()
            || evidence.target_id.as_str() != target_id(execution.target())?.as_str()
            || evidence.correlation_token() != execution.correlation_token()?
        {
            return None;
        }
        let (source_kind, kind) = match (evidence.source, evidence.kind) {
            (
                TrustedReadinessSource::ApplicationRegistry,
                TrustedGuiReadinessKind::ApplicationRegistered,
            ) => (
                TrustedSourceKind::ApplicationRegistry,
                ObservationEvidenceKind::ApplicationRegistered,
            ),
            (
                TrustedReadinessSource::ApplicationRegistry,
                TrustedGuiReadinessKind::ApplicationReady,
            ) => (
                TrustedSourceKind::ApplicationRegistry,
                ObservationEvidenceKind::ReadySignal,
            ),
            (
                TrustedReadinessSource::DisplayServer,
                TrustedGuiReadinessKind::FirstWindowRegistered,
            ) => (
                TrustedSourceKind::DisplayServer,
                ObservationEvidenceKind::WindowRegistered,
            ),
            (
                TrustedReadinessSource::ControlPanel,
                TrustedGuiReadinessKind::SettingsPageActivated,
            ) => (
                TrustedSourceKind::ControlPanel,
                ObservationEvidenceKind::SettingsPageActivated(evidence.target_id.clone()),
            ),
            (TrustedReadinessSource::ControlPanel, TrustedGuiReadinessKind::SettingsPageReady) => (
                TrustedSourceKind::ControlPanel,
                ObservationEvidenceKind::ReadySignal,
            ),
            (
                TrustedReadinessSource::ProcessLifecycle,
                TrustedGuiReadinessKind::ProcessExitedEarly,
            ) => (
                TrustedSourceKind::ApplicationRegistry,
                ObservationEvidenceKind::Failed { public_code: 1 },
            ),
            (TrustedReadinessSource::ControlPanel, TrustedGuiReadinessKind::ProcessExitedEarly) => {
                (
                    TrustedSourceKind::ControlPanel,
                    ObservationEvidenceKind::Failed { public_code: 1 },
                )
            }
            _ => return None,
        };
        let correlation_token = evidence.correlation_token();
        Some(ObservationEvidence::trusted(
            EvidenceId(evidence.evidence_id.0),
            source_kind,
            evidence.source_identity,
            evidence.session_id,
            AuthorityTime(evidence.timestamp),
            execution.registry_generation(),
            evidence.execution_id,
            correlation_token,
            execution.target().clone(),
            kind,
        ))
    }
}

impl<const EXECUTIONS: usize, const SOURCES: usize, const EVENTS: usize> Default
    for TrustedReadinessIngress<EXECUTIONS, SOURCES, EVENTS>
{
    fn default() -> Self {
        Self::new()
    }
}

fn lifecycle_source(source: TrustedLifecycleSource) -> TrustedReadinessSource {
    match source {
        TrustedLifecycleSource::DisplayServer => TrustedReadinessSource::DisplayServer,
        TrustedLifecycleSource::ApplicationRegistry => TrustedReadinessSource::ApplicationRegistry,
        TrustedLifecycleSource::ControlPanel => TrustedReadinessSource::ControlPanel,
        TrustedLifecycleSource::ProcessLifecycle => TrustedReadinessSource::ProcessLifecycle,
    }
}

fn target_id(target: &ActionTarget) -> Option<&TypedIdentifier> {
    match target {
        ActionTarget::Application(id) | ActionTarget::SettingsPage(id) => Some(id),
        _ => None,
    }
}

#[cfg(any(
    test,
    feature = "trusted-session-readiness-v1-test",
    feature = "gui-live-action-activation-v1-test"
))]
pub fn run_deterministic_trust_gate() -> bool {
    let mut authority = TrustedGraphicalSessionAuthority::<4>::new();
    let session = test_session();
    authority.install_active_session(session);
    let verified = match authority.attest_registered_gui_client(
        ProcessId(40),
        GuiClientInstanceId(2),
        TRUSTED_SESSION_PROTOCOL_VERSION,
        10,
        20,
    ) {
        Ok(value) => value,
        Err(_) => return false,
    };
    if authority
        .validate(&verified, ProcessId(41), GuiClientInstanceId(2), 11)
        .is_ok()
    {
        return false;
    }
    if authority
        .validate(&verified, ProcessId(40), GuiClientInstanceId(2), 11)
        .is_err()
    {
        return false;
    }
    authority.revoke_gui_process(ProcessId(40));
    if authority
        .validate(&verified, ProcessId(40), GuiClientInstanceId(2), 11)
        .is_ok()
    {
        return false;
    }

    let token = LaunchCorrelationToken::new_for_test([0x31; 32]);
    let app = match TypedIdentifier::new("calculator") {
        Ok(value) => value,
        Err(_) => return false,
    };
    let display = match TypedIdentifier::new("display-server") {
        Ok(value) => value,
        Err(_) => return false,
    };
    let mut readiness = TrustedReadinessIngress::<2, 3, 3>::new();
    if readiness
        .register_execution(ExecutionId(1), SessionId(7), app.clone(), token, false)
        .is_err()
    {
        return false;
    }
    let display_cap = readiness.issue_source_capability(TrustedLifecycleSource::DisplayServer, 1);
    if readiness
        .ingest(
            &display_cap,
            GuiReadinessEvidenceId(1),
            display,
            ExecutionId(1),
            token,
            SessionId(7),
            app.clone(),
            1,
            12,
            TrustedGuiReadinessKind::ApplicationReady,
        )
        .is_err()
        || readiness.next_event().is_none()
    {
        return false;
    }
    true
}

#[cfg(any(
    test,
    feature = "trusted-session-readiness-v1-test",
    feature = "gui-live-action-activation-v1-test"
))]
fn test_session() -> AuthoritativeGraphicalSession {
    AuthoritativeGraphicalSession {
        session_id: SessionId(7),
        requester: RequestedBy::User(9),
        requester_pid: ProcessId(40),
        gui_instance_id: GuiClientInstanceId(2),
        desktop_process_id: ProcessId(30),
        locale: String::try_from("en").unwrap(),
        session_generation: SessionGeneration(3),
        runtime_snapshot_generation: 11,
        application_registry_generation: 1,
        settings_registry_generation: 1,
        active: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority() -> TrustedGraphicalSessionAuthority<4> {
        let mut authority = TrustedGraphicalSessionAuthority::new();
        authority.install_active_session(test_session());
        authority
    }

    #[test]
    fn availability_is_informational_and_optional_absence_is_typed() {
        assert_eq!(
            WiseOwlAuthorityConnections {
                session_authority: false,
                console_attested: false,
                display_lifecycle: false,
                control_panel_lifecycle: false,
                actions_disabled_by_policy: false,
            }
            .availability(),
            WiseOwlLiveActionAvailability::SessionAuthorityUnavailable
        );
        assert_eq!(
            WiseOwlAuthorityConnections {
                session_authority: true,
                console_attested: true,
                display_lifecycle: true,
                control_panel_lifecycle: true,
                actions_disabled_by_policy: true,
            }
            .availability(),
            WiseOwlLiveActionAvailability::DisabledByPolicy
        );
    }

    #[test]
    fn verified_session_source_has_no_serialization_or_public_constructor() {
        let source = include_str!("gui_bridge.rs");
        let start = source.find("pub struct VerifiedGraphicalSession").unwrap();
        let end = source[start..].find("pub enum GuiBindingError").unwrap() + start;
        let definition = &source[start..end];
        assert!(!definition.contains("Serialize"));
        assert!(!definition.contains("Deserialize"));
        assert!(!definition.contains("pub fn from_authority"));
    }

    #[test]
    fn active_registered_gui_is_attested_and_cross_process_replay_is_rejected() {
        let mut authority = authority();
        let verified = authority
            .attest_registered_gui_client(ProcessId(40), GuiClientInstanceId(2), 1, 10, 20)
            .unwrap();
        assert!(authority
            .validate(&verified, ProcessId(40), GuiClientInstanceId(2), 11)
            .is_ok());
        assert_eq!(
            authority.validate(&verified, ProcessId(41), GuiClientInstanceId(2), 11),
            Err(SessionAttestationError::CallerPidMismatch)
        );
        assert_eq!(
            authority.attest_registered_gui_client(
                ProcessId(40),
                GuiClientInstanceId(9),
                1,
                10,
                20
            ),
            Err(SessionAttestationError::CallerNotRegisteredGui)
        );
    }

    #[test]
    fn session_logout_gui_exit_and_authority_restart_revoke_attestations() {
        let mut authority = authority();
        let verified = authority
            .attest_registered_gui_client(ProcessId(40), GuiClientInstanceId(2), 1, 10, 20)
            .unwrap();
        authority.revoke_gui_process(ProcessId(40));
        assert!(authority
            .validate(&verified, ProcessId(40), GuiClientInstanceId(2), 11)
            .is_err());

        authority.install_active_session(test_session());
        let verified = authority
            .attest_registered_gui_client(ProcessId(40), GuiClientInstanceId(2), 1, 10, 20)
            .unwrap();
        authority.revoke_session(SessionId(7));
        assert!(authority
            .validate(&verified, ProcessId(40), GuiClientInstanceId(2), 11)
            .is_err());
        authority.restart_authority();
        assert!(authority
            .validate(&verified, ProcessId(40), GuiClientInstanceId(2), 11)
            .is_err());
    }

    #[test]
    fn readiness_requires_capability_exact_correlation_and_monotonic_sequence() {
        let token = LaunchCorrelationToken::new_for_test([0x11; 32]);
        let app = TypedIdentifier::new("calculator").unwrap();
        let display = TypedIdentifier::new("display-server").unwrap();
        let wrong = TypedIdentifier::new("network").unwrap();
        let mut ingress = TrustedReadinessIngress::<2, 2, 2>::new();
        ingress
            .register_execution(ExecutionId(1), SessionId(7), app.clone(), token, false)
            .unwrap();
        let display_cap = ingress.issue_source_capability(TrustedLifecycleSource::DisplayServer, 1);
        assert_eq!(
            ingress.ingest(
                &display_cap,
                GuiReadinessEvidenceId(1),
                display.clone(),
                ExecutionId(1),
                token,
                SessionId(7),
                wrong,
                1,
                10,
                TrustedGuiReadinessKind::ApplicationReady,
            ),
            Err(TrustedReadinessError::Correlation(
                ReadinessCorrelationError::WrongTarget
            ))
        );
        ingress
            .ingest(
                &display_cap,
                GuiReadinessEvidenceId(2),
                display,
                ExecutionId(1),
                token,
                SessionId(7),
                app,
                1,
                11,
                TrustedGuiReadinessKind::ApplicationReady,
            )
            .unwrap();
        assert_eq!(
            ingress.ingest(
                &display_cap,
                GuiReadinessEvidenceId(3),
                TypedIdentifier::new("display-server").unwrap(),
                ExecutionId(1),
                token,
                SessionId(7),
                TypedIdentifier::new("calculator").unwrap(),
                1,
                12,
                TrustedGuiReadinessKind::ApplicationReady,
            ),
            Err(TrustedReadinessError::DuplicateOrOutOfOrder)
        );
    }

    #[test]
    fn settings_require_control_panel_and_exact_page() {
        let token = LaunchCorrelationToken::new_for_test([0x22; 32]);
        let page = TypedIdentifier::new("display").unwrap();
        let panel = TypedIdentifier::new("control-panel").unwrap();
        let mut ingress = TrustedReadinessIngress::<2, 2, 2>::new();
        ingress
            .register_execution(ExecutionId(2), SessionId(7), page.clone(), token, true)
            .unwrap();
        let display_cap = ingress.issue_source_capability(TrustedLifecycleSource::DisplayServer, 1);
        assert_eq!(
            ingress.ingest(
                &display_cap,
                GuiReadinessEvidenceId(1),
                panel.clone(),
                ExecutionId(2),
                token,
                SessionId(7),
                page.clone(),
                1,
                10,
                TrustedGuiReadinessKind::SettingsPageActivated,
            ),
            Err(TrustedReadinessError::Correlation(
                ReadinessCorrelationError::WrongSource
            ))
        );
        let panel_cap = ingress.issue_source_capability(TrustedLifecycleSource::ControlPanel, 1);
        assert_eq!(
            ingress.ingest(
                &panel_cap,
                GuiReadinessEvidenceId(2),
                panel.clone(),
                ExecutionId(2),
                token,
                SessionId(7),
                TypedIdentifier::new("network").unwrap(),
                1,
                11,
                TrustedGuiReadinessKind::SettingsPageActivated,
            ),
            Err(TrustedReadinessError::Correlation(
                ReadinessCorrelationError::WrongTarget
            ))
        );
        ingress
            .ingest(
                &panel_cap,
                GuiReadinessEvidenceId(3),
                panel,
                ExecutionId(2),
                token,
                SessionId(7),
                page,
                1,
                12,
                TrustedGuiReadinessKind::SettingsPageActivated,
            )
            .unwrap();
    }
}
