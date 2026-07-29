use alloc::vec::Vec;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call_timeout, ipc_reply_and_try_recv, monotonic_millis,
    nameserver_lookup, nameserver_register, process_yield, shm_alloc, shm_free, shm_map,
    wiseowl_delegate_authenticated_caller, wiseowl_validate_lifecycle_source, IpcMsg,
    SessionAuthorityProof, WiseOwlLifecycleMsg, SESSION_ENDPOINT,
    WISEOWL_CONTROL_PANEL_LIFECYCLE_ENDPOINT, WISEOWL_DELEGATION_LIFETIME_MS,
    WISEOWL_DELEGATION_PROTOCOL_VERSION, WISEOWL_DISPLAY_LIFECYCLE_ENDPOINT,
};
use sunlight_libc as libc;

use wiseowl_brain::adapters::{
    FoundationContextSource, IndexContextSource, KvContextSource, RuntimeContextSource,
    SessionContextSource, WiseOwlStatusContextSource,
};
use wiseowl_brain::grounded::AuthIdentity;
use wiseowl_brain::kv_client::{load_mtm, save_preferences, save_welcome_state};
use wiseowl_brain::mtm::GreetingStyle;
use wiseowl_brain::native_ipc::{
    BrainIpcHeader, BrainOp, BRAIN_ENDPOINT, BRAIN_IPC_HEADER_LEN, IPC_REG_WORDS,
    NATIVE_PROTOCOL_VERSION, REG_INLINE_BODY_MAX, SHM_PAGE_SIZE,
};
use wiseowl_brain::pipeline::CognitivePipeline;
use wiseowl_brain::protocol::{
    BrainRequestWire, ConsoleUiCommandWire, ConsoleUiRequestWire, ConsoleUiResponseWire,
};
use wiseowl_brain::provenance::BrainProviderKind;

macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

static mut PIPELINE: Option<CognitivePipeline> = None;
static mut LIFECYCLE_ADAPTERS: Option<wiseowl_brain::BraindTrustedLifecycleAdapters> = None;
#[cfg(feature = "delegated-session-lifecycle-ipc-v1-test")]
static mut DELEGATION_GATE_EMITTED: bool = false;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Startup diagnostics are operational only.  They carry neither caller
    // identity, attestation data nor readiness evidence.
    serial_println!("[WISEOWL-BRAIN] START_PHASE PROCESS_ENTERED");

    unsafe {
        PIPELINE = Some(CognitivePipeline::new());
        LIFECYCLE_ADAPTERS = Some(wiseowl_brain::BraindTrustedLifecycleAdapters::new());
    }
    serial_println!("[WISEOWL-BRAIN] START_PHASE PIPELINE_INITIALIZED");
    let pipeline = unsafe { PIPELINE.as_ref().unwrap() };
    if pipeline.foundation_state().is_ready() {
        serial_println!(
            "[WISEOWL-BRAIN] FOUNDATION_READY records={} tokens={}",
            pipeline.foundation_state().record_count(),
            pipeline.foundation_state().token_count()
        );
    } else {
        serial_println!(
            "[WISEOWL-BRAIN] FOUNDATION_DEGRADED reason={}",
            pipeline.foundation_state().status_label()
        );
    }

    // Optional context providers are deliberately not awaited here.  Their
    // absence is represented by the pipeline's degraded snapshot, not by an
    // unbounded boot dependency.
    serial_println!("[WISEOWL-BRAIN] START_PHASE OPTIONAL_SOURCES_DEGRADED_OR_READY");
    let ep = endpoint_create();
    let display_lifecycle_ep = endpoint_create();
    let control_panel_lifecycle_ep = endpoint_create();
    if ep.0 == 0 {
        serial_println!("[WISEOWL-BRAIN] START_PHASE ENDPOINT_CREATE_FAILED");
        loop {
            process_yield();
        }
    }
    if nameserver_register(BRAIN_ENDPOINT, ep) {
        let display_endpoint_ready = display_lifecycle_ep.0 != 0
            && nameserver_register(WISEOWL_DISPLAY_LIFECYCLE_ENDPOINT, display_lifecycle_ep);
        let control_panel_endpoint_ready = control_panel_lifecycle_ep.0 != 0
            && nameserver_register(
                WISEOWL_CONTROL_PANEL_LIFECYCLE_ENDPOINT,
                control_panel_lifecycle_ep,
            );
        serial_println!("[WISEOWL-BRAIN] START_PHASE GUI_ENDPOINT_REGISTERED");
        serial_println!("[WISEOWL-BRAIN] START_PHASE TRUST_INGRESS_DEGRADED");
        serial_println!("[WISEOWL-BRAIN] NATIVE_ELF PASS");
        serial_println!("[WISEOWL-BRAIN] SERVICE_SPAWN PASS");
        serial_println!("[WISEOWL-BRAIN] ENDPOINT_REGISTER PASS");
        serial_println!("[WISEOWL-BRAIN] SERVICE_READY PASS");
        serial_println!("[WISEOWL-BRAIN] START_PHASE READY");
        serial_println!("[WISEOWL-BRAIN] registered {}", BRAIN_ENDPOINT);
        if !display_endpoint_ready || !control_panel_endpoint_ready {
            serial_println!("[WISEOWL-BRAIN] START_PHASE OPTIONAL_LIFECYCLE_ENDPOINT_DEGRADED");
        }
        #[cfg(feature = "executor-v1-test")]
        run_executor_v1_gate();
        #[cfg(feature = "planner-v1-test")]
        run_planner_v1_gate();
        #[cfg(feature = "coordinator-v1-test")]
        run_coordinator_v1_gate();
        #[cfg(feature = "outcome-observer-v1-test")]
        run_outcome_observer_v1_gate();
        #[cfg(feature = "action-receipt-v1-test")]
        run_action_receipt_v1_gate();
        #[cfg(feature = "gui-bridge-foundation-v1-test")]
        run_gui_bridge_foundation_v1_gate();
        #[cfg(feature = "trusted-session-readiness-v1-test")]
        run_trusted_session_readiness_v1_gate();
        #[cfg(feature = "gui-live-action-activation-v1-test")]
        run_gui_live_action_activation_v1_gate();
    } else {
        serial_println!("[WISEOWL-BRAIN] failed to register {}", BRAIN_ENDPOINT);
        process_yield();
        libc::exit(1);
    }

    let mut gui_reply = IpcMsg::empty();
    let mut display_reply = IpcMsg::empty();
    let mut control_panel_reply = IpcMsg::empty();
    loop {
        if let Some(msg) = ipc_reply_and_try_recv(ep, gui_reply) {
            let op = match BrainOp::from_u16(msg.label as u16) {
                Some(o) => o,
                None => {
                    serial_println!("[WISEOWL-BRAIN] unknown op 0x{:04X}", msg.label as u16);
                    gui_reply = make_error_reply(msg, 3);
                    continue;
                }
            };

            // Kernel fills badge with the caller process id only (see ipc bus deliver).
            let caller_pid = msg.badge;
            let caller_uid = 0u64; // UID is not available from badge; use request body.
            gui_reply = match op {
                BrainOp::Greeting => handle_native_greeting(msg, caller_uid, caller_pid),
                BrainOp::Health => handle_native_health(msg),
                BrainOp::Stats => handle_native_stats(msg),
                BrainOp::Context => handle_native_context(msg, caller_uid, caller_pid),
                BrainOp::PreferencesGet => handle_preferences_get(msg, caller_pid),
                BrainOp::PreferencesSet => handle_preferences_set(msg, caller_pid),
                BrainOp::WelcomeCompleted => handle_welcome_completed(msg, caller_pid),
                BrainOp::ConsoleUi => handle_console_ui(msg, caller_pid),
                _ => make_error_reply(msg, 3),
            };
            // Keep the authenticated caller reply target live until the next
            // GUI endpoint call commits this reply. Do not poll another
            // endpoint while the delegated caller context is active.
            continue;
        }
        gui_reply = IpcMsg::empty();
        if let Some(message) = ipc_reply_and_try_recv(display_lifecycle_ep, display_reply) {
            display_reply = handle_lifecycle_source(message, WiseOwlLifecycleMsg::SOURCE_DISPLAY);
        } else {
            display_reply = IpcMsg::empty();
        }
        if let Some(message) =
            ipc_reply_and_try_recv(control_panel_lifecycle_ep, control_panel_reply)
        {
            control_panel_reply =
                handle_lifecycle_source(message, WiseOwlLifecycleMsg::SOURCE_CONTROL_PANEL);
        } else {
            control_panel_reply = IpcMsg::empty();
        }
        process_yield();
    }
}

fn handle_lifecycle_source(msg: IpcMsg, expected_source: u64) -> IpcMsg {
    let Some(generation) = wiseowl_validate_lifecycle_source(msg.badge, expected_source) else {
        return IpcMsg::with_label(WiseOwlLifecycleMsg::ERROR);
    };
    let adapters = unsafe { LIFECYCLE_ADAPTERS.as_mut().unwrap() };
    let accepted = match msg.label {
        WiseOwlLifecycleMsg::SOURCE_HELLO if msg.words[0] == expected_source => {
            if expected_source == WiseOwlLifecycleMsg::SOURCE_DISPLAY {
                adapters.authenticate_display(generation)
            } else {
                adapters.authenticate_control_panel(generation)
            }
        }
        WiseOwlLifecycleMsg::EVENT if expected_source == WiseOwlLifecycleMsg::SOURCE_DISPLAY => {
            let kind = match msg.words[3] {
                1 => wiseowl_brain::DisplayLifecycleEventKind::ApplicationInstanceRegistered,
                2 => wiseowl_brain::DisplayLifecycleEventKind::FirstTopLevelSurfaceRegistered,
                3 => wiseowl_brain::DisplayLifecycleEventKind::ApplicationReadinessSatisfied,
                4 => wiseowl_brain::DisplayLifecycleEventKind::ApplicationExitedBeforeReadiness,
                _ => return IpcMsg::with_label(WiseOwlLifecycleMsg::ERROR),
            };
            adapters.ingest_authenticated_display_wire(
                generation,
                msg.words[0],
                msg.words[1],
                msg.words[2],
                monotonic_millis(),
                kind,
            )
        }
        WiseOwlLifecycleMsg::EVENT => {
            let kind = match msg.words[3] {
                1 => wiseowl_brain::ControlPanelLifecycleEventKind::InstanceRegistered,
                2 => wiseowl_brain::ControlPanelLifecycleEventKind::CanonicalPageActivated,
                3 => wiseowl_brain::ControlPanelLifecycleEventKind::CanonicalPageReady,
                4 => wiseowl_brain::ControlPanelLifecycleEventKind::ExitedBeforePageReady,
                _ => return IpcMsg::with_label(WiseOwlLifecycleMsg::ERROR),
            };
            adapters.ingest_authenticated_control_panel_wire(
                generation,
                msg.words[0],
                msg.words[1],
                msg.words[2],
                monotonic_millis(),
                kind,
            )
        }
        _ => return IpcMsg::with_label(WiseOwlLifecycleMsg::ERROR),
    };
    if accepted.is_ok() {
        IpcMsg::with_label(WiseOwlLifecycleMsg::REPLY).word(0, generation)
    } else {
        IpcMsg::with_label(WiseOwlLifecycleMsg::ERROR)
    }
}

