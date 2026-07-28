//! Bounded, typed post-dispatch observation for Wise Owl safe actions.
//!
//! This module deliberately has no launch, retry, process-control, parsing,
//! filesystem, or generic inspection capability.  It consumes only successful
//! executor results and correlated evidence from bounded trusted source kinds.

use heapless::{Deque, Vec};

use crate::action_intent::{
    ActionOperation, ActionTarget, AuditId, IntentId, RequestedBy, SessionId, TypedIdentifier,
};
use crate::confirmation::AuthorityTime;
use crate::executor::{ExecutionId, ExecutionResult, ExecutionResultCode, LaunchCorrelationToken};

pub const DEFAULT_OBSERVATION_CAPACITY: usize = 16;
pub const DEFAULT_OBSERVATION_AUDIT_CAPACITY: usize = 96;
pub const DEFAULT_EVIDENCE_REPLAY_CAPACITY: usize = 64;
pub const DEFAULT_EVIDENCE_SUMMARY_CAPACITY: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObservationId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EvidenceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessContract {
    DispatchAccepted,
    ProcessRegistered,
    ApplicationRegistered,
    FirstWindowRegistered,
    ApplicationReady,
    ControlPanelDispatched,
    ControlPanelWindowRegistered,
    RequestedPageActivated,
    PageReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AchievedReadiness {
    DispatchAccepted,
    ProcessRegistered,
    ApplicationRegistered,
    WindowRegistered,
    PageActivated,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservationDeadlines {
    pub process_registration: AuthorityTime,
    pub registration: AuthorityTime,
    pub window_or_page: AuthorityTime,
    pub final_deadline: AuthorityTime,
}

impl ObservationDeadlines {
    pub const fn uniform(deadline: AuthorityTime) -> Self {
        Self {
            process_registration: deadline,
            registration: deadline,
            window_or_page: deadline,
            final_deadline: deadline,
        }
    }

    fn for_state(self, state: ObservationState) -> AuthorityTime {
        match state {
            ObservationState::WaitingForProcess => self.process_registration,
            ObservationState::WaitingForRegistration => self.registration,
            ObservationState::WaitingForWindow | ObservationState::WaitingForPage => {
                self.window_or_page
            }
            _ => self.final_deadline,
        }
    }
}

/// Immutable envelope derived from an accepted `ExecutionResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationRequest {
    observation_id: ObservationId,
    execution_id: ExecutionId,
    intent_id: IntentId,
    operation: ActionOperation,
    target: ActionTarget,
    session_id: SessionId,
    requester: RequestedBy,
    dispatch_timestamp: AuthorityTime,
    registry_generation: u64,
    readiness_contract: ReadinessContract,
    deadlines: ObservationDeadlines,
    audit_id: AuditId,
    correlation_token: LaunchCorrelationToken,
}

impl ObservationRequest {
    pub fn from_execution<R: OutcomeRegistry>(
        observation_id: ObservationId,
        execution: &ExecutionResult,
        registry: &R,
        deadlines: ObservationDeadlines,
    ) -> Result<Self, ObservationCreateError> {
        if !matches!(
            execution.operation(),
            ActionOperation::OpenApplication | ActionOperation::OpenSettingsPage
        ) {
            return Err(ObservationCreateError::UnsupportedObservation);
        }
        if execution.code() != ExecutionResultCode::Succeeded {
            return Err(ObservationCreateError::DispatchNotAccepted);
        }
        let dispatch_timestamp = execution
            .dispatch_timestamp()
            .ok_or(ObservationCreateError::DispatchNotAccepted)?;
        let correlation_token = execution
            .correlation_token()
            .ok_or(ObservationCreateError::MissingCorrelation)?;
        if registry.generation(execution.operation()) != execution.registry_generation() {
            return Err(ObservationCreateError::RegistryInvalidated);
        }
        let readiness_contract = registry
            .readiness_contract(execution.operation(), execution.target())
            .ok_or(ObservationCreateError::UnsupportedObservation)?;
        if !contract_matches_operation(readiness_contract, execution.operation()) {
            return Err(ObservationCreateError::ContractMismatch);
        }
        if deadlines.final_deadline < dispatch_timestamp {
            return Err(ObservationCreateError::InvalidDeadline);
        }
        Ok(Self {
            observation_id,
            execution_id: execution.execution_id(),
            intent_id: execution.intent_id(),
            operation: execution.operation(),
            target: execution.target().clone(),
            session_id: execution.session_id(),
            requester: execution.requester(),
            dispatch_timestamp,
            registry_generation: execution.registry_generation(),
            readiness_contract,
            deadlines,
            audit_id: execution.audit_id(),
            correlation_token,
        })
    }

    pub const fn observation_id(&self) -> ObservationId {
        self.observation_id
    }
    pub const fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }
    pub const fn intent_id(&self) -> IntentId {
        self.intent_id
    }
    pub const fn operation(&self) -> ActionOperation {
        self.operation
    }
    pub fn target(&self) -> &ActionTarget {
        &self.target
    }
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub const fn requester(&self) -> RequestedBy {
        self.requester
    }
    pub const fn dispatch_timestamp(&self) -> AuthorityTime {
        self.dispatch_timestamp
    }
    pub const fn registry_generation(&self) -> u64 {
        self.registry_generation
    }
    pub const fn readiness_contract(&self) -> ReadinessContract {
        self.readiness_contract
    }
    pub const fn deadlines(&self) -> ObservationDeadlines {
        self.deadlines
    }
    pub const fn audit_id(&self) -> AuditId {
        self.audit_id
    }
    pub const fn correlation_token(&self) -> LaunchCorrelationToken {
        self.correlation_token
    }
}

