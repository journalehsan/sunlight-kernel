//! Wise Owl GUI Bridge Foundation v1.
//!
//! This module is deliberately downstream of the action pipeline.  It owns
//! public presentation events and trusted binding/correlation validation, but
//! has no planner, executor, launch, or policy-mutation entry point.

use heapless::{Deque, String, Vec};
use sha2::{Digest, Sha256};

use crate::action_intent::{RequestedBy, SessionId, TypedIdentifier};
use crate::action_receipt::{ActionReceiptId, ActionReceiptTerminalStatus};
use crate::coordinator::CoordinatorActionId;
use crate::executor::{ExecutionId, LaunchCorrelationToken};
use crate::planner::{ConversationId, PlannerRequestId};

pub const GUI_BRIDGE_PROTOCOL_VERSION: u16 = 1;
pub const MAX_GUI_BRIDGE_EVENTS: usize = 64;
pub const MAX_GUI_BRIDGE_DEDUP: usize = 128;
pub const MAX_GUI_PUBLIC_TEXT: usize = 256;
pub const MAX_GUI_LOCALIZED_KEY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GuiSessionBindingId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GuiEventId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GuiReadinessEvidenceId(pub u64);

/// Presentation-only coordinator state.  It intentionally carries no policy,
/// executor, grant, or launch-adapter data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatorPresentationKind {
    ConversationAccepted,
    NoAction,
    ClarificationRequired,
    ConfirmationRequired,
    PolicyEvaluating,
    PolicyDenied,
    ReadyForExecution,
    DispatchAccepted,
    AwaitingOutcome,
    OutcomeReady,
    OutcomeFailed,
    OutcomeTimedOut,
    Cancelled,
    Expired,
    SessionInvalidated,
    RegistryInvalidated,
}

impl CoordinatorPresentationKind {
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::NoAction
                | Self::PolicyDenied
                | Self::OutcomeReady
                | Self::OutcomeFailed
                | Self::OutcomeTimedOut
                | Self::Cancelled
                | Self::Expired
                | Self::SessionInvalidated
                | Self::RegistryInvalidated
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicPresentationPayload {
    localized_key: String<MAX_GUI_LOCALIZED_KEY>,
    text: String<MAX_GUI_PUBLIC_TEXT>,
}

impl PublicPresentationPayload {
    pub fn new(localized_key: &str, text: &str) -> Option<Self> {
        Some(Self {
            localized_key: String::try_from(localized_key).ok()?,
            text: String::try_from(text).ok()?,
        })
    }

    pub fn localized_key(&self) -> &str {
        self.localized_key.as_str()
    }

