pub(crate) mod delivery;
mod lifecycle;
use delivery::{GrantDelivery, RecipientIdentity};
#[cfg(test)]
mod lifecycle_tests;
use crate::serial_println;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::structures::paging::{PhysFrame, Size4KiB};
use x86_64::PhysAddr;

#[cfg(feature = "cap_issuance_test")]
pub mod gate;

/// The category a token belongs to, encoded in its high tag bits.
///
/// This is what lets the kernel decide in `O(1)` — by masking the top two
/// bits of the token word — whether a presented token is an IPC capability or
/// a VFS capability, *before* it ever touches the broker tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapKind {
    /// IPC endpoint capability (send / receive / grant).
    Ipc,
    /// VFS object capability (read / write / edit / remove).
    Vfs,
    /// Legacy / special tokens minted before tagging (e.g. shared pages,
    /// `SPAWN_TOKEN`). Treated as type-agnostic.
    Untagged,
}

/// A capability token: opaque to user-space, meaningful to the kernel.
///
/// The low 62 bits are a fresh payload from the kernel's conditioned entropy
/// stream, checked against every active broker token; the
/// top two bits are a *type tag* (`TAG_IPC` / `TAG_VFS`). Type checks are then
/// a single shift+compare instead of a table walk.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CapabilityToken(pub u64);

impl CapabilityToken {
    pub const INVALID: Self = Self(0);

    /// Bit position of the 2-bit type tag.
    const TAG_SHIFT: u64 = 62;
    /// Mask covering the tag bits.
    const TAG_MASK: u64 = 0b11 << Self::TAG_SHIFT;
    /// Mask covering the random payload bits (62-bit bearer strength).
    const PAYLOAD_MASK: u64 = !Self::TAG_MASK;

    /// Tag value for IPC capabilities.
    pub const TAG_IPC: u64 = 0b01;
    /// Tag value for VFS capabilities.
    pub const TAG_VFS: u64 = 0b10;

    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Build a token from a raw payload plus a type tag.
    #[inline]
    const fn tagged(payload: u64, tag: u64) -> Self {
        Self((payload & Self::PAYLOAD_MASK) | (tag << Self::TAG_SHIFT))
    }

    /// Extract the type tag in `O(1)`.
    #[inline]
    pub const fn kind(self) -> CapKind {
        match (self.0 & Self::TAG_MASK) >> Self::TAG_SHIFT {
            x if x == Self::TAG_IPC => CapKind::Ipc,
            x if x == Self::TAG_VFS => CapKind::Vfs,
            _ => CapKind::Untagged,
        }
    }

    /// `true` if the token is tagged for IPC (or is untagged/legacy).
    #[inline]
    pub const fn is_ipc(self) -> bool {
        matches!(self.kind(), CapKind::Ipc | CapKind::Untagged)
    }

    /// `true` if the token is tagged for VFS (or is untagged/legacy).
    #[inline]
    pub const fn is_vfs(self) -> bool {
        matches!(self.kind(), CapKind::Vfs | CapKind::Untagged)
    }
}

pub use heapless::String;

/// What rights a capability grants. The variant *is* the capability class —
/// an IPC token can never be checked against VFS rights or vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityRights {
    /// Rights over an IPC endpoint.
    Ipc {
        can_send: bool,
        can_receive: bool,
        can_grant: bool,
    },
    /// Rights over a VFS object.
    Vfs {
        read: bool,
        write: bool,
        edit: bool,
        remove: bool,
    },
}

impl CapabilityRights {
    pub const SEND_RECV: Self = Self::Ipc {
        can_send: true,
        can_receive: true,
        can_grant: false,
    };

    pub const SEND_ONLY: Self = Self::Ipc {
        can_send: true,
        can_receive: false,
        can_grant: false,
    };

    pub const SEND: Self = Self::SEND_ONLY;

    pub const RECV_ONLY: Self = Self::Ipc {
        can_send: false,
        can_receive: true,
        can_grant: false,
    };

    /// IPC capability with the grant right — the prerequisite a delegating
    /// tool (e.g. `runas`) must hold to ask the broker to mint new tokens.
    pub const GRANT: Self = Self::Ipc {
        can_send: true,
        can_receive: false,
        can_grant: true,
    };

    /// Read-only VFS capability.
    pub const VFS_READ: Self = Self::Vfs {
        read: true,
        write: false,
        edit: false,
        remove: false,
    };

    /// Read+write VFS capability.
    pub const VFS_RW: Self = Self::Vfs {
        read: true,
        write: true,
        edit: false,
        remove: false,
    };

    /// Full VFS capability (read/write/edit/remove).
    pub const VFS_ALL: Self = Self::Vfs {
        read: true,
        write: true,
        edit: true,
        remove: true,
    };

    /// `true` if these rights describe an IPC capability.
    #[inline]
    pub const fn is_ipc(&self) -> bool {
        matches!(self, Self::Ipc { .. })
    }

    /// `true` if these rights describe a VFS capability.
    #[inline]
    pub const fn is_vfs(&self) -> bool {
        matches!(self, Self::Vfs { .. })
    }

    /// Destructure as IPC rights, or `None` if this is a VFS capability.
    #[inline]
    fn as_ipc(&self) -> Option<(bool, bool, bool)> {
        match self {
            Self::Ipc {
                can_send,
                can_receive,
                can_grant,
            } => Some((*can_send, *can_receive, *can_grant)),
            Self::Vfs { .. } => None,
        }
    }

