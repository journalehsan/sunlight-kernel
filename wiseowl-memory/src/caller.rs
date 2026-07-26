//! Caller identity for authorization (shared host + native).

extern crate alloc;

use alloc::vec::Vec;

use crate::caps::CapabilitySet;
use crate::ids::{ClientId, SessionId};

/// Caller identity for authorization.
#[derive(Debug, Clone)]
pub struct CallerIdentity {
    pub client_id: Option<ClientId>,
    pub caps: CapabilitySet,
    pub owned_sessions: Vec<SessionId>,
}

impl CallerIdentity {
    pub fn admin() -> Self {
        Self {
            client_id: None,
            caps: CapabilitySet::admin(),
            owned_sessions: Vec::new(),
        }
    }

    pub fn client(client_id: ClientId, caps: CapabilitySet) -> Self {
        Self {
            client_id: Some(client_id),
            caps,
            owned_sessions: Vec::new(),
        }
    }

    /// Diagnostic CLI defaults: stats + list + maintenance, no payload by default.
    pub fn diagnostic() -> Self {
        Self {
            client_id: None,
            caps: CapabilitySet::diagnostic(),
            owned_sessions: Vec::new(),
        }
    }
}
