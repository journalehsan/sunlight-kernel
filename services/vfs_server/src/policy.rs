//! Registration-lived policy, root-stat and retention routing. Copied kernel
//! channels authenticate dispatch; ordinary IPC cannot supply authority.
//! Obtain `authenticated` independently of payloads and serialize policy changes
//! with caller/service lifecycle. Never recreate this owner within an epoch.
use crate::directory::{DirectoryNamespace, DirectoryPair};
use alloc::sync::Arc;
use sunlight_fs::{BlockDevice, FileStat, FsError, NullDevice};
use sunlight_ipc::vfs_rpc::{
    lifecycle::Caller,
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
    Retired,
    Filesystem(FsError),
    Retention(retention::Error<FsError>),
}

pub(crate) struct Binding<Device: BlockDevice> {
    scope: Scope,
    pair: Arc<DirectoryPair<Device>>,
}

impl<Device: BlockDevice> Clone for Binding<Device> {
    fn clone(&self) -> Self {
        Self {
            scope: self.scope,
            pair: self.pair.clone(),
        }
    }
}

/// Policy capacity and remote lease capacity are separate. Each policy owns one
/// pair of real handles; leases share ownership of those exact pinned objects,
/// never re-resolve saved paths. Withdrawal drops policy ownership only.
pub struct Policy<const P: usize, const L: usize, Device: BlockDevice = NullDevice> {
    registration: Registration,
    bindings: [Option<Binding<Device>>; P],
    last_generation: u64,
    leases: Table<Arc<DirectoryPair<Device>>, L>,
    retired: bool,
}

impl<const P: usize, const L: usize, Device: BlockDevice> Policy<P, L, Device> {
    pub fn new(registration: Registration) -> Result<Self, Error> {
        let leases = Table::new(registration).map_err(|_| Error::Identity)?;
        Ok(Self {
            registration,
            bindings: [const { None }; P],
            last_generation: 0,
            leases,
            retired: false,
        })
    }