    /// `true` if a token holding `self` satisfies a request for `wanted`.
    /// Both must be the same class; rights are monotone (held ⊇ wanted).
    fn satisfies(&self, wanted: &Self) -> bool {
        match (self, wanted) {
            (
                Self::Ipc {
                    can_send: hs,
                    can_receive: hr,
                    can_grant: hg,
                },
                Self::Ipc {
                    can_send: ws,
                    can_receive: wr,
                    can_grant: wg,
                },
            ) => (!*ws || *hs) && (!*wr || *hr) && (!*wg || *hg),
            (
                Self::Vfs {
                    read: hr,
                    write: hw,
                    edit: he,
                    remove: hrm,
                },
                Self::Vfs {
                    read: wr,
                    write: ww,
                    edit: we,
                    remove: wrm,
                },
            ) => (!*wr || *hr) && (!*ww || *hw) && (!*we || *he) && (!*wrm || *hrm),
            // Cross-class never satisfies — the core security invariant.
            _ => false,
        }
    }
}

/// Capability rights for VFS object access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessFlags {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl AccessFlags {
    pub const READ: Self = Self {
        read: true,
        write: false,
        execute: false,
    };

    pub const READ_WRITE: Self = Self {
        read: true,
        write: true,
        execute: false,
    };

    pub const READ_EXECUTE: Self = Self {
        read: true,
        write: false,
        execute: true,
    };

    pub const ALL: Self = Self {
        read: true,
        write: true,
        execute: true,
    };
}

/// VFS-relevant capability data.
#[derive(Debug, Clone)]
pub struct VfsCapability {
    pub allowed_prefix: String<64>,
    pub flags: AccessFlags,
}

/// Kernel-managed shared memory object. Backing frames need not be physically contiguous;
/// they are mapped contiguously into each client's chosen virtual window.
#[derive(Debug)]
pub struct ShmObject {
    pub frames: alloc::vec::Vec<PhysFrame<Size4KiB>>,
    pub size: usize, // requested/rounded size in bytes
}

/// Entry in the shared-region table.
struct ShmEntry {
    token: CapabilityToken,
    obj: ShmObject,
    owner_pid: usize,
    // Counts views, including multiple views in one address space. Retirement
    // fences admission without discarding backing needed by existing views.
    lifetime: crate::memory::shared_lifetime::SharedLifetime,
}

/// Errors from the capability broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapError {
    InvalidToken,
    InsufficientRights,
    EndpointNotFound,
    /// Token does not exist in the broker's table at all (forged/guessed).
    NotFound,
    /// Token existed but has been revoked (e.g. owning process exited).
    Revoked,
    /// Not the trusted capability-broker requesting kernel minting.
    InvalidCaller,
    /// No more token-capability slots in kernel table.
    CapabilityStoreFull,
    EntropyUnavailable,
    TokenCollisionLimit,
    EndpointIdExhausted,
}

/// Bounded active authority, including all token classes. No history registry.
const MAX_ACTIVE_TOKENS: usize = 8192;
const MAX_ENDPOINTS: usize = 4096;
const TOKEN_ATTEMPTS: usize = 16;

#[derive(Debug, Default, Clone, Copy)]
pub struct IssuanceStats {
    pub prepared: u64,
    pub collisions: u64,
    pub entropy_failures: u64,
    pub retry_failures: u64,
}

/// Holds the exclusive broker borrow (and therefore its caller's lock guard)
/// from reservation through publication. Dropping it publishes no authority.
#[must_use]
pub(crate) struct PreparedSharedRegion<'a> {
    broker: &'a mut CapabilityBroker,
    token: CapabilityToken,
}

impl PreparedSharedRegion<'_> {
    /// Infallible commit after backing and the complete owner mapping exist.
    pub(crate) fn commit(
        self,
        frames: alloc::vec::Vec<PhysFrame<Size4KiB>>,
        size: usize,
        owner_pid: usize,
    ) -> CapabilityToken {
        assert!(!self.broker.token_in_use(self.token));
        assert!(self.broker.shared_regions.len() < self.broker.shared_regions.capacity());
        self.broker.shared_regions.push(ShmEntry {
            token: self.token,
            obj: ShmObject { frames, size },
            owner_pid,
            lifetime: crate::memory::shared_lifetime::SharedLifetime::new(),
        });
        self.token
    }
}

/// Global capability broker instance.
pub static CAP_BROKER: spin::Mutex<CapabilityBroker> = spin::Mutex::new(CapabilityBroker::new());

/// Global spawn capability token. A hardcoded special token that the kernel
/// recognizes in `ipc_call` to handle spawn requests directly.
pub const SPAWN_TOKEN: CapabilityToken = CapabilityToken(0xCAFEBABE_DEADBEEF);

/// Legacy VFS mint slot constraint; never sufficient authorization.
pub const CAPABILITY_BROKER_PID: u32 = 6;

/// A single-use authorization emitted by UAC after password verification.
/// Unlike an IPC capability it cannot be used to contact arbitrary services;
/// the kernel consumes it only for the authenticated-session spawn operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthSessionGrant {
    pub token: CapabilityToken,
    pub owner_pid: usize,
    pub uid: u32,
    pub gid: u32,
    pub expires_at_tick: u64,
    delivery: Option<GrantDelivery>,
}

