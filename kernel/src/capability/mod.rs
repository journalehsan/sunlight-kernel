use crate::serial_println;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::structures::paging::{PhysFrame, Size4KiB};
use x86_64::PhysAddr;

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
/// The low 62 bits are an unpredictable payload (counter XOR TSC seed); the
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
    /// Mask covering the unpredictable payload bits.
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
}

/// Global token counter and seed.
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
static TOKEN_SEED: AtomicU64 = AtomicU64::new(0);

/// Global capability broker instance.
pub static CAP_BROKER: spin::Mutex<CapabilityBroker> = spin::Mutex::new(CapabilityBroker::new());

/// Global spawn capability token. A hardcoded special token that the kernel
/// recognizes in `ipc_call` to handle spawn requests directly.
pub const SPAWN_TOKEN: CapabilityToken = CapabilityToken(0xCAFEBABE_DEADBEEF);

/// PID expected to call `sys_grant_capability`.
pub const CAPABILITY_BROKER_PID: u32 = 6;

/// Initialize the token seed from TSC.
pub fn init_token_seed() {
    let tsc = unsafe { core::arch::x86_64::_rdtsc() };
    TOKEN_SEED.store(tsc, Ordering::SeqCst);
}

/// Generate a new unpredictable, type-tagged capability token.
fn generate_token(tag: u64) -> CapabilityToken {
    let counter = NEXT_TOKEN.fetch_add(1, Ordering::SeqCst);
    let seed = TOKEN_SEED.load(Ordering::SeqCst);
    CapabilityToken::tagged(counter ^ seed, tag)
}

/// The capability broker manages endpoints and capability tokens.
pub struct CapabilityBroker {
    endpoints: alloc::vec::Vec<Endpoint>,
    capabilities: alloc::vec::Vec<(CapabilityToken, u32, CapabilityRights)>,
    /// Shared memory regions (multi-page capable): (token, object, owner_pid, revoked)
    shared_regions: alloc::vec::Vec<(CapabilityToken, Arc<ShmObject>, usize, bool)>,
    vfs_caps: alloc::vec::Vec<(CapabilityToken, VfsCapability, usize)>,
}

impl CapabilityBroker {
    pub const fn new() -> Self {
        Self {
            endpoints: alloc::vec::Vec::new(),
            capabilities: alloc::vec::Vec::new(),
            shared_regions: alloc::vec::Vec::new(),
            vfs_caps: alloc::vec::Vec::new(),
        }
    }