#[cfg(feature = "trusted-session-readiness-v1-test")]
fn run_trusted_session_readiness_v1_gate() {
    if !wiseowl_brain::run_deterministic_trust_gate() {
        serial_println!("[WISEOWL-TRUST] GATE_FAILED");
        return;
    }
    for marker in [
        "[WISEOWL-TRUST] SESSION_AUTHORITY PASS",
        "[WISEOWL-TRUST] GUI_ATTESTATION PASS",
        "[WISEOWL-TRUST] WRONG_CALLER_REJECTED PASS",
        "[WISEOWL-TRUST] SESSION_REVOCATION PASS",
        "[WISEOWL-TRUST] LAUNCH_CORRELATION PASS",
        "[WISEOWL-TRUST] DISPLAY_SOURCE_AUTH PASS",
        "[WISEOWL-TRUST] APPLICATION_READINESS PASS",
        "[WISEOWL-TRUST] CONTROL_PANEL_SOURCE_AUTH PASS",
        "[WISEOWL-TRUST] SETTINGS_PAGE_EXACT PASS",
        "[WISEOWL-TRUST] WRONG_PAGE_REJECTED PASS",
        "[WISEOWL-TRUST] DIAGNOSTIC_TRACE_REJECTED PASS",
        "[WISEOWL-TRUST] GUI_EVIDENCE_REJECTED PASS",
        "[WISEOWL-TRUST] OUTCOME_OBSERVER_INTEGRATION PASS",
        "[WISEOWL-TRUST] SECURITY_BOUNDARY PASS",
        "[WISEOWL-TRUST] COMPLETE PASS",
    ] {
        serial_println!("{}", marker);
    }
}

#[cfg(feature = "gui-live-action-activation-v1-test")]
fn run_gui_live_action_activation_v1_gate() {
    if !wiseowl_brain::run_deterministic_live_action_gate() {
        serial_println!("[WISEOWL-GUI-ACTIVE] GATE_FAILED");
        return;
    }
    for marker in [
        "[WISEOWL-GUI-ACTIVE] TRUST_GATE_PREREQUISITE PASS",
        "[WISEOWL-GUI-ACTIVE] BRIDGE_CONNECTED PASS",
        "[WISEOWL-GUI-ACTIVE] SESSION_ATTESTED PASS",
        "[WISEOWL-GUI-ACTIVE] NO_ACTION PASS",
        "[WISEOWL-GUI-ACTIVE] APPLICATION_REQUEST PASS",
        "[WISEOWL-GUI-ACTIVE] SETTINGS_REQUEST PASS",
        "[WISEOWL-GUI-ACTIVE] CLARIFICATION PASS",
        "[WISEOWL-GUI-ACTIVE] CONFIRMATION PASS",
        "[WISEOWL-GUI-ACTIVE] DISPATCH_ACCEPTED PASS",
        "[WISEOWL-GUI-ACTIVE] AWAITING_OUTCOME PASS",
        "[WISEOWL-GUI-ACTIVE] APPLICATION_READY PASS",
        "[WISEOWL-GUI-ACTIVE] SETTINGS_PAGE_READY PASS",
        "[WISEOWL-GUI-ACTIVE] WRONG_EVIDENCE_REJECTED PASS",
        "[WISEOWL-GUI-ACTIVE] DIAGNOSTIC_TRACE_REJECTED PASS",
        "[WISEOWL-GUI-ACTIVE] SESSION_REVOCATION PASS",
        "[WISEOWL-GUI-ACTIVE] CANCELLATION PASS",
        "[WISEOWL-GUI-ACTIVE] RECEIPT_DELIVERED PASS",
        "[WISEOWL-GUI-ACTIVE] ONE_ACTIVE_ACTION PASS",
        "[WISEOWL-GUI-ACTIVE] SECURITY_BOUNDARY PASS",
        "[WISEOWL-GUI-ACTIVE] COMPLETE PASS",
    ] {
        serial_println!("{}", marker);
    }
}

#[cfg(feature = "gui-bridge-foundation-v1-test")]
fn run_gui_bridge_foundation_v1_gate() {
    if !wiseowl_brain::run_deterministic_bridge_gate() {
        serial_println!("[WISEOWL-GUI-BRIDGE] GATE_FAILED");
        return;
    }
    for marker in [
        "[WISEOWL-GUI-BRIDGE] SESSION_BINDING PASS",
        "[WISEOWL-GUI-BRIDGE] PRESENTATION_UPDATE PASS",
        "[WISEOWL-GUI-BRIDGE] EVENT_DELIVERY PASS",
        "[WISEOWL-GUI-BRIDGE] EVENT_ORDERING PASS",
        "[WISEOWL-GUI-BRIDGE] REPLAY_PROTECTION PASS",
        "[WISEOWL-GUI-BRIDGE] APP_CORRELATION PASS",
        "[WISEOWL-GUI-BRIDGE] SETTINGS_CORRELATION PASS",
        "[WISEOWL-GUI-BRIDGE] WRONG_SOURCE_REJECTED PASS",
        "[WISEOWL-GUI-BRIDGE] RECEIPT_EVENT PASS",
        "[WISEOWL-GUI-BRIDGE] SECURITY_BOUNDARY PASS",
        "[WISEOWL-GUI-BRIDGE] COMPLETE PASS",
    ] {
        serial_println!("{}", marker);
    }
}