    /// Trusted local policy installation, never callable from a decoded frame.
    /// Generations must increase across the entire registration, including after
    /// withdrawal. Failed pin/allocation attempts consume their generation too.
    /// Paths are resolved once, transactionally, before publishing the binding.
    pub fn install(
        &mut self,
        scope: Scope,
        namespace: &DirectoryNamespace<Device>,
        root: &str,
        directory: &str,
    ) -> Result<(), Error> {
        if self.retired {
            return Err(Error::Retired);
        }
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

    /// Trusted lifecycle transition only; ordinary request decoding cannot call
    /// this API. The owner must authenticate credentials and serialize lifecycle
    /// changes through revalidation. A false revalidation withdraws old admission.
    /// Old leases keep their exact pair, but cannot dispatch after replacement.
    pub(crate) fn change_directory(
        &mut self,
        old: Scope,
        next: Scope,
        path: &str,
        caller: sunlight_fs::Credential,
        revalidate: impl FnOnce() -> bool,
    ) -> Result<(), Error> {
        self.transition(old, next, path, caller, false, revalidate)
    }

    pub(crate) fn change_root(
        &mut self,
        old: Scope,
        next: Scope,
        path: &str,
        caller: sunlight_fs::Credential,
        revalidate: impl FnOnce() -> bool,
    ) -> Result<(), Error> {
        self.transition(old, next, path, caller, true, revalidate)
    }

    fn transition(
        &mut self,
        old: Scope,
        next: Scope,
        path: &str,
        caller: sunlight_fs::Credential,
        replace_root: bool,
        revalidate: impl FnOnce() -> bool,
    ) -> Result<(), Error> {
        if self.retired {
            return Err(Error::Retired);
        }
        wire::PendingRetain::new(self.registration, next, 1).map_err(Error::Protocol)?;
        if old.caller != next.caller || old.namespace_id != next.namespace_id {
            return Err(Error::Scope);
        }
        let slot = self
            .bindings
            .iter()
            .position(|b| b.as_ref().is_some_and(|b| b.scope == old))
            .ok_or(Error::Scope)?;
        if next.namespace_generation <= self.last_generation {
            return Err(Error::Generation);
        }
        // Failed attempts cannot reuse generations, including failed lookup.
        self.last_generation = next.namespace_generation;
        let previous = &self.bindings[slot].as_ref().unwrap().pair;
        let pair = if replace_root {
            previous.change_root(path, caller)
        } else {
            previous.change_directory(path, caller)
        }
        .map_err(Error::Filesystem)?;
        let pair = Arc::try_new(pair).map_err(|_| Error::Filesystem(FsError::NoMemory))?;
        if !revalidate() {
            self.withdraw(old)?;
            return Err(Error::Identity);
        }
        self.bindings[slot] = Some(Binding { scope: next, pair });
        Ok(())
    }

    /// Derive a separately withdrawable caller binding from exact retained
    /// objects. No pathname lookup, credential inheritance or numeric object
    /// claim is involved. The lifecycle owner authenticates parent and child.
    pub(crate) fn derive_caller(
        &mut self,
        parent: Scope,
        child: Scope,
        revalidate: impl FnOnce() -> bool,
    ) -> Result<(), Error> {
        if self.retired {
            return Err(Error::Retired);
        }
        let pair = self
            .bindings
            .iter()
            .flatten()
            .find(|b| b.scope == parent)
            .map(|b| Arc::clone(&b.pair))
            .ok_or(Error::Scope)?;
        self.derive_pair(parent, child, pair, revalidate)
    }

    /// The copied kernel handoff pins this exact lease while the source caller
    /// can retire. Ordinary retention requests never invoke this authority.
    pub(crate) fn derive_retained_caller(
        &mut self,
        source: Receipt,
        child: Scope,
        revalidate: impl FnOnce() -> bool,
    ) -> Result<(), Error> {
        let pair = Arc::clone(
            self.leases
                .get(self.registration, source)
                .map_err(|_| Error::Scope)?,
        );
        self.derive_pair(source.scope(), child, pair, revalidate)
    }

    fn derive_pair(
        &mut self,
        parent: Scope,
        child: Scope,
        pair: Arc<DirectoryPair<Device>>,
        revalidate: impl FnOnce() -> bool,
    ) -> Result<(), Error> {
        if self.retired {
            return Err(Error::Retired);
        }
        wire::PendingRetain::new(self.registration, child, 1).map_err(Error::Protocol)?;
        if parent.caller == child.caller || parent.namespace_id != child.namespace_id {
            return Err(Error::Scope);
        }
        if self
            .bindings
            .iter()
            .flatten()
            .any(|b| b.scope.caller == child.caller)
        {
            return Err(Error::Scope);
        }
        if child.namespace_generation <= self.last_generation {
            return Err(Error::Generation);
        }
        let slot = self
            .bindings
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        self.last_generation = child.namespace_generation;
        if !revalidate() {
            return Err(Error::Identity);
        }
        self.bindings[slot] = Some(Binding { scope: child, pair });
        Ok(())
    }

    pub(crate) fn save_directory(&self, scope: Scope) -> Result<Binding<Device>, Error> {
        self.bindings
            .iter()
            .flatten()
            .find(|b| b.scope == scope)
            .cloned()
            .ok_or(Error::Scope)
    }
    pub(crate) fn restore_directory(&mut self, scope: Scope, saved: Binding<Device>) {
        if let Some(slot) = self
            .bindings
            .iter_mut()
            .find(|b| b.as_ref().is_some_and(|b| b.scope == scope))
        {
            *slot = Some(saved);
        }
    }

    pub(crate) fn cwd(&self, scope: Scope) -> Result<alloc::string::String, Error> {
        self.bindings
            .iter()
            .flatten()
            .find(|b| b.scope == scope)
            .ok_or(Error::Scope)?
            .pair
            .cwd()
            .map_err(Error::Filesystem)
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

    /// Trusted lifecycle event, serialized with requests and provisioning. Drop
    /// admission pins for the exact caller, preserving all retained leases.
    pub fn retire_caller(&mut self, caller: Caller) {
        for binding in &mut self.bindings {
            if binding.as_ref().is_some_and(|b| b.scope.caller == caller) {
                drop(binding.take());
            }
        }
    }

    /// Permanently close provisioning and retain resolution for this epoch.
    /// Keep the lease table and replay watermarks, including when empty, so late
    /// retains cannot resurrect policy and lost release acks remain retryable.
    pub fn retire(&mut self) {
        self.retired = true;
        for binding in &mut self.bindings {
            drop(binding.take());
        }
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

    /// Consume a new retain without acquiring any lease. Use the retention
    /// table so an identity failure/replay can never falsely deny an old lease.
    pub(crate) fn refuse(
        &mut self,
        authenticated: Registration,
        bytes: &[u8],
    ) -> Result<wire::Frame, Error> {
        let wire::Frame::Retain {
            registration,
            scope,
            request_id,
        } = wire::Frame::decode(bytes).map_err(Error::Protocol)?
        else {
            return Err(Error::Protocol(ProtocolError::Operation));
        };
        let request =
            wire::PendingRetain::new(registration, scope, request_id).map_err(Error::Protocol)?;
        self.leases
            .retain_reply(authenticated, request, || Err(FsError::PermissionDenied))
            .map_err(Error::Retention)
    }

    /// Retained-root/cwd stat dispatch. The copied kernel channel authenticates dispatch;
    /// retained lease lookup binds numeric scope/object fields to real pins.
    pub(crate) fn stat(
        &self,
        authenticated: Registration,
        dispatch: &sunlight_ipc::vfs_rpc::context::Dispatch<'_>,
    ) -> Result<sunlight_ipc::vfs_rpc::response::Metadata, Error> {
        use sunlight_ipc::vfs_rpc::{
            codec::Request,
            context::LOOKUP_METADATA,
            response::{Kind, Metadata, Provenance},
        };
        let context = dispatch.context;
        if self.retired
            || authenticated != self.registration
            || context.service != authenticated.service()
        {
            return Err(Error::Identity);
        }
        let Request::Stat { path } = dispatch.request else {
            return Err(Error::Scope);
        };
        if context.grants != LOOKUP_METADATA
            || !context.groups.is_empty()
            || context.namespace.root != context.namespace.directory
        {
            return Err(Error::Scope);
        }
        let scope = Scope {
            caller: context.caller,
            namespace_id: context.namespace.id,
            namespace_generation: context.namespace.generation,
        };
        // Withdrawal closes admission, even when an outstanding lease still
        // owns pins for late completion/release. Both records must be live.
        if !self.bindings.iter().flatten().any(|b| b.scope == scope) {
            return Err(Error::Scope);
        }
        let pair = self
            .leases
            .get_scope(authenticated, scope, context.namespace.root)
            .map_err(|_| Error::Scope)?;
        let object = pair
            .stat(
                path,
                sunlight_fs::Credential {
                    uid: context.uid,
                    gid: context.gid,
                },
            )
            .map_err(Error::Filesystem)?;
        let stat = object.stat;
        Ok(Metadata {
            provenance: Provenance {
                mount_id: object.mount_id,
                mount_generation: object.mount_generation,
                object_id: object.object_id,
                object_generation: object.object_generation,
            },
            kind: match stat.file_type {
                sunlight_fs::FileType::Directory => Kind::Directory,
                sunlight_fs::FileType::File => Kind::File,
            },
            mode: stat.mode & 0o7777,
            uid: stat.uid,
            gid: stat.gid,
            links: stat.nlinks,
            size: stat.size as u64,
        })
    }

    /// Trusted adapter only: receipt is independently authenticated, and the
    /// caller's credentials are captured under lifecycle serialization.
    pub(crate) fn open_file(
        &self,
        authenticated: Registration,
        receipt: Receipt,
        path: &str,
        caller: sunlight_fs::Credential,
        options: sunlight_fs::vfs::OpenOptions,
        storage_admin: bool,
    ) -> Result<crate::open::File<Device>, Error> {
        if self.retired || authenticated != self.registration {
            return Err(Error::Identity);
        }
        let pair = self
            .leases
            .get(authenticated, receipt)
            .map_err(|_| Error::Scope)?;
        if !self
            .bindings
            .iter()
            .flatten()
            .any(|b| b.scope == receipt.scope())
        {
            return Err(Error::Scope);
        }
        pair.open_file(path, caller, options, storage_admin)
            .map_err(Error::Filesystem)
    }

    #[cfg(feature = "vfs-c1e2-test")]
    pub(crate) fn mutate_namespace(
        &self,
        authenticated: Registration,
        receipt: Receipt,
        request: sunlight_ipc::vfs_rpc::file_transport::Request<'_>,
    ) -> Result<(), Error> {
        if self.retired || authenticated != self.registration {
            return Err(Error::Identity);
        }
        if !self.bindings.iter().flatten().any(|b| b.scope == receipt.scope()) {
            return Err(Error::Scope);
        }
        self.leases.get(authenticated, receipt).map_err(|_| Error::Scope)?
            .mutate(request).map_err(Error::Filesystem)
    }

    #[cfg(feature = "vfs-c1e2-test")]
    pub(crate) fn mount_observation(
        &self,
        authenticated: Registration,
        receipt: Receipt,
        caller: sunlight_fs::Credential,
        path: Option<&str>,
    ) -> Result<(u64, u64, alloc::vec::Vec<u8>), crate::directory::attachment::Error> {
        use crate::directory::attachment::Error;
        if self.retired || authenticated != self.registration
            || !self.bindings.iter().flatten().any(|binding| binding.scope == receipt.scope())
        {
            return Err(Error::Stale);
        }
        self.leases.get(authenticated, receipt).map_err(|_| Error::Stale)?
            .mount_observation(caller, authenticated.epoch, path)
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
