//! Immutable, fail-closed action policy for Wise Owl.
//!
//! Policy decides whether an operation may proceed. It does not choose
//! operations, perform them, modify runtime state, or learn from decisions.

use core::fmt::Write;

use crate::runtime_context::RuntimeContextSnapshot;

pub const POLICY_V1_VERSION: PolicyVersion = PolicyVersion::new(1, 0);
const MAX_AUDIT_RECORD_LEN: usize = 192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyVersion {
    pub major: u16,
    pub minor: u16,
}

impl PolicyVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyCategory {
    Read,
    Observe,
    Recommend,
    Execute,
    Modify,
    Delete,
    Critical,
}

impl PolicyCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "Read",
            Self::Observe => "Observe",
            Self::Recommend => "Recommend",
            Self::Execute => "Execute",
            Self::Modify => "Modify",
            Self::Delete => "Delete",
            Self::Critical => "Critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyOperation {
    ReadHostname,
    ReadTimezone,
    ReadNetwork,
    ObserveRuntime,
    RecommendAction,
    LaunchCalculator,
    OpenControlPanel,
    OpenApplication,
    OpenSettingsPage,
    LaunchUtility,
    RestartService,
    StopService,
    InstallPackage,
    RemovePackage,
    ModifyFile,
    WriteInstallerDisk,
    DiskErase,
    DeleteFiles,
    RecoveryMaintenance,
    FormatDisk,
    ModifyBootloader,
    /// A caller-provided operation not understood by this policy version.
    Other(u16),
}

impl PolicyOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadHostname => "ReadHostname",
            Self::ReadTimezone => "ReadTimezone",
            Self::ReadNetwork => "ReadNetwork",
            Self::ObserveRuntime => "ObserveRuntime",
            Self::RecommendAction => "RecommendAction",
            Self::LaunchCalculator => "LaunchCalculator",
            Self::OpenControlPanel => "OpenControlPanel",
            Self::OpenApplication => "OpenApplication",
            Self::OpenSettingsPage => "OpenSettingsPage",
            Self::LaunchUtility => "LaunchUtility",
            Self::RestartService => "RestartService",
            Self::StopService => "StopService",
            Self::InstallPackage => "InstallPackage",
            Self::RemovePackage => "RemovePackage",
            Self::ModifyFile => "ModifyFile",
            Self::WriteInstallerDisk => "WriteInstallerDisk",
            Self::DiskErase => "DiskErase",
            Self::DeleteFiles => "DeleteFiles",
            Self::RecoveryMaintenance => "RecoveryMaintenance",
            Self::FormatDisk => "FormatDisk",
            Self::ModifyBootloader => "ModifyBootloader",
            Self::Other(_) => "UnknownOperation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationLevel {
    None,
    Soft,
    Strong,
    Critical,
}

impl ConfirmationLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Soft => "Soft",
            Self::Strong => "Strong",
            Self::Critical => "Critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyResult {
    Allowed,
    Denied,
    ConfirmationRequired,
    Unknown,
}

impl PolicyResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "Allowed",
            Self::Denied => "Denied",
            Self::ConfirmationRequired => "ConfirmationRequired",
            Self::Unknown => "Unknown",
        }
    }
}

/// Stable, user-safe explanations. These labels reveal policy outcomes, not
/// implementation structure or runtime provider details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyReason {
    ExplicitlyAllowed,
    ConfirmationRequired,
    AlwaysProtected,
    InstallerMode,
    RecoveryMode,
    DesktopMode,
    ModeNotPermitted,
    RuntimeModeUnknown,
    RuntimeModeConflicting,
    NoMatchingRule,
}

impl PolicyReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitlyAllowed => "ExplicitlyAllowed",
            Self::ConfirmationRequired => "ConfirmationRequired",
            Self::AlwaysProtected => "AlwaysProtected",
            Self::InstallerMode => "InstallerMode",
            Self::RecoveryMode => "RecoveryMode",
            Self::DesktopMode => "DesktopMode",
            Self::ModeNotPermitted => "ModeNotPermitted",
            Self::RuntimeModeUnknown => "RuntimeModeUnknown",
            Self::RuntimeModeConflicting => "RuntimeModeConflicting",
            Self::NoMatchingRule => "NoMatchingRule",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PolicyDecision {
    pub(crate) version: PolicyVersion,
    pub(crate) operation: PolicyOperation,
    pub(crate) category: Option<PolicyCategory>,
    pub(crate) result: PolicyResult,
    pub(crate) confirmation: ConfirmationLevel,
    pub(crate) reason: PolicyReason,
}