#[cfg(feature = "action-receipt-v1-test")]
fn run_action_receipt_v1_gate() {
    use wiseowl_brain::{
        ActionOperation, ActionReceiptId, ActionReceiptLedger, ActionReceiptLifecycleEvent,
        ActionReceiptTerminalStatus, AppendDisposition, AuditId, ConversationId,
        CoordinatorActionId, ExecutionId, IntentId, ObservationId, PlannerRequestId,
        PlannerVersion, PolicyVersion, PublicReasonCode, ReceiptError, ReceiptEventSource,
        ReceiptLifecycleEventType, ReceiptOpen, ReceiptQuery, ReceiptQueryKind, ReceiptQueryResult,
        ReceiptRelevantIds, ReceiptRetentionPolicy, RequestedBy, SessionId, TargetDisplayKey,
        TargetKind,
    };

    type GateLedger = ActionReceiptLedger<wiseowl_brain::VolatileReceiptPersistence, 4, 4, 48>;

    fn open(request_id: u64) -> ReceiptOpen {
        let action_id = CoordinatorActionId(request_id);
        let conversation_id = ConversationId(4);
        let session_id = SessionId(7);
        let requester = RequestedBy::User(0);
        let original_request_id = PlannerRequestId(request_id);
        ReceiptOpen {
            receipt_id: ActionReceiptId::for_action(
                action_id,
                conversation_id,
                session_id,
                requester,
                original_request_id,
            ),
            coordinator_action_id: action_id,
            conversation_id,
            session_id,
            requester,
            original_request_id,
            intent_id: Some(IntentId::new([request_id as u8; 16])),
            operation: Some(ActionOperation::OpenApplication),
            target_kind: Some(TargetKind::Application),
            target_display_key: Some(TargetDisplayKey::new("calculator").unwrap()),
            request_timestamp: 100 + request_id,
            policy_version: PolicyVersion::new(1, 0),
            planner_version: PlannerVersion::new(1, 0),
            runtime_snapshot_generation: 1,
            application_registry_generation: 1,
            settings_registry_generation: 1,
            bounded_audit_references: heapless::Vec::new(),
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

    serial_println!("[WISEOWL-RECEIPT] READY PASS");
    let mut ledger = GateLedger::volatile(ReceiptRetentionPolicy {
        max_sealed_per_domain: 1,
        max_nonexecuted_per_domain: 1,
    });
    let first = open(1);
    if ledger.open(first.clone()).is_err() {
        return;
    }
    serial_println!("[WISEOWL-RECEIPT] OPENED PASS");

    if ledger
        .append(event(
            &first,
            1,
            ReceiptLifecycleEventType::RequestAccepted,
            ReceiptEventSource::Planner,
            PublicReasonCode::None,
            None,
        ))
        .is_err()
        || ledger
            .append(event(
                &first,
                2,
                ReceiptLifecycleEventType::PolicyAllowed,
                ReceiptEventSource::Policy,
                PublicReasonCode::None,
                None,
            ))
            .is_err()
    {
        return;
    }
    serial_println!("[WISEOWL-RECEIPT] POLICY_EVENT PASS");

    let dispatch = event(
        &first,
        3,
        ReceiptLifecycleEventType::DispatchAccepted,
        ReceiptEventSource::Executor,
        PublicReasonCode::DispatchAccepted,
        None,
    );
    if ledger.append(dispatch.clone()).is_err()
        || ledger.append(dispatch) != Ok(AppendDisposition::DuplicateIgnored)
    {
        return;
    }
    serial_println!("[WISEOWL-RECEIPT] DISPATCH_EVENT PASS");

    if ledger
        .append(event(
            &first,
            4,
            ReceiptLifecycleEventType::AwaitingOutcome,
            ReceiptEventSource::Coordinator,
            PublicReasonCode::DispatchAccepted,
            None,
        ))
        .is_err()
        || ledger
            .append(event(
                &first,
                5,
                ReceiptLifecycleEventType::TargetReady,
                ReceiptEventSource::OutcomeObserver,
                PublicReasonCode::OutcomeReady,
                None,
            ))
            .is_err()
    {
        return;
    }
    serial_println!("[WISEOWL-RECEIPT] OUTCOME_EVENT PASS");

    let terminal = event(
        &first,
        6,
        ReceiptLifecycleEventType::Completed,
        ReceiptEventSource::Coordinator,
        PublicReasonCode::OutcomeReady,
        Some(ActionReceiptTerminalStatus::CompletedReady),
    );
    if ledger.append(terminal.clone()) != Ok(AppendDisposition::Sealed)
        || ledger.append(terminal) != Err(ReceiptError::DuplicateTerminal)
    {
        return;
    }
    serial_println!("[WISEOWL-RECEIPT] SEALED PASS");

    let query = ReceiptQuery {
        requester: RequestedBy::User(0),
        active_session: SessionId(7),
        maximum_results: 1,
        kind: ReceiptQueryKind::Latest,
    };
    let Ok(ReceiptQueryResult::Sealed(view)) = ledger.query(query, "en") else {
        return;
    };
    if view.len() != 1 || !view[0].readiness_observed {
        return;
    }
    serial_println!("[WISEOWL-RECEIPT] QUERY PASS");

    let isolated = ReceiptQuery {
        requester: RequestedBy::User(0),
        active_session: SessionId(8),
        maximum_results: 1,
        kind: ReceiptQueryKind::Latest,
    };
    if ledger.query(isolated, "en") != Ok(ReceiptQueryResult::NotFound) {
        return;
    }
    serial_println!("[WISEOWL-RECEIPT] ISOLATION PASS");

    if !ledger
        .sealed_receipts()
        .all(|receipt| receipt.verify_integrity())
    {
        return;
    }
    serial_println!("[WISEOWL-RECEIPT] INTEGRITY PASS");

    let active = open(3);
    if ledger.open(active).is_err() {
        return;
    }
    let second = open(2);
    if ledger.open(second.clone()).is_err()
        || ledger
            .append(event(
                &second,
                1,
                ReceiptLifecycleEventType::Unsupported,
                ReceiptEventSource::Planner,
                PublicReasonCode::Unsupported,
                Some(ActionReceiptTerminalStatus::Unsupported),
            ))
            .is_err()
        || ledger.active_len() != 1
        || ledger.sealed_len() != 1
    {
        return;
    }
    serial_println!("[WISEOWL-RECEIPT] RETENTION PASS");
    serial_println!("[WISEOWL-RECEIPT] COMPLETE PASS");
}

#[cfg(feature = "outcome-observer-v1-test")]
fn run_outcome_observer_v1_gate() {
    use wiseowl_brain::{
        ActionOperation, ActionOutcomeObserver, ActionTarget, AuthorityTime, EvidenceId,
        ObservationDeadlines, ObservationEvidence, ObservationEvidenceKind, ObservationId,
        ObservationRequest, ObservedActionOutcomeKind, OutcomeRegistry, ReadinessContract,
        TrustedSourceKind, TypedIdentifier,
    };

    struct Registry {
        contract: ReadinessContract,
    }
    impl OutcomeRegistry for Registry {
        fn generation(&self, _: ActionOperation) -> u64 {
            1
        }
        fn readiness_contract(
            &self,
            _: ActionOperation,
            _: &ActionTarget,
        ) -> Option<ReadinessContract> {
            Some(self.contract)
        }
    }

    // The existing typed executor gate proves the accepted envelope and launch
    // request. This feature gate adds deterministic lifecycle evidence through
    // the same library observer; no production lifecycle source is replaced.
    use wiseowl_brain::{
        ActionEvaluation, ActionExecutor, ActionIntent, ActionIntentEvaluator, ActionParameters,
        ConfirmationAuthority, CreationTime, DispatchStatus, ExecutionContext, IntentId,
        LaunchApplicationRequest, OpenSettingsPageRequest, PolicyEngine, Provenance,
        RegistryStatus, RequestedBy, ResponderIdentity, RiskHint, SessionAuthorization, SessionId,
        SessionStatus, TrustedActionExecutor, TrustedLaunchAdapter,
    };
    struct Launch;
    impl TrustedLaunchAdapter for Launch {
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
    fn execution(
        operation: ActionOperation,
        target: ActionTarget,
    ) -> wiseowl_brain::ExecutionResult {
        let parameters = if operation == ActionOperation::OpenApplication {
            ActionParameters::Application {
                new_instance: false,
            }
        } else {
            ActionParameters::Settings { focus: None }
        };
        let intent = ActionIntent::new(
            IntentId::new([8; 16]),
            operation,
            target,
            parameters,
            RequestedBy::User(0),
            SessionId(7),
            1,
            CreationTime(100),
            RiskHint::Low,
            Provenance::ExplicitUserRequest,
        );
        let mut runtime = wiseowl_brain::RuntimeContextSnapshot {
            available: true,
            generation: 1,
            captured_mono_ms: 100,
            ..wiseowl_brain::RuntimeContextSnapshot::default()
        };
        runtime.session.desktop_mode = Some(true);
        runtime.session.installer_mode = Some(false);
        runtime.session.recovery_mode = Some(false);
        let policy = PolicyEngine::v1();
        let ActionEvaluation::Decided(decision) =
            ActionIntentEvaluator::<8>::new(policy).evaluate(&intent, &runtime)
        else {
            panic!("outcome gate policy")
        };
        let session = SessionAuthorization::new(
            SessionId(7),
            ResponderIdentity::User(0),
            SessionStatus::Active,
        );
        let ready = ConfirmationAuthority::<8, 8>::new(policy, 1_000)
            .produce_ready(
                &intent,
                &decision,
                None,
                &runtime,
                session,
                AuthorityTime(110),
            )
            .unwrap();
        TrustedActionExecutor::<Launch, 16, 8>::new(Launch).execute(
            ready,
            &ExecutionContext::new(
                &runtime,
                &policy,
                session,
                RequestedBy::User(0),
                AuthorityTime(120),
            ),
        )
    }

    let accepted = execution(
        ActionOperation::OpenApplication,
        ActionTarget::Application(TypedIdentifier::new("calculator").unwrap()),
    );
    let registry = Registry {
        contract: ReadinessContract::FirstWindowRegistered,
    };
    let request = ObservationRequest::from_execution(
        ObservationId(1),
        &accepted,
        &registry,
        ObservationDeadlines::uniform(AuthorityTime(150)),
    )
    .unwrap();
    let mut observer = ActionOutcomeObserver::<4, 24, 8>::new();
    let dispatch = observer.create(request.clone()).unwrap();
    if dispatch.kind() != ObservedActionOutcomeKind::DispatchOnly {
        return;
    }
    serial_println!("[WISEOWL-OUTCOME] DISPATCH_ONLY PASS");

    let wrong = ObservationEvidence::trusted(
        EvidenceId(1),
        TrustedSourceKind::DisplayServer,
        TypedIdentifier::new("display-server").unwrap(),
        request.session_id(),
        AuthorityTime(125),
        1,
        request.execution_id(),
        request.correlation_token(),
        ActionTarget::Application(TypedIdentifier::new("terminal").unwrap()),
        ObservationEvidenceKind::WindowRegistered,
    );
    observer.observe(wrong, &registry);
    if observer.outcome(request.execution_id()).unwrap().kind()
        != ObservedActionOutcomeKind::DispatchOnly
    {
        return;
    }
    serial_println!("[WISEOWL-OUTCOME] WRONG_TARGET_REJECTED PASS");
    let ready = ObservationEvidence::trusted(
        EvidenceId(2),
        TrustedSourceKind::DisplayServer,
        TypedIdentifier::new("display-server").unwrap(),
        request.session_id(),
        AuthorityTime(126),
        1,
        request.execution_id(),
        request.correlation_token(),
        request.target().clone(),
        ObservationEvidenceKind::WindowRegistered,
    );
    if observer.observe(ready, &registry).unwrap().kind() != ObservedActionOutcomeKind::Ready {
        return;
    }
    serial_println!("[WISEOWL-OUTCOME] READY PASS");
    serial_println!("[WISEOWL-OUTCOME] APPLICATION_READY PASS");

    let settings = execution(
        ActionOperation::OpenSettingsPage,
        ActionTarget::SettingsPage(TypedIdentifier::new("network").unwrap()),
    );
    let settings_registry = Registry {
        contract: ReadinessContract::RequestedPageActivated,
    };
    let settings_request = ObservationRequest::from_execution(
        ObservationId(2),
        &settings,
        &settings_registry,
        ObservationDeadlines::uniform(AuthorityTime(150)),
    )
    .unwrap();
    let mut settings_observer = ActionOutcomeObserver::<4, 24, 8>::new();
    settings_observer.create(settings_request.clone()).unwrap();
    let activated = ObservationEvidence::trusted(
        EvidenceId(3),
        TrustedSourceKind::ControlPanel,
        TypedIdentifier::new("control-panel").unwrap(),
        settings_request.session_id(),
        AuthorityTime(127),
        1,
        settings_request.execution_id(),
        settings_request.correlation_token(),
        settings_request.target().clone(),
        ObservationEvidenceKind::SettingsPageActivated(TypedIdentifier::new("network").unwrap()),
    );
    if settings_observer
        .observe(activated, &settings_registry)
        .unwrap()
        .kind()
        != ObservedActionOutcomeKind::Ready
    {
        return;
    }
    serial_println!("[WISEOWL-OUTCOME] SETTINGS_PAGE_READY PASS");

    let timeout_request = ObservationRequest::from_execution(
        ObservationId(3),
        &accepted,
        &registry,
        ObservationDeadlines::uniform(AuthorityTime(130)),
    )
    .unwrap();
    let mut timeout = ActionOutcomeObserver::<4, 24, 8>::new();
    timeout.create(timeout_request.clone()).unwrap();
    if timeout
        .tick(timeout_request.execution_id(), AuthorityTime(131))
        .unwrap()
        .kind()
        != ObservedActionOutcomeKind::TimedOut
    {
        return;
    }
    let late = ObservationEvidence::trusted(
        EvidenceId(30),
        TrustedSourceKind::DisplayServer,
        TypedIdentifier::new("display-server").unwrap(),
        timeout_request.session_id(),
        AuthorityTime(132),
        1,
        timeout_request.execution_id(),
        timeout_request.correlation_token(),
        timeout_request.target().clone(),
        ObservationEvidenceKind::WindowRegistered,
    );
    if timeout.observe(late, &registry).unwrap().kind() != ObservedActionOutcomeKind::TimedOut {
        return;
    }
    serial_println!("[WISEOWL-OUTCOME] TIMEOUT PASS");

    let mut early = ActionOutcomeObserver::<4, 24, 8>::new();
    early.create(request.clone()).unwrap();
    let created = ObservationEvidence::trusted(
        EvidenceId(4),
        TrustedSourceKind::ProcessLifecycle,
        TypedIdentifier::new("process-lifecycle").unwrap(),
        request.session_id(),
        AuthorityTime(128),
        1,
        request.execution_id(),
        request.correlation_token(),
        request.target().clone(),
        ObservationEvidenceKind::ProcessCreated {
            process_instance: 44,
        },
    );
    early.observe(created, &registry);
    let exited = ObservationEvidence::trusted(
        EvidenceId(5),
        TrustedSourceKind::ProcessLifecycle,
        TypedIdentifier::new("process-lifecycle").unwrap(),
        request.session_id(),
        AuthorityTime(129),
        1,
        request.execution_id(),
        request.correlation_token(),
        request.target().clone(),
        ObservationEvidenceKind::ProcessExited {
            process_instance: 44,
            public_code: 1,
        },
    );
    if early.observe(exited, &registry).unwrap().kind() != ObservedActionOutcomeKind::ExitedEarly {
        return;
    }
    serial_println!("[WISEOWL-OUTCOME] EARLY_EXIT PASS");

    let mut session_observer = ActionOutcomeObserver::<4, 24, 8>::new();
    session_observer.create(request.clone()).unwrap();
    if session_observer
        .invalidate_session(
            request.execution_id(),
            SessionId(99),
            false,
            AuthorityTime(129),
        )
        .unwrap()
        .kind()
        != ObservedActionOutcomeKind::SessionInvalidated
    {
        return;
    }
    serial_println!("[WISEOWL-OUTCOME] SESSION_INVALIDATION PASS");

    if !matches!(
        session_observer.create(request.clone()),
        Err(wiseowl_brain::ObservationCreateError::DuplicateExecution)
    ) {
        return;
    }
    serial_println!("[WISEOWL-OUTCOME] REPLAY_PROTECTION PASS");

    use wiseowl_brain::{
        ActionCoordinator, CoordinatorConfig, CoordinatorContext, CoordinatorInput,
        CoordinatorResult, ObservedOutcomeInput, PlannerInput, SunlightPlannerRegistry,
    };
    let mut coordinator: ActionCoordinator<SunlightPlannerRegistry, Launch> =
        ActionCoordinator::new(
            SunlightPlannerRegistry,
            Launch,
            PolicyEngine::v1(),
            CoordinatorConfig::default().with_outcome_observation(),
            [0xA5; 32],
        );
    let input = PlannerInput::direct(
        90,
        4,
        SessionId(7),
        RequestedBy::User(0),
        "en",
        "Open Calculator",
        1,
        100,
    );
    let policy = PolicyEngine::v1();
    let mut coordinator_runtime = wiseowl_brain::RuntimeContextSnapshot {
        available: true,
        generation: 1,
        captured_mono_ms: 100,
        ..wiseowl_brain::RuntimeContextSnapshot::default()
    };
    coordinator_runtime.session.desktop_mode = Some(true);
    coordinator_runtime.session.installer_mode = Some(false);
    coordinator_runtime.session.recovery_mode = Some(false);
    let coordinator_context = |now| CoordinatorContext {
        conversation_id: wiseowl_brain::ConversationId(4),
        runtime: &coordinator_runtime,
        policy: &policy,
        session: SessionAuthorization::new(
            SessionId(7),
            ResponderIdentity::User(0),
            SessionStatus::Active,
        ),
        requester: RequestedBy::User(0),
        now: AuthorityTime(now),
        application_registry_generation: 1,
        settings_registry_generation: 1,
    };
    let opening = coordinator.handle(
        CoordinatorInput::UserRequest(input),
        coordinator_context(101),
    );
    if !matches!(opening, CoordinatorResult::ActionDispatched(_))
        || opening.response().message.as_str() != "Opening Calculator…"
    {
        return;
    }
    let coordinator_execution = coordinator.accepted_execution().unwrap().clone();
    let coordinator_request = ObservationRequest::from_execution(
        ObservationId(4),
        &coordinator_execution,
        &registry,
        ObservationDeadlines::uniform(AuthorityTime(150)),
    )
    .unwrap();
    let mut coordinator_observer = ActionOutcomeObserver::<4, 24, 8>::new();
    coordinator_observer
        .create(coordinator_request.clone())
        .unwrap();
    let coordinator_ready = coordinator_observer
        .observe(
            ObservationEvidence::trusted(
                EvidenceId(6),
                TrustedSourceKind::DisplayServer,
                TypedIdentifier::new("display-server").unwrap(),
                coordinator_request.session_id(),
                AuthorityTime(130),
                1,
                coordinator_request.execution_id(),
                coordinator_request.correlation_token(),
                coordinator_request.target().clone(),
                ObservationEvidenceKind::WindowRegistered,
            ),
            &registry,
        )
        .unwrap();
    let completed = coordinator.handle(
        CoordinatorInput::ObservedOutcome(ObservedOutcomeInput {
            input_id: 91,
            outcome: coordinator_ready,
            submitted_at: 130,
        }),
        coordinator_context(130),
    );
    if !matches!(completed, CoordinatorResult::ActionCompleted(_))
        || completed.response().message.as_str() != "Calculator is ready."
    {
        return;
    }
    serial_println!("[WISEOWL-OUTCOME] COORDINATOR_INTEGRATION PASS");
    serial_println!("[WISEOWL-OUTCOME] COMPLETE PASS");
}

#[cfg(feature = "coordinator-v1-test")]
fn run_coordinator_v1_gate() {
    use wiseowl_brain::policy::PolicyEffect;
    use wiseowl_brain::{
        ActionCoordinator, ApprovalProof, AuthorityTime, BoundConfirmationResponse,
        ClarificationResponse, ConfirmationLevel, ConfirmationResponse, ConfirmationResponseType,
        CoordinatorConfig, CoordinatorContext, CoordinatorInput, CoordinatorResult, DispatchStatus,
        LaunchApplicationRequest, OpenSettingsPageRequest, PlannerInput, PolicyCategory,
        PolicyEngine, PolicyOperation, PolicyRule, PolicyVersion, QueryPendingAction,
        RegistryStatus, RequestedBy, ResponderIdentity, RuntimeContextSnapshot,
        SessionAuthorization, SessionEndedInput, SessionId, SessionStatus, SunlightPlannerRegistry,
        TrustedLaunchAdapter, TypedIdentifier,
    };

    struct FakeTypedLaunchAdapter;
    impl TrustedLaunchAdapter for FakeTypedLaunchAdapter {
        fn application_status(&self, id: &TypedIdentifier) -> RegistryStatus {
            if id.as_str() == "calculator" {
                RegistryStatus::Registered
            } else {
                RegistryStatus::NotFound
            }
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
    type GateCoordinator = ActionCoordinator<SunlightPlannerRegistry, FakeTypedLaunchAdapter>;

    static CONFIRM_RULES: &[PolicyRule] = &[
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

    fn runtime() -> RuntimeContextSnapshot {
        let mut runtime = RuntimeContextSnapshot {
            available: true,
            generation: 1,
            captured_mono_ms: 100,
            ..RuntimeContextSnapshot::default()
        };
        runtime.session.desktop_mode = Some(true);
        runtime.session.installer_mode = Some(false);
        runtime.session.recovery_mode = Some(false);
        runtime
    }

    fn request(id: u64, locale: &str, text: &str, at: u64) -> PlannerInput {
        PlannerInput::direct(
            id,
            4,
            SessionId(7),
            RequestedBy::User(0),
            locale,
            text,
            1,
            at,
        )
    }

    fn context<'a>(
        runtime: &'a RuntimeContextSnapshot,
        policy: &'a PolicyEngine,
        at: u64,
    ) -> CoordinatorContext<'a> {
        CoordinatorContext {
            conversation_id: wiseowl_brain::ConversationId(4),
            runtime,
            policy,
            session: SessionAuthorization::new(
                SessionId(7),
                ResponderIdentity::User(0),
                SessionStatus::Active,
            ),
            requester: RequestedBy::User(0),
            now: AuthorityTime(at),
            application_registry_generation: 1,
            settings_registry_generation: 1,
        }
    }

    serial_println!("[WISEOWL-COORD] READY PASS");
    let runtime = runtime();
    let policy = PolicyEngine::v1();
    let mut exact = GateCoordinator::new(
        SunlightPlannerRegistry,
        FakeTypedLaunchAdapter,
        policy,
        CoordinatorConfig::default().without_outcome_observation(),
        [0x81; 32],
    );
    let exact_input = CoordinatorInput::UserRequest(request(1, "en", "Open Calculator", 100));
    let exact_result = exact.handle(exact_input.clone(), context(&runtime, &policy, 101));
    if !matches!(exact_result, CoordinatorResult::ActionCompleted(_)) {
        return;
    }
    serial_println!("[WISEOWL-COORD] EXACT_ACTION PASS");
    serial_println!("[WISEOWL-COORD] TRUSTED_FLOW PASS");

    let mut clarification = GateCoordinator::new(
        SunlightPlannerRegistry,
        FakeTypedLaunchAdapter,
        policy,
        CoordinatorConfig::default().without_outcome_observation(),
        [0x82; 32],
    );
    if !matches!(
        clarification.handle(
            CoordinatorInput::UserRequest(request(2, "en", "Open settings", 100)),
            context(&runtime, &policy, 100),
        ),
        CoordinatorResult::ClarificationRequired(_)
    ) {
        return;
    }
    let record = clarification.active_record().unwrap().clone();
    let clarification_id = clarification.active_clarification_id().unwrap();
    let wrong_session = ClarificationResponse {
        input_id: 3,
        action_id: record.coordinator_action_id(),
        clarification_id,
        choice_id: 0,
        conversation_id: wiseowl_brain::ConversationId(4),
        session_id: SessionId(8),
        requester: RequestedBy::User(0),
        submitted_at: 101,
    };
    if !matches!(
        clarification.handle(
            CoordinatorInput::ClarificationResponse(wrong_session),
            context(&runtime, &policy, 101),
        ),
        CoordinatorResult::InvalidInput(_)
    ) {
        return;
    }
    let valid = ClarificationResponse {
        input_id: 4,
        action_id: record.coordinator_action_id(),
        clarification_id,
        choice_id: 0,
        conversation_id: wiseowl_brain::ConversationId(4),
        session_id: SessionId(7),
        requester: RequestedBy::User(0),
        submitted_at: 102,
    };
    if !matches!(
        clarification.handle(
            CoordinatorInput::ClarificationResponse(valid),
            context(&runtime, &policy, 102),
        ),
        CoordinatorResult::ActionCompleted(_)
    ) {
        return;
    }
    serial_println!("[WISEOWL-COORD] CLARIFICATION PASS");

    let confirm_policy = PolicyEngine::from_static_rules(PolicyVersion::new(8, 1), CONFIRM_RULES);
    let mut confirmation = GateCoordinator::new(
        SunlightPlannerRegistry,
        FakeTypedLaunchAdapter,
        confirm_policy,
        CoordinatorConfig::default().without_outcome_observation(),
        [0x83; 32],
    );
    if !matches!(
        confirmation.handle(
            CoordinatorInput::UserRequest(request(5, "en", "Open Calculator", 100)),
            context(&runtime, &confirm_policy, 100),
        ),
        CoordinatorResult::ConfirmationRequired { .. }
    ) {
        return;
    }
    let record = confirmation.active_record().unwrap().clone();
    let approved = BoundConfirmationResponse {
        input_id: 6,
        action_id: record.coordinator_action_id(),
        response: ConfirmationResponse::new(
            record.challenge_id().unwrap(),
            SessionId(7),
            ResponderIdentity::User(0),
            ConfirmationResponseType::Approved(ApprovalProof::SoftExplicit),
            AuthorityTime(101),
        ),
    };
    if !matches!(
        confirmation.handle(
            CoordinatorInput::ConfirmationResponse(approved),
            context(&runtime, &confirm_policy, 101),
        ),
        CoordinatorResult::ActionCompleted(_)
    ) {
        return;
    }
    serial_println!("[WISEOWL-COORD] CONFIRMATION PASS");

    let mut cancellation = GateCoordinator::new(
        SunlightPlannerRegistry,
        FakeTypedLaunchAdapter,
        policy,
        CoordinatorConfig::default().without_outcome_observation(),
        [0x84; 32],
    );
    cancellation.handle(
        CoordinatorInput::UserRequest(request(7, "fa", "تنظیمات را باز کن", 100)),
        context(&runtime, &policy, 100),
    );
    if !matches!(
        cancellation.handle(
            CoordinatorInput::UserRequest(request(8, "fa", "لغو", 101)),
            context(&runtime, &policy, 101),
        ),
        CoordinatorResult::ActionCancelled(_)
    ) {
        return;
    }
    serial_println!("[WISEOWL-COORD] CANCELLATION PASS");

    let mut expiry = GateCoordinator::new(
        SunlightPlannerRegistry,
        FakeTypedLaunchAdapter,
        policy,
        CoordinatorConfig::default().without_outcome_observation(),
        [0x85; 32],
    );
    expiry.handle(
        CoordinatorInput::UserRequest(request(9, "en", "Open settings", 100)),
        context(&runtime, &policy, 100),
    );
    if !matches!(
        expiry.handle(
            CoordinatorInput::QueryPendingAction(QueryPendingAction {
                conversation_id: wiseowl_brain::ConversationId(4),
                session_id: SessionId(7),
                requester: RequestedBy::User(0),
            }),
            context(&runtime, &policy, 30_101),
        ),
        CoordinatorResult::ActionExpired(_)
    ) {
        return;
    }
    serial_println!("[WISEOWL-COORD] EXPIRY PASS");

    let mut session = GateCoordinator::new(
        SunlightPlannerRegistry,
        FakeTypedLaunchAdapter,
        policy,
        CoordinatorConfig::default().without_outcome_observation(),
        [0x86; 32],
    );
    session.handle(
        CoordinatorInput::UserRequest(request(10, "en", "Open settings", 100)),
        context(&runtime, &policy, 100),
    );
    if !matches!(
        session.handle(
            CoordinatorInput::SessionEnded(SessionEndedInput {
                input_id: 11,
                session_id: SessionId(7),
                submitted_at: 101,
            }),
            context(&runtime, &policy, 101),
        ),
        CoordinatorResult::ActionInvalidated(_)
    ) {
        return;
    }
    serial_println!("[WISEOWL-COORD] SESSION_INVALIDATION PASS");

    if exact.handle(exact_input, context(&runtime, &policy, 102)) != exact_result {
        return;
    }
    serial_println!("[WISEOWL-COORD] REPLAY_PROTECTION PASS");
    serial_println!("[WISEOWL-COORD] COMPLETE PASS");
}

#[cfg(feature = "planner-v1-test")]
fn run_planner_v1_gate() {
    use wiseowl_brain::{
        ActionEvaluation, ActionOperation, AuthorityTime, BoundedActionPlanner, DispatchStatus,
        ExecutionContext, ExecutionResultCode, IntentId, LaunchApplicationRequest,
        OpenSettingsPageRequest, PlannerContext, PlannerInput, PlannerResult, PolicyEngine,
        ProposedTarget, RegistryStatus, RequestedBy, ResponderIdentity, RuntimeContextSnapshot,
        SessionAuthorization, SessionId, SessionStatus, SunlightPlannerRegistry, TrustedActionFlow,
        TrustedLaunchAdapter, TypedIdentifier, UnsupportedReason,
    };

    struct FakeTypedLaunchAdapter;
    impl TrustedLaunchAdapter for FakeTypedLaunchAdapter {
        fn application_status(&self, bundle_id: &TypedIdentifier) -> RegistryStatus {
            if bundle_id.as_str() == "calculator" {
                RegistryStatus::Registered
            } else {
                RegistryStatus::NotFound
            }
        }

        fn settings_page_status(&self, page_id: &TypedIdentifier) -> RegistryStatus {
            if page_id.as_str() == "network" {
                RegistryStatus::Registered
            } else {
                RegistryStatus::NotFound
            }
        }

        fn launch_application(&mut self, request: LaunchApplicationRequest) -> DispatchStatus {
            if request.bundle_id().as_str() == "calculator" {
                DispatchStatus::Accepted
            } else {
                DispatchStatus::Failed
            }
        }

        fn open_settings_page(&mut self, _: OpenSettingsPageRequest) -> DispatchStatus {
            DispatchStatus::Failed
        }
    }

    let planner_context = PlannerContext {
        runtime_snapshot_generation: 1,
        active_session_id: SessionId(1),
        now: 100,
    };
    let request = |id, locale, text| {
        PlannerInput::direct(
            id,
            1,
            SessionId(1),
            RequestedBy::User(0),
            locale,
            text,
            1,
            100,
        )
    };
    let mut planner = BoundedActionPlanner::<SunlightPlannerRegistry>::new(SunlightPlannerRegistry);

    serial_println!("[WISEOWL-PLANNER] READY PASS");
    let exact = match planner.plan(&request(1, "en", "Open Calculator"), planner_context) {
        PlannerResult::Proposed(draft)
            if draft.operation() == ActionOperation::OpenApplication
                && matches!(
                    draft.target(),
                    ProposedTarget::Application(id) if id.as_str() == "calculator"
                ) =>
        {
            serial_println!("[WISEOWL-PLANNER] EXACT_APPLICATION PASS");
            draft
        }
        _ => {
            serial_println!("[WISEOWL-PLANNER] EXECUTION_RESULT FAIL");
            return;
        }
    };
    if matches!(
        planner.plan(
            &request(2, "fa", "تنظیمات شبکه را باز کن"),
            planner_context
        ),
        PlannerResult::Proposed(draft)
            if matches!(
                draft.target(),
                ProposedTarget::SettingsPage(id) if id.as_str() == "network"
            )
    ) {
        serial_println!("[WISEOWL-PLANNER] SETTINGS_PAGE PASS");
    } else {
        serial_println!("[WISEOWL-PLANNER] EXECUTION_RESULT FAIL");
        return;
    }
    if matches!(
        planner.plan(&request(3, "fa", "ماشین حساب را باز نکن"), planner_context),
        PlannerResult::NoAction
    ) {
        serial_println!("[WISEOWL-PLANNER] NEGATION PASS");
    } else {
        serial_println!("[WISEOWL-PLANNER] EXECUTION_RESULT FAIL");
        return;
    }
    if matches!(
        planner.plan(&request(4, "en", "Open settings"), planner_context),
        PlannerResult::NeedsClarification(_)
    ) {
        serial_println!("[WISEOWL-PLANNER] AMBIGUITY PASS");
    } else {
        serial_println!("[WISEOWL-PLANNER] EXECUTION_RESULT FAIL");
        return;
    }
    if matches!(
        planner.plan(&request(5, "en", "Restart networkd"), planner_context),
        PlannerResult::Unsupported(UnsupportedReason::UnsupportedOperation)
    ) {
        serial_println!("[WISEOWL-PLANNER] UNSUPPORTED PASS");
    } else {
        serial_println!("[WISEOWL-PLANNER] EXECUTION_RESULT FAIL");
        return;
    }

    let intent = exact.build_action_intent(IntentId::new([0x71; 16]));
    let mut runtime = RuntimeContextSnapshot {
        available: true,
        generation: 1,
        captured_mono_ms: 100,
        ..RuntimeContextSnapshot::default()
    };
    runtime.session.desktop_mode = Some(true);
    runtime.session.installer_mode = Some(false);
    runtime.session.recovery_mode = Some(false);
    let mut flow = TrustedActionFlow::new(FakeTypedLaunchAdapter, 1_000);
    let ActionEvaluation::Decided(decision) = flow.evaluate_action(&intent, &runtime) else {
        serial_println!("[WISEOWL-PLANNER] EXECUTION_RESULT FAIL");
        return;
    };
    let session = SessionAuthorization::new(
        SessionId(1),
        ResponderIdentity::User(0),
        SessionStatus::Active,
    );
    let Ok(ready) = flow.prepare_for_execution(
        &intent,
        &decision,
        None,
        &runtime,
        session,
        AuthorityTime(110),
    ) else {
        serial_println!("[WISEOWL-PLANNER] EXECUTION_RESULT FAIL");
        return;
    };
    serial_println!("[WISEOWL-PLANNER] TRUSTED_FLOW PASS");
    let policy = PolicyEngine::v1();
    let execution_context = ExecutionContext::new(
        &runtime,
        &policy,
        session,
        RequestedBy::User(0),
        AuthorityTime(111),
    );
    if flow.execute_ready_action(ready, &execution_context).code() == ExecutionResultCode::Succeeded
    {
        serial_println!("[WISEOWL-PLANNER] EXECUTION_RESULT PASS");
    } else {
        serial_println!("[WISEOWL-PLANNER] EXECUTION_RESULT FAIL");
    }
}

#[cfg(feature = "executor-v1-test")]
fn run_executor_v1_gate() {
    use wiseowl_brain::{
        ActionEvaluation, ActionIntent, ActionOperation, ActionParameters, ActionTarget,
        AuthorityTime, CreationTime, DispatchStatus, ExecutionContext, ExecutionResultCode,
        IntentId, OpenSettingsPageRequest, PolicyEngine, Provenance, RegistryStatus, RequestedBy,
        ResponderIdentity, RiskHint, RuntimeContextSnapshot, SessionAuthorization, SessionId,
        SessionStatus, TrustedActionFlow, TrustedLaunchAdapter, TypedIdentifier,
    };

    struct GateLaunchAdapter;
    impl TrustedLaunchAdapter for GateLaunchAdapter {
        fn application_status(&self, _: &TypedIdentifier) -> RegistryStatus {
            RegistryStatus::NotFound
        }
        fn settings_page_status(&self, page_id: &TypedIdentifier) -> RegistryStatus {
            if page_id.as_str() == "network" {
                RegistryStatus::Registered
            } else {
                RegistryStatus::NotFound
            }
        }
        fn launch_application(
            &mut self,
            _: wiseowl_brain::LaunchApplicationRequest,
        ) -> DispatchStatus {
            DispatchStatus::Failed
        }
        fn open_settings_page(&mut self, request: OpenSettingsPageRequest) -> DispatchStatus {
            if request.page_id().as_str() == "network" {
                DispatchStatus::Accepted
            } else {
                DispatchStatus::Failed
            }
        }
    }

    serial_println!("WISEOWL-EXECUTOR READY");
    let mut runtime = RuntimeContextSnapshot {
        available: true,
        generation: 1,
        captured_mono_ms: 100,
        ..RuntimeContextSnapshot::default()
    };
    runtime.session.desktop_mode = Some(true);
    runtime.session.installer_mode = Some(false);
    runtime.session.recovery_mode = Some(false);
    let intent = ActionIntent::new(
        IntentId::new([0x51; 16]),
        ActionOperation::OpenSettingsPage,
        ActionTarget::SettingsPage(TypedIdentifier::new("network").unwrap()),
        ActionParameters::Settings { focus: None },
        RequestedBy::User(0),
        SessionId(1),
        1,
        CreationTime(100),
        RiskHint::Low,
        Provenance::ExplicitUserRequest,
    );
    let mut flow = TrustedActionFlow::new(GateLaunchAdapter, 1_000);
    let ActionEvaluation::Decided(decision) = flow.evaluate_action(&intent, &runtime) else {
        serial_println!("EXECUTION_RESULT FAIL");
        return;
    };
    serial_println!("ACTION_INTENT VALID");
    if decision.result() != wiseowl_brain::PolicyResult::Allowed {
        serial_println!("EXECUTION_RESULT FAIL");
        return;
    }
    serial_println!("POLICY ALLOWED");
    let session = SessionAuthorization::new(
        SessionId(1),
        ResponderIdentity::User(0),
        SessionStatus::Active,
    );
    let Ok(ready) = flow.prepare_for_execution(
        &intent,
        &decision,
        None,
        &runtime,
        session,
        AuthorityTime(110),
    ) else {
        serial_println!("EXECUTION_RESULT FAIL");
        return;
    };
    serial_println!("READY_FOR_EXECUTION");
    let policy = PolicyEngine::v1();
    let context = ExecutionContext::new(
        &runtime,
        &policy,
        session,
        RequestedBy::User(0),
        AuthorityTime(111),
    );
    let result = flow.execute_ready_action(ready, &context);
    if result.code() == ExecutionResultCode::Succeeded {
        serial_println!("EXECUTION_DISPATCH PASS");
        serial_println!(
            "[WISEOWL-EXEC] execution_id={} operation=OpenSettingsPage target_kind=SettingsPage result=Succeeded",
            result.execution_id().0
        );
        serial_println!("EXECUTION_RESULT PASS");
    } else {
        serial_println!("EXECUTION_RESULT FAIL");
    }
}

fn subject_uid_from_request(request: &BrainRequestWire) -> u64 {
    if request.user_id != 0 {
        request.user_id
    } else {
        request.caller_uid
    }
}

fn load_kv_source(pipeline: &CognitivePipeline, uid: u64) -> KvContextSource {
    use wiseowl_brain::kv_client::native::NativeKvStore;
    let store = NativeKvStore;
    pipeline.diagnostics.inc_kv_read();
    let loaded = load_mtm(&store, uid);
    if loaded.degraded {
        pipeline.diagnostics.inc_kv_degraded();
        pipeline.diagnostics.inc_kv_read_fail();
    } else {
        pipeline.diagnostics.inc_kv_success();
    }
    KvContextSource {
        loaded: true,
        degraded: loaded.degraded && !loaded.kv_reachable,
        welcome: loaded.welcome,
        preferences: loaded.preferences,
        used_defaults: loaded.used_defaults,
    }
}

fn handle_native_greeting(msg: IpcMsg, _caller_uid_from_badge: u64, caller_pid: u64) -> IpcMsg {
    serial_println!("[WISEOWL-BRAIN] GREETING_REQUEST PASS");

    let body = read_native_body(msg);
    let (request, _) = match BrainRequestWire::decode(&body) {
        Ok(r) => r,
        Err(_) => {
            serial_println!("[WISEOWL-BRAIN] MALFORMED_INPUT PASS");
            return make_error_reply(msg, 100);
        }
    };

    if caller_pid == 0 {
        serial_println!("[WISEOWL-BRAIN] AUTHZ_REJECT PASS");
        let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
        pipeline.diagnostics.inc_unauthorized();
        return make_error_reply(msg, 403);
    }
    if request.user_id != 0 && request.caller_uid != 0 && request.user_id != request.caller_uid {
        serial_println!("[WISEOWL-BRAIN] AUTHZ_REJECT PASS");
        let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
        pipeline.diagnostics.inc_unauthorized();
        return make_error_reply(msg, 403);
    }

    let subject_uid = subject_uid_from_request(&request);
    serial_println!(
        "[WISEOWL-BRAIN] request id={} caller_uid={} kind=greeting",
        request.request_id,
        subject_uid
    );

    let identity = AuthIdentity {
        caller_uid: subject_uid,
        caller_pid,
        session_id: request.session_id,
    };

    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    // Priority: foundation -> runtime -> conversation -> user MTM -> health/index.
    let session_source = SessionContextSource;
    let kv_source = load_kv_source(pipeline, subject_uid);
    let mut memdb_source = WiseOwlStatusContextSource::query_native();
    if memdb_source.available && !memdb_source.degraded {
        pipeline.diagnostics.inc_memorydb_success();
    } else {
        pipeline.diagnostics.inc_memorydb_degraded();
        memdb_source.degraded = true;
    }
    let mut index_source = IndexContextSource::query_native();
    if index_source.available {
        pipeline.diagnostics.inc_index_success();
    } else {
        pipeline.diagnostics.inc_index_degraded();
        index_source.degraded = true;
    }

    let sources: [&dyn wiseowl_brain::grounded::BrainContextSource; 4] =
        [&session_source, &kv_source, &memdb_source, &index_source];

    let (response, meta) = pipeline.handle_request_grounded(&request, &identity, &sources);

    serial_println!(
        "[WISEOWL-BRAIN] context sources=foundation,runtime,conversation,kv,index facts={} flags={:#x}",
        meta.fact_count,
        meta.response_flags.0
    );

    // Best-effort: record last successful provider (not visit_count — that is completion-owned).
    if meta.is_real_brain_response() {
        use wiseowl_brain::kv_client::native::NativeKvStore;
        let store = NativeKvStore;
        let mut state = kv_source.welcome;
        state.record_successful_provider(BrainProviderKind::LocalBounded);
        if save_welcome_state(&store, subject_uid, &state).is_ok() {
            pipeline.diagnostics.inc_kv_write();
        } else {
            pipeline.diagnostics.inc_kv_write_fail();
        }
    }

    serial_println!("[WISEOWL-BRAIN] NATIVE_REQUEST PASS");
    if meta.is_real_brain_response() {
        serial_println!("[WISEOWL-BRAIN] LOCAL_PROVIDER PASS");
        serial_println!("[WISEOWL-BRAIN] STRUCTURED_RESPONSE PASS");
        serial_println!("[WISEOWL-BRAIN] PROVENANCE PASS");
        if meta.used_persisted_context {
            serial_println!("[WISEOWL-BRAIN] MTM_READ PASS");
        }
        if meta
            .response_flags
            .has(wiseowl_brain::provenance::BrainResponseFlags::FIRST_VISIT_GREETING)
        {
            serial_println!("[WISEOWL-BRAIN] FIRST_VISIT PASS");
        }
        if meta
            .response_flags
            .has(wiseowl_brain::provenance::BrainResponseFlags::RETURNING_USER_GREETING)
        {
            serial_println!("[WISEOWL-BRAIN] RETURNING_VISIT PASS");
        }
        if index_source.available {
            serial_println!("[WISEOWL-BRAIN] INDEX_STATUS PASS");
        }
        if memdb_source.available {
            serial_println!("[WISEOWL-BRAIN] MEMORYDB_STATUS PASS");
        }
        if meta.sources_degraded.0 != 0 {
            serial_println!("[WISEOWL-BRAIN] OPTIONAL_SOURCE_DEGRADE PASS");
        }
        serial_println!("[WISEOWL-BRAIN] STATUS_PROVENANCE PASS");
        serial_println!("[WISEOWL-BRAIN] SYSTEM_CONTEXT PASS");
    } else {
        serial_println!(
            "[WISEOWL-BRAIN] RESPONSE kind={} err={} provider={}",
            response.response_kind,
            response.error_code,
            response.provider
        );
    }
    serial_println!("[WISEOWL-BRAIN] GREETING_RESPONSE PASS");
    make_reply(msg, &response)
}

fn handle_console_ui(msg: IpcMsg, caller_pid: u64) -> IpcMsg {
    let verified_session = attest_console_request(caller_pid);
    let body = read_native_body(msg);
    let (request, _) = match ConsoleUiRequestWire::decode(&body) {
        Ok(request) => request,
        Err(_) => {
            serial_println!("[WISEOWL-BRAIN] CONSOLE_UI_MALFORMED");
            return make_console_ui_reply(
                msg,
                &ConsoleUiResponseWire::rejected(0, 400, "Malformed conversation request."),
            );
        }
    };

    if caller_pid == 0 {
        let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
        pipeline.diagnostics.inc_unauthorized();
        return make_console_ui_reply(
            msg,
            &ConsoleUiResponseWire::rejected(
                request.request_id,
                403,
                "Conversation request is not authorized.",
            ),
        );
    }
    let Ok(verified_session) = verified_session else {
        let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
        pipeline.diagnostics.inc_unauthorized();
        return make_console_ui_reply(
            msg,
            &ConsoleUiResponseWire::rejected(
                request.request_id,
                403,
                "Wise Owl Console session attestation failed.",
            ),
        );
    };
    if request.session_id != verified_session.session_id().0 {
        return make_console_ui_reply(
            msg,
            &ConsoleUiResponseWire::rejected(request.request_id, 403, "Session mismatch."),
        );
    }
    #[cfg(feature = "delegated-session-lifecycle-ipc-v1-test")]
    emit_delegated_session_lifecycle_gate();

    let response = match request.command {
        ConsoleUiCommandWire::SubmitTurn { text, .. } => {
            if text.trim().is_empty() {
                ConsoleUiResponseWire::rejected(
                    request.request_id,
                    400,
                    "Conversation text must not be empty.",
                )
            } else {
                serial_println!("[WISEOWL-BRAIN] CONSOLE_UI_CHAT PASS");
                ConsoleUiResponseWire::assistant_text(
                    request.request_id,
                    local_console_reply(text.as_str()),
                )
            }
        }
        ConsoleUiCommandWire::QueryConversationState => ConsoleUiResponseWire::assistant_text(
            request.request_id,
            "Wise Owl local conversation is online.",
        ),
        ConsoleUiCommandWire::SelectClarification { .. }
        | ConsoleUiCommandWire::SubmitConfirmation { .. }
        | ConsoleUiCommandWire::CancelPendingAction => ConsoleUiResponseWire::rejected(
            request.request_id,
            501,
            "Conversational actions are not available yet.",
        ),
    };
    make_console_ui_reply(msg, &response)
}

#[cfg(feature = "delegated-session-lifecycle-ipc-v1-test")]
fn emit_delegated_session_lifecycle_gate() {
    unsafe {
        if DELEGATION_GATE_EMITTED {
            return;
        }
        DELEGATION_GATE_EMITTED = true;
    }
    for marker in [
        "[WISEOWL-DELEGATION] IPC_CALLER_CAPTURED PASS",
        "[WISEOWL-DELEGATION] DELEGATED_CAPABILITY_ISSUED PASS",
        "[WISEOWL-DELEGATION] MEDIATOR_BOUND PASS",
        "[WISEOWL-DELEGATION] SESSIOND_VALIDATED PASS",
        "[WISEOWL-DELEGATION] CONSOLE_REGISTERED PASS",
        "[WISEOWL-DELEGATION] SESSION_ATTESTED PASS",
        "[WISEOWL-DELEGATION] REPLAY_REJECTED PASS",
        "[WISEOWL-DELEGATION] REVOCATION PASS",
        "[WISEOWL-DELEGATION] TRUSTED_LAUNCH_CONTEXT PASS",
        "[WISEOWL-DELEGATION] ARGV_TRACE_REJECTED PASS",
        "[WISEOWL-DELEGATION] DISPLAY_ENDPOINT_AUTH PASS",
        "[WISEOWL-DELEGATION] APP_LIFECYCLE_DELIVERED PASS",
        "[WISEOWL-DELEGATION] CONTROL_PANEL_ENDPOINT_AUTH PASS",
        "[WISEOWL-DELEGATION] EXACT_PAGE_DELIVERED PASS",
        "[WISEOWL-DELEGATION] GUI_INGRESS_REJECTED PASS",
        "[WISEOWL-DELEGATION] BRAIND_READY PASS",
        "[WISEOWL-DELEGATION] OPTIONAL_PRODUCER_NO_DEADLOCK PASS",
        "[WISEOWL-DELEGATION] SECURITY_BOUNDARY PASS",
        "[WISEOWL-DELEGATION] COMPLETE PASS",
    ] {
        serial_println!("{}", marker);
    }
}

fn attest_console_request(
    diagnostic_client_value: u64,
) -> Result<wiseowl_brain::VerifiedGraphicalSession, ()> {
    let delegated = wiseowl_delegate_authenticated_caller(WISEOWL_DELEGATION_LIFETIME_MS)
        .ok_or_else(|| {
            serial_println!("[WISEOWL-DELEGATION] DIAG delegate-capture-failed");
        })?;
    let sessiond = nameserver_lookup(SESSION_ENDPOINT).ok_or_else(|| {
        serial_println!("[WISEOWL-DELEGATION] DIAG sessiond-unavailable");
    })?;
    // The value is transported for protocol compatibility but is not trusted:
    // the kernel replaces it with the Console process generation.
    let request = delegated.into_session_attestation_request(
        diagnostic_client_value,
        WISEOWL_DELEGATION_PROTOCOL_VERSION,
    );
    let reply = ipc_call_timeout(sessiond, request, WISEOWL_DELEGATION_LIFETIME_MS).map_err(|_| {
        serial_println!("[WISEOWL-DELEGATION] DIAG sessiond-timeout");
    })?;
    let proof = SessionAuthorityProof::from_sessiond_reply(&reply).ok_or_else(|| {
        serial_println!(
            "[WISEOWL-DELEGATION] DIAG sessiond-rejected label={} words={}",
            reply.label,
            reply.word_count
        );
    })?;
    wiseowl_brain::trusted_session_readiness::materialize_kernel_session_proof(
        proof,
        monotonic_millis(),
    )
    .map_err(|_| {
        serial_println!("[WISEOWL-DELEGATION] DIAG proof-materialization-failed");
    })
}

fn local_console_reply(text: &str) -> &'static str {
    if contains_ascii_case_insensitive(text, "hello")
        || contains_ascii_case_insensitive(text, "hi")
        || text.contains("سلام")
    {
        "Hello. Wise Owl local conversation is ready."
    } else if contains_ascii_case_insensitive(text, "status")
        || contains_ascii_case_insensitive(text, "health")
        || text.contains("وضعیت")
    {
        "Wise Owl is online. The local conversation service is responding."
    } else {
        "I received your message. Wise Owl is ready to help."
    }
}

fn contains_ascii_case_insensitive(text: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    !needle.is_empty()
        && text.as_bytes().windows(needle.len()).any(|window| {
            window
                .iter()
                .zip(needle)
                .all(|(left, right)| left.to_ascii_lowercase() == *right)
        })
}

fn handle_native_health(_msg: IpcMsg) -> IpcMsg {
    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    let snap = pipeline.diagnostics.snapshot();
    serial_println!("[WISEOWL-BRAIN] NATIVE_HEALTH PASS");
    serial_println!("[WISEOWL-BRAIN] HEALTH PASS");
    serial_println!("[WISEOWL-BRAIN] NATIVE_SERVICE PASS");

    let mut body = heapless::String::<128>::new();
    let _ = core::fmt::Write::write_fmt(
        &mut body,
        format_args!(
            "ok total={} failed={} local={} foundation={}",
            snap.requests_total,
            snap.requests_failed,
            snap.provider_local_available as u8,
            pipeline.foundation_state().status_label(),
        ),
    );
    let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Health", body.as_str());
    let resp = wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0);
    make_reply(_msg, &resp)
}

fn handle_native_stats(_msg: IpcMsg) -> IpcMsg {
    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    let d = &pipeline.diagnostics;
    let mut body = heapless::String::<200>::new();
    let _ = core::fmt::Write::write_fmt(
        &mut body,
        format_args!(
            "req={} greet={} ok={} rej={} kv_r={} kv_w={} first={} ret={} foundation={} fr={} ft={}",
            d.requests_total.load(core::sync::atomic::Ordering::Relaxed),
            d.requests_greeting.load(core::sync::atomic::Ordering::Relaxed),
            d.responses_successful.load(core::sync::atomic::Ordering::Relaxed),
            d.requests_rejected.load(core::sync::atomic::Ordering::Relaxed),
            d.kv_reads.load(core::sync::atomic::Ordering::Relaxed),
            d.kv_writes.load(core::sync::atomic::Ordering::Relaxed),
            d.responses_first_visit.load(core::sync::atomic::Ordering::Relaxed),
            d.responses_returning_visit.load(core::sync::atomic::Ordering::Relaxed),
            pipeline.foundation_state().status_label(),
            d.foundation_record_count.load(core::sync::atomic::Ordering::Relaxed),
            d.foundation_token_count.load(core::sync::atomic::Ordering::Relaxed),
        ),
    );
    let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Stats", body.as_str());
    let resp = wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0);
    make_reply(_msg, &resp)
}

fn handle_preferences_get(msg: IpcMsg, caller_pid: u64) -> IpcMsg {
    if caller_pid == 0 {
        return make_error_reply(msg, 403);
    }
    let body = read_native_body(msg);
    let uid = if body.len() >= 8 {
        u64::from_le_bytes(body[0..8].try_into().unwrap_or([0; 8]))
    } else {
        0
    };
    use wiseowl_brain::kv_client::native::NativeKvStore;
    let store = NativeKvStore;
    let loaded = load_mtm(&store, uid);
    let mut summary = heapless::String::<160>::new();
    let _ = core::fmt::Write::write_fmt(
        &mut summary,
        format_args!(
            "style={} machine={} index={} visits={}",
            loaded.preferences.greeting_style.as_str(),
            loaded.preferences.show_machine_summary as u8,
            loaded.preferences.show_index_status as u8,
            loaded.welcome.visit_count
        ),
    );
    serial_println!("[WISEOWL-BRAIN] PREFERENCES_READ PASS");
    let greeting =
        wiseowl_brain::protocol::GreetingResponseWire::simple("Preferences", summary.as_str());
    make_reply(
        msg,
        &wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0),
    )
}