/// The capability broker manages endpoints and capability tokens.
struct AccountGrant {
    grant: sunlight_ipc::accounts::Grant,
    delivery: Option<GrantDelivery>,
}

pub struct CapabilityBroker {
    next_endpoint_id: u32,
    endpoints: alloc::vec::Vec<Endpoint>,
    capabilities: alloc::vec::Vec<(CapabilityToken, u32, CapabilityRights)>,
    shared_regions: alloc::vec::Vec<ShmEntry>,
    vfs_caps: alloc::vec::Vec<(CapabilityToken, VfsCapability, usize)>,
    auth_session_grants: alloc::vec::Vec<AuthSessionGrant>,
    account_grants: alloc::vec::Vec<(CapabilityToken, AccountGrant)>,
    issuance: IssuanceStats,
    // Instance-local injection exists only in host tests and the opt-in native
    // security gate. The global production broker never installs an override.
    #[cfg(any(test, feature = "cap_issuance_test"))]
    test_words: Option<alloc::collections::VecDeque<Option<u64>>>,
    #[cfg(any(test, feature = "cap_issuance_test"))]
    test_reservation_failure: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CapabilityDiagnosticSnapshot {
    pub public_send_derivations: u64,
    pub rejected_rights_escalations: u64,
}

static PUBLIC_SEND_DERIVATIONS: AtomicU64 = AtomicU64::new(0);
static REJECTED_RIGHTS_ESCALATIONS: AtomicU64 = AtomicU64::new(0);

pub fn diagnostic_snapshot() -> CapabilityDiagnosticSnapshot {
    CapabilityDiagnosticSnapshot {
        public_send_derivations: PUBLIC_SEND_DERIVATIONS.load(Ordering::Relaxed),
        rejected_rights_escalations: REJECTED_RIGHTS_ESCALATIONS.load(Ordering::Relaxed),
    }
}

impl CapabilityBroker {
    pub const fn new() -> Self {
        Self {
            next_endpoint_id: 0,
            endpoints: alloc::vec::Vec::new(),
            capabilities: alloc::vec::Vec::new(),
            shared_regions: alloc::vec::Vec::new(),
            vfs_caps: alloc::vec::Vec::new(),
            auth_session_grants: alloc::vec::Vec::new(),
            account_grants: alloc::vec::Vec::new(),
            issuance: IssuanceStats {
                prepared: 0,
                collisions: 0,
                entropy_failures: 0,
                retry_failures: 0,
            },
            #[cfg(any(test, feature = "cap_issuance_test"))]
            test_words: None,
            #[cfg(any(test, feature = "cap_issuance_test"))]
            test_reservation_failure: false,
        }
    }

    pub fn issuance_stats(&self) -> IssuanceStats {
        self.issuance
    }

    fn active_tokens(&self) -> usize {
        self.capabilities.len()
            + self.shared_regions.len()
            + self.vfs_caps.len()
            + self.auth_session_grants.len()
            + self.account_grants.len()
    }

    fn check_capacity(&self, additional: usize) -> Result<(), CapError> {
        #[cfg(any(test, feature = "cap_issuance_test"))]
        if self.test_reservation_failure {
            return Err(CapError::CapabilityStoreFull);
        }
        if additional > MAX_ACTIVE_TOKENS.saturating_sub(self.active_tokens()) {
            return Err(CapError::CapabilityStoreFull);
        }
        Ok(())
    }

    fn token_in_use(&self, token: CapabilityToken) -> bool {
        self.capabilities.iter().any(|(t, _, _)| *t == token)
            || self.shared_regions.iter().any(|entry| entry.token == token)
            || self.vfs_caps.iter().any(|(t, _, _)| *t == token)
            || self
                .auth_session_grants
                .iter()
                .any(|grant| grant.token == token)
            || self.account_grants.iter().any(|(t, _)| *t == token)
    }

    fn issue_token(&mut self, tag: u64) -> Result<CapabilityToken, CapError> {
        for _ in 0..TOKEN_ATTEMPTS {
            #[cfg(any(test, feature = "cap_issuance_test"))]
            let word = match self.test_words.as_mut() {
                Some(words) => words.pop_front().flatten(),
                None => crate::entropy::next_u64(),
            };
            #[cfg(not(any(test, feature = "cap_issuance_test")))]
            let word = crate::entropy::next_u64();
            let Some(word) = word else {
                self.issuance.entropy_failures = self.issuance.entropy_failures.saturating_add(1);
                return Err(CapError::EntropyUnavailable);
            };
            let token = CapabilityToken::tagged(word, tag);
            if word & CapabilityToken::PAYLOAD_MASK == 0
                || word & CapabilityToken::PAYLOAD_MASK
                    == SPAWN_TOKEN.as_u64() & CapabilityToken::PAYLOAD_MASK
                || self.token_in_use(token)
            {
                self.issuance.collisions = self.issuance.collisions.saturating_add(1);
                continue;
            }
            self.issuance.prepared = self.issuance.prepared.saturating_add(1);
            return Ok(token);
        }
        self.issuance.retry_failures = self.issuance.retry_failures.saturating_add(1);
        Err(CapError::TokenCollisionLimit)
    }

    #[cfg(any(test, feature = "cap_issuance_test"))]
    pub(crate) fn test_entropy(&mut self, words: &[Option<u64>]) {
        self.test_words = Some(words.iter().copied().collect());
    }

