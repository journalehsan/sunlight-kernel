//! Immutable, redacted Action Receipt Ledger v1.
//!
//! This module is downstream-only. It consumes typed lifecycle facts and has
//! no policy, confirmation, execution, retry, observation, parser, filesystem,
//! or arbitrary IPC capability.

use alloc::vec::Vec as AllocVec;
use heapless::{Deque, String, Vec};
use sha2::{Digest, Sha256};

use crate::action_intent::{
    ActionDecision, ActionOperation, AuditId, IntentId, RequestedBy, SessionId, TargetKind,
};
use crate::coordinator::{ActionConversationRecord, CoordinatorActionId, PublicReasonCode};
use crate::executor::{ExecutionId, ExecutionResult, ExecutionResultCode};
use crate::outcome::{ObservationId, ObservedActionOutcome, ObservedActionOutcomeKind};
use crate::planner::{ConversationId, PlannerRequestId, PlannerVersion};
use crate::policy::{PolicyResult, PolicyVersion};

pub const ACTION_RECEIPT_SCHEMA_VERSION: u16 = 1;
pub const ACTION_RECEIPT_DIGEST_DOMAIN: &[u8] = b"wiseowl.action-receipt.v1\0";
pub const MAX_TARGET_DISPLAY_KEY_LEN: usize = 64;
pub const MAX_RECEIPT_TIMELINE_EVENTS: usize = 24;
pub const MAX_RECEIPT_AUDIT_REFERENCES: usize = 8;
pub const MAX_RECEIPT_QUERY_RESULTS: usize = 8;
pub const MAX_RECEIPT_VIEW_TEXT: usize = 192;
pub const MAX_RECEIPT_VIEW_STEPS: usize = 24;
pub const DEFAULT_ACTIVE_RECEIPTS: usize = 8;
pub const DEFAULT_SEALED_RECEIPTS: usize = 32;
pub const DEFAULT_RECEIPT_AUDIT_CAPACITY: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActionReceiptId([u8; 16]);

