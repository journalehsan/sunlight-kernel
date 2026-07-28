//! Bounded, proposal-only conversation Action Planner v1.
//!
//! This module recognizes a deliberately small command grammar and resolves
//! targets through an injected authoritative registry. It has no executor,
//! launcher, policy shortcut, IPC transport, path, argument, environment, or
//! callback type.

use core::fmt::Write;

use heapless::{Deque, String, Vec};
use sha2::{Digest, Sha256};

use crate::action_intent::{
    ActionIntent, ActionOperation, ActionParameters, ActionTarget, CreationTime, IntentId,
    Provenance as IntentProvenance, RequestedBy, RiskHint, SessionId, TypedIdentifier,
};

pub const PLANNER_V1: PlannerVersion = PlannerVersion::new(1, 0);
pub const ALIAS_MODEL_V1: u16 = 1;
pub const MAX_USER_TEXT_LEN: usize = 256;
pub const MAX_LOCALE_LEN: usize = 16;
pub const MAX_PUBLIC_TEXT_LEN: usize = 160;
pub const MAX_CANDIDATES: usize = 8;
pub const MAX_CLARIFICATIONS: usize = 16;
pub const DEFAULT_PLANNER_AUDIT_CAPACITY: usize = 64;
pub const DEFAULT_CLARIFICATION_TTL_MS: u64 = 30_000;
pub const DEFAULT_MAX_INPUT_AGE_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannerVersion {
    pub major: u16,
    pub minor: u16,
}

impl PlannerVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlannerRequestId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConversationId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClarificationId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannerInputProvenance {
    DirectUserCommand,
    ClarificationResponse(ClarificationId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannerInputError {
    EmptyText,
    OversizedText,
    MalformedUtf8,
    OversizedLocale,
    MalformedLocale,
}

/// Immutable bounded input. Invalid untrusted bytes are represented only by a
/// public error code; oversized or malformed source text is never retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerInput {
    request_id: PlannerRequestId,
    conversation_id: ConversationId,
    session_id: SessionId,
    requester: RequestedBy,
    locale: String<MAX_LOCALE_LEN>,
    user_text: String<MAX_USER_TEXT_LEN>,
    runtime_snapshot_generation: u64,
    timestamp: u64,
    provenance: PlannerInputProvenance,
    input_error: Option<PlannerInputError>,
}

impl PlannerInput {
    #[allow(clippy::too_many_arguments)]
    pub fn from_untrusted(
        request_id: PlannerRequestId,
        conversation_id: ConversationId,
        session_id: SessionId,
        requester: RequestedBy,
        locale: &[u8],
        user_text: &[u8],
        runtime_snapshot_generation: u64,
        timestamp: u64,
        provenance: PlannerInputProvenance,
    ) -> Self {
        let mut input = Self {
            request_id,
            conversation_id,
            session_id,
            requester,
            locale: String::new(),
            user_text: String::new(),
            runtime_snapshot_generation,
            timestamp,
            provenance,
            input_error: None,
        };

        let locale = match core::str::from_utf8(locale) {
            Ok(value) if value.len() <= MAX_LOCALE_LEN => value,
            Ok(_) => {
                input.input_error = Some(PlannerInputError::OversizedLocale);
                return input;
            }
            Err(_) => {
                input.input_error = Some(PlannerInputError::MalformedLocale);
                return input;
            }
        };
        if locale.is_empty()
            || !locale
                .bytes()
                .all(|byte| byte.is_ascii_alphabetic() || byte == b'-')
        {
            input.input_error = Some(PlannerInputError::MalformedLocale);
            return input;
        }
        let _ = input.locale.push_str(locale);

        if user_text.is_empty() {
            input.input_error = Some(PlannerInputError::EmptyText);
            return input;
        }
        if user_text.len() > MAX_USER_TEXT_LEN {
            input.input_error = Some(PlannerInputError::OversizedText);
            return input;
        }
        let text = match core::str::from_utf8(user_text) {
            Ok(value) => value,
            Err(_) => {
                input.input_error = Some(PlannerInputError::MalformedUtf8);
                return input;
            }
        };
        if input.user_text.push_str(text).is_err() {
            input.input_error = Some(PlannerInputError::OversizedText);
        }
        input
    }

    pub fn direct(
        request_id: u64,
        conversation_id: u64,
        session_id: SessionId,
        requester: RequestedBy,
        locale: &str,
        user_text: &str,
        runtime_snapshot_generation: u64,
        timestamp: u64,
    ) -> Self {
        Self::from_untrusted(
            PlannerRequestId(request_id),
            ConversationId(conversation_id),
            session_id,
            requester,
            locale.as_bytes(),
            user_text.as_bytes(),
            runtime_snapshot_generation,
            timestamp,
            PlannerInputProvenance::DirectUserCommand,
        )
    }

    pub fn clarification_response(
        request_id: u64,
        conversation_id: u64,
        session_id: SessionId,
        requester: RequestedBy,
        locale: &str,
        user_text: &str,
        runtime_snapshot_generation: u64,
        timestamp: u64,
        clarification_id: ClarificationId,
    ) -> Self {
        let mut input = Self::direct(
            request_id,
            conversation_id,
            session_id,
            requester,
            locale,
            user_text,
            runtime_snapshot_generation,
            timestamp,
        );
        input.provenance = PlannerInputProvenance::ClarificationResponse(clarification_id);
        input
    }