    pub fn text(&self) -> &str {
        self.text.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinatorPresentationUpdate {
    pub conversation_id: ConversationId,
    pub request_id: PlannerRequestId,
    pub action_id: Option<CoordinatorActionId>,
    pub session_id: SessionId,
    pub sequence: u64,
    pub kind: CoordinatorPresentationKind,
    pub payload: PublicPresentationPayload,
}

impl CoordinatorPresentationUpdate {
    pub const fn terminal(&self) -> bool {
        self.kind.terminal()
    }
}

/// Issued only by `GuiSessionBindingAuthority`; its fields remain private so a
/// GUI process can carry a binding but cannot manufacture or alter one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WiseOwlGuiSessionBinding {
    binding_id: GuiSessionBindingId,
    session_id: SessionId,
    requester: RequestedBy,
    locale: String<16>,
    runtime_snapshot_generation: u64,
    application_registry_generation: u64,
    settings_registry_generation: u64,
    issued_at: u64,
    expires_at: u64,
    integrity_digest: [u8; 16],
}

impl WiseOwlGuiSessionBinding {
    pub const fn binding_id(&self) -> GuiSessionBindingId {
        self.binding_id
    }
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub const fn requester(&self) -> RequestedBy {
        self.requester
    }
    pub fn locale(&self) -> &str {
        self.locale.as_str()
    }
    pub const fn runtime_snapshot_generation(&self) -> u64 {
        self.runtime_snapshot_generation
    }
    pub const fn application_registry_generation(&self) -> u64 {
        self.application_registry_generation
    }
    pub const fn settings_registry_generation(&self) -> u64 {
        self.settings_registry_generation
    }
    pub const fn expires_at(&self) -> u64 {
        self.expires_at
    }
}

/// A session authority obtains this value from the active graphical session
/// service.  Its constructor is crate-private, preventing GUI construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedGraphicalSession {
    attestation_id: u64,
    session_id: SessionId,
    requester: RequestedBy,
    locale: String<16>,
    session_generation: u64,
    authority_generation: u64,
    issued_at: u64,
    expires_at: u64,
    runtime_snapshot_generation: u64,
    application_registry_generation: u64,
    settings_registry_generation: u64,
}

impl VerifiedGraphicalSession {
    /// Only the trusted graphical-session authority may create this value.
    /// Its constructor is crate-private so a GUI IPC payload cannot become an
    /// attestation merely by carrying a session id or PID.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_authority(
        attestation_id: u64,
        session_id: SessionId,
        requester: RequestedBy,
        locale: &str,
        runtime_snapshot_generation: u64,
        application_registry_generation: u64,
        settings_registry_generation: u64,
        session_generation: u64,
        authority_generation: u64,
        issued_at: u64,
        expires_at: u64,
    ) -> Option<Self> {
        if expires_at <= issued_at {
            return None;
        }
        Some(Self {
            attestation_id,
            session_id,
            requester,
            locale: String::try_from(locale).ok()?,
            session_generation,
            authority_generation,
            issued_at,
            expires_at,
            runtime_snapshot_generation,
            application_registry_generation,
            settings_registry_generation,
        })
    }

    pub const fn attestation_id(&self) -> u64 {
        self.attestation_id
    }
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub const fn requester(&self) -> RequestedBy {
        self.requester
    }
    pub const fn session_generation(&self) -> u64 {
        self.session_generation
    }
    pub const fn authority_generation(&self) -> u64 {
        self.authority_generation
    }
    pub const fn expires_at(&self) -> u64 {
        self.expires_at
    }