    #[cfg(any(test, feature = "cap_issuance_test"))]
    pub(crate) fn test_fail_reservation(&mut self, fail: bool) {
        self.test_reservation_failure = fail;
    }

    fn mint_account_grant(
        &mut self,
        binding: sunlight_ipc::accounts::GrantBinding,
        now: u64,
    ) -> Option<CapabilityToken> {
        self.account_grants
            .retain(|(_, grant)| !grant.grant.used && now < grant.grant.expires);
        if self.account_grants.len() >= 64 {
            return None;
        }
        self.check_capacity(1).ok()?;
        self.account_grants.try_reserve(1).ok()?;
        let token = self.issue_token(CapabilityToken::TAG_IPC).ok()?;
        #[cfg(all(feature = "cap_issuance_test", target_os = "none"))]
        assert!(
            crate::sched::SCHEDULER.try_lock().is_none(),
            "account commit escaped lifecycle serialization"
        );
        self.account_grants.push((
            token,
            AccountGrant {
                grant: sunlight_ipc::accounts::Grant {
                    binding,
                    expires: now.saturating_add(500),
                    used: false,
                },
                delivery: None,
            },
        ));
        Some(token)
    }

    #[cfg(test)]
    pub fn consume_account_grant(
        &mut self,
        token: CapabilityToken,
        binding: sunlight_ipc::accounts::GrantBinding,
        now: u64,
    ) -> bool {
        let Some(index) = self.account_grants.iter().position(|(t, _)| *t == token) else {
            return false;
        };
        let (_, mut grant) = self.account_grants.swap_remove(index);
        grant.delivery.is_none() && grant.grant.consume(binding, now)
    }

    fn mint_auth_session_grant(
        &mut self,
        owner_pid: usize,
        uid: u32,
        gid: u32,
        expires_at_tick: u64,
    ) -> Option<CapabilityToken> {
        if self.auth_session_grants.len() >= 64 {
            return None;
        }
        self.check_capacity(1).ok()?;
        self.auth_session_grants.try_reserve(1).ok()?;
        let token = self.issue_token(CapabilityToken::TAG_IPC).ok()?;
        #[cfg(all(feature = "cap_issuance_test", target_os = "none"))]
        assert!(
            crate::sched::SCHEDULER.try_lock().is_none(),
            "auth commit escaped lifecycle serialization"
        );
        self.auth_session_grants.push(AuthSessionGrant {
            token,
            owner_pid,
            uid,
            gid,
            expires_at_tick,
            delivery: None,
        });
        Some(token)
    }

    /// Consume a grant exactly once. Expired and mismatched grants are removed
    /// as well, so a credential cannot be replayed after a failed attempt.
    #[cfg(test)]
    pub fn consume_auth_session_grant(
        &mut self,
        token: CapabilityToken,
        owner_pid: usize,
        now_tick: u64,
    ) -> Option<(u32, u32)> {
        let idx = self
            .auth_session_grants
            .iter()
            .position(|grant| grant.token == token)?;
        let grant = self.auth_session_grants.swap_remove(idx);
        if grant.delivery.is_some()
            || grant.owner_pid != owner_pid
            || now_tick > grant.expires_at_tick
        {
            return None;
        }
        Some((grant.uid, grant.gid))
    }

    pub(crate) fn consume_live_auth_grant(
        &mut self,
        token: CapabilityToken,
        recipient: RecipientIdentity,
        now: u64,
    ) -> Option<(u32, u32)> {
        let idx = self
            .auth_session_grants
            .iter()
            .position(|g| g.token == token)?;
        let g = self.auth_session_grants.swap_remove(idx);
        (g.delivery.is_some_and(|d| d.usable(recipient)) && now <= g.expires_at_tick)
            .then_some((g.uid, g.gid))
    }

    pub(crate) fn consume_live_account_grant(
        &mut self,
        token: CapabilityToken,
        expected: GrantDelivery,
        binding: sunlight_ipc::accounts::GrantBinding,
        now: u64,
    ) -> bool {
        let Some(idx) = self.account_grants.iter().position(|(t, _)| *t == token) else {
            return false;
        };
        let (_, mut g) = self.account_grants.swap_remove(idx);
        g.delivery
            .is_some_and(|d| d.usable(expected.recipient) || d == expected)
            && g.grant.consume(binding, now)
    }

    /// Called with lifecycle and broker serialization. Successful transport
    /// delivers only listed tokens to this exact VM; omitted pending grants are
    /// retired too. No user copy or blocking transport runs under this helper.
    pub(crate) fn settle_grant_delivery(
        &mut self,
        pid: usize,
        incarnation: u64,
        request: u64,
        recipient: Option<RecipientIdentity>,
        tokens: &[u64],
    ) {
        fn keep(
            d: &mut Option<GrantDelivery>,
            token: CapabilityToken,
            pid: usize,
            incarnation: u64,
            request: u64,
            recipient: Option<RecipientIdentity>,
            tokens: &[u64],
        ) -> bool {
            let Some(d) = d else {
                return true;
            };
            if d.delivered || !d.matches(pid, incarnation, request) {
                return true;
            }
            if recipient == Some(d.recipient) && tokens.contains(&token.0) {
                d.delivered = true;
                true
            } else {
                false
            }
        }
        self.auth_session_grants.retain_mut(|g| {
            keep(
                &mut g.delivery,
                g.token,
                pid,
                incarnation,
                request,
                recipient,
                tokens,
            )
        });
        self.account_grants.retain_mut(|(t, g)| {
            keep(
                &mut g.delivery,
                *t,
                pid,
                incarnation,
                request,
                recipient,
                tokens,
            )
        });
    }