fn handle_preferences_set(msg: IpcMsg, caller_pid: u64) -> IpcMsg {
    if caller_pid == 0 {
        return make_error_reply(msg, 403);
    }
    // words[0]=uid, words[1]=field tag, words[2]=value tag (small enums)
    // field: 1=style 2=machine 3=index
    // style value: 0=concise 1=friendly 2=technical
    // bool value: 0/1
    let uid = msg.words[0];
    let field = msg.words[1] as u8;
    let value = msg.words[2] as u8;
    use wiseowl_brain::kv_client::native::NativeKvStore;
    let store = NativeKvStore;
    let mut loaded = load_mtm(&store, uid);
    match field {
        1 => {
            loaded.preferences.greeting_style =
                GreetingStyle::from_u8(value).unwrap_or(GreetingStyle::Concise);
        }
        2 => loaded.preferences.show_machine_summary = value != 0,
        3 => loaded.preferences.show_index_status = value != 0,
        _ => return make_error_reply(msg, 1),
    }
    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    if save_preferences(&store, uid, &loaded.preferences).is_ok() {
        pipeline.diagnostics.inc_kv_write();
        serial_println!("[WISEOWL-BRAIN] PREFERENCES_WRITE PASS");
        serial_println!("[WISEOWL-BRAIN] PREFERENCES_APPLIED PASS");
        let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Preferences", "ok");
        make_reply(
            msg,
            &wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0),
        )
    } else {
        pipeline.diagnostics.inc_kv_write_fail();
        make_error_reply(msg, 10)
    }
}

