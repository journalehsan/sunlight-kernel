//! Session-bound, bounded conversational action orchestration.
//!
//! This module owns conversation lifecycle state only. Target resolution stays
//! in [`BoundedActionPlanner`], authorization/readiness stay in
//! [`TrustedActionFlow`], and dispatch stays behind [`TrustedLaunchAdapter`].

use core::fmt::Write;

use heapless::{Deque, String, Vec};
use sha2::{Digest, Sha256};

use crate::action_intent::{
    ActionDecision, ActionEvaluation, ActionIntent, ActionOperation, AuditId, IntentId,
    RequestedBy, SessionId,
};
use crate::confirmation::{
    AuthorityTime, ChallengeId, ConfirmationChallenge, ConfirmationGrant, ConfirmationNonce,
    ConfirmationResponse as AuthorityConfirmationResponse, ConfirmationResponseType,
    ConfirmationView, ResponseValidationContext, SessionAuthorization, SessionStatus,
};
use crate::executor::{
    ExecutionContext, ExecutionId, ExecutionResultCode, TrustedActionFlow, TrustedLaunchAdapter,
};
use crate::planner::{
    BoundedActionPlanner, ClarificationId, ClarificationRequest, ConversationId, PlannerContext,
    PlannerInput, PlannerRegistry, PlannerRequestId, PlannerResult, PlannerTargetKind,
    PlannerVersion, UnsupportedReason, MAX_LOCALE_LEN, MAX_PUBLIC_TEXT_LEN,
};
use crate::policy::{ConfirmationLevel, PolicyEngine, PolicyResult, PolicyVersion};
use crate::runtime_context::RuntimeContextSnapshot;

pub const DEFAULT_COORDINATOR_AUDIT_CAPACITY: usize = 96;
pub const DEFAULT_COORDINATOR_RECORD_CAPACITY: usize = 16;
pub const DEFAULT_COORDINATOR_REPLAY_CAPACITY: usize = 64;
pub const MAX_RESPONSE_TEXT_LEN: usize = 256;
pub const MAX_PUBLIC_CHOICES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CoordinatorActionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatorState {
    Idle,
    Planning,
    AwaitingClarification,
    EvaluatingPolicy,
    AwaitingConfirmation,
    PreparingExecution,
    Dispatching,
    Completed,
    Rejected,
    Cancelled,
    Expired,
    Invalidated,
}