pub trait OutcomeRegistry {
    fn generation(&self, operation: ActionOperation) -> u64;
    fn readiness_contract(
        &self,
        operation: ActionOperation,
        target: &ActionTarget,
    ) -> Option<ReadinessContract>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationCreateError {
    DispatchNotAccepted,
    MissingCorrelation,
    DuplicateExecution,
    CapacityExhausted,
    RegistryInvalidated,
    ContractMismatch,
    InvalidDeadline,
    UnsupportedObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedSourceKind {
    TrustedExecutor,
    ProcessLifecycle,
    SessionManager,
    DisplayServer,
    ApplicationRegistry,
    ControlPanel,
    Coordinator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationEvidenceKind {
    DispatchAccepted,
    ProcessCreated {
        process_instance: u64,
    },
    ProcessExited {
        process_instance: u64,
        public_code: u16,
    },
    ApplicationRegistered,
    WindowRegistered,
    SettingsPageActivated(TypedIdentifier),
    ReadySignal,
    Failed {
        public_code: u16,
    },
    SessionEnded,
    RegistryChanged,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationEvidence {
    evidence_id: EvidenceId,
    source_kind: TrustedSourceKind,
    source_identity: TypedIdentifier,
    session_id: SessionId,
    timestamp: AuthorityTime,
    generation: u64,
    execution_id: ExecutionId,
    correlation_token: LaunchCorrelationToken,
    target: ActionTarget,
    kind: ObservationEvidenceKind,
}

impl ObservationEvidence {
    #[allow(clippy::too_many_arguments)]
    pub fn trusted(
        evidence_id: EvidenceId,
        source_kind: TrustedSourceKind,
        source_identity: TypedIdentifier,
        session_id: SessionId,
        timestamp: AuthorityTime,
        generation: u64,
        execution_id: ExecutionId,
        correlation_token: LaunchCorrelationToken,
        target: ActionTarget,
        kind: ObservationEvidenceKind,
    ) -> Self {
        Self {
            evidence_id,
            source_kind,
            source_identity,
            session_id,
            timestamp,
            generation,
            execution_id,
            correlation_token,
            target,
            kind,
        }
    }

    pub const fn evidence_id(&self) -> EvidenceId {
        self.evidence_id
    }
    pub const fn source_kind(&self) -> TrustedSourceKind {
        self.source_kind
    }
    pub fn source_identity(&self) -> &TypedIdentifier {
        &self.source_identity
    }
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub const fn timestamp(&self) -> AuthorityTime {
        self.timestamp
    }
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub const fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }
    pub const fn correlation_token(&self) -> LaunchCorrelationToken {
        self.correlation_token
    }
    pub fn target(&self) -> &ActionTarget {
        &self.target
    }
    pub const fn kind(&self) -> &ObservationEvidenceKind {
        &self.kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationState {
    Created,
    WaitingForProcess,
    WaitingForRegistration,
    WaitingForWindow,
    WaitingForPage,
    Ready,
    Failed,
    ExitedEarly,
    TimedOut,
    SessionInvalidated,
    RegistryInvalidated,
    Cancelled,
}

impl ObservationState {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Ready
                | Self::Failed
                | Self::ExitedEarly
                | Self::TimedOut
                | Self::SessionInvalidated
                | Self::RegistryInvalidated
                | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicOutcomeCode {
    Ready,
    DispatchAccepted,
    Failed,
    ExitedBeforeReady,
    ReadinessTimedOut,
    SessionInvalidated,
    RegistryInvalidated,
    ObservationCancelled,
    UnsupportedObservation,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedActionOutcomeKind {
    Ready,
    DispatchOnly,
    Failed,
    ExitedEarly,
    TimedOut,
    SessionInvalidated,
    RegistryInvalidated,
    Cancelled,
    UnsupportedObservation,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceSummary {
    pub accepted: u16,
    pub rejected: u16,
    pub duplicate: u16,
    pub late: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedActionOutcome {
    observation_id: ObservationId,
    execution_id: ExecutionId,
    intent_id: IntentId,
    target_kind: crate::action_intent::TargetKind,
    kind: ObservedActionOutcomeKind,
    public_result_code: PublicOutcomeCode,
    achieved_readiness: AchievedReadiness,
    terminal_timestamp: Option<AuthorityTime>,
    evidence_summary: EvidenceSummary,
    audit_id: AuditId,
}

impl ObservedActionOutcome {
    pub const fn observation_id(&self) -> ObservationId {
        self.observation_id
    }
    pub const fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }
    pub const fn intent_id(&self) -> IntentId {
        self.intent_id
    }
    pub const fn target_kind(&self) -> crate::action_intent::TargetKind {
        self.target_kind
    }
    pub const fn kind(&self) -> ObservedActionOutcomeKind {
        self.kind
    }
    pub const fn public_result_code(&self) -> PublicOutcomeCode {
        self.public_result_code
    }
    pub const fn achieved_readiness(&self) -> AchievedReadiness {
        self.achieved_readiness
    }
    pub const fn terminal_timestamp(&self) -> Option<AuthorityTime> {
        self.terminal_timestamp
    }
    pub const fn evidence_summary(&self) -> EvidenceSummary {
        self.evidence_summary
    }
    pub const fn audit_id(&self) -> AuditId {
        self.audit_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationAuditEvent {
    ObservationCreated,
    EvidenceAccepted,
    EvidenceRejected,
    DuplicateEvidenceRejected,
    StateTransition,
    ReadinessAchieved,
    EarlyExit,
    Failure,
    Timeout,
    SessionInvalidation,
    RegistryInvalidation,
    Cancellation,
    LateEventIgnored,
    TerminalOutcomeProduced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservationAuditEntry {
    pub observation_id: ObservationId,
    pub execution_id: ExecutionId,
    pub operation: ActionOperation,
    pub readiness: ReadinessContract,
    pub state: ObservationState,
    pub event: ObservationAuditEvent,
    pub timestamp: AuthorityTime,
}

pub struct ObservationAuditLog<const N: usize = DEFAULT_OBSERVATION_AUDIT_CAPACITY> {
    entries: Deque<ObservationAuditEntry, N>,
    evicted: u64,
}

impl<const N: usize> ObservationAuditLog<N> {
    pub const fn new() -> Self {
        Self {
            entries: Deque::new(),
            evicted: 0,
        }
    }
    pub fn entries(&self) -> impl Iterator<Item = &ObservationAuditEntry> {
        self.entries.iter()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub const fn evicted(&self) -> u64 {
        self.evicted
    }
    fn push(&mut self, entry: ObservationAuditEntry) {
        if N == 0 {
            self.evicted = self.evicted.saturating_add(1);
        } else {
            if self.entries.is_full() {
                let _ = self.entries.pop_front();
                self.evicted = self.evicted.saturating_add(1);
            }
            let _ = self.entries.push_back(entry);
        }
    }
}

impl<const N: usize> Default for ObservationAuditLog<N> {
    fn default() -> Self {
        Self::new()
    }
}

struct ObservationRecord<const REPLAY: usize> {
    request: ObservationRequest,
    state: ObservationState,
    achieved: AchievedReadiness,
    process_instance: Option<u64>,
    seen: Vec<EvidenceId, REPLAY>,
    summary: EvidenceSummary,
    outcome: ObservedActionOutcome,
}

pub struct ActionOutcomeObserver<
    const CAPACITY: usize = DEFAULT_OBSERVATION_CAPACITY,
    const AUDIT: usize = DEFAULT_OBSERVATION_AUDIT_CAPACITY,
    const REPLAY: usize = DEFAULT_EVIDENCE_REPLAY_CAPACITY,
> {
    observations: Vec<ObservationRecord<REPLAY>, CAPACITY>,
    audit: ObservationAuditLog<AUDIT>,
}

impl<const CAPACITY: usize, const AUDIT: usize, const REPLAY: usize>
    ActionOutcomeObserver<CAPACITY, AUDIT, REPLAY>
{
    pub const fn new() -> Self {
        Self {
            observations: Vec::new(),
            audit: ObservationAuditLog::new(),
        }
    }

    pub const fn audit(&self) -> &ObservationAuditLog<AUDIT> {
        &self.audit
    }

    pub fn create(
        &mut self,
        request: ObservationRequest,
    ) -> Result<ObservedActionOutcome, ObservationCreateError> {
        if self
            .observations
            .iter()
            .any(|item| item.request.execution_id == request.execution_id)
        {
            return Err(ObservationCreateError::DuplicateExecution);
        }
        if self.observations.is_full() {
            return Err(ObservationCreateError::CapacityExhausted);
        }
        let state = initial_state(request.readiness_contract);
        let achieved = AchievedReadiness::DispatchAccepted;
        let outcome = make_outcome(
            &request,
            state,
            achieved,
            EvidenceSummary {
                accepted: 1,
                rejected: 0,
                duplicate: 0,
                late: 0,
            },
            if state == ObservationState::Ready {
                Some(request.dispatch_timestamp)
            } else {
                None
            },
        );
        self.audit_for(
            &request,
            state,
            ObservationAuditEvent::ObservationCreated,
            request.dispatch_timestamp,
        );
        if state == ObservationState::Ready {
            self.audit_for(
                &request,
                state,
                ObservationAuditEvent::ReadinessAchieved,
                request.dispatch_timestamp,
            );
        }
        let returned = outcome.clone();
        let _ = self.observations.push(ObservationRecord {
            request,
            state,
            achieved,
            process_instance: None,
            seen: Vec::new(),
            summary: returned.evidence_summary,
            outcome,
        });
        Ok(returned)
    }

    pub fn outcome(&self, execution_id: ExecutionId) -> Option<&ObservedActionOutcome> {
        self.observations
            .iter()
            .find(|record| record.request.execution_id == execution_id)
            .map(|record| &record.outcome)
    }

    pub fn state(&self, execution_id: ExecutionId) -> Option<ObservationState> {
        self.observations
            .iter()
            .find(|record| record.request.execution_id == execution_id)
            .map(|record| record.state)
    }

    pub fn observe<R: OutcomeRegistry>(
        &mut self,
        evidence: ObservationEvidence,
        registry: &R,
    ) -> Option<ObservedActionOutcome> {
        let index = self
            .observations
            .iter()
            .position(|record| record.request.execution_id == evidence.execution_id)?;
        let request = self.observations[index].request.clone();
        let now = evidence.timestamp;

        if self.observations[index].state.is_terminal() {
            self.observations[index].summary.late =
                self.observations[index].summary.late.saturating_add(1);
            self.audit_for(
                &request,
                self.observations[index].state,
                ObservationAuditEvent::LateEventIgnored,
                now,
            );
            self.refresh_outcome(index, Some(now));
            return Some(self.observations[index].outcome.clone());
        }
        if now
            > self.observations[index]
                .request
                .deadlines
                .for_state(self.observations[index].state)
        {
            self.terminal(
                index,
                ObservationState::TimedOut,
                ObservationAuditEvent::Timeout,
                now,
            );
            self.observations[index].summary.late =
                self.observations[index].summary.late.saturating_add(1);
            self.audit_for(
                &request,
                ObservationState::TimedOut,
                ObservationAuditEvent::LateEventIgnored,
                now,
            );
            self.refresh_outcome(index, Some(now));
            return Some(self.observations[index].outcome.clone());
        }
        if self.observations[index]
            .seen
            .iter()
            .any(|id| *id == evidence.evidence_id)
        {
            self.observations[index].summary.duplicate =
                self.observations[index].summary.duplicate.saturating_add(1);
            self.audit_for(
                &request,
                self.observations[index].state,
                ObservationAuditEvent::DuplicateEvidenceRejected,
                now,
            );
            self.refresh_outcome(index, None);
            return Some(self.observations[index].outcome.clone());
        }
        if self.observations[index]
            .seen
            .push(evidence.evidence_id)
            .is_err()
            || !correlates(&request, &evidence)
            || !source_allows(evidence.source_kind, &evidence.kind)
        {
            self.observations[index].summary.rejected =
                self.observations[index].summary.rejected.saturating_add(1);
            self.audit_for(
                &request,
                self.observations[index].state,
                ObservationAuditEvent::EvidenceRejected,
                now,
            );
            self.refresh_outcome(index, None);
            return Some(self.observations[index].outcome.clone());
        }
        if registry.generation(request.operation) != request.registry_generation
            || evidence.generation != request.registry_generation
        {
            let unchanged = registry
                .readiness_contract(request.operation, &request.target)
                .is_some_and(|contract| contract == request.readiness_contract);
            if !unchanged {
                self.observations[index].summary.accepted =
                    self.observations[index].summary.accepted.saturating_add(1);
                self.terminal(
                    index,
                    ObservationState::RegistryInvalidated,
                    ObservationAuditEvent::RegistryInvalidation,
                    now,
                );
                return Some(self.observations[index].outcome.clone());
            }
        }

        self.observations[index].summary.accepted =
            self.observations[index].summary.accepted.saturating_add(1);
        self.audit_for(
            &request,
            self.observations[index].state,
            ObservationAuditEvent::EvidenceAccepted,
            now,
        );
        self.apply_evidence(index, evidence.kind, now);
        Some(self.observations[index].outcome.clone())
    }

    pub fn tick(
        &mut self,
        execution_id: ExecutionId,
        now: AuthorityTime,
    ) -> Option<ObservedActionOutcome> {
        let index = self
            .observations
            .iter()
            .position(|record| record.request.execution_id == execution_id)?;
        if !self.observations[index].state.is_terminal()
            && now
                > self.observations[index]
                    .request
                    .deadlines
                    .for_state(self.observations[index].state)
        {
            self.terminal(
                index,
                ObservationState::TimedOut,
                ObservationAuditEvent::Timeout,
                now,
            );
        }
        Some(self.observations[index].outcome.clone())
    }

    pub fn cancel(
        &mut self,
        execution_id: ExecutionId,
        now: AuthorityTime,
    ) -> Option<ObservedActionOutcome> {
        self.external_terminal(
            execution_id,
            ObservationState::Cancelled,
            ObservationAuditEvent::Cancellation,
            now,
        )
    }

    pub fn invalidate_session(
        &mut self,
        execution_id: ExecutionId,
        session_id: SessionId,
        requester_authorized: bool,
        now: AuthorityTime,
    ) -> Option<ObservedActionOutcome> {
        let index = self
            .observations
            .iter()
            .position(|record| record.request.execution_id == execution_id)?;
        if self.observations[index].request.session_id != session_id || !requester_authorized {
            if !self.observations[index].state.is_terminal() {
                self.terminal(
                    index,
                    ObservationState::SessionInvalidated,
                    ObservationAuditEvent::SessionInvalidation,
                    now,
                );
            }
        }
        Some(self.observations[index].outcome.clone())
    }

    fn external_terminal(
        &mut self,
        execution_id: ExecutionId,
        state: ObservationState,
        event: ObservationAuditEvent,
        now: AuthorityTime,
    ) -> Option<ObservedActionOutcome> {
        let index = self
            .observations
            .iter()
            .position(|record| record.request.execution_id == execution_id)?;
        if !self.observations[index].state.is_terminal() {
            self.terminal(index, state, event, now);
        }
        Some(self.observations[index].outcome.clone())
    }

    fn apply_evidence(&mut self, index: usize, kind: ObservationEvidenceKind, now: AuthorityTime) {
        match kind {
            ObservationEvidenceKind::ProcessCreated { process_instance } => {
                self.observations[index].process_instance = Some(process_instance);
                self.advance(index, AchievedReadiness::ProcessRegistered, now);
            }
            ObservationEvidenceKind::ProcessExited {
                process_instance, ..
            } if self.observations[index].process_instance == Some(process_instance) => {
                self.terminal(
                    index,
                    ObservationState::ExitedEarly,
                    ObservationAuditEvent::EarlyExit,
                    now,
                );
                return;
            }
            ObservationEvidenceKind::ApplicationRegistered => {
                self.advance(index, AchievedReadiness::ApplicationRegistered, now);
            }
            ObservationEvidenceKind::WindowRegistered => {
                self.advance(index, AchievedReadiness::WindowRegistered, now);
            }
            ObservationEvidenceKind::SettingsPageActivated(page)
                if matches!(
                    self.observations[index].request.target,
                    ActionTarget::SettingsPage(ref expected) if *expected == page
                ) =>
            {
                self.advance(index, AchievedReadiness::PageActivated, now);
            }
            ObservationEvidenceKind::ReadySignal => {
                self.advance(index, AchievedReadiness::Ready, now);
            }
            ObservationEvidenceKind::Failed { .. } => {
                self.terminal(
                    index,
                    ObservationState::Failed,
                    ObservationAuditEvent::Failure,
                    now,
                );
                return;
            }
            ObservationEvidenceKind::SessionEnded => {
                self.terminal(
                    index,
                    ObservationState::SessionInvalidated,
                    ObservationAuditEvent::SessionInvalidation,
                    now,
                );
                return;
            }
            ObservationEvidenceKind::RegistryChanged => {
                self.terminal(
                    index,
                    ObservationState::RegistryInvalidated,
                    ObservationAuditEvent::RegistryInvalidation,
                    now,
                );
                return;
            }
            _ => {}
        }
        self.refresh_outcome(index, None);
    }

    fn advance(&mut self, index: usize, achieved: AchievedReadiness, now: AuthorityTime) {
        if achieved > self.observations[index].achieved {
            self.observations[index].achieved = achieved;
        }
        let contract = self.observations[index].request.readiness_contract;
        if contract_satisfied(contract, self.observations[index].achieved) {
            self.terminal(
                index,
                ObservationState::Ready,
                ObservationAuditEvent::ReadinessAchieved,
                now,
            );
            return;
        }
        let next = waiting_state(contract, self.observations[index].achieved);
        if state_rank(next) > state_rank(self.observations[index].state) {
            self.observations[index].state = next;
            let request = self.observations[index].request.clone();
            self.audit_for(&request, next, ObservationAuditEvent::StateTransition, now);
        }
    }

    fn terminal(
        &mut self,
        index: usize,
        state: ObservationState,
        event: ObservationAuditEvent,
        now: AuthorityTime,
    ) {
        if self.observations[index].state.is_terminal() {
            return;
        }
        self.observations[index].state = state;
        let request = self.observations[index].request.clone();
        self.audit_for(&request, state, event, now);
        self.audit_for(
            &request,
            state,
            ObservationAuditEvent::TerminalOutcomeProduced,
            now,
        );
        self.refresh_outcome(index, Some(now));
    }

    fn refresh_outcome(&mut self, index: usize, terminal_timestamp: Option<AuthorityTime>) {
        let record = &mut self.observations[index];
        let existing_terminal = record.outcome.terminal_timestamp;
        record.outcome = make_outcome(
            &record.request,
            record.state,
            record.achieved,
            record.summary,
            existing_terminal.or(terminal_timestamp),
        );
    }

    fn audit_for(
        &mut self,
        request: &ObservationRequest,
        state: ObservationState,
        event: ObservationAuditEvent,
        timestamp: AuthorityTime,
    ) {
        self.audit.push(ObservationAuditEntry {
            observation_id: request.observation_id,
            execution_id: request.execution_id,
            operation: request.operation,
            readiness: request.readiness_contract,
            state,
            event,
            timestamp,
        });
    }
}

impl<const CAPACITY: usize, const AUDIT: usize, const REPLAY: usize> Default
    for ActionOutcomeObserver<CAPACITY, AUDIT, REPLAY>
{
    fn default() -> Self {
        Self::new()
    }
}

fn initial_state(contract: ReadinessContract) -> ObservationState {
    match contract {
        ReadinessContract::DispatchAccepted | ReadinessContract::ControlPanelDispatched => {
            ObservationState::Ready
        }
        _ => ObservationState::WaitingForProcess,
    }
}

fn waiting_state(contract: ReadinessContract, achieved: AchievedReadiness) -> ObservationState {
    match contract {
        ReadinessContract::ProcessRegistered => ObservationState::WaitingForProcess,
        ReadinessContract::ApplicationRegistered | ReadinessContract::ApplicationReady => {
            if achieved < AchievedReadiness::ProcessRegistered {
                ObservationState::WaitingForProcess
            } else {
                ObservationState::WaitingForRegistration
            }
        }
        ReadinessContract::FirstWindowRegistered
        | ReadinessContract::ControlPanelWindowRegistered => {
            if achieved < AchievedReadiness::ProcessRegistered {
                ObservationState::WaitingForProcess
            } else {
                ObservationState::WaitingForWindow
            }
        }
        ReadinessContract::RequestedPageActivated | ReadinessContract::PageReady => {
            if achieved < AchievedReadiness::ProcessRegistered {
                ObservationState::WaitingForProcess
            } else {
                ObservationState::WaitingForPage
            }
        }
        _ => ObservationState::Created,
    }
}

fn state_rank(state: ObservationState) -> u8 {
    match state {
        ObservationState::Created => 0,
        ObservationState::WaitingForProcess => 1,
        ObservationState::WaitingForRegistration => 2,
        ObservationState::WaitingForWindow | ObservationState::WaitingForPage => 3,
        _ => 4,
    }
}

fn contract_satisfied(contract: ReadinessContract, achieved: AchievedReadiness) -> bool {
    match contract {
        ReadinessContract::DispatchAccepted | ReadinessContract::ControlPanelDispatched => {
            achieved >= AchievedReadiness::DispatchAccepted
        }
        ReadinessContract::ProcessRegistered => achieved >= AchievedReadiness::ProcessRegistered,
        ReadinessContract::ApplicationRegistered => {
            achieved >= AchievedReadiness::ApplicationRegistered
        }
        ReadinessContract::FirstWindowRegistered
        | ReadinessContract::ControlPanelWindowRegistered => {
            achieved >= AchievedReadiness::WindowRegistered
        }
        ReadinessContract::ApplicationReady | ReadinessContract::PageReady => {
            achieved >= AchievedReadiness::Ready
        }
        ReadinessContract::RequestedPageActivated => achieved >= AchievedReadiness::PageActivated,
    }
}

fn contract_matches_operation(contract: ReadinessContract, operation: ActionOperation) -> bool {
    match operation {
        ActionOperation::OpenApplication => matches!(
            contract,
            ReadinessContract::DispatchAccepted
                | ReadinessContract::ProcessRegistered
                | ReadinessContract::ApplicationRegistered
                | ReadinessContract::FirstWindowRegistered
                | ReadinessContract::ApplicationReady
        ),
        ActionOperation::OpenSettingsPage => matches!(
            contract,
            ReadinessContract::ControlPanelDispatched
                | ReadinessContract::ControlPanelWindowRegistered
                | ReadinessContract::RequestedPageActivated
                | ReadinessContract::PageReady
        ),
        _ => false,
    }
}

fn correlates(request: &ObservationRequest, evidence: &ObservationEvidence) -> bool {
    request.execution_id == evidence.execution_id
        && request.correlation_token == evidence.correlation_token
        && request.session_id == evidence.session_id
        && request.target == evidence.target
        && evidence.timestamp >= request.dispatch_timestamp
}

fn source_allows(source: TrustedSourceKind, evidence: &ObservationEvidenceKind) -> bool {
    matches!(
        (source, evidence),
        (
            TrustedSourceKind::TrustedExecutor,
            ObservationEvidenceKind::DispatchAccepted
        ) | (
            TrustedSourceKind::ProcessLifecycle,
            ObservationEvidenceKind::ProcessCreated { .. }
                | ObservationEvidenceKind::ProcessExited { .. }
        ) | (
            TrustedSourceKind::ApplicationRegistry,
            ObservationEvidenceKind::ApplicationRegistered
                | ObservationEvidenceKind::RegistryChanged
                | ObservationEvidenceKind::ReadySignal
                | ObservationEvidenceKind::Failed { .. }
        ) | (
            TrustedSourceKind::DisplayServer,
            ObservationEvidenceKind::WindowRegistered
        ) | (
            TrustedSourceKind::ControlPanel,
            ObservationEvidenceKind::SettingsPageActivated(_)
                | ObservationEvidenceKind::ReadySignal
                | ObservationEvidenceKind::Failed { .. }
        ) | (
            TrustedSourceKind::SessionManager,
            ObservationEvidenceKind::SessionEnded
        ) | (
            TrustedSourceKind::Coordinator,
            ObservationEvidenceKind::Unknown
        )
    )
}

fn make_outcome(
    request: &ObservationRequest,
    state: ObservationState,
    achieved: AchievedReadiness,
    summary: EvidenceSummary,
    terminal_timestamp: Option<AuthorityTime>,
) -> ObservedActionOutcome {
    use ObservedActionOutcomeKind as Kind;
    use PublicOutcomeCode as Code;
    let (kind, public_result_code) = match state {
        ObservationState::Ready => (Kind::Ready, Code::Ready),
        ObservationState::Failed => (Kind::Failed, Code::Failed),
        ObservationState::ExitedEarly => (Kind::ExitedEarly, Code::ExitedBeforeReady),
        ObservationState::TimedOut => (Kind::TimedOut, Code::ReadinessTimedOut),
        ObservationState::SessionInvalidated => {
            (Kind::SessionInvalidated, Code::SessionInvalidated)
        }
        ObservationState::RegistryInvalidated => {
            (Kind::RegistryInvalidated, Code::RegistryInvalidated)
        }
        ObservationState::Cancelled => (Kind::Cancelled, Code::ObservationCancelled),
        _ => (Kind::DispatchOnly, Code::DispatchAccepted),
    };
    ObservedActionOutcome {
        observation_id: request.observation_id,
        execution_id: request.execution_id,
        intent_id: request.intent_id,
        target_kind: request.target.kind(),
        kind,
        public_result_code,
        achieved_readiness: achieved,
        terminal_timestamp,
        evidence_summary: summary,
        audit_id: request.audit_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_intent::{
        ActionEvaluation, ActionIntent, ActionIntentEvaluator, ActionParameters, CreationTime,
        Provenance, RiskHint,
    };
    use crate::confirmation::{
        ConfirmationAuthority, ResponderIdentity, SessionAuthorization, SessionStatus,
    };
    use crate::executor::{
        ActionExecutor, DispatchStatus, ExecutionContext, LaunchApplicationRequest,
        OpenSettingsPageRequest, RegistryStatus, TrustedActionExecutor, TrustedLaunchAdapter,
    };
    use crate::policy::{PolicyEngine, PolicyResult};
    use crate::runtime_context::RuntimeContextSnapshot;

    struct FakeLaunch;
    impl TrustedLaunchAdapter for FakeLaunch {
        fn application_status(&self, _: &TypedIdentifier) -> RegistryStatus {
            RegistryStatus::Registered
        }
        fn settings_page_status(&self, _: &TypedIdentifier) -> RegistryStatus {
            RegistryStatus::Registered
        }
        fn launch_application(&mut self, _: LaunchApplicationRequest) -> DispatchStatus {
            DispatchStatus::Accepted
        }
        fn open_settings_page(&mut self, _: OpenSettingsPageRequest) -> DispatchStatus {
            DispatchStatus::Accepted
        }
    }

    #[derive(Clone)]
    struct Registry {
        generation: u64,
        contract: ReadinessContract,
    }
    impl OutcomeRegistry for Registry {
        fn generation(&self, _: ActionOperation) -> u64 {
            self.generation
        }
        fn readiness_contract(
            &self,
            _: ActionOperation,
            _: &ActionTarget,
        ) -> Option<ReadinessContract> {
            Some(self.contract)
        }
    }

    fn accepted(operation: ActionOperation, target: ActionTarget) -> ExecutionResult {
        let parameters = match operation {
            ActionOperation::OpenApplication => ActionParameters::Application {
                new_instance: false,
            },
            ActionOperation::OpenSettingsPage => ActionParameters::Settings { focus: None },
            ActionOperation::Observe => ActionParameters::Observe {
                include_health: false,
            },
            _ => ActionParameters::None,
        };
        let intent = ActionIntent::new(
            IntentId::new([7; 16]),
            operation,
            target,
            parameters,
            RequestedBy::User(42),
            SessionId(9),
            4,
            CreationTime(100),
            RiskHint::Low,
            Provenance::ExplicitUserRequest,
        );
        let mut runtime = RuntimeContextSnapshot {
            available: true,
            generation: 4,
            captured_mono_ms: 100,
            ..RuntimeContextSnapshot::default()
        };
        runtime.session.desktop_mode = Some(true);
        runtime.session.installer_mode = Some(false);
        runtime.session.recovery_mode = Some(false);
        let policy = PolicyEngine::v1();
        let ActionEvaluation::Decided(decision) =
            ActionIntentEvaluator::<8>::new(policy).evaluate(&intent, &runtime)
        else {
            panic!("decision")
        };
        assert_eq!(decision.result(), PolicyResult::Allowed);
        let ready = ConfirmationAuthority::<8, 8>::new(policy, 1_000)
            .produce_ready(
                &intent,
                &decision,
                None,
                &runtime,
                SessionAuthorization::new(
                    SessionId(9),
                    ResponderIdentity::User(42),
                    SessionStatus::Active,
                ),
                AuthorityTime(110),
            )
            .unwrap();
        TrustedActionExecutor::<FakeLaunch, 16, 8>::new(FakeLaunch).execute(
            ready,
            &ExecutionContext::new(
                &runtime,
                &policy,
                SessionAuthorization::new(
                    SessionId(9),
                    ResponderIdentity::User(42),
                    SessionStatus::Active,
                ),
                RequestedBy::User(42),
                AuthorityTime(120),
            ),
        )
    }

    fn setup(
        operation: ActionOperation,
        id: &str,
        contract: ReadinessContract,
    ) -> (
        ActionOutcomeObserver<4, 12, 8>,
        Registry,
        ObservationRequest,
    ) {
        let target = if operation == ActionOperation::OpenApplication {
            ActionTarget::Application(TypedIdentifier::new(id).unwrap())
        } else {
            ActionTarget::SettingsPage(TypedIdentifier::new(id).unwrap())
        };
        let execution = accepted(operation, target);
        let registry = Registry {
            generation: 4,
            contract,
        };
        let request = ObservationRequest::from_execution(
            ObservationId(32),
            &execution,
            &registry,
            ObservationDeadlines::uniform(AuthorityTime(200)),
        )
        .unwrap();
        (ActionOutcomeObserver::new(), registry, request)
    }

    fn evidence(
        request: &ObservationRequest,
        id: u64,
        source: TrustedSourceKind,
        kind: ObservationEvidenceKind,
    ) -> ObservationEvidence {
        ObservationEvidence::trusted(
            EvidenceId(id),
            source,
            TypedIdentifier::new("trusted-source").unwrap(),
            request.session_id,
            AuthorityTime(130 + id),
            request.registry_generation,
            request.execution_id,
            request.correlation_token,
            request.target.clone(),
            kind,
        )
    }

    #[test]
    fn dispatch_acceptance_is_not_application_readiness() {
        let (mut observer, _, request) = setup(
            ActionOperation::OpenApplication,
            "calculator",
            ReadinessContract::FirstWindowRegistered,
        );
        let outcome = observer.create(request.clone()).unwrap();
        assert_eq!(outcome.kind(), ObservedActionOutcomeKind::DispatchOnly);
        assert_eq!(
            observer.state(request.execution_id),
            Some(ObservationState::WaitingForProcess)
        );
    }

    #[test]
    fn process_window_and_background_contracts_are_distinct() {
        let (mut observer, registry, request) = setup(
            ActionOperation::OpenApplication,
            "calculator",
            ReadinessContract::FirstWindowRegistered,
        );
        observer.create(request.clone()).unwrap();
        let process = evidence(
            &request,
            1,
            TrustedSourceKind::ProcessLifecycle,
            ObservationEvidenceKind::ProcessCreated {
                process_instance: 77,
            },
        );
        assert_eq!(
            observer.observe(process, &registry).unwrap().kind(),
            ObservedActionOutcomeKind::DispatchOnly
        );
        let window = evidence(
            &request,
            2,
            TrustedSourceKind::DisplayServer,
            ObservationEvidenceKind::WindowRegistered,
        );
        assert_eq!(
            observer.observe(window, &registry).unwrap().kind(),
            ObservedActionOutcomeKind::Ready
        );

        let (mut background, registry, request) = setup(
            ActionOperation::OpenApplication,
            "indexer",
            ReadinessContract::ApplicationRegistered,
        );
        background.create(request.clone()).unwrap();
        let registered = evidence(
            &request,
            3,
            TrustedSourceKind::ApplicationRegistry,
            ObservationEvidenceKind::ApplicationRegistered,
        );
        assert_eq!(
            background.observe(registered, &registry).unwrap().kind(),
            ObservedActionOutcomeKind::Ready
        );
    }

    #[test]
    fn exact_settings_page_activation_is_required() {
        let (mut observer, registry, request) = setup(
            ActionOperation::OpenSettingsPage,
            "network",
            ReadinessContract::RequestedPageActivated,
        );
        observer.create(request.clone()).unwrap();
        let wrong = evidence(
            &request,
            1,
            TrustedSourceKind::ControlPanel,
            ObservationEvidenceKind::SettingsPageActivated(
                TypedIdentifier::new("display").unwrap(),
            ),
        );
        assert_eq!(
            observer.observe(wrong, &registry).unwrap().kind(),
            ObservedActionOutcomeKind::DispatchOnly
        );
        let exact = evidence(
            &request,
            2,
            TrustedSourceKind::ControlPanel,
            ObservationEvidenceKind::SettingsPageActivated(
                TypedIdentifier::new("network").unwrap(),
            ),
        );
        assert_eq!(
            observer.observe(exact, &registry).unwrap().kind(),
            ObservedActionOutcomeKind::Ready
        );
    }

    #[test]
    fn wrong_session_target_token_and_pid_reuse_cannot_complete() {
        let (mut observer, registry, request) = setup(
            ActionOperation::OpenApplication,
            "calculator",
            ReadinessContract::FirstWindowRegistered,
        );
        observer.create(request.clone()).unwrap();
        let mut wrong_session = evidence(
            &request,
            1,
            TrustedSourceKind::DisplayServer,
            ObservationEvidenceKind::WindowRegistered,
        );
        wrong_session.session_id = SessionId(10);
        observer.observe(wrong_session, &registry);
        let mut wrong_target = evidence(
            &request,
            2,
            TrustedSourceKind::DisplayServer,
            ObservationEvidenceKind::WindowRegistered,
        );
        wrong_target.target = ActionTarget::Application(TypedIdentifier::new("terminal").unwrap());
        observer.observe(wrong_target, &registry);
        let mut wrong_token = evidence(
            &request,
            3,
            TrustedSourceKind::DisplayServer,
            ObservationEvidenceKind::WindowRegistered,
        );
        wrong_token.correlation_token = accepted(
            ActionOperation::OpenApplication,
            ActionTarget::Application(TypedIdentifier::new("terminal").unwrap()),
        )
        .correlation_token()
        .unwrap();
        observer.observe(wrong_token, &registry);
        let reused_exit = evidence(
            &request,
            4,
            TrustedSourceKind::ProcessLifecycle,
            ObservationEvidenceKind::ProcessExited {
                process_instance: 77,
                public_code: 1,
            },
        );
        observer.observe(reused_exit, &registry);
        assert_eq!(
            observer.state(request.execution_id),
            Some(ObservationState::WaitingForProcess)
        );
        assert_eq!(
            observer
                .outcome(request.execution_id)
                .unwrap()
                .evidence_summary()
                .rejected,
            3
        );
    }

    #[test]
    fn early_exit_timeout_and_late_event_are_terminal() {
        let (mut observer, registry, request) = setup(
            ActionOperation::OpenApplication,
            "calculator",
            ReadinessContract::FirstWindowRegistered,
        );
        observer.create(request.clone()).unwrap();
        observer.observe(
            evidence(
                &request,
                1,
                TrustedSourceKind::ProcessLifecycle,
                ObservationEvidenceKind::ProcessCreated {
                    process_instance: 5,
                },
            ),
            &registry,
        );
        let exited = observer
            .observe(
                evidence(
                    &request,
                    2,
                    TrustedSourceKind::ProcessLifecycle,
                    ObservationEvidenceKind::ProcessExited {
                        process_instance: 5,
                        public_code: 2,
                    },
                ),
                &registry,
            )
            .unwrap();
        assert_eq!(exited.kind(), ObservedActionOutcomeKind::ExitedEarly);

        let (mut timeout, registry, request) = setup(
            ActionOperation::OpenApplication,
            "calculator",
            ReadinessContract::FirstWindowRegistered,
        );
        timeout.create(request.clone()).unwrap();
        assert_eq!(
            timeout
                .tick(request.execution_id, AuthorityTime(201))
                .unwrap()
                .kind(),
            ObservedActionOutcomeKind::TimedOut
        );
        let late = timeout
            .observe(
                evidence(
                    &request,
                    3,
                    TrustedSourceKind::DisplayServer,
                    ObservationEvidenceKind::WindowRegistered,
                ),
                &registry,
            )
            .unwrap();
        assert_eq!(late.kind(), ObservedActionOutcomeKind::TimedOut);
        assert_eq!(late.evidence_summary().late, 1);
    }

    #[test]
    fn replay_creation_session_registry_and_cancellation_fail_closed() {
        let (mut observer, registry, request) = setup(
            ActionOperation::OpenApplication,
            "calculator",
            ReadinessContract::ApplicationRegistered,
        );
        observer.create(request.clone()).unwrap();
        assert_eq!(
            observer.create(request.clone()).unwrap_err(),
            ObservationCreateError::DuplicateExecution
        );
        let item = evidence(
            &request,
            1,
            TrustedSourceKind::ApplicationRegistry,
            ObservationEvidenceKind::ApplicationRegistered,
        );
        observer.observe(item.clone(), &registry);
        let stable = observer.observe(item, &registry).unwrap();
        assert_eq!(stable.kind(), ObservedActionOutcomeKind::Ready);

        let (mut cancelled, _, request) = setup(
            ActionOperation::OpenApplication,
            "calculator",
            ReadinessContract::ApplicationReady,
        );
        cancelled.create(request.clone()).unwrap();
        assert_eq!(
            cancelled
                .cancel(request.execution_id, AuthorityTime(140))
                .unwrap()
                .kind(),
            ObservedActionOutcomeKind::Cancelled
        );

        let (mut session, _, request) = setup(
            ActionOperation::OpenApplication,
            "calculator",
            ReadinessContract::ApplicationReady,
        );
        session.create(request.clone()).unwrap();
        assert_eq!(
            session
                .invalidate_session(
                    request.execution_id,
                    SessionId(99),
                    false,
                    AuthorityTime(140),
                )
                .unwrap()
                .kind(),
            ObservedActionOutcomeKind::SessionInvalidated
        );
    }

    #[test]
    fn audit_is_bounded_and_contains_only_redacted_typed_metadata() {
        let (mut observer, registry, request) = setup(
            ActionOperation::OpenApplication,
            "calculator",
            ReadinessContract::ApplicationReady,
        );
        observer.create(request.clone()).unwrap();
        for id in 1..20 {
            let mut item = evidence(
                &request,
                id,
                TrustedSourceKind::DisplayServer,
                ObservationEvidenceKind::WindowRegistered,
            );
            item.session_id = SessionId(999);
            observer.observe(item, &registry);
        }
        assert_eq!(observer.audit().len(), 12);
        assert!(observer.audit().evicted() > 0);
        let debug = format!(
            "{:?}",
            observer.audit().entries().collect::<std::vec::Vec<_>>()
        );
        assert!(!debug.contains("calculator"));
        assert!(!debug.contains("window title"));
        assert!(!debug.contains('/'));
    }

    #[test]
    fn registry_contract_change_and_unsupported_operation_fail_closed() {
        let (mut observer, _, request) = setup(
            ActionOperation::OpenApplication,
            "calculator",
            ReadinessContract::FirstWindowRegistered,
        );
        observer.create(request.clone()).unwrap();
        let changed = Registry {
            generation: 5,
            contract: ReadinessContract::ApplicationRegistered,
        };
        let outcome = observer
            .observe(
                evidence(
                    &request,
                    1,
                    TrustedSourceKind::ApplicationRegistry,
                    ObservationEvidenceKind::RegistryChanged,
                ),
                &changed,
            )
            .unwrap();
        assert_eq!(
            outcome.kind(),
            ObservedActionOutcomeKind::RegistryInvalidated
        );

        let unsupported = accepted(ActionOperation::Observe, ActionTarget::System);
        assert_eq!(
            ObservationRequest::from_execution(
                ObservationId(90),
                &unsupported,
                &changed,
                ObservationDeadlines::uniform(AuthorityTime(300)),
            )
            .unwrap_err(),
            ObservationCreateError::UnsupportedObservation
        );
    }
}