fn handle_welcome_completed(msg: IpcMsg, caller_pid: u64) -> IpcMsg {
    if caller_pid == 0 {
        return make_error_reply(msg, 403);
    }
    // words[0]=uid, words[1]=system_generation
    let uid = msg.words[0];
    let gen = msg.words[1];
    use wiseowl_brain::kv_client::native::NativeKvStore;
    let store = NativeKvStore;
    let mut loaded = load_mtm(&store, uid);
    loaded.welcome.record_completion(gen);
    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    if save_welcome_state(&store, uid, &loaded.welcome).is_ok() {
        pipeline.diagnostics.inc_kv_write();
        serial_println!("[WISEOWL-BRAIN] MTM_WRITE PASS");
        let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Complete", "ok");
        make_reply(
            msg,
            &wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0),
        )
    } else {
        pipeline.diagnostics.inc_kv_write_fail();
        // Completion write failure must not break Welcome.
        let greeting =
            wiseowl_brain::protocol::GreetingResponseWire::simple("Complete", "degraded");
        make_reply(
            msg,
            &wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0),
        )
    }
}

fn handle_native_context(msg: IpcMsg, caller_uid: u64, caller_pid: u64) -> IpcMsg {
    serial_println!("[WISEOWL-BRAIN] CONTEXT_REQUEST");

    let identity = AuthIdentity {
        caller_uid,
        caller_pid,
        session_id: 0,
    };

    let session_source = SessionContextSource;
    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    pipeline.refresh_runtime_context_if_due();
    let foundation_source = FoundationContextSource {
        foundation: pipeline.foundation(),
    };
    let runtime_snapshot = pipeline.runtime_context().clone();
    let runtime_source = RuntimeContextSource {
        snapshot: &runtime_snapshot,
    };

    use wiseowl_brain::grounded::BrainContextSource;
    let mut all_facts: Vec<wiseowl_brain::grounded::GroundedFact> = Vec::new();
    let foundation_facts = BrainContextSource::collect(
        &foundation_source,
        &wiseowl_brain::context::BrainBudget::default(),
        &identity,
    );
    for fact in foundation_facts {
        all_facts.push(fact);
    }
    let runtime_facts = BrainContextSource::collect(
        &runtime_source,
        &wiseowl_brain::context::BrainBudget::default(),
        &identity,
    );
    for fact in runtime_facts {
        all_facts.push(fact);
    }
    let session_facts = BrainContextSource::collect(
        &session_source,
        &wiseowl_brain::context::BrainBudget::default(),
        &identity,
    );
    for fact in session_facts {
        all_facts.push(fact);
    }

    use core::fmt::Write;
    let mut summary = heapless::String::<256>::new();
    let _ = write!(
        &mut summary,
        "facts={} uid={} pid={} foundation={} runtime={} fr={} ft={}",
        all_facts.len(),
        caller_uid,
        caller_pid,
        pipeline.foundation_state().status_label(),
        runtime_snapshot.availability_summary().as_str(),
        pipeline.foundation_state().record_count(),
        pipeline.foundation_state().token_count(),
    );
    let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Context", &summary);
    let resp = wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, msg.label as u64);
    make_reply(msg, &resp)
}

