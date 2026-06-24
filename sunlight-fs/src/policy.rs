use crate::{path, FsError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor<'a> {
    User { uid: u32, name: &'a str },
    Service { name: &'a str },
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsOperation {
    Write,
    Create,
    Mkdir,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityRights {
    pub write: bool,
    pub create: bool,
    pub mkdir: bool,
    pub delete: bool,
}

impl CapabilityRights {
    pub const WRITE_CREATE: Self = Self {
        write: true,
        create: true,
        mkdir: false,
        delete: false,
    };

    pub fn allows(self, operation: FsOperation) -> bool {
        match operation {
            FsOperation::Write => self.write,
            FsOperation::Create => self.create,
            FsOperation::Mkdir => self.mkdir,
            FsOperation::Delete => self.delete,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsCapability<'a> {
    pub issuer: Actor<'a>,
    pub subject: Actor<'a>,
    pub rights: CapabilityRights,
    pub scope: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyReason {
    AllowedTmp,
    AllowedCurrentUserHome,
    AllowedRuntimePathWithUac,
    AllowedServiceState,
    AllowedByScopedCapability,
    DeniedImmutableRoot,
    DeniedProtectedPath,
    DeniedOtherUserHome,
    DeniedRuntimePathNeedsUac,
    DeniedMissingCapability,
    DeniedInvalidCapability,
    DeniedUnknownActor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,
    pub reason: PolicyReason,
    pub error: Option<FsError>,
}

impl Decision {
    const fn allow(reason: PolicyReason) -> Self {
        Self {
            allowed: true,
            reason,
            error: None,
        }
    }

    const fn deny(reason: PolicyReason, error: FsError) -> Self {
        Self {
            allowed: false,
            reason,
            error: Some(error),
        }
    }
}

pub fn can_write(
    actor: Actor<'_>,
    path: &str,
    operation: FsOperation,
    capability: Option<&FsCapability<'_>>,
    uac_approved: bool,
) -> Decision {
    if path::validate_absolute(path).is_err() {
        return Decision::deny(PolicyReason::DeniedImmutableRoot, FsError::InvalidPath);
    }

    if is_protected_path(path) {
        return Decision::deny(
            PolicyReason::DeniedProtectedPath,
            FsError::ReadOnlyFilesystem,
        );
    }

    if let Some(cap) = capability {
        match validate_capability(actor, path, operation, cap) {
            CapabilityCheck::Allowed => {
                return Decision::allow(PolicyReason::AllowedByScopedCapability);
            }
            CapabilityCheck::Invalid => {
                return Decision::deny(
                    PolicyReason::DeniedInvalidCapability,
                    FsError::OperationNotPermitted,
                );
            }
        }
    }

    match actor {
        Actor::User { uid, name } => user_decision(uid, name, path, uac_approved),
        Actor::Service { name } => service_decision(name, path),
        Actor::Unknown => Decision::deny(
            PolicyReason::DeniedUnknownActor,
            FsError::OperationNotPermitted,
        ),
    }
}

fn user_decision(uid: u32, name: &str, path: &str, uac_approved: bool) -> Decision {
    if path == "/tmp" || is_under(path, "/tmp") {
        return Decision::allow(PolicyReason::AllowedTmp);
    }

    if uid == 0
        && (path == "/root"
            || is_under(path, "/root")
            || path == "/home"
            || is_under(path, "/home"))
    {
        return Decision::allow(PolicyReason::AllowedCurrentUserHome);
    }

    if let Some(home_user) = home_user(path) {
        if home_user == name {
            return Decision::allow(PolicyReason::AllowedCurrentUserHome);
        }
        return Decision::deny(PolicyReason::DeniedOtherUserHome, FsError::PermissionDenied);
    }

    if is_runtime_path(path) {
        return if uac_approved {
            Decision::allow(PolicyReason::AllowedRuntimePathWithUac)
        } else {
            Decision::deny(
                PolicyReason::DeniedRuntimePathNeedsUac,
                FsError::OperationNotPermitted,
            )
        };
    }

    if is_service_state_path(path) {
        return Decision::deny(
            PolicyReason::DeniedMissingCapability,
            FsError::OperationNotPermitted,
        );
    }

    Decision::deny(
        PolicyReason::DeniedImmutableRoot,
        FsError::ReadOnlyFilesystem,
    )
}

fn service_decision(service: &str, path: &str) -> Decision {
    if service_state_owner(path) == Some(service) {
        return Decision::allow(PolicyReason::AllowedServiceState);
    }
    if is_service_state_path(path) {
        return Decision::deny(
            PolicyReason::DeniedMissingCapability,
            FsError::OperationNotPermitted,
        );
    }
    Decision::deny(
        PolicyReason::DeniedImmutableRoot,
        FsError::ReadOnlyFilesystem,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapabilityCheck {
    Allowed,
    Invalid,
}

fn validate_capability(
    actor: Actor<'_>,
    path: &str,
    operation: FsOperation,
    cap: &FsCapability<'_>,
) -> CapabilityCheck {
    if !is_trusted_service(cap.issuer) || actor != cap.subject {
        return CapabilityCheck::Invalid;
    }
    if !cap.rights.allows(operation) {
        return CapabilityCheck::Invalid;
    }
    if cap.scope == "/" || is_protected_path(cap.scope) {
        return CapabilityCheck::Invalid;
    }
    if path == cap.scope || is_under(path, cap.scope) {
        CapabilityCheck::Allowed
    } else {
        CapabilityCheck::Invalid
    }
}

pub fn broker_mint_fs_capability<'a>(
    requester: Actor<'a>,
    subject: Actor<'a>,
    rights: CapabilityRights,
    scope: &'a str,
) -> Result<FsCapability<'a>, PolicyReason> {
    if !is_trusted_service(requester) {
        return Err(PolicyReason::DeniedUnknownActor);
    }
    if scope == "/" || is_protected_path(scope) || path::validate_absolute(scope).is_err() {
        return Err(PolicyReason::DeniedProtectedPath);
    }
    match subject {
        Actor::Service { name } if service_state_owner(scope) == Some(name) => Ok(FsCapability {
            issuer: requester,
            subject,
            rights,
            scope,
        }),
        _ => Err(PolicyReason::DeniedMissingCapability),
    }
}

fn is_trusted_service(actor: Actor<'_>) -> bool {
    matches!(
        actor,
        Actor::Service {
            name: "sunlight-kv" | "sunlight-tls" | "sunlight-uac" | "capability-broker"
        }
    )
}

fn is_protected_path(path: &str) -> bool {
    [
        "/boot",
        "/kernel",
        "/bin",
        "/sbin",
        "/services",
        "/etc",
        "/proc",
        "/sys",
    ]
    .iter()
    .any(|prefix| path == *prefix || is_under(path, prefix))
}

fn is_runtime_path(path: &str) -> bool {
    path == "/run" || is_under(path, "/run")
}

fn is_service_state_path(path: &str) -> bool {
    path == "/state" || is_under(path, "/state") || path == "/var/lib" || is_under(path, "/var/lib")
}

fn home_user(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/home/")?;
    let end = rest.find('/').unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(&rest[..end])
    }
}

fn service_state_owner(path: &str) -> Option<&str> {
    let rest = path
        .strip_prefix("/state/")
        .or_else(|| path.strip_prefix("/var/lib/"))?;
    let end = rest.find('/').unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(&rest[..end])
    }
}

fn is_under(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .map(|suffix| suffix.starts_with('/'))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: Actor<'static> = Actor::User {
        uid: 1000,
        name: "user",
    };
    const ROOT: Actor<'static> = Actor::User {
        uid: 0,
        name: "root",
    };
    const KV: Actor<'static> = Actor::Service {
        name: "sunlight-kv",
    };
    const TLS: Actor<'static> = Actor::Service {
        name: "sunlight-tls",
    };

    fn deny_reason(path: &str) -> PolicyReason {
        can_write(USER, path, FsOperation::Create, None, false).reason
    }

    #[test]
    fn normal_user_write_policy() {
        assert_eq!(
            can_write(USER, "/tmp/file", FsOperation::Create, None, false).reason,
            PolicyReason::AllowedTmp
        );
        assert_eq!(
            can_write(USER, "/home/user/file", FsOperation::Create, None, false).reason,
            PolicyReason::AllowedCurrentUserHome
        );
        assert_eq!(deny_reason("/file"), PolicyReason::DeniedImmutableRoot);
        assert_eq!(
            deny_reason("/services/file"),
            PolicyReason::DeniedProtectedPath
        );
        assert_eq!(deny_reason("/bin/file"), PolicyReason::DeniedProtectedPath);
        assert_eq!(deny_reason("/etc/file"), PolicyReason::DeniedProtectedPath);
        assert_eq!(deny_reason("/dev/file"), PolicyReason::DeniedImmutableRoot);
        assert_eq!(
            deny_reason("/run/file"),
            PolicyReason::DeniedRuntimePathNeedsUac
        );
        assert_eq!(
            deny_reason("/home/other/file"),
            PolicyReason::DeniedOtherUserHome
        );
    }

    #[test]
    fn root_can_write_root_and_user_home_trees() {
        assert_eq!(
            can_write(ROOT, "/root/file", FsOperation::Create, None, false).reason,
            PolicyReason::AllowedCurrentUserHome
        );
        assert_eq!(
            can_write(ROOT, "/home/user/file", FsOperation::Create, None, false).reason,
            PolicyReason::AllowedCurrentUserHome
        );
        assert_eq!(
            can_write(ROOT, "/home/other/file", FsOperation::Create, None, false).reason,
            PolicyReason::AllowedCurrentUserHome
        );
        assert_eq!(
            can_write(ROOT, "/etc/file", FsOperation::Create, None, false).reason,
            PolicyReason::DeniedProtectedPath
        );
    }

    #[test]
    fn uac_only_gates_runtime_paths() {
        assert_eq!(
            can_write(USER, "/run/file", FsOperation::Create, None, true).reason,
            PolicyReason::AllowedRuntimePathWithUac
        );
        assert_eq!(
            can_write(USER, "/etc/file", FsOperation::Create, None, true).reason,
            PolicyReason::DeniedProtectedPath
        );
    }

    #[test]
    fn service_state_is_service_scoped() {
        assert_eq!(
            can_write(
                KV,
                "/state/sunlight-kv/file",
                FsOperation::Create,
                None,
                false
            )
            .reason,
            PolicyReason::AllowedServiceState
        );
        assert_eq!(
            can_write(
                KV,
                "/state/sunlight-tls/file",
                FsOperation::Create,
                None,
                false
            )
            .reason,
            PolicyReason::DeniedMissingCapability
        );
        assert_eq!(
            can_write(KV, "/services/file", FsOperation::Create, None, false).reason,
            PolicyReason::DeniedProtectedPath
        );
    }

    #[test]
    fn broker_mints_only_trusted_scoped_service_caps() {
        let cap =
            broker_mint_fs_capability(KV, KV, CapabilityRights::WRITE_CREATE, "/state/sunlight-kv")
                .expect("trusted service state cap");
        assert_eq!(
            can_write(
                KV,
                "/state/sunlight-kv/db",
                FsOperation::Create,
                Some(&cap),
                false
            )
            .reason,
            PolicyReason::AllowedByScopedCapability
        );
        assert_eq!(
            can_write(KV, "/etc/db", FsOperation::Create, Some(&cap), false).reason,
            PolicyReason::DeniedProtectedPath
        );
        assert_eq!(
            can_write(
                TLS,
                "/state/sunlight-kv/db",
                FsOperation::Create,
                Some(&cap),
                false
            )
            .reason,
            PolicyReason::DeniedInvalidCapability
        );
        assert!(
            broker_mint_fs_capability(USER, USER, CapabilityRights::WRITE_CREATE, "/etc").is_err()
        );
        assert!(broker_mint_fs_capability(TLS, TLS, CapabilityRights::WRITE_CREATE, "/").is_err());
    }

    #[test]
    fn traversal_paths_are_rejected_before_prefix_policy() {
        assert_eq!(
            can_write(USER, "/tmp/../etc/x", FsOperation::Create, None, false).error,
            Some(FsError::InvalidPath)
        );
        assert_eq!(
            can_write(
                USER,
                "/home/user/../../etc/x",
                FsOperation::Create,
                None,
                false
            )
            .error,
            Some(FsError::InvalidPath)
        );
        assert_eq!(
            can_write(
                KV,
                "/state/sunlight-kv/../../../bin/x",
                FsOperation::Create,
                None,
                false
            )
            .error,
            Some(FsError::InvalidPath)
        );
    }
}