    /// Create a new endpoint, return its id and a send+recv capability to the owner.
    pub fn create_endpoint(&mut self, owner_pid: usize) -> (u32, CapabilityToken) {
        let id = self.endpoints.len() as u32;
        self.endpoints.push(Endpoint { id, owner_pid });
        let token = generate_token(CapabilityToken::TAG_IPC);
        self.capabilities
            .push((token, id, CapabilityRights::SEND_RECV));
        serial_println!("[CAP] Created endpoint {} for pid={}", id, owner_pid);
        (id, token)
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
    pub fn mint(&mut self, endpoint_id: u32, rights: CapabilityRights) -> Option<CapabilityToken> {
        if !rights.is_ipc() {
            return None;
        }
        if !self.endpoints.iter().any(|e| e.id == endpoint_id) {
            return None;
        }
        let token = generate_token(CapabilityToken::TAG_IPC);
        self.capabilities.push((token, endpoint_id, rights));
        Some(token)
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

    /// Revoke a capability token.
    pub fn revoke(&mut self, token: CapabilityToken) {
        if let Some(idx) = self.capabilities.iter().position(|(t, _, _)| *t == token) {
            self.capabilities.swap_remove(idx);
            serial_println!("[CAP] Revoked token {:#x}", token.as_u64());
        }
    }

    /// Get endpoint owner.
    pub fn endpoint_owner(&self, endpoint_id: u32) -> Option<usize> {
        self.endpoints
            .iter()
            .find(|e| e.id == endpoint_id)
            .map(|e| e.owner_pid)
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
    }

    /// Mint a capability token granting access to map a shared physical frame (single-page compat).
    pub fn mint_shared_page(&mut self, phys: PhysAddr, owner_pid: usize) -> CapabilityToken {
        let frame = unsafe { PhysFrame::<Size4KiB>::from_start_address_unchecked(phys) };
        self.mint_shared_region(alloc::vec![frame], 4096, owner_pid)
    }

    /// Mint a capability token for a multi-page shared memory region.
    /// The token represents the whole object; mapping it maps all backing frames contiguously.
    pub fn mint_shared_region(
        &mut self,
        frames: alloc::vec::Vec<PhysFrame<Size4KiB>>,
        size: usize,
        owner_pid: usize,
    ) -> CapabilityToken {
        let obj = Arc::new(ShmObject { frames, size });
        let token = generate_token(CapabilityToken::TAG_VFS);
        self.shared_regions.push((token, obj, owner_pid, false));
        serial_println!(
            "[CAP] Minted shm-region token {:#x} size={}KiB owner={}",
            token.as_u64(),
            size / 1024,
            owner_pid
        );
        token
    }

    /// Resolve a shared-page capability token to its physical frame (first page for compat / 1-page regions).
    pub fn resolve_shared_page(&self, token: CapabilityToken) -> Option<PhysAddr> {
        self.resolve_shared_region(token)
            .and_then(|obj| obj.frames.first().map(|f| f.start_address()))
    }

    /// Resolve a (possibly multi-page) shared region capability to its object.
    pub fn resolve_shared_region(&self, token: CapabilityToken) -> Option<Arc<ShmObject>> {
        self.shared_regions.iter().find_map(|(t, obj, _, revoked)| {
            if *t == token && !*revoked {
                Some(Arc::clone(obj))
            } else {
                None
            }
        })
    }

    /// Validate a shared-page token, distinguishing "never existed" (forged/guessed)
    /// from "existed but revoked" (owner exited). Used by security self-tests and
    /// can be used by callers that need to report the precise rejection reason.
    pub fn validate_shared_page(&self, token: CapabilityToken) -> Result<PhysAddr, CapError> {
        let entry = self
            .shared_regions
            .iter()
            .find(|(t, _, _, _)| *t == token)
            .ok_or(CapError::NotFound)?;
        if entry.3 {
            return Err(CapError::Revoked);
        }
        // Return first phys for compat with single-page tests
        entry
            .1
            .frames
            .first()
            .map(|f| f.start_address())
            .ok_or(CapError::NotFound)
    }

    /// Revoke a shared region grant token (called on owner free or cleanup).
    pub fn revoke_shared(&mut self, token: CapabilityToken) {
        if let Some(idx) = self
            .shared_regions
            .iter()
            .position(|(t, _, _, _)| *t == token)
        {
            self.shared_regions.swap_remove(idx);
            serial_println!("[CAP] Revoked shm-region token {:#x}", token.as_u64());
        }
    }

    /// Mark all shared region grants owned by `pid` as revoked, without removing
    /// them from the table.
    pub fn revoke_all_for(&mut self, pid: usize) {
        for entry in self.shared_regions.iter_mut() {
            if entry.2 == pid {
                entry.3 = true;
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
        let token = generate_token(CapabilityToken::TAG_VFS);
        if self.vfs_caps.len() >= CAP_VFS_LIMIT {
            return Err(CapError::CapabilityStoreFull);
        }

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
        for prefix in ELEVATED_EXEC_PREFIXES {
            let mut allowed_prefix = String::new();
            if allowed_prefix.push_str(prefix).is_err() {
                return Err(CapError::CapabilityStoreFull);
            }
            let cap = VfsCapability {
                allowed_prefix,
                flags: AccessFlags::READ_EXECUTE,
            };
            let token = self.grant_vfs_capability(owner_pid, cap)?;
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

/// Mock kernel entry to enforce trusted-broker minting.
pub fn sys_grant_capability(
    caller_pid: u32,
    cap: VfsCapability,
) -> Result<CapabilityToken, CapError> {
    if caller_pid != CAPABILITY_BROKER_PID {
        serial_println!(
            "[SEC] Capability mint denied: caller_pid={} is not broker {}",
            caller_pid,
            CAPABILITY_BROKER_PID
        );
        return Err(CapError::InvalidCaller);
    }

    let mut broker = CAP_BROKER.lock();
    broker.grant_vfs_capability(caller_pid as usize, cap)
}

/// A registered IPC endpoint.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub id: u32,
    pub owner_pid: usize,
}
