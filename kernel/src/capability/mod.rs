use crate::serial_println;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::PhysAddr;

/// A capability token: opaque to user-space, meaningful to the kernel.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CapabilityToken(pub u64);

impl CapabilityToken {
    pub const INVALID: Self = Self(0);

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// What rights a capability grants.
#[derive(Debug, Clone, Copy)]
pub struct CapabilityRights {
    pub can_send: bool,
    pub can_receive: bool,
    pub can_grant: bool,
}

impl CapabilityRights {
    pub const SEND_RECV: Self = Self {
        can_send: true,
        can_receive: true,
        can_grant: false,
    };

    pub const SEND_ONLY: Self = Self {
        can_send: true,
        can_receive: false,
        can_grant: false,
    };

    pub const SEND: Self = Self::SEND_ONLY;

    pub const RECV_ONLY: Self = Self {
        can_send: false,
        can_receive: true,
        can_grant: false,
    };
}

/// A registered IPC endpoint.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub id: u32,
    pub owner_pid: usize,
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
}

/// Global token counter and seed.
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
static TOKEN_SEED: AtomicU64 = AtomicU64::new(0);

/// Global capability broker instance.
pub static CAP_BROKER: spin::Mutex<CapabilityBroker> = spin::Mutex::new(CapabilityBroker::new());

/// Global spawn capability token. A hardcoded special token that the kernel
/// recognizes in `ipc_call` to handle spawn requests directly.
pub const SPAWN_TOKEN: CapabilityToken = CapabilityToken(0xCAFEBABE_DEADBEEF);

/// Initialize the token seed from TSC.
pub fn init_token_seed() {
    let tsc = unsafe { core::arch::x86_64::_rdtsc() };
    TOKEN_SEED.store(tsc, Ordering::SeqCst);
}

/// Generate a new unpredictable capability token.
fn generate_token() -> CapabilityToken {
    let counter = NEXT_TOKEN.fetch_add(1, Ordering::SeqCst);
    let seed = TOKEN_SEED.load(Ordering::SeqCst);
    CapabilityToken(counter ^ seed)
}

/// The capability broker manages endpoints and capability tokens.
pub struct CapabilityBroker {
    endpoints: alloc::vec::Vec<Endpoint>,
    capabilities: alloc::vec::Vec<(CapabilityToken, u32, CapabilityRights)>,
    /// Shared memory grants: (token, phys_frame, owner_pid, revoked)
    shared_grants: alloc::vec::Vec<(CapabilityToken, PhysAddr, usize, bool)>,
}

impl CapabilityBroker {
    pub const fn new() -> Self {
        Self {
            endpoints: alloc::vec::Vec::new(),
            capabilities: alloc::vec::Vec::new(),
            shared_grants: alloc::vec::Vec::new(),
        }
    }

    /// Create a new endpoint, return its id and a send+recv capability to the owner.
    pub fn create_endpoint(&mut self, owner_pid: usize) -> (u32, CapabilityToken) {
        let id = self.endpoints.len() as u32;
        self.endpoints.push(Endpoint { id, owner_pid });
        let token = generate_token();
        self.capabilities
            .push((token, id, CapabilityRights::SEND_RECV));
        serial_println!("[CAP] Created endpoint {} for pid={}", id, owner_pid);
        (id, token)
    }

    /// Validate that `token` grants at least the requested rights for its endpoint.
    pub fn check(&self, token: CapabilityToken, rights: CapabilityRights) -> Result<u32, CapError> {
        for (t, ep_id, r) in &self.capabilities {
            if *t == token {
                if rights.can_send && !r.can_send {
                    return Err(CapError::InsufficientRights);
                }
                if rights.can_receive && !r.can_receive {
                    return Err(CapError::InsufficientRights);
                }
                if rights.can_grant && !r.can_grant {
                    return Err(CapError::InsufficientRights);
                }
                return Ok(*ep_id);
            }
        }
        Err(CapError::InvalidToken)
    }

    /// Mint a new capability token for an existing endpoint.
    pub fn mint(&mut self, endpoint_id: u32, rights: CapabilityRights) -> Option<CapabilityToken> {
        if !self.endpoints.iter().any(|e| e.id == endpoint_id) {
            return None;
        }
        let token = generate_token();
        self.capabilities.push((token, endpoint_id, rights));
        Some(token)
    }

    /// Return an existing token for an endpoint if one already has the rights.
    pub fn token_for_endpoint(
        &self,
        endpoint_id: u32,
        rights: CapabilityRights,
    ) -> Option<CapabilityToken> {
        self.capabilities
            .iter()
            .find_map(|(token, id, token_rights)| {
                if *id != endpoint_id {
                    return None;
                }
                if rights.can_send && !token_rights.can_send {
                    return None;
                }
                if rights.can_receive && !token_rights.can_receive {
                    return None;
                }
                if rights.can_grant && !token_rights.can_grant {
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

    /// Mint a capability token granting access to map a shared physical frame.
    pub fn mint_shared_page(&mut self, phys: PhysAddr, owner_pid: usize) -> CapabilityToken {
        let token = generate_token();
        self.shared_grants.push((token, phys, owner_pid, false));
        serial_println!(
            "[CAP] Minted shared-page token {:#x} phys={:#x} owner={}",
            token.as_u64(),
            phys.as_u64(),
            owner_pid
        );
        token
    }

    /// Resolve a shared-page capability token to its physical frame (if valid and not revoked).
    pub fn resolve_shared_page(&self, token: CapabilityToken) -> Option<PhysAddr> {
        self.shared_grants
            .iter()
            .find_map(|(t, phys, _, revoked)| if *t == token && !*revoked { Some(*phys) } else { None })
    }

    /// Validate a shared-page token, distinguishing "never existed" (forged/guessed)
    /// from "existed but revoked" (owner exited). Used by security self-tests and
    /// can be used by callers that need to report the precise rejection reason.
    pub fn validate_shared_page(&self, token: CapabilityToken) -> Result<PhysAddr, CapError> {
        let entry = self
            .shared_grants
            .iter()
            .find(|(t, _, _, _)| *t == token)
            .ok_or(CapError::NotFound)?;
        if entry.3 {
            return Err(CapError::Revoked);
        }
        Ok(entry.1)
    }

    /// Revoke a shared-page grant token (called on owner free or cleanup).
    pub fn revoke_shared(&mut self, token: CapabilityToken) {
        if let Some(idx) = self.shared_grants.iter().position(|(t, _, _, _)| *t == token) {
            self.shared_grants.swap_remove(idx);
            serial_println!("[CAP] Revoked shared-page token {:#x}", token.as_u64());
        }
    }

    /// Mark all shared-page grants owned by `pid` as revoked, without removing
    /// them from the table (so a subsequent lookup correctly reports
    /// `CapError::Revoked` rather than `CapError::NotFound`).
    /// Called when a process exits, before its frames are freed.
    pub fn revoke_all_for(&mut self, pid: usize) {
        for entry in self.shared_grants.iter_mut() {
            if entry.2 == pid {
                entry.3 = true;
            }
        }
    }
}
