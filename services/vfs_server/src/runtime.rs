//! Service-process owner of trusted registration policies and copied control.
//! Every identity argument must come from independently authenticated lifecycle
//! or transport records. Decoded ordinary IPC cannot grant or retire policy.
use crate::{
    directory::DirectoryNamespace,
    policy::{Error, Policy},
};
use alloc::boxed::Box;
use sunlight_fs::{FileStat, FsError};
use sunlight_ipc::vfs_rpc::{
    lifecycle::Caller,
    registration::Registration,
    retention::{wire, Receipt, Scope},
};

struct Entry<const P: usize, const L: usize> {
    registration: Registration,
    policy: Policy<P, L>,
}

/// Keep this owner for the entire service process lifetime. Retired policies
/// consume capacity even when empty: there is no authenticated drain barrier yet.
/// This preserves replay protection and duplicate-release replies indefinitely.
/// Heap allocation is lazy and fallible; an unprovisioned service owns no policy.
pub struct Runtime<const P: usize, const L: usize, const R: usize> {
    entries: [Option<Box<Entry<P, L>>>; R],
    active: Option<Registration>,
    last_epoch: u64,
}

impl<const P: usize, const L: usize, const R: usize> Runtime<P, L, R> {
    pub const fn new() -> Self {
        Self {
            entries: [const { None }; R],
            active: None,
            last_epoch: 0,
        }
    }

    /// Trusted provisioning only. Idempotence preserves policy and allocators.
    /// Explicit retirement must precede replacement; admission/allocation must
    /// succeed before publishing a new registration or consuming its epoch.
    pub fn bind(&mut self, registration: Registration) -> Result<(), Error> {
        if self.active == Some(registration) {
            return Ok(());
        }
        if self.active.is_some() || registration.epoch <= self.last_epoch {
            return Err(Error::Identity);
        }
        let slot = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let policy = Policy::new(registration)?;
        let entry = Box::try_new(Entry {
            registration,
            policy,
        })
        .map_err(|_| Error::Filesystem(FsError::NoMemory))?;
        self.entries[slot] = Some(entry);
        self.active = Some(registration);
        self.last_epoch = registration.epoch;
        Ok(())
    }

    /// Authenticated newer active snapshot proves withdrawal of the prior epoch.
    /// Failure to allocate the replacement leaves the old policy retired and
    /// retryable; it cannot grant authority or discard retained leases.
    pub fn observe_active(&mut self, registration: Registration) -> Result<(), Error> {
        if self.active == Some(registration) {
            return Ok(());
        }
        if registration.epoch <= self.last_epoch {
            return Err(Error::Identity);
        }
        // Validate before changing any existing policy.
        sunlight_ipc::vfs_rpc::retention::Table::<(), 0>::new(registration)
            .map_err(|_| Error::Identity)?;
        if let Some(old) = self.active {
            self.retire_registration(old)?;
        }
        self.bind(registration)
    }

    /// Stable bounded inventory, including retired policies. Never reclaim a
    /// policy from missing kernel status: that is not a remote drain receipt.
    pub fn registrations(&self) -> [Option<Registration>; R] {
        core::array::from_fn(|i| self.entries[i].as_ref().map(|e| e.registration))
    }

    pub fn active(&self) -> Option<Registration> {
        self.active
    }

    pub fn install(
        &mut self,
        registration: Registration,
        scope: Scope,
        namespace: &DirectoryNamespace,
        root: &str,
        directory: &str,
    ) -> Result<(), Error> {
        if self.active != Some(registration) {
            return Err(Error::Identity);
        }
        self.policy_mut(registration)?
            .install(scope, namespace, root, directory)
    }

    /// Service-side cwd transition for an independently authenticated lifecycle
    /// owner. Credentials and scopes must be captured under lifecycle serialization;
    /// revalidate must hold that serialization through publication. No copied
    /// syscall routing is installed by this API. Failed lookup preserves old
    /// admission; failed revalidation withdraws it and drops provisional pins.
    pub fn change_directory(
        &mut self,
        authenticated: Registration,
        old: Scope,
        next: Scope,
        path: &str,
        caller: sunlight_fs::Credential,
        revalidate: impl FnOnce() -> bool,
    ) -> Result<(), Error> {
        if self.active != Some(authenticated) {
            return Err(Error::Identity);
        }
        self.policy_mut(authenticated)?
            .change_directory(old, next, path, caller, revalidate)
    }

    pub fn withdraw(&mut self, registration: Registration, scope: Scope) -> Result<(), Error> {
        self.policy_mut(registration)?.withdraw(scope)
    }

    /// Trusted caller lifecycle event spans all retained registration epochs.
    pub fn retire_caller(&mut self, caller: Caller) {
        for entry in self.entries.iter_mut().flatten() {
            entry.policy.retire_caller(caller);
        }
    }