impl CoordinatorState {
    pub const fn is_pending(self) -> bool {
        matches!(
            self,
            Self::AwaitingClarification | Self::AwaitingConfirmation
        )
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Rejected | Self::Cancelled | Self::Expired | Self::Invalidated
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicActionStatus {
    None,
    Proposed,
    ClarificationRequired,
    ConfirmationRequired,
    Completed,
    Rejected,
    Unsupported,
    Cancelled,
    Expired,
    Invalidated,
    AlreadyPending,
    InvalidInput,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicReasonCode {
    None,
    ClarificationNeeded,
    ConfirmationNeeded,
    DispatchAccepted,
    DispatchFailed,
    TargetNotFound,
    TargetUnavailable,
    PolicyDenied,
    Unsupported,
    InvalidInput,
    InvalidSelection,
    RejectedByUser,
    CancelledByUser,
    Expired,
    SessionEnded,
    SessionChanged,
    PolicyChanged,
    RuntimeChanged,
    ApplicationRegistryChanged,
    SettingsRegistryChanged,
    ReplayRejected,
    ActionAlreadyPending,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionChoiceView {
    pub choice_id: u8,
    pub label: String<MAX_PUBLIC_TEXT_LEN>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionResponseView {
    pub status: PublicActionStatus,
    pub title: String<MAX_RESPONSE_TEXT_LEN>,
    pub message: String<MAX_RESPONSE_TEXT_LEN>,
    pub action_summary: String<MAX_RESPONSE_TEXT_LEN>,
    pub target_display_name: String<MAX_PUBLIC_TEXT_LEN>,
    pub available_choices: Vec<ActionChoiceView, MAX_PUBLIC_CHOICES>,
    pub confirmation_level: Option<ConfirmationLevel>,
    pub expires_at: Option<u64>,
    pub recoverable: bool,
    pub public_reason_code: PublicReasonCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatorResult {
    NoAction(ActionResponseView),
    ActionProposed(ActionResponseView),
    ClarificationRequired(ActionResponseView),
    ConfirmationRequired {
        response: ActionResponseView,
        confirmation: ConfirmationView,
    },
    ActionDispatched(ActionResponseView),
    ActionCompleted(ActionResponseView),
    ActionRejected(ActionResponseView),
    ActionUnsupported(ActionResponseView),
    ActionCancelled(ActionResponseView),
    ActionExpired(ActionResponseView),
    ActionInvalidated(ActionResponseView),
    ActionAlreadyPending(ActionResponseView),
    InvalidInput(ActionResponseView),
    Unknown(ActionResponseView),
}

impl CoordinatorResult {
    pub fn response(&self) -> &ActionResponseView {
        match self {
            Self::NoAction(view)
            | Self::ActionProposed(view)
            | Self::ClarificationRequired(view)
            | Self::ActionDispatched(view)
            | Self::ActionCompleted(view)
            | Self::ActionRejected(view)
            | Self::ActionUnsupported(view)
            | Self::ActionCancelled(view)
            | Self::ActionExpired(view)
            | Self::ActionInvalidated(view)
            | Self::ActionAlreadyPending(view)
            | Self::InvalidInput(view)
            | Self::Unknown(view) => view,
            Self::ConfirmationRequired { response, .. } => response,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClarificationResponse {
    pub input_id: u64,
    pub action_id: CoordinatorActionId,
    pub clarification_id: ClarificationId,
    pub choice_id: u8,
    pub conversation_id: ConversationId,
    pub session_id: SessionId,
    pub requester: RequestedBy,
    pub submitted_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundConfirmationResponse {
    pub input_id: u64,
    pub action_id: CoordinatorActionId,
    pub response: AuthorityConfirmationResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelPendingAction {
    pub input_id: u64,
    pub action_id: CoordinatorActionId,
    pub conversation_id: ConversationId,
    pub session_id: SessionId,
    pub requester: RequestedBy,
    pub submitted_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryPendingAction {
    pub conversation_id: ConversationId,
    pub session_id: SessionId,
    pub requester: RequestedBy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionEndedInput {
    pub input_id: u64,
    pub session_id: SessionId,
    pub submitted_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeInvalidation {
    pub input_id: u64,
    pub runtime_snapshot_generation: u64,
    pub policy_version: PolicyVersion,
    pub application_registry_generation: u64,
    pub settings_registry_generation: u64,
    pub submitted_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatorInput {
    UserRequest(PlannerInput),
    ClarificationResponse(ClarificationResponse),
    ConfirmationResponse(BoundConfirmationResponse),
    CancelPendingAction(CancelPendingAction),
    QueryPendingAction(QueryPendingAction),
    SessionEnded(SessionEndedInput),
    RuntimeInvalidated(RuntimeInvalidation),
}

pub struct CoordinatorContext<'a> {
    pub conversation_id: ConversationId,
    pub runtime: &'a RuntimeContextSnapshot,
    pub policy: &'a PolicyEngine,
    pub session: SessionAuthorization,
    pub requester: RequestedBy,
    pub now: AuthorityTime,
    pub application_registry_generation: u64,
    pub settings_registry_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoordinatorConfig {
    pub clarification_ttl_ms: u64,
    pub confirmation_ttl_ms: u64,
    pub ready_ttl_ms: u64,
    pub record_retention_ms: u64,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            clarification_ttl_ms: 30_000,
            confirmation_ttl_ms: 30_000,
            ready_ttl_ms: 5_000,
            record_retention_ms: 300_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionConversationRecord {
    coordinator_action_id: CoordinatorActionId,
    conversation_id: ConversationId,
    session_id: SessionId,
    requester: RequestedBy,
    original_request_id: PlannerRequestId,
    current_state: CoordinatorState,
    planner_version: PlannerVersion,
    policy_version: PolicyVersion,
    runtime_snapshot_generation: u64,
    application_registry_generation: u64,
    settings_registry_generation: u64,
    intent_id: Option<IntentId>,
    challenge_id: Option<ChallengeId>,
    readiness_id: Option<AuditId>,
    execution_id: Option<ExecutionId>,
    creation_time: u64,
    update_time: u64,
    expiry_time: u64,
    public_outcome_code: PublicReasonCode,
    bounded_audit_reference: u64,
}

impl ActionConversationRecord {
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
    pub const fn current_state(&self) -> CoordinatorState {
        self.current_state
    }
    pub const fn planner_version(&self) -> PlannerVersion {
        self.planner_version
    }
    pub const fn policy_version(&self) -> PolicyVersion {
        self.policy_version
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
    pub const fn intent_id(&self) -> Option<IntentId> {
        self.intent_id
    }
    pub const fn challenge_id(&self) -> Option<ChallengeId> {
        self.challenge_id
    }
    pub const fn readiness_id(&self) -> Option<AuditId> {
        self.readiness_id
    }
    pub const fn execution_id(&self) -> Option<ExecutionId> {
        self.execution_id
    }
    pub const fn creation_time(&self) -> u64 {
        self.creation_time
    }
    pub const fn update_time(&self) -> u64 {
        self.update_time
    }
    pub const fn expiry_time(&self) -> u64 {
        self.expiry_time
    }
    pub const fn public_outcome_code(&self) -> PublicReasonCode {
        self.public_outcome_code
    }
    pub const fn bounded_audit_reference(&self) -> u64 {
        self.bounded_audit_reference
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatorAuditEvent {
    FlowCreated,
    PlannerResultReceived,
    ClarificationIssued,
    ClarificationAccepted,
    ClarificationRejected,
    PolicyResultReceived,
    ConfirmationChallengeIssued,
    ConfirmationResponseAccepted,
    ConfirmationResponseRejected,
    ReadinessProduced,
    DispatchAttempted,
    ExecutionResultReceived,
    Cancellation,
    Expiry,
    Invalidation,
    ReplayRejected,
    FinalState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoordinatorAuditEntry {
    pub audit_reference: u64,
    pub action_id: CoordinatorActionId,
    pub conversation_id: ConversationId,
    pub state: CoordinatorState,
    pub operation: Option<ActionOperation>,
    pub event: CoordinatorAuditEvent,
    pub public_result: PublicReasonCode,
    pub timestamp: u64,
}

pub struct CoordinatorAuditLog<const N: usize = DEFAULT_COORDINATOR_AUDIT_CAPACITY> {
    entries: Deque<CoordinatorAuditEntry, N>,
    evicted: u64,
}

impl<const N: usize> CoordinatorAuditLog<N> {
    pub const fn new() -> Self {
        Self {
            entries: Deque::new(),
            evicted: 0,
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = &CoordinatorAuditEntry> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub const fn evicted(&self) -> u64 {
        self.evicted
    }

    fn push(&mut self, entry: CoordinatorAuditEntry) {
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

impl<const N: usize> Default for CoordinatorAuditLog<N> {
    fn default() -> Self {
        Self::new()
    }
}

struct ActiveFlow {
    record: ActionConversationRecord,
    locale: String<MAX_LOCALE_LEN>,
    operation: Option<ActionOperation>,
    target_display_name: String<MAX_PUBLIC_TEXT_LEN>,
    clarification: Option<ClarificationRequest>,
    intent: Option<ActionIntent>,
    decision: Option<ActionDecision>,
    challenge: Option<ConfirmationChallenge>,
    grant: Option<ConfirmationGrant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplayKey {
    Request(PlannerRequestId),
    Clarification(u64),
    Confirmation(u64),
    Cancellation(u64),
    SessionEnd(u64),
    Invalidation(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplayEntry {
    key: ReplayKey,
    result: CoordinatorResult,
}

pub struct ActionCoordinator<
    R: PlannerRegistry,
    A: TrustedLaunchAdapter,
    const AUDIT: usize = DEFAULT_COORDINATOR_AUDIT_CAPACITY,
    const RECORDS: usize = DEFAULT_COORDINATOR_RECORD_CAPACITY,
    const REPLAY: usize = DEFAULT_COORDINATOR_REPLAY_CAPACITY,
> {
    planner: BoundedActionPlanner<R>,
    flow: TrustedActionFlow<A>,
    config: CoordinatorConfig,
    active: Option<ActiveFlow>,
    records: Deque<ActionConversationRecord, RECORDS>,
    replay: Vec<ReplayEntry, REPLAY>,
    audit: CoordinatorAuditLog<AUDIT>,
    next_action_id: u64,
    next_audit_reference: u64,
    entropy_seed: [u8; 32],
}

impl<
        R: PlannerRegistry,
        A: TrustedLaunchAdapter,
        const AUDIT: usize,
        const RECORDS: usize,
        const REPLAY: usize,
    > ActionCoordinator<R, A, AUDIT, RECORDS, REPLAY>
{
    pub fn new(
        registry: R,
        adapter: A,
        policy: PolicyEngine,
        config: CoordinatorConfig,
        entropy_seed: [u8; 32],
    ) -> Self {
        Self {
            planner: BoundedActionPlanner::new(registry)
                .with_clarification_ttl(config.clarification_ttl_ms),
            flow: TrustedActionFlow::with_policy(adapter, config.ready_ttl_ms, policy),
            config,
            active: None,
            records: Deque::new(),
            replay: Vec::new(),
            audit: CoordinatorAuditLog::new(),
            next_action_id: 1,
            next_audit_reference: 1,
            entropy_seed,
        }
    }

    pub fn active_record(&self) -> Option<&ActionConversationRecord> {
        self.active.as_ref().map(|active| &active.record)
    }

    /// Opaque binding for the presentation controller; candidates remain
    /// exposed only as coordinator-local numeric choices.
    pub fn active_clarification_id(&self) -> Option<ClarificationId> {
        self.active
            .as_ref()?
            .clarification
            .as_ref()
            .map(ClarificationRequest::clarification_id)
    }

    pub fn records(&self) -> impl Iterator<Item = &ActionConversationRecord> {
        self.records.iter()
    }

    pub const fn audit(&self) -> &CoordinatorAuditLog<AUDIT> {
        &self.audit
    }

    pub fn handle(
        &mut self,
        input: CoordinatorInput,
        context: CoordinatorContext<'_>,
    ) -> CoordinatorResult {
        self.expire_records(context.now.0);
        let key = replay_key(&input);
        if let Some(key) = key {
            if let Some(entry) = self.replay.iter().find(|entry| entry.key == key) {
                return entry.result.clone();
            }
            if self.replay.is_full() {
                self.audit_active(
                    CoordinatorAuditEvent::ReplayRejected,
                    PublicReasonCode::ReplayRejected,
                    context.now.0,
                );
                return CoordinatorResult::Unknown(localized_view(
                    "en",
                    PublicActionStatus::Unknown,
                    PublicReasonCode::ReplayRejected,
                    "",
                    None,
                    false,
                ));
            }
        }

        if !matches!(
            input,
            CoordinatorInput::SessionEnded(_) | CoordinatorInput::RuntimeInvalidated(_)
        ) {
            if let Some(result) = self.invalidate_if_environment_changed(&context) {
                self.cache(key, &result);
                return result;
            }
            if let Some(result) = self.expire_pending(context.now.0, &context) {
                self.cache(key, &result);
                return result;
            }
        }

        let result = match input {
            CoordinatorInput::UserRequest(input) => self.user_request(input, &context),
            CoordinatorInput::ClarificationResponse(response) => {
                self.clarification_response(response, &context)
            }
            CoordinatorInput::ConfirmationResponse(response) => {
                self.confirmation_response(response, &context)
            }
            CoordinatorInput::CancelPendingAction(cancel) => self.cancel(cancel, &context),
            CoordinatorInput::QueryPendingAction(query) => self.query(query),
            CoordinatorInput::SessionEnded(ended) => self.session_ended(ended, &context),
            CoordinatorInput::RuntimeInvalidated(invalidation) => {
                self.runtime_invalidated(invalidation, &context)
            }
        };
        self.cache(key, &result);
        result
    }

    fn user_request(
        &mut self,
        input: PlannerInput,
        context: &CoordinatorContext<'_>,
    ) -> CoordinatorResult {
        if self.active.is_some() && is_exact_cancellation(input.user_text(), input.locale()) {
            let action_id = self.active.as_ref().unwrap().record.coordinator_action_id;
            return self.cancel(
                CancelPendingAction {
                    input_id: input.request_id().0,
                    action_id,
                    conversation_id: input.conversation_id(),
                    session_id: input.session_id(),
                    requester: input.requester(),
                    submitted_at: input.timestamp(),
                },
                context,
            );
        }

        let planner_result = self.planner.plan(
            &input,
            PlannerContext {
                runtime_snapshot_generation: context.runtime.generation,
                active_session_id: context.session.session_id(),
                now: context.now.0,
            },
        );

        if self.active.is_some() {
            return match planner_result {
                PlannerResult::NoAction => CoordinatorResult::NoAction(localized_view(
                    input.locale(),
                    PublicActionStatus::None,
                    PublicReasonCode::None,
                    "",
                    None,
                    true,
                )),
                PlannerResult::InvalidInput(_) => CoordinatorResult::InvalidInput(localized_view(
                    input.locale(),
                    PublicActionStatus::InvalidInput,
                    PublicReasonCode::InvalidInput,
                    "",
                    None,
                    true,
                )),
                PlannerResult::Unsupported(_) | PlannerResult::Unknown => {
                    CoordinatorResult::ActionUnsupported(localized_view(
                        input.locale(),
                        PublicActionStatus::Unsupported,
                        PublicReasonCode::Unsupported,
                        "",
                        None,
                        true,
                    ))
                }
                PlannerResult::Proposed(_) | PlannerResult::NeedsClarification(_) => {
                    CoordinatorResult::ActionAlreadyPending(localized_view(
                        input.locale(),
                        PublicActionStatus::AlreadyPending,
                        PublicReasonCode::ActionAlreadyPending,
                        "",
                        self.active.as_ref().map(|active| active.record.expiry_time),
                        true,
                    ))
                }
            };
        }

        match planner_result {
            PlannerResult::NoAction => CoordinatorResult::NoAction(localized_view(
                input.locale(),
                PublicActionStatus::None,
                PublicReasonCode::None,
                "",
                None,
                false,
            )),
            PlannerResult::InvalidInput(_) => CoordinatorResult::InvalidInput(localized_view(
                input.locale(),
                PublicActionStatus::InvalidInput,
                PublicReasonCode::InvalidInput,
                "",
                None,
                false,
            )),
            PlannerResult::Unsupported(reason) => {
                self.create_active(&input, context, context.now.0);
                self.audit_active(
                    CoordinatorAuditEvent::PlannerResultReceived,
                    PublicReasonCode::Unsupported,
                    context.now.0,
                );
                let public_reason = unsupported_reason(reason);
                let result = CoordinatorResult::ActionUnsupported(localized_view(
                    input.locale(),
                    PublicActionStatus::Unsupported,
                    public_reason,
                    "",
                    None,
                    false,
                ));
                self.finish(CoordinatorState::Rejected, public_reason, context.now.0);
                result
            }
            PlannerResult::Unknown => CoordinatorResult::Unknown(localized_view(
                input.locale(),
                PublicActionStatus::Unknown,
                PublicReasonCode::Unknown,
                "",
                None,
                false,
            )),
            PlannerResult::NeedsClarification(clarification) => {
                self.create_active(&input, context, clarification.expires_at());
                let active = self.active.as_mut().unwrap();
                active.record.planner_version = clarification.planner_version();
                active.record.current_state = CoordinatorState::AwaitingClarification;
                active.record.update_time = context.now.0;
                active.operation =
                    clarification
                        .candidates()
                        .first()
                        .map(|candidate| match candidate.kind() {
                            PlannerTargetKind::Application => ActionOperation::OpenApplication,
                            PlannerTargetKind::SettingsPage => ActionOperation::OpenSettingsPage,
                        });
                active.clarification = Some(clarification.clone());
                self.audit_active(
                    CoordinatorAuditEvent::PlannerResultReceived,
                    PublicReasonCode::ClarificationNeeded,
                    context.now.0,
                );
                self.audit_active(
                    CoordinatorAuditEvent::ClarificationIssued,
                    PublicReasonCode::ClarificationNeeded,
                    context.now.0,
                );
                CoordinatorResult::ClarificationRequired(clarification_view(
                    input.locale(),
                    &clarification,
                ))
            }
            PlannerResult::Proposed(draft) => {
                self.create_active(&input, context, context.now.0 + self.config.ready_ttl_ms);
                {
                    let active = self.active.as_mut().unwrap();
                    active.operation = Some(draft.operation());
                    active.record.planner_version = draft.planner_version();
                    active.target_display_name = target_display(draft.public_interpretation());
                    let intent_id =
                        intent_id(active.record.coordinator_action_id, input.request_id());
                    let intent = draft.build_action_intent(intent_id);
                    active.record.intent_id = Some(intent_id);
                    active.intent = Some(intent);
                }
                self.audit_active(
                    CoordinatorAuditEvent::PlannerResultReceived,
                    PublicReasonCode::None,
                    context.now.0,
                );
                self.evaluate_and_continue(context)
            }
        }
    }

    fn clarification_response(
        &mut self,
        response: ClarificationResponse,
        context: &CoordinatorContext<'_>,
    ) -> CoordinatorResult {
        let Some(active) = self.active.as_ref() else {
            return invalid_selection("en");
        };
        let locale = active.locale.clone();
        if active.record.current_state != CoordinatorState::AwaitingClarification
            || active.record.coordinator_action_id != response.action_id
            || active.record.conversation_id != response.conversation_id
            || active.record.session_id != response.session_id
            || active.record.requester != response.requester
            || context.conversation_id != response.conversation_id
            || context.session.session_id() != response.session_id
            || context.requester != response.requester
        {
            self.audit_active(
                CoordinatorAuditEvent::ClarificationRejected,
                PublicReasonCode::InvalidSelection,
                context.now.0,
            );
            return invalid_selection(locale.as_str());
        }
        let clarification = active.clarification.as_ref().unwrap();
        if clarification.clarification_id() != response.clarification_id
            || response.submitted_at > clarification.expires_at()
            || response.submitted_at > context.now.0
        {
            self.audit_active(
                CoordinatorAuditEvent::ClarificationRejected,
                PublicReasonCode::InvalidSelection,
                context.now.0,
            );
            return invalid_selection(locale.as_str());
        }
        let Some(candidate) = clarification.candidates().get(response.choice_id as usize) else {
            self.audit_active(
                CoordinatorAuditEvent::ClarificationRejected,
                PublicReasonCode::InvalidSelection,
                context.now.0,
            );
            return invalid_selection(locale.as_str());
        };
        let selected = candidate.canonical_id().as_str();
        let planner_input = PlannerInput::clarification_response(
            response.input_id,
            response.conversation_id.0,
            response.session_id,
            response.requester,
            locale.as_str(),
            selected,
            context.runtime.generation,
            response.submitted_at,
            response.clarification_id,
        );
        let result = self.planner.plan(
            &planner_input,
            PlannerContext {
                runtime_snapshot_generation: context.runtime.generation,
                active_session_id: context.session.session_id(),
                now: context.now.0,
            },
        );
        match result {
            PlannerResult::Proposed(draft) => {
                {
                    let active = self.active.as_mut().unwrap();
                    active.record.current_state = CoordinatorState::Planning;
                    active.record.update_time = context.now.0;
                    active.record.expiry_time = context.now.0 + self.config.ready_ttl_ms;
                    active.operation = Some(draft.operation());
                    active.target_display_name = target_display(draft.public_interpretation());
                    active.clarification = None;
                    let id = intent_id(
                        active.record.coordinator_action_id,
                        PlannerRequestId(response.input_id),
                    );
                    active.record.intent_id = Some(id);
                    active.intent = Some(draft.build_action_intent(id));
                }
                self.audit_active(
                    CoordinatorAuditEvent::ClarificationAccepted,
                    PublicReasonCode::None,
                    context.now.0,
                );
                self.evaluate_and_continue(context)
            }
            PlannerResult::Unsupported(UnsupportedReason::ClarificationExpired) => {
                self.expire_active(context.now.0, locale.as_str())
            }
            _ => {
                self.audit_active(
                    CoordinatorAuditEvent::ClarificationRejected,
                    PublicReasonCode::InvalidSelection,
                    context.now.0,
                );
                invalid_selection(locale.as_str())
            }
        }
    }

    fn evaluate_and_continue(&mut self, context: &CoordinatorContext<'_>) -> CoordinatorResult {
        {
            let active = self.active.as_mut().unwrap();
            active.record.current_state = CoordinatorState::EvaluatingPolicy;
            active.record.update_time = context.now.0;
        }
        let evaluation = {
            let intent = self.active.as_ref().unwrap().intent.as_ref().unwrap();
            self.flow.evaluate_action(intent, context.runtime)
        };
        self.audit_active(
            CoordinatorAuditEvent::PolicyResultReceived,
            PublicReasonCode::None,
            context.now.0,
        );
        match evaluation {
            ActionEvaluation::Rejected { .. } => {
                let locale = self.active.as_ref().unwrap().locale.clone();
                let result = CoordinatorResult::ActionRejected(localized_view(
                    locale.as_str(),
                    PublicActionStatus::Rejected,
                    PublicReasonCode::PolicyDenied,
                    "",
                    None,
                    false,
                ));
                self.finish(
                    CoordinatorState::Rejected,
                    PublicReasonCode::PolicyDenied,
                    context.now.0,
                );
                result
            }
            ActionEvaluation::Decided(decision) => {
                let policy_result = decision.result();
                self.active.as_mut().unwrap().decision = Some(decision);
                match policy_result {
                    PolicyResult::Allowed => self.prepare_and_dispatch(context),
                    PolicyResult::ConfirmationRequired => self.issue_confirmation(context),
                    PolicyResult::Denied | PolicyResult::Unknown => {
                        let locale = self.active.as_ref().unwrap().locale.clone();
                        let result = CoordinatorResult::ActionRejected(localized_view(
                            locale.as_str(),
                            PublicActionStatus::Rejected,
                            PublicReasonCode::PolicyDenied,
                            "",
                            None,
                            false,
                        ));
                        self.finish(
                            CoordinatorState::Rejected,
                            PublicReasonCode::PolicyDenied,
                            context.now.0,
                        );
                        result
                    }
                }
            }
        }
    }

    fn issue_confirmation(&mut self, context: &CoordinatorContext<'_>) -> CoordinatorResult {
        let action_id = self.active.as_ref().unwrap().record.coordinator_action_id;
        let (challenge_id, nonce) = self.challenge_material(action_id, context.now.0);
        let expires = context
            .now
            .0
            .saturating_add(self.config.confirmation_ttl_ms);
        let challenge = {
            let active = self.active.as_ref().unwrap();
            self.flow.create_confirmation_challenge(
                active.intent.as_ref().unwrap(),
                active.decision.as_ref().unwrap(),
                challenge_id,
                nonce,
                context.session.responder(),
                context.now,
                AuthorityTime(expires),
            )
        };
        let Ok(challenge) = challenge else {
            return self.invalidate_active(PublicReasonCode::Unknown, context.now.0, "en");
        };
        {
            let active = self.active.as_mut().unwrap();
            active.record.challenge_id = Some(challenge_id);
            active.record.current_state = CoordinatorState::AwaitingConfirmation;
            active.record.update_time = context.now.0;
            active.record.expiry_time = expires;
            active.challenge = Some(challenge.clone());
        }
        self.audit_active(
            CoordinatorAuditEvent::ConfirmationChallengeIssued,
            PublicReasonCode::ConfirmationNeeded,
            context.now.0,
        );
        let active = self.active.as_ref().unwrap();
        let mut response = localized_view(
            active.locale.as_str(),
            PublicActionStatus::ConfirmationRequired,
            PublicReasonCode::ConfirmationNeeded,
            active.target_display_name.as_str(),
            Some(expires),
            true,
        );
        response.confirmation_level = Some(challenge.confirmation_level());
        CoordinatorResult::ConfirmationRequired {
            response,
            confirmation: challenge.view(),
        }
    }

    fn confirmation_response(
        &mut self,
        bound: BoundConfirmationResponse,
        context: &CoordinatorContext<'_>,
    ) -> CoordinatorResult {
        let Some(active) = self.active.as_ref() else {
            return invalid_selection("en");
        };
        let locale = active.locale.clone();
        let response = &bound.response;
        if active.record.current_state != CoordinatorState::AwaitingConfirmation
            || active.record.coordinator_action_id != bound.action_id
            || active.record.challenge_id != Some(response.challenge_id())
            || active.record.session_id != response.session_id()
            || context.session.session_id() != response.session_id()
            || context.session.responder() != response.responder()
            || context.requester != active.record.requester
        {
            self.audit_active(
                CoordinatorAuditEvent::ConfirmationResponseRejected,
                PublicReasonCode::InvalidInput,
                context.now.0,
            );
            return invalid_selection(locale.as_str());
        }
        let accepted = {
            let active = self.active.as_ref().unwrap();
            self.flow.accept_confirmation(
                response,
                ResponseValidationContext::new(
                    active.intent.as_ref().unwrap(),
                    context.runtime,
                    context.session,
                ),
            )
        };
        match accepted {
            Ok(Some(grant)) => {
                self.active.as_mut().unwrap().grant = Some(grant);
                self.audit_active(
                    CoordinatorAuditEvent::ConfirmationResponseAccepted,
                    PublicReasonCode::None,
                    context.now.0,
                );
                self.prepare_and_dispatch(context)
            }
            Ok(None) => {
                let (state, reason, result) = match response.response_type() {
                    ConfirmationResponseType::Cancelled => (
                        CoordinatorState::Cancelled,
                        PublicReasonCode::CancelledByUser,
                        CoordinatorResult::ActionCancelled(localized_view(
                            locale.as_str(),
                            PublicActionStatus::Cancelled,
                            PublicReasonCode::CancelledByUser,
                            "",
                            None,
                            false,
                        )),
                    ),
                    ConfirmationResponseType::Expired => (
                        CoordinatorState::Expired,
                        PublicReasonCode::Expired,
                        CoordinatorResult::ActionExpired(localized_view(
                            locale.as_str(),
                            PublicActionStatus::Expired,
                            PublicReasonCode::Expired,
                            "",
                            None,
                            false,
                        )),
                    ),
                    _ => (
                        CoordinatorState::Rejected,
                        PublicReasonCode::RejectedByUser,
                        CoordinatorResult::ActionRejected(localized_view(
                            locale.as_str(),
                            PublicActionStatus::Rejected,
                            PublicReasonCode::RejectedByUser,
                            "",
                            None,
                            false,
                        )),
                    ),
                };
                self.audit_active(
                    CoordinatorAuditEvent::ConfirmationResponseAccepted,
                    reason,
                    context.now.0,
                );
                self.finish(state, reason, context.now.0);
                result
            }
            Err(_) if response.submitted_at().0 > active.record.expiry_time => {
                self.expire_active(context.now.0, locale.as_str())
            }
            Err(_) => {
                self.audit_active(
                    CoordinatorAuditEvent::ConfirmationResponseRejected,
                    PublicReasonCode::InvalidInput,
                    context.now.0,
                );
                self.invalidate_active(
                    PublicReasonCode::InvalidInput,
                    context.now.0,
                    locale.as_str(),
                )
            }
        }
    }

    fn prepare_and_dispatch(&mut self, context: &CoordinatorContext<'_>) -> CoordinatorResult {
        {
            let active = self.active.as_mut().unwrap();
            active.record.current_state = CoordinatorState::PreparingExecution;
            active.record.update_time = context.now.0;
            active.record.expiry_time = context.now.0.saturating_add(self.config.ready_ttl_ms);
        }
        let ready = {
            let active = self.active.as_ref().unwrap();
            self.flow.prepare_for_execution(
                active.intent.as_ref().unwrap(),
                active.decision.as_ref().unwrap(),
                active.grant.as_ref(),
                context.runtime,
                context.session,
                context.now,
            )
        };
        let Ok(ready) = ready else {
            let locale = self.active.as_ref().unwrap().locale.clone();
            return self.invalidate_active(
                PublicReasonCode::RuntimeChanged,
                context.now.0,
                locale.as_str(),
            );
        };
        {
            let active = self.active.as_mut().unwrap();
            active.record.readiness_id = Some(ready.audit_id());
            active.record.current_state = CoordinatorState::Dispatching;
            active.record.update_time = context.now.0;
        }
        self.audit_active(
            CoordinatorAuditEvent::ReadinessProduced,
            PublicReasonCode::None,
            context.now.0,
        );
        self.audit_active(
            CoordinatorAuditEvent::DispatchAttempted,
            PublicReasonCode::None,
            context.now.0,
        );
        let execution = self.flow.execute_ready_action(
            ready,
            &ExecutionContext::new(
                context.runtime,
                context.policy,
                context.session,
                context.requester,
                context.now,
            ),
        );
        let locale = self.active.as_ref().unwrap().locale.clone();
        let target = self.active.as_ref().unwrap().target_display_name.clone();
        self.active.as_mut().unwrap().record.execution_id = Some(execution.execution_id());
        self.audit_active(
            CoordinatorAuditEvent::ExecutionResultReceived,
            execution_reason(execution.code()),
            context.now.0,
        );
        if execution.code() == ExecutionResultCode::Succeeded {
            let result = CoordinatorResult::ActionCompleted(localized_view(
                locale.as_str(),
                PublicActionStatus::Completed,
                PublicReasonCode::DispatchAccepted,
                target.as_str(),
                None,
                false,
            ));
            self.finish(
                CoordinatorState::Completed,
                PublicReasonCode::DispatchAccepted,
                context.now.0,
            );
            result
        } else {
            let reason = execution_reason(execution.code());
            let result = CoordinatorResult::ActionRejected(localized_view(
                locale.as_str(),
                PublicActionStatus::Rejected,
                reason,
                target.as_str(),
                None,
                false,
            ));
            self.finish(CoordinatorState::Rejected, reason, context.now.0);
            result
        }
    }

    fn cancel(
        &mut self,
        cancel: CancelPendingAction,
        context: &CoordinatorContext<'_>,
    ) -> CoordinatorResult {
        let Some(active) = self.active.as_ref() else {
            return invalid_selection("en");
        };
        let locale = active.locale.clone();
        if !active.record.current_state.is_pending()
            || active.record.coordinator_action_id != cancel.action_id
            || active.record.conversation_id != cancel.conversation_id
            || active.record.session_id != cancel.session_id
            || active.record.requester != cancel.requester
            || context.conversation_id != cancel.conversation_id
            || context.session.session_id() != cancel.session_id
        {
            return invalid_selection(locale.as_str());
        }
        if active.record.current_state == CoordinatorState::AwaitingConfirmation {
            let challenge_id = active.record.challenge_id.unwrap();
            let cancellation = AuthorityConfirmationResponse::new(
                challenge_id,
                cancel.session_id,
                context.session.responder(),
                ConfirmationResponseType::Cancelled,
                AuthorityTime(cancel.submitted_at),
            );
            let intent = active.intent.as_ref().unwrap();
            let _ = self.flow.accept_confirmation(
                &cancellation,
                ResponseValidationContext::new(intent, context.runtime, context.session),
            );
        }
        self.audit_active(
            CoordinatorAuditEvent::Cancellation,
            PublicReasonCode::CancelledByUser,
            context.now.0,
        );
        let result = CoordinatorResult::ActionCancelled(localized_view(
            locale.as_str(),
            PublicActionStatus::Cancelled,
            PublicReasonCode::CancelledByUser,
            "",
            None,
            false,
        ));
        self.finish(
            CoordinatorState::Cancelled,
            PublicReasonCode::CancelledByUser,
            context.now.0,
        );
        result
    }

    fn query(&self, query: QueryPendingAction) -> CoordinatorResult {
        let Some(active) = self.active.as_ref() else {
            return CoordinatorResult::NoAction(localized_view(
                "en",
                PublicActionStatus::None,
                PublicReasonCode::None,
                "",
                None,
                false,
            ));
        };
        if active.record.conversation_id != query.conversation_id
            || active.record.session_id != query.session_id
            || active.record.requester != query.requester
        {
            return invalid_selection(active.locale.as_str());
        }
        match active.record.current_state {
            CoordinatorState::AwaitingClarification => {
                CoordinatorResult::ClarificationRequired(clarification_view(
                    active.locale.as_str(),
                    active.clarification.as_ref().unwrap(),
                ))
            }
            CoordinatorState::AwaitingConfirmation => CoordinatorResult::ConfirmationRequired {
                response: localized_view(
                    active.locale.as_str(),
                    PublicActionStatus::ConfirmationRequired,
                    PublicReasonCode::ConfirmationNeeded,
                    active.target_display_name.as_str(),
                    Some(active.record.expiry_time),
                    true,
                ),
                confirmation: active.challenge.as_ref().unwrap().view(),
            },
            _ => CoordinatorResult::ActionProposed(localized_view(
                active.locale.as_str(),
                PublicActionStatus::Proposed,
                PublicReasonCode::None,
                active.target_display_name.as_str(),
                Some(active.record.expiry_time),
                true,
            )),
        }
    }

    fn session_ended(
        &mut self,
        ended: SessionEndedInput,
        context: &CoordinatorContext<'_>,
    ) -> CoordinatorResult {
        let Some(active) = self.active.as_ref() else {
            return CoordinatorResult::NoAction(localized_view(
                "en",
                PublicActionStatus::None,
                PublicReasonCode::None,
                "",
                None,
                false,
            ));
        };
        if active.record.session_id != ended.session_id {
            return invalid_selection(active.locale.as_str());
        }
        let locale = active.locale.clone();
        self.terminate_confirmation(
            ConfirmationResponseType::Invalid,
            ended.submitted_at,
            context,
        );
        self.audit_active(
            CoordinatorAuditEvent::Invalidation,
            PublicReasonCode::SessionEnded,
            ended.submitted_at,
        );
        self.invalidate_active(
            PublicReasonCode::SessionEnded,
            ended.submitted_at,
            locale.as_str(),
        )
    }

    fn runtime_invalidated(
        &mut self,
        invalidation: RuntimeInvalidation,
        context: &CoordinatorContext<'_>,
    ) -> CoordinatorResult {
        let Some(active) = self.active.as_ref() else {
            return CoordinatorResult::NoAction(localized_view(
                "en",
                PublicActionStatus::None,
                PublicReasonCode::None,
                "",
                None,
                false,
            ));
        };
        let reason = environment_reason(
            &active.record,
            invalidation.runtime_snapshot_generation,
            invalidation.policy_version,
            invalidation.application_registry_generation,
            invalidation.settings_registry_generation,
            active.operation,
        );
        let Some(reason) = reason else {
            return self.query(QueryPendingAction {
                conversation_id: active.record.conversation_id,
                session_id: active.record.session_id,
                requester: active.record.requester,
            });
        };
        let locale = active.locale.clone();
        self.terminate_confirmation(
            ConfirmationResponseType::Invalid,
            invalidation.submitted_at,
            context,
        );
        self.audit_active(
            CoordinatorAuditEvent::Invalidation,
            reason,
            invalidation.submitted_at,
        );
        self.invalidate_active(reason, invalidation.submitted_at, locale.as_str())
    }

    fn create_active(
        &mut self,
        input: &PlannerInput,
        context: &CoordinatorContext<'_>,
        expiry_time: u64,
    ) {
        let action_id = CoordinatorActionId(self.next_action_id);
        self.next_action_id = self.next_action_id.saturating_add(1);
        let audit_reference = self.next_audit_reference;
        self.next_audit_reference = self.next_audit_reference.saturating_add(1);
        let locale = String::try_from(input.locale()).unwrap_or_default();
        self.active = Some(ActiveFlow {
            record: ActionConversationRecord {
                coordinator_action_id: action_id,
                conversation_id: input.conversation_id(),
                session_id: input.session_id(),
                requester: input.requester(),
                original_request_id: input.request_id(),
                current_state: CoordinatorState::Planning,
                planner_version: crate::planner::PLANNER_V1,
                policy_version: context.policy.version(),
                runtime_snapshot_generation: context.runtime.generation,
                application_registry_generation: context.application_registry_generation,
                settings_registry_generation: context.settings_registry_generation,
                intent_id: None,
                challenge_id: None,
                readiness_id: None,
                execution_id: None,
                creation_time: context.now.0,
                update_time: context.now.0,
                expiry_time,
                public_outcome_code: PublicReasonCode::None,
                bounded_audit_reference: audit_reference,
            },
            locale,
            operation: None,
            target_display_name: String::new(),
            clarification: None,
            intent: None,
            decision: None,
            challenge: None,
            grant: None,
        });
        self.audit_active(
            CoordinatorAuditEvent::FlowCreated,
            PublicReasonCode::None,
            context.now.0,
        );
    }

    fn finish(&mut self, state: CoordinatorState, reason: PublicReasonCode, now: u64) {
        if let Some(mut active) = self.active.take() {
            active.record.current_state = state;
            active.record.public_outcome_code = reason;
            active.record.update_time = now;
            active.record.expiry_time = now.saturating_add(self.config.record_retention_ms);
            self.audit.push(CoordinatorAuditEntry {
                audit_reference: active.record.bounded_audit_reference,
                action_id: active.record.coordinator_action_id,
                conversation_id: active.record.conversation_id,
                state,
                operation: active.operation,
                event: CoordinatorAuditEvent::FinalState,
                public_result: reason,
                timestamp: now,
            });
            if RECORDS > 0 {
                if self.records.is_full() {
                    let _ = self.records.pop_front();
                }
                let _ = self.records.push_back(active.record);
            }
        }
    }

    fn audit_active(&mut self, event: CoordinatorAuditEvent, reason: PublicReasonCode, now: u64) {
        if let Some(active) = self.active.as_ref() {
            self.audit.push(CoordinatorAuditEntry {
                audit_reference: active.record.bounded_audit_reference,
                action_id: active.record.coordinator_action_id,
                conversation_id: active.record.conversation_id,
                state: active.record.current_state,
                operation: active.operation,
                event,
                public_result: reason,
                timestamp: now,
            });
        }
    }

    fn invalidate_if_environment_changed(
        &mut self,
        context: &CoordinatorContext<'_>,
    ) -> Option<CoordinatorResult> {
        let active = self.active.as_ref()?;
        if active.record.conversation_id != context.conversation_id
            || active.record.session_id != context.session.session_id()
            || active.record.requester != context.requester
            || context.session.status() != SessionStatus::Active
        {
            let locale = active.locale.clone();
            return Some(self.invalidate_active(
                PublicReasonCode::SessionChanged,
                context.now.0,
                locale.as_str(),
            ));
        }
        let reason = environment_reason(
            &active.record,
            context.runtime.generation,
            context.policy.version(),
            context.application_registry_generation,
            context.settings_registry_generation,
            active.operation,
        )?;
        let locale = active.locale.clone();
        Some(self.invalidate_active(reason, context.now.0, locale.as_str()))
    }

    fn invalidate_active(
        &mut self,
        reason: PublicReasonCode,
        now: u64,
        locale: &str,
    ) -> CoordinatorResult {
        self.audit_active(CoordinatorAuditEvent::Invalidation, reason, now);
        let result = CoordinatorResult::ActionInvalidated(localized_view(
            locale,
            PublicActionStatus::Invalidated,
            reason,
            "",
            None,
            false,
        ));
        self.finish(CoordinatorState::Invalidated, reason, now);
        result
    }

    fn expire_pending(
        &mut self,
        now: u64,
        context: &CoordinatorContext<'_>,
    ) -> Option<CoordinatorResult> {
        let active = self.active.as_ref()?;
        if !active.record.current_state.is_pending() || now <= active.record.expiry_time {
            return None;
        }
        let locale = active.locale.clone();
        self.terminate_confirmation(ConfirmationResponseType::Expired, now, context);
        Some(self.expire_active(now, locale.as_str()))
    }

    fn terminate_confirmation(
        &mut self,
        response_type: ConfirmationResponseType,
        submitted_at: u64,
        context: &CoordinatorContext<'_>,
    ) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        if active.record.current_state != CoordinatorState::AwaitingConfirmation {
            return;
        }
        let response = AuthorityConfirmationResponse::new(
            active.record.challenge_id.unwrap(),
            active.record.session_id,
            context.session.responder(),
            response_type,
            AuthorityTime(submitted_at),
        );
        let _ = self.flow.accept_confirmation(
            &response,
            ResponseValidationContext::new(
                active.intent.as_ref().unwrap(),
                context.runtime,
                context.session,
            ),
        );
    }

    fn expire_active(&mut self, now: u64, locale: &str) -> CoordinatorResult {
        self.audit_active(
            CoordinatorAuditEvent::Expiry,
            PublicReasonCode::Expired,
            now,
        );
        let result = CoordinatorResult::ActionExpired(localized_view(
            locale,
            PublicActionStatus::Expired,
            PublicReasonCode::Expired,
            "",
            None,
            false,
        ));
        self.finish(CoordinatorState::Expired, PublicReasonCode::Expired, now);
        result
    }

    fn expire_records(&mut self, now: u64) {
        while self
            .records
            .front()
            .is_some_and(|record| now > record.expiry_time)
        {
            let _ = self.records.pop_front();
        }
    }

    fn cache(&mut self, key: Option<ReplayKey>, result: &CoordinatorResult) {
        if let Some(key) = key {
            let _ = self.replay.push(ReplayEntry {
                key,
                result: result.clone(),
            });
        }
    }

    fn challenge_material(
        &self,
        action_id: CoordinatorActionId,
        now: u64,
    ) -> (ChallengeId, ConfirmationNonce) {
        let mut hasher = Sha256::new();
        hasher.update(self.entropy_seed);
        hasher.update(action_id.0.to_le_bytes());
        hasher.update(now.to_le_bytes());
        let digest = hasher.finalize();
        let mut challenge = [0u8; 16];
        let mut nonce = [0u8; 16];
        challenge.copy_from_slice(&digest[..16]);
        nonce.copy_from_slice(&digest[16..]);
        (ChallengeId::new(challenge), ConfirmationNonce::new(nonce))
    }
}

fn replay_key(input: &CoordinatorInput) -> Option<ReplayKey> {
    match input {
        CoordinatorInput::UserRequest(input) => Some(ReplayKey::Request(input.request_id())),
        CoordinatorInput::ClarificationResponse(input) => {
            Some(ReplayKey::Clarification(input.input_id))
        }
        CoordinatorInput::ConfirmationResponse(input) => {
            Some(ReplayKey::Confirmation(input.input_id))
        }
        CoordinatorInput::CancelPendingAction(input) => {
            Some(ReplayKey::Cancellation(input.input_id))
        }
        CoordinatorInput::QueryPendingAction(_) => None,
        CoordinatorInput::SessionEnded(input) => Some(ReplayKey::SessionEnd(input.input_id)),
        CoordinatorInput::RuntimeInvalidated(input) => {
            Some(ReplayKey::Invalidation(input.input_id))
        }
    }
}

fn intent_id(action_id: CoordinatorActionId, request_id: PlannerRequestId) -> IntentId {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&action_id.0.to_le_bytes());
    bytes[8..].copy_from_slice(&request_id.0.to_le_bytes());
    IntentId::new(bytes)
}

fn target_display(interpretation: &str) -> String<MAX_PUBLIC_TEXT_LEN> {
    let value = interpretation
        .strip_prefix("Open application ")
        .or_else(|| interpretation.strip_prefix("Open settings page "))
        .unwrap_or(interpretation);
    String::try_from(value).unwrap_or_default()
}

fn clarification_view(locale: &str, clarification: &ClarificationRequest) -> ActionResponseView {
    let mut view = localized_view(
        locale,
        PublicActionStatus::ClarificationRequired,
        PublicReasonCode::ClarificationNeeded,
        "",
        Some(clarification.expires_at()),
        true,
    );
    for (index, candidate) in clarification.candidates().iter().enumerate() {
        let mut label = String::new();
        let _ = label.push_str(candidate.public_name());
        let _ = view.available_choices.push(ActionChoiceView {
            choice_id: index as u8,
            label,
        });
    }
    view
}

fn invalid_selection(locale: &str) -> CoordinatorResult {
    CoordinatorResult::InvalidInput(localized_view(
        locale,
        PublicActionStatus::InvalidInput,
        PublicReasonCode::InvalidSelection,
        "",
        None,
        true,
    ))
}

fn unsupported_reason(reason: UnsupportedReason) -> PublicReasonCode {
    match reason {
        UnsupportedReason::UnknownApplication | UnsupportedReason::UnknownSettingsPage => {
            PublicReasonCode::TargetNotFound
        }
        UnsupportedReason::ClarificationExpired => PublicReasonCode::Expired,
        UnsupportedReason::ClarificationReplay => PublicReasonCode::ReplayRejected,
        UnsupportedReason::ClarificationTargetInvalid
        | UnsupportedReason::ClarificationNotFound => PublicReasonCode::InvalidSelection,
        _ => PublicReasonCode::Unsupported,
    }
}

fn execution_reason(code: ExecutionResultCode) -> PublicReasonCode {
    match code {
        ExecutionResultCode::Succeeded => PublicReasonCode::DispatchAccepted,
        ExecutionResultCode::TargetNotFound => PublicReasonCode::TargetNotFound,
        ExecutionResultCode::TargetUnavailable => PublicReasonCode::TargetUnavailable,
        ExecutionResultCode::PolicyChanged => PublicReasonCode::PolicyChanged,
        ExecutionResultCode::RuntimeStale => PublicReasonCode::RuntimeChanged,
        ExecutionResultCode::ConfirmationExpired => PublicReasonCode::Expired,
        ExecutionResultCode::AlreadyConsumed => PublicReasonCode::ReplayRejected,
        ExecutionResultCode::DispatchFailed => PublicReasonCode::DispatchFailed,
        _ => PublicReasonCode::PolicyDenied,
    }
}

fn environment_reason(
    record: &ActionConversationRecord,
    runtime_generation: u64,
    policy_version: PolicyVersion,
    app_registry_generation: u64,
    settings_registry_generation: u64,
    operation: Option<ActionOperation>,
) -> Option<PublicReasonCode> {
    if record.policy_version != policy_version {
        return Some(PublicReasonCode::PolicyChanged);
    }
    if record.runtime_snapshot_generation != runtime_generation {
        return Some(PublicReasonCode::RuntimeChanged);
    }
    if operation == Some(ActionOperation::OpenApplication)
        && record.application_registry_generation != app_registry_generation
    {
        return Some(PublicReasonCode::ApplicationRegistryChanged);
    }
    if operation == Some(ActionOperation::OpenSettingsPage)
        && record.settings_registry_generation != settings_registry_generation
    {
        return Some(PublicReasonCode::SettingsRegistryChanged);
    }
    None
}

fn is_exact_cancellation(text: &str, locale: &str) -> bool {
    let trimmed = text.trim();
    if locale.starts_with("fa") {
        matches!(trimmed, "لغو" | "بی‌خیال" | "بی خیال" | "انجام نده")
    } else {
        trimmed.eq_ignore_ascii_case("cancel")
            || trimmed.eq_ignore_ascii_case("never mind")
            || trimmed.eq_ignore_ascii_case("stop")
    }
}

fn localized_view(
    locale: &str,
    status: PublicActionStatus,
    reason: PublicReasonCode,
    target: &str,
    expires_at: Option<u64>,
    recoverable: bool,
) -> ActionResponseView {
    let fa = locale.starts_with("fa");
    let (title, base_message) = localized_text(fa, status, reason);
    let mut message = String::new();
    if reason == PublicReasonCode::DispatchAccepted && !target.is_empty() {
        if fa {
            let _ = write!(&mut message, "{} باز شد.", target);
        } else {
            let _ = write!(&mut message, "{} was opened.", target);
        }
    } else {
        let _ = message.push_str(base_message);
    }
    let mut title_text = String::new();
    let _ = title_text.push_str(title);
    let mut summary = String::new();
    if !target.is_empty() {
        let _ = summary.push_str(if fa { "باز کردن " } else { "Open " });
        let _ = summary.push_str(target);
    }
    let mut target_name = String::new();
    let _ = target_name.push_str(target);
    ActionResponseView {
        status,
        title: title_text,
        message,
        action_summary: summary,
        target_display_name: target_name,
        available_choices: Vec::new(),
        confirmation_level: if status == PublicActionStatus::ConfirmationRequired {
            Some(ConfirmationLevel::Soft)
        } else {
            None
        },
        expires_at,
        recoverable,
        public_reason_code: reason,
    }
}

fn localized_text(
    fa: bool,
    status: PublicActionStatus,
    reason: PublicReasonCode,
) -> (&'static str, &'static str) {
    if fa {
        return match (status, reason) {
            (PublicActionStatus::ClarificationRequired, _) => {
                ("نیاز به توضیح", "کدام بخش تنظیمات را باز کنم؟")
            }
            (PublicActionStatus::ConfirmationRequired, _) => {
                ("تأیید لازم است", "آیا می‌خواهید این کار انجام شود؟")
            }
            (PublicActionStatus::Completed, _) => ("انجام شد", "درخواست پذیرفته شد."),
            (PublicActionStatus::Cancelled, _) => ("لغو شد", "درخواست لغو شد."),
            (PublicActionStatus::Expired, _) => {
                ("زمان پایان یافت", "زمان این درخواست به پایان رسید.")
            }
            (PublicActionStatus::AlreadyPending, _) => (
                "درخواست در انتظار است",
                "ابتدا درخواست در انتظار را لغو کنید.",
            ),
            (_, PublicReasonCode::TargetNotFound) => ("یافت نشد", "هدف درخواستی پیدا نشد."),
            (_, PublicReasonCode::DispatchFailed) => ("ناموفق", "باز کردن هدف انجام نشد."),
            (_, PublicReasonCode::InvalidSelection) => ("نامعتبر", "انتخاب معتبر نیست."),
            (PublicActionStatus::Invalidated, _) => (
                "نامعتبر شد",
                "محیط تغییر کرده است؛ درخواست را دوباره آغاز کنید.",
            ),
            (PublicActionStatus::Unsupported, _) => {
                ("پشتیبانی نمی‌شود", "این درخواست پشتیبانی نمی‌شود.")
            }
            (PublicActionStatus::Rejected, _) => ("رد شد", "این درخواست انجام نشد."),
            (PublicActionStatus::InvalidInput, _) => ("نامعتبر", "ورودی معتبر نیست."),
            _ => ("", ""),
        };
    }
    match (status, reason) {
        (PublicActionStatus::ClarificationRequired, _) => {
            ("Clarification needed", "Which settings page should I open?")
        }
        (PublicActionStatus::ConfirmationRequired, _) => (
            "Confirmation required",
            "Do you want me to perform this action?",
        ),
        (PublicActionStatus::Completed, _) => ("Completed", "The dispatch was accepted."),
        (PublicActionStatus::Cancelled, _) => ("Cancelled", "The request was cancelled."),
        (PublicActionStatus::Expired, _) => ("Expired", "The request expired."),
        (PublicActionStatus::AlreadyPending, _) => (
            "Action already pending",
            "Cancel the pending action before starting another.",
        ),
        (_, PublicReasonCode::TargetNotFound) => {
            ("Not found", "I could not find that registered target.")
        }
        (_, PublicReasonCode::DispatchFailed) => ("Dispatch failed", "The target was not opened."),
        (_, PublicReasonCode::InvalidSelection) => ("Invalid choice", "That choice is not valid."),
        (PublicActionStatus::Invalidated, _) => (
            "Request invalidated",
            "The environment changed. Start the request again.",
        ),
        (PublicActionStatus::Unsupported, _) => ("Unsupported", "This request is not supported."),
        (PublicActionStatus::Rejected, _) => ("Rejected", "The request was not performed."),
        (PublicActionStatus::InvalidInput, _) => ("Invalid input", "The input is not valid."),
        _ => ("", ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_intent::TypedIdentifier;
    use crate::confirmation::{ApprovalProof, ConfirmationResponseType, ResponderIdentity};
    use crate::executor::{
        DispatchStatus, LaunchApplicationRequest, OpenSettingsPageRequest, RegistryStatus,
    };
    use crate::planner::{RegistryAliasRef, RegistryTargetRef};
    use crate::policy::{
        PolicyCategory, PolicyEffect, PolicyOperation, PolicyRule, POLICY_V1_VERSION,
    };

    static APP_ALIASES: &[RegistryAliasRef<'static>] = &[
        RegistryAliasRef {
            locale: "en",
            value: "calc",
        },
        RegistryAliasRef {
            locale: "fa",
            value: "ماشین حساب",
        },
    ];
    static NETWORK_ALIASES: &[RegistryAliasRef<'static>] = &[RegistryAliasRef {
        locale: "fa",
        value: "شبکه",
    }];
    static DISPLAY_ALIASES: &[RegistryAliasRef<'static>] = &[];

    #[derive(Clone, Copy)]
    struct Registry;

    impl PlannerRegistry for Registry {
        fn alias_model_version(&self) -> u16 {
            crate::planner::ALIAS_MODEL_V1
        }

        fn visit_applications(&self, visitor: &mut dyn FnMut(RegistryTargetRef<'_>)) {
            visitor(RegistryTargetRef {
                canonical_id: "calculator",
                display_name: "Calculator",
                aliases: APP_ALIASES,
            });
        }

        fn visit_settings_pages(&self, visitor: &mut dyn FnMut(RegistryTargetRef<'_>)) {
            visitor(RegistryTargetRef {
                canonical_id: "network",
                display_name: "Network",
                aliases: NETWORK_ALIASES,
            });
            visitor(RegistryTargetRef {
                canonical_id: "display",
                display_name: "Display",
                aliases: DISPLAY_ALIASES,
            });
        }
    }

    #[derive(Default)]
    struct Adapter {
        fail: bool,
    }

    impl TrustedLaunchAdapter for Adapter {
        fn application_status(&self, id: &TypedIdentifier) -> RegistryStatus {
            if id.as_str() == "calculator" {
                RegistryStatus::Registered
            } else {
                RegistryStatus::NotFound
            }
        }

        fn settings_page_status(&self, id: &TypedIdentifier) -> RegistryStatus {
            if matches!(id.as_str(), "network" | "display") {
                RegistryStatus::Registered
            } else {
                RegistryStatus::NotFound
            }
        }

        fn launch_application(&mut self, _: LaunchApplicationRequest) -> DispatchStatus {
            if self.fail {
                DispatchStatus::Failed
            } else {
                DispatchStatus::Accepted
            }
        }

        fn open_settings_page(&mut self, _: OpenSettingsPageRequest) -> DispatchStatus {
            if self.fail {
                DispatchStatus::Failed
            } else {
                DispatchStatus::Accepted
            }
        }
    }

    static CONFIRM_OPEN_RULES: &[PolicyRule] = &[
        PolicyRule::new(
            PolicyOperation::OpenApplication,
            PolicyCategory::Execute,
            PolicyEffect::Confirm(ConfirmationLevel::Soft),
        ),
        PolicyRule::new(
            PolicyOperation::OpenSettingsPage,
            PolicyCategory::Execute,
            PolicyEffect::Confirm(ConfirmationLevel::Soft),
        ),
    ];
    static DENY_OPEN_RULES: &[PolicyRule] = &[PolicyRule::new(
        PolicyOperation::OpenApplication,
        PolicyCategory::Execute,
        PolicyEffect::Deny,
    )];

    fn runtime(generation: u64, now: u64) -> RuntimeContextSnapshot {
        let mut runtime = RuntimeContextSnapshot {
            available: true,
            generation,
            captured_mono_ms: now,
            ..RuntimeContextSnapshot::default()
        };
        runtime.session.desktop_mode = Some(true);
        runtime.session.installer_mode = Some(false);
        runtime.session.recovery_mode = Some(false);
        runtime
    }

    fn request(id: u64, locale: &str, text: &str, generation: u64, now: u64) -> PlannerInput {
        PlannerInput::direct(
            id,
            4,
            SessionId(7),
            RequestedBy::User(42),
            locale,
            text,
            generation,
            now,
        )
    }

    fn context<'a>(
        runtime: &'a RuntimeContextSnapshot,
        policy: &'a PolicyEngine,
        now: u64,
    ) -> CoordinatorContext<'a> {
        CoordinatorContext {
            conversation_id: ConversationId(4),
            runtime,
            policy,
            session: SessionAuthorization::new(
                SessionId(7),
                ResponderIdentity::User(42),
                SessionStatus::Active,
            ),
            requester: RequestedBy::User(42),
            now: AuthorityTime(now),
            application_registry_generation: 11,
            settings_registry_generation: 13,
        }
    }

    fn new_coordinator(policy: PolicyEngine) -> ActionCoordinator<Registry, Adapter, 96, 16, 64> {
        ActionCoordinator::new(
            Registry,
            Adapter::default(),
            policy,
            CoordinatorConfig::default(),
            [0x51; 32],
        )
    }

    #[test]
    fn exact_application_and_settings_requests_complete_through_trusted_flow() {
        let runtime = runtime(1, 100);
        let policy = PolicyEngine::v1();
        let mut coordinator = new_coordinator(policy);
        let result = coordinator.handle(
            CoordinatorInput::UserRequest(request(1, "en", "Open Calculator", 1, 100)),
            context(&runtime, &policy, 101),
        );
        assert!(matches!(result, CoordinatorResult::ActionCompleted(_)));
        assert_eq!(
            result.response().public_reason_code,
            PublicReasonCode::DispatchAccepted
        );
        assert_eq!(
            coordinator.records().last().unwrap().current_state,
            CoordinatorState::Completed
        );

        let result = coordinator.handle(
            CoordinatorInput::UserRequest(request(2, "en", "Open Network settings", 1, 102)),
            context(&runtime, &policy, 103),
        );
        assert!(matches!(result, CoordinatorResult::ActionCompleted(_)));
    }

    #[test]
    fn ambiguity_valid_clarification_and_replay_are_bound_and_single_use() {
        let runtime = runtime(1, 100);
        let policy = PolicyEngine::v1();
        let mut coordinator = new_coordinator(policy);
        let result = coordinator.handle(
            CoordinatorInput::UserRequest(request(1, "en", "Open settings", 1, 100)),
            context(&runtime, &policy, 100),
        );
        assert!(matches!(
            result,
            CoordinatorResult::ClarificationRequired(_)
        ));
        assert_eq!(result.response().available_choices.len(), 2);
        let record = coordinator.active_record().unwrap().clone();
        let response = ClarificationResponse {
            input_id: 2,
            action_id: record.coordinator_action_id,
            clarification_id: coordinator
                .active
                .as_ref()
                .unwrap()
                .clarification
                .as_ref()
                .unwrap()
                .clarification_id(),
            choice_id: 0,
            conversation_id: ConversationId(4),
            session_id: SessionId(7),
            requester: RequestedBy::User(42),
            submitted_at: 101,
        };
        let completed = coordinator.handle(
            CoordinatorInput::ClarificationResponse(response.clone()),
            context(&runtime, &policy, 101),
        );
        assert!(matches!(completed, CoordinatorResult::ActionCompleted(_)));
        let replayed = coordinator.handle(
            CoordinatorInput::ClarificationResponse(response),
            context(&runtime, &policy, 102),
        );
        assert_eq!(replayed, completed);
        assert_eq!(
            coordinator
                .audit()
                .entries()
                .filter(|entry| entry.event == CoordinatorAuditEvent::DispatchAttempted)
                .count(),
            1
        );
    }

    #[test]
    fn invalid_and_expired_clarification_never_dispatch() {
        let runtime = runtime(1, 100);
        let policy = PolicyEngine::v1();
        let mut coordinator = new_coordinator(policy);
        coordinator.handle(
            CoordinatorInput::UserRequest(request(1, "en", "Open settings", 1, 100)),
            context(&runtime, &policy, 100),
        );
        let active = coordinator.active.as_ref().unwrap();
        let invalid = ClarificationResponse {
            input_id: 2,
            action_id: active.record.coordinator_action_id,
            clarification_id: active.clarification.as_ref().unwrap().clarification_id(),
            choice_id: 7,
            conversation_id: ConversationId(4),
            session_id: SessionId(7),
            requester: RequestedBy::User(42),
            submitted_at: 101,
        };
        assert!(matches!(
            coordinator.handle(
                CoordinatorInput::ClarificationResponse(invalid),
                context(&runtime, &policy, 101)
            ),
            CoordinatorResult::InvalidInput(_)
        ));
        assert_eq!(
            coordinator.active_record().unwrap().current_state,
            CoordinatorState::AwaitingClarification
        );
        assert!(matches!(
            coordinator.handle(
                CoordinatorInput::QueryPendingAction(QueryPendingAction {
                    conversation_id: ConversationId(4),
                    session_id: SessionId(7),
                    requester: RequestedBy::User(42),
                }),
                context(&runtime, &policy, 30_101)
            ),
            CoordinatorResult::ActionExpired(_)
        ));
        assert!(!coordinator
            .audit()
            .entries()
            .any(|entry| entry.event == CoordinatorAuditEvent::DispatchAttempted));
    }

    #[test]
    fn confirmation_approval_rejection_expiry_and_duplicate_are_authority_owned() {
        let policy = PolicyEngine::from_static_rules(PolicyVersion::new(9, 1), CONFIRM_OPEN_RULES);
        let runtime = runtime(1, 100);
        let mut coordinator = new_coordinator(policy);
        let required = coordinator.handle(
            CoordinatorInput::UserRequest(request(1, "en", "Open Calculator", 1, 100)),
            context(&runtime, &policy, 100),
        );
        assert!(matches!(
            required,
            CoordinatorResult::ConfirmationRequired { .. }
        ));
        let active = coordinator.active_record().unwrap().clone();
        let response = BoundConfirmationResponse {
            input_id: 2,
            action_id: active.coordinator_action_id,
            response: AuthorityConfirmationResponse::new(
                active.challenge_id.unwrap(),
                SessionId(7),
                ResponderIdentity::User(42),
                ConfirmationResponseType::Approved(ApprovalProof::SoftExplicit),
                AuthorityTime(101),
            ),
        };
        let completed = coordinator.handle(
            CoordinatorInput::ConfirmationResponse(response.clone()),
            context(&runtime, &policy, 101),
        );
        assert!(matches!(completed, CoordinatorResult::ActionCompleted(_)));
        assert_eq!(
            coordinator.handle(
                CoordinatorInput::ConfirmationResponse(response),
                context(&runtime, &policy, 102)
            ),
            completed
        );

        let mut rejected = new_coordinator(policy);
        rejected.handle(
            CoordinatorInput::UserRequest(request(10, "en", "Open Calculator", 1, 100)),
            context(&runtime, &policy, 100),
        );
        let record = rejected.active_record().unwrap().clone();
        assert!(matches!(
            rejected.handle(
                CoordinatorInput::ConfirmationResponse(BoundConfirmationResponse {
                    input_id: 11,
                    action_id: record.coordinator_action_id,
                    response: AuthorityConfirmationResponse::new(
                        record.challenge_id.unwrap(),
                        SessionId(7),
                        ResponderIdentity::User(42),
                        ConfirmationResponseType::Rejected,
                        AuthorityTime(101),
                    ),
                }),
                context(&runtime, &policy, 101)
            ),
            CoordinatorResult::ActionRejected(_)
        ));

        let mut expired = new_coordinator(policy);
        expired.handle(
            CoordinatorInput::UserRequest(request(20, "en", "Open Calculator", 1, 100)),
            context(&runtime, &policy, 100),
        );
        assert!(matches!(
            expired.handle(
                CoordinatorInput::QueryPendingAction(QueryPendingAction {
                    conversation_id: ConversationId(4),
                    session_id: SessionId(7),
                    requester: RequestedBy::User(42),
                }),
                context(&runtime, &policy, 30_101)
            ),
            CoordinatorResult::ActionExpired(_)
        ));
    }

    #[test]
    fn cancellation_conflict_and_information_preserve_one_active_action() {
        let runtime = runtime(1, 100);
        let policy = PolicyEngine::v1();
        let mut coordinator = new_coordinator(policy);
        coordinator.handle(
            CoordinatorInput::UserRequest(request(1, "en", "Open settings", 1, 100)),
            context(&runtime, &policy, 100),
        );
        let action = coordinator.active_record().unwrap().coordinator_action_id;
        assert!(matches!(
            coordinator.handle(
                CoordinatorInput::UserRequest(request(2, "en", "Open Calculator", 1, 101)),
                context(&runtime, &policy, 101)
            ),
            CoordinatorResult::ActionAlreadyPending(_)
        ));
        assert!(matches!(
            coordinator.handle(
                CoordinatorInput::UserRequest(request(
                    3,
                    "en",
                    "Is the network connected?",
                    1,
                    102
                )),
                context(&runtime, &policy, 102)
            ),
            CoordinatorResult::NoAction(_)
        ));
        assert_eq!(
            coordinator.active_record().unwrap().coordinator_action_id,
            action
        );
        assert!(matches!(
            coordinator.handle(
                CoordinatorInput::UserRequest(request(4, "en", "never mind", 1, 103)),
                context(&runtime, &policy, 103)
            ),
            CoordinatorResult::ActionCancelled(_)
        ));

        let mut persian = new_coordinator(policy);
        persian.handle(
            CoordinatorInput::UserRequest(request(10, "fa", "تنظیمات را باز کن", 1, 100)),
            context(&runtime, &policy, 100),
        );
        assert!(matches!(
            persian.handle(
                CoordinatorInput::UserRequest(request(11, "fa", "لغو", 1, 101)),
                context(&runtime, &policy, 101)
            ),
            CoordinatorResult::ActionCancelled(_)
        ));
    }

    #[test]
    fn cancellation_of_confirmation_prevents_dispatch() {
        let policy = PolicyEngine::from_static_rules(PolicyVersion::new(9, 1), CONFIRM_OPEN_RULES);
        let runtime = runtime(1, 100);
        let mut coordinator = new_coordinator(policy);
        coordinator.handle(
            CoordinatorInput::UserRequest(request(1, "en", "Open Calculator", 1, 100)),
            context(&runtime, &policy, 100),
        );
        let record = coordinator.active_record().unwrap().clone();
        assert!(matches!(
            coordinator.handle(
                CoordinatorInput::CancelPendingAction(CancelPendingAction {
                    input_id: 2,
                    action_id: record.coordinator_action_id,
                    conversation_id: ConversationId(4),
                    session_id: SessionId(7),
                    requester: RequestedBy::User(42),
                    submitted_at: 101,
                }),
                context(&runtime, &policy, 101)
            ),
            CoordinatorResult::ActionCancelled(_)
        ));
        assert!(!coordinator
            .audit()
            .entries()
            .any(|entry| entry.event == CoordinatorAuditEvent::DispatchAttempted));
    }

    #[test]
    fn session_policy_runtime_and_registry_changes_invalidate_pending_state() {
        let base_runtime = runtime(1, 100);
        let policy = PolicyEngine::v1();

        let mut session_end = new_coordinator(policy);
        session_end.handle(
            CoordinatorInput::UserRequest(request(1, "en", "Open settings", 1, 100)),
            context(&base_runtime, &policy, 100),
        );
        assert!(matches!(
            session_end.handle(
                CoordinatorInput::SessionEnded(SessionEndedInput {
                    input_id: 2,
                    session_id: SessionId(7),
                    submitted_at: 101,
                }),
                context(&base_runtime, &policy, 101)
            ),
            CoordinatorResult::ActionInvalidated(_)
        ));

        for reason in [
            PublicReasonCode::PolicyChanged,
            PublicReasonCode::RuntimeChanged,
            PublicReasonCode::SettingsRegistryChanged,
        ] {
            let mut coordinator = new_coordinator(policy);
            coordinator.handle(
                CoordinatorInput::UserRequest(request(10, "en", "Open settings", 1, 100)),
                context(&base_runtime, &policy, 100),
            );
            let changed_policy =
                PolicyEngine::from_static_rules(PolicyVersion::new(2, 0), CONFIRM_OPEN_RULES);
            let changed_runtime = runtime(2, 101);
            let mut changed_context = context(&base_runtime, &policy, 101);
            match reason {
                PublicReasonCode::PolicyChanged => changed_context.policy = &changed_policy,
                PublicReasonCode::RuntimeChanged => changed_context.runtime = &changed_runtime,
                PublicReasonCode::SettingsRegistryChanged => {
                    changed_context.settings_registry_generation = 14
                }
                _ => unreachable!(),
            }
            let result = coordinator.handle(
                CoordinatorInput::QueryPendingAction(QueryPendingAction {
                    conversation_id: ConversationId(4),
                    session_id: SessionId(7),
                    requester: RequestedBy::User(42),
                }),
                changed_context,
            );
            assert!(matches!(result, CoordinatorResult::ActionInvalidated(_)));
            assert_eq!(result.response().public_reason_code, reason);
        }
    }

    #[test]
    fn confirmation_pending_does_not_survive_session_change() {
        let policy = PolicyEngine::from_static_rules(PolicyVersion::new(9, 1), CONFIRM_OPEN_RULES);
        let runtime = runtime(1, 100);
        let mut coordinator = new_coordinator(policy);
        coordinator.handle(
            CoordinatorInput::UserRequest(request(1, "en", "Open Calculator", 1, 100)),
            context(&runtime, &policy, 100),
        );
        let mut changed = context(&runtime, &policy, 101);
        changed.session = SessionAuthorization::new(
            SessionId(8),
            ResponderIdentity::User(42),
            SessionStatus::Active,
        );
        assert!(matches!(
            coordinator.handle(
                CoordinatorInput::QueryPendingAction(QueryPendingAction {
                    conversation_id: ConversationId(4),
                    session_id: SessionId(8),
                    requester: RequestedBy::User(42),
                }),
                changed
            ),
            CoordinatorResult::ActionInvalidated(_)
        ));

        let mut ended = new_coordinator(policy);
        ended.handle(
            CoordinatorInput::UserRequest(request(2, "en", "Open Calculator", 1, 100)),
            context(&runtime, &policy, 100),
        );
        assert!(matches!(
            ended.handle(
                CoordinatorInput::SessionEnded(SessionEndedInput {
                    input_id: 3,
                    session_id: SessionId(7),
                    submitted_at: 101,
                }),
                context(&runtime, &policy, 101)
            ),
            CoordinatorResult::ActionInvalidated(_)
        ));

        let mut registry_changed = new_coordinator(policy);
        registry_changed.handle(
            CoordinatorInput::UserRequest(request(4, "en", "Open Calculator", 1, 100)),
            context(&runtime, &policy, 100),
        );
        let mut changed_registry = context(&runtime, &policy, 101);
        changed_registry.application_registry_generation = 12;
        let invalidated = registry_changed.handle(
            CoordinatorInput::QueryPendingAction(QueryPendingAction {
                conversation_id: ConversationId(4),
                session_id: SessionId(7),
                requester: RequestedBy::User(42),
            }),
            changed_registry,
        );
        assert!(matches!(
            invalidated,
            CoordinatorResult::ActionInvalidated(_)
        ));
        assert_eq!(
            invalidated.response().public_reason_code,
            PublicReasonCode::ApplicationRegistryChanged
        );
    }

    #[test]
    fn policy_denial_is_terminal_before_readiness_or_dispatch() {
        let policy = PolicyEngine::from_static_rules(PolicyVersion::new(9, 2), DENY_OPEN_RULES);
        let runtime = runtime(1, 100);
        let mut coordinator = new_coordinator(policy);
        let result = coordinator.handle(
            CoordinatorInput::UserRequest(request(1, "en", "Open Calculator", 1, 100)),
            context(&runtime, &policy, 100),
        );
        assert!(matches!(result, CoordinatorResult::ActionRejected(_)));
        assert!(!coordinator.audit().entries().any(|entry| matches!(
            entry.event,
            CoordinatorAuditEvent::ReadinessProduced | CoordinatorAuditEvent::DispatchAttempted
        )));
    }

    #[test]
    fn dispatch_failure_is_terminal_and_duplicate_request_is_idempotent() {
        let runtime = runtime(1, 100);
        let policy = PolicyEngine::v1();
        let mut coordinator = ActionCoordinator::<Registry, Adapter>::new(
            Registry,
            Adapter { fail: true },
            policy,
            CoordinatorConfig::default(),
            [0x61; 32],
        );
        let input = CoordinatorInput::UserRequest(request(1, "en", "Open Calculator", 1, 100));
        let first = coordinator.handle(input.clone(), context(&runtime, &policy, 101));
        assert_eq!(
            first.response().public_reason_code,
            PublicReasonCode::DispatchFailed
        );
        let second = coordinator.handle(input, context(&runtime, &policy, 102));
        assert_eq!(first, second);
        assert_eq!(
            coordinator
                .audit()
                .entries()
                .filter(|entry| entry.event == CoordinatorAuditEvent::DispatchAttempted)
                .count(),
            1
        );
    }

    #[test]
    fn unsupported_negated_quoted_and_persian_clarification_are_safe() {
        let runtime = runtime(1, 100);
        let policy = PolicyEngine::v1();
        let mut coordinator = new_coordinator(policy);
        assert!(matches!(
            coordinator.handle(
                CoordinatorInput::UserRequest(request(1, "en", "Restart networkd", 1, 100)),
                context(&runtime, &policy, 100)
            ),
            CoordinatorResult::ActionUnsupported(_)
        ));
        for (id, text) in [
            (2, "Do not open Calculator"),
            (3, "He said \"Open Calculator\""),
        ] {
            assert!(matches!(
                coordinator.handle(
                    CoordinatorInput::UserRequest(request(id, "en", text, 1, 101)),
                    context(&runtime, &policy, 101)
                ),
                CoordinatorResult::NoAction(_)
            ));
        }
        let result = coordinator.handle(
            CoordinatorInput::UserRequest(request(4, "fa", "تنظیمات را باز کن", 1, 102)),
            context(&runtime, &policy, 102),
        );
        assert!(matches!(
            result,
            CoordinatorResult::ClarificationRequired(_)
        ));
        assert_eq!(
            result.response().message.as_str(),
            "کدام بخش تنظیمات را باز کنم؟"
        );
    }

    #[test]
    fn audit_and_record_retention_are_bounded_redacted_and_ordered() {
        let runtime = runtime(1, 100);
        let policy = PolicyEngine::v1();
        let mut coordinator = ActionCoordinator::<Registry, Adapter, 4, 2, 8>::new(
            Registry,
            Adapter::default(),
            policy,
            CoordinatorConfig {
                record_retention_ms: 10,
                ..CoordinatorConfig::default()
            },
            [0x71; 32],
        );
        for id in 1..=3 {
            coordinator.handle(
                CoordinatorInput::UserRequest(request(id, "en", "Open Calculator", 1, 100)),
                context(&runtime, &policy, 101),
            );
        }
        assert_eq!(coordinator.records().count(), 2);
        assert_eq!(coordinator.audit().len(), 4);
        assert!(coordinator.audit().evicted() > 0);
        assert!(coordinator.audit().entries().all(|entry| {
            entry.operation == Some(ActionOperation::OpenApplication)
                && entry.public_result != PublicReasonCode::Unknown
        }));
        coordinator.handle(
            CoordinatorInput::QueryPendingAction(QueryPendingAction {
                conversation_id: ConversationId(4),
                session_id: SessionId(7),
                requester: RequestedBy::User(42),
            }),
            context(&runtime, &policy, 112),
        );
        assert_eq!(coordinator.records().count(), 0);
    }

    #[test]
    fn mandatory_stages_precede_dispatch_and_confirmation_cannot_be_skipped() {
        let policy = PolicyEngine::from_static_rules(PolicyVersion::new(9, 1), CONFIRM_OPEN_RULES);
        let runtime = runtime(1, 100);
        let mut coordinator = new_coordinator(policy);
        coordinator.handle(
            CoordinatorInput::UserRequest(request(1, "en", "Open Calculator", 1, 100)),
            context(&runtime, &policy, 100),
        );
        assert!(!coordinator
            .audit()
            .entries()
            .any(|entry| entry.event == CoordinatorAuditEvent::DispatchAttempted));
        let record = coordinator.active_record().unwrap().clone();
        coordinator.handle(
            CoordinatorInput::ConfirmationResponse(BoundConfirmationResponse {
                input_id: 2,
                action_id: record.coordinator_action_id,
                response: AuthorityConfirmationResponse::new(
                    record.challenge_id.unwrap(),
                    SessionId(7),
                    ResponderIdentity::User(42),
                    ConfirmationResponseType::Approved(ApprovalProof::SoftExplicit),
                    AuthorityTime(101),
                ),
            }),
            context(&runtime, &policy, 101),
        );
        let events: std::vec::Vec<_> = coordinator.audit().entries().map(|e| e.event).collect();
        let policy_index = events
            .iter()
            .position(|event| *event == CoordinatorAuditEvent::PolicyResultReceived)
            .unwrap();
        let confirmation_index = events
            .iter()
            .position(|event| *event == CoordinatorAuditEvent::ConfirmationResponseAccepted)
            .unwrap();
        let readiness_index = events
            .iter()
            .position(|event| *event == CoordinatorAuditEvent::ReadinessProduced)
            .unwrap();
        let dispatch_index = events
            .iter()
            .position(|event| *event == CoordinatorAuditEvent::DispatchAttempted)
            .unwrap();
        assert!(policy_index < confirmation_index);
        assert!(confirmation_index < readiness_index);
        assert!(readiness_index < dispatch_index);
    }

    #[test]
    fn default_policy_version_is_stable_for_fixture() {
        assert_eq!(PolicyEngine::v1().version(), POLICY_V1_VERSION);
    }
}