impl PolicyDecision {
    /// Produces a bounded operator-facing record suitable for the existing
    /// logging facilities. It contains no rule layout or provider internals.
    pub fn audit_record(&self) -> heapless::String<MAX_AUDIT_RECORD_LEN> {
        let mut record = heapless::String::new();
        let _ = writeln!(&mut record, "POLICY");
        let _ = writeln!(&mut record, "operation={}", self.operation.as_str());
        let _ = writeln!(&mut record, "result={}", self.result.as_str());
        let _ = writeln!(&mut record, "reason={}", self.reason.as_str());
        let _ = writeln!(&mut record, "confirmation={}", self.confirmation.as_str());
        let _ = write!(
            &mut record,
            "version={}.{}",
            self.version.major, self.version.minor
        );
        record
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyEffect {
    Allow,
    Deny,
    Confirm(ConfirmationLevel),
    InstallerDiskWrite,
    DiskEraseByMode,
    RecoveryMaintenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyRule {
    pub operation: PolicyOperation,
    pub category: PolicyCategory,
    pub effect: PolicyEffect,
}

impl PolicyRule {
    pub const fn new(
        operation: PolicyOperation,
        category: PolicyCategory,
        effect: PolicyEffect,
    ) -> Self {
        Self {
            operation,
            category,
            effect,
        }
    }
}

/// A policy is an immutable version and immutable rule slice. Future OS policy
/// groups can ship a different static slice without changing evaluation.
#[derive(Debug, Clone, Copy)]
pub struct PolicyEngine {
    version: PolicyVersion,
    rules: &'static [PolicyRule],
}

impl PolicyEngine {
    pub const fn v1() -> Self {
        Self::from_static_rules(POLICY_V1_VERSION, POLICY_V1_RULES)
    }

    pub const fn from_static_rules(version: PolicyVersion, rules: &'static [PolicyRule]) -> Self {
        Self { version, rules }
    }

    pub const fn version(&self) -> PolicyVersion {
        self.version
    }

    pub fn rules(&self) -> &'static [PolicyRule] {
        self.rules
    }

    pub(crate) fn evaluate(
        &self,
        operation: PolicyOperation,
        runtime: &RuntimeContextSnapshot,
    ) -> PolicyDecision {
        let Some(rule) = self.rules.iter().find(|rule| rule.operation == operation) else {
            return self.decision(
                operation,
                None,
                PolicyResult::Unknown,
                ConfirmationLevel::None,
                PolicyReason::NoMatchingRule,
            );
        };

        match rule.effect {
            PolicyEffect::Allow => self.decision(
                operation,
                Some(rule.category),
                PolicyResult::Allowed,
                ConfirmationLevel::None,
                PolicyReason::ExplicitlyAllowed,
            ),
            PolicyEffect::Deny => self.decision(
                operation,
                Some(rule.category),
                PolicyResult::Denied,
                ConfirmationLevel::None,
                PolicyReason::AlwaysProtected,
            ),
            PolicyEffect::Confirm(level) => self.decision(
                operation,
                Some(rule.category),
                PolicyResult::ConfirmationRequired,
                level,
                PolicyReason::ConfirmationRequired,
            ),
            PolicyEffect::InstallerDiskWrite => {
                self.evaluate_installer_disk_write(operation, rule.category, runtime)
            }
            PolicyEffect::DiskEraseByMode => {
                self.evaluate_disk_erase(operation, rule.category, runtime)
            }
            PolicyEffect::RecoveryMaintenance => {
                self.evaluate_recovery_maintenance(operation, rule.category, runtime)
            }
        }
    }

    fn evaluate_installer_disk_write(
        &self,
        operation: PolicyOperation,
        category: PolicyCategory,
        runtime: &RuntimeContextSnapshot,
    ) -> PolicyDecision {
        match runtime_mode(runtime) {
            Ok(RuntimeMode::Installer) => self.decision(
                operation,
                Some(category),
                PolicyResult::Allowed,
                ConfirmationLevel::None,
                PolicyReason::InstallerMode,
            ),
            Ok(RuntimeMode::Desktop) => self.decision(
                operation,
                Some(category),
                PolicyResult::Denied,
                ConfirmationLevel::None,
                PolicyReason::DesktopMode,
            ),
            Ok(RuntimeMode::Recovery) => self.decision(
                operation,
                Some(category),
                PolicyResult::Denied,
                ConfirmationLevel::None,
                PolicyReason::ModeNotPermitted,
            ),
            Err(reason) => self.decision(
                operation,
                Some(category),
                PolicyResult::Unknown,
                ConfirmationLevel::None,
                reason,
            ),
        }
    }

    fn evaluate_disk_erase(
        &self,
        operation: PolicyOperation,
        category: PolicyCategory,
        runtime: &RuntimeContextSnapshot,
    ) -> PolicyDecision {
        match runtime_mode(runtime) {
            Ok(RuntimeMode::Installer) => self.decision(
                operation,
                Some(category),
                PolicyResult::ConfirmationRequired,
                ConfirmationLevel::Critical,
                PolicyReason::InstallerMode,
            ),
            Ok(RuntimeMode::Recovery) => self.decision(
                operation,
                Some(category),
                PolicyResult::ConfirmationRequired,
                ConfirmationLevel::Critical,
                PolicyReason::RecoveryMode,
            ),
            Ok(RuntimeMode::Desktop) => self.decision(
                operation,
                Some(category),
                PolicyResult::Denied,
                ConfirmationLevel::None,
                PolicyReason::DesktopMode,
            ),
            Err(reason) => self.decision(
                operation,
                Some(category),
                PolicyResult::Unknown,
                ConfirmationLevel::None,
                reason,
            ),
        }
    }

    fn evaluate_recovery_maintenance(
        &self,
        operation: PolicyOperation,
        category: PolicyCategory,
        runtime: &RuntimeContextSnapshot,
    ) -> PolicyDecision {
        match runtime_mode(runtime) {
            Ok(RuntimeMode::Recovery) => self.decision(
                operation,
                Some(category),
                PolicyResult::Allowed,
                ConfirmationLevel::None,
                PolicyReason::RecoveryMode,
            ),
            Ok(_) => self.decision(
                operation,
                Some(category),
                PolicyResult::Denied,
                ConfirmationLevel::None,
                PolicyReason::ModeNotPermitted,
            ),
            Err(reason) => self.decision(
                operation,
                Some(category),
                PolicyResult::Unknown,
                ConfirmationLevel::None,
                reason,
            ),
        }
    }

    fn decision(
        &self,
        operation: PolicyOperation,
        category: Option<PolicyCategory>,
        result: PolicyResult,
        confirmation: ConfirmationLevel,
        reason: PolicyReason,
    ) -> PolicyDecision {
        PolicyDecision {
            version: self.version,
            operation,
            category,
            result,
            confirmation,
            reason,
        }
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::v1()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeMode {
    Installer,
    Recovery,
    Desktop,
}

fn runtime_mode(runtime: &RuntimeContextSnapshot) -> Result<RuntimeMode, PolicyReason> {
    let installer = runtime.session.installer_mode == Some(true);
    let recovery = runtime.session.recovery_mode == Some(true);
    let desktop = runtime.session.desktop_mode == Some(true);
    let known_mode_count = u8::from(installer) + u8::from(recovery) + u8::from(desktop);

    match known_mode_count {
        0 => Err(PolicyReason::RuntimeModeUnknown),
        1 if installer => Ok(RuntimeMode::Installer),
        1 if recovery => Ok(RuntimeMode::Recovery),
        1 if desktop => Ok(RuntimeMode::Desktop),
        _ => Err(PolicyReason::RuntimeModeConflicting),
    }
}

pub static POLICY_V1_RULES: &[PolicyRule] = &[
    PolicyRule::new(
        PolicyOperation::ReadHostname,
        PolicyCategory::Read,
        PolicyEffect::Allow,
    ),
    PolicyRule::new(
        PolicyOperation::ReadTimezone,
        PolicyCategory::Read,
        PolicyEffect::Allow,
    ),
    PolicyRule::new(
        PolicyOperation::ReadNetwork,
        PolicyCategory::Read,
        PolicyEffect::Allow,
    ),
    PolicyRule::new(
        PolicyOperation::ObserveRuntime,
        PolicyCategory::Observe,
        PolicyEffect::Allow,
    ),
    PolicyRule::new(
        PolicyOperation::RecommendAction,
        PolicyCategory::Recommend,
        PolicyEffect::Allow,
    ),
    PolicyRule::new(
        PolicyOperation::LaunchCalculator,
        PolicyCategory::Execute,
        PolicyEffect::Allow,
    ),
    PolicyRule::new(
        PolicyOperation::OpenControlPanel,
        PolicyCategory::Execute,
        PolicyEffect::Allow,
    ),
    PolicyRule::new(
        PolicyOperation::OpenApplication,
        PolicyCategory::Execute,
        PolicyEffect::Allow,
    ),
    PolicyRule::new(
        PolicyOperation::OpenSettingsPage,
        PolicyCategory::Execute,
        PolicyEffect::Allow,
    ),
    PolicyRule::new(
        PolicyOperation::LaunchUtility,
        PolicyCategory::Execute,
        PolicyEffect::Confirm(ConfirmationLevel::Soft),
    ),
    PolicyRule::new(
        PolicyOperation::RestartService,
        PolicyCategory::Execute,
        PolicyEffect::Confirm(ConfirmationLevel::Soft),
    ),
    PolicyRule::new(
        PolicyOperation::StopService,
        PolicyCategory::Execute,
        PolicyEffect::Confirm(ConfirmationLevel::Strong),
    ),
    PolicyRule::new(
        PolicyOperation::InstallPackage,
        PolicyCategory::Modify,
        PolicyEffect::Confirm(ConfirmationLevel::Strong),
    ),
    PolicyRule::new(
        PolicyOperation::RemovePackage,
        PolicyCategory::Delete,
        PolicyEffect::Confirm(ConfirmationLevel::Strong),
    ),
    PolicyRule::new(
        PolicyOperation::ModifyFile,
        PolicyCategory::Modify,
        PolicyEffect::Confirm(ConfirmationLevel::Strong),
    ),
    PolicyRule::new(
        PolicyOperation::WriteInstallerDisk,
        PolicyCategory::Modify,
        PolicyEffect::InstallerDiskWrite,
    ),
    PolicyRule::new(
        PolicyOperation::DiskErase,
        PolicyCategory::Delete,
        PolicyEffect::DiskEraseByMode,
    ),
    PolicyRule::new(
        PolicyOperation::DeleteFiles,
        PolicyCategory::Delete,
        PolicyEffect::Confirm(ConfirmationLevel::Strong),
    ),
    PolicyRule::new(
        PolicyOperation::RecoveryMaintenance,
        PolicyCategory::Modify,
        PolicyEffect::RecoveryMaintenance,
    ),
    PolicyRule::new(
        PolicyOperation::FormatDisk,
        PolicyCategory::Critical,
        PolicyEffect::Deny,
    ),
    PolicyRule::new(
        PolicyOperation::ModifyBootloader,
        PolicyCategory::Critical,
        PolicyEffect::Deny,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(installer: bool, recovery: bool, desktop: bool) -> RuntimeContextSnapshot {
        let mut runtime = RuntimeContextSnapshot::default();
        runtime.session.installer_mode = Some(installer);
        runtime.session.recovery_mode = Some(recovery);
        runtime.session.desktop_mode = Some(desktop);
        runtime
    }

    #[test]
    fn allows_explicit_read_and_execute_paths() {
        let engine = PolicyEngine::v1();
        let runtime = RuntimeContextSnapshot::default();

        assert_eq!(
            engine
                .evaluate(PolicyOperation::ReadHostname, &runtime)
                .result,
            PolicyResult::Allowed
        );
        assert_eq!(
            engine
                .evaluate(PolicyOperation::LaunchCalculator, &runtime)
                .result,
            PolicyResult::Allowed
        );
    }

    #[test]
    fn always_protected_paths_are_denied() {
        let decision = PolicyEngine::v1().evaluate(
            PolicyOperation::ModifyBootloader,
            &snapshot(true, false, false),
        );

        assert_eq!(decision.result, PolicyResult::Denied);
        assert_eq!(decision.reason, PolicyReason::AlwaysProtected);
    }

    #[test]
    fn confirmation_paths_report_the_required_level() {
        let runtime = RuntimeContextSnapshot::default();
        let soft = PolicyEngine::v1().evaluate(PolicyOperation::RestartService, &runtime);
        let strong = PolicyEngine::v1().evaluate(PolicyOperation::DeleteFiles, &runtime);

        assert_eq!(soft.result, PolicyResult::ConfirmationRequired);
        assert_eq!(soft.confirmation, ConfirmationLevel::Soft);
        assert_eq!(strong.result, PolicyResult::ConfirmationRequired);
        assert_eq!(strong.confirmation, ConfirmationLevel::Strong);
    }

    #[test]
    fn unknown_operation_never_becomes_allowed() {
        let decision =
            PolicyEngine::v1().evaluate(PolicyOperation::Other(900), &snapshot(true, false, false));

        assert_eq!(decision.result, PolicyResult::Unknown);
        assert_eq!(decision.reason, PolicyReason::NoMatchingRule);
    }

    #[test]
    fn missing_or_conflicting_mode_stays_unknown() {
        let engine = PolicyEngine::v1();
        let missing = engine.evaluate(
            PolicyOperation::WriteInstallerDisk,
            &RuntimeContextSnapshot::default(),
        );
        let conflicting = engine.evaluate(PolicyOperation::DiskErase, &snapshot(true, true, false));

        assert_eq!(missing.result, PolicyResult::Unknown);
        assert_eq!(missing.reason, PolicyReason::RuntimeModeUnknown);
        assert_eq!(conflicting.result, PolicyResult::Unknown);
        assert_eq!(conflicting.reason, PolicyReason::RuntimeModeConflicting);
    }

    #[test]
    fn disk_policy_is_mode_specific() {
        let engine = PolicyEngine::v1();
        let installer = engine.evaluate(
            PolicyOperation::WriteInstallerDisk,
            &snapshot(true, false, false),
        );
        let desktop = engine.evaluate(PolicyOperation::DiskErase, &snapshot(false, false, true));
        let recovery = engine.evaluate(
            PolicyOperation::RecoveryMaintenance,
            &snapshot(false, true, false),
        );

        assert_eq!(installer.result, PolicyResult::Allowed);
        assert_eq!(installer.reason, PolicyReason::InstallerMode);
        assert_eq!(desktop.result, PolicyResult::Denied);
        assert_eq!(desktop.reason, PolicyReason::DesktopMode);
        assert_eq!(recovery.result, PolicyResult::Allowed);
        assert_eq!(recovery.reason, PolicyReason::RecoveryMode);
    }

    #[test]
    fn critical_confirmation_and_audit_record_are_explainable() {
        let decision =
            PolicyEngine::v1().evaluate(PolicyOperation::DiskErase, &snapshot(true, false, false));
        let audit = decision.audit_record();

        assert_eq!(decision.result, PolicyResult::ConfirmationRequired);
        assert_eq!(decision.confirmation, ConfirmationLevel::Critical);
        assert!(audit.contains("operation=DiskErase"));
        assert!(audit.contains("result=ConfirmationRequired"));
        assert!(audit.contains("reason=InstallerMode"));
        assert!(audit.contains("version=1.0"));
    }

    #[test]
    fn evaluation_does_not_modify_runtime_snapshot() {
        let runtime = snapshot(true, false, false);
        let captured = runtime.captured_mono_ms;
        let installer = runtime.session.installer_mode;

        let _ = PolicyEngine::v1().evaluate(PolicyOperation::WriteInstallerDisk, &runtime);

        assert_eq!(runtime.captured_mono_ms, captured);
        assert_eq!(runtime.session.installer_mode, installer);
    }
}