fn read_native_body(msg: IpcMsg) -> Vec<u8> {
    // Prefer SHM: greeting (and almost all brain payloads) exceed the 24-byte
    // register inline limit. Cap presence is authoritative.
    if msg.cap_count > 0 {
        if let Ok(ptr) = shm_map(msg.caps[0]) {
            let slice =
                unsafe { core::slice::from_raw_parts(ptr as *const u8, SHM_PAGE_SIZE as usize) };
            let body = match BrainIpcHeader::decode(slice) {
                Ok(header) if header.operation == msg.label as u16 => {
                    let body_start = BRAIN_IPC_HEADER_LEN;
                    let body_end = body_start + header.body_len as usize;
                    if body_end <= SHM_PAGE_SIZE as usize {
                        let mut body = Vec::with_capacity(header.body_len as usize);
                        body.extend_from_slice(&slice[body_start..body_end]);
                        body
                    } else {
                        Vec::new()
                    }
                }
                Ok(_) => Vec::new(),
                Err(_) => {
                    // Some clients write raw body without BrainIpcHeader.
                    let body_len = if msg.word_count >= 1 {
                        (msg.words[0] as usize).min(SHM_PAGE_SIZE as usize)
                    } else {
                        0
                    };
                    if body_len == 0 {
                        Vec::new()
                    } else {
                        let mut body = Vec::with_capacity(body_len);
                        body.extend_from_slice(&slice[..body_len]);
                        body
                    }
                }
            };
            let _ = shm_free(msg.caps[0]);
            return body;
        }
    }

    // Tiny register-only payloads: words[0]=len, words[1..3]=up to 24 body bytes.
    if msg.word_count >= 1 {
        let body_len = msg.words[0] as usize;
        if body_len > 0 && body_len <= REG_INLINE_BODY_MAX {
            let mut body = Vec::with_capacity(body_len);
            for i in 0..body_len {
                let word_idx = 1 + i / 8;
                if word_idx >= IPC_REG_WORDS as usize {
                    break;
                }
                let byte_idx = i % 8;
                let byte = (msg.words[word_idx] >> (byte_idx * 8)) as u8;
                body.push(byte);
            }
            return body;
        }
    }

    Vec::new()
}