    pub(crate) fn prepare_shared_region(&mut self) -> Result<PreparedSharedRegion<'_>, CapError> {
        self.check_capacity(1)?;
        self.shared_regions
            .try_reserve(1)
            .map_err(|_| CapError::CapabilityStoreFull)?;
        let token = self.issue_token(CapabilityToken::TAG_VFS)?;
        Ok(PreparedSharedRegion {
            broker: self,
            token,
        })
    }

    pub fn shared_region_count(&self) -> usize {
        self.shared_regions.len()
    }

    pub fn shared_region_map_count(&self, token: CapabilityToken) -> Option<usize> {
        self.shared_regions
            .iter()
            .find(|entry| entry.token == token)
            .map(|entry| entry.lifetime.views())
    }

    /// Create a new endpoint, return its id and a send+recv capability to the owner.
    pub fn create_endpoint(
        &mut self,
        owner_pid: usize,
    ) -> Result<(u32, CapabilityToken), CapError> {
        // IDs are never reused. This prevents a delayed IRQ/deadline belonging
        // to a destroyed endpoint instance from addressing a later endpoint.
        let id = self.next_endpoint_id;
        let next_id = self
            .next_endpoint_id
            .checked_add(1)
            .ok_or(CapError::EndpointIdExhausted)?;
        if self.endpoints.len() >= MAX_ENDPOINTS {
            return Err(CapError::CapabilityStoreFull);
        }
        self.check_capacity(1)?;
        self.endpoints
            .try_reserve(1)
            .map_err(|_| CapError::CapabilityStoreFull)?;
        self.capabilities
            .try_reserve(1)
            .map_err(|_| CapError::CapabilityStoreFull)?;
        let token = self.issue_token(CapabilityToken::TAG_IPC)?;
        self.next_endpoint_id = next_id;
        self.endpoints.push(Endpoint { id, owner_pid });
        self.capabilities
            .push((token, id, CapabilityRights::SEND_RECV));
        serial_println!("[CAP] Created endpoint {} for pid={}", id, owner_pid);
        Ok((id, token))
    }

    /// Validate that `token` grants at least the requested rights for its IPC
    /// endpoint.
    ///
    /// Strict type validation, cheapest checks first:
    ///   1. `O(1)` tag check — the request must be IPC and the token must not
    ///      be tagged as a VFS capability.
    ///   2. table walk — the stored rights must satisfy the requested rights,
    ///      and (defensively) both must be the same class.
    pub fn check(&self, token: CapabilityToken, rights: CapabilityRights) -> Result<u32, CapError> {
        // (1) Type gate: `check` resolves IPC endpoints only.
        if !rights.is_ipc() {
            return Err(CapError::InsufficientRights);
        }
        if !token.is_ipc() {
            return Err(CapError::InvalidToken);
        }

        // (2) Authorization: locate the token and compare rights.
        for (t, ep_id, r) in &self.capabilities {
            if *t == token {
                if r.satisfies(&rights) {
                    return Ok(*ep_id);
                }
                return Err(CapError::InsufficientRights);
            }
        }
        Err(CapError::InvalidToken)
    }

    /// Mint a new IPC capability token for an existing endpoint.
    ///
    /// Refuses to mint if `rights` are not IPC rights, keeping the IPC and VFS
    /// capability spaces strictly disjoint.
    pub fn mint(
        &mut self,
        endpoint_id: u32,
        rights: CapabilityRights,
    ) -> Result<CapabilityToken, CapError> {
        if !rights.is_ipc() {
            return Err(CapError::InsufficientRights);
        }
        if !self.endpoints.iter().any(|e| e.id == endpoint_id) {
            return Err(CapError::EndpointNotFound);
        }
        self.check_capacity(1)?;
        self.capabilities
            .try_reserve(1)
            .map_err(|_| CapError::CapabilityStoreFull)?;
        let token = self.issue_token(CapabilityToken::TAG_IPC)?;
        self.capabilities.push((token, endpoint_id, rights));
        Ok(token)
    }

