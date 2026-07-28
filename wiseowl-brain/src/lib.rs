#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", allow(static_mut_refs))]

extern crate alloc;

pub mod action_intent;
pub mod adapters;
pub mod caps;
pub mod context;
pub mod diagnostics;
pub mod error;
pub mod foundation;
pub mod greeting;
pub mod grounded;
pub mod kv_client;
pub mod memory_layers;
pub mod mtm;
pub mod native_ipc;
pub mod pipeline;
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
pub use diagnostics::BrainDiagnostics;
pub use mtm::{BrainPreferences, GreetingStyle, WelcomeMemoryState};
pub use pipeline::CognitivePipeline;
pub use policy::{
    ConfirmationLevel, PolicyCategory, PolicyEngine, PolicyOperation, PolicyReason, PolicyResult,
    PolicyRule, PolicyVersion,
};
pub use provider::{BrainProvider, FutureOnlineProvider, LocalBoundedProvider, ProviderRegistry};
pub use runtime_context::{
    ContextProvider, ContextProviderError, RefreshClass, RuntimeContextCache,
    RuntimeContextSnapshot,
};
