//! Exact, session-bound, non-executing confirmation authority.
//!
//! This module issues presentation-safe challenges, validates typed responses,
//! creates exact single-use grants, and performs final fail-closed readiness
//! checks. It deliberately contains no executor, service handle, IPC endpoint,
//! syscall, command string, callback, or execution result.

use core::fmt;

use heapless::{Deque, String, Vec};
use sha2::{Digest, Sha256};

use crate::action_intent::{
    validate, ActionDecision, ActionIntent, ActionOperation, ActionParameters, ActionTarget,
    AuditId, IntentId, IntentValidation, Provenance, RequestedBy, SessionId, TargetKind,
};
use crate::policy::{ConfirmationLevel, PolicyEngine, PolicyResult, PolicyVersion};
use crate::runtime_context::RuntimeContextSnapshot;

pub const MAX_CONFIRMATION_TEXT_LEN: usize = 160;
pub const DEFAULT_CONFIRMATION_CAPACITY: usize = 16;
pub const DEFAULT_CONFIRMATION_AUDIT_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChallengeId([u8; 16]);

impl ChallengeId {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GrantId([u8; 16]);

impl GrantId {
    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConfirmationNonce([u8; 16]);

impl ConfirmationNonce {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingDigest([u8; 32]);

impl BindingDigest {
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AuthorityTime(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResponderIdentity {
    User(u32),
    TrustedUi(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Active,
    Inactive,
    Unknown,
}

/// Trusted, current session authorization supplied to final revalidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionAuthorization {
    session_id: SessionId,
    responder: ResponderIdentity,
    status: SessionStatus,
}

impl SessionAuthorization {
    pub const fn new(
        session_id: SessionId,
        responder: ResponderIdentity,
        status: SessionStatus,
    ) -> Self {
        Self {
            session_id,
            responder,
            status,
        }
    }

    pub const fn session_id(self) -> SessionId {
        self.session_id
    }

    pub const fn responder(self) -> ResponderIdentity {
        self.responder
    }

    pub const fn status(self) -> SessionStatus {
        self.status
    }
}

/// Trusted current inputs checked while accepting an approval response.
#[derive(Clone, Copy)]
pub struct ResponseValidationContext<'a> {
    intent: &'a ActionIntent,
    runtime: &'a RuntimeContextSnapshot,
    session: SessionAuthorization,
}

impl<'a> ResponseValidationContext<'a> {
    pub const fn new(
        intent: &'a ActionIntent,
        runtime: &'a RuntimeContextSnapshot,
        session: SessionAuthorization,
    ) -> Self {
        Self {
            intent,
            runtime,
            session,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationChoice {
    Approve,
    Reject,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationView {
    title: String<MAX_CONFIRMATION_TEXT_LEN>,
    action_summary: String<MAX_CONFIRMATION_TEXT_LEN>,
    target_summary: String<MAX_CONFIRMATION_TEXT_LEN>,
    consequence_summary: String<MAX_CONFIRMATION_TEXT_LEN>,
    confirmation_level: ConfirmationLevel,
    available_choices: Vec<ConfirmationChoice, 3>,
    expires_at: AuthorityTime,
}

impl ConfirmationView {
    pub fn title(&self) -> &str {
        self.title.as_str()
    }
    pub fn action_summary(&self) -> &str {
        self.action_summary.as_str()
    }
    pub fn target_summary(&self) -> &str {
        self.target_summary.as_str()
    }
    pub fn consequence_summary(&self) -> &str {
        self.consequence_summary.as_str()
    }
    pub const fn confirmation_level(&self) -> ConfirmationLevel {
        self.confirmation_level
    }
    pub fn available_choices(&self) -> &[ConfirmationChoice] {
        self.available_choices.as_slice()
    }
    /// v1 never supplies a default choice, especially not approval.
    pub const fn default_choice(&self) -> Option<ConfirmationChoice> {
        None
    }
    pub const fn expires_at(&self) -> AuthorityTime {
        self.expires_at
    }
}

/// Immutable challenge. Private fields prevent a UI from changing its binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationChallenge {
    challenge_id: ChallengeId,
    intent_id: IntentId,
    confirmation_level: ConfirmationLevel,
    public_action_summary: String<MAX_CONFIRMATION_TEXT_LEN>,
    public_risk_summary: String<MAX_CONFIRMATION_TEXT_LEN>,
    target_summary: String<MAX_CONFIRMATION_TEXT_LEN>,
    policy_version: PolicyVersion,
    runtime_snapshot_generation: u64,
    session_id: SessionId,
    requested_by: RequestedBy,
    issued_at: AuthorityTime,
    expires_at: AuthorityTime,
    nonce: ConfirmationNonce,
    target_digest: BindingDigest,
}

impl ConfirmationChallenge {
    pub const fn challenge_id(&self) -> ChallengeId {
        self.challenge_id
    }
    pub const fn intent_id(&self) -> IntentId {
        self.intent_id
    }
    pub const fn confirmation_level(&self) -> ConfirmationLevel {
        self.confirmation_level
    }
    pub fn public_action_summary(&self) -> &str {
        self.public_action_summary.as_str()
    }
    pub fn public_risk_summary(&self) -> &str {
        self.public_risk_summary.as_str()
    }
    pub fn target_summary(&self) -> &str {
        self.target_summary.as_str()
    }
    pub const fn policy_version(&self) -> PolicyVersion {
        self.policy_version
    }
    pub const fn runtime_snapshot_generation(&self) -> u64 {
        self.runtime_snapshot_generation
    }
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub const fn nonce(&self) -> ConfirmationNonce {
        self.nonce
    }
    pub const fn requested_by(&self) -> RequestedBy {
        self.requested_by
    }
    pub const fn issued_at(&self) -> AuthorityTime {
        self.issued_at
    }
    pub const fn target_digest(&self) -> BindingDigest {
        self.target_digest
    }
    pub const fn expires_at(&self) -> AuthorityTime {
        self.expires_at
    }
    pub fn view(&self) -> ConfirmationView {
        let mut choices = Vec::new();
        let _ = choices.push(ConfirmationChoice::Approve);
        let _ = choices.push(ConfirmationChoice::Reject);
        let _ = choices.push(ConfirmationChoice::Cancel);
        ConfirmationView {
            title: bounded_text(match self.confirmation_level {
                ConfirmationLevel::Soft => "Confirm action",
                ConfirmationLevel::Strong => "Strong confirmation required",
                ConfirmationLevel::Critical => "Critical confirmation required",
                ConfirmationLevel::None => "Confirmation unavailable",
            }),
            action_summary: self.public_action_summary.clone(),
            target_summary: self.target_summary.clone(),
            consequence_summary: self.public_risk_summary.clone(),
            confirmation_level: self.confirmation_level,
            available_choices: choices,
            expires_at: self.expires_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsequenceAcknowledgement {
    Acknowledged,
}

/// Closed proof taxonomy. There is no arbitrary string or boolean proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalProof {
    SoftExplicit,
    Strong {
        nonce: ConfirmationNonce,
    },
    Critical {
        nonce: ConfirmationNonce,
        exact_target: BindingDigest,
        consequence: ConsequenceAcknowledgement,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationResponseType {
    Approved(ApprovalProof),
    Rejected,
    Cancelled,
    Expired,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmationResponse {
    challenge_id: ChallengeId,
    session_id: SessionId,
    responder: ResponderIdentity,
    response_type: ConfirmationResponseType,
    submitted_at: AuthorityTime,
}

impl ConfirmationResponse {
    pub const fn new(
        challenge_id: ChallengeId,
        session_id: SessionId,
        responder: ResponderIdentity,
        response_type: ConfirmationResponseType,
        submitted_at: AuthorityTime,
    ) -> Self {
        Self {
            challenge_id,
            session_id,
            responder,
            response_type,
            submitted_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfirmationState {
    Created,
    Issued,
    Approved,
    Rejected,
    Cancelled,
    Expired,
    /// The exact grant has been reserved by a readiness envelope. Dispatch is
    /// still pending; executor replay protection performs final consumption.
    Ready,
    Consumed,
    Invalidated,
}

/// Immutable exact grant. Single use is enforced by its issuing authority.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfirmationGrant {
    grant_id: GrantId,
    challenge_id: ChallengeId,
    intent_id: IntentId,
    intent_digest: BindingDigest,
    operation: ActionOperation,
    target: ActionTarget,
    parameters_digest: BindingDigest,
    provenance: Provenance,
    policy_version: PolicyVersion,
    runtime_snapshot_generation: u64,
    session_id: SessionId,
    responder: ResponderIdentity,
    confirmation_level: ConfirmationLevel,
    issued_at: AuthorityTime,
    expires_at: AuthorityTime,
}

impl fmt::Debug for ConfirmationGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfirmationGrant")
            .field("grant_id", &self.grant_id)
            .field("challenge_id", &self.challenge_id)
            .field("intent_id", &self.intent_id)
            .field("operation", &self.operation)
            .field("target_kind", &self.target.kind())
            .field("policy_version", &self.policy_version)
            .field(
                "runtime_snapshot_generation",
                &self.runtime_snapshot_generation,
            )
            .field("session_id", &self.session_id)
            .field("responder", &self.responder)
            .field("confirmation_level", &self.confirmation_level)
            .finish_non_exhaustive()
    }
}

impl ConfirmationGrant {
    pub const fn grant_id(&self) -> GrantId {
        self.grant_id
    }
    pub const fn challenge_id(&self) -> ChallengeId {
        self.challenge_id
    }
    pub const fn intent_id(&self) -> IntentId {
        self.intent_id
    }
    pub const fn confirmation_level(&self) -> ConfirmationLevel {
        self.confirmation_level
    }
    pub const fn responder(&self) -> ResponderIdentity {
        self.responder
    }
    pub const fn issued_at(&self) -> AuthorityTime {
        self.issued_at
    }
    pub const fn expires_at(&self) -> AuthorityTime {
        self.expires_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalValidationResult {
    Valid,
}

/// Capability-neutral data only. `Consumed` means made ready, not executed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyForExecution {
    intent: ActionIntent,
    decision: ActionDecision,
    confirmation_grant: Option<ConfirmationGrant>,
    final_validation_result: FinalValidationResult,
    policy_version: PolicyVersion,
    runtime_snapshot_generation: u64,
    readiness_timestamp: AuthorityTime,
    audit_id: AuditId,
}

impl ReadyForExecution {
    pub fn intent(&self) -> &ActionIntent {
        &self.intent
    }
    pub fn decision(&self) -> &ActionDecision {
        &self.decision
    }
    pub fn confirmation_grant(&self) -> Option<&ConfirmationGrant> {
        self.confirmation_grant.as_ref()
    }
    pub const fn final_validation_result(&self) -> FinalValidationResult {
        self.final_validation_result
    }
    pub const fn audit_id(&self) -> AuditId {
        self.audit_id
    }
    pub const fn policy_version(&self) -> PolicyVersion {
        self.policy_version
    }
    pub const fn runtime_snapshot_generation(&self) -> u64 {
        self.runtime_snapshot_generation
    }
    pub const fn readiness_timestamp(&self) -> AuthorityTime {
        self.readiness_timestamp
    }

    /// Recomputes all bindings held by this immutable envelope. This is used
    /// immediately before dispatch; it does not consult mutable runtime state.
    pub(crate) fn has_valid_integrity(&self) -> bool {
        if self.final_validation_result != FinalValidationResult::Valid
            || self.policy_version != self.decision.policy_version()
            || self.runtime_snapshot_generation != self.decision.runtime_snapshot_generation()
            || !decision_matches_intent(&self.decision, &self.intent)
        {
            return false;
        }

        match self.decision.result() {
            PolicyResult::Allowed => self.confirmation_grant.is_none(),
            PolicyResult::ConfirmationRequired => {
                let Some(grant) = self.confirmation_grant.as_ref() else {
                    return false;
                };
                grant.intent_id == self.intent.intent_id()
                    && grant.intent_digest == digest_intent(&self.intent)
                    && grant.operation == self.intent.operation()
                    && grant.target == *self.intent.target()
                    && grant.parameters_digest == digest_parameters(self.intent.parameters())
                    && grant.provenance == self.intent.provenance()
                    && grant.policy_version == self.policy_version
                    && grant.runtime_snapshot_generation == self.runtime_snapshot_generation
                    && grant.session_id == self.intent.session_id()
            }
            PolicyResult::Denied | PolicyResult::Unknown => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessDenialReason {
    IntentInvalid,
    IntentChanged,
    DecisionChanged,
    PolicyChanged,
    PolicyDenied,
    RuntimeStale,
    SessionInactive,
    WrongSession,
    WrongResponder,
    ChallengeExpired,
    GrantMissing,
    GrantChanged,
    GrantConsumed,
    ConfirmationLevelInsufficient,
    UnknownState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityError {
    Capacity,
    ChallengeNotFound,
    ChallengeNotRequired,
    ChallengeNotIssued,
    AlreadyFinal,
    Expired,
    WrongSession,
    WrongResponder,
    InvalidProof,
    Replay,
    Denied(ReadinessDenialReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationAuditEvent {
    ChallengeCreated {
        challenge_id: ChallengeId,
        intent_id: IntentId,
        level: ConfirmationLevel,
        target_kind: TargetKind,
    },
    ChallengeIssued {
        challenge_id: ChallengeId,
    },
    ResponseAccepted {
        challenge_id: ChallengeId,
        state: ConfirmationState,
    },
    ResponseRejected {
        challenge_id: ChallengeId,
    },
    GrantCreated {
        challenge_id: ChallengeId,
        grant_id: GrantId,
    },
    GrantInvalidated {
        challenge_id: ChallengeId,
        reason: ReadinessDenialReason,
    },
    GrantReserved {
        challenge_id: ChallengeId,
        grant_id: GrantId,
    },
    ReadinessProduced {
        intent_id: IntentId,
        audit_id: AuditId,
    },
    ReadinessDenied {
        intent_id: IntentId,
        reason: ReadinessDenialReason,
    },
}

pub struct ConfirmationAuditLog<const N: usize = DEFAULT_CONFIRMATION_AUDIT_CAPACITY> {
    entries: Deque<ConfirmationAuditEvent, N>,
    evicted: u64,
}

impl<const N: usize> ConfirmationAuditLog<N> {
    pub const fn new() -> Self {
        Self {
            entries: Deque::new(),
            evicted: 0,
        }
    }
    pub fn entries(&self) -> impl Iterator<Item = &ConfirmationAuditEvent> {
        self.entries.iter()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub const fn evicted(&self) -> u64 {
        self.evicted
    }
    fn record(&mut self, event: ConfirmationAuditEvent) {
        if N == 0 {
            self.evicted = self.evicted.saturating_add(1);
            return;
        }
        if self.entries.is_full() {
            let _ = self.entries.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
        let _ = self.entries.push_back(event);
    }
}

impl<const N: usize> Default for ConfirmationAuditLog<N> {
    fn default() -> Self {
        Self::new()
    }
}

struct ChallengeRecord {
    challenge: ConfirmationChallenge,
    intent_digest: BindingDigest,
    operation: ActionOperation,
    target: ActionTarget,
    parameters_digest: BindingDigest,
    provenance: Provenance,
    authorized_responder: ResponderIdentity,
    state: ConfirmationState,
    grant: Option<ConfirmationGrant>,
}

/// Bounded owner of challenge lifecycle, replay state, grants, and audit.
pub struct ConfirmationAuthority<
    const N: usize = DEFAULT_CONFIRMATION_CAPACITY,
    const A: usize = DEFAULT_CONFIRMATION_AUDIT_CAPACITY,
> {
    policy: PolicyEngine,
    records: Vec<ChallengeRecord, N>,
    audit: ConfirmationAuditLog<A>,
    max_runtime_age_ms: u64,
    next_audit_id: u64,
}

impl<const N: usize, const A: usize> ConfirmationAuthority<N, A> {
    pub const fn new(policy: PolicyEngine, max_runtime_age_ms: u64) -> Self {
        Self {
            policy,
            records: Vec::new(),
            audit: ConfirmationAuditLog::new(),
            max_runtime_age_ms,
            next_audit_id: 1,
        }
    }

    pub const fn audit(&self) -> &ConfirmationAuditLog<A> {
        &self.audit
    }

    /// Installs a newly shipped immutable policy set. Existing grants remain
    /// bound to their original version and therefore fail final revalidation.
    pub fn update_policy(&mut self, policy: PolicyEngine) {
        self.policy = policy;
    }

    pub fn state(&self, challenge_id: ChallengeId) -> Option<ConfirmationState> {
        self.records
            .iter()
            .find(|record| record.challenge.challenge_id == challenge_id)
            .map(|record| record.state)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_challenge(
        &mut self,
        intent: &ActionIntent,
        decision: &ActionDecision,
        challenge_id: ChallengeId,
        nonce: ConfirmationNonce,
        authorized_responder: ResponderIdentity,
        issued_at: AuthorityTime,
        expires_at: AuthorityTime,
    ) -> Result<(), AuthorityError> {
        if decision.result() != PolicyResult::ConfirmationRequired
            || decision.confirmation_level() == ConfirmationLevel::None
        {
            return Err(AuthorityError::ChallengeNotRequired);
        }
        if expires_at <= issued_at
            || !decision_matches_intent(decision, intent)
            || decision.policy_version() != self.policy.version()
        {
            return Err(AuthorityError::ChallengeNotRequired);
        }
        if self
            .records
            .iter()
            .any(|record| record.challenge.challenge_id == challenge_id)
        {
            return Err(AuthorityError::Replay);
        }

        let target_digest = digest_target(intent.target());
        let challenge = ConfirmationChallenge {
            challenge_id,
            intent_id: intent.intent_id(),
            confirmation_level: decision.confirmation_level(),
            public_action_summary: action_summary(intent.operation()),
            public_risk_summary: risk_summary(decision.confirmation_level()),
            target_summary: target_summary(intent.target()),
            policy_version: decision.policy_version(),
            runtime_snapshot_generation: decision.runtime_snapshot_generation(),
            session_id: intent.session_id(),
            requested_by: intent.requested_by(),
            issued_at,
            expires_at,
            nonce,
            target_digest,
        };
        let record = ChallengeRecord {
            challenge: challenge.clone(),
            intent_digest: digest_intent(intent),
            operation: intent.operation(),
            target: intent.target().clone(),
            parameters_digest: digest_parameters(intent.parameters()),
            provenance: intent.provenance(),
            authorized_responder,
            state: ConfirmationState::Created,
            grant: None,
        };
        self.records
            .push(record)
            .map_err(|_| AuthorityError::Capacity)?;
        self.audit.record(ConfirmationAuditEvent::ChallengeCreated {
            challenge_id,
            intent_id: intent.intent_id(),
            level: decision.confirmation_level(),
            target_kind: intent.target().kind(),
        });
        Ok(())
    }

    pub fn issue_challenge(
        &mut self,
        challenge_id: ChallengeId,
    ) -> Result<ConfirmationChallenge, AuthorityError> {
        let record = self.record_mut(challenge_id)?;
        if record.state != ConfirmationState::Created {
            return Err(AuthorityError::AlreadyFinal);
        }
        record.state = ConfirmationState::Issued;
        let challenge = record.challenge.clone();
        self.audit
            .record(ConfirmationAuditEvent::ChallengeIssued { challenge_id });
        Ok(challenge)
    }

    pub fn submit_response(
        &mut self,
        response: &ConfirmationResponse,
        context: ResponseValidationContext<'_>,
    ) -> Result<Option<ConfirmationGrant>, AuthorityError> {
        let record_index = self
            .records
            .iter()
            .position(|record| record.challenge.challenge_id == response.challenge_id)
            .ok_or(AuthorityError::ChallengeNotFound)?;
        let state = self.records[record_index].state;
        if state != ConfirmationState::Issued {
            return Err(if state >= ConfirmationState::Approved {
                AuthorityError::Replay
            } else {
                AuthorityError::ChallengeNotIssued
            });
        }
        if response.session_id != self.records[record_index].challenge.session_id {
            self.records[record_index].state = ConfirmationState::Invalidated;
            self.audit.record(ConfirmationAuditEvent::ResponseRejected {
                challenge_id: response.challenge_id,
            });
            return Err(AuthorityError::WrongSession);
        }
        if response.responder != self.records[record_index].authorized_responder {
            self.records[record_index].state = ConfirmationState::Invalidated;
            self.audit.record(ConfirmationAuditEvent::ResponseRejected {
                challenge_id: response.challenge_id,
            });
            return Err(AuthorityError::WrongResponder);
        }
        if response.submitted_at > self.records[record_index].challenge.expires_at {
            self.records[record_index].state = ConfirmationState::Expired;
            self.audit.record(ConfirmationAuditEvent::ResponseAccepted {
                challenge_id: response.challenge_id,
                state: ConfirmationState::Expired,
            });
            return Err(AuthorityError::Expired);
        }
        if response.submitted_at < self.records[record_index].challenge.issued_at {
            self.records[record_index].state = ConfirmationState::Invalidated;
            self.audit.record(ConfirmationAuditEvent::ResponseRejected {
                challenge_id: response.challenge_id,
            });
            return Err(AuthorityError::InvalidProof);
        }

        let terminal = match response.response_type {
            ConfirmationResponseType::Rejected => ConfirmationState::Rejected,
            ConfirmationResponseType::Cancelled => ConfirmationState::Cancelled,
            ConfirmationResponseType::Expired => ConfirmationState::Expired,
            ConfirmationResponseType::Invalid => ConfirmationState::Invalidated,
            ConfirmationResponseType::Approved(proof) => {
                if let Err(reason) =
                    self.validate_approval_context(record_index, response, &context)
                {
                    self.records[record_index].state = ConfirmationState::Invalidated;
                    self.audit.record(ConfirmationAuditEvent::ResponseRejected {
                        challenge_id: response.challenge_id,
                    });
                    return Err(AuthorityError::Denied(reason));
                }
                if !proof_satisfies(&self.records[record_index].challenge, proof) {
                    self.records[record_index].state = ConfirmationState::Invalidated;
                    self.audit.record(ConfirmationAuditEvent::ResponseRejected {
                        challenge_id: response.challenge_id,
                    });
                    return Err(AuthorityError::InvalidProof);
                }
                ConfirmationState::Approved
            }
        };
        self.records[record_index].state = terminal;
        self.audit.record(ConfirmationAuditEvent::ResponseAccepted {
            challenge_id: response.challenge_id,
            state: terminal,
        });
        if terminal != ConfirmationState::Approved {
            return Ok(None);
        }

        let record = &mut self.records[record_index];
        let grant = ConfirmationGrant {
            grant_id: GrantId(response.challenge_id.bytes()),
            challenge_id: response.challenge_id,
            intent_id: record.challenge.intent_id,
            intent_digest: record.intent_digest,
            operation: record.operation,
            target: record.target.clone(),
            parameters_digest: record.parameters_digest,
            provenance: record.provenance,
            policy_version: record.challenge.policy_version,
            runtime_snapshot_generation: record.challenge.runtime_snapshot_generation,
            session_id: record.challenge.session_id,
            responder: response.responder,
            confirmation_level: record.challenge.confirmation_level,
            issued_at: response.submitted_at,
            expires_at: record.challenge.expires_at,
        };
        record.grant = Some(grant.clone());
        self.audit.record(ConfirmationAuditEvent::GrantCreated {
            challenge_id: response.challenge_id,
            grant_id: grant.grant_id,
        });
        Ok(Some(grant))
    }

    fn validate_approval_context(
        &self,
        record_index: usize,
        response: &ConfirmationResponse,
        context: &ResponseValidationContext<'_>,
    ) -> Result<(), ReadinessDenialReason> {
        let record = &self.records[record_index];
        if record.challenge.policy_version != self.policy.version() {
            return Err(ReadinessDenialReason::PolicyChanged);
        }
        if digest_intent(context.intent) != record.intent_digest
            || context.intent.intent_id() != record.challenge.intent_id
        {
            return Err(ReadinessDenialReason::IntentChanged);
        }
        if context.runtime.generation != record.challenge.runtime_snapshot_generation
            || !context.runtime.available
            || response.submitted_at.0 < context.runtime.captured_mono_ms
            || response
                .submitted_at
                .0
                .saturating_sub(context.runtime.captured_mono_ms)
                > self.max_runtime_age_ms
        {
            return Err(ReadinessDenialReason::RuntimeStale);
        }
        if context.session.status != SessionStatus::Active {
            return Err(if context.session.status == SessionStatus::Unknown {
                ReadinessDenialReason::UnknownState
            } else {
                ReadinessDenialReason::SessionInactive
            });
        }
        if context.session.session_id != record.challenge.session_id {
            return Err(ReadinessDenialReason::WrongSession);
        }
        if context.session.responder != response.responder {
            return Err(ReadinessDenialReason::WrongResponder);
        }
        let validation = validate(context.intent, context.runtime.generation);
        let valid = validation
            .valid
            .ok_or(if validation.status == IntentValidation::Unknown {
                ReadinessDenialReason::UnknownState
            } else {
                ReadinessDenialReason::IntentInvalid
            })?;
        let current = self
            .policy
            .evaluate(valid.policy_operation, context.runtime);
        if current.result != PolicyResult::ConfirmationRequired {
            return Err(ReadinessDenialReason::PolicyDenied);
        }
        if current.confirmation != record.challenge.confirmation_level {
            return Err(ReadinessDenialReason::ConfirmationLevelInsufficient);
        }
        Ok(())
    }

    pub fn produce_ready(
        &mut self,
        intent: &ActionIntent,
        original_decision: &ActionDecision,
        grant: Option<&ConfirmationGrant>,
        runtime: &RuntimeContextSnapshot,
        session: SessionAuthorization,
        now: AuthorityTime,
    ) -> Result<ReadyForExecution, AuthorityError> {
        let result = self.revalidate(intent, original_decision, grant, runtime, session, now);
        match result {
            Ok(()) => {}
            Err(reason) => {
                if let Some(grant) = grant {
                    if let Some(record) = self
                        .records
                        .iter_mut()
                        .find(|record| record.challenge.challenge_id == grant.challenge_id)
                    {
                        if record.state == ConfirmationState::Approved {
                            record.state = ConfirmationState::Invalidated;
                            self.audit.record(ConfirmationAuditEvent::GrantInvalidated {
                                challenge_id: grant.challenge_id,
                                reason,
                            });
                        }
                    }
                }
                self.audit.record(ConfirmationAuditEvent::ReadinessDenied {
                    intent_id: intent.intent_id(),
                    reason,
                });
                return Err(AuthorityError::Denied(reason));
            }
        }

        let confirmed_grant = if original_decision.result() == PolicyResult::ConfirmationRequired {
            let grant = grant
                .cloned()
                .ok_or(AuthorityError::Denied(ReadinessDenialReason::GrantMissing))?;
            let record = self.record_mut(grant.challenge_id)?;
            record.state = ConfirmationState::Ready;
            self.audit.record(ConfirmationAuditEvent::GrantReserved {
                challenge_id: grant.challenge_id,
                grant_id: grant.grant_id,
            });
            Some(grant)
        } else {
            None
        };
        let audit_id = AuditId(self.next_audit_id);
        self.next_audit_id = self.next_audit_id.saturating_add(1);
        self.audit
            .record(ConfirmationAuditEvent::ReadinessProduced {
                intent_id: intent.intent_id(),
                audit_id,
            });
        Ok(ReadyForExecution {
            intent: intent.clone(),
            decision: original_decision.clone(),
            confirmation_grant: confirmed_grant,
            final_validation_result: FinalValidationResult::Valid,
            policy_version: self.policy.version(),
            runtime_snapshot_generation: runtime.generation,
            readiness_timestamp: now,
            audit_id,
        })
    }

    fn revalidate(
        &self,
        intent: &ActionIntent,
        original_decision: &ActionDecision,
        grant: Option<&ConfirmationGrant>,
        runtime: &RuntimeContextSnapshot,
        session: SessionAuthorization,
        now: AuthorityTime,
    ) -> Result<(), ReadinessDenialReason> {
        let validation = validate(intent, runtime.generation);
        let Some(valid) = validation.valid else {
            return Err(if validation.status == IntentValidation::Unknown {
                ReadinessDenialReason::UnknownState
            } else if runtime.generation != intent.runtime_snapshot_generation() {
                ReadinessDenialReason::RuntimeStale
            } else {
                ReadinessDenialReason::IntentInvalid
            });
        };
        if !decision_matches_intent(original_decision, intent) {
            return Err(ReadinessDenialReason::IntentChanged);
        }
        if original_decision.policy_version() != self.policy.version() {
            return Err(ReadinessDenialReason::PolicyChanged);
        }
        if !runtime.available
            || now.0 < runtime.captured_mono_ms
            || now.0.saturating_sub(runtime.captured_mono_ms) > self.max_runtime_age_ms
        {
            return Err(ReadinessDenialReason::RuntimeStale);
        }
        if session.status != SessionStatus::Active {
            return Err(if session.status == SessionStatus::Unknown {
                ReadinessDenialReason::UnknownState
            } else {
                ReadinessDenialReason::SessionInactive
            });
        }
        if session.session_id != intent.session_id() {
            return Err(ReadinessDenialReason::WrongSession);
        }
        let current = self.policy.evaluate(valid.policy_operation, runtime);
        if current.result == PolicyResult::Denied || current.result == PolicyResult::Unknown {
            return Err(ReadinessDenialReason::PolicyDenied);
        }
        if current.result != original_decision.result()
            || current.confirmation != original_decision.confirmation_level()
            || current.reason != original_decision.public_reason_code()
        {
            return Err(ReadinessDenialReason::DecisionChanged);
        }

        if current.result == PolicyResult::Allowed {
            return if grant.is_none() {
                Ok(())
            } else {
                Err(ReadinessDenialReason::GrantChanged)
            };
        }
        let grant = grant.ok_or(ReadinessDenialReason::GrantMissing)?;
        let record = self
            .records
            .iter()
            .find(|record| record.challenge.challenge_id == grant.challenge_id)
            .ok_or(ReadinessDenialReason::GrantChanged)?;
        if matches!(
            record.state,
            ConfirmationState::Ready | ConfirmationState::Consumed
        ) {
            return Err(ReadinessDenialReason::GrantConsumed);
        }
        if record.state != ConfirmationState::Approved || record.grant.as_ref() != Some(grant) {
            return Err(ReadinessDenialReason::GrantChanged);
        }
        if now > grant.expires_at {
            return Err(ReadinessDenialReason::ChallengeExpired);
        }
        if grant.intent_id != intent.intent_id()
            || grant.intent_digest != digest_intent(intent)
            || grant.operation != intent.operation()
            || grant.target != *intent.target()
            || grant.parameters_digest != digest_parameters(intent.parameters())
            || grant.provenance != intent.provenance()
            || grant.policy_version != self.policy.version()
            || grant.runtime_snapshot_generation != runtime.generation
            || grant.session_id != intent.session_id()
        {
            return Err(ReadinessDenialReason::GrantChanged);
        }
        if grant.responder != session.responder {
            return Err(ReadinessDenialReason::WrongResponder);
        }
        if !level_satisfies(grant.confirmation_level, current.confirmation) {
            return Err(ReadinessDenialReason::ConfirmationLevelInsufficient);
        }
        Ok(())
    }

    fn record_mut(
        &mut self,
        challenge_id: ChallengeId,
    ) -> Result<&mut ChallengeRecord, AuthorityError> {
        self.records
            .iter_mut()
            .find(|record| record.challenge.challenge_id == challenge_id)
            .ok_or(AuthorityError::ChallengeNotFound)
    }
}

fn proof_satisfies(challenge: &ConfirmationChallenge, proof: ApprovalProof) -> bool {
    match (challenge.confirmation_level, proof) {
        (ConfirmationLevel::Soft, ApprovalProof::SoftExplicit) => true,
        (ConfirmationLevel::Strong, ApprovalProof::Strong { nonce }) => nonce == challenge.nonce,
        (
            ConfirmationLevel::Critical,
            ApprovalProof::Critical {
                nonce,
                exact_target,
                consequence: ConsequenceAcknowledgement::Acknowledged,
            },
        ) => nonce == challenge.nonce && exact_target == challenge.target_digest,
        _ => false,
    }
}

fn level_satisfies(actual: ConfirmationLevel, required: ConfirmationLevel) -> bool {
    matches!(
        (actual, required),
        (ConfirmationLevel::Soft, ConfirmationLevel::Soft)
            | (ConfirmationLevel::Strong, ConfirmationLevel::Strong)
            | (ConfirmationLevel::Critical, ConfirmationLevel::Critical)
    )
}

fn decision_matches_intent(decision: &ActionDecision, intent: &ActionIntent) -> bool {
    decision.intent_id() == intent.intent_id()
        && decision.bound_operation() == intent.operation()
        && decision.bound_target() == intent.target()
        && decision.bound_parameters() == intent.parameters()
        && decision.bound_requested_by() == intent.requested_by()
        && decision.bound_session_id() == intent.session_id()
        && decision.bound_creation_time() == intent.creation_time()
        && decision.bound_risk_hint() == intent.risk_hint()
        && decision.bound_provenance() == intent.provenance()
        && decision.runtime_snapshot_generation() == intent.runtime_snapshot_generation()
}

fn bounded_text(value: &str) -> String<MAX_CONFIRMATION_TEXT_LEN> {
    let mut output = String::new();
    let _ = output.push_str(value);
    output
}

fn action_summary(operation: ActionOperation) -> String<MAX_CONFIRMATION_TEXT_LEN> {
    bounded_text(match operation {
        ActionOperation::Observe => "Observe system state",
        ActionOperation::OpenApplication => "Open application",
        ActionOperation::OpenSettingsPage => "Open settings page",
        ActionOperation::LaunchUtility => "Launch utility",
        ActionOperation::RestartService => "Restart service",
        ActionOperation::StopService => "Stop service",
        ActionOperation::InstallPackage => "Install package",
        ActionOperation::RemovePackage => "Remove package",
        ActionOperation::ModifyFile => "Modify file",
        ActionOperation::DeleteFile => "Delete file",
        ActionOperation::ModifyBootConfiguration => "Modify boot configuration",
        ActionOperation::EraseDisk => "Erase disk",
        ActionOperation::UnknownOperation(_) => "Unknown action",
    })
}

fn risk_summary(level: ConfirmationLevel) -> String<MAX_CONFIRMATION_TEXT_LEN> {
    bounded_text(match level {
        ConfirmationLevel::Soft => "This action needs explicit approval.",
        ConfirmationLevel::Strong => "This action may disrupt or change the system.",
        ConfirmationLevel::Critical => {
            "This action can cause critical, destructive, or irreversible consequences."
        }
        ConfirmationLevel::None => "No confirmation is available.",
    })
}

fn target_summary(target: &ActionTarget) -> String<MAX_CONFIRMATION_TEXT_LEN> {
    match target {
        ActionTarget::Application(value)
        | ActionTarget::SettingsPage(value)
        | ActionTarget::Utility(value)
        | ActionTarget::Service(value)
        | ActionTarget::Package(value)
        | ActionTarget::Disk(value) => bounded_text(value.as_str()),
        ActionTarget::File(value) => bounded_text(value.as_str()),
        ActionTarget::System => bounded_text("System"),
        ActionTarget::Unknown => bounded_text("Unknown target"),
    }
}

fn digest_intent(intent: &ActionIntent) -> BindingDigest {
    let mut hash = DigestBuilder::new(b"wiseowl.intent.v1");
    hash.bytes(&intent.intent_id().bytes());
    hash.u16(operation_tag(intent.operation()));
    hash.digest(digest_target(intent.target()));
    hash.digest(digest_parameters(intent.parameters()));
    hash.u64(intent.session_id().0);
    hash.u64(intent.runtime_snapshot_generation());
    hash.u64(intent.creation_time().0);
    hash.requested_by(intent.requested_by());
    hash.u16(risk_tag(intent.risk_hint()));
    hash.u16(provenance_tag(intent.provenance()));
    hash.finish()
}

fn digest_target(target: &ActionTarget) -> BindingDigest {
    let mut hash = DigestBuilder::new(b"wiseowl.target.v1");
    hash.u16(target.kind() as u16);
    match target {
        ActionTarget::Application(value)
        | ActionTarget::SettingsPage(value)
        | ActionTarget::Utility(value)
        | ActionTarget::Service(value)
        | ActionTarget::Package(value)
        | ActionTarget::Disk(value) => hash.bytes(value.as_str().as_bytes()),
        ActionTarget::File(value) => hash.bytes(value.as_str().as_bytes()),
        ActionTarget::System | ActionTarget::Unknown => {}
    }
    hash.finish()
}

fn digest_parameters(parameters: &ActionParameters) -> BindingDigest {
    let mut hash = DigestBuilder::new(b"wiseowl.parameters.v1");
    match parameters {
        ActionParameters::None => hash.u16(0),
        ActionParameters::Observe { include_health } => {
            hash.u16(1);
            hash.u16(u16::from(*include_health));
        }
        ActionParameters::Application { new_instance } => {
            hash.u16(2);
            hash.u16(u16::from(*new_instance));
        }
        ActionParameters::Settings { focus } => {
            hash.u16(3);
            if let Some(value) = focus {
                hash.bytes(value.as_str().as_bytes());
            }
        }
        ActionParameters::Utility { mode } => {
            hash.u16(4);
            if let Some(value) = mode {
                hash.bytes(value.as_str().as_bytes());
            }
        }
        ActionParameters::Service { force } => {
            hash.u16(5);
            hash.u16(u16::from(*force));
        }
        ActionParameters::Package { version } => {
            hash.u16(6);
            if let Some(value) = version {
                hash.bytes(value.as_str().as_bytes());
            }
        }
        ActionParameters::File { recursive } => {
            hash.u16(7);
            hash.u16(u16::from(*recursive));
        }
        ActionParameters::Disk { whole_device } => {
            hash.u16(8);
            hash.u16(u16::from(*whole_device));
        }
    }
    hash.finish()
}

fn operation_tag(operation: ActionOperation) -> u16 {
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
        ActionOperation::UnknownOperation(value) => value | 0x8000,
    }
}

fn risk_tag(value: crate::action_intent::RiskHint) -> u16 {
    match value {
        crate::action_intent::RiskHint::Low => 1,
        crate::action_intent::RiskHint::Moderate => 2,
        crate::action_intent::RiskHint::High => 3,
        crate::action_intent::RiskHint::Critical => 4,
        crate::action_intent::RiskHint::Unknown => 5,
    }
}

fn provenance_tag(value: Provenance) -> u16 {
    match value {
        Provenance::Conversation => 1,
        Provenance::ExplicitUserRequest => 2,
        Provenance::SystemRecommendation => 3,
        Provenance::Other(value) => value | 0x8000,
    }
}

/// Domain-separated SHA-256 binding digest. This is an identity digest, not a
/// secret MAC; authority-owned state is still required for authenticity/use.
struct DigestBuilder(Sha256);

impl DigestBuilder {
    fn new(domain: &[u8]) -> Self {
        let mut builder = Self(Sha256::new());
        builder.bytes(domain);
        builder
    }
    fn bytes(&mut self, bytes: &[u8]) {
        self.0.update((bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
    }
    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }
    fn requested_by(&mut self, value: RequestedBy) {
        match value {
            RequestedBy::User(id) => {
                self.u16(1);
                self.u32(id);
            }
            RequestedBy::WiseOwlReasoning => self.u16(2),
            RequestedBy::SystemComponent(id) => {
                self.u16(3);
                self.u16(id);
            }
        }
    }
    fn digest(&mut self, value: BindingDigest) {
        self.bytes(&value.0);
    }
    fn finish(self) -> BindingDigest {
        BindingDigest(self.0.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;

    use super::*;
    use crate::action_intent::{
        ActionEvaluation, ActionIntentEvaluator, ActionParameters, BoundedFilePath, CreationTime,
        RiskHint, TypedIdentifier,
    };
    use crate::policy::{PolicyRule, POLICY_V1_VERSION};

    const RESPONDER: ResponderIdentity = ResponderIdentity::User(42);

    fn runtime(generation: u64) -> RuntimeContextSnapshot {
        let mut runtime = RuntimeContextSnapshot {
            available: true,
            generation,
            captured_mono_ms: 100,
            ..RuntimeContextSnapshot::default()
        };
        runtime.session.desktop_mode = Some(true);
        runtime.session.installer_mode = Some(false);
        runtime.session.recovery_mode = Some(false);
        runtime
    }

    fn target(value: &str) -> TypedIdentifier {
        TypedIdentifier::new(value).unwrap()
    }

    fn intent_for(
        operation: ActionOperation,
        target: ActionTarget,
        parameters: ActionParameters,
    ) -> ActionIntent {
        ActionIntent::new(
            IntentId::new([7; 16]),
            operation,
            target,
            parameters,
            RequestedBy::User(42),
            SessionId(9),
            1,
            CreationTime(100),
            RiskHint::Moderate,
            Provenance::ExplicitUserRequest,
        )
    }

    fn decision(intent: &ActionIntent, runtime: &RuntimeContextSnapshot) -> ActionDecision {
        let ActionEvaluation::Decided(decision) =
            ActionIntentEvaluator::<8>::new(PolicyEngine::v1()).evaluate(intent, runtime)
        else {
            panic!("expected decision")
        };
        decision
    }

    fn proof(challenge: &ConfirmationChallenge) -> ApprovalProof {
        match challenge.confirmation_level() {
            ConfirmationLevel::Soft => ApprovalProof::SoftExplicit,
            ConfirmationLevel::Strong => ApprovalProof::Strong {
                nonce: challenge.nonce(),
            },
            ConfirmationLevel::Critical => ApprovalProof::Critical {
                nonce: challenge.nonce(),
                exact_target: challenge.target_digest(),
                consequence: ConsequenceAcknowledgement::Acknowledged,
            },
            ConfirmationLevel::None => panic!("none cannot be confirmed"),
        }
    }

    fn issued(
        intent: &ActionIntent,
        decision: &ActionDecision,
    ) -> (ConfirmationAuthority<8, 32>, ConfirmationChallenge) {
        let mut authority = ConfirmationAuthority::new(PolicyEngine::v1(), 1_000);
        authority
            .create_challenge(
                intent,
                decision,
                ChallengeId::new([3; 16]),
                ConfirmationNonce::new([5; 16]),
                RESPONDER,
                AuthorityTime(110),
                AuthorityTime(500),
            )
            .unwrap();
        let challenge = authority
            .issue_challenge(ChallengeId::new([3; 16]))
            .unwrap();
        (authority, challenge)
    }

    fn approve<const N: usize, const A: usize>(
        authority: &mut ConfirmationAuthority<N, A>,
        challenge: &ConfirmationChallenge,
        intent: &ActionIntent,
        runtime: &RuntimeContextSnapshot,
    ) -> ConfirmationGrant {
        authority
            .submit_response(
                &ConfirmationResponse::new(
                    challenge.challenge_id(),
                    challenge.session_id(),
                    RESPONDER,
                    ConfirmationResponseType::Approved(proof(challenge)),
                    AuthorityTime(120),
                ),
                ResponseValidationContext::new(
                    intent,
                    runtime,
                    SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
                ),
            )
            .unwrap()
            .unwrap()
    }

    fn soft_intent() -> ActionIntent {
        intent_for(
            ActionOperation::RestartService,
            ActionTarget::Service(target("networkd")),
            ActionParameters::Service { force: false },
        )
    }

    fn strong_intent() -> ActionIntent {
        intent_for(
            ActionOperation::DeleteFile,
            ActionTarget::File(BoundedFilePath::new("/private/secret.txt").unwrap()),
            ActionParameters::File { recursive: false },
        )
    }

    fn critical_intent() -> (ActionIntent, RuntimeContextSnapshot) {
        let intent = intent_for(
            ActionOperation::EraseDisk,
            ActionTarget::Disk(target("disk0")),
            ActionParameters::Disk { whole_device: true },
        );
        let mut runtime = runtime(1);
        runtime.session.desktop_mode = Some(false);
        runtime.session.installer_mode = Some(true);
        (intent, runtime)
    }

    #[test]
    fn soft_strong_and_critical_confirmation_succeed() {
        for (intent, runtime) in [
            (soft_intent(), runtime(1)),
            (strong_intent(), runtime(1)),
            critical_intent(),
        ] {
            let decision = decision(&intent, &runtime);
            let (mut authority, challenge) = issued(&intent, &decision);
            let grant = approve(&mut authority, &challenge, &intent, &runtime);
            assert_eq!(grant.confirmation_level(), decision.confirmation_level());
            assert_eq!(
                authority.state(challenge.challenge_id()),
                Some(ConfirmationState::Approved)
            );
        }
    }

    #[test]
    fn rejection_and_cancellation_are_terminal_without_grants() {
        for response_type in [
            ConfirmationResponseType::Rejected,
            ConfirmationResponseType::Cancelled,
        ] {
            let intent = soft_intent();
            let decision = decision(&intent, &runtime(1));
            let (mut authority, challenge) = issued(&intent, &decision);
            assert_eq!(
                authority
                    .submit_response(
                        &ConfirmationResponse::new(
                            challenge.challenge_id(),
                            SessionId(9),
                            RESPONDER,
                            response_type,
                            AuthorityTime(120),
                        ),
                        ResponseValidationContext::new(
                            &intent,
                            &runtime(1),
                            SessionAuthorization::new(
                                SessionId(9),
                                RESPONDER,
                                SessionStatus::Active,
                            ),
                        ),
                    )
                    .unwrap(),
                None
            );
            assert!(matches!(
                authority.state(challenge.challenge_id()),
                Some(ConfirmationState::Rejected | ConfirmationState::Cancelled)
            ));
        }
    }

    #[test]
    fn timeout_window_close_and_missing_response_never_approve() {
        let intent = soft_intent();
        let original_decision = decision(&intent, &runtime(1));
        let (mut authority, challenge) = issued(&intent, &original_decision);
        assert_eq!(
            authority.submit_response(
                &ConfirmationResponse::new(
                    challenge.challenge_id(),
                    SessionId(9),
                    RESPONDER,
                    ConfirmationResponseType::Approved(ApprovalProof::SoftExplicit),
                    AuthorityTime(501),
                ),
                ResponseValidationContext::new(
                    &intent,
                    &runtime(1),
                    SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
                ),
            ),
            Err(AuthorityError::Expired)
        );
        assert_eq!(
            authority.state(challenge.challenge_id()),
            Some(ConfirmationState::Expired)
        );
    }

    #[test]
    fn wrong_session_and_wrong_responder_fail_closed() {
        for (session, responder, expected) in [
            (SessionId(10), RESPONDER, AuthorityError::WrongSession),
            (
                SessionId(9),
                ResponderIdentity::User(99),
                AuthorityError::WrongResponder,
            ),
        ] {
            let intent = soft_intent();
            let decision = decision(&intent, &runtime(1));
            let (mut authority, challenge) = issued(&intent, &decision);
            assert_eq!(
                authority.submit_response(
                    &ConfirmationResponse::new(
                        challenge.challenge_id(),
                        session,
                        responder,
                        ConfirmationResponseType::Approved(ApprovalProof::SoftExplicit),
                        AuthorityTime(120),
                    ),
                    ResponseValidationContext::new(
                        &intent,
                        &runtime(1),
                        SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
                    ),
                ),
                Err(expected)
            );
        }
    }

    #[test]
    fn changed_target_parameters_and_provenance_invalidate_grant() {
        let original = soft_intent();
        let decision = decision(&original, &runtime(1));
        let variants = [
            (
                ActionIntent::new(
                    original.intent_id(),
                    original.operation(),
                    ActionTarget::Service(target("resolved")),
                    original.parameters().clone(),
                    original.requested_by(),
                    original.session_id(),
                    1,
                    original.creation_time(),
                    original.risk_hint(),
                    original.provenance(),
                ),
                ReadinessDenialReason::IntentChanged,
            ),
            (
                ActionIntent::new(
                    original.intent_id(),
                    original.operation(),
                    original.target().clone(),
                    ActionParameters::Service { force: true },
                    original.requested_by(),
                    original.session_id(),
                    1,
                    original.creation_time(),
                    original.risk_hint(),
                    original.provenance(),
                ),
                ReadinessDenialReason::IntentChanged,
            ),
            (
                ActionIntent::new(
                    original.intent_id(),
                    original.operation(),
                    original.target().clone(),
                    original.parameters().clone(),
                    original.requested_by(),
                    original.session_id(),
                    1,
                    original.creation_time(),
                    original.risk_hint(),
                    Provenance::Conversation,
                ),
                ReadinessDenialReason::IntentChanged,
            ),
        ];
        for (changed, expected) in variants {
            let (mut authority, challenge) = issued(&original, &decision);
            let grant = approve(&mut authority, &challenge, &original, &runtime(1));
            assert!(matches!(
                authority.produce_ready(
                    &changed,
                    &decision,
                    Some(&grant),
                    &runtime(1),
                    SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
                    AuthorityTime(130),
                ),
                Err(AuthorityError::Denied(reason)) if reason == expected
            ));
        }
    }

    #[test]
    fn changed_policy_and_stale_runtime_fail_closed() {
        let intent = soft_intent();
        let original_decision = decision(&intent, &runtime(1));
        let (mut authority, challenge) = issued(&intent, &original_decision);
        let grant = approve(&mut authority, &challenge, &intent, &runtime(1));
        authority.update_policy(PolicyEngine::from_static_rules(
            PolicyVersion::new(2, 0),
            PolicyEngine::v1().rules(),
        ));
        assert!(matches!(
            authority.produce_ready(
                &intent,
                &original_decision,
                Some(&grant),
                &runtime(1),
                SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
                AuthorityTime(130),
            ),
            Err(AuthorityError::Denied(ReadinessDenialReason::PolicyChanged))
        ));

        let refreshed_decision = decision(&intent, &runtime(1));
        let (mut authority, challenge) = issued(&intent, &refreshed_decision);
        let grant = approve(&mut authority, &challenge, &intent, &runtime(1));
        assert!(matches!(
            authority.produce_ready(
                &intent,
                &refreshed_decision,
                Some(&grant),
                &runtime(2),
                SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
                AuthorityTime(130),
            ),
            Err(AuthorityError::Denied(ReadinessDenialReason::RuntimeStale))
        ));
    }

    #[test]
    fn approval_after_policy_intent_or_runtime_change_creates_no_grant() {
        let intent = soft_intent();
        let current_runtime = runtime(1);
        let policy_decision = decision(&intent, &current_runtime);
        let response_for = |challenge: &ConfirmationChallenge| {
            ConfirmationResponse::new(
                challenge.challenge_id(),
                SessionId(9),
                RESPONDER,
                ConfirmationResponseType::Approved(proof(challenge)),
                AuthorityTime(120),
            )
        };

        let (mut authority, challenge) = issued(&intent, &policy_decision);
        authority.update_policy(PolicyEngine::from_static_rules(
            PolicyVersion::new(2, 0),
            PolicyEngine::v1().rules(),
        ));
        assert_eq!(
            authority.submit_response(
                &response_for(&challenge),
                ResponseValidationContext::new(
                    &intent,
                    &current_runtime,
                    SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
                ),
            ),
            Err(AuthorityError::Denied(ReadinessDenialReason::PolicyChanged))
        );

        let (mut authority, challenge) = issued(&intent, &policy_decision);
        let changed = ActionIntent::new(
            intent.intent_id(),
            intent.operation(),
            ActionTarget::Service(target("resolved")),
            intent.parameters().clone(),
            intent.requested_by(),
            intent.session_id(),
            1,
            intent.creation_time(),
            intent.risk_hint(),
            intent.provenance(),
        );
        assert_eq!(
            authority.submit_response(
                &response_for(&challenge),
                ResponseValidationContext::new(
                    &changed,
                    &current_runtime,
                    SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
                ),
            ),
            Err(AuthorityError::Denied(ReadinessDenialReason::IntentChanged))
        );

        let mut authority = ConfirmationAuthority::<8, 32>::new(PolicyEngine::v1(), 10);
        authority
            .create_challenge(
                &intent,
                &policy_decision,
                ChallengeId::new([3; 16]),
                ConfirmationNonce::new([5; 16]),
                RESPONDER,
                AuthorityTime(110),
                AuthorityTime(500),
            )
            .unwrap();
        let challenge = authority
            .issue_challenge(ChallengeId::new([3; 16]))
            .unwrap();
        assert_eq!(
            authority.submit_response(
                &response_for(&challenge),
                ResponseValidationContext::new(
                    &intent,
                    &current_runtime,
                    SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
                ),
            ),
            Err(AuthorityError::Denied(ReadinessDenialReason::RuntimeStale))
        );
    }

    #[test]
    fn replayed_response_and_reused_grant_are_rejected() {
        let intent = soft_intent();
        let runtime = runtime(1);
        let decision = decision(&intent, &runtime);
        let (mut authority, challenge) = issued(&intent, &decision);
        let response = ConfirmationResponse::new(
            challenge.challenge_id(),
            SessionId(9),
            RESPONDER,
            ConfirmationResponseType::Approved(proof(&challenge)),
            AuthorityTime(120),
        );
        let response_context = ResponseValidationContext::new(
            &intent,
            &runtime,
            SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
        );
        let grant = authority
            .submit_response(&response, response_context)
            .unwrap()
            .unwrap();
        assert_eq!(
            authority.submit_response(&response, response_context),
            Err(AuthorityError::Replay)
        );
        let session = SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active);
        authority
            .produce_ready(
                &intent,
                &decision,
                Some(&grant),
                &runtime,
                session,
                AuthorityTime(130),
            )
            .unwrap();
        assert!(matches!(
            authority.produce_ready(
                &intent,
                &decision,
                Some(&grant),
                &runtime,
                session,
                AuthorityTime(131),
            ),
            Err(AuthorityError::Denied(ReadinessDenialReason::GrantConsumed))
        ));
    }

    #[test]
    fn confirmation_level_downgrade_is_invalid_proof() {
        let intent = strong_intent();
        let decision = decision(&intent, &runtime(1));
        let (mut authority, challenge) = issued(&intent, &decision);
        assert_eq!(
            authority.submit_response(
                &ConfirmationResponse::new(
                    challenge.challenge_id(),
                    SessionId(9),
                    RESPONDER,
                    ConfirmationResponseType::Approved(ApprovalProof::SoftExplicit),
                    AuthorityTime(120),
                ),
                ResponseValidationContext::new(
                    &intent,
                    &runtime(1),
                    SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
                ),
            ),
            Err(AuthorityError::InvalidProof)
        );
    }

    #[test]
    fn denied_and_unknown_intents_cannot_create_challenges() {
        let denied = intent_for(
            ActionOperation::ModifyBootConfiguration,
            ActionTarget::System,
            ActionParameters::None,
        );
        let denied_decision = decision(&denied, &runtime(1));
        let mut authority = ConfirmationAuthority::<4, 8>::new(PolicyEngine::v1(), 1_000);
        assert_eq!(
            authority.create_challenge(
                &denied,
                &denied_decision,
                ChallengeId::new([1; 16]),
                ConfirmationNonce::new([2; 16]),
                RESPONDER,
                AuthorityTime(110),
                AuthorityTime(200),
            ),
            Err(AuthorityError::ChallengeNotRequired)
        );

        let unknown = intent_for(
            ActionOperation::UnknownOperation(99),
            ActionTarget::System,
            ActionParameters::None,
        );
        assert!(matches!(
            ActionIntentEvaluator::<4>::new(PolicyEngine::v1()).evaluate(&unknown, &runtime(1)),
            ActionEvaluation::Rejected {
                result: IntentValidation::Unknown,
                ..
            }
        ));
    }

    #[test]
    fn approved_grant_produces_ready_without_executing() {
        let intent = soft_intent();
        let runtime = runtime(1);
        let decision = decision(&intent, &runtime);
        let (mut authority, challenge) = issued(&intent, &decision);
        let grant = approve(&mut authority, &challenge, &intent, &runtime);
        let ready = authority
            .produce_ready(
                &intent,
                &decision,
                Some(&grant),
                &runtime,
                SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Active),
                AuthorityTime(130),
            )
            .unwrap();
        assert_eq!(
            ready.final_validation_result(),
            FinalValidationResult::Valid
        );
        assert_eq!(ready.confirmation_grant(), Some(&grant));
        assert_eq!(
            authority.state(challenge.challenge_id()),
            Some(ConfirmationState::Ready)
        );
        // The only output is an immutable data envelope. There is no execute method.
        assert_eq!(ready.intent().intent_id(), intent.intent_id());
    }

    #[test]
    fn inactive_unknown_and_cross_user_sessions_fail_closed() {
        for (session, reason) in [
            (
                SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Inactive),
                ReadinessDenialReason::SessionInactive,
            ),
            (
                SessionAuthorization::new(SessionId(9), RESPONDER, SessionStatus::Unknown),
                ReadinessDenialReason::UnknownState,
            ),
            (
                SessionAuthorization::new(
                    SessionId(9),
                    ResponderIdentity::User(77),
                    SessionStatus::Active,
                ),
                ReadinessDenialReason::WrongResponder,
            ),
        ] {
            let intent = soft_intent();
            let runtime = runtime(1);
            let decision = decision(&intent, &runtime);
            let (mut authority, challenge) = issued(&intent, &decision);
            let grant = approve(&mut authority, &challenge, &intent, &runtime);
            assert_eq!(
                authority.produce_ready(
                    &intent,
                    &decision,
                    Some(&grant),
                    &runtime,
                    session,
                    AuthorityTime(130),
                ),
                Err(AuthorityError::Denied(reason))
            );
        }
    }

    #[test]
    fn audit_ring_is_bounded_and_redacted() {
        let intent = strong_intent();
        let decision = decision(&intent, &runtime(1));
        let mut authority = ConfirmationAuthority::<4, 2>::new(PolicyEngine::v1(), 1_000);
        authority
            .create_challenge(
                &intent,
                &decision,
                ChallengeId::new([3; 16]),
                ConfirmationNonce::new([5; 16]),
                RESPONDER,
                AuthorityTime(110),
                AuthorityTime(500),
            )
            .unwrap();
        let challenge = authority
            .issue_challenge(ChallengeId::new([3; 16]))
            .unwrap();
        let _ = approve(&mut authority, &challenge, &intent, &runtime(1));
        assert_eq!(authority.audit().len(), 2);
        assert_eq!(authority.audit().evicted(), 2);
        let rendered = format!(
            "{:?}",
            authority.audit().entries().collect::<alloc::vec::Vec<_>>()
        );
        assert!(!rendered.contains("/private/secret.txt"));
        assert!(!rendered.contains("typed phrase"));
        assert!(!rendered.contains("PolicyRule"));
    }

    #[test]
    fn presentation_view_has_explicit_non_default_choices() {
        let intent = strong_intent();
        let decision = decision(&intent, &runtime(1));
        let (_, challenge) = issued(&intent, &decision);
        let view = challenge.view();
        assert_eq!(
            view.available_choices(),
            &[
                ConfirmationChoice::Approve,
                ConfirmationChoice::Reject,
                ConfirmationChoice::Cancel
            ]
        );
        assert_eq!(view.target_summary(), "/private/secret.txt");
        assert_eq!(view.default_choice(), None);
    }

    #[test]
    fn v1_policy_constant_remains_the_authority_default_contract() {
        let no_extra_rules: &[PolicyRule] = PolicyEngine::v1().rules();
        assert!(!no_extra_rules.is_empty());
        assert_eq!(PolicyEngine::v1().version(), POLICY_V1_VERSION);
    }
}