    pub fn retire_registration(&mut self, registration: Registration) -> Result<(), Error> {
        self.policy_mut(registration)?.retire();
        if self.active == Some(registration) {
            self.active = None;
        }
        Ok(())
    }

    /// Structural correlation only; authentication is an integration prerequisite.
    /// Retired epochs remain routable for late refusals and release retries.
    pub fn request(
        &mut self,
        authenticated: Registration,
        bytes: &[u8],
    ) -> Result<wire::Frame, Error> {
        self.policy_mut(authenticated)?
            .request(authenticated, bytes)
    }

    pub fn metadata(
        &self,
        authenticated: Registration,
        receipt: Receipt,
    ) -> Result<[(u64, FileStat); 2], Error> {
        self.entries
            .iter()
            .flatten()
            .find(|e| e.registration == authenticated)
            .ok_or(Error::Identity)?
            .policy
            .metadata(authenticated, receipt)
    }

    /// Only successfully copied stat input may reach this path. Ordinary IPC
    /// dispatch never calls it, regardless of badges, labels or encoded context.
    pub fn stat(
        &self,
        authenticated: Registration,
        bytes: &[u8],
    ) -> Result<[u8; sunlight_ipc::vfs_rpc::stat_transport::REPLY_BYTES], Error> {
        use sunlight_ipc::vfs_rpc::{
            codec::Request,
            context,
            response::{Reply, ReplyContract, METADATA_BYTES},
            stat_transport,
        };
        let mut groups = [0; context::MAX_GROUPS];
        let dispatch = context::decode(bytes, &mut groups).map_err(Error::Protocol)?;
        if self.active != Some(authenticated) {
            return Err(Error::Identity);
        }
        let policy = &self
            .entries
            .iter()
            .flatten()
            .find(|e| e.registration == authenticated)
            .ok_or(Error::Identity)?
            .policy;
        let metadata = policy.stat(authenticated, &dispatch);
        let mut output = [0; METADATA_BYTES];
        let (errno, length) = match metadata {
            Ok(metadata) => (
                0,
                ReplyContract::for_request(Request::Stat { path: "/" })
                    .map_err(Error::Protocol)?
                    .encode(Reply::Metadata(metadata), &mut output)
                    .map_err(Error::Protocol)?,
            ),
            Err(Error::Filesystem(error)) => (
                match error {
                    FsError::NotFound => 2,
                    FsError::NotDir => 20,
                    FsError::PermissionDenied | FsError::OperationNotPermitted => 13,
                    FsError::InvalidPath => 22,
                    FsError::BadHandle => 9,
                    FsError::Unsupported => 95,
                    FsError::NoMemory => 12,
                    _ => 5,
                },
                0,
            ),
            Err(_) => (5, 0),
        };
        stat_transport::encode_reply(dispatch.request_id, errno, &output[..length])
            .map_err(Error::Protocol)
    }

    fn policy_mut(&mut self, registration: Registration) -> Result<&mut Policy<P, L>, Error> {
        self.entries
            .iter_mut()
            .flatten()
            .find(|e| e.registration == registration)
            .map(|e| &mut e.policy)
            .ok_or(Error::Identity)
    }
}

#[cfg(test)]
mod tests;

pub mod control;

/// Policy supplied by a trusted lifecycle owner, never decoded from a retain frame.
pub struct ProvisionedScope<'a> {
    pub registration: Registration,
    pub scope: Scope,
    pub namespace: &'a DirectoryNamespace,
    pub root: &'a str,
    pub directory: &'a str,
}

/// The provider must serialize lifecycle changes across both callbacks and return
/// false if caller, registration, namespace identity, or path policy changed.
/// A production implementation requires an independently authenticated source.
pub trait ScopeAuthority {
    fn resolve(&mut self) -> Result<ProvisionedScope<'_>, Error>;
    fn revalidate(&mut self, registration: Registration, scope: Scope) -> bool;
}

impl<const P: usize, const L: usize, const R: usize> Runtime<P, L, R> {
    /// Install pins before retain handling, then fail closed on lifecycle races.
    /// Exclusive runtime access prevents requests from observing provisional pins.
    pub fn provision(&mut self, authority: &mut impl ScopeAuthority) -> Result<Scope, Error> {
        let policy = authority.resolve()?;
        let registration = policy.registration;
        let scope = policy.scope;
        self.install(
            registration,
            scope,
            policy.namespace,
            policy.root,
            policy.directory,
        )?;
        if !authority.revalidate(registration, scope) {
            self.withdraw(registration, scope)?;
            return Err(Error::Identity);
        }
        Ok(scope)
    }
}
