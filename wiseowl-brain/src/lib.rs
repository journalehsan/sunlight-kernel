#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", allow(static_mut_refs))]

extern crate alloc;

pub mod action_intent;
pub mod action_receipt;
pub mod adapters;
pub mod caps;
pub mod confirmation;
pub mod context;
pub mod coordinator;
pub mod delegated_session_lifecycle;
pub mod diagnostics;
pub mod error;
pub mod executor;
pub mod foundation;
pub mod greeting;
pub mod grounded;
pub mod gui_bridge;
pub mod gui_live_action_activation;
pub mod kv_client;
pub mod memory_layers;
pub mod mtm;
pub mod native_ipc;
pub mod outcome;
pub mod pipeline;
pub mod planner;
pub mod policy;
pub mod protocol;
pub mod provenance;
pub mod provider;
pub mod runtime_context;
pub mod trusted_session_readiness;

pub use action_intent::{
    ActionDecision, ActionEvaluation, ActionIntent, ActionIntentEvaluator, ActionOperation,
    ActionParameters, ActionTarget, AuditEntry, AuditId, AuditLog, BoundedFilePath,
    ConfirmationBinding, CreationTime, IdentifierError, IntentId, IntentValidation, ParameterValue,
    Provenance, RequestedBy, RiskHint, SessionId, TargetKind, TypedIdentifier, ValidationReason,
};
pub use action_receipt::{
    ActionReceipt, ActionReceiptId, ActionReceiptLedger, ActionReceiptLifecycleEvent,
    ActionReceiptTerminalStatus, ActionReceiptView, AppendDisposition, ConfirmationReceiptOutcome,
    ConfirmationReceiptSummary, DispatchReceiptStatus, DispatchReceiptSummary,
    ObservedOutcomeReceiptSummary, ObservedReceiptOutcome, PendingActionReceiptView,
    PendingReceiptState, PolicyReceiptSummary, ReceiptAuditEntry, ReceiptAuditEvent,
    ReceiptAuditLog, ReceiptConversationAnswer, ReceiptConversationQuestion, ReceiptError,
    ReceiptEventSource, ReceiptLifecycleEventType, ReceiptOpen, ReceiptPersistence,
    ReceiptPersistenceError, ReceiptQuery, ReceiptQueryKind, ReceiptQueryResult,
    ReceiptReadinessAnswer, ReceiptRelevantIds, ReceiptRetentionPolicy, TargetDisplayKey,
    VolatileReceiptPersistence, ACTION_RECEIPT_DIGEST_DOMAIN, ACTION_RECEIPT_SCHEMA_VERSION,
};
pub use confirmation::{
    ApprovalProof, AuthorityError, AuthorityTime, BindingDigest, ChallengeId,
    ConfirmationAuditEvent, ConfirmationAuditLog, ConfirmationAuthority, ConfirmationChallenge,
    ConfirmationChoice, ConfirmationGrant, ConfirmationNonce, ConfirmationResponse,
    ConfirmationResponseType, ConfirmationState, ConfirmationView, ConsequenceAcknowledgement,
    FinalValidationResult, GrantId, ReadinessDenialReason, ReadyForExecution, ResponderIdentity,
    ResponseValidationContext, SessionAuthorization, SessionStatus,
};
pub use coordinator::{
    ActionChoiceView, ActionConversationRecord, ActionCoordinator, ActionResponseView,
    BoundConfirmationResponse, CancelPendingAction, ClarificationResponse, CoordinatorActionId,
    CoordinatorAuditEntry, CoordinatorAuditEvent, CoordinatorAuditLog, CoordinatorConfig,
    CoordinatorContext, CoordinatorInput, CoordinatorResult, CoordinatorState,
    ObservedOutcomeInput, PublicActionStatus, PublicReasonCode, QueryPendingAction,
    RuntimeInvalidation, SessionEndedInput,
};
pub use delegated_session_lifecycle::{
    AcceptedLifecycleEvent, BraindTrustedLifecycleAdapters, ControlPanelLifecycleEventKind,
    DisplayLifecycleEventKind, LifecycleIngressError, LifecycleTargetKind, TrustedLaunchContext,
};
pub use diagnostics::BrainDiagnostics;
#[cfg(feature = "sunlightos")]
pub use executor::SunlightLaunchAdapter;
pub use executor::{
    ActionExecutor, DispatchStatus, ExecutionAuditEntry, ExecutionAuditEvent, ExecutionAuditLog,
    ExecutionContext, ExecutionId, ExecutionResult, ExecutionResultCode, LaunchApplicationRequest,
    LaunchCorrelationToken, OpenSettingsPageRequest, RegistryStatus, TrustedActionExecutor,
    TrustedActionFlow, TrustedLaunchAdapter,
};
#[cfg(feature = "gui-bridge-foundation-v1-test")]
pub use gui_bridge::run_deterministic_bridge_gate;
pub use gui_bridge::{
    CoordinatorPresentationKind, CoordinatorPresentationUpdate, CorrelatedGuiReadinessEvidence,
    GuiBindingError, GuiEventBroker, GuiEventError, GuiEventId, GuiReadinessEvidenceId,
    GuiSessionBindingAuthority, GuiSessionBindingId, PublicPresentationPayload,
    ReadinessCorrelationError, ReceiptSealedView, TrustedGuiReadinessKind,
    TrustedReadinessCorrelation, TrustedReadinessSource, VerifiedGraphicalSession, WiseOwlGuiEvent,
    WiseOwlGuiEventPayload, WiseOwlGuiSessionBinding, GUI_BRIDGE_PROTOCOL_VERSION,
    MAX_GUI_BRIDGE_DEDUP, MAX_GUI_BRIDGE_EVENTS,
};
#[cfg(all(feature = "sunlightos", feature = "gui-live-action-activation-v1-test"))]
pub use gui_live_action_activation::run_deterministic_live_action_gate;
pub use mtm::{BrainPreferences, GreetingStyle, WelcomeMemoryState};
pub use outcome::{
    AchievedReadiness, ActionOutcomeObserver, EvidenceId, EvidenceSummary, ObservationAuditEntry,
    ObservationAuditEvent, ObservationAuditLog, ObservationCreateError, ObservationDeadlines,
    ObservationEvidence, ObservationEvidenceKind, ObservationId, ObservationRequest,
    ObservationState, ObservedActionOutcome, ObservedActionOutcomeKind, OutcomeRegistry,
    PublicOutcomeCode, ReadinessContract, TrustedSourceKind,
};
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
#[cfg(feature = "trusted-session-readiness-v1-test")]
pub use trusted_session_readiness::run_deterministic_trust_gate;
pub use trusted_session_readiness::{
    AuthorityGeneration, GuiClientInstanceId, ProcessId, SessionAttestationError,
    SessionAttestationId, SessionGeneration, TrustedGraphicalSessionAuthority,
    TrustedLifecycleSource, TrustedReadinessError, TrustedReadinessIngress,
    TrustedReadinessSourceCapability, WiseOwlAuthorityConnections, WiseOwlLiveActionAvailability,
    TRUSTED_SESSION_PROTOCOL_VERSION,
};