impl ActionReceiptId {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }

    pub fn for_action(
        action_id: CoordinatorActionId,
        conversation_id: ConversationId,
        session_id: SessionId,
        requester: RequestedBy,
        request_id: PlannerRequestId,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"wiseowl.action-receipt-id.v1\0");
        hasher.update(action_id.0.to_le_bytes());
        hasher.update(conversation_id.0.to_le_bytes());
        hasher.update(session_id.0.to_le_bytes());
        hash_requester(&mut hasher, requester);
        hasher.update(request_id.0.to_le_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let mut id = [0u8; 16];
        id.copy_from_slice(&digest[..16]);
        Self(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDisplayKey(String<MAX_TARGET_DISPLAY_KEY_LEN>);

impl TargetDisplayKey {
    pub fn new(value: &str) -> Result<Self, ReceiptError> {
        if value.is_empty() || value.len() > MAX_TARGET_DISPLAY_KEY_LEN {
            return Err(ReceiptError::InvalidTargetDisplayKey);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(ReceiptError::InvalidTargetDisplayKey);
        }
        String::try_from(value)
            .map(Self)
            .map_err(|_| ReceiptError::InvalidTargetDisplayKey)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptEventSource {
    Planner,
    Policy,
    ConfirmationAuthority,
    Coordinator,
    Executor,
    OutcomeObserver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptLifecycleEventType {
    RequestAccepted,
    NoAction,
    Unsupported,
    ClarificationRequested,
    ClarificationResolved,
    ClarificationExpired,
    PolicyAllowed,
    PolicyDenied,
    ConfirmationRequested,
    ConfirmationApproved,
    ConfirmationRejected,
    ConfirmationExpired,
    Cancelled,
    ReadyForExecutionProduced,
    DispatchAccepted,
    DispatchFailed,
    AwaitingOutcome,
    TargetReady,
    TargetExitedEarly,
    OutcomeTimedOut,
    SessionInvalidated,
    RegistryInvalidated,
    InvalidInput,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionReceiptTerminalStatus {
    CompletedReady,
    DispatchAcceptedOnly,
    Denied,
    Unsupported,
    Cancelled,
    ClarificationExpired,
    ConfirmationRejected,
    ConfirmationExpired,
    DispatchFailed,
    ExitedEarly,
    OutcomeTimedOut,
    SessionInvalidated,
    RegistryInvalidated,
    InvalidInput,
    Interrupted,
    Unknown,
}

impl ActionReceiptTerminalStatus {
    pub const fn is_success(self) -> bool {
        matches!(self, Self::CompletedReady | Self::DispatchAcceptedOnly)
    }

    pub const fn is_executed(self) -> bool {
        matches!(
            self,
            Self::CompletedReady
                | Self::DispatchAcceptedOnly
                | Self::ExitedEarly
                | Self::OutcomeTimedOut
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReceiptRelevantIds {
    pub intent_id: Option<IntentId>,
    pub execution_id: Option<ExecutionId>,
    pub observation_id: Option<ObservationId>,
    pub audit_id: Option<AuditId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionReceiptLifecycleEvent {
    receipt_id: ActionReceiptId,
    coordinator_action_id: CoordinatorActionId,
    session_id: SessionId,
    requester: RequestedBy,
    sequence: u16,
    timestamp: u64,
    event_type: ReceiptLifecycleEventType,
    public_reason: PublicReasonCode,
    ids: ReceiptRelevantIds,
    source: ReceiptEventSource,
    terminal_status: Option<ActionReceiptTerminalStatus>,
}

impl ActionReceiptLifecycleEvent {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        receipt_id: ActionReceiptId,
        coordinator_action_id: CoordinatorActionId,
        session_id: SessionId,
        requester: RequestedBy,
        sequence: u16,
        timestamp: u64,
        event_type: ReceiptLifecycleEventType,
        public_reason: PublicReasonCode,
        ids: ReceiptRelevantIds,
        source: ReceiptEventSource,
        terminal_status: Option<ActionReceiptTerminalStatus>,
    ) -> Self {
        Self {
            receipt_id,
            coordinator_action_id,
            session_id,
            requester,
            sequence,
            timestamp,
            event_type,
            public_reason,
            ids,
            source,
            terminal_status,
        }
    }

    pub const fn receipt_id(&self) -> ActionReceiptId {
        self.receipt_id
    }
    pub const fn coordinator_action_id(&self) -> CoordinatorActionId {
        self.coordinator_action_id
    }
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub const fn requester(&self) -> RequestedBy {
        self.requester
    }
    pub const fn sequence(&self) -> u16 {
        self.sequence
    }
    pub const fn timestamp(&self) -> u64 {
        self.timestamp
    }
    pub const fn event_type(&self) -> ReceiptLifecycleEventType {
        self.event_type
    }
    pub const fn public_reason(&self) -> PublicReasonCode {
        self.public_reason
    }
    pub const fn ids(&self) -> ReceiptRelevantIds {
        self.ids
    }
    pub const fn source(&self) -> ReceiptEventSource {
        self.source
    }
    pub const fn terminal_status(&self) -> Option<ActionReceiptTerminalStatus> {
        self.terminal_status
    }

    pub fn from_policy_decision(
        open: &ReceiptOpen,
        sequence: u16,
        timestamp: u64,
        decision: &ActionDecision,
    ) -> Result<Self, ReceiptError> {
        if open.intent_id != Some(decision.intent_id())
            || open.policy_version != decision.policy_version()
            || open.runtime_snapshot_generation != decision.runtime_snapshot_generation()
        {
            return Err(ReceiptError::WrongAction);
        }
        let (event_type, reason, terminal_status) = match decision.result() {
            PolicyResult::Allowed => (
                ReceiptLifecycleEventType::PolicyAllowed,
                PublicReasonCode::None,
                None,
            ),
            PolicyResult::ConfirmationRequired => (
                ReceiptLifecycleEventType::PolicyAllowed,
                PublicReasonCode::ConfirmationNeeded,
                None,
            ),
            PolicyResult::Denied | PolicyResult::Unknown => (
                ReceiptLifecycleEventType::PolicyDenied,
                PublicReasonCode::PolicyDenied,
                Some(ActionReceiptTerminalStatus::Denied),
            ),
        };
        Ok(Self::new(
            open.receipt_id,
            open.coordinator_action_id,
            open.session_id,
            open.requester,
            sequence,
            timestamp,
            event_type,
            reason,
            ReceiptRelevantIds {
                intent_id: Some(decision.intent_id()),
                execution_id: None,
                observation_id: None,
                audit_id: Some(decision.audit_id()),
            },
            ReceiptEventSource::Policy,
            terminal_status,
        ))
    }

    pub fn from_execution_result(
        open: &ReceiptOpen,
        sequence: u16,
        execution: &ExecutionResult,
    ) -> Result<Self, ReceiptError> {
        if open.intent_id != Some(execution.intent_id())
            || open.session_id != execution.session_id()
            || open.requester != execution.requester()
        {
            return Err(ReceiptError::WrongAction);
        }
        let timestamp = execution
            .dispatch_timestamp()
            .or(execution.completion_timestamp())
            .map(|value| value.0)
            .unwrap_or(open.request_timestamp);
        let accepted = execution.code() == ExecutionResultCode::Succeeded;
        Ok(Self::new(
            open.receipt_id,
            open.coordinator_action_id,
            open.session_id,
            open.requester,
            sequence,
            timestamp,
            if accepted {
                ReceiptLifecycleEventType::DispatchAccepted
            } else {
                ReceiptLifecycleEventType::DispatchFailed
            },
            execution_reason(execution.code()),
            ReceiptRelevantIds {
                intent_id: Some(execution.intent_id()),
                execution_id: Some(execution.execution_id()),
                observation_id: None,
                audit_id: Some(execution.audit_id()),
            },
            ReceiptEventSource::Executor,
            if accepted {
                None
            } else {
                Some(ActionReceiptTerminalStatus::DispatchFailed)
            },
        ))
    }

    pub fn from_observed_outcome(
        open: &ReceiptOpen,
        sequence: u16,
        outcome: &ObservedActionOutcome,
    ) -> Result<Self, ReceiptError> {
        if open.intent_id != Some(outcome.intent_id()) {
            return Err(ReceiptError::WrongAction);
        }
        let (event_type, reason, terminal_status) = match outcome.kind() {
            ObservedActionOutcomeKind::Ready => (
                ReceiptLifecycleEventType::TargetReady,
                PublicReasonCode::OutcomeReady,
                None,
            ),
            ObservedActionOutcomeKind::DispatchOnly => (
                ReceiptLifecycleEventType::AwaitingOutcome,
                PublicReasonCode::DispatchAccepted,
                None,
            ),
            ObservedActionOutcomeKind::ExitedEarly | ObservedActionOutcomeKind::Failed => (
                ReceiptLifecycleEventType::TargetExitedEarly,
                PublicReasonCode::ExitedBeforeReady,
                Some(ActionReceiptTerminalStatus::ExitedEarly),
            ),
            ObservedActionOutcomeKind::TimedOut => (
                ReceiptLifecycleEventType::OutcomeTimedOut,
                PublicReasonCode::OutcomeTimedOut,
                Some(ActionReceiptTerminalStatus::OutcomeTimedOut),
            ),
            ObservedActionOutcomeKind::SessionInvalidated => (
                ReceiptLifecycleEventType::SessionInvalidated,
                PublicReasonCode::SessionEnded,
                Some(ActionReceiptTerminalStatus::SessionInvalidated),
            ),
            ObservedActionOutcomeKind::RegistryInvalidated => (
                ReceiptLifecycleEventType::RegistryInvalidated,
                PublicReasonCode::ApplicationRegistryChanged,
                Some(ActionReceiptTerminalStatus::RegistryInvalidated),
            ),
            ObservedActionOutcomeKind::Cancelled => (
                ReceiptLifecycleEventType::Cancelled,
                PublicReasonCode::ObservationCancelled,
                Some(ActionReceiptTerminalStatus::Cancelled),
            ),
            ObservedActionOutcomeKind::UnsupportedObservation
            | ObservedActionOutcomeKind::Unknown => (
                ReceiptLifecycleEventType::Completed,
                PublicReasonCode::Unknown,
                Some(ActionReceiptTerminalStatus::Unknown),
            ),
        };
        Ok(Self::new(
            open.receipt_id,
            open.coordinator_action_id,
            open.session_id,
            open.requester,
            sequence,
            outcome
                .terminal_timestamp()
                .map(|value| value.0)
                .unwrap_or(open.request_timestamp),
            event_type,
            reason,
            ReceiptRelevantIds {
                intent_id: Some(outcome.intent_id()),
                execution_id: Some(outcome.execution_id()),
                observation_id: Some(outcome.observation_id()),
                audit_id: Some(outcome.audit_id()),
            },
            ReceiptEventSource::OutcomeObserver,
            terminal_status,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyReceiptSummary {
    pub result: PolicyResult,
    pub public_reason: PublicReasonCode,
    pub audit_id: Option<AuditId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationReceiptOutcome {
    NotRequired,
    Requested,
    Approved,
    Rejected,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmationReceiptSummary {
    pub outcome: ConfirmationReceiptOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchReceiptStatus {
    NotDispatched,
    ReadyProduced,
    Accepted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchReceiptSummary {
    pub status: DispatchReceiptStatus,
    pub execution_id: Option<ExecutionId>,
    pub audit_id: Option<AuditId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedReceiptOutcome {
    NotObserved,
    Awaiting,
    Ready,
    ExitedEarly,
    TimedOut,
    SessionInvalidated,
    RegistryInvalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedOutcomeReceiptSummary {
    pub outcome: ObservedReceiptOutcome,
    pub observation_id: Option<ObservationId>,
    pub audit_id: Option<AuditId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptOpen {
    pub receipt_id: ActionReceiptId,
    pub coordinator_action_id: CoordinatorActionId,
    pub conversation_id: ConversationId,
    pub session_id: SessionId,
    pub requester: RequestedBy,
    pub original_request_id: PlannerRequestId,
    pub intent_id: Option<IntentId>,
    pub operation: Option<ActionOperation>,
    pub target_kind: Option<TargetKind>,
    pub target_display_key: Option<TargetDisplayKey>,
    pub request_timestamp: u64,
    pub policy_version: PolicyVersion,
    pub planner_version: PlannerVersion,
    pub runtime_snapshot_generation: u64,
    pub application_registry_generation: u64,
    pub settings_registry_generation: u64,
    pub bounded_audit_references: Vec<AuditId, MAX_RECEIPT_AUDIT_REFERENCES>,
}

impl ReceiptOpen {
    pub fn from_coordinator_record(
        record: &ActionConversationRecord,
        operation: Option<ActionOperation>,
        target_kind: Option<TargetKind>,
        target_display_key: Option<TargetDisplayKey>,
    ) -> Self {
        let receipt_id = ActionReceiptId::for_action(
            record.coordinator_action_id(),
            record.conversation_id(),
            record.session_id(),
            record.requester(),
            record.original_request_id(),
        );
        let mut bounded_audit_references = Vec::new();
        if record.bounded_audit_reference() != 0 {
            let _ = bounded_audit_references.push(AuditId(record.bounded_audit_reference()));
        }
        Self {
            receipt_id,
            coordinator_action_id: record.coordinator_action_id(),
            conversation_id: record.conversation_id(),
            session_id: record.session_id(),
            requester: record.requester(),
            original_request_id: record.original_request_id(),
            intent_id: record.intent_id(),
            operation,
            target_kind,
            target_display_key,
            request_timestamp: record.creation_time(),
            policy_version: record.policy_version(),
            planner_version: record.planner_version(),
            runtime_snapshot_generation: record.runtime_snapshot_generation(),
            application_registry_generation: record.application_registry_generation(),
            settings_registry_generation: record.settings_registry_generation(),
            bounded_audit_references,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReceiptBuilder {
    open: ReceiptOpen,
    timeline: Vec<ActionReceiptLifecycleEvent, MAX_RECEIPT_TIMELINE_EVENTS>,
    policy: Option<PolicyReceiptSummary>,
    confirmation: ConfirmationReceiptSummary,
    dispatch: DispatchReceiptSummary,
    observed_outcome: ObservedOutcomeReceiptSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionReceipt {
    receipt_id: ActionReceiptId,
    coordinator_action_id: CoordinatorActionId,
    conversation_id: ConversationId,
    session_id: SessionId,
    requester: RequestedBy,
    original_request_id: PlannerRequestId,
    intent_id: Option<IntentId>,
    execution_id: Option<ExecutionId>,
    observation_id: Option<ObservationId>,
    operation: Option<ActionOperation>,
    target_kind: Option<TargetKind>,
    target_display_key: Option<TargetDisplayKey>,
    request_timestamp: u64,
    terminal_timestamp: u64,
    terminal_status: ActionReceiptTerminalStatus,
    policy: Option<PolicyReceiptSummary>,
    confirmation: ConfirmationReceiptSummary,
    dispatch: DispatchReceiptSummary,
    observed_outcome: ObservedOutcomeReceiptSummary,
    policy_version: PolicyVersion,
    planner_version: PlannerVersion,
    runtime_snapshot_generation: u64,
    application_registry_generation: u64,
    settings_registry_generation: u64,
    bounded_audit_references: Vec<AuditId, MAX_RECEIPT_AUDIT_REFERENCES>,
    timeline: Vec<ActionReceiptLifecycleEvent, MAX_RECEIPT_TIMELINE_EVENTS>,
    integrity_digest: [u8; 32],
    schema_version: u16,
}

impl ActionReceipt {
    pub const fn receipt_id(&self) -> ActionReceiptId {
        self.receipt_id
    }
    pub const fn coordinator_action_id(&self) -> CoordinatorActionId {
        self.coordinator_action_id
    }
    pub const fn conversation_id(&self) -> ConversationId {
        self.conversation_id
    }
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub const fn requester(&self) -> RequestedBy {
        self.requester
    }
    pub const fn original_request_id(&self) -> PlannerRequestId {
        self.original_request_id
    }
    pub const fn intent_id(&self) -> Option<IntentId> {
        self.intent_id
    }
    pub const fn execution_id(&self) -> Option<ExecutionId> {
        self.execution_id
    }
    pub const fn observation_id(&self) -> Option<ObservationId> {
        self.observation_id
    }
    pub const fn operation(&self) -> Option<ActionOperation> {
        self.operation
    }
    pub const fn target_kind(&self) -> Option<TargetKind> {
        self.target_kind
    }
    pub fn target_display_key(&self) -> Option<&TargetDisplayKey> {
        self.target_display_key.as_ref()
    }
    pub const fn request_timestamp(&self) -> u64 {
        self.request_timestamp
    }
    pub const fn terminal_timestamp(&self) -> u64 {
        self.terminal_timestamp
    }
    pub const fn terminal_status(&self) -> ActionReceiptTerminalStatus {
        self.terminal_status
    }
    pub const fn policy_summary(&self) -> Option<PolicyReceiptSummary> {
        self.policy
    }
    pub const fn confirmation_summary(&self) -> ConfirmationReceiptSummary {
        self.confirmation
    }
    pub const fn dispatch_summary(&self) -> DispatchReceiptSummary {
        self.dispatch
    }
    pub const fn observed_outcome_summary(&self) -> ObservedOutcomeReceiptSummary {
        self.observed_outcome
    }
    pub const fn integrity_digest(&self) -> [u8; 32] {
        self.integrity_digest
    }
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }
    pub fn timeline(&self) -> &[ActionReceiptLifecycleEvent] {
        self.timeline.as_slice()
    }

    pub fn verify_integrity(&self) -> bool {
        self.schema_version == ACTION_RECEIPT_SCHEMA_VERSION
            && self.integrity_digest == compute_integrity_digest(self)
            && self.timeline.last().and_then(|event| event.terminal_status)
                == Some(self.terminal_status)
    }

    pub fn encode_sealed(&self) -> Result<AllocVec<u8>, ReceiptError> {
        if !self.verify_integrity() {
            return Err(ReceiptError::IntegrityFailure);
        }
        let mut bytes = AllocVec::with_capacity(512);
        bytes.extend_from_slice(b"WORC");
        bytes.extend_from_slice(&self.schema_version.to_le_bytes());
        bytes.extend_from_slice(&self.receipt_id.0);
        bytes.extend_from_slice(&self.integrity_digest);
        bytes.extend_from_slice(&self.terminal_timestamp.to_le_bytes());
        bytes.extend_from_slice(&(self.timeline.len() as u16).to_le_bytes());
        for event in self.timeline.iter() {
            encode_event(&mut bytes, event);
        }
        if bytes.len() > wiseowl_memorydb::action_receipts::MAX_SEALED_RECEIPT_BYTES {
            return Err(ReceiptError::ReceiptTooLarge);
        }
        Ok(bytes)
    }

    #[cfg(test)]
    fn corrupt_digest_for_test(&mut self) {
        self.integrity_digest[0] ^= 0xff;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptRetentionPolicy {
    pub max_sealed_per_domain: usize,
    pub max_nonexecuted_per_domain: usize,
}

impl ReceiptRetentionPolicy {
    pub const V1: Self = Self {
        max_sealed_per_domain: 16,
        max_nonexecuted_per_domain: 4,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptQueryKind {
    Latest,
    LatestCompleted,
    ByReceiptId(ActionReceiptId),
    RecentLimited,
    PendingCurrentAction,
    LastFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptQuery {
    pub requester: RequestedBy,
    pub active_session: SessionId,
    pub maximum_results: u8,
    pub kind: ReceiptQueryKind,
}

impl ReceiptQuery {
    pub fn validate(self) -> Result<Self, ReceiptError> {
        if self.maximum_results == 0 || self.maximum_results as usize > MAX_RECEIPT_QUERY_RESULTS {
            return Err(ReceiptError::UnboundedQuery);
        }
        if !matches!(self.kind, ReceiptQueryKind::RecentLimited) && self.maximum_results != 1 {
            return Err(ReceiptError::UnboundedQuery);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingReceiptState {
    AwaitingClarification,
    AwaitingConfirmation,
    Dispatching,
    AwaitingOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingActionReceiptView {
    pub receipt_id: ActionReceiptId,
    pub state: PendingReceiptState,
    pub operation_label: String<MAX_RECEIPT_VIEW_TEXT>,
    pub target_label: String<MAX_RECEIPT_VIEW_TEXT>,
    pub start_time: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionReceiptView {
    pub receipt_id: ActionReceiptId,
    pub localized_title: String<MAX_RECEIPT_VIEW_TEXT>,
    pub operation_display_label: String<MAX_RECEIPT_VIEW_TEXT>,
    pub target_display_label: String<MAX_RECEIPT_VIEW_TEXT>,
    pub terminal_status_label: String<MAX_RECEIPT_VIEW_TEXT>,
    pub start_time: u64,
    pub completion_time: u64,
    pub lifecycle_steps: Vec<String<MAX_RECEIPT_VIEW_TEXT>, MAX_RECEIPT_VIEW_STEPS>,
    pub public_reason: PublicReasonCode,
    pub confirmation_occurred: bool,
    pub readiness_observed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptQueryResult {
    Sealed(Vec<ActionReceiptView, MAX_RECEIPT_QUERY_RESULTS>),
    Pending(PendingActionReceiptView),
    NotFound,
    ReceiptUnavailable,
    ReceiptIntegrityFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptConversationQuestion {
    WhatDidYouDo,
    DidItOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptReadinessAnswer {
    Ready,
    NotReady,
    Pending,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptConversationAnswer {
    pub view: Option<ActionReceiptView>,
    pub readiness: ReceiptReadinessAnswer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptAuditEvent {
    ReceiptOpened,
    LifecycleEventAppended,
    DuplicateIgnored,
    MismatchedEventRejected,
    ReceiptSealed,
    IntegrityVerified,
    IntegrityFailure,
    ReceiptEvicted,
    QueryAllowed,
    QueryDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptAuditEntry {
    pub receipt_id: Option<ActionReceiptId>,
    pub coordinator_action_id: Option<CoordinatorActionId>,
    pub session_id: SessionId,
    pub event: ReceiptAuditEvent,
    pub public_reason: PublicReasonCode,
    pub timestamp: u64,
}

pub struct ReceiptAuditLog<const N: usize = DEFAULT_RECEIPT_AUDIT_CAPACITY> {
    entries: Deque<ReceiptAuditEntry, N>,
    evicted: u64,
}

impl<const N: usize> ReceiptAuditLog<N> {
    pub const fn new() -> Self {
        Self {
            entries: Deque::new(),
            evicted: 0,
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = &ReceiptAuditEntry> {
        self.entries.iter()
    }

    pub const fn evicted(&self) -> u64 {
        self.evicted
    }

    fn push(&mut self, entry: ReceiptAuditEntry) {
        if N == 0 {
            self.evicted = self.evicted.saturating_add(1);
            return;
        }
        if self.entries.is_full() {
            let _ = self.entries.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
        let _ = self.entries.push_back(entry);
    }
}

impl<const N: usize> Default for ReceiptAuditLog<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendDisposition {
    Appended,
    DuplicateIgnored,
    Sealed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptPersistenceError {
    Unavailable,
    Corrupt,
    Capacity,
}

pub trait ReceiptPersistence {
    fn append_fragment(
        &mut self,
        owner_domain: u64,
        session_id: SessionId,
        receipt_id: ActionReceiptId,
        fragment: &[u8],
    ) -> Result<(), ReceiptPersistenceError>;

    fn seal(
        &mut self,
        owner_domain: u64,
        session_id: SessionId,
        receipt_id: ActionReceiptId,
        receipt: &[u8],
    ) -> Result<(), ReceiptPersistenceError>;

    fn evict(
        &mut self,
        owner_domain: u64,
        session_id: SessionId,
        receipt_id: ActionReceiptId,
    ) -> Result<(), ReceiptPersistenceError>;
}

#[derive(Default)]
pub struct VolatileReceiptPersistence;

impl ReceiptPersistence for VolatileReceiptPersistence {
    fn append_fragment(
        &mut self,
        _owner_domain: u64,
        _session_id: SessionId,
        _receipt_id: ActionReceiptId,
        _fragment: &[u8],
    ) -> Result<(), ReceiptPersistenceError> {
        Ok(())
    }

    fn seal(
        &mut self,
        _owner_domain: u64,
        _session_id: SessionId,
        _receipt_id: ActionReceiptId,
        _receipt: &[u8],
    ) -> Result<(), ReceiptPersistenceError> {
        Ok(())
    }

    fn evict(
        &mut self,
        _owner_domain: u64,
        _session_id: SessionId,
        _receipt_id: ActionReceiptId,
    ) -> Result<(), ReceiptPersistenceError> {
        Ok(())
    }
}

impl<S: wiseowl_memorydb::DurableStore> ReceiptPersistence
    for wiseowl_memorydb::ActionReceiptBlobStore<S>
{
    fn append_fragment(
        &mut self,
        owner_domain: u64,
        session_id: SessionId,
        receipt_id: ActionReceiptId,
        fragment: &[u8],
    ) -> Result<(), ReceiptPersistenceError> {
        wiseowl_memorydb::ActionReceiptBlobStore::append_fragment(
            self,
            wiseowl_memorydb::ReceiptBlobKey::new(owner_domain, session_id.0, receipt_id.bytes()),
            fragment,
        )
        .map_err(map_persistence_error)
    }

    fn seal(
        &mut self,
        owner_domain: u64,
        session_id: SessionId,
        receipt_id: ActionReceiptId,
        receipt: &[u8],
    ) -> Result<(), ReceiptPersistenceError> {
        wiseowl_memorydb::ActionReceiptBlobStore::seal(
            self,
            wiseowl_memorydb::ReceiptBlobKey::new(owner_domain, session_id.0, receipt_id.bytes()),
            receipt,
        )
        .map_err(map_persistence_error)
    }

    fn evict(
        &mut self,
        owner_domain: u64,
        session_id: SessionId,
        receipt_id: ActionReceiptId,
    ) -> Result<(), ReceiptPersistenceError> {
        wiseowl_memorydb::ActionReceiptBlobStore::evict_sealed(
            self,
            wiseowl_memorydb::ReceiptBlobKey::new(owner_domain, session_id.0, receipt_id.bytes()),
        )
        .map_err(map_persistence_error)
    }
}

fn map_persistence_error(error: wiseowl_memorydb::DbError) -> ReceiptPersistenceError {
    match error {
        wiseowl_memorydb::DbError::Corrupt { .. } | wiseowl_memorydb::DbError::WalIncomplete => {
            ReceiptPersistenceError::Corrupt
        }
        wiseowl_memorydb::DbError::PayloadTooLarge { .. }
        | wiseowl_memorydb::DbError::QuotaExceeded(_) => ReceiptPersistenceError::Capacity,
        _ => ReceiptPersistenceError::Unavailable,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptError {
    DuplicateReceipt,
    ActiveCapacity,
    ReceiptNotFound,
    AlreadySealed,
    WrongAction,
    WrongSession,
    WrongRequester,
    UnknownSource,
    UnsupportedEvent,
    OutOfOrder,
    DuplicateSequenceConflict,
    DuplicateTerminal,
    TimelineFull,
    InvalidTerminalStatus,
    InvalidTargetDisplayKey,
    UnboundedQuery,
    IntegrityFailure,
    ReceiptTooLarge,
    PersistenceFailure,
}

pub struct ActionReceiptLedger<
    P: ReceiptPersistence = VolatileReceiptPersistence,
    const ACTIVE: usize = DEFAULT_ACTIVE_RECEIPTS,
    const SEALED: usize = DEFAULT_SEALED_RECEIPTS,
    const AUDIT: usize = DEFAULT_RECEIPT_AUDIT_CAPACITY,
> {
    persistence: P,
    active: Vec<ReceiptBuilder, ACTIVE>,
    sealed: Vec<ActionReceipt, SEALED>,
    retention: ReceiptRetentionPolicy,
    audit: ReceiptAuditLog<AUDIT>,
}

impl<P: ReceiptPersistence, const ACTIVE: usize, const SEALED: usize, const AUDIT: usize>
    ActionReceiptLedger<P, ACTIVE, SEALED, AUDIT>
{
    pub fn new(persistence: P, retention: ReceiptRetentionPolicy) -> Self {
        Self {
            persistence,
            active: Vec::new(),
            sealed: Vec::new(),
            retention,
            audit: ReceiptAuditLog::new(),
        }
    }

    pub fn audit(&self) -> &ReceiptAuditLog<AUDIT> {
        &self.audit
    }

    pub fn active_len(&self) -> usize {
        self.active.len()
    }

    pub fn sealed_len(&self) -> usize {
        self.sealed.len()
    }

    pub fn sealed_receipts(&self) -> impl Iterator<Item = &ActionReceipt> {
        self.sealed.iter()
    }

    pub fn into_persistence(self) -> P {
        self.persistence
    }

    pub fn open(&mut self, open: ReceiptOpen) -> Result<(), ReceiptError> {
        if self
            .active
            .iter()
            .any(|builder| builder.open.receipt_id == open.receipt_id)
            || self
                .sealed
                .iter()
                .any(|receipt| receipt.receipt_id == open.receipt_id)
        {
            return Err(ReceiptError::DuplicateReceipt);
        }
        if self.active.is_full() {
            return Err(ReceiptError::ActiveCapacity);
        }
        let fragment = encode_open_fragment(&open);
        self.persistence
            .append_fragment(
                requester_domain(open.requester),
                open.session_id,
                open.receipt_id,
                fragment.as_slice(),
            )
            .map_err(|_| ReceiptError::PersistenceFailure)?;
        let session_id = open.session_id;
        let receipt_id = open.receipt_id;
        let coordinator_action_id = open.coordinator_action_id;
        self.active
            .push(ReceiptBuilder {
                open,
                timeline: Vec::new(),
                policy: None,
                confirmation: ConfirmationReceiptSummary {
                    outcome: ConfirmationReceiptOutcome::NotRequired,
                },
                dispatch: DispatchReceiptSummary {
                    status: DispatchReceiptStatus::NotDispatched,
                    execution_id: None,
                    audit_id: None,
                },
                observed_outcome: ObservedOutcomeReceiptSummary {
                    outcome: ObservedReceiptOutcome::NotObserved,
                    observation_id: None,
                    audit_id: None,
                },
            })
            .map_err(|_| ReceiptError::ActiveCapacity)?;
        self.audit.push(ReceiptAuditEntry {
            receipt_id: Some(receipt_id),
            coordinator_action_id: Some(coordinator_action_id),
            session_id,
            event: ReceiptAuditEvent::ReceiptOpened,
            public_reason: PublicReasonCode::None,
            timestamp: 0,
        });
        Ok(())
    }

    pub fn append(
        &mut self,
        event: ActionReceiptLifecycleEvent,
    ) -> Result<AppendDisposition, ReceiptError> {
        let Some(index) = self
            .active
            .iter()
            .position(|builder| builder.open.receipt_id == event.receipt_id)
        else {
            if self
                .sealed
                .iter()
                .any(|receipt| receipt.receipt_id == event.receipt_id)
            {
                self.audit_rejection(&event, ReceiptAuditEvent::MismatchedEventRejected);
                return Err(if event.terminal_status.is_some() {
                    ReceiptError::DuplicateTerminal
                } else {
                    ReceiptError::AlreadySealed
                });
            }
            self.audit_rejection(&event, ReceiptAuditEvent::MismatchedEventRejected);
            return Err(ReceiptError::ReceiptNotFound);
        };

        if let Err(error) = validate_event_binding(&self.active[index], &event) {
            self.audit_rejection(&event, ReceiptAuditEvent::MismatchedEventRejected);
            return Err(error);
        }
        if let Err(error) = validate_source(event.source, event.event_type) {
            self.audit_rejection(&event, ReceiptAuditEvent::MismatchedEventRejected);
            return Err(error);
        }
        if let Err(error) = validate_terminal(event.event_type, event.terminal_status) {
            self.audit_rejection(&event, ReceiptAuditEvent::MismatchedEventRejected);
            return Err(error);
        }
        {
            let builder = &self.active[index];
            if let Some(existing) = builder
                .timeline
                .iter()
                .find(|existing| existing.sequence == event.sequence)
            {
                if existing == &event {
                    self.audit_rejection(&event, ReceiptAuditEvent::DuplicateIgnored);
                    return Ok(AppendDisposition::DuplicateIgnored);
                }
                self.audit_rejection(&event, ReceiptAuditEvent::MismatchedEventRejected);
                return Err(ReceiptError::DuplicateSequenceConflict);
            }
            let expected = builder
                .timeline
                .last()
                .map(|previous| previous.sequence.saturating_add(1))
                .unwrap_or(1);
            if event.sequence != expected {
                self.audit_rejection(&event, ReceiptAuditEvent::MismatchedEventRejected);
                return Err(ReceiptError::OutOfOrder);
            }
            if builder.timeline.len() >= MAX_RECEIPT_TIMELINE_EVENTS {
                return Err(ReceiptError::TimelineFull);
            }
        }

        let fragment = encode_event_fragment(&event);
        let owner_domain = requester_domain(event.requester);
        self.persistence
            .append_fragment(
                owner_domain,
                event.session_id,
                event.receipt_id,
                fragment.as_slice(),
            )
            .map_err(|_| ReceiptError::PersistenceFailure)?;

        let terminal_status = event.terminal_status;
        update_summaries(&mut self.active[index], &event);
        self.active[index]
            .timeline
            .push(event.clone())
            .map_err(|_| ReceiptError::TimelineFull)?;
        self.audit.push(ReceiptAuditEntry {
            receipt_id: Some(event.receipt_id),
            coordinator_action_id: Some(event.coordinator_action_id),
            session_id: event.session_id,
            event: ReceiptAuditEvent::LifecycleEventAppended,
            public_reason: event.public_reason,
            timestamp: event.timestamp,
        });

        if let Some(status) = terminal_status {
            let builder = self.active[index].clone();
            let receipt = seal_builder(builder, status, event.timestamp);
            let encoded = receipt.encode_sealed()?;
            self.persistence
                .seal(
                    owner_domain,
                    receipt.session_id,
                    receipt.receipt_id,
                    encoded.as_slice(),
                )
                .map_err(|_| ReceiptError::PersistenceFailure)?;
            let _ = self.active.remove(index);
            self.insert_sealed(receipt)?;
            self.audit.push(ReceiptAuditEntry {
                receipt_id: Some(event.receipt_id),
                coordinator_action_id: Some(event.coordinator_action_id),
                session_id: event.session_id,
                event: ReceiptAuditEvent::ReceiptSealed,
                public_reason: event.public_reason,
                timestamp: event.timestamp,
            });
            return Ok(AppendDisposition::Sealed);
        }

        Ok(AppendDisposition::Appended)
    }

    pub fn query(
        &mut self,
        query: ReceiptQuery,
        locale: &str,
    ) -> Result<ReceiptQueryResult, ReceiptError> {
        let query = match query.validate() {
            Ok(query) => query,
            Err(error) => {
                self.audit.push(ReceiptAuditEntry {
                    receipt_id: None,
                    coordinator_action_id: None,
                    session_id: query.active_session,
                    event: ReceiptAuditEvent::QueryDenied,
                    public_reason: PublicReasonCode::InvalidInput,
                    timestamp: 0,
                });
                return Err(error);
            }
        };

        if matches!(query.kind, ReceiptQueryKind::PendingCurrentAction) {
            let pending = self
                .active
                .iter()
                .rev()
                .find(|builder| {
                    builder.open.session_id == query.active_session
                        && builder.open.requester == query.requester
                })
                .map(|builder| pending_view(builder, locale));
            let Some(pending) = pending else {
                self.audit_query(&query, ReceiptAuditEvent::QueryAllowed);
                return Ok(ReceiptQueryResult::NotFound);
            };
            self.audit_query(&query, ReceiptAuditEvent::QueryAllowed);
            return Ok(ReceiptQueryResult::Pending(pending));
        }

        let mut selected: Vec<ActionReceipt, MAX_RECEIPT_QUERY_RESULTS> = Vec::new();
        for receipt in self.sealed.iter().rev() {
            if receipt.session_id != query.active_session || receipt.requester != query.requester {
                continue;
            }
            let matches = match query.kind {
                ReceiptQueryKind::Latest | ReceiptQueryKind::RecentLimited => true,
                ReceiptQueryKind::LatestCompleted => receipt.terminal_status.is_success(),
                ReceiptQueryKind::ByReceiptId(id) => receipt.receipt_id == id,
                ReceiptQueryKind::LastFailure => !receipt.terminal_status.is_success(),
                ReceiptQueryKind::PendingCurrentAction => false,
            };
            if matches {
                let _ = selected.push(receipt.clone());
                if selected.len() >= query.maximum_results as usize {
                    break;
                }
            }
        }
        if selected.is_empty() {
            self.audit_query(&query, ReceiptAuditEvent::QueryAllowed);
            return Ok(ReceiptQueryResult::NotFound);
        }
        let mut views = Vec::new();
        for receipt in selected.iter() {
            if !receipt.verify_integrity() {
                self.audit.push(ReceiptAuditEntry {
                    receipt_id: Some(receipt.receipt_id),
                    coordinator_action_id: Some(receipt.coordinator_action_id),
                    session_id: receipt.session_id,
                    event: ReceiptAuditEvent::IntegrityFailure,
                    public_reason: PublicReasonCode::Unknown,
                    timestamp: receipt.terminal_timestamp,
                });
                return Ok(ReceiptQueryResult::ReceiptIntegrityFailure);
            }
            let _ = views.push(presentation_view(receipt, locale));
            self.audit.push(ReceiptAuditEntry {
                receipt_id: Some(receipt.receipt_id),
                coordinator_action_id: Some(receipt.coordinator_action_id),
                session_id: receipt.session_id,
                event: ReceiptAuditEvent::IntegrityVerified,
                public_reason: receipt
                    .timeline
                    .last()
                    .map(|event| event.public_reason)
                    .unwrap_or(PublicReasonCode::Unknown),
                timestamp: receipt.terminal_timestamp,
            });
        }
        self.audit_query(&query, ReceiptAuditEvent::QueryAllowed);
        Ok(ReceiptQueryResult::Sealed(views))
    }

    pub fn answer_conversation_query(
        &mut self,
        requester: RequestedBy,
        session_id: SessionId,
        question: ReceiptConversationQuestion,
        locale: &str,
    ) -> Result<ReceiptConversationAnswer, ReceiptError> {
        let result = self.query(
            ReceiptQuery {
                requester,
                active_session: session_id,
                maximum_results: 1,
                kind: ReceiptQueryKind::Latest,
            },
            locale,
        )?;
        let ReceiptQueryResult::Sealed(mut views) = result else {
            return Ok(ReceiptConversationAnswer {
                view: None,
                readiness: match result {
                    ReceiptQueryResult::Pending(_) => ReceiptReadinessAnswer::Pending,
                    _ => ReceiptReadinessAnswer::Unknown,
                },
            });
        };
        let view = views.pop();
        let readiness = if matches!(question, ReceiptConversationQuestion::DidItOpen) {
            match view.as_ref() {
                Some(view) if view.readiness_observed => ReceiptReadinessAnswer::Ready,
                Some(_) => ReceiptReadinessAnswer::NotReady,
                None => ReceiptReadinessAnswer::Unknown,
            }
        } else {
            ReceiptReadinessAnswer::Unknown
        };
        Ok(ReceiptConversationAnswer { view, readiness })
    }

    fn insert_sealed(&mut self, receipt: ActionReceipt) -> Result<(), ReceiptError> {
        while self.sealed.is_full() {
            self.evict_oldest_matching(|_| true)?;
        }
        self.sealed
            .push(receipt)
            .map_err(|_| ReceiptError::ReceiptTooLarge)?;
        self.enforce_domain_retention()
    }

    fn enforce_domain_retention(&mut self) -> Result<(), ReceiptError> {
        loop {
            let mut over_domain = None;
            let mut over_nonexecuted = None;
            for index in 0..self.sealed.len() {
                let receipt = &self.sealed[index];
                let domain_count = self
                    .sealed
                    .iter()
                    .filter(|candidate| same_domain(candidate, receipt))
                    .count();
                if domain_count > self.retention.max_sealed_per_domain {
                    over_domain = Some((receipt.requester, receipt.session_id));
                    break;
                }
                let nonexecuted_count = self
                    .sealed
                    .iter()
                    .filter(|candidate| {
                        same_domain(candidate, receipt) && !candidate.terminal_status.is_executed()
                    })
                    .count();
                if nonexecuted_count > self.retention.max_nonexecuted_per_domain {
                    over_nonexecuted = Some((receipt.requester, receipt.session_id));
                    break;
                }
            }
            if let Some((requester, session)) = over_domain {
                self.evict_oldest_matching(|receipt| {
                    receipt.requester == requester && receipt.session_id == session
                })?;
                continue;
            }
            if let Some((requester, session)) = over_nonexecuted {
                self.evict_oldest_matching(|receipt| {
                    receipt.requester == requester
                        && receipt.session_id == session
                        && !receipt.terminal_status.is_executed()
                })?;
                continue;
            }
            return Ok(());
        }
    }

    fn evict_oldest_matching(
        &mut self,
        predicate: impl Fn(&ActionReceipt) -> bool,
    ) -> Result<(), ReceiptError> {
        let Some(index) = self.sealed.iter().position(predicate) else {
            return Err(ReceiptError::ReceiptTooLarge);
        };
        let evicted = &self.sealed[index];
        self.persistence
            .evict(
                requester_domain(evicted.requester),
                evicted.session_id,
                evicted.receipt_id,
            )
            .map_err(|_| ReceiptError::PersistenceFailure)?;
        let evicted = self.sealed.remove(index);
        self.audit.push(ReceiptAuditEntry {
            receipt_id: Some(evicted.receipt_id),
            coordinator_action_id: Some(evicted.coordinator_action_id),
            session_id: evicted.session_id,
            event: ReceiptAuditEvent::ReceiptEvicted,
            public_reason: PublicReasonCode::None,
            timestamp: evicted.terminal_timestamp,
        });
        Ok(())
    }

    fn audit_rejection(
        &mut self,
        event: &ActionReceiptLifecycleEvent,
        audit_event: ReceiptAuditEvent,
    ) {
        self.audit.push(ReceiptAuditEntry {
            receipt_id: Some(event.receipt_id),
            coordinator_action_id: Some(event.coordinator_action_id),
            session_id: event.session_id,
            event: audit_event,
            public_reason: PublicReasonCode::ReplayRejected,
            timestamp: event.timestamp,
        });
    }

    fn audit_query(&mut self, query: &ReceiptQuery, event: ReceiptAuditEvent) {
        self.audit.push(ReceiptAuditEntry {
            receipt_id: match query.kind {
                ReceiptQueryKind::ByReceiptId(id) => Some(id),
                _ => None,
            },
            coordinator_action_id: None,
            session_id: query.active_session,
            event,
            public_reason: PublicReasonCode::None,
            timestamp: 0,
        });
    }
}

impl<const ACTIVE: usize, const SEALED: usize, const AUDIT: usize>
    ActionReceiptLedger<VolatileReceiptPersistence, ACTIVE, SEALED, AUDIT>
{
    pub fn volatile(retention: ReceiptRetentionPolicy) -> Self {
        Self::new(VolatileReceiptPersistence, retention)
    }
}

fn validate_event_binding(
    builder: &ReceiptBuilder,
    event: &ActionReceiptLifecycleEvent,
) -> Result<(), ReceiptError> {
    if builder.open.coordinator_action_id != event.coordinator_action_id {
        return Err(ReceiptError::WrongAction);
    }
    if builder.open.session_id != event.session_id {
        return Err(ReceiptError::WrongSession);
    }
    if builder.open.requester != event.requester {
        return Err(ReceiptError::WrongRequester);
    }
    Ok(())
}

fn validate_source(
    source: ReceiptEventSource,
    event: ReceiptLifecycleEventType,
) -> Result<(), ReceiptError> {
    let valid = match source {
        ReceiptEventSource::Planner => matches!(
            event,
            ReceiptLifecycleEventType::RequestAccepted
                | ReceiptLifecycleEventType::NoAction
                | ReceiptLifecycleEventType::Unsupported
                | ReceiptLifecycleEventType::ClarificationRequested
                | ReceiptLifecycleEventType::ClarificationResolved
                | ReceiptLifecycleEventType::ClarificationExpired
                | ReceiptLifecycleEventType::InvalidInput
        ),
        ReceiptEventSource::Policy => matches!(
            event,
            ReceiptLifecycleEventType::PolicyAllowed | ReceiptLifecycleEventType::PolicyDenied
        ),
        ReceiptEventSource::ConfirmationAuthority => matches!(
            event,
            ReceiptLifecycleEventType::ConfirmationRequested
                | ReceiptLifecycleEventType::ConfirmationApproved
                | ReceiptLifecycleEventType::ConfirmationRejected
                | ReceiptLifecycleEventType::ConfirmationExpired
        ),
        ReceiptEventSource::Coordinator => matches!(
            event,
            ReceiptLifecycleEventType::RequestAccepted
                | ReceiptLifecycleEventType::Cancelled
                | ReceiptLifecycleEventType::ReadyForExecutionProduced
                | ReceiptLifecycleEventType::AwaitingOutcome
                | ReceiptLifecycleEventType::SessionInvalidated
                | ReceiptLifecycleEventType::RegistryInvalidated
                | ReceiptLifecycleEventType::Completed
        ),
        ReceiptEventSource::Executor => matches!(
            event,
            ReceiptLifecycleEventType::DispatchAccepted | ReceiptLifecycleEventType::DispatchFailed
        ),
        ReceiptEventSource::OutcomeObserver => matches!(
            event,
            ReceiptLifecycleEventType::TargetReady
                | ReceiptLifecycleEventType::TargetExitedEarly
                | ReceiptLifecycleEventType::OutcomeTimedOut
                | ReceiptLifecycleEventType::SessionInvalidated
                | ReceiptLifecycleEventType::RegistryInvalidated
        ),
    };
    if valid {
        Ok(())
    } else {
        Err(ReceiptError::UnsupportedEvent)
    }
}

fn validate_terminal(
    event: ReceiptLifecycleEventType,
    status: Option<ActionReceiptTerminalStatus>,
) -> Result<(), ReceiptError> {
    let valid = match status {
        None => !matches!(
            event,
            ReceiptLifecycleEventType::NoAction
                | ReceiptLifecycleEventType::Unsupported
                | ReceiptLifecycleEventType::ClarificationExpired
                | ReceiptLifecycleEventType::PolicyDenied
                | ReceiptLifecycleEventType::ConfirmationRejected
                | ReceiptLifecycleEventType::ConfirmationExpired
                | ReceiptLifecycleEventType::Cancelled
                | ReceiptLifecycleEventType::DispatchFailed
                | ReceiptLifecycleEventType::TargetExitedEarly
                | ReceiptLifecycleEventType::OutcomeTimedOut
                | ReceiptLifecycleEventType::SessionInvalidated
                | ReceiptLifecycleEventType::RegistryInvalidated
                | ReceiptLifecycleEventType::InvalidInput
                | ReceiptLifecycleEventType::Completed
        ),
        Some(ActionReceiptTerminalStatus::Unsupported) => {
            event == ReceiptLifecycleEventType::Unsupported
        }
        Some(ActionReceiptTerminalStatus::ClarificationExpired) => {
            event == ReceiptLifecycleEventType::ClarificationExpired
        }
        Some(ActionReceiptTerminalStatus::Denied) => {
            event == ReceiptLifecycleEventType::PolicyDenied
        }
        Some(ActionReceiptTerminalStatus::ConfirmationRejected) => {
            event == ReceiptLifecycleEventType::ConfirmationRejected
        }
        Some(ActionReceiptTerminalStatus::ConfirmationExpired) => {
            event == ReceiptLifecycleEventType::ConfirmationExpired
        }
        Some(ActionReceiptTerminalStatus::Cancelled) => {
            event == ReceiptLifecycleEventType::Cancelled
        }
        Some(ActionReceiptTerminalStatus::DispatchFailed) => {
            event == ReceiptLifecycleEventType::DispatchFailed
        }
        Some(ActionReceiptTerminalStatus::ExitedEarly) => {
            event == ReceiptLifecycleEventType::TargetExitedEarly
        }
        Some(ActionReceiptTerminalStatus::OutcomeTimedOut) => {
            event == ReceiptLifecycleEventType::OutcomeTimedOut
        }
        Some(ActionReceiptTerminalStatus::SessionInvalidated) => {
            event == ReceiptLifecycleEventType::SessionInvalidated
        }
        Some(ActionReceiptTerminalStatus::RegistryInvalidated) => {
            event == ReceiptLifecycleEventType::RegistryInvalidated
        }
        Some(ActionReceiptTerminalStatus::InvalidInput) => {
            event == ReceiptLifecycleEventType::InvalidInput
                || event == ReceiptLifecycleEventType::NoAction
        }
        Some(
            ActionReceiptTerminalStatus::CompletedReady
            | ActionReceiptTerminalStatus::DispatchAcceptedOnly
            | ActionReceiptTerminalStatus::Interrupted
            | ActionReceiptTerminalStatus::Unknown,
        ) => event == ReceiptLifecycleEventType::Completed,
    };
    if valid {
        Ok(())
    } else {
        Err(ReceiptError::InvalidTerminalStatus)
    }
}

fn update_summaries(builder: &mut ReceiptBuilder, event: &ActionReceiptLifecycleEvent) {
    match event.event_type {
        ReceiptLifecycleEventType::PolicyAllowed => {
            builder.policy = Some(PolicyReceiptSummary {
                result: PolicyResult::Allowed,
                public_reason: event.public_reason,
                audit_id: event.ids.audit_id,
            });
        }
        ReceiptLifecycleEventType::PolicyDenied => {
            builder.policy = Some(PolicyReceiptSummary {
                result: PolicyResult::Denied,
                public_reason: event.public_reason,
                audit_id: event.ids.audit_id,
            });
        }
        ReceiptLifecycleEventType::ConfirmationRequested => {
            builder.confirmation.outcome = ConfirmationReceiptOutcome::Requested;
        }
        ReceiptLifecycleEventType::ConfirmationApproved => {
            builder.confirmation.outcome = ConfirmationReceiptOutcome::Approved;
        }
        ReceiptLifecycleEventType::ConfirmationRejected => {
            builder.confirmation.outcome = ConfirmationReceiptOutcome::Rejected;
        }
        ReceiptLifecycleEventType::ConfirmationExpired => {
            builder.confirmation.outcome = ConfirmationReceiptOutcome::Expired;
        }
        ReceiptLifecycleEventType::ReadyForExecutionProduced => {
            builder.dispatch.status = DispatchReceiptStatus::ReadyProduced;
            builder.dispatch.audit_id = event.ids.audit_id;
        }
        ReceiptLifecycleEventType::DispatchAccepted => {
            builder.dispatch.status = DispatchReceiptStatus::Accepted;
            builder.dispatch.execution_id = event.ids.execution_id;
            builder.dispatch.audit_id = event.ids.audit_id;
        }
        ReceiptLifecycleEventType::DispatchFailed => {
            builder.dispatch.status = DispatchReceiptStatus::Failed;
            builder.dispatch.execution_id = event.ids.execution_id;
            builder.dispatch.audit_id = event.ids.audit_id;
        }
        ReceiptLifecycleEventType::AwaitingOutcome => {
            builder.observed_outcome.outcome = ObservedReceiptOutcome::Awaiting;
            builder.observed_outcome.observation_id = event.ids.observation_id;
        }
        ReceiptLifecycleEventType::TargetReady => {
            builder.observed_outcome.outcome = ObservedReceiptOutcome::Ready;
            builder.observed_outcome.observation_id = event.ids.observation_id;
            builder.observed_outcome.audit_id = event.ids.audit_id;
        }
        ReceiptLifecycleEventType::TargetExitedEarly => {
            builder.observed_outcome.outcome = ObservedReceiptOutcome::ExitedEarly;
            builder.observed_outcome.observation_id = event.ids.observation_id;
            builder.observed_outcome.audit_id = event.ids.audit_id;
        }
        ReceiptLifecycleEventType::OutcomeTimedOut => {
            builder.observed_outcome.outcome = ObservedReceiptOutcome::TimedOut;
            builder.observed_outcome.observation_id = event.ids.observation_id;
            builder.observed_outcome.audit_id = event.ids.audit_id;
        }
        ReceiptLifecycleEventType::SessionInvalidated => {
            builder.observed_outcome.outcome = ObservedReceiptOutcome::SessionInvalidated;
        }
        ReceiptLifecycleEventType::RegistryInvalidated => {
            builder.observed_outcome.outcome = ObservedReceiptOutcome::RegistryInvalidated;
        }
        _ => {}
    }
}

fn seal_builder(
    builder: ReceiptBuilder,
    terminal_status: ActionReceiptTerminalStatus,
    terminal_timestamp: u64,
) -> ActionReceipt {
    let execution_id = builder
        .timeline
        .iter()
        .rev()
        .find_map(|event| event.ids.execution_id)
        .or(builder.dispatch.execution_id);
    let observation_id = builder
        .timeline
        .iter()
        .rev()
        .find_map(|event| event.ids.observation_id)
        .or(builder.observed_outcome.observation_id);
    let mut receipt = ActionReceipt {
        receipt_id: builder.open.receipt_id,
        coordinator_action_id: builder.open.coordinator_action_id,
        conversation_id: builder.open.conversation_id,
        session_id: builder.open.session_id,
        requester: builder.open.requester,
        original_request_id: builder.open.original_request_id,
        intent_id: builder.open.intent_id,
        execution_id,
        observation_id,
        operation: builder.open.operation,
        target_kind: builder.open.target_kind,
        target_display_key: builder.open.target_display_key,
        request_timestamp: builder.open.request_timestamp,
        terminal_timestamp,
        terminal_status,
        policy: builder.policy,
        confirmation: builder.confirmation,
        dispatch: builder.dispatch,
        observed_outcome: builder.observed_outcome,
        policy_version: builder.open.policy_version,
        planner_version: builder.open.planner_version,
        runtime_snapshot_generation: builder.open.runtime_snapshot_generation,
        application_registry_generation: builder.open.application_registry_generation,
        settings_registry_generation: builder.open.settings_registry_generation,
        bounded_audit_references: builder.open.bounded_audit_references,
        timeline: builder.timeline,
        integrity_digest: [0; 32],
        schema_version: ACTION_RECEIPT_SCHEMA_VERSION,
    };
    receipt.integrity_digest = compute_integrity_digest(&receipt);
    receipt
}

fn compute_integrity_digest(receipt: &ActionReceipt) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(ACTION_RECEIPT_DIGEST_DOMAIN);
    hasher.update(receipt.schema_version.to_le_bytes());
    hasher.update(receipt.receipt_id.0);
    hasher.update(receipt.coordinator_action_id.0.to_le_bytes());
    hasher.update(receipt.conversation_id.0.to_le_bytes());
    hasher.update(receipt.session_id.0.to_le_bytes());
    hash_requester(&mut hasher, receipt.requester);
    hasher.update(receipt.original_request_id.0.to_le_bytes());
    hash_optional_intent(&mut hasher, receipt.intent_id);
    hash_optional_u64(&mut hasher, receipt.execution_id.map(|id| id.0));
    hash_optional_u64(&mut hasher, receipt.observation_id.map(|id| id.0));
    hash_optional_u16(&mut hasher, receipt.operation.map(action_operation_code));
    hash_optional_u16(&mut hasher, receipt.target_kind.map(target_kind_code));
    match receipt.target_display_key.as_ref() {
        Some(key) => {
            hasher.update([1]);
            hasher.update((key.as_str().len() as u16).to_le_bytes());
            hasher.update(key.as_str().as_bytes());
        }
        None => hasher.update([0]),
    }
    hasher.update(receipt.request_timestamp.to_le_bytes());
    hasher.update(receipt.terminal_timestamp.to_le_bytes());
    hasher.update([terminal_status_code(receipt.terminal_status)]);
    hasher.update(receipt.policy_version.major.to_le_bytes());
    hasher.update(receipt.policy_version.minor.to_le_bytes());
    hasher.update(receipt.planner_version.major.to_le_bytes());
    hasher.update(receipt.planner_version.minor.to_le_bytes());
    hasher.update(receipt.runtime_snapshot_generation.to_le_bytes());
    hasher.update(receipt.application_registry_generation.to_le_bytes());
    hasher.update(receipt.settings_registry_generation.to_le_bytes());
    hasher.update((receipt.bounded_audit_references.len() as u16).to_le_bytes());
    for audit_id in receipt.bounded_audit_references.iter() {
        hasher.update(audit_id.0.to_le_bytes());
    }
    hasher.update((receipt.timeline.len() as u16).to_le_bytes());
    for event in receipt.timeline.iter() {
        hash_event(&mut hasher, event);
    }
    hasher.finalize().into()
}

fn hash_event(hasher: &mut Sha256, event: &ActionReceiptLifecycleEvent) {
    hasher.update(event.receipt_id.0);
    hasher.update(event.coordinator_action_id.0.to_le_bytes());
    hasher.update(event.session_id.0.to_le_bytes());
    hash_requester(hasher, event.requester);
    hasher.update(event.sequence.to_le_bytes());
    hasher.update(event.timestamp.to_le_bytes());
    hasher.update([event_type_code(event.event_type)]);
    hasher.update([reason_code(event.public_reason)]);
    hasher.update([source_code(event.source)]);
    hash_optional_intent(hasher, event.ids.intent_id);
    hash_optional_u64(hasher, event.ids.execution_id.map(|id| id.0));
    hash_optional_u64(hasher, event.ids.observation_id.map(|id| id.0));
    hash_optional_u64(hasher, event.ids.audit_id.map(|id| id.0));
    hash_optional_u8(hasher, event.terminal_status.map(terminal_status_code));
}

fn encode_open_fragment(open: &ReceiptOpen) -> AllocVec<u8> {
    let mut bytes = AllocVec::with_capacity(96);
    bytes.extend_from_slice(b"OPEN");
    bytes.extend_from_slice(&open.receipt_id.0);
    bytes.extend_from_slice(&open.coordinator_action_id.0.to_le_bytes());
    bytes.extend_from_slice(&open.conversation_id.0.to_le_bytes());
    bytes.extend_from_slice(&open.session_id.0.to_le_bytes());
    bytes.extend_from_slice(&open.original_request_id.0.to_le_bytes());
    bytes.extend_from_slice(&open.request_timestamp.to_le_bytes());
    bytes
}

fn encode_event_fragment(event: &ActionReceiptLifecycleEvent) -> AllocVec<u8> {
    let mut bytes = AllocVec::with_capacity(96);
    bytes.extend_from_slice(b"EVNT");
    encode_event(&mut bytes, event);
    bytes
}

fn encode_event(bytes: &mut AllocVec<u8>, event: &ActionReceiptLifecycleEvent) {
    bytes.extend_from_slice(&event.receipt_id.0);
    bytes.extend_from_slice(&event.coordinator_action_id.0.to_le_bytes());
    bytes.extend_from_slice(&event.session_id.0.to_le_bytes());
    bytes.extend_from_slice(&event.sequence.to_le_bytes());
    bytes.extend_from_slice(&event.timestamp.to_le_bytes());
    bytes.push(event_type_code(event.event_type));
    bytes.push(reason_code(event.public_reason));
    bytes.push(source_code(event.source));
    bytes.push(
        event
            .terminal_status
            .map(terminal_status_code)
            .unwrap_or(u8::MAX),
    );
}

fn presentation_view(receipt: &ActionReceipt, locale: &str) -> ActionReceiptView {
    let persian = locale.starts_with("fa");
    let mut lifecycle_steps = Vec::new();
    for event in receipt.timeline.iter() {
        let mut step = String::new();
        let _ = step.push_str(event_label(event.event_type, persian));
        let _ = lifecycle_steps.push(step);
    }
    ActionReceiptView {
        receipt_id: receipt.receipt_id,
        localized_title: bounded_text(if persian {
            "گزارش عملیات"
        } else {
            "Action receipt"
        }),
        operation_display_label: bounded_text(operation_label(receipt.operation, persian)),
        target_display_label: bounded_text(target_label(
            receipt.target_display_key.as_ref(),
            persian,
        )),
        terminal_status_label: bounded_text(terminal_label(receipt.terminal_status, persian)),
        start_time: receipt.request_timestamp,
        completion_time: receipt.terminal_timestamp,
        lifecycle_steps,
        public_reason: receipt
            .timeline
            .last()
            .map(|event| event.public_reason)
            .unwrap_or(PublicReasonCode::Unknown),
        confirmation_occurred: !matches!(
            receipt.confirmation.outcome,
            ConfirmationReceiptOutcome::NotRequired
        ),
        readiness_observed: receipt.observed_outcome.outcome == ObservedReceiptOutcome::Ready,
    }
}

fn pending_view(builder: &ReceiptBuilder, locale: &str) -> PendingActionReceiptView {
    let persian = locale.starts_with("fa");
    let state = match builder.timeline.last().map(|event| event.event_type) {
        Some(ReceiptLifecycleEventType::ClarificationRequested) => {
            PendingReceiptState::AwaitingClarification
        }
        Some(ReceiptLifecycleEventType::ConfirmationRequested) => {
            PendingReceiptState::AwaitingConfirmation
        }
        Some(ReceiptLifecycleEventType::DispatchAccepted)
        | Some(ReceiptLifecycleEventType::AwaitingOutcome) => PendingReceiptState::AwaitingOutcome,
        _ => PendingReceiptState::Dispatching,
    };
    PendingActionReceiptView {
        receipt_id: builder.open.receipt_id,
        state,
        operation_label: bounded_text(operation_label(builder.open.operation, persian)),
        target_label: bounded_text(target_label(
            builder.open.target_display_key.as_ref(),
            persian,
        )),
        start_time: builder.open.request_timestamp,
    }
}

fn bounded_text(value: &str) -> String<MAX_RECEIPT_VIEW_TEXT> {
    let mut output = String::new();
    let _ = output.push_str(value);
    output
}

fn operation_label(operation: Option<ActionOperation>, persian: bool) -> &'static str {
    match (operation, persian) {
        (Some(ActionOperation::OpenApplication), false) => "Open application",
        (Some(ActionOperation::OpenApplication), true) => "باز کردن برنامه",
        (Some(ActionOperation::OpenSettingsPage), false) => "Open settings",
        (Some(ActionOperation::OpenSettingsPage), true) => "باز کردن تنظیمات",
        (Some(_), false) => "System action",
        (Some(_), true) => "عملیات سامانه",
        (None, false) => "Action",
        (None, true) => "عملیات",
    }
}

fn target_label(key: Option<&TargetDisplayKey>, persian: bool) -> &'static str {
    match (key.map(TargetDisplayKey::as_str), persian) {
        (Some("calculator"), false) => "Calculator",
        (Some("calculator"), true) => "ماشین حساب",
        (Some("display-settings"), false) => "Display Settings",
        (Some("display-settings"), true) => "تنظیمات نمایش",
        (Some("network-settings"), false) => "Network Settings",
        (Some("network-settings"), true) => "تنظیمات شبکه",
        (Some(_), false) => "Requested target",
        (Some(_), true) => "هدف درخواست‌شده",
        (None, false) => "Unknown target",
        (None, true) => "هدف نامشخص",
    }
}

fn terminal_label(status: ActionReceiptTerminalStatus, persian: bool) -> &'static str {
    match (status, persian) {
        (ActionReceiptTerminalStatus::CompletedReady, false) => "Ready",
        (ActionReceiptTerminalStatus::CompletedReady, true) => "آماده",
        (ActionReceiptTerminalStatus::DispatchAcceptedOnly, false) => "Launch accepted",
        (ActionReceiptTerminalStatus::DispatchAcceptedOnly, true) => "اجرای برنامه پذیرفته شد",
        (ActionReceiptTerminalStatus::Denied, false) => "Denied",
        (ActionReceiptTerminalStatus::Denied, true) => "رد شد",
        (ActionReceiptTerminalStatus::Unsupported, false) => "Unsupported",
        (ActionReceiptTerminalStatus::Unsupported, true) => "پشتیبانی نمی‌شود",
        (ActionReceiptTerminalStatus::Cancelled, false) => "Cancelled",
        (ActionReceiptTerminalStatus::Cancelled, true) => "لغو شد",
        (ActionReceiptTerminalStatus::OutcomeTimedOut, false) => "Timed out",
        (ActionReceiptTerminalStatus::OutcomeTimedOut, true) => "زمان انتظار پایان یافت",
        (ActionReceiptTerminalStatus::DispatchFailed, false) => "Launch failed",
        (ActionReceiptTerminalStatus::DispatchFailed, true) => "اجرا ناموفق بود",
        (_, false) => "Not completed successfully",
        (_, true) => "با موفقیت کامل نشد",
    }
}

fn event_label(event: ReceiptLifecycleEventType, persian: bool) -> &'static str {
    match (event, persian) {
        (ReceiptLifecycleEventType::RequestAccepted, false) => "Request understood",
        (ReceiptLifecycleEventType::RequestAccepted, true) => "درخواست تشخیص داده شد",
        (ReceiptLifecycleEventType::PolicyAllowed, false) => "Policy allowed",
        (ReceiptLifecycleEventType::PolicyAllowed, true) => "سیاست اجازه داد",
        (ReceiptLifecycleEventType::PolicyDenied, false) => "Policy denied",
        (ReceiptLifecycleEventType::PolicyDenied, true) => "سیاست اجازه نداد",
        (ReceiptLifecycleEventType::ConfirmationRequested, false) => "Confirmation requested",
        (ReceiptLifecycleEventType::ConfirmationRequested, true) => "تأیید درخواست شد",
        (ReceiptLifecycleEventType::ConfirmationApproved, false) => "Confirmation approved",
        (ReceiptLifecycleEventType::ConfirmationApproved, true) => "تأیید پذیرفته شد",
        (ReceiptLifecycleEventType::ReadyForExecutionProduced, false) => {
            "Action prepared for trusted execution"
        }
        (ReceiptLifecycleEventType::ReadyForExecutionProduced, true) => {
            "عملیات برای اجرای امن آماده شد"
        }
        (ReceiptLifecycleEventType::DispatchAccepted, false) => "Launch accepted",
        (ReceiptLifecycleEventType::DispatchAccepted, true) => "اجرای برنامه پذیرفته شد",
        (ReceiptLifecycleEventType::AwaitingOutcome, false) => "Waiting for target readiness",
        (ReceiptLifecycleEventType::AwaitingOutcome, true) => "در انتظار آماده‌شدن هدف",
        (ReceiptLifecycleEventType::TargetReady, false) => "Target became ready",
        (ReceiptLifecycleEventType::TargetReady, true) => "هدف آماده شد",
        (ReceiptLifecycleEventType::Completed, false) => "Action completed",
        (ReceiptLifecycleEventType::Completed, true) => "عملیات کامل شد",
        (_, false) => "Action lifecycle updated",
        (_, true) => "روند عملیات به‌روزرسانی شد",
    }
}

fn same_domain(left: &ActionReceipt, right: &ActionReceipt) -> bool {
    left.requester == right.requester && left.session_id == right.session_id
}

pub const fn requester_domain(requester: RequestedBy) -> u64 {
    match requester {
        RequestedBy::User(user_id) => user_id as u64,
        RequestedBy::WiseOwlReasoning => u64::MAX - 1,
        RequestedBy::SystemComponent(component) => (u64::MAX - 0x1_0000) + component as u64,
    }
}

fn hash_requester(hasher: &mut Sha256, requester: RequestedBy) {
    match requester {
        RequestedBy::User(user_id) => {
            hasher.update([1]);
            hasher.update(user_id.to_le_bytes());
        }
        RequestedBy::WiseOwlReasoning => hasher.update([2]),
        RequestedBy::SystemComponent(component) => {
            hasher.update([3]);
            hasher.update(component.to_le_bytes());
        }
    }
}

fn hash_optional_intent(hasher: &mut Sha256, value: Option<IntentId>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.bytes());
        }
        None => hasher.update([0]),
    }
}

fn hash_optional_u64(hasher: &mut Sha256, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_le_bytes());
        }
        None => hasher.update([0]),
    }
}

fn hash_optional_u16(hasher: &mut Sha256, value: Option<u16>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_le_bytes());
        }
        None => hasher.update([0]),
    }
}

fn hash_optional_u8(hasher: &mut Sha256, value: Option<u8>) {
    match value {
        Some(value) => hasher.update([1, value]),
        None => hasher.update([0]),
    }
}

const fn action_operation_code(operation: ActionOperation) -> u16 {
    match operation {
        ActionOperation::Observe => 1,
        ActionOperation::OpenApplication => 2,
        ActionOperation::OpenSettingsPage => 3,
        ActionOperation::LaunchUtility => 4,
        ActionOperation::RestartService => 5,
        ActionOperation::StopService => 6,
        ActionOperation::InstallPackage => 7,
        ActionOperation::RemovePackage => 8,
        ActionOperation::ModifyFile => 9,
        ActionOperation::DeleteFile => 10,
        ActionOperation::ModifyBootConfiguration => 11,
        ActionOperation::EraseDisk => 12,
        ActionOperation::UnknownOperation(value) => 0x8000 | value,
    }
}

const fn target_kind_code(kind: TargetKind) -> u16 {
    match kind {
        TargetKind::Application => 1,
        TargetKind::SettingsPage => 2,
        TargetKind::Utility => 3,
        TargetKind::Service => 4,
        TargetKind::Package => 5,
        TargetKind::File => 6,
        TargetKind::Disk => 7,
        TargetKind::System => 8,
        TargetKind::Unknown => 0,
    }
}

const fn source_code(source: ReceiptEventSource) -> u8 {
    match source {
        ReceiptEventSource::Planner => 1,
        ReceiptEventSource::Policy => 2,
        ReceiptEventSource::ConfirmationAuthority => 3,
        ReceiptEventSource::Coordinator => 4,
        ReceiptEventSource::Executor => 5,
        ReceiptEventSource::OutcomeObserver => 6,
    }
}

const fn event_type_code(event: ReceiptLifecycleEventType) -> u8 {
    match event {
        ReceiptLifecycleEventType::RequestAccepted => 1,
        ReceiptLifecycleEventType::NoAction => 2,
        ReceiptLifecycleEventType::Unsupported => 3,
        ReceiptLifecycleEventType::ClarificationRequested => 4,
        ReceiptLifecycleEventType::ClarificationResolved => 5,
        ReceiptLifecycleEventType::ClarificationExpired => 6,
        ReceiptLifecycleEventType::PolicyAllowed => 7,
        ReceiptLifecycleEventType::PolicyDenied => 8,
        ReceiptLifecycleEventType::ConfirmationRequested => 9,
        ReceiptLifecycleEventType::ConfirmationApproved => 10,
        ReceiptLifecycleEventType::ConfirmationRejected => 11,
        ReceiptLifecycleEventType::ConfirmationExpired => 12,
        ReceiptLifecycleEventType::Cancelled => 13,
        ReceiptLifecycleEventType::ReadyForExecutionProduced => 14,
        ReceiptLifecycleEventType::DispatchAccepted => 15,
        ReceiptLifecycleEventType::DispatchFailed => 16,
        ReceiptLifecycleEventType::AwaitingOutcome => 17,
        ReceiptLifecycleEventType::TargetReady => 18,
        ReceiptLifecycleEventType::TargetExitedEarly => 19,
        ReceiptLifecycleEventType::OutcomeTimedOut => 20,
        ReceiptLifecycleEventType::SessionInvalidated => 21,
        ReceiptLifecycleEventType::RegistryInvalidated => 22,
        ReceiptLifecycleEventType::InvalidInput => 23,
        ReceiptLifecycleEventType::Completed => 24,
    }
}

const fn terminal_status_code(status: ActionReceiptTerminalStatus) -> u8 {
    match status {
        ActionReceiptTerminalStatus::CompletedReady => 1,
        ActionReceiptTerminalStatus::DispatchAcceptedOnly => 2,
        ActionReceiptTerminalStatus::Denied => 3,
        ActionReceiptTerminalStatus::Unsupported => 4,
        ActionReceiptTerminalStatus::Cancelled => 5,
        ActionReceiptTerminalStatus::ClarificationExpired => 6,
        ActionReceiptTerminalStatus::ConfirmationRejected => 7,
        ActionReceiptTerminalStatus::ConfirmationExpired => 8,
        ActionReceiptTerminalStatus::DispatchFailed => 9,
        ActionReceiptTerminalStatus::ExitedEarly => 10,
        ActionReceiptTerminalStatus::OutcomeTimedOut => 11,
        ActionReceiptTerminalStatus::SessionInvalidated => 12,
        ActionReceiptTerminalStatus::RegistryInvalidated => 13,
        ActionReceiptTerminalStatus::InvalidInput => 14,
        ActionReceiptTerminalStatus::Interrupted => 15,
        ActionReceiptTerminalStatus::Unknown => 0,
    }
}

const fn reason_code(reason: PublicReasonCode) -> u8 {
    match reason {
        PublicReasonCode::None => 0,
        PublicReasonCode::ClarificationNeeded => 1,
        PublicReasonCode::ConfirmationNeeded => 2,
        PublicReasonCode::DispatchAccepted => 3,
        PublicReasonCode::OutcomeReady => 4,
        PublicReasonCode::OutcomeFailed => 5,
        PublicReasonCode::OutcomeTimedOut => 6,
        PublicReasonCode::ExitedBeforeReady => 7,
        PublicReasonCode::ObservationCancelled => 8,
        PublicReasonCode::DispatchFailed => 9,
        PublicReasonCode::TargetNotFound => 10,
        PublicReasonCode::TargetUnavailable => 11,
        PublicReasonCode::PolicyDenied => 12,
        PublicReasonCode::Unsupported => 13,
        PublicReasonCode::InvalidInput => 14,
        PublicReasonCode::InvalidSelection => 15,
        PublicReasonCode::RejectedByUser => 16,
        PublicReasonCode::CancelledByUser => 17,
        PublicReasonCode::Expired => 18,
        PublicReasonCode::SessionEnded => 19,
        PublicReasonCode::SessionChanged => 20,
        PublicReasonCode::PolicyChanged => 21,
        PublicReasonCode::RuntimeChanged => 22,
        PublicReasonCode::ApplicationRegistryChanged => 23,
        PublicReasonCode::SettingsRegistryChanged => 24,
        PublicReasonCode::ReplayRejected => 25,
        PublicReasonCode::ActionAlreadyPending => 26,
        PublicReasonCode::Unknown => 255,
    }
}

const fn execution_reason(code: ExecutionResultCode) -> PublicReasonCode {
    match code {
        ExecutionResultCode::Succeeded => PublicReasonCode::DispatchAccepted,
        ExecutionResultCode::TargetNotFound => PublicReasonCode::TargetNotFound,
        ExecutionResultCode::TargetUnavailable => PublicReasonCode::TargetUnavailable,
        ExecutionResultCode::SessionInactive => PublicReasonCode::SessionEnded,
        ExecutionResultCode::PolicyChanged => PublicReasonCode::PolicyChanged,
        ExecutionResultCode::RuntimeStale => PublicReasonCode::RuntimeChanged,
        ExecutionResultCode::ConfirmationExpired => PublicReasonCode::Expired,
        ExecutionResultCode::DispatchFailed => PublicReasonCode::DispatchFailed,
        ExecutionResultCode::Rejected
        | ExecutionResultCode::UnsupportedOperation
        | ExecutionResultCode::InvalidEnvelope
        | ExecutionResultCode::ConfirmationInvalid
        | ExecutionResultCode::AlreadyConsumed
        | ExecutionResultCode::Unknown => PublicReasonCode::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiseowl_memorydb::{ActionReceiptBlobStore, MemoryStore};

    type Ledger = ActionReceiptLedger<ActionReceiptBlobStore<MemoryStore>, 4, 8, 64>;

    fn ledger() -> Ledger {
        ActionReceiptLedger::new(
            ActionReceiptBlobStore::open(MemoryStore::default()).unwrap(),
            ReceiptRetentionPolicy {
                max_sealed_per_domain: 3,
                max_nonexecuted_per_domain: 2,
            },
        )
    }

    fn open(request: u64, user: u32, session: u64, target: &str) -> ReceiptOpen {
        let action_id = CoordinatorActionId(request);
        let conversation_id = ConversationId(44);
        let session_id = SessionId(session);
        let requester = RequestedBy::User(user);
        let request_id = PlannerRequestId(request);
        ReceiptOpen {
            receipt_id: ActionReceiptId::for_action(
                action_id,
                conversation_id,
                session_id,
                requester,
                request_id,
            ),
            coordinator_action_id: action_id,
            conversation_id,
            session_id,
            requester,
            original_request_id: request_id,
            intent_id: Some(IntentId::new([request as u8; 16])),
            operation: Some(ActionOperation::OpenApplication),
            target_kind: Some(TargetKind::Application),
            target_display_key: Some(TargetDisplayKey::new(target).unwrap()),
            request_timestamp: 100 + request,
            policy_version: PolicyVersion::new(1, 0),
            planner_version: PlannerVersion::new(1, 0),
            runtime_snapshot_generation: 7,
            application_registry_generation: 9,
            settings_registry_generation: 10,
            bounded_audit_references: Vec::new(),
        }
    }

    fn event(
        open: &ReceiptOpen,
        sequence: u16,
        event_type: ReceiptLifecycleEventType,
        source: ReceiptEventSource,
        reason: PublicReasonCode,
        terminal_status: Option<ActionReceiptTerminalStatus>,
    ) -> ActionReceiptLifecycleEvent {
        ActionReceiptLifecycleEvent::new(
            open.receipt_id,
            open.coordinator_action_id,
            open.session_id,
            open.requester,
            sequence,
            200 + sequence as u64,
            event_type,
            reason,
            ReceiptRelevantIds {
                intent_id: open.intent_id,
                execution_id: Some(ExecutionId(31)),
                observation_id: Some(ObservationId(41)),
                audit_id: Some(AuditId(sequence as u64)),
            },
            source,
            terminal_status,
        )
    }

    fn seal_ready(ledger: &mut Ledger, open: &ReceiptOpen) {
        ledger.open(open.clone()).unwrap();
        let events = [
            (
                ReceiptLifecycleEventType::RequestAccepted,
                ReceiptEventSource::Planner,
                PublicReasonCode::None,
                None,
            ),
            (
                ReceiptLifecycleEventType::PolicyAllowed,
                ReceiptEventSource::Policy,
                PublicReasonCode::None,
                None,
            ),
            (
                ReceiptLifecycleEventType::ReadyForExecutionProduced,
                ReceiptEventSource::Coordinator,
                PublicReasonCode::None,
                None,
            ),
            (
                ReceiptLifecycleEventType::DispatchAccepted,
                ReceiptEventSource::Executor,
                PublicReasonCode::DispatchAccepted,
                None,
            ),
            (
                ReceiptLifecycleEventType::AwaitingOutcome,
                ReceiptEventSource::Coordinator,
                PublicReasonCode::DispatchAccepted,
                None,
            ),
            (
                ReceiptLifecycleEventType::TargetReady,
                ReceiptEventSource::OutcomeObserver,
                PublicReasonCode::OutcomeReady,
                None,
            ),
            (
                ReceiptLifecycleEventType::Completed,
                ReceiptEventSource::Coordinator,
                PublicReasonCode::OutcomeReady,
                Some(ActionReceiptTerminalStatus::CompletedReady),
            ),
        ];
        for (index, (kind, source, reason, terminal)) in events.into_iter().enumerate() {
            ledger
                .append(event(
                    open,
                    index as u16 + 1,
                    kind,
                    source,
                    reason,
                    terminal,
                ))
                .unwrap();
        }
    }

    #[test]
    fn successful_application_receipt_preserves_dispatch_ready_distinction() {
        let mut ledger = ledger();
        let open = open(1, 7, 11, "calculator");
        seal_ready(&mut ledger, &open);
        let receipt = ledger.sealed_receipts().next().unwrap();
        assert_eq!(
            receipt.dispatch_summary().status,
            DispatchReceiptStatus::Accepted
        );
        assert_eq!(
            receipt.observed_outcome_summary().outcome,
            ObservedReceiptOutcome::Ready
        );
        assert_eq!(
            receipt.terminal_status(),
            ActionReceiptTerminalStatus::CompletedReady
        );
        assert!(receipt.verify_integrity());
    }

    #[test]
    fn settings_receipt_and_bilingual_presentation() {
        let mut ledger = ledger();
        let mut open = open(2, 7, 11, "display-settings");
        open.operation = Some(ActionOperation::OpenSettingsPage);
        open.target_kind = Some(TargetKind::SettingsPage);
        seal_ready(&mut ledger, &open);
        let english = ledger
            .query(
                ReceiptQuery {
                    requester: open.requester,
                    active_session: open.session_id,
                    maximum_results: 1,
                    kind: ReceiptQueryKind::Latest,
                },
                "en",
            )
            .unwrap();
        let persian = ledger
            .query(
                ReceiptQuery {
                    requester: open.requester,
                    active_session: open.session_id,
                    maximum_results: 1,
                    kind: ReceiptQueryKind::Latest,
                },
                "fa-IR",
            )
            .unwrap();
        let ReceiptQueryResult::Sealed(english) = english else {
            panic!()
        };
        let ReceiptQueryResult::Sealed(persian) = persian else {
            panic!()
        };
        assert_eq!(english[0].target_display_label.as_str(), "Display Settings");
        assert_eq!(persian[0].target_display_label.as_str(), "تنظیمات نمایش");
    }

    #[test]
    fn duplicate_out_of_order_wrong_binding_and_seal_are_rejected() {
        let mut ledger = ledger();
        let open = open(3, 7, 11, "calculator");
        ledger.open(open.clone()).unwrap();
        let first = event(
            &open,
            1,
            ReceiptLifecycleEventType::RequestAccepted,
            ReceiptEventSource::Planner,
            PublicReasonCode::None,
            None,
        );
        assert_eq!(
            ledger.append(first.clone()).unwrap(),
            AppendDisposition::Appended
        );
        assert_eq!(
            ledger.append(first).unwrap(),
            AppendDisposition::DuplicateIgnored
        );
        assert_eq!(
            ledger.append(event(
                &open,
                3,
                ReceiptLifecycleEventType::PolicyAllowed,
                ReceiptEventSource::Policy,
                PublicReasonCode::None,
                None,
            )),
            Err(ReceiptError::OutOfOrder)
        );
        let mut wrong = event(
            &open,
            2,
            ReceiptLifecycleEventType::PolicyAllowed,
            ReceiptEventSource::Policy,
            PublicReasonCode::None,
            None,
        );
        wrong.session_id = SessionId(99);
        assert_eq!(ledger.append(wrong), Err(ReceiptError::WrongSession));
        let terminal = event(
            &open,
            2,
            ReceiptLifecycleEventType::PolicyDenied,
            ReceiptEventSource::Policy,
            PublicReasonCode::PolicyDenied,
            Some(ActionReceiptTerminalStatus::Denied),
        );
        assert_eq!(
            ledger.append(terminal.clone()).unwrap(),
            AppendDisposition::Sealed
        );
        assert_eq!(
            ledger.append(terminal),
            Err(ReceiptError::DuplicateTerminal)
        );
    }

    #[test]
    fn terminal_failure_variants_are_typed() {
        let variants = [
            (
                ReceiptLifecycleEventType::Unsupported,
                ReceiptEventSource::Planner,
                ActionReceiptTerminalStatus::Unsupported,
            ),
            (
                ReceiptLifecycleEventType::ClarificationExpired,
                ReceiptEventSource::Planner,
                ActionReceiptTerminalStatus::ClarificationExpired,
            ),
            (
                ReceiptLifecycleEventType::ConfirmationRejected,
                ReceiptEventSource::ConfirmationAuthority,
                ActionReceiptTerminalStatus::ConfirmationRejected,
            ),
            (
                ReceiptLifecycleEventType::ConfirmationExpired,
                ReceiptEventSource::ConfirmationAuthority,
                ActionReceiptTerminalStatus::ConfirmationExpired,
            ),
            (
                ReceiptLifecycleEventType::Cancelled,
                ReceiptEventSource::Coordinator,
                ActionReceiptTerminalStatus::Cancelled,
            ),
            (
                ReceiptLifecycleEventType::DispatchFailed,
                ReceiptEventSource::Executor,
                ActionReceiptTerminalStatus::DispatchFailed,
            ),
            (
                ReceiptLifecycleEventType::TargetExitedEarly,
                ReceiptEventSource::OutcomeObserver,
                ActionReceiptTerminalStatus::ExitedEarly,
            ),
            (
                ReceiptLifecycleEventType::OutcomeTimedOut,
                ReceiptEventSource::OutcomeObserver,
                ActionReceiptTerminalStatus::OutcomeTimedOut,
            ),
            (
                ReceiptLifecycleEventType::SessionInvalidated,
                ReceiptEventSource::OutcomeObserver,
                ActionReceiptTerminalStatus::SessionInvalidated,
            ),
            (
                ReceiptLifecycleEventType::RegistryInvalidated,
                ReceiptEventSource::OutcomeObserver,
                ActionReceiptTerminalStatus::RegistryInvalidated,
            ),
        ];
        for (index, (kind, source, status)) in variants.into_iter().enumerate() {
            let mut ledger = ledger();
            let open = open(index as u64 + 10, 7, 11, "calculator");
            ledger.open(open.clone()).unwrap();
            ledger
                .append(event(
                    &open,
                    1,
                    kind,
                    source,
                    PublicReasonCode::Unknown,
                    Some(status),
                ))
                .unwrap();
            assert_eq!(
                ledger.sealed_receipts().next().unwrap().terminal_status(),
                status
            );
        }
    }

    #[test]
    fn bounded_retention_is_deterministic_and_active_is_not_evicted() {
        let mut ledger = ledger();
        let active = open(90, 7, 11, "calculator");
        ledger.open(active).unwrap();
        for request in 1..=5 {
            let open = open(request, 7, 11, "calculator");
            seal_ready(&mut ledger, &open);
        }
        assert_eq!(ledger.active_len(), 1);
        assert_eq!(ledger.sealed_len(), 3);
        let ids: AllocVec<_> = ledger
            .sealed_receipts()
            .map(ActionReceipt::original_request_id)
            .collect();
        assert_eq!(
            ids,
            vec![
                PlannerRequestId(3),
                PlannerRequestId(4),
                PlannerRequestId(5)
            ]
        );
    }

    #[test]
    fn user_guest_and_session_isolation_are_fail_closed() {
        let mut ledger = ledger();
        let open = open(1, 7, 11, "calculator");
        seal_ready(&mut ledger, &open);
        for (requester, session) in [
            (RequestedBy::User(8), SessionId(11)),
            (RequestedBy::User(7), SessionId(12)),
            (RequestedBy::User(65534), SessionId(11)),
        ] {
            assert_eq!(
                ledger
                    .query(
                        ReceiptQuery {
                            requester,
                            active_session: session,
                            maximum_results: 1,
                            kind: ReceiptQueryKind::Latest,
                        },
                        "en",
                    )
                    .unwrap(),
                ReceiptQueryResult::NotFound
            );
        }
    }

    #[test]
    fn corrupted_receipt_is_hidden_and_unknown_is_not_success() {
        let mut ledger = ledger();
        let open = open(1, 7, 11, "calculator");
        seal_ready(&mut ledger, &open);
        ledger.sealed[0].corrupt_digest_for_test();
        assert_eq!(
            ledger
                .query(
                    ReceiptQuery {
                        requester: open.requester,
                        active_session: open.session_id,
                        maximum_results: 1,
                        kind: ReceiptQueryKind::Latest,
                    },
                    "en",
                )
                .unwrap(),
            ReceiptQueryResult::ReceiptIntegrityFailure
        );
        assert!(!ActionReceiptTerminalStatus::Unknown.is_success());
    }

    #[test]
    fn conversation_readiness_uses_observer_result_not_dispatch() {
        let mut ledger = ledger();
        let open = open(1, 7, 11, "calculator");
        ledger.open(open.clone()).unwrap();
        for (sequence, kind, source, terminal) in [
            (
                1,
                ReceiptLifecycleEventType::RequestAccepted,
                ReceiptEventSource::Planner,
                None,
            ),
            (
                2,
                ReceiptLifecycleEventType::PolicyAllowed,
                ReceiptEventSource::Policy,
                None,
            ),
            (
                3,
                ReceiptLifecycleEventType::DispatchAccepted,
                ReceiptEventSource::Executor,
                None,
            ),
            (
                4,
                ReceiptLifecycleEventType::Completed,
                ReceiptEventSource::Coordinator,
                Some(ActionReceiptTerminalStatus::DispatchAcceptedOnly),
            ),
        ] {
            ledger
                .append(event(
                    &open,
                    sequence,
                    kind,
                    source,
                    PublicReasonCode::DispatchAccepted,
                    terminal,
                ))
                .unwrap();
        }
        let answer = ledger
            .answer_conversation_query(
                open.requester,
                open.session_id,
                ReceiptConversationQuestion::DidItOpen,
                "en",
            )
            .unwrap();
        assert_eq!(answer.readiness, ReceiptReadinessAnswer::NotReady);
    }

    #[test]
    fn bounded_queries_reject_enumeration() {
        let mut ledger = ledger();
        assert_eq!(
            ledger.query(
                ReceiptQuery {
                    requester: RequestedBy::User(7),
                    active_session: SessionId(11),
                    maximum_results: 0,
                    kind: ReceiptQueryKind::RecentLimited,
                },
                "en",
            ),
            Err(ReceiptError::UnboundedQuery)
        );
    }

    #[test]
    fn audit_contains_only_redacted_identifiers_and_codes() {
        let mut ledger = ledger();
        let open = open(1, 7, 11, "calculator");
        seal_ready(&mut ledger, &open);
        assert!(ledger.audit().entries().all(|entry| {
            entry.session_id == SessionId(11)
                && !matches!(entry.event, ReceiptAuditEvent::QueryDenied)
        }));
    }
}