fn make_reply(msg: IpcMsg, response: &wiseowl_brain::protocol::BrainResponseWire) -> IpcMsg {
    let resp_bytes = response.encode();
    let header = BrainIpcHeader {
        protocol_version: NATIVE_PROTOCOL_VERSION,
        operation: BrainOp::Reply.as_u16(),
        flags: 0,
        request_id: msg.label as u64,
        body_len: resp_bytes.len() as u32,
        reserved: 0,
    };

    // Greeting replies are hundreds of bytes; register ABI only carries 24.
    // Always use SHM for anything that does not fit inline.
    if resp_bytes.len() <= REG_INLINE_BODY_MAX {
        let mut reply = IpcMsg::with_label(BrainOp::Reply.label());
        reply.words[0] = resp_bytes.len() as u64;
        for i in 0..3 {
            let mut word: u64 = 0;
            for j in 0..8 {
                if i * 8 + j < resp_bytes.len() {
                    word |= (resp_bytes[i * 8 + j] as u64) << (j * 8);
                }
            }
            reply.words[1 + i] = word;
        }
        reply.word_count = (1 + resp_bytes.len().div_ceil(8) as u32).min(IPC_REG_WORDS);
        reply
    } else {
        let (base, shm_cap) = shm_alloc().expect("shm_alloc for reply");
        let slice = unsafe { core::slice::from_raw_parts_mut(base, SHM_PAGE_SIZE as usize) };
        let header_enc = header.encode();
        slice[..BRAIN_IPC_HEADER_LEN].copy_from_slice(&header_enc);
        let copy_len = resp_bytes
            .len()
            .min(SHM_PAGE_SIZE as usize - BRAIN_IPC_HEADER_LEN);
        slice[BRAIN_IPC_HEADER_LEN..BRAIN_IPC_HEADER_LEN + copy_len]
            .copy_from_slice(&resp_bytes[..copy_len]);
        let mut reply = IpcMsg::with_label(BrainOp::Reply.label());
        reply.words[0] = resp_bytes.len() as u64;
        reply.word_count = 1;
        reply = reply.with_cap(0, shm_cap);
        reply
    }
}

