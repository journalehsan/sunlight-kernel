//! Registration-lived trusted policy and retention routing for isolated tmpfs.
//! No IPC route is installed. The integration must authenticate kernel origin,
//! obtain `authenticated` independently of payloads, and serialize policy changes
//! with caller/service lifecycle. Never recreate this owner within an epoch.
use crate::directory::{DirectoryNamespace, DirectoryPair};
use alloc::sync::Arc;
use sunlight_fs::{FileStat, FsError};
use sunlight_ipc::vfs_rpc::{
    registration::Registration,
    retention::{self, wire, Receipt, Scope, Table},
    ProtocolError,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Protocol(ProtocolError),
    Identity,
    Scope,
    Capacity,
    Generation,
    Filesystem(FsError),
    Retention(retention::Error<FsError>),
}

struct Binding {
    scope: Scope,
    pair: Arc<DirectoryPair>,
}

/// Policy capacity and remote lease capacity are separate. Each policy owns one
/// pair of real handles; leases share ownership of those exact pinned objects,
/// never re-resolve saved paths. Withdrawal drops policy ownership only.
pub struct Policy<const P: usize, const L: usize> {
    registration: Registration,
    bindings: [Option<Binding>; P],
    last_generation: u64,
    leases: Table<Arc<DirectoryPair>, L>,
}

impl<const P: usize, const L: usize> Policy<P, L> {
    pub fn new(registration: Registration) -> Result<Self, Error> {
        let leases = Table::new(registration).map_err(|_| Error::Identity)?;
        Ok(Self {
            registration,
            bindings: [const { None }; P],
            last_generation: 0,
            leases,
        })
    }

    /// Trusted local policy installation, never callable from a decoded frame.
    /// Generations must increase across the entire registration, including after
    /// withdrawal. Failed pin/allocation attempts consume their generation too.
    /// Paths are resolved once, transactionally, before publishing the binding.
    pub fn install(
        &mut self,
        scope: Scope,
        namespace: &DirectoryNamespace,
        root: &str,
        directory: &str,
    ) -> Result<(), Error> {
        wire::PendingRetain::new(self.registration, scope, 1).map_err(Error::Protocol)?;
        if scope.namespace_generation <= self.last_generation {
            return Err(Error::Generation);
        }
        if self
            .bindings
            .iter()
            .flatten()
            .any(|b| b.scope.caller == scope.caller)
        {
            return Err(Error::Scope);
        }
        let slot = self
            .bindings
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        self.last_generation = scope.namespace_generation;
        let pair = namespace.pin(root, directory).map_err(Error::Filesystem)?;
        let pair = Arc::try_new(pair).map_err(|_| Error::Filesystem(FsError::NoMemory))?;
        self.bindings[slot] = Some(Binding { scope, pair });
        Ok(())
    }

    /// Complete scope match prevents stale exit/exec teardown from withdrawing
    /// a replacement. Existing leases remain releasable after policy withdrawal.
    pub fn withdraw(&mut self, scope: Scope) -> Result<(), Error> {
        let slot = self
            .bindings
            .iter()
            .position(|b| b.as_ref().is_some_and(|b| b.scope == scope))
            .ok_or(Error::Scope)?;
        drop(self.bindings[slot].take());
        Ok(())
    }

    /// Structural request routing only. `authenticated` MUST come from a future
    /// kernel-origin transport record. Passing decoded registration is unsafe
    /// integration even though this Rust API has no memory-unsafe operations.
    /// Identity/replay errors never produce a refusal; they may name a live lease.
    pub fn request(
        &mut self,
        authenticated: Registration,
        bytes: &[u8],
    ) -> Result<wire::Frame, Error> {
        if authenticated != self.registration {
            return Err(Error::Identity);
        }
        match wire::Frame::decode(bytes).map_err(Error::Protocol)? {
            wire::Frame::Retain {
                registration,
                scope,
                request_id,
            } => {
                let request = wire::PendingRetain::new(registration, scope, request_id)
                    .map_err(Error::Protocol)?;
                let bindings = &self.bindings;
                self.leases
                    .retain_reply(authenticated, request, || {
                        bindings
                            .iter()
                            .flatten()
                            .find(|b| b.scope == scope)
                            .map(|b| Arc::clone(&b.pair))
                            .ok_or(FsError::PermissionDenied)
                    })
                    .map_err(Error::Retention)
            }
            wire::Frame::Release(receipt) => self
                .leases
                .release(authenticated, receipt)
                .map(wire::Frame::Released)
                .map_err(|_| Error::Identity),
            _ => Err(Error::Protocol(ProtocolError::Operation)),
        }
    }

    /// Diagnostics of retained objects, with no pathname lookup or new authority.
    pub fn metadata(
        &self,
        authenticated: Registration,
        receipt: Receipt,
    ) -> Result<[(u64, FileStat); 2], Error> {
        self.leases
            .get(authenticated, receipt)
            .map_err(|_| Error::Scope)?
            .metadata()
            .map_err(Error::Filesystem)
    }
}

#[cfg(test)]
mod tests;
