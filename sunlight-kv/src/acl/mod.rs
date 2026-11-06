//! Caller identity, per-key ACLs and namespace policy for sunlight-kv.
//!
//! The security model has three layers:
//!
//! 1. **Caller identity** ([`Caller`]) is derived from kernel-authoritative IPC
//!    metadata (the sender PID stamped into the message badge, resolved to real
//!    UID/GID and a trusted-service key). It is never supplied by the client.
//! 2. **Per-key ACL** ([`Acl`]) is stored with every record and decides
//!    read/write rights for a concrete caller.
//! 3. **Namespace policy** ([`Namespace`]) applies coarse, key-prefix-scoped
//!    rules on top of the per-key ACL (own-user accounts, vault isolation and
//!    public read-only namespaces such as TLS trust anchors).

#[cfg(feature = "host")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "sunlightos")]
use alloc::string::String;
#[cfg(feature = "sunlightos")]
use alloc::vec;
#[cfg(feature = "sunlightos")]
use alloc::vec::Vec; // brings vec! macro into scope for no_std

/// FNV-1a 64-bit hash, matching `sunlight_ipc::name_to_u64`.
///
/// Stable service-identity keys are derived with this hash so the daemon and the
/// kernel agree on `service:<name>` without shipping strings across the ABI.
pub const fn name_to_u64(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash = 0xcbf29ce484222325u64;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        i += 1;
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

/// The auth-broker service key (UAC): the only non-root identity permitted to
/// read another user's `account:<uid>:*` namespace.
pub const AUTH_BROKER_SERVICE: u64 = name_to_u64("sunlight-uac");
/// TLS service key, recognised for public trust-anchor administration.
pub const TLS_SERVICE: u64 = name_to_u64("sunlight-tls");
/// KV service key.
pub const KV_SERVICE: u64 = name_to_u64("sunlight-kv");

/// Reserved ACL identity granting read access to every caller (World).
///
/// Encoded inside the `read` list so the on-disk ACL layout stays byte-identical
/// to the legacy `{owner, read, write}` record and existing stores keep loading.
pub const WORLD: &str = "*";

/// Authenticated caller identity for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    /// UID 0, or a trusted kernel service with no narrower role.
    Root,
    /// An authenticated system daemon, identified by its stable service key.
    Service(u64),
    /// A regular session application running as `uid`.
    User(u32),
    /// Identity could not be resolved; denied except for world-readable data.
    Unknown,
}

impl Caller {
    /// Canonical ACL identity string persisted in records.
    ///
    /// Kept as a string so the on-disk ACL stays human-inspectable and the host
    /// and SunlightOS builds share one serialization shape.
    pub fn identity(&self) -> String {
        match self {
            Caller::Root => String::from("root"),
            Caller::Service(key) => service_identity(*key),
            Caller::User(uid) => user_identity(*uid),
            Caller::Unknown => String::from("unknown"),
        }
    }

    /// Parse a host-mode `SO_PEERCRED` identity string ("root", "uid:N",
    /// "service:N", "unknown") back into a [`Caller`]. Used only by the host
    /// daemon; the SunlightOS build resolves identity from kernel metadata.
    pub fn from_identity_str(s: &str) -> Caller {
        if s == "root" {
            return Caller::Root;
        }
        if let Some(rest) = s.strip_prefix("uid:") {
            return Caller::User(rest.parse::<u32>().unwrap_or(u32::MAX));
        }
        if let Some(rest) = s.strip_prefix("service:") {
            return Caller::Service(rest.parse::<u64>().unwrap_or(u64::MAX));
        }
        Caller::Unknown
    }

    /// True for identities that may perform an administrative override.
    pub fn is_privileged(&self) -> bool {
        matches!(self, Caller::Root | Caller::Service(_))
    }

    /// True only for the root identity, the strictest administrative authority.
    /// Used for cross-user account and vault namespaces, where a generic
    /// service (e.g. TLS) must not be able to read another user's tokens.
    pub fn is_root(&self) -> bool {
        matches!(self, Caller::Root)
    }

    /// True when this caller is the explicitly authorised auth broker.
    pub fn is_auth_broker(&self) -> bool {
        matches!(self, Caller::Service(AUTH_BROKER_SERVICE))
    }

    /// True when this caller may administer public trust anchors.
    pub fn is_tls_service(&self) -> bool {
        matches!(self, Caller::Service(TLS_SERVICE))
    }
}

#[cfg(feature = "host")]
impl core::fmt::Display for Caller {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.identity())
    }
}

/// Format the ACL identity string for a user uid (`uid:<n>`).
pub fn user_identity(uid: u32) -> String {
    let mut s = String::from("uid:");
    push_u64(&mut s, uid as u64);
    s
}

/// Format the ACL identity string for a service key (`service:<n>`).
pub fn service_identity(key: u64) -> String {
    let mut s = String::from("service:");
    push_u64(&mut s, key);
    s
}