    pub const fn request_id(&self) -> PlannerRequestId {
        self.request_id
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
    pub fn locale(&self) -> &str {
        self.locale.as_str()
    }
    pub fn user_text(&self) -> &str {
        self.user_text.as_str()
    }
    pub const fn runtime_snapshot_generation(&self) -> u64 {
        self.runtime_snapshot_generation
    }
    pub const fn timestamp(&self) -> u64 {
        self.timestamp
    }
    pub const fn provenance(&self) -> PlannerInputProvenance {
        self.provenance
    }
    pub const fn input_error(&self) -> Option<PlannerInputError> {
        self.input_error
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannerTargetKind {
    Application,
    SettingsPage,
}

#[derive(Debug, Clone, Copy)]
pub struct RegistryAliasRef<'a> {
    pub locale: &'a str,
    pub value: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct RegistryTargetRef<'a> {
    pub canonical_id: &'a str,
    pub display_name: &'a str,
    pub aliases: &'a [RegistryAliasRef<'a>],
}

/// Read-only, path-free view of the registries also used by typed execution.
pub trait PlannerRegistry {
    fn alias_model_version(&self) -> u16;
    fn visit_applications(&self, visitor: &mut dyn FnMut(RegistryTargetRef<'_>));
    fn visit_settings_pages(&self, visitor: &mut dyn FnMut(RegistryTargetRef<'_>));
}

#[cfg(feature = "sunlightos")]
pub struct SunlightPlannerRegistry;

#[cfg(feature = "sunlightos")]
impl PlannerRegistry for SunlightPlannerRegistry {
    fn alias_model_version(&self) -> u16 {
        sunlight_libc::sun_exec::ACTION_ALIAS_MODEL_VERSION
    }

    fn visit_applications(&self, visitor: &mut dyn FnMut(RegistryTargetRef<'_>)) {
        for entry in sunlight_libc::sun_exec::application_registry() {
            let mut aliases: Vec<RegistryAliasRef<'_>, 16> = Vec::new();
            for alias in entry.aliases() {
                let _ = aliases.push(RegistryAliasRef {
                    locale: alias.locale(),
                    value: alias.value(),
                });
            }
            visitor(RegistryTargetRef {
                canonical_id: entry.canonical_id(),
                display_name: entry.display_name(),
                aliases: aliases.as_slice(),
            });
        }
    }

    fn visit_settings_pages(&self, visitor: &mut dyn FnMut(RegistryTargetRef<'_>)) {
        for entry in sunlight_libc::sun_exec::settings_page_registry() {
            let mut aliases: Vec<RegistryAliasRef<'_>, 16> = Vec::new();
            for alias in entry.aliases() {
                let _ = aliases.push(RegistryAliasRef {
                    locale: alias.locale(),
                    value: alias.value(),
                });
            }
            visitor(RegistryTargetRef {
                canonical_id: entry.canonical_id(),
                display_name: entry.display_name(),
                aliases: aliases.as_slice(),
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfidenceClass {
    Exact,
    AliasExact,
    ClarifiedExact,
    Ambiguous,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftProvenance {
    ExactCanonicalName,
    ExactDisplayName,
    ExplicitRegisteredAlias,
    ClarificationResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceSpan {
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposedTarget {
    Application(TypedIdentifier),
    SettingsPage(TypedIdentifier),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizedParameters {
    ApplicationDefault,
    SettingsDefault,
}

/// Immutable proposal. It contains only typed registry IDs and cannot dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionIntentDraft {
    request_id: PlannerRequestId,
    operation: ActionOperation,
    target: ProposedTarget,
    parameters: NormalizedParameters,
    confidence: ConfidenceClass,
    evidence: Vec<EvidenceSpan, 2>,
    public_interpretation: String<MAX_PUBLIC_TEXT_LEN>,
    planner_version: PlannerVersion,
    alias_model_version: u16,
    runtime_snapshot_generation: u64,
    requester: RequestedBy,
    session_id: SessionId,
    timestamp: u64,
    provenance: DraftProvenance,
}

impl ActionIntentDraft {
    pub const fn request_id(&self) -> PlannerRequestId {
        self.request_id
    }
    pub const fn operation(&self) -> ActionOperation {
        self.operation
    }
    pub fn target(&self) -> &ProposedTarget {
        &self.target
    }
    pub const fn parameters(&self) -> NormalizedParameters {
        self.parameters
    }
    pub const fn confidence(&self) -> ConfidenceClass {
        self.confidence
    }
    pub fn evidence(&self) -> &[EvidenceSpan] {
        self.evidence.as_slice()
    }
    pub fn public_interpretation(&self) -> &str {
        self.public_interpretation.as_str()
    }
    pub const fn planner_version(&self) -> PlannerVersion {
        self.planner_version
    }
    pub const fn alias_model_version(&self) -> u16 {
        self.alias_model_version
    }
    pub const fn runtime_snapshot_generation(&self) -> u64 {
        self.runtime_snapshot_generation
    }
    pub const fn provenance(&self) -> DraftProvenance {
        self.provenance
    }

    /// Final construction still goes through the Action Intent type. The
    /// returned intent has no authorization and must enter TrustedActionFlow.
    pub fn build_action_intent(&self, intent_id: IntentId) -> ActionIntent {
        let (target, parameters) = match &self.target {
            ProposedTarget::Application(id) => (
                ActionTarget::Application(id.clone()),
                ActionParameters::Application {
                    new_instance: false,
                },
            ),
            ProposedTarget::SettingsPage(id) => (
                ActionTarget::SettingsPage(id.clone()),
                ActionParameters::Settings { focus: None },
            ),
        };
        ActionIntent::new(
            intent_id,
            self.operation,
            target,
            parameters,
            self.requester,
            self.session_id,
            self.runtime_snapshot_generation,
            CreationTime(self.timestamp),
            RiskHint::Low,
            IntentProvenance::ExplicitUserRequest,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClarificationCandidate {
    kind: PlannerTargetKind,
    canonical_id: TypedIdentifier,
    public_name: String<MAX_PUBLIC_TEXT_LEN>,
}

impl ClarificationCandidate {
    pub const fn kind(&self) -> PlannerTargetKind {
        self.kind
    }
    pub fn canonical_id(&self) -> &TypedIdentifier {
        &self.canonical_id
    }
    pub fn public_name(&self) -> &str {
        self.public_name.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClarificationRequest {
    request_id: PlannerRequestId,
    clarification_id: ClarificationId,
    public_question: String<MAX_PUBLIC_TEXT_LEN>,
    candidates: Vec<ClarificationCandidate, MAX_CANDIDATES>,
    expires_at: u64,
    session_id: SessionId,
    planner_version: PlannerVersion,
}

impl ClarificationRequest {
    pub const fn request_id(&self) -> PlannerRequestId {
        self.request_id
    }
    pub const fn clarification_id(&self) -> ClarificationId {
        self.clarification_id
    }
    pub fn public_question(&self) -> &str {
        self.public_question.as_str()
    }
    pub fn candidates(&self) -> &[ClarificationCandidate] {
        self.candidates.as_slice()
    }
    pub const fn expires_at(&self) -> u64 {
        self.expires_at
    }
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    pub const fn planner_version(&self) -> PlannerVersion {
        self.planner_version
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedReason {
    UnsupportedGrammar,
    UnsupportedOperation,
    UnknownApplication,
    UnknownSettingsPage,
    AliasCollision,
    MultipleActions,
    ClarificationExpired,
    ClarificationReplay,
    ClarificationNotFound,
    ClarificationTargetInvalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidInputReason {
    Input(PlannerInputError),
    StaleRuntimeSnapshot,
    WrongSession,
    InvalidTimestamp,
    RegistryVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannerResult {
    Proposed(ActionIntentDraft),
    NeedsClarification(ClarificationRequest),
    Unsupported(UnsupportedReason),
    NoAction,
    InvalidInput(InvalidInputReason),
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub struct PlannerContext {
    pub runtime_snapshot_generation: u64,
    pub active_session_id: SessionId,
    pub now: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannerAuditResult {
    InputAccepted,
    InputRejected(InvalidInputReason),
    GrammarMatched(ActionOperation),
    TargetResolved(ConfidenceClass),
    AmbiguityDetected,
    Unsupported(UnsupportedReason),
    DraftProposed,
    NoAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannerAuditEntry {
    pub request_id: PlannerRequestId,
    pub input_digest: [u8; 8],
    pub operation: Option<ActionOperation>,
    pub target_kind: Option<PlannerTargetKind>,
    pub result: PlannerAuditResult,
}

pub struct PlannerAuditLog<const N: usize = DEFAULT_PLANNER_AUDIT_CAPACITY> {
    entries: Deque<PlannerAuditEntry, N>,
    evicted: u64,
}

impl<const N: usize> PlannerAuditLog<N> {
    pub const fn new() -> Self {
        Self {
            entries: Deque::new(),
            evicted: 0,
        }
    }
    pub fn entries(&self) -> impl Iterator<Item = &PlannerAuditEntry> {
        self.entries.iter()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub const fn evicted(&self) -> u64 {
        self.evicted
    }
    fn record(&mut self, entry: PlannerAuditEntry) {
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

impl<const N: usize> Default for PlannerAuditLog<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
struct ClarificationState {
    id: ClarificationId,
    conversation_id: ConversationId,
    session_id: SessionId,
    operation: ActionOperation,
    candidates: Vec<ClarificationCandidate, MAX_CANDIDATES>,
    expires_at: u64,
    consumed: bool,
}

pub struct BoundedActionPlanner<R: PlannerRegistry, const N: usize = DEFAULT_PLANNER_AUDIT_CAPACITY>
{
    registry: R,
    audit: PlannerAuditLog<N>,
    clarifications: Vec<ClarificationState, MAX_CLARIFICATIONS>,
    next_clarification_id: u64,
    clarification_ttl_ms: u64,
}

impl<R: PlannerRegistry, const N: usize> BoundedActionPlanner<R, N> {
    pub const fn new(registry: R) -> Self {
        Self {
            registry,
            audit: PlannerAuditLog::new(),
            clarifications: Vec::new(),
            next_clarification_id: 1,
            clarification_ttl_ms: DEFAULT_CLARIFICATION_TTL_MS,
        }
    }

    pub const fn with_clarification_ttl(mut self, ttl_ms: u64) -> Self {
        self.clarification_ttl_ms = ttl_ms;
        self
    }

    pub const fn audit(&self) -> &PlannerAuditLog<N> {
        &self.audit
    }

    pub fn plan(&mut self, input: &PlannerInput, context: PlannerContext) -> PlannerResult {
        let digest = input_digest(input.user_text.as_bytes());
        if let Some(error) = input.input_error {
            return self.invalid(input, digest, InvalidInputReason::Input(error));
        }
        if input.runtime_snapshot_generation != context.runtime_snapshot_generation {
            return self.invalid(input, digest, InvalidInputReason::StaleRuntimeSnapshot);
        }
        if input.session_id != context.active_session_id {
            return self.invalid(input, digest, InvalidInputReason::WrongSession);
        }
        if input.timestamp > context.now
            || context.now.saturating_sub(input.timestamp) > DEFAULT_MAX_INPUT_AGE_MS
        {
            return self.invalid(input, digest, InvalidInputReason::InvalidTimestamp);
        }
        if self.registry.alias_model_version() != ALIAS_MODEL_V1 {
            return self.invalid(input, digest, InvalidInputReason::RegistryVersion);
        }
        self.record(input, digest, None, None, PlannerAuditResult::InputAccepted);

        if let PlannerInputProvenance::ClarificationResponse(id) = input.provenance {
            return self.plan_clarification(input, context, digest, id);
        }

        let normalized = match normalize(input.user_text()) {
            Some(value) if !value.is_empty() => value,
            _ => return self.no_action(input, digest),
        };
        let text = normalized.as_str();

        if is_quoted_or_embedded(text) || is_informational(text) || is_negated(text) {
            return self.no_action(input, digest);
        }

        let Some(grammar) = match_grammar(text, locale_language(input.locale())) else {
            let unsupported = unsupported_operation(text);
            return match unsupported {
                Some(reason) => self.unsupported(input, digest, reason),
                None => self.no_action(input, digest),
            };
        };
        self.record(
            input,
            digest,
            Some(grammar.operation),
            Some(grammar.kind),
            PlannerAuditResult::GrammarMatched(grammar.operation),
        );

        if grammar.generic_settings {
            let candidates = self.all_candidates(PlannerTargetKind::SettingsPage);
            return self.clarify(
                input,
                digest,
                grammar.operation,
                candidates,
                "Which settings page should I open?",
                context.now,
            );
        }

        let resolution = self.resolve(grammar.kind, grammar.target, input.locale());
        match resolution {
            Resolution::One(candidate, confidence, provenance) => {
                self.record(
                    input,
                    digest,
                    Some(grammar.operation),
                    Some(grammar.kind),
                    PlannerAuditResult::TargetResolved(confidence),
                );
                self.propose(
                    input,
                    digest,
                    grammar.operation,
                    grammar.kind,
                    candidate,
                    confidence,
                    provenance,
                )
            }
            Resolution::Many(candidates) => self.clarify(
                input,
                digest,
                grammar.operation,
                candidates,
                "That name matches more than one registered target. Which one?",
                context.now,
            ),
            Resolution::None if contains_multiple_joiner(grammar.target) => {
                self.unsupported(input, digest, UnsupportedReason::MultipleActions)
            }
            Resolution::None => self.unsupported(
                input,
                digest,
                match grammar.kind {
                    PlannerTargetKind::Application => UnsupportedReason::UnknownApplication,
                    PlannerTargetKind::SettingsPage => UnsupportedReason::UnknownSettingsPage,
                },
            ),
        }
    }

    fn plan_clarification(
        &mut self,
        input: &PlannerInput,
        context: PlannerContext,
        digest: [u8; 8],
        id: ClarificationId,
    ) -> PlannerResult {
        let Some(index) = self.clarifications.iter().position(|state| state.id == id) else {
            return self.unsupported(input, digest, UnsupportedReason::ClarificationNotFound);
        };
        let state = &self.clarifications[index];
        if state.consumed {
            return self.unsupported(input, digest, UnsupportedReason::ClarificationReplay);
        }
        if context.now > state.expires_at {
            return self.unsupported(input, digest, UnsupportedReason::ClarificationExpired);
        }
        if state.session_id != input.session_id || state.conversation_id != input.conversation_id {
            return self.invalid(input, digest, InvalidInputReason::WrongSession);
        }

        let Some(normalized) = normalize(input.user_text()) else {
            return self.unsupported(input, digest, UnsupportedReason::ClarificationTargetInvalid);
        };
        let mut matches: Vec<ClarificationCandidate, MAX_CANDIDATES> = Vec::new();
        for candidate in &state.candidates {
            let canonical = candidate.canonical_id.as_str();
            let public = normalize(candidate.public_name()).unwrap_or_default();
            if normalized.as_str() == canonical || normalized.as_str() == public.as_str() {
                let _ = matches.push(candidate.clone());
                continue;
            }
            let resolved = self.resolve(candidate.kind, normalized.as_str(), input.locale());
            if let Resolution::One(found, _, _) = resolved {
                if found.canonical_id == candidate.canonical_id {
                    let _ = matches.push(candidate.clone());
                }
            }
        }
        if matches.len() != 1 {
            return self.unsupported(input, digest, UnsupportedReason::ClarificationTargetInvalid);
        }
        let operation = state.operation;
        self.clarifications[index].consumed = true;
        self.propose(
            input,
            digest,
            operation,
            matches[0].kind,
            matches[0].clone(),
            ConfidenceClass::ClarifiedExact,
            DraftProvenance::ClarificationResponse,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn propose(
        &mut self,
        input: &PlannerInput,
        digest: [u8; 8],
        operation: ActionOperation,
        kind: PlannerTargetKind,
        candidate: ClarificationCandidate,
        confidence: ConfidenceClass,
        provenance: DraftProvenance,
    ) -> PlannerResult {
        if !matches!(
            confidence,
            ConfidenceClass::Exact | ConfidenceClass::AliasExact | ConfidenceClass::ClarifiedExact
        ) {
            return PlannerResult::Unknown;
        }
        let target = match kind {
            PlannerTargetKind::Application => {
                ProposedTarget::Application(candidate.canonical_id.clone())
            }
            PlannerTargetKind::SettingsPage => {
                ProposedTarget::SettingsPage(candidate.canonical_id.clone())
            }
        };
        let parameters = match kind {
            PlannerTargetKind::Application => NormalizedParameters::ApplicationDefault,
            PlannerTargetKind::SettingsPage => NormalizedParameters::SettingsDefault,
        };
        let mut interpretation = String::new();
        let _ = write!(
            &mut interpretation,
            "{} {}",
            match operation {
                ActionOperation::OpenApplication => "Open application",
                ActionOperation::OpenSettingsPage => "Open settings page",
                _ => "Unsupported",
            },
            candidate.public_name()
        );
        let mut evidence = Vec::new();
        let _ = evidence.push(EvidenceSpan {
            start: 0,
            end: input.user_text.len().min(u16::MAX as usize) as u16,
        });
        let draft = ActionIntentDraft {
            request_id: input.request_id,
            operation,
            target,
            parameters,
            confidence,
            evidence,
            public_interpretation: interpretation,
            planner_version: PLANNER_V1,
            alias_model_version: ALIAS_MODEL_V1,
            runtime_snapshot_generation: input.runtime_snapshot_generation,
            requester: input.requester,
            session_id: input.session_id,
            timestamp: input.timestamp,
            provenance,
        };
        self.record(
            input,
            digest,
            Some(operation),
            Some(kind),
            PlannerAuditResult::DraftProposed,
        );
        PlannerResult::Proposed(draft)
    }

    fn clarify(
        &mut self,
        input: &PlannerInput,
        digest: [u8; 8],
        operation: ActionOperation,
        candidates: Vec<ClarificationCandidate, MAX_CANDIDATES>,
        question: &str,
        now: u64,
    ) -> PlannerResult {
        if candidates.is_empty() {
            return self.unsupported(input, digest, UnsupportedReason::AliasCollision);
        }
        let id = ClarificationId(self.next_clarification_id);
        self.next_clarification_id = self.next_clarification_id.saturating_add(1);
        let expires_at = now.saturating_add(self.clarification_ttl_ms);
        if self.clarifications.is_full() {
            self.clarifications.remove(0);
        }
        let _ = self.clarifications.push(ClarificationState {
            id,
            conversation_id: input.conversation_id,
            session_id: input.session_id,
            operation,
            candidates: candidates.clone(),
            expires_at,
            consumed: false,
        });
        let mut public_question = String::new();
        let _ = public_question.push_str(question);
        self.record(
            input,
            digest,
            Some(operation),
            None,
            PlannerAuditResult::AmbiguityDetected,
        );
        PlannerResult::NeedsClarification(ClarificationRequest {
            request_id: input.request_id,
            clarification_id: id,
            public_question,
            candidates,
            expires_at,
            session_id: input.session_id,
            planner_version: PLANNER_V1,
        })
    }

    fn all_candidates(
        &self,
        kind: PlannerTargetKind,
    ) -> Vec<ClarificationCandidate, MAX_CANDIDATES> {
        let mut candidates = Vec::new();
        self.visit(kind, &mut |entry| {
            if candidates.len() < MAX_CANDIDATES {
                if let Some(candidate) = candidate_from(entry, kind) {
                    let _ = candidates.push(candidate);
                }
            }
        });
        candidates
    }

    fn resolve(&self, kind: PlannerTargetKind, target: &str, locale: &str) -> Resolution {
        let Some(target) = normalize(target) else {
            return Resolution::None;
        };
        let language = locale_language(locale);
        let mut matches: Vec<
            (ClarificationCandidate, ConfidenceClass, DraftProvenance),
            MAX_CANDIDATES,
        > = Vec::new();
        self.visit(kind, &mut |entry| {
            let canonical = normalize(entry.canonical_id).unwrap_or_default();
            let display = normalize(entry.display_name).unwrap_or_default();
            let direct = if target == canonical {
                Some((ConfidenceClass::Exact, DraftProvenance::ExactCanonicalName))
            } else if target == display {
                Some((ConfidenceClass::Exact, DraftProvenance::ExactDisplayName))
            } else {
                entry
                    .aliases
                    .iter()
                    .filter(|alias| locale_language(alias.locale) == language)
                    .find_map(|alias| {
                        (normalize(alias.value).as_ref() == Some(&target)).then_some((
                            ConfidenceClass::AliasExact,
                            DraftProvenance::ExplicitRegisteredAlias,
                        ))
                    })
            };
            if let Some((confidence, provenance)) = direct {
                if matches
                    .iter()
                    .any(|(candidate, _, _)| candidate.canonical_id.as_str() == entry.canonical_id)
                {
                    return;
                }
                if let Some(candidate) = candidate_from(entry, kind) {
                    let _ = matches.push((candidate, confidence, provenance));
                }
            }
        });
        match matches.len() {
            0 => Resolution::None,
            1 => {
                let (candidate, confidence, provenance) = matches.remove(0);
                Resolution::One(candidate, confidence, provenance)
            }
            _ => {
                let mut candidates = Vec::new();
                for (candidate, _, _) in matches {
                    let _ = candidates.push(candidate);
                }
                Resolution::Many(candidates)
            }
        }
    }

    fn visit(&self, kind: PlannerTargetKind, visitor: &mut dyn FnMut(RegistryTargetRef<'_>)) {
        match kind {
            PlannerTargetKind::Application => self.registry.visit_applications(visitor),
            PlannerTargetKind::SettingsPage => self.registry.visit_settings_pages(visitor),
        }
    }

    fn invalid(
        &mut self,
        input: &PlannerInput,
        digest: [u8; 8],
        reason: InvalidInputReason,
    ) -> PlannerResult {
        self.record(
            input,
            digest,
            None,
            None,
            PlannerAuditResult::InputRejected(reason),
        );
        PlannerResult::InvalidInput(reason)
    }

    fn unsupported(
        &mut self,
        input: &PlannerInput,
        digest: [u8; 8],
        reason: UnsupportedReason,
    ) -> PlannerResult {
        self.record(
            input,
            digest,
            None,
            None,
            PlannerAuditResult::Unsupported(reason),
        );
        PlannerResult::Unsupported(reason)
    }

    fn no_action(&mut self, input: &PlannerInput, digest: [u8; 8]) -> PlannerResult {
        self.record(input, digest, None, None, PlannerAuditResult::NoAction);
        PlannerResult::NoAction
    }

    fn record(
        &mut self,
        input: &PlannerInput,
        digest: [u8; 8],
        operation: Option<ActionOperation>,
        target_kind: Option<PlannerTargetKind>,
        result: PlannerAuditResult,
    ) {
        self.audit.record(PlannerAuditEntry {
            request_id: input.request_id,
            input_digest: digest,
            operation,
            target_kind,
            result,
        });
    }
}

#[derive(Debug)]
enum Resolution {
    One(ClarificationCandidate, ConfidenceClass, DraftProvenance),
    Many(Vec<ClarificationCandidate, MAX_CANDIDATES>),
    None,
}

struct GrammarMatch<'a> {
    operation: ActionOperation,
    kind: PlannerTargetKind,
    target: &'a str,
    generic_settings: bool,
}

fn match_grammar<'a>(text: &'a str, language: &str) -> Option<GrammarMatch<'a>> {
    if language == "fa" {
        let text = text.strip_prefix("لطفا ").unwrap_or(text);
        let target = text.strip_suffix(" را باز کن")?;
        if target == "تنظیمات" {
            return Some(GrammarMatch {
                operation: ActionOperation::OpenSettingsPage,
                kind: PlannerTargetKind::SettingsPage,
                target: "",
                generic_settings: true,
            });
        }
        if let Some(settings) = target.strip_prefix("تنظیمات ") {
            return Some(GrammarMatch {
                operation: ActionOperation::OpenSettingsPage,
                kind: PlannerTargetKind::SettingsPage,
                target: settings,
                generic_settings: false,
            });
        }
        return Some(GrammarMatch {
            operation: ActionOperation::OpenApplication,
            kind: PlannerTargetKind::Application,
            target,
            generic_settings: false,
        });
    }

    let text = text.strip_prefix("please ").unwrap_or(text);
    if let Some(page) = text
        .strip_prefix("show ")
        .and_then(|target| target.strip_suffix(" settings"))
    {
        return Some(GrammarMatch {
            operation: ActionOperation::OpenSettingsPage,
            kind: PlannerTargetKind::SettingsPage,
            target: page,
            generic_settings: false,
        });
    }
    let target = ["open ", "launch ", "start "]
        .iter()
        .find_map(|prefix| text.strip_prefix(prefix))?;
    if target == "settings" {
        return Some(GrammarMatch {
            operation: ActionOperation::OpenSettingsPage,
            kind: PlannerTargetKind::SettingsPage,
            target: "",
            generic_settings: true,
        });
    }
    if let Some(page) = target.strip_suffix(" settings") {
        return Some(GrammarMatch {
            operation: ActionOperation::OpenSettingsPage,
            kind: PlannerTargetKind::SettingsPage,
            target: page,
            generic_settings: false,
        });
    }
    Some(GrammarMatch {
        operation: ActionOperation::OpenApplication,
        kind: PlannerTargetKind::Application,
        target,
        generic_settings: false,
    })
}

fn candidate_from(
    entry: RegistryTargetRef<'_>,
    kind: PlannerTargetKind,
) -> Option<ClarificationCandidate> {
    let canonical_id = TypedIdentifier::new(entry.canonical_id).ok()?;
    let mut public_name = String::new();
    public_name.push_str(entry.display_name).ok()?;
    Some(ClarificationCandidate {
        kind,
        canonical_id,
        public_name,
    })
}

fn normalize(value: &str) -> Option<String<MAX_USER_TEXT_LEN>> {
    let mut out = String::new();
    let value = value.trim();
    for ch in value.chars() {
        if ch.is_control() {
            return None;
        }
        let ch = match ch {
            // Conservative Arabic/Persian presentation normalization.
            '\u{064A}' | '\u{0649}' => '\u{06CC}',
            '\u{0643}' => '\u{06A9}',
            _ => ch,
        };
        if ch.is_ascii_uppercase() {
            out.push(ch.to_ascii_lowercase()).ok()?;
        } else {
            out.push(ch).ok()?;
        }
    }
    while matches!(out.chars().last(), Some('.' | '!' | '?' | '؟' | '؛' | ',')) {
        out.pop();
    }
    while out.ends_with(' ') {
        out.pop();
    }
    Some(out)
}

fn locale_language(locale: &str) -> &str {
    locale.split('-').next().unwrap_or(locale)
}

fn is_informational(text: &str) -> bool {
    [
        "what ",
        "what's ",
        "why ",
        "how ",
        "is ",
        "are ",
        "does ",
        "do you ",
        "آیا ",
        "چرا ",
        "چیست ",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
        || text.ends_with(" چیست")
}

fn is_negated(text: &str) -> bool {
    text.starts_with("do not ")
        || text.starts_with("don't ")
        || text.starts_with("never ")
        || text.contains(" باز نکن")
}

fn is_quoted_or_embedded(text: &str) -> bool {
    text.contains("```")
        || text.contains('`')
        || text.contains('"')
        || text.contains('\'')
        || text.contains('“')
        || text.contains('”')
        || text.contains('«')
        || text.contains('»')
        || text.starts_with("the message says ")
        || text.starts_with("the log says ")
}

fn contains_multiple_joiner(target: &str) -> bool {
    target.contains(" and ") || target.contains(" و ")
}

fn unsupported_operation(text: &str) -> Option<UnsupportedReason> {
    let dangerous_or_out_of_scope = [
        "restart ", "stop ", "delete ", "remove ", "install ", "format ", "erase ", "modify ",
        "run ", "execute ", "shell ",
    ];
    dangerous_or_out_of_scope
        .iter()
        .any(|prefix| text.starts_with(prefix))
        .then_some(UnsupportedReason::UnsupportedOperation)
}

fn input_digest(bytes: &[u8]) -> [u8; 8] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::PolicyEffect;
    use crate::{
        ActionEvaluation, ActionIntentEvaluator, ConfirmationLevel, PolicyCategory, PolicyEngine,
        PolicyOperation, PolicyResult, PolicyRule, PolicyVersion, RuntimeContextSnapshot,
    };

    const CALC_ALIASES: &[RegistryAliasRef<'static>] = &[
        RegistryAliasRef {
            locale: "en",
            value: "calc",
        },
        RegistryAliasRef {
            locale: "fa",
            value: "ماشین حساب",
        },
    ];
    const FILE_ALIASES: &[RegistryAliasRef<'static>] = &[RegistryAliasRef {
        locale: "fa",
        value: "پرونده‌ها",
    }];
    const NETWORK_ALIASES: &[RegistryAliasRef<'static>] = &[RegistryAliasRef {
        locale: "fa",
        value: "شبکه",
    }];
    const DISPLAY_ALIASES: &[RegistryAliasRef<'static>] = &[
        RegistryAliasRef {
            locale: "en",
            value: "monitor",
        },
        RegistryAliasRef {
            locale: "fa",
            value: "نمایشگر",
        },
    ];
    const EMPTY_ALIASES: &[RegistryAliasRef<'static>] = &[];

    #[derive(Clone, Copy)]
    struct TestRegistry;

    impl PlannerRegistry for TestRegistry {
        fn alias_model_version(&self) -> u16 {
            ALIAS_MODEL_V1
        }

        fn visit_applications(&self, visitor: &mut dyn FnMut(RegistryTargetRef<'_>)) {
            for target in [
                RegistryTargetRef {
                    canonical_id: "calculator",
                    display_name: "Calculator",
                    aliases: CALC_ALIASES,
                },
                RegistryTargetRef {
                    canonical_id: "files",
                    display_name: "Files",
                    aliases: FILE_ALIASES,
                },
            ] {
                visitor(target);
            }
        }

        fn visit_settings_pages(&self, visitor: &mut dyn FnMut(RegistryTargetRef<'_>)) {
            for target in [
                RegistryTargetRef {
                    canonical_id: "network",
                    display_name: "Network",
                    aliases: NETWORK_ALIASES,
                },
                RegistryTargetRef {
                    canonical_id: "display",
                    display_name: "Display",
                    aliases: DISPLAY_ALIASES,
                },
                RegistryTargetRef {
                    canonical_id: "wallpaper",
                    display_name: "Wallpaper",
                    aliases: EMPTY_ALIASES,
                },
            ] {
                visitor(target);
            }
        }
    }

    #[derive(Clone, Copy)]
    struct CollisionRegistry;

    impl PlannerRegistry for CollisionRegistry {
        fn alias_model_version(&self) -> u16 {
            ALIAS_MODEL_V1
        }
        fn visit_applications(&self, visitor: &mut dyn FnMut(RegistryTargetRef<'_>)) {
            let collision = &[RegistryAliasRef {
                locale: "en",
                value: "desk",
            }];
            visitor(RegistryTargetRef {
                canonical_id: "calculator",
                display_name: "Calculator",
                aliases: collision,
            });
            visitor(RegistryTargetRef {
                canonical_id: "files",
                display_name: "Files",
                aliases: collision,
            });
        }
        fn visit_settings_pages(&self, _: &mut dyn FnMut(RegistryTargetRef<'_>)) {}
    }

    fn input(text: &str, locale: &str) -> PlannerInput {
        PlannerInput::direct(
            14,
            3,
            SessionId(7),
            RequestedBy::User(42),
            locale,
            text,
            9,
            100,
        )
    }

    fn context() -> PlannerContext {
        PlannerContext {
            runtime_snapshot_generation: 9,
            active_session_id: SessionId(7),
            now: 100,
        }
    }

    fn proposed(text: &str, locale: &str) -> ActionIntentDraft {
        let mut planner = BoundedActionPlanner::<TestRegistry, 16>::new(TestRegistry);
        match planner.plan(&input(text, locale), context()) {
            PlannerResult::Proposed(draft) => draft,
            other => panic!("expected proposal, got {other:?}"),
        }
    }

    #[test]
    fn exact_and_registered_alias_application_resolution() {
        let exact = proposed("Open Calculator", "en-US");
        assert_eq!(exact.confidence(), ConfidenceClass::Exact);
        assert!(matches!(
            exact.target(),
            ProposedTarget::Application(id) if id.as_str() == "calculator"
        ));

        let alias = proposed("please launch calc!", "en");
        assert_eq!(alias.confidence(), ConfidenceClass::AliasExact);
        assert_eq!(alias.provenance(), DraftProvenance::ExplicitRegisteredAlias);
    }

    #[test]
    fn exact_settings_and_show_display_are_typed() {
        let network = proposed("Open network settings", "en");
        assert_eq!(network.operation(), ActionOperation::OpenSettingsPage);
        assert!(matches!(
            network.target(),
            ProposedTarget::SettingsPage(id) if id.as_str() == "network"
        ));
        let display = proposed("Show display settings", "en");
        assert!(matches!(
            display.target(),
            ProposedTarget::SettingsPage(id) if id.as_str() == "display"
        ));
    }

    #[test]
    fn persian_application_settings_and_aliases_are_supported() {
        let application = proposed("ماشین حساب را باز کن", "fa-IR");
        assert!(matches!(
            application.target(),
            ProposedTarget::Application(id) if id.as_str() == "calculator"
        ));
        let settings = proposed("تنظیمات شبکه را باز کن", "fa");
        assert!(matches!(
            settings.target(),
            ProposedTarget::SettingsPage(id) if id.as_str() == "network"
        ));
    }

    #[test]
    fn unknown_targets_fail_closed() {
        let mut planner = BoundedActionPlanner::<TestRegistry>::new(TestRegistry);
        assert_eq!(
            planner.plan(&input("Open Firefox", "en"), context()),
            PlannerResult::Unsupported(UnsupportedReason::UnknownApplication)
        );
        assert_eq!(
            planner.plan(&input("Open bluetooth settings", "en"), context()),
            PlannerResult::Unsupported(UnsupportedReason::UnknownSettingsPage)
        );
    }

    #[test]
    fn collision_and_generic_settings_require_clarification() {
        let mut collision = BoundedActionPlanner::<CollisionRegistry>::new(CollisionRegistry);
        let PlannerResult::NeedsClarification(request) =
            collision.plan(&input("Open desk", "en"), context())
        else {
            panic!("collision silently resolved");
        };
        assert_eq!(request.candidates().len(), 2);

        let mut planner = BoundedActionPlanner::<TestRegistry>::new(TestRegistry);
        let PlannerResult::NeedsClarification(request) =
            planner.plan(&input("Open settings", "en"), context())
        else {
            panic!("generic settings silently resolved");
        };
        assert_eq!(request.candidates().len(), 3);
    }

    #[test]
    fn multiple_actions_are_never_split_or_selected() {
        let mut planner = BoundedActionPlanner::<TestRegistry>::new(TestRegistry);
        assert_eq!(
            planner.plan(&input("Open Calculator and Files", "en"), context()),
            PlannerResult::Unsupported(UnsupportedReason::MultipleActions)
        );
    }

    #[test]
    fn destructive_service_and_package_requests_are_unsupported() {
        let mut planner = BoundedActionPlanner::<TestRegistry>::new(TestRegistry);
        for text in [
            "Restart networkd",
            "Delete this file",
            "Install Firefox",
            "Format the disk",
        ] {
            assert_eq!(
                planner.plan(&input(text, "en"), context()),
                PlannerResult::Unsupported(UnsupportedReason::UnsupportedOperation)
            );
        }
    }

    #[test]
    fn informational_negated_quoted_and_code_requests_are_no_action() {
        let mut planner = BoundedActionPlanner::<TestRegistry>::new(TestRegistry);
        for (text, locale) in [
            ("What is Calculator?", "en"),
            ("Is the network connected?", "en"),
            ("Why is the display resolution low?", "en"),
            ("Do not open Calculator", "en"),
            ("ماشین حساب را باز نکن", "fa"),
            ("The message says 'open calculator'", "en"),
            ("```open calculator```", "en"),
        ] {
            assert_eq!(
                planner.plan(&input(text, locale), context()),
                PlannerResult::NoAction,
                "{text}"
            );
        }
    }

    #[test]
    fn oversized_and_malformed_utf8_are_invalid_without_retention() {
        let oversized = [b'a'; MAX_USER_TEXT_LEN + 1];
        let malformed = [0xff, 0xfe];
        for (bytes, expected) in [
            (
                oversized.as_slice(),
                InvalidInputReason::Input(PlannerInputError::OversizedText),
            ),
            (
                malformed.as_slice(),
                InvalidInputReason::Input(PlannerInputError::MalformedUtf8),
            ),
        ] {
            let candidate = PlannerInput::from_untrusted(
                PlannerRequestId(1),
                ConversationId(1),
                SessionId(7),
                RequestedBy::User(42),
                b"en",
                bytes,
                9,
                100,
                PlannerInputProvenance::DirectUserCommand,
            );
            assert!(candidate.user_text().is_empty());
            let mut planner = BoundedActionPlanner::<TestRegistry>::new(TestRegistry);
            assert_eq!(
                planner.plan(&candidate, context()),
                PlannerResult::InvalidInput(expected)
            );
        }
    }

    #[test]
    fn stale_snapshot_and_wrong_session_are_invalid() {
        let mut planner = BoundedActionPlanner::<TestRegistry>::new(TestRegistry);
        let mut stale = context();
        stale.runtime_snapshot_generation = 10;
        assert_eq!(
            planner.plan(&input("Open Calculator", "en"), stale),
            PlannerResult::InvalidInput(InvalidInputReason::StaleRuntimeSnapshot)
        );
        let mut wrong = context();
        wrong.active_session_id = SessionId(8);
        assert_eq!(
            planner.plan(&input("Open Calculator", "en"), wrong),
            PlannerResult::InvalidInput(InvalidInputReason::WrongSession)
        );
        let mut future = context();
        future.now = 99;
        assert_eq!(
            planner.plan(&input("Open Calculator", "en"), future),
            PlannerResult::InvalidInput(InvalidInputReason::InvalidTimestamp)
        );
    }

    #[test]
    fn clarification_success_expiry_replay_and_binding_are_enforced() {
        let mut planner =
            BoundedActionPlanner::<TestRegistry>::new(TestRegistry).with_clarification_ttl(10);
        let PlannerResult::NeedsClarification(question) =
            planner.plan(&input("Open settings", "en"), context())
        else {
            panic!("expected clarification");
        };
        let response = PlannerInput::clarification_response(
            15,
            3,
            SessionId(7),
            RequestedBy::User(42),
            "en",
            "network",
            9,
            105,
            question.clarification_id(),
        );
        let PlannerResult::Proposed(draft) = planner.plan(
            &response,
            PlannerContext {
                now: 105,
                ..context()
            },
        ) else {
            panic!("clarification did not produce a fresh draft");
        };
        assert_eq!(draft.confidence(), ConfidenceClass::ClarifiedExact);
        assert_eq!(
            planner.plan(
                &response,
                PlannerContext {
                    now: 106,
                    ..context()
                }
            ),
            PlannerResult::Unsupported(UnsupportedReason::ClarificationReplay)
        );

        let PlannerResult::NeedsClarification(expiring) =
            planner.plan(&input("Open settings", "en"), context())
        else {
            panic!("expected clarification");
        };
        let expired = PlannerInput::clarification_response(
            16,
            3,
            SessionId(7),
            RequestedBy::User(42),
            "en",
            "network",
            9,
            111,
            expiring.clarification_id(),
        );
        assert_eq!(
            planner.plan(
                &expired,
                PlannerContext {
                    now: 111,
                    ..context()
                }
            ),
            PlannerResult::Unsupported(UnsupportedReason::ClarificationExpired)
        );
    }

    #[test]
    fn draft_builds_only_a_still_untrusted_action_intent() {
        let draft = proposed("Open Calculator", "en");
        let intent = draft.build_action_intent(IntentId::new([7; 16]));
        assert_eq!(intent.operation(), ActionOperation::OpenApplication);
        assert!(matches!(
            intent.target(),
            ActionTarget::Application(id) if id.as_str() == "calculator"
        ));
        assert!(matches!(
            intent.parameters(),
            ActionParameters::Application {
                new_instance: false
            }
        ));
    }

    #[test]
    fn planner_output_still_requires_validation_and_policy() {
        let draft = proposed("Open Calculator", "en");
        let intent = draft.build_action_intent(IntentId::new([8; 16]));
        let runtime = RuntimeContextSnapshot {
            generation: 9,
            ..RuntimeContextSnapshot::default()
        };
        let result =
            ActionIntentEvaluator::<8>::new(PolicyEngine::v1()).evaluate(&intent, &runtime);
        assert!(matches!(
            result,
            ActionEvaluation::Decided(decision) if decision.result() == PolicyResult::Allowed
        ));
    }

    #[test]
    fn policy_can_still_demand_confirmation_for_a_planner_proposal() {
        static RULES: &[PolicyRule] = &[PolicyRule::new(
            PolicyOperation::OpenApplication,
            PolicyCategory::Execute,
            PolicyEffect::Confirm(ConfirmationLevel::Soft),
        )];
        let draft = proposed("Open Calculator", "en");
        let intent = draft.build_action_intent(IntentId::new([9; 16]));
        let runtime = RuntimeContextSnapshot {
            generation: 9,
            ..RuntimeContextSnapshot::default()
        };
        let policy = PolicyEngine::from_static_rules(PolicyVersion::new(99, 1), RULES);
        let result = ActionIntentEvaluator::<8>::new(policy).evaluate(&intent, &runtime);
        assert!(matches!(
            result,
            ActionEvaluation::Decided(decision)
                if decision.result() == PolicyResult::ConfirmationRequired
                    && decision.confirmation_level() == ConfirmationLevel::Soft
        ));
    }

    #[test]
    fn audit_is_bounded_and_contains_no_source_text() {
        let mut planner = BoundedActionPlanner::<TestRegistry, 2>::new(TestRegistry);
        for _ in 0..3 {
            let _ = planner.plan(&input("Open Calculator", "en"), context());
        }
        assert_eq!(planner.audit().len(), 2);
        assert!(planner.audit().evicted() > 0);
        for entry in planner.audit().entries() {
            let rendered = std::format!("{entry:?}");
            assert!(!rendered.to_ascii_lowercase().contains("calculator"));
            assert!(!rendered.contains("Open "));
        }
    }

    #[test]
    fn proposal_surface_has_no_execution_payload_types() {
        let names = [
            core::any::type_name::<PlannerInput>(),
            core::any::type_name::<ActionIntentDraft>(),
            core::any::type_name::<PlannerResult>(),
        ];
        for name in names {
            assert!(!name.contains("Path"));
            assert!(!name.contains("Command"));
            assert!(!name.contains("Executor"));
            assert!(!name.contains("ReadyForExecution"));
        }
    }
}