    /// Derive an attenuated IPC capability from `source` without modifying it.
    ///
    /// The requested rights must be a subset of the source rights and the new
    /// token always retains the source endpoint identity. Exact-rights tokens
    /// are reused, so publishing an endpoint does not allocate per LOOKUP.
    pub fn derive(
        &mut self,
        source: CapabilityToken,
        requested: CapabilityRights,
    ) -> Result<CapabilityToken, CapError> {
        if !source.is_ipc() || !requested.is_ipc() {
            REJECTED_RIGHTS_ESCALATIONS.fetch_add(1, Ordering::Relaxed);
            return Err(CapError::InvalidToken);
        }
        let (endpoint_id, source_rights) = self
            .capabilities
            .iter()
            .find_map(|(token, endpoint_id, rights)| {
                (*token == source).then_some((*endpoint_id, *rights))
            })
            .ok_or(CapError::InvalidToken)?;
        if !source_rights.satisfies(&requested) {
            REJECTED_RIGHTS_ESCALATIONS.fetch_add(1, Ordering::Relaxed);
            return Err(CapError::InsufficientRights);
        }
        if !self
            .endpoints
            .iter()
            .any(|endpoint| endpoint.id == endpoint_id)
        {
            return Err(CapError::EndpointNotFound);
        }
        if let Some(token) = self.capabilities.iter().find_map(|(token, id, rights)| {
            (*id == endpoint_id && *rights == requested).then_some(*token)
        }) {
            return Ok(token);
        }

        self.check_capacity(1)?;
        self.capabilities
            .try_reserve(1)
            .map_err(|_| CapError::CapabilityStoreFull)?;
        let token = self.issue_token(CapabilityToken::TAG_IPC)?;
        self.capabilities.push((token, endpoint_id, requested));
        if requested == CapabilityRights::SEND_ONLY {
            PUBLIC_SEND_DERIVATIONS.fetch_add(1, Ordering::Relaxed);
        }
        Ok(token)
    }

    /// Return an existing token for an endpoint if one already has the rights.
    pub fn token_for_endpoint(
        &self,
        endpoint_id: u32,
        rights: CapabilityRights,
    ) -> Option<CapabilityToken> {
        if !rights.is_ipc() {
            return None;
        }
        self.capabilities
            .iter()
            .find_map(|(token, id, token_rights)| {
                if *id != endpoint_id {
                    return None;
                }
                if !token_rights.satisfies(&rights) {
                    return None;
                }
                Some(*token)
            })
    }

    /// Return an existing token for the first endpoint owned by `owner_pid`
    /// that satisfies the requested IPC rights.
    pub fn token_for_owner_endpoint(
        &self,
        owner_pid: usize,
        rights: CapabilityRights,
    ) -> Option<CapabilityToken> {
        if !rights.is_ipc() {
            return None;
        }
        self.capabilities
            .iter()
            .find_map(|(token, endpoint_id, token_rights)| {
                if !token_rights.satisfies(&rights) {
                    return None;
                }
                let owner = self.endpoint_owner(*endpoint_id)?;
                if owner == owner_pid {
                    Some(*token)
                } else {
                    None
                }
            })
    }

    /// Revoke a capability token.
    pub fn revoke(&mut self, token: CapabilityToken) {
        if let Some(idx) = self.capabilities.iter().position(|(t, _, _)| *t == token) {
            self.capabilities.swap_remove(idx);
            serial_println!("[CAP] Revoked IPC capability");
        }
    }

    /// Get endpoint owner.
    pub fn endpoint_owner(&self, endpoint_id: u32) -> Option<usize> {
        self.endpoints
            .iter()
            .find(|e| e.id == endpoint_id)
            .map(|e| e.owner_pid)
    }

    pub fn endpoint_is_live(&self, token: CapabilityToken, endpoint_id: u32) -> bool {
        self.check(token, CapabilityRights::SEND) == Ok(endpoint_id)
            && self.endpoint_owner(endpoint_id).is_some()
    }

    /// Destroy an endpoint using receive/owner authority and revoke every token
    /// derived for that endpoint.
    pub fn destroy_endpoint(
        &mut self,
        caller_pid: usize,
        owner_token: CapabilityToken,
    ) -> Result<u32, CapError> {
        let endpoint_id = self.check(owner_token, CapabilityRights::RECV_ONLY)?;
        if self.endpoint_owner(endpoint_id) != Some(caller_pid) {
            return Err(CapError::InsufficientRights);
        }
        self.endpoints.retain(|endpoint| endpoint.id != endpoint_id);
        self.capabilities
            .retain(|(_, capability_endpoint, _)| *capability_endpoint != endpoint_id);
        crate::memory::security::revoke_display_authority_for_endpoint(endpoint_id);
        Ok(endpoint_id)
    }

    /// Resolve a token to its endpoint owner after checking rights.
    pub fn token_owner(
        &self,
        token: CapabilityToken,
        rights: CapabilityRights,
    ) -> Result<(u32, usize), CapError> {
        let endpoint_id = self.check(token, rights)?;
        let owner = self
            .endpoint_owner(endpoint_id)
            .ok_or(CapError::EndpointNotFound)?;
        Ok((endpoint_id, owner))
    }

    pub fn debug_resolve_ipc(
        &self,
        token: CapabilityToken,
        rights: CapabilityRights,
    ) -> Option<(u32, usize, CapabilityRights)> {
        if !rights.is_ipc() || !token.is_ipc() {
            return None;
        }
        self.capabilities
            .iter()
            .find_map(|(t, endpoint_id, token_rights)| {
                if *t != token || !token_rights.satisfies(&rights) {
                    return None;
                }
                let owner = self.endpoint_owner(*endpoint_id)?;
                Some((*endpoint_id, owner, *token_rights))
            })
    }

    pub fn debug_endpoints(&self) -> alloc::vec::Vec<(u32, usize)> {
        self.endpoints
            .iter()
            .map(|endpoint| (endpoint.id, endpoint.owner_pid))
            .collect()
    }

    pub fn endpoints_owned_by(&self, owner_pid: usize) -> alloc::vec::Vec<u32> {
        self.endpoints
            .iter()
            .filter(|endpoint| endpoint.owner_pid == owner_pid)
            .map(|endpoint| endpoint.id)
            .collect()
    }