fn push_u64(out: &mut String, value: u64) {
    use core::fmt::Write;
    let _ = write!(out, "{}", value);
}

/// Access control list persisted with every key record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(Serialize, Deserialize))]
pub struct Acl {
    /// The identity that created the key. Owner implicitly has read and write.
    pub owner: String,
    /// Identities explicitly allowed to read the value. [`WORLD`] grants all.
    pub read: Vec<String>,
    /// Identities explicitly allowed to write (overwrite or delete).
    pub write: Vec<String>,
}

impl Acl {
    /// Create a minimal ACL for a first-time PUT by `owner`.
    /// Only the owner has read and write rights.
    pub fn new(owner: impl Into<String>) -> Self {
        let o = owner.into();
        Self {
            owner: o.clone(),
            read: vec![o.clone()],
            write: vec![o],
        }
    }

    /// Owner-only ACL for a concrete [`Caller`].
    pub fn for_caller(caller: &Caller) -> Self {
        Self::new(caller.identity())
    }

    /// A publicly readable key: any caller may read, only Root/Service may write.
    /// Used for TLS trust anchors and other non-secret published material.
    pub fn public_read(owner: &Caller) -> Self {
        let o = owner.identity();
        Self {
            read: vec![o.clone(), String::from(WORLD)],
            write: vec![String::from("root")],
            owner: o,
        }
    }

    /// True when this record is publicly readable by any caller.
    pub fn is_world_readable(&self) -> bool {
        self.read.iter().any(|r| r == WORLD)
    }

    /// Upgrade this ACL to world-readable, preserving the existing owner and
    /// any explicit grants. Used when an existing key is rewritten inside a
    /// public namespace: legacy records may still carry a `root`-only ACL, and
    /// this migration makes public material readable by every caller.
    pub fn ensure_public_read(&mut self) {
        if !self.is_world_readable() {
            self.read.push(String::from(WORLD));
        }
    }

    fn matches_identity(&self, caller: &Caller) -> bool {
        let id = caller.identity();
        self.owner == id || self.read.iter().any(|r| *r == id)
    }

    fn write_matches_identity(&self, caller: &Caller) -> bool {
        let id = caller.identity();
        self.owner == id || self.write.iter().any(|w| *w == id)
    }

    /// Returns true if `caller` is allowed to read.
    ///
    /// Root and authenticated services act as administrators and bypass the
    /// per-key list, matching the legacy `"root"` admin bypass.
    pub fn allows_read(&self, caller: &Caller) -> bool {
        if caller.is_privileged() {
            return true;
        }
        if self.is_world_readable() {
            return true;
        }
        self.matches_identity(caller)
    }

    /// Returns true if `caller` is allowed to write or delete.
    pub fn allows_write(&self, caller: &Caller) -> bool {
        if caller.is_privileged() {
            return true;
        }
        self.write_matches_identity(caller)
    }

    /// Render the ACL identity of a caller; exposed for diagnostics.
    pub fn caller_identity(caller: &Caller) -> String {
        caller.identity()
    }
}

/// Structured KV error, mirroring the IPC `KV_ERROR` reason codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvError {
    /// The caller may not read or mutate this key.
    PermissionDenied,
    /// The key does not exist (or was deleted).
    NotFound,
    /// The request was malformed (empty key, bad SHM frame, …).
    InvalidRequest,
    /// An internal persistence/storage failure.
    Internal,
}

/// Coarse namespace classification from a key prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    /// `account:<uid>:*` — one authenticated user's private namespace.
    Account(u32),
    /// `vault:*` — cross-user secret material; Root/Service/broker only.
    Vault,
    /// Public read-only published material (`tls.*`, `pki.*`, `ca.*`, `pub.*`).
    Public,
    /// Ordinary key: governed solely by the per-key ACL.
    General,
}

/// Public read-only key prefixes. These are non-secret published namespaces:
/// TLS/PKI trust anchors. They are readable by any caller but writable only by
/// Root/Service. (Per-user calculator history stays user-owned, not public.)
pub const PUBLIC_PREFIXES: &[&str] = &["tls.", "pki.", "ca.", "pub."];

