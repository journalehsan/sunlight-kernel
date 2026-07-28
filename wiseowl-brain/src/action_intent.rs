//! Typed, non-executing Action Intent boundary for Wise Owl.
//!
//! Reasoning may construct a proposal. This module validates and evaluates it,
//! but intentionally contains no executor, syscall, service, or IPC adapter.

use core::fmt;
use heapless::{Deque, String};

use crate::policy::{
    ConfirmationLevel, PolicyDecision, PolicyEngine, PolicyOperation, PolicyReason, PolicyResult,
    PolicyVersion,
};
use crate::runtime_context::RuntimeContextSnapshot;

pub const MAX_IDENTIFIER_LEN: usize = 64;
pub const MAX_FILE_PATH_LEN: usize = 160;
pub const MAX_PARAMETER_VALUE_LEN: usize = 96;
pub const DEFAULT_AUDIT_CAPACITY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntentId([u8; 16]);

impl IntentId {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CreationTime(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuditId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionOperation {
    Observe,
    OpenApplication,
    OpenSettingsPage,
    LaunchUtility,
    RestartService,
    StopService,
    InstallPackage,
    RemovePackage,
    ModifyFile,
    DeleteFile,
    ModifyBootConfiguration,
    EraseDisk,
    UnknownOperation(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskHint {
    Low,
    Moderate,
    High,
    Critical,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestedBy {
    User(u32),
    WiseOwlReasoning,
    SystemComponent(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    Conversation,
    ExplicitUserRequest,
    SystemRecommendation,
    Other(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierError {
    Empty,
    Oversized,
    Malformed,
}

/// A bounded identifier. It permits ASCII letters, digits, `.`, `_`, and `-`,
/// but never whitespace, separators, or command syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedIdentifier(String<MAX_IDENTIFIER_LEN>);

impl TypedIdentifier {
    pub fn new(value: &str) -> Result<Self, IdentifierError> {
        bounded_identifier(value).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedFilePath(String<MAX_FILE_PATH_LEN>);

impl BoundedFilePath {
    pub fn new(value: &str) -> Result<Self, IdentifierError> {
        if value.is_empty() {
            return Err(IdentifierError::Empty);
        }
        if value.len() > MAX_FILE_PATH_LEN {
            return Err(IdentifierError::Oversized);
        }
        if !value.starts_with('/')
            || value.bytes().any(|byte| {
                byte == 0 || byte < b' ' || matches!(byte, b'\'' | b'"' | b'`' | b'|' | b'&' | b';')
            })
            || value.split('/').any(|part| part == "..")
        {
            return Err(IdentifierError::Malformed);
        }
        String::try_from(value)
            .map(Self)
            .map_err(|_| IdentifierError::Oversized)
    }

    /// Intended for display only in trusted UI. Audit entries never include it.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionTarget {
    Application(TypedIdentifier),
    SettingsPage(TypedIdentifier),
    Utility(TypedIdentifier),
    Service(TypedIdentifier),
    Package(TypedIdentifier),
    File(BoundedFilePath),
    Disk(TypedIdentifier),
    System,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterValue(String<MAX_PARAMETER_VALUE_LEN>);

impl ParameterValue {
    pub fn new(value: &str) -> Result<Self, IdentifierError> {
        if value.is_empty() {
            return Err(IdentifierError::Empty);
        }
        if value.len() > MAX_PARAMETER_VALUE_LEN {
            return Err(IdentifierError::Oversized);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
        {
            return Err(IdentifierError::Malformed);
        }
        String::try_from(value)
            .map(Self)
            .map_err(|_| IdentifierError::Oversized)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Each variant is meaningful only for its corresponding operation. Validation
/// rejects every unsupported combination instead of discarding fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionParameters {
    None,
    Observe { include_health: bool },
    Application { new_instance: bool },
    Settings { focus: Option<TypedIdentifier> },
    Utility { mode: Option<TypedIdentifier> },
    Service { force: bool },
    Package { version: Option<ParameterValue> },
    File { recursive: bool },
    Disk { whole_device: bool },
}

/// Immutable outside this module: all fields are private and set at creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionIntent {
    intent_id: IntentId,
    operation: ActionOperation,
    target: ActionTarget,
    parameters: ActionParameters,
    requested_by: RequestedBy,
    session_id: SessionId,
    runtime_snapshot_generation: u64,
    creation_time: CreationTime,
    risk_hint: RiskHint,
    provenance: Provenance,
}

impl ActionIntent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        intent_id: IntentId,
        operation: ActionOperation,
        target: ActionTarget,
        parameters: ActionParameters,
        requested_by: RequestedBy,
        session_id: SessionId,
        runtime_snapshot_generation: u64,
        creation_time: CreationTime,
        risk_hint: RiskHint,
        provenance: Provenance,
    ) -> Self {
        Self {
            intent_id,
            operation,
            target,
            parameters,
            requested_by,
            session_id,
            runtime_snapshot_generation,
            creation_time,
            risk_hint,
            provenance,
        }
    }

    pub const fn intent_id(&self) -> IntentId {
        self.intent_id
    }
    pub const fn operation(&self) -> ActionOperation {
        self.operation
    }
    pub const fn requested_by(&self) -> RequestedBy {
        self.requested_by
    }
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub const fn runtime_snapshot_generation(&self) -> u64 {
        self.runtime_snapshot_generation
    }
    pub const fn creation_time(&self) -> CreationTime {
        self.creation_time
    }
    pub const fn risk_hint(&self) -> RiskHint {
        self.risk_hint
    }
    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }
    pub fn target(&self) -> &ActionTarget {
        &self.target
    }
    pub fn parameters(&self) -> &ActionParameters {
        &self.parameters
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentValidation {
    Valid,
    Invalid,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationReason {
    WellFormed,
    UnknownOperation,
    UnknownTarget,
    TargetOperationMismatch,
    UnsupportedParameters,
    AmbiguousTarget,
    StaleRuntimeSnapshot,
}

/// Proof that validation succeeded. It cannot be constructed by callers.
#[derive(Debug, Clone, Copy)]
pub struct ValidatedActionIntent<'a> {
    intent: &'a ActionIntent,
    policy_operation: PolicyOperation,
}

impl<'a> ValidatedActionIntent<'a> {
    pub fn intent(&self) -> &'a ActionIntent {
        self.intent
    }
}

#[derive(Debug, Clone, Copy)]
struct Validation<'a> {
    status: IntentValidation,
    reason: ValidationReason,
    valid: Option<ValidatedActionIntent<'a>>,
}

fn validate<'a>(intent: &'a ActionIntent, runtime_generation: u64) -> Validation<'a> {
    if matches!(intent.operation, ActionOperation::UnknownOperation(_)) {
        return invalid(
            IntentValidation::Unknown,
            ValidationReason::UnknownOperation,
        );
    }
    if matches!(intent.target, ActionTarget::Unknown) {
        return invalid(IntentValidation::Unknown, ValidationReason::UnknownTarget);
    }
    if intent.runtime_snapshot_generation != runtime_generation {
        return invalid(
            IntentValidation::Invalid,
            ValidationReason::StaleRuntimeSnapshot,
        );
    }

    let operation = match (&intent.operation, &intent.target, &intent.parameters) {
        (ActionOperation::Observe, ActionTarget::System, ActionParameters::Observe { .. }) => {
            PolicyOperation::ObserveRuntime
        }
        (
            ActionOperation::OpenApplication,
            ActionTarget::Application(_),
            ActionParameters::Application { .. },
        ) => PolicyOperation::OpenApplication,
        (
            ActionOperation::OpenSettingsPage,
            ActionTarget::SettingsPage(_),
            ActionParameters::Settings { .. },
        ) => PolicyOperation::OpenSettingsPage,
        (
            ActionOperation::LaunchUtility,
            ActionTarget::Utility(_),
            ActionParameters::Utility { .. },
        ) => PolicyOperation::LaunchUtility,
        (
            ActionOperation::RestartService,
            ActionTarget::Service(_),
            ActionParameters::Service { .. },
        ) => PolicyOperation::RestartService,
        (
            ActionOperation::StopService,
            ActionTarget::Service(_),
            ActionParameters::Service { .. },
        ) => PolicyOperation::StopService,
        (
            ActionOperation::InstallPackage,
            ActionTarget::Package(_),
            ActionParameters::Package { .. },
        ) => PolicyOperation::InstallPackage,
        (
            ActionOperation::RemovePackage,
            ActionTarget::Package(_),
            ActionParameters::Package { .. },
        ) => PolicyOperation::RemovePackage,
        (ActionOperation::ModifyFile, ActionTarget::File(_), ActionParameters::File { .. }) => {
            PolicyOperation::ModifyFile
        }
        (ActionOperation::DeleteFile, ActionTarget::File(_), ActionParameters::File { .. }) => {
            PolicyOperation::DeleteFiles
        }
        (
            ActionOperation::ModifyBootConfiguration,
            ActionTarget::System,
            ActionParameters::None,
        ) => PolicyOperation::ModifyBootloader,
        (
            ActionOperation::EraseDisk,
            ActionTarget::Disk(_),
            ActionParameters::Disk { whole_device: true },
        ) => PolicyOperation::DiskErase,
        (ActionOperation::EraseDisk, ActionTarget::Disk(_), ActionParameters::Disk { .. }) => {
            return invalid(
                IntentValidation::Unsupported,
                ValidationReason::UnsupportedParameters,
            )
        }
        (operation, target, _) if target_kind_matches(*operation, target) => {
            return invalid(
                IntentValidation::Unsupported,
                ValidationReason::UnsupportedParameters,
            )
        }
        _ => {
            return invalid(
                IntentValidation::Invalid,
                ValidationReason::TargetOperationMismatch,
            )
        }
    };

    Validation {
        status: IntentValidation::Valid,
        reason: ValidationReason::WellFormed,
        valid: Some(ValidatedActionIntent {
            intent,
            policy_operation: operation,
        }),
    }
}

fn invalid<'a>(status: IntentValidation, reason: ValidationReason) -> Validation<'a> {
    Validation {
        status,
        reason,
        valid: None,
    }
}

fn target_kind_matches(operation: ActionOperation, target: &ActionTarget) -> bool {
    matches!(
        (operation, target),
        (ActionOperation::Observe, ActionTarget::System)
            | (
                ActionOperation::OpenApplication,
                ActionTarget::Application(_)
            )
            | (
                ActionOperation::OpenSettingsPage,
                ActionTarget::SettingsPage(_)
            )
            | (ActionOperation::LaunchUtility, ActionTarget::Utility(_))
            | (
                ActionOperation::RestartService | ActionOperation::StopService,
                ActionTarget::Service(_)
            )
            | (
                ActionOperation::InstallPackage | ActionOperation::RemovePackage,
                ActionTarget::Package(_)
            )
            | (
                ActionOperation::ModifyFile | ActionOperation::DeleteFile,
                ActionTarget::File(_)
            )
            | (
                ActionOperation::ModifyBootConfiguration,
                ActionTarget::System
            )
            | (ActionOperation::EraseDisk, ActionTarget::Disk(_))
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Application,
    SettingsPage,
    Utility,
    Service,
    Package,
    File,
    Disk,
    System,
    Unknown,
}

impl ActionTarget {
    pub const fn kind(&self) -> TargetKind {
        match self {
            Self::Application(_) => TargetKind::Application,
            Self::SettingsPage(_) => TargetKind::SettingsPage,
            Self::Utility(_) => TargetKind::Utility,
            Self::Service(_) => TargetKind::Service,
            Self::Package(_) => TargetKind::Package,
            Self::File(_) => TargetKind::File,
            Self::Disk(_) => TargetKind::Disk,
            Self::System => TargetKind::System,
            Self::Unknown => TargetKind::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditEntry {
    IntentProposed {
        intent_id: IntentId,
        operation: ActionOperation,
        target_kind: TargetKind,
    },
    Validation {
        intent_id: IntentId,
        result: IntentValidation,
        reason: ValidationReason,
    },
    PolicyDecision {
        audit_id: AuditId,
        intent_id: IntentId,
        result: PolicyResult,
        reason: PolicyReason,
        policy_version: PolicyVersion,
    },
}

/// Fixed-capacity, in-memory audit. Oldest entries are explicitly evicted.
pub struct AuditLog<const N: usize = DEFAULT_AUDIT_CAPACITY> {
    entries: Deque<AuditEntry, N>,
    evicted: u64,
}

impl<const N: usize> AuditLog<N> {
    pub const fn new() -> Self {
        Self {
            entries: Deque::new(),
            evicted: 0,
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = &AuditEntry> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub const fn evicted(&self) -> u64 {
        self.evicted
    }

    fn record(&mut self, entry: AuditEntry) {
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

impl<const N: usize> Default for AuditLog<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Public, immutable decision envelope. It reveals only stable public reasons.
#[derive(Clone, PartialEq, Eq)]
pub struct ActionDecision {
    intent_id: IntentId,
    policy_version: PolicyVersion,
    result: PolicyResult,
    confirmation_level: ConfirmationLevel,
    public_reason_code: PolicyReason,
    runtime_snapshot_generation: u64,
    audit_id: AuditId,
    bound_operation: ActionOperation,
    bound_target: ActionTarget,
    bound_parameters: ActionParameters,
}

impl fmt::Debug for ActionDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActionDecision")
            .field("intent_id", &self.intent_id)
            .field("policy_version", &self.policy_version)
            .field("result", &self.result)
            .field("confirmation_level", &self.confirmation_level)
            .field("public_reason_code", &self.public_reason_code)
            .field(
                "runtime_snapshot_generation",
                &self.runtime_snapshot_generation,
            )
            .field("audit_id", &self.audit_id)
            .finish_non_exhaustive()
    }
}

impl ActionDecision {
    pub const fn intent_id(&self) -> IntentId {
        self.intent_id
    }
    pub const fn policy_version(&self) -> PolicyVersion {
        self.policy_version
    }
    pub const fn result(&self) -> PolicyResult {
        self.result
    }
    pub const fn confirmation_level(&self) -> ConfirmationLevel {
        self.confirmation_level
    }
    pub const fn public_reason_code(&self) -> PolicyReason {
        self.public_reason_code
    }
    pub const fn runtime_snapshot_generation(&self) -> u64 {
        self.runtime_snapshot_generation
    }
    pub const fn audit_id(&self) -> AuditId {
        self.audit_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionEvaluation {
    Decided(ActionDecision),
    Rejected {
        intent_id: IntentId,
        result: IntentValidation,
        reason: ValidationReason,
    },
}

/// Exact, reusable-only-for-the-same-proposal confirmation preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationBinding {
    intent_id: IntentId,
    operation: ActionOperation,
    target: ActionTarget,
    parameters: ActionParameters,
    policy_version: PolicyVersion,
    runtime_snapshot_generation: u64,
}

impl ConfirmationBinding {
    pub fn from_required(decision: &ActionDecision) -> Option<Self> {
        if decision.result != PolicyResult::ConfirmationRequired {
            return None;
        }
        Some(Self {
            intent_id: decision.intent_id,
            operation: decision.bound_operation,
            target: decision.bound_target.clone(),
            parameters: decision.bound_parameters.clone(),
            policy_version: decision.policy_version,
            runtime_snapshot_generation: decision.runtime_snapshot_generation,
        })
    }

    pub fn matches(
        &self,
        intent: &ActionIntent,
        policy_version: PolicyVersion,
        current_runtime_generation: u64,
    ) -> bool {
        self.intent_id == intent.intent_id
            && self.operation == intent.operation
            && self.target == intent.target
            && self.parameters == intent.parameters
            && self.policy_version == policy_version
            && self.runtime_snapshot_generation == intent.runtime_snapshot_generation
            && self.runtime_snapshot_generation == current_runtime_generation
    }
}

pub struct ActionIntentEvaluator<const N: usize = DEFAULT_AUDIT_CAPACITY> {
    policy: PolicyEngine,
    audit: AuditLog<N>,
    next_audit_id: u64,
}

impl<const N: usize> ActionIntentEvaluator<N> {
    pub const fn new(policy: PolicyEngine) -> Self {
        Self {
            policy,
            audit: AuditLog::new(),
            next_audit_id: 1,
        }
    }

    pub const fn policy(&self) -> &PolicyEngine {
        &self.policy
    }

    pub const fn audit(&self) -> &AuditLog<N> {
        &self.audit
    }

    pub fn evaluate(
        &mut self,
        intent: &ActionIntent,
        runtime: &RuntimeContextSnapshot,
    ) -> ActionEvaluation {
        self.audit.record(AuditEntry::IntentProposed {
            intent_id: intent.intent_id,
            operation: intent.operation,
            target_kind: intent.target.kind(),
        });
        let validation = validate(intent, runtime.generation);
        self.audit.record(AuditEntry::Validation {
            intent_id: intent.intent_id,
            result: validation.status,
            reason: validation.reason,
        });
        let Some(valid) = validation.valid else {
            return ActionEvaluation::Rejected {
                intent_id: intent.intent_id,
                result: validation.status,
                reason: validation.reason,
            };
        };

        let policy = self.policy.evaluate(valid.policy_operation, runtime);
        let audit_id = AuditId(self.next_audit_id);
        self.next_audit_id = self.next_audit_id.saturating_add(1);
        let decision = decision_envelope(valid, policy, runtime.generation, audit_id);
        self.audit.record(AuditEntry::PolicyDecision {
            audit_id,
            intent_id: intent.intent_id,
            result: decision.result,
            reason: decision.public_reason_code,
            policy_version: decision.policy_version,
        });
        ActionEvaluation::Decided(decision)
    }
}

fn decision_envelope(
    valid: ValidatedActionIntent<'_>,
    policy: PolicyDecision,
    runtime_generation: u64,
    audit_id: AuditId,
) -> ActionDecision {
    ActionDecision {
        intent_id: valid.intent.intent_id,
        policy_version: policy.version,
        result: policy.result,
        confirmation_level: policy.confirmation,
        public_reason_code: policy.reason,
        runtime_snapshot_generation: runtime_generation,
        audit_id,
        bound_operation: valid.intent.operation,
        bound_target: valid.intent.target.clone(),
        bound_parameters: valid.intent.parameters.clone(),
    }
}

fn bounded_identifier(value: &str) -> Result<String<MAX_IDENTIFIER_LEN>, IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if value.len() > MAX_IDENTIFIER_LEN {
        return Err(IdentifierError::Oversized);
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(IdentifierError::Malformed);
    }
    String::try_from(value).map_err(|_| IdentifierError::Oversized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u8) -> IntentId {
        IntentId::new([value; 16])
    }

    fn target_id(value: &str) -> TypedIdentifier {
        TypedIdentifier::new(value).unwrap()
    }

    fn intent(
        operation: ActionOperation,
        target: ActionTarget,
        parameters: ActionParameters,
        generation: u64,
    ) -> ActionIntent {
        ActionIntent::new(
            id(1),
            operation,
            target,
            parameters,
            RequestedBy::WiseOwlReasoning,
            SessionId(7),
            generation,
            CreationTime(100),
            RiskHint::Low,
            Provenance::Conversation,
        )
    }

    fn runtime(generation: u64) -> RuntimeContextSnapshot {
        RuntimeContextSnapshot {
            generation,
            ..RuntimeContextSnapshot::default()
        }
    }

    #[test]
    fn valid_observe_intent_is_allowed() {
        let proposal = intent(
            ActionOperation::Observe,
            ActionTarget::System,
            ActionParameters::Observe {
                include_health: true,
            },
            4,
        );
        let result =
            ActionIntentEvaluator::<8>::new(PolicyEngine::v1()).evaluate(&proposal, &runtime(4));

        assert!(matches!(
            result,
            ActionEvaluation::Decided(decision)
                if decision.result() == PolicyResult::Allowed
        ));
    }

    #[test]
    fn valid_application_launch_is_allowed() {
        let proposal = intent(
            ActionOperation::OpenApplication,
            ActionTarget::Application(target_id("org.sunlight.Calculator")),
            ActionParameters::Application {
                new_instance: false,
            },
            9,
        );
        let result =
            ActionIntentEvaluator::<8>::new(PolicyEngine::v1()).evaluate(&proposal, &runtime(9));

        assert!(matches!(
            result,
            ActionEvaluation::Decided(decision)
                if decision.result() == PolicyResult::Allowed
        ));
    }

    #[test]
    fn malformed_target_and_oversized_parameter_are_rejected() {
        assert_eq!(
            TypedIdentifier::new("../../bin/sh"),
            Err(IdentifierError::Malformed)
        );
        let oversized = "x".repeat(MAX_PARAMETER_VALUE_LEN + 1);
        assert_eq!(
            ParameterValue::new(&oversized),
            Err(IdentifierError::Oversized)
        );
    }

    #[test]
    fn unknown_operation_never_reaches_policy() {
        let proposal = intent(
            ActionOperation::UnknownOperation(700),
            ActionTarget::System,
            ActionParameters::None,
            1,
        );
        let mut evaluator = ActionIntentEvaluator::<8>::new(PolicyEngine::v1());
        let result = evaluator.evaluate(&proposal, &runtime(1));

        assert!(matches!(
            result,
            ActionEvaluation::Rejected {
                result: IntentValidation::Unknown,
                ..
            }
        ));
        assert_eq!(evaluator.audit().len(), 2);
    }

    #[test]
    fn denied_and_confirmation_required_policy_results_are_preserved() {
        let boot = intent(
            ActionOperation::ModifyBootConfiguration,
            ActionTarget::System,
            ActionParameters::None,
            1,
        );
        let restart = intent(
            ActionOperation::RestartService,
            ActionTarget::Service(target_id("networkd")),
            ActionParameters::Service { force: false },
            1,
        );
        let mut evaluator = ActionIntentEvaluator::<8>::new(PolicyEngine::v1());

        assert!(matches!(
            evaluator.evaluate(&boot, &runtime(1)),
            ActionEvaluation::Decided(decision)
                if decision.result() == PolicyResult::Denied
        ));
        assert!(matches!(
            evaluator.evaluate(&restart, &runtime(1)),
            ActionEvaluation::Decided(decision)
                if decision.result() == PolicyResult::ConfirmationRequired
        ));
    }

    #[test]
    fn changed_parameters_invalidate_confirmation_binding() {
        let original = intent(
            ActionOperation::RestartService,
            ActionTarget::Service(target_id("networkd")),
            ActionParameters::Service { force: false },
            3,
        );
        let changed = ActionIntent::new(
            original.intent_id(),
            original.operation(),
            original.target().clone(),
            ActionParameters::Service { force: true },
            original.requested_by(),
            original.session_id(),
            original.runtime_snapshot_generation(),
            original.creation_time(),
            original.risk_hint(),
            original.provenance(),
        );
        let mut evaluator = ActionIntentEvaluator::<8>::new(PolicyEngine::v1());
        let ActionEvaluation::Decided(decision) = evaluator.evaluate(&original, &runtime(3)) else {
            panic!("expected policy decision");
        };
        let binding = ConfirmationBinding::from_required(&decision).unwrap();

        assert!(binding.matches(&original, PolicyVersion::new(1, 0), 3));
        assert!(!binding.matches(&changed, PolicyVersion::new(1, 0), 3));
    }

    #[test]
    fn stale_snapshot_is_rejected_before_policy() {
        let proposal = intent(
            ActionOperation::Observe,
            ActionTarget::System,
            ActionParameters::Observe {
                include_health: false,
            },
            2,
        );
        let result =
            ActionIntentEvaluator::<8>::new(PolicyEngine::v1()).evaluate(&proposal, &runtime(3));

        assert!(matches!(
            result,
            ActionEvaluation::Rejected {
                result: IntentValidation::Invalid,
                reason: ValidationReason::StaleRuntimeSnapshot,
                ..
            }
        ));
    }

    #[test]
    fn audit_is_bounded_and_redacts_target_values() {
        let proposal = intent(
            ActionOperation::DeleteFile,
            ActionTarget::File(BoundedFilePath::new("/private/user/secret.txt").unwrap()),
            ActionParameters::File { recursive: false },
            1,
        );
        let mut evaluator = ActionIntentEvaluator::<2>::new(PolicyEngine::v1());
        let _ = evaluator.evaluate(&proposal, &runtime(1));

        assert_eq!(evaluator.audit().len(), 2);
        assert_eq!(evaluator.audit().evicted(), 1);
        assert!(evaluator.audit().entries().all(|entry| match entry {
            AuditEntry::IntentProposed { target_kind, .. } => *target_kind == TargetKind::File,
            _ => true,
        }));
    }

    #[test]
    fn evaluation_api_has_no_execution_result_or_executor() {
        let proposal = intent(
            ActionOperation::Observe,
            ActionTarget::System,
            ActionParameters::Observe {
                include_health: false,
            },
            1,
        );
        let result =
            ActionIntentEvaluator::<4>::new(PolicyEngine::v1()).evaluate(&proposal, &runtime(1));
        assert!(matches!(result, ActionEvaluation::Decided(_)));
    }
}