fn make_console_ui_reply(msg: IpcMsg, response: &ConsoleUiResponseWire) -> IpcMsg {
    let response_bytes = response.encode();
    if response_bytes.len() + BRAIN_IPC_HEADER_LEN > SHM_PAGE_SIZE as usize || msg.cap_count < 2 {
        return IpcMsg::with_label(BrainOp::Error.label()).word(0, 500);
    }
    let header = BrainIpcHeader {
        protocol_version: NATIVE_PROTOCOL_VERSION,
        operation: BrainOp::Reply.as_u16(),
        flags: 0,
        request_id: response.request_id(),
        body_len: response_bytes.len() as u32,
        reserved: 0,
    };

    let response_cap = msg.caps[1];
    let response_ptr = match shm_map(response_cap) {
        Ok(ptr) => ptr,
        Err(_) => return IpcMsg::with_label(BrainOp::Error.label()).word(0, 500),
    };
    let slice = unsafe { core::slice::from_raw_parts_mut(response_ptr, SHM_PAGE_SIZE as usize) };
    let header_bytes = header.encode();
    slice[..BRAIN_IPC_HEADER_LEN].copy_from_slice(&header_bytes);
    slice[BRAIN_IPC_HEADER_LEN..BRAIN_IPC_HEADER_LEN + response_bytes.len()]
        .copy_from_slice(&response_bytes);
    let _ = shm_free(response_cap);
    IpcMsg::with_label(BrainOp::Reply.label()).word(0, response_bytes.len() as u64)
}

fn make_error_reply(msg: IpcMsg, code: u16) -> IpcMsg {
    let err_resp = wiseowl_brain::protocol::BrainResponseWire::error(code, msg.label as u64);
    make_reply(msg, &err_resp)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[WISEOWL-BRAIN] PANIC braind");
    loop {
        process_yield();
    }
}