/// Returns true if `key` lives in a public read-only namespace.
pub fn is_public_key(key: &str) -> bool {
    PUBLIC_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// Classify a key into a [`Namespace`].
pub fn classify(key: &str) -> Namespace {
    if let Some(rest) = key.strip_prefix("account:") {
        if let Some((uid_str, _)) = rest.split_once(':') {
            if let Ok(uid) = uid_str.parse::<u32>() {
                return Namespace::Account(uid);
            }
        }
    }
    if key.starts_with("vault:") {
        return Namespace::Vault;
    }
    if is_public_key(key) {
        return Namespace::Public;
    }
    Namespace::General
}

/// Parse the uid out of an `account:<uid>:*` key, if present.
pub fn account_uid(key: &str) -> Option<u32> {
    match classify(key) {
        Namespace::Account(uid) => Some(uid),
        _ => None,
    }
}

/// Coarse namespace gate evaluated before the per-key ACL.
///
/// Returns `true` when the caller is permitted to *reach* the key at all; the
/// per-key ACL is still consulted afterwards for the read/write decision.
pub fn namespace_allows(caller: &Caller, key: &str) -> bool {
    match classify(key) {
        Namespace::Account(uid) => {
            caller.is_root()
                || caller.is_auth_broker()
                || matches!(caller, Caller::User(call_uid) if *call_uid == uid)
        }
        Namespace::Vault => caller.is_root() || caller.is_auth_broker(),
        Namespace::Public | Namespace::General => true,
    }
}

/// True when a caller may own (create/overwrite) keys in the key's namespace.
///
/// Public namespaces are writable only by Root/Service/TLS; accounts only by
/// their user (or an authorised broker); the vault only by Root/Service/broker.
pub fn namespace_allows_write(caller: &Caller, key: &str) -> bool {
    match classify(key) {
        Namespace::Public => caller.is_privileged() || caller.is_tls_service(),
        Namespace::Account(_) | Namespace::Vault => namespace_allows(caller, key),
        Namespace::General => true,
    }
}

/// Decide whether `caller` may read `key`, given the record's stored ACL.
///
/// Public-namespace keys are world-readable by definition (TLS/PKI trust
/// anchors). This is enforced at the policy layer rather than relying on the
/// stored ACL alone, so a *legacy* record created before this model existed —
/// which may still carry a `root`-only ACL — stays readable to every caller
/// after an upgrade, without requiring a rewrite-on-first-read migration.
///
/// All non-public namespaces fall back to the per-key ACL.
pub fn read_allowed(caller: &Caller, key: &str, acl: &Acl) -> bool {
    if is_public_key(key) {
        return true;
    }
    acl.allows_read(caller)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_cannot_read_other_user_key() {
        let owner = Caller::User(1001);
        let acl = Acl::for_caller(&owner);
        assert!(acl.allows_read(&owner));
        assert!(!acl.allows_read(&Caller::User(1000)));
        assert!(acl.allows_read(&Caller::Root));
    }

    #[test]
    fn user_cannot_write_root_key() {
        let acl = Acl::for_caller(&Caller::Root);
        assert!(matches!(
            {
                let c = Caller::User(1000);
                if acl.allows_write(&c) {
                    Ok(())
                } else {
                    Err(KvError::PermissionDenied)
                }
            },
            Err(KvError::PermissionDenied)
        ));
    }

    #[test]
    fn public_key_is_world_readable_but_not_user_writable() {
        let acl = Acl::public_read(&Caller::Service(TLS_SERVICE));
        assert!(acl.is_world_readable());
        assert!(acl.allows_read(&Caller::User(1000)));
        assert!(acl.allows_read(&Caller::Unknown));
        assert!(!acl.allows_write(&Caller::User(1000)));
        assert!(acl.allows_write(&Caller::Root));
    }

    #[test]
    fn account_namespace_isolates_users() {
        let k = "account:1000:token";
        assert_eq!(classify(k), Namespace::Account(1000));
        assert!(namespace_allows(&Caller::User(1000), k));
        assert!(!namespace_allows(&Caller::User(1001), k));
        assert!(namespace_allows(&Caller::Root, k));
        assert!(namespace_allows(&Caller::Service(AUTH_BROKER_SERVICE), k));
        assert!(!namespace_allows(&Caller::Service(TLS_SERVICE), k));
        assert!(namespace_allows_write(&Caller::User(1000), k));
        assert!(!namespace_allows_write(&Caller::User(1001), k));
    }

    #[test]
    fn vault_namespace_is_privileged_only() {
        assert_eq!(classify("vault:secret"), Namespace::Vault);
        assert!(!namespace_allows(&Caller::User(1000), "vault:secret"));
        assert!(namespace_allows(&Caller::Root, "vault:secret"));
        assert!(!namespace_allows_write(&Caller::User(1000), "vault:secret"));
    }

    #[test]
    fn acl_identity_round_trips() {
        for c in [
            Caller::Root,
            Caller::User(1000),
            Caller::Service(TLS_SERVICE),
        ] {
            assert_eq!(Caller::from_identity_str(&c.identity()), c);
        }
    }

    #[test]
    fn legacy_public_key_stays_world_readable() {
        // A pre-upgrade TLS trust anchor may carry a `root`-only ACL. Policy
        // must still grant read to every caller so the handshake path survives.
        let legacy = Acl::new("root");
        assert!(read_allowed(&Caller::User(1000), "tls.ca.roots", &legacy));
        assert!(read_allowed(&Caller::Unknown, "tls.ca.roots", &legacy));
        // The same root-only ACL on a private key denies a non-owner.
        assert!(!read_allowed(&Caller::User(1000), "secret", &legacy));
    }
}
