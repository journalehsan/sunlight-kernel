//! Bounded delegated-session lifecycle contracts.
//!
//! This module owns only authority adapters and opaque launch correlation.
//! Diagnostic argv, environment, log, process-name, title and PID fields have
//! no conversion into these types.

use heapless::{Deque, Vec};

use crate::gui_bridge::{
    CorrelatedGuiReadinessEvidence, GuiReadinessEvidenceId, TrustedGuiReadinessKind,
    TrustedReadinessSource,
};
use crate::trusted_session_readiness::TrustedReadinessIngress;
use crate::{
    ExecutionId, ExecutionResult, LaunchCorrelationToken, ObservationEvidence, SessionId,
    TypedIdentifier,
};

pub const MAX_LAUNCH_CONTEXTS: usize = 16;
pub const MAX_LIFECYCLE_EVENTS_PER_EXECUTION: usize = 8;
pub const MAX_DISPLAY_SOURCE_CONNECTIONS: usize = 1;
pub const MAX_CONTROL_PANEL_SOURCE_CONNECTIONS: usize = 1;
pub const MAX_SOURCE_SEQUENCE_RECORDS: usize = 2;
pub const MAX_EXPIRED_CONTEXT_TOMBSTONES: usize = 32;
pub const MAX_COMPONENT_REGISTRATION_RECORDS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleTargetKind {
    Application,
    SettingsPage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayLifecycleEventKind {
    ApplicationInstanceRegistered,
    FirstTopLevelSurfaceRegistered,
    ApplicationReadinessSatisfied,
    ApplicationExitedBeforeReadiness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPanelLifecycleEventKind {
    InstanceRegistered,
    CanonicalPageActivated,
    CanonicalPageReady,
    ExitedBeforePageReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleIngressError {
    UnauthorizedSource,
    ContextNotFound,
    ContextExpired,
    ContextConsumed,
    WrongSourceGeneration,
    WrongSession,
    WrongTarget,
    WrongApplicationInstance,
    DuplicateOrOutOfOrder,
    ReadinessContractNotSatisfied,
    NavigationRequestIsNotActivation,
    CapacityExhausted,
}

/// Opaque context delivered by a trusted launcher registration attachment.
/// It deliberately exposes no raw ID, fields, Debug, Clone or Copy surface.
pub struct TrustedLaunchContext {
    opaque_id: u64,
    integrity_tag: u64,
}

struct LaunchContextRecord {
    opaque_id: u64,
    integrity_tag: u64,
    execution_id: ExecutionId,
    canonical_target: TypedIdentifier,
    target_kind: LifecycleTargetKind,
    graphical_session: SessionId,
    application_instance: u64,
    registry_generation: u64,
    launch_attempt: u16,
    source_generation: u64,
    correlation_token: LaunchCorrelationToken,
    event_count: u8,
    expires_at: u64,
    terminal: bool,
}

struct SourceCapability {
    source: SourceService,
    generation: u64,
    nonce: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceService {
    Display,
    ControlPanel,
}

#[derive(Clone, Copy)]
struct SourceSequenceRecord {
    source: SourceService,
    generation: u64,
    last_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedLifecycleEvent {
    pub execution_id: ExecutionId,
    pub session_id: SessionId,
    pub target: TypedIdentifier,
    pub sequence: u64,
    pub source_generation: u64,
    pub display_kind: Option<DisplayLifecycleEventKind>,
    pub control_panel_kind: Option<ControlPanelLifecycleEventKind>,
    timestamp: u64,
    correlation_token: LaunchCorrelationToken,
}

/// Braind-owned bounded adapter. Production source capabilities are installed
/// only after kernel-authenticated service identity checks at the dedicated
/// endpoints; there is no plugin/source registration API.
pub struct BraindTrustedLifecycleAdapters {
    contexts: Vec<LaunchContextRecord, MAX_LAUNCH_CONTEXTS>,
    sequences: Vec<SourceSequenceRecord, MAX_SOURCE_SEQUENCE_RECORDS>,
    accepted: Deque<AcceptedLifecycleEvent, MAX_LAUNCH_CONTEXTS>,
    tombstones: Deque<u64, MAX_EXPIRED_CONTEXT_TOMBSTONES>,
    next_id: u64,
    display: Option<SourceCapability>,
    control_panel: Option<SourceCapability>,
}

impl BraindTrustedLifecycleAdapters {
    pub const fn new() -> Self {
        Self {
            contexts: Vec::new(),
            sequences: Vec::new(),
            accepted: Deque::new(),
            tombstones: Deque::new(),
            next_id: 1,
            display: None,
            control_panel: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register_launch_context(
        &mut self,
        execution_id: ExecutionId,
        canonical_target: TypedIdentifier,
        target_kind: LifecycleTargetKind,
        graphical_session: SessionId,
        application_instance: u64,
        registry_generation: u64,
        launch_attempt: u16,
        source_generation: u64,
        correlation_token: LaunchCorrelationToken,
        now: u64,
        expires_at: u64,
    ) -> Result<TrustedLaunchContext, LifecycleIngressError> {
        if self.contexts.is_full()
            || application_instance == 0
            || launch_attempt == 0
            || source_generation == 0
            || expires_at <= now
        {
            return Err(LifecycleIngressError::CapacityExhausted);
        }
        let opaque_id = self.next_id;
        self.next_id = self.next_id.saturating_add(1).max(1);
        let integrity_tag = opaque_id
            ^ execution_id.0.rotate_left(17)
            ^ graphical_session.0.rotate_left(31)
            ^ source_generation.rotate_left(47)
            ^ 0x574f_4c43_5458_5631;
        self.contexts
            .push(LaunchContextRecord {
                opaque_id,
                integrity_tag,
                execution_id,
                canonical_target,
                target_kind,
                graphical_session,
                application_instance,
                registry_generation,
                launch_attempt,
                source_generation,
                correlation_token,
                event_count: 0,
                expires_at,
                terminal: false,
            })
            .map_err(|_| LifecycleIngressError::CapacityExhausted)?;
        Ok(TrustedLaunchContext {
            opaque_id,
            integrity_tag,
        })
    }

    pub fn authenticate_display(&mut self, generation: u64) -> Result<(), LifecycleIngressError> {
        if generation == 0 {
            return Err(LifecycleIngressError::CapacityExhausted);
        }
        self.display = Some(SourceCapability {
            source: SourceService::Display,
            generation,
            nonce: generation ^ 0x4453_504c_4159,
        });
        Ok(())
    }

    pub fn authenticate_control_panel(
        &mut self,
        generation: u64,
    ) -> Result<(), LifecycleIngressError> {
        if generation == 0 {
            return Err(LifecycleIngressError::CapacityExhausted);
        }
        self.control_panel = Some(SourceCapability {
            source: SourceService::ControlPanel,
            generation,
            nonce: generation ^ 0x4350_414e_454c,
        });
        Ok(())
    }

    /// Dedicated display endpoint adapter. The caller has already been
    /// authenticated by the braind-only kernel syscall; opaque context fields
    /// are resolved against the executor-installed bounded table here.
    pub fn ingest_authenticated_display_wire(
        &mut self,
        source_generation: u64,
        opaque_id: u64,
        integrity_tag: u64,
        sequence: u64,
        now: u64,
        kind: DisplayLifecycleEventKind,
    ) -> Result<(), LifecycleIngressError> {
        let record = self
            .contexts
            .iter()
            .find(|record| record.opaque_id == opaque_id && record.integrity_tag == integrity_tag)
            .ok_or(LifecycleIngressError::ContextNotFound)?;
        let session = record.graphical_session;
        let target = record.canonical_target.clone();
        let instance = record.application_instance;
        let context = TrustedLaunchContext {
            opaque_id,
            integrity_tag,
        };
        self.ingest_display(
            &context,
            session,
            &target,
            instance,
            source_generation,
            sequence,
            now,
            true,
            kind,
        )
    }

    /// Dedicated Control Panel endpoint adapter. Canonical page and session
    /// are obtained from the trusted settings launch context, never a title,
    /// argv page name, or navigation request payload.
    pub fn ingest_authenticated_control_panel_wire(
        &mut self,
        source_generation: u64,
        opaque_id: u64,
        integrity_tag: u64,
        sequence: u64,
        now: u64,
        kind: ControlPanelLifecycleEventKind,
    ) -> Result<(), LifecycleIngressError> {
        let record = self
            .contexts
            .iter()
            .find(|record| record.opaque_id == opaque_id && record.integrity_tag == integrity_tag)
            .ok_or(LifecycleIngressError::ContextNotFound)?;
        let session = record.graphical_session;
        let page = record.canonical_target.clone();
        let instance = record.application_instance;
        let context = TrustedLaunchContext {
            opaque_id,
            integrity_tag,
        };
        self.ingest_control_panel(
            &context,
            session,
            &page,
            instance,
            source_generation,
            sequence,
            now,
            true,
            kind,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ingest_display(
        &mut self,
        context: &TrustedLaunchContext,
        session_id: SessionId,
        target: &TypedIdentifier,
        application_instance: u64,
        source_generation: u64,
        sequence: u64,
        now: u64,
        registry_readiness_satisfied: bool,
        kind: DisplayLifecycleEventKind,
    ) -> Result<(), LifecycleIngressError> {
        let cap = self
            .display
            .as_ref()
            .ok_or(LifecycleIngressError::UnauthorizedSource)?;
        if cap.source != SourceService::Display
            || cap.nonce == 0
            || cap.generation != source_generation
        {
            return Err(LifecycleIngressError::WrongSourceGeneration);
        }
        self.ingest(
            SourceService::Display,
            context,
            session_id,
            target,
            application_instance,
            source_generation,
            sequence,
            now,
            matches!(
                kind,
                DisplayLifecycleEventKind::ApplicationReadinessSatisfied
            ) && !registry_readiness_satisfied,
            Some(kind),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ingest_control_panel(
        &mut self,
        context: &TrustedLaunchContext,
        session_id: SessionId,
        exact_page: &TypedIdentifier,
        control_panel_instance: u64,
        source_generation: u64,
        sequence: u64,
        now: u64,
        authoritative_active_page: bool,
        kind: ControlPanelLifecycleEventKind,
    ) -> Result<(), LifecycleIngressError> {
        let cap = self
            .control_panel
            .as_ref()
            .ok_or(LifecycleIngressError::UnauthorizedSource)?;
        if cap.source != SourceService::ControlPanel
            || cap.nonce == 0
            || cap.generation != source_generation
        {
            return Err(LifecycleIngressError::WrongSourceGeneration);
        }
        if matches!(kind, ControlPanelLifecycleEventKind::CanonicalPageActivated)
            && !authoritative_active_page
        {
            return Err(LifecycleIngressError::NavigationRequestIsNotActivation);
        }
        self.ingest(
            SourceService::ControlPanel,
            context,
            session_id,
            exact_page,
            control_panel_instance,
            source_generation,
            sequence,
            now,
            false,
            None,
            Some(kind),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn ingest(
        &mut self,
        source: SourceService,
        context: &TrustedLaunchContext,
        session_id: SessionId,
        target: &TypedIdentifier,
        application_instance: u64,
        source_generation: u64,
        sequence: u64,
        now: u64,
        readiness_contract_failed: bool,
        display_kind: Option<DisplayLifecycleEventKind>,
        control_panel_kind: Option<ControlPanelLifecycleEventKind>,
    ) -> Result<(), LifecycleIngressError> {
        if self.tombstones.iter().any(|id| *id == context.opaque_id) {
            return Err(LifecycleIngressError::ContextConsumed);
        }
        let record = self
            .contexts
            .iter_mut()
            .find(|record| {
                record.opaque_id == context.opaque_id
                    && record.integrity_tag == context.integrity_tag
            })
            .ok_or(LifecycleIngressError::ContextNotFound)?;
        if now > record.expires_at {
            return Err(LifecycleIngressError::ContextExpired);
        }
        if record.terminal {
            return Err(LifecycleIngressError::ContextConsumed);
        }
        if record.graphical_session != session_id {
            return Err(LifecycleIngressError::WrongSession);
        }
        if &record.canonical_target != target
            || (source == SourceService::Display
                && record.target_kind != LifecycleTargetKind::Application)
            || (source == SourceService::ControlPanel
                && record.target_kind != LifecycleTargetKind::SettingsPage)
        {
            return Err(LifecycleIngressError::WrongTarget);
        }
        if record.application_instance != application_instance {
            return Err(LifecycleIngressError::WrongApplicationInstance);
        }
        if record.source_generation != source_generation {
            return Err(LifecycleIngressError::WrongSourceGeneration);
        }
        if readiness_contract_failed {
            return Err(LifecycleIngressError::ReadinessContractNotSatisfied);
        }
        let sequence_record = if let Some(existing) = self
            .sequences
            .iter_mut()
            .find(|entry| entry.source == source)
        {
            existing
        } else {
            self.sequences
                .push(SourceSequenceRecord {
                    source,
                    generation: source_generation,
                    last_sequence: 0,
                })
                .map_err(|_| LifecycleIngressError::CapacityExhausted)?;
            self.sequences.last_mut().unwrap()
        };
        if sequence_record.generation != source_generation {
            return Err(LifecycleIngressError::WrongSourceGeneration);
        }
        if sequence <= sequence_record.last_sequence {
            return Err(LifecycleIngressError::DuplicateOrOutOfOrder);
        }
        if record.event_count as usize >= MAX_LIFECYCLE_EVENTS_PER_EXECUTION
            || self.accepted.is_full()
        {
            return Err(LifecycleIngressError::CapacityExhausted);
        }
        sequence_record.last_sequence = sequence;
        record.event_count = record.event_count.saturating_add(1);
        let terminal = matches!(
            display_kind,
            Some(DisplayLifecycleEventKind::ApplicationReadinessSatisfied)
                | Some(DisplayLifecycleEventKind::ApplicationExitedBeforeReadiness)
        ) || matches!(
            control_panel_kind,
            Some(ControlPanelLifecycleEventKind::CanonicalPageReady)
                | Some(ControlPanelLifecycleEventKind::ExitedBeforePageReady)
        );
        record.terminal = terminal;
        let _registry_generation = record.registry_generation;
        let _launch_attempt = record.launch_attempt;
        self.accepted
            .push_back(AcceptedLifecycleEvent {
                execution_id: record.execution_id,
                session_id,
                target: target.clone(),
                sequence,
                source_generation,
                display_kind,
                control_panel_kind,
                timestamp: now,
                correlation_token: record.correlation_token,
            })
            .map_err(|_| LifecycleIngressError::CapacityExhausted)?;
        if terminal {
            if self.tombstones.is_full() {
                self.tombstones.pop_front();
            }
            self.tombstones
                .push_back(context.opaque_id)
                .map_err(|_| LifecycleIngressError::CapacityExhausted)?;
        }
        Ok(())
    }

    pub(crate) fn next_accepted(&mut self) -> Option<AcceptedLifecycleEvent> {
        self.accepted.pop_front()
    }

    /// Delivers accepted authority evidence through the existing readiness
    /// conversion used by the Outcome Observer. There is no second observer
    /// protocol or GUI-facing evidence constructor.
    pub(crate) fn next_observer_evidence(
        &mut self,
        execution: &ExecutionResult,
    ) -> Option<ObservationEvidence> {
        let accepted = self.next_accepted()?;
        let (source, source_identity, kind) =
            match (accepted.display_kind, accepted.control_panel_kind) {
                (Some(DisplayLifecycleEventKind::ApplicationInstanceRegistered), None) => (
                    TrustedReadinessSource::ApplicationRegistry,
                    "sunlight-application-registry",
                    TrustedGuiReadinessKind::ApplicationRegistered,
                ),
                (Some(DisplayLifecycleEventKind::FirstTopLevelSurfaceRegistered), None) => (
                    TrustedReadinessSource::DisplayServer,
                    "sunlight-display",
                    TrustedGuiReadinessKind::FirstWindowRegistered,
                ),
                (Some(DisplayLifecycleEventKind::ApplicationReadinessSatisfied), None) => (
                    TrustedReadinessSource::ApplicationRegistry,
                    "sunlight-application-registry",
                    TrustedGuiReadinessKind::ApplicationReady,
                ),
                (Some(DisplayLifecycleEventKind::ApplicationExitedBeforeReadiness), None) => (
                    TrustedReadinessSource::ProcessLifecycle,
                    "sunlight-process-lifecycle",
                    TrustedGuiReadinessKind::ProcessExitedEarly,
                ),
                (None, Some(ControlPanelLifecycleEventKind::CanonicalPageActivated)) => (
                    TrustedReadinessSource::ControlPanel,
                    "sunlight-control-panel",
                    TrustedGuiReadinessKind::SettingsPageActivated,
                ),
                (None, Some(ControlPanelLifecycleEventKind::CanonicalPageReady)) => (
                    TrustedReadinessSource::ControlPanel,
                    "sunlight-control-panel",
                    TrustedGuiReadinessKind::SettingsPageReady,
                ),
                (None, Some(ControlPanelLifecycleEventKind::ExitedBeforePageReady)) => (
                    TrustedReadinessSource::ControlPanel,
                    "sunlight-control-panel",
                    TrustedGuiReadinessKind::ProcessExitedEarly,
                ),
                // Registration establishes source state but is not readiness.
                (None, Some(ControlPanelLifecycleEventKind::InstanceRegistered)) => return None,
                _ => return None,
            };
        let evidence = CorrelatedGuiReadinessEvidence::from_authenticated_lifecycle(
            GuiReadinessEvidenceId(accepted.execution_id.0.rotate_left(23) ^ accepted.sequence),
            source,
            TypedIdentifier::new(source_identity).ok()?,
            accepted.execution_id,
            accepted.session_id,
            accepted.target,
            accepted.source_generation,
            accepted.sequence,
            accepted.timestamp,
            kind,
            accepted.correlation_token,
        );
        TrustedReadinessIngress::<1, 1, 1>::into_observer_evidence(evidence, execution)
    }
}

impl Default for BraindTrustedLifecycleAdapters {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_context(adapter: &mut BraindTrustedLifecycleAdapters) -> TrustedLaunchContext {
        adapter
            .register_launch_context(
                ExecutionId(1),
                TypedIdentifier::new("calculator").unwrap(),
                LifecycleTargetKind::Application,
                SessionId(7),
                41,
                3,
                1,
                9,
                LaunchCorrelationToken::new_for_test([1; 32]),
                10,
                100,
            )
            .unwrap()
    }

    #[test]
    fn display_requires_authenticated_source_exact_context_and_readiness_contract() {
        let mut adapter = BraindTrustedLifecycleAdapters::new();
        let context = app_context(&mut adapter);
        let app = TypedIdentifier::new("calculator").unwrap();
        assert_eq!(
            adapter.ingest_display(
                &context,
                SessionId(7),
                &app,
                41,
                9,
                1,
                11,
                true,
                DisplayLifecycleEventKind::ApplicationInstanceRegistered,
            ),
            Err(LifecycleIngressError::UnauthorizedSource)
        );
        adapter.authenticate_display(9).unwrap();
        assert_eq!(
            adapter.ingest_display(
                &context,
                SessionId(8),
                &app,
                41,
                9,
                1,
                11,
                true,
                DisplayLifecycleEventKind::ApplicationInstanceRegistered,
            ),
            Err(LifecycleIngressError::WrongSession)
        );
        assert_eq!(
            adapter.ingest_display(
                &context,
                SessionId(7),
                &TypedIdentifier::new("writer").unwrap(),
                41,
                9,
                1,
                11,
                true,
                DisplayLifecycleEventKind::ApplicationInstanceRegistered,
            ),
            Err(LifecycleIngressError::WrongTarget)
        );
        adapter
            .ingest_display(
                &context,
                SessionId(7),
                &app,
                41,
                9,
                1,
                11,
                true,
                DisplayLifecycleEventKind::ApplicationInstanceRegistered,
            )
            .unwrap();
        assert_eq!(
            adapter.ingest_display(
                &context,
                SessionId(7),
                &app,
                41,
                9,
                2,
                12,
                false,
                DisplayLifecycleEventKind::ApplicationReadinessSatisfied,
            ),
            Err(LifecycleIngressError::ReadinessContractNotSatisfied)
        );
        assert!(adapter.next_accepted().is_some());
    }

    #[test]
    fn control_panel_requires_exact_authoritative_page_activation() {
        let mut adapter = BraindTrustedLifecycleAdapters::new();
        adapter.authenticate_control_panel(5).unwrap();
        let page = TypedIdentifier::new("network").unwrap();
        let context = adapter
            .register_launch_context(
                ExecutionId(2),
                page.clone(),
                LifecycleTargetKind::SettingsPage,
                SessionId(7),
                77,
                4,
                1,
                5,
                LaunchCorrelationToken::new_for_test([2; 32]),
                10,
                100,
            )
            .unwrap();
        assert_eq!(
            adapter.ingest_control_panel(
                &context,
                SessionId(7),
                &page,
                77,
                5,
                1,
                11,
                false,
                ControlPanelLifecycleEventKind::CanonicalPageActivated,
            ),
            Err(LifecycleIngressError::NavigationRequestIsNotActivation)
        );
        assert_eq!(
            adapter.ingest_control_panel(
                &context,
                SessionId(7),
                &TypedIdentifier::new("home").unwrap(),
                77,
                5,
                1,
                11,
                true,
                ControlPanelLifecycleEventKind::CanonicalPageActivated,
            ),
            Err(LifecycleIngressError::WrongTarget)
        );
        adapter
            .ingest_control_panel(
                &context,
                SessionId(7),
                &page,
                77,
                5,
                1,
                11,
                true,
                ControlPanelLifecycleEventKind::CanonicalPageActivated,
            )
            .unwrap();
        assert_eq!(
            adapter.ingest_control_panel(
                &context,
                SessionId(7),
                &page,
                77,
                5,
                1,
                12,
                true,
                ControlPanelLifecycleEventKind::CanonicalPageReady,
            ),
            Err(LifecycleIngressError::DuplicateOrOutOfOrder)
        );
    }

    #[test]
    fn no_diagnostic_or_pid_only_context_constructor_exists() {
        assert!(!core::any::type_name::<TrustedLaunchContext>().contains("String"));
        assert_eq!(MAX_LAUNCH_CONTEXTS, 16);
        assert_eq!(MAX_LIFECYCLE_EVENTS_PER_EXECUTION, 8);
    }
}
