//! Capability-bounded Trusted Action Executor v1.
//!
//! The only public execution entry point consumes a [`ReadyForExecution`].
//! This module has no language parser, command representation, path type,
//! process-spawn callback, filesystem API, or generic IPC endpoint.

use heapless::{Deque, Vec};

use crate::action_intent::{
    validate, ActionOperation, ActionParameters, ActionTarget, AuditId, IntentId, RequestedBy,
    SessionId, TargetKind, TypedIdentifier,
};
use crate::confirmation::{
    AuthorityError, AuthorityTime, ChallengeId, ConfirmationAuthority, ConfirmationChallenge,
    ConfirmationGrant, ConfirmationNonce, ConfirmationResponse, ReadyForExecution,
    ResponderIdentity, ResponseValidationContext, SessionAuthorization, SessionStatus,
};
use crate::policy::{PolicyEngine, PolicyResult};
use crate::runtime_context::RuntimeContextSnapshot;

pub const DEFAULT_EXECUTION_AUDIT_CAPACITY: usize = 64;
pub const DEFAULT_REPLAY_CAPACITY: usize = 32;
pub const DEFAULT_MAX_RUNTIME_AGE_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExecutionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionResultCode {
    Succeeded,
    Rejected,
    UnsupportedOperation,
    InvalidEnvelope,
    TargetNotFound,
    TargetUnavailable,
    SessionInactive,
    PolicyChanged,
    RuntimeStale,
    ConfirmationExpired,
    ConfirmationInvalid,
    DispatchFailed,
    AlreadyConsumed,
    Unknown,
}

/// Immutable, public-safe outcome. Dispatch success means that the trusted
/// launcher accepted the request, not that the application became ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    intent_id: IntentId,
    execution_id: ExecutionId,
    operation: ActionOperation,
    code: ExecutionResultCode,
    dispatch_timestamp: Option<AuthorityTime>,
    completion_timestamp: Option<AuthorityTime>,
    audit_id: AuditId,
}

