#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", allow(static_mut_refs))]

extern crate alloc;

pub mod action_intent;
pub mod adapters;
pub mod caps;
pub mod confirmation;
pub mod context;
pub mod diagnostics;
pub mod error;
pub mod executor;
pub mod foundation;
pub mod greeting;
pub mod grounded;
pub mod kv_client;
pub mod memory_layers;
pub mod mtm;
pub mod native_ipc;
pub mod pipeline;
pub mod planner;
pub mod policy;
pub mod protocol;
pub mod provenance;
pub mod provider;
pub mod runtime_context;

pub use action_intent::{
    ActionDecision, ActionEvaluation, ActionIntent, ActionIntentEvaluator, ActionOperation,
    ActionParameters, ActionTarget, AuditEntry, AuditId, AuditLog, BoundedFilePath,
    ConfirmationBinding, CreationTime, IdentifierError, IntentId, IntentValidation, ParameterValue,
    Provenance, RequestedBy, RiskHint, SessionId, TargetKind, TypedIdentifier, ValidationReason,
};
pub use confirmation::{
    ApprovalProof, AuthorityError, AuthorityTime, BindingDigest, ChallengeId,
    ConfirmationAuditEvent, ConfirmationAuditLog, ConfirmationAuthority, ConfirmationChallenge,
    ConfirmationChoice, ConfirmationGrant, ConfirmationNonce, ConfirmationResponse,
    ConfirmationResponseType, ConfirmationState, ConfirmationView, ConsequenceAcknowledgement,
    FinalValidationResult, GrantId, ReadinessDenialReason, ReadyForExecution, ResponderIdentity,
    ResponseValidationContext, SessionAuthorization, SessionStatus,
};
pub use diagnostics::BrainDiagnostics;
#[cfg(feature = "sunlightos")]
pub use executor::SunlightLaunchAdapter;
pub use executor::{
    ActionExecutor, DispatchStatus, ExecutionAuditEntry, ExecutionAuditEvent, ExecutionAuditLog,
    ExecutionContext, ExecutionId, ExecutionResult, ExecutionResultCode, LaunchApplicationRequest,
    OpenSettingsPageRequest, RegistryStatus, TrustedActionExecutor, TrustedActionFlow,
    TrustedLaunchAdapter,
};
pub use mtm::{BrainPreferences, GreetingStyle, WelcomeMemoryState};
pub use pipeline::CognitivePipeline;
#[cfg(feature = "sunlightos")]
pub use planner::SunlightPlannerRegistry;
pub use planner::{
    ActionIntentDraft, BoundedActionPlanner, ClarificationCandidate, ClarificationId,
    ClarificationRequest, ConfidenceClass, ConversationId, DraftProvenance, EvidenceSpan,
    InvalidInputReason, NormalizedParameters, PlannerAuditEntry, PlannerAuditLog,
    PlannerAuditResult, PlannerContext, PlannerInput, PlannerInputError, PlannerInputProvenance,
    PlannerRegistry, PlannerRequestId, PlannerResult, PlannerTargetKind, PlannerVersion,
    ProposedTarget, RegistryAliasRef, RegistryTargetRef, UnsupportedReason, ALIAS_MODEL_V1,
    PLANNER_V1,
};
pub use policy::{
    ConfirmationLevel, PolicyCategory, PolicyEngine, PolicyOperation, PolicyReason, PolicyResult,
    PolicyRule, PolicyVersion,
};
pub use provider::{BrainProvider, FutureOnlineProvider, LocalBoundedProvider, ProviderRegistry};
pub use runtime_context::{
    ContextProvider, ContextProviderError, RefreshClass, RuntimeContextCache,
    RuntimeContextSnapshot,
};