    pub fn revoke_endpoints_owned_by(&mut self, owner_pid: usize) {
        let endpoint_ids = self.endpoints_owned_by(owner_pid);
        self.endpoints
            .retain(|endpoint| endpoint.owner_pid != owner_pid);
        self.capabilities
            .retain(|(_, endpoint_id, _)| !endpoint_ids.iter().any(|id| id == endpoint_id));
        // VFS tokens are transferable bearers, with no existing owner-death
        // revocation contract. Preserve that lifetime; the bound remains 64.
        self.retire_recipient_grants(owner_pid);
        crate::memory::security::revoke_display_authority_for_owner(owner_pid);
    }

    #[cfg(feature = "cap_issuance_test")]
    pub(crate) fn recipient_grant_count(&self, pid: usize) -> usize {
        self.auth_session_grants
            .iter()
            .filter(|r| r.owner_pid == pid)
            .count()
            + self
                .account_grants
                .iter()
                .filter(|(_, r)| r.grant.binding.pid == pid as u64)
                .count()
    }

    /// Called under lifecycle serialization on exit and successful VM replacement.
    /// Does not revoke transferable VFS/SHM or endpoint authority.
    pub(crate) fn retire_recipient_grants(&mut self, owner_pid: usize) {
        crate::delegated_caller::DELEGATED_CALLERS
            .lock()
            .retire_process(owner_pid);
        self.auth_session_grants.retain(|grant| {
            grant.owner_pid != owner_pid
                && !grant
                    .delivery
                    .is_some_and(|d| !d.delivered && d.issuer_pid == owner_pid)
        });
        self.account_grants.retain(|(_, grant)| {
            grant.grant.binding.pid != owner_pid as u64
                && !grant
                    .delivery
                    .is_some_and(|d| !d.delivered && d.issuer_pid == owner_pid)
        });
    }

    /// Mint a capability token granting access to map a shared physical frame (single-page compat).
    pub fn mint_shared_page(
        &mut self,
        phys: PhysAddr,
        owner_pid: usize,
    ) -> Result<CapabilityToken, CapError> {
        let prepared = self.prepare_shared_region()?;
        let frame = unsafe { PhysFrame::<Size4KiB>::from_start_address_unchecked(phys) };
        let mut frames = alloc::vec::Vec::new();
        frames
            .try_reserve_exact(1)
            .map_err(|_| CapError::CapabilityStoreFull)?;
        frames.push(frame);
        Ok(prepared.commit(frames, 4096, owner_pid))
    }

    /// Resolve a shared-page capability token to its physical frame (first page for compat / 1-page regions).
    pub fn resolve_shared_page(&self, token: CapabilityToken) -> Option<PhysAddr> {
        self.resolve_shared_region(token)
            .and_then(|obj| obj.frames.first().map(|f| f.start_address()))
    }

    /// Resolve a (possibly multi-page) shared region capability to its object.
    pub fn resolve_shared_region(&self, token: CapabilityToken) -> Option<&ShmObject> {
        self.shared_regions.iter().find_map(|e| {
            if e.token == token && !e.lifetime.is_retired() {
                Some(&e.obj)
            } else {
                None
            }
        })
    }

    /// Cleanup authority comes from the task's existing mapping record, not
    /// admission through a still-live token. Never use this to create a view.
    pub(crate) fn shared_region_for_cleanup(&self, token: CapabilityToken) -> Option<&ShmObject> {
        self.shared_regions
            .iter()
            .find(|entry| entry.token == token)
            .map(|entry| &entry.obj)
    }

    pub(crate) fn preflight_shared_mapping(&self, token: CapabilityToken) -> bool {
        self.shared_regions
            .iter()
            .find(|entry| entry.token == token)
            .is_some_and(|entry| entry.lifetime.preflight_add().is_ok())
    }

    /// Validate a shared-page token, distinguishing "never existed" (forged/guessed)
    /// from "existed but revoked" (owner exited). Used by security self-tests and
    /// can be used by callers that need to report the precise rejection reason.
    pub fn validate_shared_page(&self, token: CapabilityToken) -> Result<PhysAddr, CapError> {
        let entry = self
            .shared_regions
            .iter()
            .find(|e| e.token == token)
            .ok_or(CapError::NotFound)?;
        if entry.lifetime.is_retired() {
            return Err(CapError::Revoked);
        }
        entry
            .obj
            .frames
            .first()
            .map(|f| f.start_address())
            .ok_or(CapError::NotFound)
    }

    /// Increment the mapping reference count for `token`.
    /// Called each time a process maps the region into its address space.
    pub fn increment_map_count(&mut self, token: CapabilityToken) {
        self.shared_regions
            .iter_mut()
            .find(|e| e.token == token)
            .expect("SHM mapping committed without backing metadata")
            .lifetime
            .add()
            .expect("SHM mapping admission changed under broker lock");
    }

    /// Decrement the mapping reference count for `token`.
    /// Returns the backing frames when this was the last mapping, so the caller
    /// can free the physical memory via the PMM.
    pub fn decrement_map_count(
        &mut self,
        token: CapabilityToken,
    ) -> Option<alloc::vec::Vec<PhysFrame<Size4KiB>>> {
        let idx = self
            .shared_regions
            .iter()
            .position(|e| e.token == token)
            .expect("SHM release lost backing metadata");
        if self.shared_regions[idx]
            .lifetime
            .release()
            .expect("SHM mapping count underflow")
        {
            let entry = self.shared_regions.swap_remove(idx);
            return Some(entry.obj.frames);
        }
        None
    }