impl ExecutionResult {
    pub const fn intent_id(&self) -> IntentId {
        self.intent_id
    }
    pub const fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }
    pub const fn operation(&self) -> ActionOperation {
        self.operation
    }
    pub const fn code(&self) -> ExecutionResultCode {
        self.code
    }
    pub const fn dispatch_timestamp(&self) -> Option<AuthorityTime> {
        self.dispatch_timestamp
    }
    pub const fn completion_timestamp(&self) -> Option<AuthorityTime> {
        self.completion_timestamp
    }
    pub const fn audit_id(&self) -> AuditId {
        self.audit_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchApplicationRequest {
    bundle_id: TypedIdentifier,
    session_id: SessionId,
    requester: RequestedBy,
    audit_id: AuditId,
    execution_id: ExecutionId,
}

impl LaunchApplicationRequest {
    pub fn bundle_id(&self) -> &TypedIdentifier {
        &self.bundle_id
    }
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub const fn requester(&self) -> RequestedBy {
        self.requester
    }
    pub const fn audit_id(&self) -> AuditId {
        self.audit_id
    }
    pub const fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSettingsPageRequest {
    page_id: TypedIdentifier,
    session_id: SessionId,
    requester: RequestedBy,
    audit_id: AuditId,
    execution_id: ExecutionId,
}

impl OpenSettingsPageRequest {
    pub fn page_id(&self) -> &TypedIdentifier {
        &self.page_id
    }
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub const fn requester(&self) -> RequestedBy {
        self.requester
    }
    pub const fn audit_id(&self) -> AuditId {
        self.audit_id
    }
    pub const fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryStatus {
    Registered,
    NotFound,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchStatus {
    Accepted,
    Failed,
}

/// Narrow capability supplied to the executor. Implementations may resolve
/// only registered application bundles and bounded Control Panel pages.
pub trait TrustedLaunchAdapter {
    fn application_status(&self, bundle_id: &TypedIdentifier) -> RegistryStatus;
    fn settings_page_status(&self, page_id: &TypedIdentifier) -> RegistryStatus;
    fn launch_application(&mut self, request: LaunchApplicationRequest) -> DispatchStatus;
    fn open_settings_page(&mut self, request: OpenSettingsPageRequest) -> DispatchStatus;
}

pub struct ExecutionContext<'a> {
    runtime: &'a RuntimeContextSnapshot,
    policy: &'a PolicyEngine,
    session: SessionAuthorization,
    requester: RequestedBy,
    now: AuthorityTime,
}

impl<'a> ExecutionContext<'a> {
    pub const fn new(
        runtime: &'a RuntimeContextSnapshot,
        policy: &'a PolicyEngine,
        session: SessionAuthorization,
        requester: RequestedBy,
        now: AuthorityTime,
    ) -> Self {
        Self {
            runtime,
            policy,
            session,
            requester,
            now,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionAuditEvent {
    ReadinessAccepted,
    FinalValidationPassed,
    FinalValidationFailed(ExecutionResultCode),
    DispatchAttempted,
    DispatchAccepted,
    DispatchFailed,
    DuplicateRejected,
    ConfirmationConsumed,
    ResultProduced(ExecutionResultCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionAuditEntry {
    pub execution_id: ExecutionId,
    pub intent_id: IntentId,
    pub operation: ActionOperation,
    pub target_kind: TargetKind,
    pub audit_id: AuditId,
    pub timestamp: AuthorityTime,
    pub event: ExecutionAuditEvent,
}

pub struct ExecutionAuditLog<const N: usize> {
    entries: Deque<ExecutionAuditEntry, N>,
    evicted: u64,
}

impl<const N: usize> ExecutionAuditLog<N> {
    pub const fn new() -> Self {
        Self {
            entries: Deque::new(),
            evicted: 0,
        }
    }
    pub fn entries(&self) -> impl Iterator<Item = &ExecutionAuditEntry> {
        self.entries.iter()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub const fn evicted(&self) -> u64 {
        self.evicted
    }
    fn record(&mut self, entry: ExecutionAuditEntry) {
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

impl<const N: usize> Default for ExecutionAuditLog<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReadinessKey {
    intent_id: IntentId,
    audit_id: AuditId,
}

pub trait ActionExecutor {
    fn execute(
        &mut self,
        ready: ReadyForExecution,
        context: &ExecutionContext<'_>,
    ) -> ExecutionResult;
}

/// The public action pipeline. It preserves the intent → policy →
/// confirmation → readiness → executor order and exposes no operation-only
/// or target-only execution shortcut.
pub struct TrustedActionFlow<A: TrustedLaunchAdapter> {
    evaluator: crate::action_intent::ActionIntentEvaluator<
        { crate::action_intent::DEFAULT_AUDIT_CAPACITY },
    >,
    confirmations: ConfirmationAuthority,
    executor: TrustedActionExecutor<A>,
}

impl<A: TrustedLaunchAdapter> TrustedActionFlow<A> {
    pub const fn new(adapter: A, max_runtime_age_ms: u64) -> Self {
        Self::with_policy(adapter, max_runtime_age_ms, PolicyEngine::v1())
    }

    /// Constructs the same closed trusted flow with an explicitly selected
    /// immutable policy set. This is not an execution shortcut: evaluation,
    /// confirmation, readiness, and final executor validation remain mandatory.
    pub const fn with_policy(adapter: A, max_runtime_age_ms: u64, policy: PolicyEngine) -> Self {
        Self {
            evaluator: crate::action_intent::ActionIntentEvaluator::new(policy),
            confirmations: ConfirmationAuthority::new(policy, max_runtime_age_ms),
            executor: TrustedActionExecutor::new(adapter).with_max_runtime_age(max_runtime_age_ms),
        }
    }

    pub fn evaluate_action(
        &mut self,
        intent: &crate::action_intent::ActionIntent,
        runtime: &RuntimeContextSnapshot,
    ) -> crate::action_intent::ActionEvaluation {
        self.evaluator.evaluate(intent, runtime)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_confirmation_challenge(
        &mut self,
        intent: &crate::action_intent::ActionIntent,
        decision: &crate::action_intent::ActionDecision,
        challenge_id: ChallengeId,
        nonce: ConfirmationNonce,
        authorized_responder: ResponderIdentity,
        issued_at: AuthorityTime,
        expires_at: AuthorityTime,
    ) -> Result<ConfirmationChallenge, AuthorityError> {
        self.confirmations.create_challenge(
            intent,
            decision,
            challenge_id,
            nonce,
            authorized_responder,
            issued_at,
            expires_at,
        )?;
        self.confirmations.issue_challenge(challenge_id)
    }

    pub fn accept_confirmation(
        &mut self,
        response: &ConfirmationResponse,
        context: ResponseValidationContext<'_>,
    ) -> Result<Option<ConfirmationGrant>, AuthorityError> {
        self.confirmations.submit_response(response, context)
    }

    pub fn prepare_for_execution(
        &mut self,
        intent: &crate::action_intent::ActionIntent,
        decision: &crate::action_intent::ActionDecision,
        grant: Option<&ConfirmationGrant>,
        runtime: &RuntimeContextSnapshot,
        session: SessionAuthorization,
        now: AuthorityTime,
    ) -> Result<ReadyForExecution, AuthorityError> {
        self.confirmations
            .produce_ready(intent, decision, grant, runtime, session, now)
    }

    pub fn execute_ready_action(
        &mut self,
        ready: ReadyForExecution,
        context: &ExecutionContext<'_>,
    ) -> ExecutionResult {
        self.executor.execute(ready, context)
    }

    pub fn action_audit(
        &self,
    ) -> &crate::action_intent::AuditLog<{ crate::action_intent::DEFAULT_AUDIT_CAPACITY }> {
        self.evaluator.audit()
    }

    pub fn confirmation_audit(&self) -> &crate::confirmation::ConfirmationAuditLog {
        self.confirmations.audit()
    }

    pub fn execution_audit(&self) -> &ExecutionAuditLog<DEFAULT_EXECUTION_AUDIT_CAPACITY> {
        self.executor.audit()
    }
}

pub struct TrustedActionExecutor<
    A,
    const AUDIT: usize = DEFAULT_EXECUTION_AUDIT_CAPACITY,
    const REPLAY: usize = DEFAULT_REPLAY_CAPACITY,
> {
    adapter: A,
    audit: ExecutionAuditLog<AUDIT>,
    consumed: Vec<ReadinessKey, REPLAY>,
    next_execution_id: u64,
    max_runtime_age_ms: u64,
}

impl<A: TrustedLaunchAdapter, const AUDIT: usize, const REPLAY: usize>
    TrustedActionExecutor<A, AUDIT, REPLAY>
{
    pub const fn new(adapter: A) -> Self {
        Self {
            adapter,
            audit: ExecutionAuditLog::new(),
            consumed: Vec::new(),
            next_execution_id: 1,
            max_runtime_age_ms: DEFAULT_MAX_RUNTIME_AGE_MS,
        }
    }

    pub const fn with_max_runtime_age(mut self, max_runtime_age_ms: u64) -> Self {
        self.max_runtime_age_ms = max_runtime_age_ms;
        self
    }

    pub fn audit(&self) -> &ExecutionAuditLog<AUDIT> {
        &self.audit
    }

    fn record(
        &mut self,
        ready: &ReadyForExecution,
        execution_id: ExecutionId,
        now: AuthorityTime,
        event: ExecutionAuditEvent,
    ) {
        self.audit.record(ExecutionAuditEntry {
            execution_id,
            intent_id: ready.intent().intent_id(),
            operation: ready.intent().operation(),
            target_kind: ready.intent().target().kind(),
            audit_id: ready.audit_id(),
            timestamp: now,
            event,
        });
    }

    fn result(
        &mut self,
        ready: &ReadyForExecution,
        execution_id: ExecutionId,
        now: AuthorityTime,
        dispatch_timestamp: Option<AuthorityTime>,
        code: ExecutionResultCode,
    ) -> ExecutionResult {
        self.record(
            ready,
            execution_id,
            now,
            ExecutionAuditEvent::ResultProduced(code),
        );
        ExecutionResult {
            intent_id: ready.intent().intent_id(),
            execution_id,
            operation: ready.intent().operation(),
            code,
            dispatch_timestamp,
            completion_timestamp: dispatch_timestamp.map(|_| now),
            audit_id: ready.audit_id(),
        }
    }

    fn reject(
        &mut self,
        ready: &ReadyForExecution,
        execution_id: ExecutionId,
        now: AuthorityTime,
        code: ExecutionResultCode,
    ) -> ExecutionResult {
        self.record(
            ready,
            execution_id,
            now,
            ExecutionAuditEvent::FinalValidationFailed(code),
        );
        self.result(ready, execution_id, now, None, code)
    }

    fn validate_final(
        &self,
        ready: &ReadyForExecution,
        context: &ExecutionContext<'_>,
    ) -> Result<(), ExecutionResultCode> {
        if !ready.has_valid_integrity() {
            return Err(ExecutionResultCode::InvalidEnvelope);
        }
        if ready.policy_version() != context.policy.version()
            || ready.decision().policy_version() != context.policy.version()
        {
            return Err(ExecutionResultCode::PolicyChanged);
        }
        if !context.runtime.available
            || context.runtime.generation != ready.runtime_snapshot_generation()
            || context.runtime.generation != ready.intent().runtime_snapshot_generation()
            || context.now.0 < context.runtime.captured_mono_ms
            || context
                .now
                .0
                .saturating_sub(context.runtime.captured_mono_ms)
                > self.max_runtime_age_ms
        {
            return Err(ExecutionResultCode::RuntimeStale);
        }
        if context.session.status() != SessionStatus::Active {
            return Err(ExecutionResultCode::SessionInactive);
        }
        if context.session.session_id() != ready.intent().session_id()
            || context.requester != ready.intent().requested_by()
        {
            return Err(ExecutionResultCode::Rejected);
        }

        let validation = validate(ready.intent(), context.runtime.generation);
        let Some(valid) = validation.valid else {
            return Err(ExecutionResultCode::InvalidEnvelope);
        };
        let current = context
            .policy
            .evaluate(valid.policy_operation, context.runtime);
        if current.result != ready.decision().result()
            || current.confirmation != ready.decision().confirmation_level()
            || current.reason != ready.decision().public_reason_code()
            || matches!(current.result, PolicyResult::Denied | PolicyResult::Unknown)
        {
            return Err(ExecutionResultCode::PolicyChanged);
        }

        match ready.intent().operation() {
            ActionOperation::OpenApplication => {
                if !matches!(
                    ready.intent().parameters(),
                    ActionParameters::Application {
                        new_instance: false
                    }
                ) {
                    return Err(ExecutionResultCode::Rejected);
                }
            }
            ActionOperation::OpenSettingsPage => {
                if !matches!(
                    ready.intent().parameters(),
                    ActionParameters::Settings { focus: None }
                ) {
                    return Err(ExecutionResultCode::Rejected);
                }
            }
            _ => return Err(ExecutionResultCode::UnsupportedOperation),
        }

        match ready.decision().result() {
            PolicyResult::Allowed => {
                if ready.confirmation_grant().is_some() {
                    return Err(ExecutionResultCode::ConfirmationInvalid);
                }
            }
            PolicyResult::ConfirmationRequired => {
                let grant = ready
                    .confirmation_grant()
                    .ok_or(ExecutionResultCode::ConfirmationInvalid)?;
                if context.now > grant.expires_at() {
                    return Err(ExecutionResultCode::ConfirmationExpired);
                }
                if !responder_matches_requester(grant.responder(), context.requester)
                    || grant.responder() != context.session.responder()
                {
                    return Err(ExecutionResultCode::ConfirmationInvalid);
                }
            }
            PolicyResult::Denied | PolicyResult::Unknown => {
                return Err(ExecutionResultCode::Rejected)
            }
        }

        let registry = match ready.intent().target() {
            ActionTarget::Application(bundle_id) => self.adapter.application_status(bundle_id),
            ActionTarget::SettingsPage(page_id) => self.adapter.settings_page_status(page_id),
            _ => return Err(ExecutionResultCode::InvalidEnvelope),
        };
        match registry {
            RegistryStatus::Registered => Ok(()),
            RegistryStatus::NotFound => Err(ExecutionResultCode::TargetNotFound),
            RegistryStatus::Unavailable => Err(ExecutionResultCode::TargetUnavailable),
        }
    }
}

impl<A: TrustedLaunchAdapter, const AUDIT: usize, const REPLAY: usize> ActionExecutor
    for TrustedActionExecutor<A, AUDIT, REPLAY>
{
    fn execute(
        &mut self,
        ready: ReadyForExecution,
        context: &ExecutionContext<'_>,
    ) -> ExecutionResult {
        let execution_id = ExecutionId(self.next_execution_id);
        self.next_execution_id = self.next_execution_id.saturating_add(1);
        let key = ReadinessKey {
            intent_id: ready.intent().intent_id(),
            audit_id: ready.audit_id(),
        };
        self.record(
            &ready,
            execution_id,
            context.now,
            ExecutionAuditEvent::ReadinessAccepted,
        );

        if self.consumed.iter().any(|consumed| *consumed == key) {
            self.record(
                &ready,
                execution_id,
                context.now,
                ExecutionAuditEvent::DuplicateRejected,
            );
            return self.result(
                &ready,
                execution_id,
                context.now,
                None,
                ExecutionResultCode::AlreadyConsumed,
            );
        }

        if let Err(code) = self.validate_final(&ready, context) {
            return self.reject(&ready, execution_id, context.now, code);
        }
        self.record(
            &ready,
            execution_id,
            context.now,
            ExecutionAuditEvent::FinalValidationPassed,
        );

        // Fail closed when replay state is saturated. Never evict a consumed
        // identity, because eviction could permit a second dispatch.
        if self.consumed.push(key).is_err() {
            return self.reject(
                &ready,
                execution_id,
                context.now,
                ExecutionResultCode::Rejected,
            );
        }
        if ready.confirmation_grant().is_some() {
            self.record(
                &ready,
                execution_id,
                context.now,
                ExecutionAuditEvent::ConfirmationConsumed,
            );
        }
        self.record(
            &ready,
            execution_id,
            context.now,
            ExecutionAuditEvent::DispatchAttempted,
        );

        let request_status = match ready.intent().target() {
            ActionTarget::Application(bundle_id) => {
                self.adapter.launch_application(LaunchApplicationRequest {
                    bundle_id: bundle_id.clone(),
                    session_id: ready.intent().session_id(),
                    requester: ready.intent().requested_by(),
                    audit_id: ready.audit_id(),
                    execution_id,
                })
            }
            ActionTarget::SettingsPage(page_id) => {
                self.adapter.open_settings_page(OpenSettingsPageRequest {
                    page_id: page_id.clone(),
                    session_id: ready.intent().session_id(),
                    requester: ready.intent().requested_by(),
                    audit_id: ready.audit_id(),
                    execution_id,
                })
            }
            _ => DispatchStatus::Failed,
        };

        match request_status {
            DispatchStatus::Accepted => {
                self.record(
                    &ready,
                    execution_id,
                    context.now,
                    ExecutionAuditEvent::DispatchAccepted,
                );
                self.result(
                    &ready,
                    execution_id,
                    context.now,
                    Some(context.now),
                    ExecutionResultCode::Succeeded,
                )
            }
            DispatchStatus::Failed => {
                self.record(
                    &ready,
                    execution_id,
                    context.now,
                    ExecutionAuditEvent::DispatchFailed,
                );
                self.result(
                    &ready,
                    execution_id,
                    context.now,
                    Some(context.now),
                    ExecutionResultCode::DispatchFailed,
                )
            }
        }
    }
}

fn responder_matches_requester(responder: ResponderIdentity, requester: RequestedBy) -> bool {
    matches!(
        (responder, requester),
        (ResponderIdentity::User(a), RequestedBy::User(b)) if a == b
    ) || matches!(
        (responder, requester),
        (ResponderIdentity::TrustedUi(a), RequestedBy::User(b)) if a == b
    )
}

/// Production adapter for SunlightOS. Brain receives this narrow typed
/// capability; executable and Control Panel argument resolution stay inside
/// the authoritative shared launcher.
#[cfg(feature = "sunlightos")]
pub struct SunlightLaunchAdapter;

#[cfg(feature = "sunlightos")]
impl TrustedLaunchAdapter for SunlightLaunchAdapter {
    fn application_status(&self, bundle_id: &TypedIdentifier) -> RegistryStatus {
        if sunlight_libc::sun_exec::registered_application_available(bundle_id.as_str().as_bytes())
        {
            RegistryStatus::Registered
        } else {
            RegistryStatus::NotFound
        }
    }

    fn settings_page_status(&self, page_id: &TypedIdentifier) -> RegistryStatus {
        if sunlight_libc::sun_exec::settings_page_available(page_id.as_str().as_bytes()) {
            RegistryStatus::Registered
        } else {
            RegistryStatus::NotFound
        }
    }

    fn launch_application(&mut self, request: LaunchApplicationRequest) -> DispatchStatus {
        use sunlight_ipc::launch_trace::{LaunchSource, LaunchTrace};

        let trace = LaunchTrace::new(
            request.execution_id().0,
            LaunchSource::Runner,
            sunlight_ipc::monotonic_millis(),
        );
        match sunlight_libc::sun_exec::launch_registered_application(
            sunlight_libc::sun_exec::RegisteredApplicationRequest {
                trace,
                source: LaunchSource::Runner,
                app_id: request.bundle_id().as_str().as_bytes(),
            },
        ) {
            Ok(_) => DispatchStatus::Accepted,
            Err(_) => DispatchStatus::Failed,
        }
    }

    fn open_settings_page(&mut self, request: OpenSettingsPageRequest) -> DispatchStatus {
        use sunlight_ipc::launch_trace::{LaunchSource, LaunchTrace};
        use sunlight_libc::sun_exec::{
            open_registered_settings_page, ControlPanelPage, RegisteredSettingsPageRequest,
        };

        let Some(page) = ControlPanelPage::from_id(request.page_id().as_str().as_bytes()) else {
            return DispatchStatus::Failed;
        };
        let trace = LaunchTrace::new(
            request.execution_id().0,
            LaunchSource::Runner,
            sunlight_ipc::monotonic_millis(),
        );
        match open_registered_settings_page(RegisteredSettingsPageRequest {
            trace,
            source: LaunchSource::Runner,
            page,
        }) {
            Ok(_) => DispatchStatus::Accepted,
            Err(_) => DispatchStatus::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_intent::{
        ActionEvaluation, ActionIntent, ActionIntentEvaluator, CreationTime, Provenance, RiskHint,
    };
    use crate::confirmation::{
        ApprovalProof, ChallengeId, ConfirmationAuthority, ConfirmationNonce, ConfirmationResponse,
        ConfirmationResponseType, ResponseValidationContext,
    };
    use crate::policy::{
        ConfirmationLevel, PolicyCategory, PolicyEffect, PolicyOperation, PolicyRule,
    };

    static CONFIRM_APP_RULES: &[PolicyRule] = &[PolicyRule::new(
        PolicyOperation::OpenApplication,
        PolicyCategory::Execute,
        PolicyEffect::Confirm(ConfirmationLevel::Soft),
    )];

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Recorded {
        Application(LaunchApplicationRequest),
        Settings(OpenSettingsPageRequest),
    }

    #[derive(Default)]
    struct FakeLaunchAdapter {
        requests: std::vec::Vec<Recorded>,
        fail_dispatch: bool,
        registry_unavailable: bool,
    }

    impl TrustedLaunchAdapter for FakeLaunchAdapter {
        fn application_status(&self, bundle_id: &TypedIdentifier) -> RegistryStatus {
            if self.registry_unavailable {
                RegistryStatus::Unavailable
            } else if bundle_id.as_str() == "calculator" {
                RegistryStatus::Registered
            } else {
                RegistryStatus::NotFound
            }
        }

        fn settings_page_status(&self, page_id: &TypedIdentifier) -> RegistryStatus {
            if self.registry_unavailable {
                RegistryStatus::Unavailable
            } else if page_id.as_str() == "network" {
                RegistryStatus::Registered
            } else {
                RegistryStatus::NotFound
            }
        }

        fn launch_application(&mut self, request: LaunchApplicationRequest) -> DispatchStatus {
            self.requests.push(Recorded::Application(request));
            if self.fail_dispatch {
                DispatchStatus::Failed
            } else {
                DispatchStatus::Accepted
            }
        }

        fn open_settings_page(&mut self, request: OpenSettingsPageRequest) -> DispatchStatus {
            self.requests.push(Recorded::Settings(request));
            if self.fail_dispatch {
                DispatchStatus::Failed
            } else {
                DispatchStatus::Accepted
            }
        }
    }

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

    fn intent(
        operation: ActionOperation,
        target: ActionTarget,
        parameters: ActionParameters,
    ) -> ActionIntent {
        ActionIntent::new(
            IntentId::new([9; 16]),
            operation,
            target,
            parameters,
            RequestedBy::User(42),
            SessionId(7),
            1,
            CreationTime(101),
            RiskHint::Low,
            Provenance::ExplicitUserRequest,
        )
    }

    fn allowed_ready_with(
        policy: PolicyEngine,
        intent: &ActionIntent,
        runtime: &RuntimeContextSnapshot,
    ) -> ReadyForExecution {
        let ActionEvaluation::Decided(decision) =
            ActionIntentEvaluator::<8>::new(policy).evaluate(intent, runtime)
        else {
            panic!("expected policy decision");
        };
        assert_eq!(decision.result(), PolicyResult::Allowed);
        ConfirmationAuthority::<8, 16>::new(policy, 1_000)
            .produce_ready(
                intent,
                &decision,
                None,
                runtime,
                SessionAuthorization::new(
                    SessionId(7),
                    ResponderIdentity::User(42),
                    SessionStatus::Active,
                ),
                AuthorityTime(120),
            )
            .unwrap()
    }

    fn allowed_ready(intent: &ActionIntent, runtime: &RuntimeContextSnapshot) -> ReadyForExecution {
        allowed_ready_with(PolicyEngine::v1(), intent, runtime)
    }

    fn context<'a>(
        runtime: &'a RuntimeContextSnapshot,
        policy: &'a PolicyEngine,
        now: u64,
    ) -> ExecutionContext<'a> {
        ExecutionContext::new(
            runtime,
            policy,
            SessionAuthorization::new(
                SessionId(7),
                ResponderIdentity::User(42),
                SessionStatus::Active,
            ),
            RequestedBy::User(42),
            AuthorityTime(now),
        )
    }

    fn application_intent(id: &str) -> ActionIntent {
        intent(
            ActionOperation::OpenApplication,
            ActionTarget::Application(TypedIdentifier::new(id).unwrap()),
            ActionParameters::Application {
                new_instance: false,
            },
        )
    }

    fn settings_intent(id: &str) -> ActionIntent {
        intent(
            ActionOperation::OpenSettingsPage,
            ActionTarget::SettingsPage(TypedIdentifier::new(id).unwrap()),
            ActionParameters::Settings { focus: None },
        )
    }

    #[test]
    fn registered_application_dispatch_is_typed_and_acknowledged() {
        let runtime = runtime(1);
        let policy = PolicyEngine::v1();
        let ready = allowed_ready(&application_intent("calculator"), &runtime);
        let mut executor = TrustedActionExecutor::<_, 32, 8>::new(FakeLaunchAdapter::default());

        let result = executor.execute(ready, &context(&runtime, &policy, 130));

        assert_eq!(result.code(), ExecutionResultCode::Succeeded);
        assert!(result.dispatch_timestamp().is_some());
        assert!(matches!(
            executor.adapter.requests.as_slice(),
            [Recorded::Application(request)]
                if request.bundle_id().as_str() == "calculator"
                    && request.session_id() == SessionId(7)
        ));
    }

    #[test]
    fn registered_settings_page_dispatch_is_typed_and_acknowledged() {
        let runtime = runtime(1);
        let policy = PolicyEngine::v1();
        let ready = allowed_ready(&settings_intent("network"), &runtime);
        let mut executor = TrustedActionExecutor::<_, 32, 8>::new(FakeLaunchAdapter::default());

        let result = executor.execute(ready, &context(&runtime, &policy, 130));

        assert_eq!(result.code(), ExecutionResultCode::Succeeded);
        assert!(matches!(
            executor.adapter.requests.as_slice(),
            [Recorded::Settings(request)] if request.page_id().as_str() == "network"
        ));
    }

    #[test]
    fn unknown_targets_and_unavailable_registry_fail_without_dispatch() {
        let runtime = runtime(1);
        let policy = PolicyEngine::v1();
        for (ready, expected) in [
            (
                allowed_ready(&application_intent("missing"), &runtime),
                ExecutionResultCode::TargetNotFound,
            ),
            (
                allowed_ready(&settings_intent("missing"), &runtime),
                ExecutionResultCode::TargetNotFound,
            ),
        ] {
            let mut executor = TrustedActionExecutor::<_, 32, 8>::new(FakeLaunchAdapter::default());
            assert_eq!(
                executor
                    .execute(ready, &context(&runtime, &policy, 130))
                    .code(),
                expected
            );
            assert!(executor.adapter.requests.is_empty());
        }

        let ready = allowed_ready(&application_intent("calculator"), &runtime);
        let adapter = FakeLaunchAdapter {
            registry_unavailable: true,
            ..FakeLaunchAdapter::default()
        };
        let mut executor = TrustedActionExecutor::<_, 32, 8>::new(adapter);
        assert_eq!(
            executor
                .execute(ready, &context(&runtime, &policy, 130))
                .code(),
            ExecutionResultCode::TargetUnavailable
        );
    }

    #[test]
    fn unsupported_operation_and_nonempty_parameters_fail_closed() {
        let runtime = runtime(1);
        let policy = PolicyEngine::v1();
        let observe = intent(
            ActionOperation::Observe,
            ActionTarget::System,
            ActionParameters::Observe {
                include_health: false,
            },
        );
        let ready = allowed_ready(&observe, &runtime);
        let mut executor = TrustedActionExecutor::<_, 32, 8>::new(FakeLaunchAdapter::default());
        assert_eq!(
            executor
                .execute(ready, &context(&runtime, &policy, 130))
                .code(),
            ExecutionResultCode::UnsupportedOperation
        );

        let parameterized = intent(
            ActionOperation::OpenApplication,
            ActionTarget::Application(TypedIdentifier::new("calculator").unwrap()),
            ActionParameters::Application { new_instance: true },
        );
        let ready = allowed_ready(&parameterized, &runtime);
        assert_eq!(
            executor
                .execute(ready, &context(&runtime, &policy, 130))
                .code(),
            ExecutionResultCode::Rejected
        );
        assert!(executor.adapter.requests.is_empty());
    }

    #[test]
    fn duplicate_and_dispatch_failure_remain_consumed() {
        let runtime = runtime(1);
        let policy = PolicyEngine::v1();
        let ready = allowed_ready(&application_intent("calculator"), &runtime);
        let adapter = FakeLaunchAdapter {
            fail_dispatch: true,
            ..FakeLaunchAdapter::default()
        };
        let mut executor = TrustedActionExecutor::<_, 32, 8>::new(adapter);

        assert_eq!(
            executor
                .execute(ready.clone(), &context(&runtime, &policy, 130))
                .code(),
            ExecutionResultCode::DispatchFailed
        );
        assert_eq!(
            executor
                .execute(ready, &context(&runtime, &policy, 131))
                .code(),
            ExecutionResultCode::AlreadyConsumed
        );
        assert_eq!(executor.adapter.requests.len(), 1);
    }

    #[test]
    fn runtime_policy_session_and_requester_are_revalidated_at_dispatch() {
        let runtime = runtime(1);
        let policy = PolicyEngine::v1();

        let ready = allowed_ready(&application_intent("calculator"), &runtime);
        let mut executor = TrustedActionExecutor::<_, 32, 8>::new(FakeLaunchAdapter::default());
        assert_eq!(
            executor
                .execute(ready, &context(&runtime, &policy, 5_101))
                .code(),
            ExecutionResultCode::RuntimeStale
        );

        let ready = allowed_ready(&application_intent("calculator"), &runtime);
        let changed = PolicyEngine::from_static_rules(
            crate::policy::PolicyVersion::new(2, 0),
            CONFIRM_APP_RULES,
        );
        assert_eq!(
            executor
                .execute(ready, &context(&runtime, &changed, 130))
                .code(),
            ExecutionResultCode::PolicyChanged
        );

        let ready = allowed_ready(&application_intent("calculator"), &runtime);
        let inactive = ExecutionContext::new(
            &runtime,
            &policy,
            SessionAuthorization::new(
                SessionId(7),
                ResponderIdentity::User(42),
                SessionStatus::Inactive,
            ),
            RequestedBy::User(42),
            AuthorityTime(130),
        );
        assert_eq!(
            executor.execute(ready, &inactive).code(),
            ExecutionResultCode::SessionInactive
        );

        let ready = allowed_ready(&application_intent("calculator"), &runtime);
        let wrong_requester = ExecutionContext::new(
            &runtime,
            &policy,
            SessionAuthorization::new(
                SessionId(7),
                ResponderIdentity::User(42),
                SessionStatus::Active,
            ),
            RequestedBy::User(99),
            AuthorityTime(130),
        );
        assert_eq!(
            executor.execute(ready, &wrong_requester).code(),
            ExecutionResultCode::Rejected
        );
        assert!(executor.adapter.requests.is_empty());
    }

    #[test]
    fn expired_confirmation_is_not_consumed_or_dispatched() {
        let runtime = runtime(1);
        let policy = PolicyEngine::from_static_rules(
            crate::policy::PolicyVersion::new(7, 1),
            CONFIRM_APP_RULES,
        );
        let intent = application_intent("calculator");
        let ActionEvaluation::Decided(decision) =
            ActionIntentEvaluator::<8>::new(policy).evaluate(&intent, &runtime)
        else {
            panic!("expected confirmation decision");
        };
        let mut authority = ConfirmationAuthority::<8, 16>::new(policy, 1_000);
        authority
            .create_challenge(
                &intent,
                &decision,
                ChallengeId::new([1; 16]),
                ConfirmationNonce::new([2; 16]),
                ResponderIdentity::User(42),
                AuthorityTime(110),
                AuthorityTime(140),
            )
            .unwrap();
        let challenge = authority
            .issue_challenge(ChallengeId::new([1; 16]))
            .unwrap();
        let grant = authority
            .submit_response(
                &ConfirmationResponse::new(
                    challenge.challenge_id(),
                    SessionId(7),
                    ResponderIdentity::User(42),
                    ConfirmationResponseType::Approved(ApprovalProof::SoftExplicit),
                    AuthorityTime(120),
                ),
                ResponseValidationContext::new(
                    &intent,
                    &runtime,
                    SessionAuthorization::new(
                        SessionId(7),
                        ResponderIdentity::User(42),
                        SessionStatus::Active,
                    ),
                ),
            )
            .unwrap()
            .unwrap();
        let ready = authority
            .produce_ready(
                &intent,
                &decision,
                Some(&grant),
                &runtime,
                SessionAuthorization::new(
                    SessionId(7),
                    ResponderIdentity::User(42),
                    SessionStatus::Active,
                ),
                AuthorityTime(130),
            )
            .unwrap();
        let mut executor = TrustedActionExecutor::<_, 32, 8>::new(FakeLaunchAdapter::default());

        assert_eq!(
            executor
                .execute(ready, &context(&runtime, &policy, 141))
                .code(),
            ExecutionResultCode::ConfirmationExpired
        );
        assert!(executor.adapter.requests.is_empty());
        assert!(!executor
            .audit()
            .entries()
            .any(|entry| matches!(entry.event, ExecutionAuditEvent::ConfirmationConsumed)));
    }

    #[test]
    fn audit_is_bounded_and_contains_no_target_text_or_payloads() {
        let runtime = runtime(1);
        let policy = PolicyEngine::v1();
        let mut executor = TrustedActionExecutor::<_, 3, 8>::new(FakeLaunchAdapter::default());
        let ready = allowed_ready(&application_intent("calculator"), &runtime);
        let _ = executor.execute(ready, &context(&runtime, &policy, 130));

        assert_eq!(executor.audit().len(), 3);
        assert!(executor.audit().evicted() > 0);
        let rendered = std::format!(
            "{:?}",
            executor.audit().entries().collect::<std::vec::Vec<_>>()
        );
        assert!(!rendered.contains("calculator"));
        assert!(!rendered.contains("/bin/"));
        assert!(!rendered.contains("--page"));
    }

    #[test]
    fn execution_api_has_only_ready_envelope_and_typed_dispatch_requests() {
        fn execute_ready<E: ActionExecutor>(
            executor: &mut E,
            ready: ReadyForExecution,
            context: &ExecutionContext<'_>,
        ) -> ExecutionResult {
            executor.execute(ready, context)
        }
        let _ = execute_ready::<TrustedActionExecutor<FakeLaunchAdapter, 8, 8>>;

        assert!(!core::any::type_name::<LaunchApplicationRequest>().contains("String"));
        assert!(!core::any::type_name::<OpenSettingsPageRequest>().contains("String"));
    }
}