    #[cfg(test)]
    fn for_test(
        session_id: SessionId,
        requester: RequestedBy,
        runtime_snapshot_generation: u64,
    ) -> Self {
        Self::from_authority(
            1,
            session_id,
            requester,
            "en",
            runtime_snapshot_generation,
            1,
            1,
            1,
            1,
            0,
            u64::MAX,
        )
        .unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiBindingError {
    Expired,
    InvalidIntegrity,
    WrongSession,
    WrongRequester,
    StaleRuntime,
    RegistryChanged,
}

pub struct GuiSessionBindingAuthority {
    secret: [u8; 32],
    next_id: u64,
}

impl GuiSessionBindingAuthority {
    pub const fn new(secret: [u8; 32]) -> Self {
        Self { secret, next_id: 1 }
    }

    pub fn issue(
        &mut self,
        verified: VerifiedGraphicalSession,
        issued_at: u64,
        expires_at: u64,
    ) -> Option<WiseOwlGuiSessionBinding> {
        if expires_at <= issued_at {
            return None;
        }
        let binding_id = GuiSessionBindingId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        let mut binding = WiseOwlGuiSessionBinding {
            binding_id,
            session_id: verified.session_id,
            requester: verified.requester,
            locale: verified.locale,
            runtime_snapshot_generation: verified.runtime_snapshot_generation,
            application_registry_generation: verified.application_registry_generation,
            settings_registry_generation: verified.settings_registry_generation,
            issued_at,
            expires_at,
            integrity_digest: [0; 16],
        };
        binding.integrity_digest = self.digest(&binding);
        Some(binding)
    }

    pub fn verify(
        &self,
        binding: &WiseOwlGuiSessionBinding,
        session_id: SessionId,
        requester: RequestedBy,
        runtime_snapshot_generation: u64,
        application_registry_generation: u64,
        settings_registry_generation: u64,
        now: u64,
    ) -> Result<(), GuiBindingError> {
        if binding.integrity_digest != self.digest(binding) {
            return Err(GuiBindingError::InvalidIntegrity);
        }
        if now > binding.expires_at {
            return Err(GuiBindingError::Expired);
        }
        if binding.session_id != session_id {
            return Err(GuiBindingError::WrongSession);
        }
        if binding.requester != requester {
            return Err(GuiBindingError::WrongRequester);
        }
        if binding.runtime_snapshot_generation != runtime_snapshot_generation {
            return Err(GuiBindingError::StaleRuntime);
        }
        if binding.application_registry_generation != application_registry_generation
            || binding.settings_registry_generation != settings_registry_generation
        {
            return Err(GuiBindingError::RegistryChanged);
        }
        Ok(())
    }

    fn digest(&self, binding: &WiseOwlGuiSessionBinding) -> [u8; 16] {
        let mut hasher = Sha256::new();
        hasher.update(b"wiseowl.gui-session-binding.v1\0");
        hasher.update(self.secret);
        hasher.update(binding.binding_id.0.to_le_bytes());
        hasher.update(binding.session_id.0.to_le_bytes());
        hash_requester(&mut hasher, binding.requester);
        hasher.update(binding.locale.as_bytes());
        hasher.update(binding.runtime_snapshot_generation.to_le_bytes());
        hasher.update(binding.application_registry_generation.to_le_bytes());
        hasher.update(binding.settings_registry_generation.to_le_bytes());
        hasher.update(binding.issued_at.to_le_bytes());
        hasher.update(binding.expires_at.to_le_bytes());
        let hash: [u8; 32] = hasher.finalize().into();
        let mut digest = [0; 16];
        digest.copy_from_slice(&hash[..16]);
        digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WiseOwlGuiEventPayload {
    Presentation(CoordinatorPresentationUpdate),
    ReceiptSealed(ReceiptSealedView),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WiseOwlGuiEvent {
    pub protocol_version: u16,
    pub event_id: GuiEventId,
    pub conversation_id: ConversationId,
    pub session_id: SessionId,
    pub sequence: u64,
    pub terminal: bool,
    pub payload: WiseOwlGuiEventPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptSealedView {
    pub receipt_id: ActionReceiptId,
    pub action_id: CoordinatorActionId,
    pub terminal_status: ActionReceiptTerminalStatus,
    pub operation_label_key: String<MAX_GUI_LOCALIZED_KEY>,
    pub target_label_key: String<MAX_GUI_LOCALIZED_KEY>,
    pub readiness_observed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiEventError {
    QueueFull,
    WrongSession,
    InvalidSequence,
    UnknownEvent,
}

/// Bounded per-session delivery queue.  Events are retained until explicitly
/// acknowledged; acknowledgement is session-bound and terminal events are
/// idempotent at the broker boundary.
pub struct GuiEventBroker<const EVENTS: usize = MAX_GUI_BRIDGE_EVENTS> {
    queue: Deque<WiseOwlGuiEvent, EVENTS>,
    next_event_id: u64,
    last_sequence: u64,
    last_terminal: Option<(ConversationId, SessionId, u64, GuiEventId)>,
}

impl<const EVENTS: usize> GuiEventBroker<EVENTS> {
    pub const fn new() -> Self {
        Self {
            queue: Deque::new(),
            next_event_id: 1,
            last_sequence: 0,
            last_terminal: None,
        }
    }

    pub fn publish_presentation(
        &mut self,
        update: CoordinatorPresentationUpdate,
    ) -> Result<GuiEventId, GuiEventError> {
        let terminal_key = (update.conversation_id, update.session_id, update.sequence);
        if update.terminal() {
            if let Some((conversation_id, session_id, sequence, event_id)) = self.last_terminal {
                if (conversation_id, session_id, sequence) == terminal_key {
                    return Ok(event_id);
                }
            }
        }
        if update.sequence <= self.last_sequence {
            return Err(GuiEventError::InvalidSequence);
        }
        self.publish(
            update.conversation_id,
            update.session_id,
            update.sequence,
            update.terminal(),
            WiseOwlGuiEventPayload::Presentation(update),
        )
    }

    pub fn publish_receipt(
        &mut self,
        conversation_id: ConversationId,
        session_id: SessionId,
        sequence: u64,
        receipt: ReceiptSealedView,
    ) -> Result<GuiEventId, GuiEventError> {
        self.publish(
            conversation_id,
            session_id,
            sequence,
            true,
            WiseOwlGuiEventPayload::ReceiptSealed(receipt),
        )
    }

    fn publish(
        &mut self,
        conversation_id: ConversationId,
        session_id: SessionId,
        sequence: u64,
        terminal: bool,
        payload: WiseOwlGuiEventPayload,
    ) -> Result<GuiEventId, GuiEventError> {
        if terminal {
            if let Some((last_conversation, last_session, last_sequence, event_id)) =
                self.last_terminal
            {
                if (last_conversation, last_session, last_sequence)
                    == (conversation_id, session_id, sequence)
                {
                    return Ok(event_id);
                }
            }
        }
        if sequence <= self.last_sequence {
            return Err(GuiEventError::InvalidSequence);
        }
        if self.queue.is_full() {
            return Err(GuiEventError::QueueFull);
        }
        let event_id = GuiEventId(self.next_event_id);
        self.next_event_id = self.next_event_id.saturating_add(1);
        self.last_sequence = sequence;
        if terminal {
            self.last_terminal = Some((conversation_id, session_id, sequence, event_id));
        }
        self.queue
            .push_back(WiseOwlGuiEvent {
                protocol_version: GUI_BRIDGE_PROTOCOL_VERSION,
                event_id,
                conversation_id,
                session_id,
                sequence,
                terminal,
                payload,
            })
            .map_err(|_| GuiEventError::QueueFull)?;
        Ok(event_id)
    }

    pub fn next_for_session(
        &self,
        session_id: SessionId,
        after: Option<GuiEventId>,
    ) -> Option<WiseOwlGuiEvent> {
        self.queue
            .iter()
            .find(|event| {
                event.session_id == session_id && after.is_none_or(|id| event.event_id.0 > id.0)
            })
            .cloned()
    }

    pub fn acknowledge(
        &mut self,
        session_id: SessionId,
        event_id: GuiEventId,
    ) -> Result<(), GuiEventError> {
        let Some(event) = self.queue.iter().find(|event| event.event_id == event_id) else {
            return Err(GuiEventError::UnknownEvent);
        };
        if event.session_id != session_id {
            return Err(GuiEventError::WrongSession);
        }
        let mut retained = Deque::new();
        while let Some(event) = self.queue.pop_front() {
            if event.event_id != event_id {
                let _ = retained.push_back(event);
            }
        }
        self.queue = retained;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

impl<const EVENTS: usize> Default for GuiEventBroker<EVENTS> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedReadinessSource {
    DisplayServer,
    ApplicationRegistry,
    ControlPanel,
    ProcessLifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedGuiReadinessKind {
    ApplicationRegistered,
    FirstWindowRegistered,
    ApplicationReady,
    SettingsPageActivated,
    SettingsPageReady,
    ProcessExitedEarly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrelatedGuiReadinessEvidence {
    pub evidence_id: GuiReadinessEvidenceId,
    pub source: TrustedReadinessSource,
    pub source_identity: TypedIdentifier,
    pub execution_id: ExecutionId,
    pub session_id: SessionId,
    pub target_id: TypedIdentifier,
    pub source_generation: u64,
    pub sequence: u64,
    pub timestamp: u64,
    pub kind: TrustedGuiReadinessKind,
    correlation_token: LaunchCorrelationToken,
}

impl CorrelatedGuiReadinessEvidence {
    pub const fn correlation_token(&self) -> LaunchCorrelationToken {
        self.correlation_token
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_authenticated_lifecycle(
        evidence_id: GuiReadinessEvidenceId,
        source: TrustedReadinessSource,
        source_identity: TypedIdentifier,
        execution_id: ExecutionId,
        session_id: SessionId,
        target_id: TypedIdentifier,
        source_generation: u64,
        sequence: u64,
        timestamp: u64,
        kind: TrustedGuiReadinessKind,
        correlation_token: LaunchCorrelationToken,
    ) -> Self {
        Self {
            evidence_id,
            source,
            source_identity,
            execution_id,
            session_id,
            target_id,
            source_generation,
            sequence,
            timestamp,
            kind,
            correlation_token,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessCorrelationError {
    UnknownExecution,
    WrongCorrelation,
    WrongSession,
    WrongTarget,
    WrongSource,
    InvalidSequence,
}

#[derive(Debug, Clone)]
struct RegisteredExecution {
    execution_id: ExecutionId,
    session_id: SessionId,
    target_id: TypedIdentifier,
    token: LaunchCorrelationToken,
    last_sequence: u64,
    settings: bool,
}

/// Daemon-owned correlation table. Registration is crate-private and receives
/// the opaque executor token directly; the GUI cannot create evidence or add
/// an execution to this table.
pub struct TrustedReadinessCorrelation<const N: usize = 16> {
    executions: Vec<RegisteredExecution, N>,
}

impl<const N: usize> TrustedReadinessCorrelation<N> {
    pub const fn new() -> Self {
        Self {
            executions: Vec::new(),
        }
    }

    pub(crate) fn register_execution(
        &mut self,
        execution_id: ExecutionId,
        session_id: SessionId,
        target_id: TypedIdentifier,
        token: LaunchCorrelationToken,
        settings: bool,
    ) -> Result<(), ReadinessCorrelationError> {
        self.executions
            .push(RegisteredExecution {
                execution_id,
                session_id,
                target_id,
                token,
                last_sequence: 0,
                settings,
            })
            .map_err(|_| ReadinessCorrelationError::UnknownExecution)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ingest(
        &mut self,
        evidence_id: GuiReadinessEvidenceId,
        source: TrustedReadinessSource,
        source_identity: TypedIdentifier,
        execution_id: ExecutionId,
        correlation_token: LaunchCorrelationToken,
        session_id: SessionId,
        target_id: TypedIdentifier,
        source_generation: u64,
        sequence: u64,
        timestamp: u64,
        kind: TrustedGuiReadinessKind,
    ) -> Result<CorrelatedGuiReadinessEvidence, ReadinessCorrelationError> {
        let record = self
            .executions
            .iter_mut()
            .find(|entry| entry.execution_id == execution_id)
            .ok_or(ReadinessCorrelationError::UnknownExecution)?;
        if record.token != correlation_token {
            return Err(ReadinessCorrelationError::WrongCorrelation);
        }
        if record.session_id != session_id {
            return Err(ReadinessCorrelationError::WrongSession);
        }
        if record.target_id != target_id {
            return Err(ReadinessCorrelationError::WrongTarget);
        }
        if sequence <= record.last_sequence {
            return Err(ReadinessCorrelationError::InvalidSequence);
        }
        if !source_allows(source, record.settings, kind) {
            return Err(ReadinessCorrelationError::WrongSource);
        }
        record.last_sequence = sequence;
        Ok(CorrelatedGuiReadinessEvidence {
            evidence_id,
            source,
            source_identity,
            execution_id,
            session_id,
            target_id,
            source_generation,
            sequence,
            timestamp,
            kind,
            correlation_token,
        })
    }
}

impl<const N: usize> Default for TrustedReadinessCorrelation<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Deterministic no_std gate used by the QEMU verification feature.  The
/// source identities are typed fixtures only; it neither invokes the launcher
/// nor accepts diagnostic trace text as evidence.
#[cfg(any(
    feature = "gui-bridge-foundation-v1-test",
    feature = "gui-live-action-activation-v1-test"
))]
pub fn run_deterministic_bridge_gate() -> bool {
    let mut authority = GuiSessionBindingAuthority::new([0x47; 32]);
    let Some(verified) = VerifiedGraphicalSession::from_authority(
        1,
        SessionId(7),
        RequestedBy::User(9),
        "en",
        11,
        1,
        1,
        1,
        1,
        10,
        20,
    ) else {
        return false;
    };
    let Some(binding) = authority.issue(verified, 10, 20) else {
        return false;
    };
    if authority
        .verify(&binding, SessionId(7), RequestedBy::User(9), 11, 1, 1, 11)
        .is_err()
    {
        return false;
    }
    let payload =
        match PublicPresentationPayload::new("wiseowl.bridge.dispatch", "Opening Calculator…") {
            Some(value) => value,
            None => return false,
        };
    let update = CoordinatorPresentationUpdate {
        conversation_id: ConversationId(2),
        request_id: PlannerRequestId(3),
        action_id: Some(CoordinatorActionId(4)),
        session_id: SessionId(7),
        sequence: 1,
        kind: CoordinatorPresentationKind::DispatchAccepted,
        payload,
    };
    let mut broker = GuiEventBroker::<2>::new();
    let Ok(event_id) = broker.publish_presentation(update) else {
        return false;
    };
    let Some(awaiting_payload) =
        PublicPresentationPayload::new("wiseowl.bridge.awaiting", "Waiting for readiness…")
    else {
        return false;
    };
    if broker.next_for_session(SessionId(7), None).is_none()
        || broker.acknowledge(SessionId(7), event_id).is_err()
        || broker
            .publish_presentation(CoordinatorPresentationUpdate {
                conversation_id: ConversationId(2),
                request_id: PlannerRequestId(3),
                action_id: Some(CoordinatorActionId(4)),
                session_id: SessionId(7),
                sequence: 1,
                kind: CoordinatorPresentationKind::AwaitingOutcome,
                payload: awaiting_payload,
            })
            .is_ok()
    {
        return false;
    }
    let token = LaunchCorrelationToken::new_for_test([0x33; 32]);
    let calculator = match TypedIdentifier::new("calculator") {
        Ok(value) => value,
        Err(_) => return false,
    };
    let display = match TypedIdentifier::new("display-server") {
        Ok(value) => value,
        Err(_) => return false,
    };
    let mut correlation = TrustedReadinessCorrelation::<2>::new();
    if correlation
        .register_execution(
            ExecutionId(1),
            SessionId(7),
            calculator.clone(),
            token,
            false,
        )
        .is_err()
        || correlation
            .ingest(
                GuiReadinessEvidenceId(1),
                TrustedReadinessSource::DisplayServer,
                display,
                ExecutionId(1),
                token,
                SessionId(7),
                calculator,
                1,
                1,
                12,
                TrustedGuiReadinessKind::ApplicationReady,
            )
            .is_err()
    {
        return false;
    }
    let settings_token = LaunchCorrelationToken::new_for_test([0x44; 32]);
    let settings = match TypedIdentifier::new("display") {
        Ok(value) => value,
        Err(_) => return false,
    };
    let control_panel = match TypedIdentifier::new("control-panel") {
        Ok(value) => value,
        Err(_) => return false,
    };
    if correlation
        .register_execution(
            ExecutionId(2),
            SessionId(7),
            settings.clone(),
            settings_token,
            true,
        )
        .is_err()
        || correlation.ingest(
            GuiReadinessEvidenceId(2),
            TrustedReadinessSource::DisplayServer,
            control_panel.clone(),
            ExecutionId(2),
            settings_token,
            SessionId(7),
            settings.clone(),
            1,
            1,
            13,
            TrustedGuiReadinessKind::SettingsPageActivated,
        ) != Err(ReadinessCorrelationError::WrongSource)
        || correlation
            .ingest(
                GuiReadinessEvidenceId(3),
                TrustedReadinessSource::ControlPanel,
                control_panel,
                ExecutionId(2),
                settings_token,
                SessionId(7),
                settings,
                1,
                1,
                14,
                TrustedGuiReadinessKind::SettingsPageActivated,
            )
            .is_err()
    {
        return false;
    }
    let receipt = ReceiptSealedView {
        receipt_id: ActionReceiptId::new([0x55; 16]),
        action_id: CoordinatorActionId(4),
        terminal_status: ActionReceiptTerminalStatus::CompletedReady,
        operation_label_key: match String::try_from("operation.open_application") {
            Ok(value) => value,
            Err(_) => return false,
        },
        target_label_key: match String::try_from("target.calculator") {
            Ok(value) => value,
            Err(_) => return false,
        },
        readiness_observed: true,
    };
    if broker
        .publish_receipt(ConversationId(2), SessionId(7), 2, receipt)
        .is_err()
    {
        return false;
    }
    true
}

fn source_allows(
    source: TrustedReadinessSource,
    settings: bool,
    kind: TrustedGuiReadinessKind,
) -> bool {
    if settings {
        matches!(source, TrustedReadinessSource::ControlPanel)
            && matches!(
                kind,
                TrustedGuiReadinessKind::SettingsPageActivated
                    | TrustedGuiReadinessKind::SettingsPageReady
                    | TrustedGuiReadinessKind::ProcessExitedEarly
            )
    } else {
        match source {
            TrustedReadinessSource::DisplayServer => matches!(
                kind,
                TrustedGuiReadinessKind::FirstWindowRegistered
                    | TrustedGuiReadinessKind::ApplicationReady
            ),
            TrustedReadinessSource::ApplicationRegistry => matches!(
                kind,
                TrustedGuiReadinessKind::ApplicationRegistered
                    | TrustedGuiReadinessKind::ApplicationReady
            ),
            TrustedReadinessSource::ProcessLifecycle => {
                kind == TrustedGuiReadinessKind::ProcessExitedEarly
            }
            TrustedReadinessSource::ControlPanel => false,
        }
    }
}

fn hash_requester(hasher: &mut Sha256, requester: RequestedBy) {
    match requester {
        RequestedBy::User(id) => {
            hasher.update([1]);
            hasher.update(id.to_le_bytes());
        }
        RequestedBy::WiseOwlReasoning => hasher.update([2]),
        RequestedBy::SystemComponent(id) => {
            hasher.update([3]);
            hasher.update(id.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> VerifiedGraphicalSession {
        VerifiedGraphicalSession::for_test(SessionId(7), RequestedBy::User(9), 11)
    }
    fn binding() -> WiseOwlGuiSessionBinding {
        GuiSessionBindingAuthority::new([7; 32])
            .issue(session(), 10, 20)
            .unwrap()
    }
    fn update(sequence: u64, kind: CoordinatorPresentationKind) -> CoordinatorPresentationUpdate {
        CoordinatorPresentationUpdate {
            conversation_id: ConversationId(2),
            request_id: PlannerRequestId(3),
            action_id: Some(CoordinatorActionId(4)),
            session_id: SessionId(7),
            sequence,
            kind,
            payload: PublicPresentationPayload::new("wiseowl.bridge.test", "safe public text")
                .unwrap(),
        }
    }

    #[test]
    fn session_binding_rejects_wrong_user_expiry_and_stale_runtime() {
        let authority = GuiSessionBindingAuthority::new([7; 32]);
        let issued = binding();
        assert!(authority
            .verify(&issued, SessionId(7), RequestedBy::User(9), 11, 1, 1, 19)
            .is_ok());
        assert_eq!(
            authority.verify(&issued, SessionId(7), RequestedBy::User(8), 11, 1, 1, 19),
            Err(GuiBindingError::WrongRequester)
        );
        assert_eq!(
            authority.verify(&issued, SessionId(7), RequestedBy::User(9), 12, 1, 1, 19),
            Err(GuiBindingError::StaleRuntime)
        );
        assert_eq!(
            authority.verify(&issued, SessionId(7), RequestedBy::User(9), 11, 1, 1, 21),
            Err(GuiBindingError::Expired)
        );
    }

    #[test]
    fn presentation_and_delivery_are_bounded_session_ordered_and_acknowledged() {
        let mut broker: GuiEventBroker<2> = GuiEventBroker::new();
        let first = broker
            .publish_presentation(update(1, CoordinatorPresentationKind::DispatchAccepted))
            .unwrap();
        assert_eq!(
            broker.publish_presentation(update(1, CoordinatorPresentationKind::AwaitingOutcome)),
            Err(GuiEventError::InvalidSequence)
        );
        assert_eq!(broker.next_for_session(SessionId(8), None), None);
        assert_eq!(
            broker.acknowledge(SessionId(8), first),
            Err(GuiEventError::WrongSession)
        );
        broker.acknowledge(SessionId(7), first).unwrap();
        broker
            .publish_receipt(
                ConversationId(2),
                SessionId(7),
                2,
                ReceiptSealedView {
                    receipt_id: ActionReceiptId::new([1; 16]),
                    action_id: CoordinatorActionId(4),
                    terminal_status: ActionReceiptTerminalStatus::CompletedReady,
                    operation_label_key: String::try_from("operation.open_application").unwrap(),
                    target_label_key: String::try_from("target.calculator").unwrap(),
                    readiness_observed: true,
                },
            )
            .unwrap();
        assert_eq!(broker.len(), 1);
    }

    #[test]
    fn terminal_presentation_is_idempotent_but_nonterminal_replay_is_rejected() {
        let mut broker: GuiEventBroker<2> = GuiEventBroker::new();
        let terminal = update(1, CoordinatorPresentationKind::OutcomeReady);
        let first = broker.publish_presentation(terminal.clone()).unwrap();
        assert_eq!(broker.publish_presentation(terminal), Ok(first));
        assert_eq!(broker.len(), 1);
        assert_eq!(
            broker.publish_presentation(update(1, CoordinatorPresentationKind::AwaitingOutcome)),
            Err(GuiEventError::InvalidSequence)
        );
    }

    #[test]
    fn readiness_requires_registered_exact_execution_correlation_and_source() {
        let token = LaunchCorrelationToken::new_for_test([4; 32]);
        let mut correlation = TrustedReadinessCorrelation::<2>::new();
        correlation
            .register_execution(
                ExecutionId(1),
                SessionId(7),
                TypedIdentifier::new("calculator").unwrap(),
                token,
                false,
            )
            .unwrap();
        let evidence = correlation
            .ingest(
                GuiReadinessEvidenceId(1),
                TrustedReadinessSource::DisplayServer,
                TypedIdentifier::new("display-server").unwrap(),
                ExecutionId(1),
                token,
                SessionId(7),
                TypedIdentifier::new("calculator").unwrap(),
                1,
                1,
                10,
                TrustedGuiReadinessKind::FirstWindowRegistered,
            )
            .unwrap();
        assert_eq!(
            evidence.kind,
            TrustedGuiReadinessKind::FirstWindowRegistered
        );
        assert_eq!(
            correlation.ingest(
                GuiReadinessEvidenceId(2),
                TrustedReadinessSource::DisplayServer,
                TypedIdentifier::new("display-server").unwrap(),
                ExecutionId(1),
                token,
                SessionId(7),
                TypedIdentifier::new("calculator").unwrap(),
                1,
                1,
                11,
                TrustedGuiReadinessKind::ApplicationReady
            ),
            Err(ReadinessCorrelationError::InvalidSequence)
        );
        assert_eq!(
            correlation.ingest(
                GuiReadinessEvidenceId(3),
                TrustedReadinessSource::ControlPanel,
                TypedIdentifier::new("control-panel").unwrap(),
                ExecutionId(1),
                token,
                SessionId(7),
                TypedIdentifier::new("calculator").unwrap(),
                1,
                2,
                12,
                TrustedGuiReadinessKind::ApplicationReady
            ),
            Err(ReadinessCorrelationError::WrongSource)
        );
    }

    #[test]
    fn settings_require_exact_page_and_control_panel_source() {
        let token = LaunchCorrelationToken::new_for_test([5; 32]);
        let mut correlation = TrustedReadinessCorrelation::<1>::new();
        correlation
            .register_execution(
                ExecutionId(2),
                SessionId(7),
                TypedIdentifier::new("display").unwrap(),
                token,
                true,
            )
            .unwrap();
        assert_eq!(
            correlation.ingest(
                GuiReadinessEvidenceId(1),
                TrustedReadinessSource::ControlPanel,
                TypedIdentifier::new("control-panel").unwrap(),
                ExecutionId(2),
                token,
                SessionId(7),
                TypedIdentifier::new("network").unwrap(),
                1,
                1,
                10,
                TrustedGuiReadinessKind::SettingsPageActivated
            ),
            Err(ReadinessCorrelationError::WrongTarget)
        );
        assert!(correlation
            .ingest(
                GuiReadinessEvidenceId(2),
                TrustedReadinessSource::ControlPanel,
                TypedIdentifier::new("control-panel").unwrap(),
                ExecutionId(2),
                token,
                SessionId(7),
                TypedIdentifier::new("display").unwrap(),
                1,
                1,
                10,
                TrustedGuiReadinessKind::SettingsPageActivated
            )
            .is_ok());
    }
}