    /// Fence new mappings; this is retirement, NOT active PTE revocation.
    /// Existing views keep the entry and frames pinned until verified unmap.
    /// Only a zero-view object transfers its backing to the caller here.
    #[must_use = "unmapped backing must be returned to the PMM by the caller"]
    pub fn revoke_shared(
        &mut self,
        token: CapabilityToken,
    ) -> Option<alloc::vec::Vec<PhysFrame<Size4KiB>>> {
        if let Some(idx) = self.shared_regions.iter().position(|e| e.token == token) {
            if self.shared_regions[idx].lifetime.retire() {
                return Some(self.shared_regions.swap_remove(idx).obj.frames);
            }
        }
        None
    }

    /// Mark all shared region grants owned by `pid` as revoked, without removing
    /// them from the table.
    pub fn revoke_all_for(&mut self, pid: usize) {
        for entry in self.shared_regions.iter_mut() {
            if entry.owner_pid == pid {
                entry.lifetime.retire();
            }
        }
    }

    /// Kernel-side VFS capability issuance and storage.
    pub fn grant_vfs_capability(
        &mut self,
        owner_pid: usize,
        cap: VfsCapability,
    ) -> Result<CapabilityToken, CapError> {
        const CAP_VFS_LIMIT: usize = 64;
        if self.vfs_caps.len() >= CAP_VFS_LIMIT {
            return Err(CapError::CapabilityStoreFull);
        }
        self.check_capacity(1)?;
        self.vfs_caps
            .try_reserve(1)
            .map_err(|_| CapError::CapabilityStoreFull)?;
        let token = self.issue_token(CapabilityToken::TAG_VFS)?;
        self.vfs_caps.push((token, cap, owner_pid));
        Ok(token)
    }

    /// Resolve an issued VFS capability token for runtime checks.
    pub fn resolve_vfs_capability(&self, token: CapabilityToken) -> Option<(VfsCapability, usize)> {
        // O(1) reject of anything not tagged as a VFS capability.
        if !token.is_vfs() {
            return None;
        }
        self.vfs_caps.iter().find_map(|(t, cap, pid)| {
            if *t == token {
                Some((cap.clone(), *pid))
            } else {
                None
            }
        })
    }

    /// Check whether an issued VFS capability `token` authorizes `access` on
    /// `path` via directory-prefix match. The cheap tag reject in
    /// `resolve_vfs_capability` runs first, so a forged or IPC-tagged token is
    /// rejected in `O(1)`.
    pub fn vfs_allows(&self, token: CapabilityToken, path: &str, access: AccessFlags) -> bool {
        match self.resolve_vfs_capability(token) {
            Some((cap, _)) => {
                path.starts_with(cap.allowed_prefix.as_str())
                    && (!access.read || cap.flags.read)
                    && (!access.write || cap.flags.write)
                    && (!access.execute || cap.flags.execute)
            }
            None => false,
        }
    }

    /// Policy: mint the base set of VFS capabilities every *elevated* session
    /// (e.g. a `runas`-spawned root shell) needs to run standard tooling.
    ///
    /// Each entry is a **directory-level** capability — `allowed_prefix` of
    /// `/bin` acts as the `/bin/*` execution wildcard the task calls for —
    /// granting read+execute over the standard binary directories. Changing the
    /// UID to 0 is not enough in a capability-first model; the session must
    /// actually carry these tokens, so `runas` requests them and passes them
    /// down to the spawned context.
    pub fn grant_elevated_vfs(
        &mut self,
        owner_pid: usize,
    ) -> Result<heapless::Vec<CapabilityToken, ELEVATED_EXEC_PREFIX_COUNT>, CapError> {
        let mut out = heapless::Vec::new();
        let original_len = self.vfs_caps.len();
        for prefix in ELEVATED_EXEC_PREFIXES {
            let mut allowed_prefix = String::new();
            if allowed_prefix.push_str(prefix).is_err() {
                self.vfs_caps.truncate(original_len);
                return Err(CapError::CapabilityStoreFull);
            }
            let cap = VfsCapability {
                allowed_prefix,
                flags: AccessFlags::READ_EXECUTE,
            };
            let token = match self.grant_vfs_capability(owner_pid, cap) {
                Ok(token) => token,
                Err(error) => {
                    self.vfs_caps.truncate(original_len);
                    return Err(error);
                }
            };
            // Vec capacity == prefix count, so this push cannot overflow.
            let _ = out.push(token);
        }
        serial_println!(
            "[CAP] Granted {} base exec capabilities to elevated pid={}",
            out.len(),
            owner_pid
        );
        Ok(out)
    }
}

/// Standard executable directories an elevated session is granted read+execute
/// access to. Directory-prefix entries behave as `/bin/*` wildcards.
pub const ELEVATED_EXEC_PREFIXES: [&str; 2] = ["/bin", "/usr/bin"];
/// Number of base execute capabilities minted per elevated session.
pub const ELEVATED_EXEC_PREFIX_COUNT: usize = ELEVATED_EXEC_PREFIXES.len();

/// A registered IPC endpoint.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub id: u32,
    pub owner_pid: usize,
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
