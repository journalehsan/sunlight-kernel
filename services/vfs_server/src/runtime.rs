//! Service-process owner of trusted registration policies and copied control.
//! Every identity argument must come from independently authenticated lifecycle
//! or transport records. Decoded ordinary IPC cannot grant or retire policy.
use crate::{
    directory::DirectoryNamespace,
    policy::{Error, Policy},
};
use alloc::boxed::Box;
use sunlight_fs::{BlockDevice, FileStat, FsError, NullDevice};
use sunlight_ipc::vfs_rpc::{
    lifecycle::Caller,
    registration::Registration,
    retention::{wire, Receipt, Scope},
};

struct Entry<const P: usize, const L: usize, Device: BlockDevice> {
    registration: Registration,
    policy: Policy<P, L, Device>,
    files: crate::open::Table<L, Device>,
}

/// Keep this owner for the entire service process lifetime. Retired policies
/// consume capacity even when empty: there is no authenticated drain barrier yet.
/// This preserves replay protection and duplicate-release replies indefinitely.
/// Heap allocation is lazy and fallible; an unprovisioned service owns no policy.
pub struct Runtime<const P: usize, const L: usize, const R: usize, Device: BlockDevice = NullDevice> {
    entries: [Option<Box<Entry<P, L, Device>>>; R],
    active: Option<Registration>,
    last_epoch: u64,
    #[cfg(feature = "vfs-d2b-test")]
    fixture_endpoint: Option<sunlight_ipc::EndpointId>,
}

impl<const P: usize, const L: usize, const R: usize, Device: BlockDevice> Runtime<P, L, R, Device> {
    pub const fn new() -> Self {
        Self {
            entries: [const { None }; R],
            active: None,
            last_epoch: 0,
            #[cfg(feature = "vfs-d2b-test")]
            fixture_endpoint: None,
        }
    }

    #[cfg(feature = "vfs-d2b-test")]
    pub fn set_fixture_endpoint(&mut self, endpoint: sunlight_ipc::EndpointId) {
        self.fixture_endpoint = Some(endpoint);
    }

    #[cfg(feature = "vfs-d2b-test")]
    pub fn fixture_withdrew(&self) -> bool {
        self.fixture_endpoint.is_none()
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
            files: crate::open::Table::new(),
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
        namespace: &DirectoryNamespace<Device>,
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
    /// direct adapters hold that serialization through publication. The copied
    /// adapter keeps publication provisional until its kernel finish acknowledgement.
    /// Failed lookup preserves old
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

    /// Trusted lifecycle primitive, not an application chroot entry point.
    /// Authenticate root-policy authorization separately from filesystem search
    /// credentials, and hold lifecycle serialization through revalidation.
    /// Resolves beneath the old root and resets cwd to the new root. Lookup
    /// failure preserves admission; failed revalidation withdraws the old scope.
    pub fn change_root(
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
            .change_root(old, next, path, caller, revalidate)
    }

    /// Provision a distinct caller from the parent's retained root/cwd objects.
    /// The owner must independently authenticate both lifetimes and namespace
    /// membership, serialize them across this call, and revalidate both before
    /// publication. This does not select kernel routing or inherit credentials.
    pub fn derive_caller(
        &mut self,
        authenticated: Registration,
        parent: Scope,
        child: Scope,
        revalidate: impl FnOnce() -> bool,
    ) -> Result<(), Error> {
        if self.active != Some(authenticated) {
            return Err(Error::Identity);
        }
        self.policy_mut(authenticated)?
            .derive_caller(parent, child, revalidate)
    }

    /// Trusted caller lifecycle event spans all retained registration epochs.
    pub fn retire_caller(&mut self, caller: Caller) {
        for entry in self.entries.iter_mut().flatten() {
            entry.policy.retire_caller(caller);
        }
    }

    /// Trusted file-owner teardown, distinct from metadata-profile withdrawal.
    /// CallerRetired also represents credential/root withdrawal and must preserve
    /// established descriptions. Only the distinct copied FileOwnerRetired event
    /// delivers exit/exec teardown; local adapters must authenticate it likewise.
    pub fn retire_file_owner(&mut self, caller: Caller) {
        for entry in self.entries.iter_mut().flatten() {
            #[cfg(feature = "vfs-d1b-test")]
            let before = entry.files.len();
            entry.files.retire_caller(caller);
            #[cfg(feature = "vfs-d1b-test")]
            sunlight_ipc::debug_log(&alloc::format!(
                "[VFS-D1B] owner retirement pid={} incarnation={} image={} epoch={} handles={}->{}",
                caller.pid, caller.incarnation, caller.address_space_generation,
                entry.registration.epoch, before, entry.files.len(),
            ));
        }
    }

    pub fn retire_registration(&mut self, registration: Registration) -> Result<(), Error> {
        self.policy_mut(registration)?.retire();
        self.entries
            .iter_mut()
            .flatten()
            .find(|e| e.registration == registration)
            .ok_or(Error::Identity)?
            .files
            .clear();
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

    /// Trusted local preparation only. Authenticate the receipt and credentials
    /// independently of application input and serialize lifecycle through commit.
    /// Failed revalidation drops the provisional backend handle before publication.
    /// This API does not extend the metadata-only copied transport's grant ceiling.
    pub fn open_file(
        &mut self,
        authenticated: Registration,
        receipt: Receipt,
        path: &str,
        caller: sunlight_fs::Credential,
        access: crate::open::Access,
        revalidate: impl FnOnce() -> bool,
    ) -> Result<u64, Error> {
        self.open_file_with_options(authenticated, receipt, path, caller, access, None, revalidate)
            .map(|(token, _)| token)
    }

    fn open_file_with_options(
        &mut self,
        authenticated: Registration,
        receipt: Receipt,
        path: &str,
        caller: sunlight_fs::Credential,
        access: crate::open::Access,
        request: Option<(u64, u16, u64)>,
        revalidate: impl FnOnce() -> bool,
    ) -> Result<(u64, usize), Error> {
        use sunlight_fs::vfs::OpenOptions;
        use sunlight_ipc::vfs_rpc::file_transport as api;
        let (flags, mode, token) = request.unwrap_or_default();
        let options = OpenOptions {
            read: access.read(),
            write: access.write(),
            create_mode: (flags & api::OPEN_CREATE != 0).then_some(mode),
            exclusive: flags & api::OPEN_EXCLUSIVE != 0,
            truncate: flags & api::OPEN_TRUNCATE != 0,
        };
        let entry = self.file_entry_mut(authenticated)?;
        entry.files.capacity().map_err(Error::Filesystem)?;
        let anchor = if token == 0 {
            None
        } else {
            Some(entry.files.directory_handle(receipt.scope().caller, token).map_err(Error::Filesystem)?)
        };
        let file = entry.policy.open_file(
            authenticated,
            receipt,
            path,
            caller,
            options,
            flags & api::OPEN_STORAGE_ADMIN != 0,
            flags & api::OPEN_DIRECTORY != 0,
            flags & api::OPEN_EXECUTABLE != 0,
            anchor,
        )?;
        if !revalidate() {
            return Err(Error::Identity);
        }
        let size = file.executable_size();
        let token = entry.files.insert(receipt.scope().caller, file, access)
            .map_err(Error::Filesystem)?;
        Ok((token, size))
    }

    fn file_entry_mut(
        &mut self,
        authenticated: Registration,
    ) -> Result<&mut Entry<P, L, Device>, Error> {
        if self.active != Some(authenticated) {
            return Err(Error::Identity);
        }
        self.entries
            .iter_mut()
            .flatten()
            .find(|e| e.registration == authenticated)
            .map(|e| &mut **e)
            .ok_or(Error::Identity)
    }

    /// Descriptor calls require an independently authenticated owner identity.
    /// Namespace withdrawal does not revoke an already-open description; caller
    /// and service retirement do. Tokens never expose backend handle numbers.
    pub fn read_file(
        &mut self,
        registration: Registration,
        caller: Caller,
        token: u64,
        output: &mut [u8],
    ) -> Result<usize, Error> {
        self.file_entry_mut(registration)?
            .files
            .read(caller, token, output)
            .map_err(Error::Filesystem)
    }
    /// Bounded positional read on the same retained description as `read_file`.
    /// The shared cursor is neither consulted nor advanced.
    pub fn read_at_file(
        &mut self,
        registration: Registration,
        caller: Caller,
        token: u64,
        offset: u64,
        output: &mut [u8],
    ) -> Result<usize, Error> {
        self.file_entry_mut(registration)?
            .files
            .read_at(caller, token, offset, output)
            .map_err(Error::Filesystem)
    }
    pub fn write_file(
        &mut self,
        registration: Registration,
        caller: Caller,
        token: u64,
        input: &[u8],
    ) -> Result<usize, Error> {
        self.file_entry_mut(registration)?
            .files
            .write(caller, token, input)
            .map_err(Error::Filesystem)
    }
    pub fn seek_file(
        &mut self,
        registration: Registration,
        caller: Caller,
        token: u64,
        seek: crate::open::Seek,
    ) -> Result<usize, Error> {
        self.file_entry_mut(registration)?
            .files
            .seek(caller, token, seek)
            .map_err(Error::Filesystem)
    }
    pub fn stat_file(
        &mut self,
        registration: Registration,
        caller: Caller,
        token: u64,
    ) -> Result<FileStat, Error> {
        self.file_entry_mut(registration)?
            .files
            .stat(caller, token)
            .map_err(Error::Filesystem)
    }
    /// Truncate to zero without changing the shared description offset.
    pub fn truncate_file(
        &mut self,
        registration: Registration,
        caller: Caller,
        token: u64,
    ) -> Result<(), Error> {
        self.file_entry_mut(registration)?
            .files
            .truncate(caller, token)
            .map_err(Error::Filesystem)
    }
    pub fn duplicate_file(
        &mut self,
        registration: Registration,
        caller: Caller,
        token: u64,
    ) -> Result<u64, Error> {
        self.file_entry_mut(registration)?
            .files
            .duplicate(caller, token)
            .map_err(Error::Filesystem)
    }
    pub fn close_file(
        &mut self,
        registration: Registration,
        caller: Caller,
        token: u64,
    ) -> Result<(), Error> {
        self.file_entry_mut(registration)?
            .files
            .close(caller, token)
            .map_err(Error::Filesystem)
    }
    /// Re-key a surviving descriptor to the replacement owner of the same
    /// process (exec) or a thread in the same address space. The kernel
    /// transport authenticates the old caller; the relation is rechecked here.
    pub fn transfer_file(
        &mut self,
        registration: Registration,
        caller: Caller,
        token: u64,
        new_caller: Caller,
    ) -> Result<u64, Error> {
        if !transfer_owner_relation(caller, new_caller) {
            return Err(Error::Identity);
        }
        self.file_entry_mut(registration)?
            .files
            .transfer(caller, token, new_caller)
            .map_err(Error::Filesystem)
    }

    pub fn dispatch_file(
        &mut self,
        control: &mut impl control::Control,
        registration: Registration,
        bytes: &[u8; sunlight_ipc::vfs_rpc::file_transport::FRAME_BYTES],
    ) -> bool {
        use sunlight_ipc::vfs_rpc::file_transport as api;
        let Some(reply_bytes) = self.file_request(control, registration, bytes) else {
            return false;
        };
        #[cfg(feature = "vfs-d2b-test")]
        if self.fixture_endpoint.is_some() {
            let request = api::Request::decode(bytes).unwrap();
            if request.op == api::PREAD && request.arg0 == 4096 {
                let start = sunlight_ipc::monotonic_millis();
                loop {
                    let action = unsafe {
                        sunlight_libc::sys::syscall2(0xf1c1, 28, request.request_id)
                    };
                    match action {
                        0 => {
                            assert!(sunlight_ipc::monotonic_millis() - start < 3000);
                            sunlight_ipc::process_yield();
                        }
                        1 => break,
                        2 => {
                            let entry = self.file_entry_mut(registration).unwrap();
                            assert_eq!(entry.files.len(), 1);
                            assert!(sunlight_ipc::endpoint_destroy(self.fixture_endpoint.take().unwrap()));
                            assert!(!control.file_reply(registration.epoch, &reply_bytes));
                            self.retire_registration(registration).unwrap();
                            assert_eq!(self.entries.iter().flatten().find(|entry|
                                entry.registration == registration).unwrap().files.len(), 0);
                            sunlight_ipc::debug_log(&alloc::format!(
                                "[VFS-D2B] endpoint-withdrawn epoch={} id={} token={} handles=1->0 reply_ack=0",
                                registration.epoch, request.request_id, request.token
                            ));
                            return false;
                        }
                        _ => panic!("mmap lifecycle rendezvous rejected"),
                    }
                }
            }
        }
        let accepted = control.file_reply(registration.epoch, &reply_bytes);
        #[cfg(feature = "vfs-d1b-test")]
        {
            let request = api::Request::decode(bytes).unwrap();
            let reply = api::Reply::decode(&reply_bytes).unwrap();
            sunlight_ipc::debug_log(&alloc::format!(
                "[VFS-D1B] service pid={} incarnation={} image={} epoch={} id={} op={} token={} result={} errno={} handles={} ack={}",
                request.caller.pid, request.caller.incarnation, request.caller.address_space_generation,
                registration.epoch, request.request_id, request.op, request.token, reply.result0,
                reply.errno, self.entries.iter().flatten().find(|entry| entry.registration == registration)
                    .map_or(0, |entry| entry.files.len()), u8::from(accepted),
            ));
        }
        if accepted {
            return true;
        }
        if let Ok(reply) = api::Reply::decode(&reply_bytes) {
            if reply.op == api::OPEN && reply.errno == 0 {
                let _ = self.close_file(registration, reply.caller, reply.result0);
            }
        }
        false
    }

    /// Dispatch one independently authenticated copied file request. Ordinary
    /// IPC never reaches this path, regardless of frame contents. The caller
    /// identity is authenticated by the kernel transport before the frame is
    /// pulled; the service still checks it against every descriptor it touches.
    pub fn file_request(
        &mut self,
        control: &mut impl control::Control,
        authenticated: Registration,
        bytes: &[u8; sunlight_ipc::vfs_rpc::file_transport::FRAME_BYTES],
    ) -> Option<[u8; sunlight_ipc::vfs_rpc::file_transport::FRAME_BYTES]> {
        use crate::open::{Access, Seek};
        use sunlight_fs::Credential;
        use sunlight_ipc::vfs_rpc::file_transport as api;
        let request = api::Request::decode(bytes).ok()?;
        if request.registration != authenticated {
            return None;
        }
        let reply = |errno: u64, result0: u64, result1: u64, payload: &[u8]| {
            let reply = api::Reply {
                registration: authenticated,
                caller: request.caller,
                request_id: request.request_id,
                op: request.op,
                errno,
                result0,
                result1,
                length: payload.len(),
                payload,
            };
            reply.encode().ok()
        };
        let failed = |error: &Error| reply(errno_for(error), 0, 0, &[]);
        match request.op {
            #[cfg(any(feature = "vfs-c1e2-test", feature = "vfs-c2e-candidate"))]
            api::MKDIR | api::UNLINK | api::RENAME => {
                if request.token != 0 && !cfg!(feature = "vfs-c2e-candidate") {
                    return reply(95, 0, 0, &[]);
                }
                let receipt = Receipt::from_parts(
                    authenticated,
                    Scope {
                        caller: request.caller,
                        namespace_id: request.namespace_id,
                        namespace_generation: request.namespace_generation,
                    },
                    request.receipt_id,
                    request.receipt_generation,
                )?;
                let entry = self.file_entry_mut(authenticated).ok()?;
                let anchor = if request.token == 0 {
                    None
                } else {
                    match entry.files.directory_handle(request.caller, request.token) {
                        Ok(handle) => Some(handle),
                        Err(error) => return failed(&Error::Filesystem(error)),
                    }
                };
                let target = if request.op == api::RENAME && request.arg1 & api::RENAME_DISTINCT_DIRS != 0 {
                    match entry.files.directory_handle(request.caller, request.arg0) {
                        Ok(handle) => Some(handle),
                        Err(error) => return failed(&Error::Filesystem(error)),
                    }
                } else {
                    None
                };
                match entry.policy.mutate_namespace(authenticated, receipt, request, (anchor, target)) {
                    Ok(()) => reply(0, 0, 0, &[]),
                    Err(Error::Filesystem(FsError::Unsupported))
                        if request.op == api::RENAME && anchor.is_some() => reply(18, 0, 0, &[]),
                    Err(Error::Filesystem(FsError::Unsupported)) => reply(38, 0, 0, &[]),
                    Err(error) => failed(&error),
                }
            }
            #[cfg(feature = "vfs-c1e2-test")]
            api::MOUNTINFO | api::STATFS => {
                use crate::directory::attachment::Error as ObservationError;
                let Some(receipt) = Receipt::from_parts(
                    authenticated,
                    Scope {
                        caller: request.caller,
                        namespace_id: request.namespace_id,
                        namespace_generation: request.namespace_generation,
                    },
                    request.receipt_id,
                    request.receipt_generation,
                ) else {
                    return reply(22, 0, 0, &[]);
                };
                let path = if request.op == api::STATFS {
                    Some(core::str::from_utf8(request.payload).ok()?)
                } else {
                    None
                };
                let entry = self.file_entry_mut(authenticated).ok()?;
                match entry.policy.mount_observation(
                    authenticated, receipt, Credential { uid: request.uid, gid: request.gid }, path,
                ) {
                    Ok((revision, mount, payload)) => reply(0, revision, mount, &payload),
                    Err(ObservationError::Stale) => reply(116, 0, 0, &[]),
                    Err(ObservationError::Filesystem(error)) => failed(&Error::Filesystem(error)),
                    Err(_) => reply(5, 0, 0, &[]),
                }
            }
            api::OPEN => {
                if !cfg!(feature = "vfs-c2e-candidate")
                    && (request.token != 0 || request.arg0 & (api::OPEN_DIRECTORY | api::OPEN_EXECUTABLE) != 0)
                {
                    return reply(95, 0, 0, &[]);
                }
                let Some(receipt) = Receipt::from_parts(
                    authenticated,
                    Scope {
                        caller: request.caller,
                        namespace_id: request.namespace_id,
                        namespace_generation: request.namespace_generation,
                    },
                    request.receipt_id,
                    request.receipt_generation,
                ) else {
                    return reply(22, 0, 0, &[]);
                };
                let access = match request.arg0 & api::OPEN_ACCESS {
                    api::ACCESS_READ => Access::Read,
                    api::ACCESS_WRITE => Access::Write,
                    _ => Access::ReadWrite,
                };
                let credential = Credential {
                    uid: request.uid,
                    gid: request.gid,
                };
                // Revalidation is independent of the request payload and runs
                // before publication. The finish path checks lifecycle again.
                let path = core::str::from_utf8(request.payload).ok()?;
                let result =
                    self.open_file_with_options(authenticated, receipt, path, credential, access,
                        Some((request.arg0, request.arg1 as u16, request.token)), || {
                        control
                            .file_next(authenticated.epoch)
                            .ok()
                            .flatten()
                            .as_ref()
                            == Some(bytes)
                    });
                match result {
                    Ok((token, size)) => reply(0, token, size as u64, &[]),
                    Err(error) => failed(&error),
                }
            }
            api::READ => {
                let mut buffer = [0; api::IO_BYTES];
                let count = request.arg1 as usize;
                match self.read_file(
                    authenticated,
                    request.caller,
                    request.token,
                    &mut buffer[..count],
                ) {
                    Ok(read) => reply(0, read as u64, 0, &buffer[..read]),
                    Err(error) => failed(&error),
                }
            }
            api::PREAD => {
                let mut buffer = [0; api::IO_BYTES];
                let count = request.arg1 as usize;
                match self.read_at_file(
                    authenticated,
                    request.caller,
                    request.token,
                    request.arg0,
                    &mut buffer[..count],
                ) {
                    Ok(read) => reply(0, read as u64, 0, &buffer[..read]),
                    Err(error) => failed(&error),
                }
            }
            api::WRITE => {
                match self.write_file(
                    authenticated,
                    request.caller,
                    request.token,
                    request.payload,
                ) {
                    Ok(written) => reply(0, written as u64, 0, &[]),
                    Err(error) => failed(&error),
                }
            }
            api::SEEK => {
                let seek = match request.arg0 {
                    api::SEEK_START => Seek::Start(request.arg1),
                    api::SEEK_CURRENT => Seek::Current(request.arg1 as i64),
                    _ => Seek::End(request.arg1 as i64),
                };
                match self.seek_file(authenticated, request.caller, request.token, seek) {
                    Ok(position) => reply(0, position as u64, 0, &[]),
                    Err(error) => failed(&error),
                }
            }
            api::STAT | api::STAT_AT => match if request.op == api::STAT {
                self.stat_file(authenticated, request.caller, request.token)
            } else {
                #[cfg(feature = "vfs-c2e-candidate")]
                {
                    let receipt = Receipt::from_parts(authenticated, Scope {
                        caller: request.caller,
                        namespace_id: request.namespace_id,
                        namespace_generation: request.namespace_generation,
                    }, request.receipt_id, request.receipt_generation)?;
                    let entry = self.file_entry_mut(authenticated).ok()?;
                    let path = core::str::from_utf8(request.payload).ok()?;
                    entry.files.directory_handle(request.caller, request.token)
                        .map_err(Error::Filesystem)
                        .and_then(|anchor| entry.policy.stat_at(authenticated, receipt, anchor, path,
                            Credential { uid: request.uid, gid: request.gid }))
                }
                #[cfg(not(feature = "vfs-c2e-candidate"))]
                return reply(95, 0, 0, &[]);
            } {
                Ok(stat) => {
                    let kind = match stat.file_type {
                        sunlight_fs::FileType::File => 1,
                        sunlight_fs::FileType::Directory => 2,
                    };
                    let record = api::encode_native_stat(
                        stat.size as u64,
                        stat.uid,
                        stat.gid,
                        stat.mode,
                        kind,
                        stat.nlinks,
                    );
                    reply(0, 0, 0, &record)
                }
                Err(error) => failed(&error),
            },
            api::TRUNCATE => match self.truncate_file(authenticated, request.caller, request.token)
            {
                Ok(()) => reply(0, 0, 0, &[]),
                Err(error) => failed(&error),
            },
            api::CLOSE => match self.close_file(authenticated, request.caller, request.token) {
                Ok(()) => reply(0, 0, 0, &[]),
                Err(error) => failed(&error),
            },
            api::DUP => match self.duplicate_file(authenticated, request.caller, request.token) {
                Ok(token) => reply(0, token, 0, &[]),
                Err(error) => failed(&error),
            },
            api::TRANSFER => {
                let new_caller = match api::decode_caller(request.payload) {
                    Ok(caller) => caller,
                    Err(_) => return reply(22, 0, 0, &[]),
                };
                match self.transfer_file(authenticated, request.caller, request.token, new_caller) {
                    Ok(token) => reply(0, token, 0, &[]),
                    Err(error) => failed(&error),
                }
            }
            _ => None,
        }
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

    fn policy_mut(&mut self, registration: Registration) -> Result<&mut Policy<P, L, Device>, Error> {
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

/// A descriptor may move only within one process lifetime (exec changes the VM
/// generation but not PID/incarnation) or within one address space (threads and
/// fork-like sharing keep the generation). It never moves to an unrelated
/// process, and the exact old/new identities must differ.
fn transfer_owner_relation(old: Caller, new: Caller) -> bool {
    old != new
        && (old.pid == new.pid && old.incarnation == new.incarnation
            || old.address_space_generation == new.address_space_generation)
}

/// POSIX errno for a policy failure. Shared by the copied file dispatch and
/// the service loop so both report the same value for the same failure.
pub(crate) fn errno_for(error: &Error) -> u64 {
    match error {
        Error::Filesystem(FsError::NotFound) => 2,
        Error::Filesystem(FsError::BadHandle) => 9,
        Error::Filesystem(FsError::NoMemory) => 12,
        Error::Filesystem(FsError::Busy) => 16,
        Error::Filesystem(FsError::AlreadyExists) => 17,
        Error::Filesystem(FsError::PermissionDenied | FsError::OperationNotPermitted) => 13,
        Error::Filesystem(FsError::NotDir) => 20,
        Error::Filesystem(FsError::IsDir) => 21,
        Error::Filesystem(FsError::InvalidPath) => 22,
        Error::Filesystem(FsError::TooManyOpenFiles) => 24,
        Error::Filesystem(FsError::NoSpace) => 28,
        Error::Filesystem(FsError::ReadOnlyFilesystem) => 30,
        Error::Filesystem(FsError::Unsupported) => 95,
        _ => 5,
    }
}

/// Policy supplied by a trusted lifecycle owner, never decoded from a retain frame.
pub struct ProvisionedScope<'a, Device: BlockDevice = NullDevice> {
    pub registration: Registration,
    pub scope: Scope,
    pub namespace: &'a DirectoryNamespace<Device>,
    pub root: &'a str,
    pub directory: &'a str,
}

/// The provider must serialize lifecycle changes across both callbacks and return
/// false if caller, registration, namespace identity, or path policy changed.
/// A production implementation requires an independently authenticated source.
pub trait ScopeAuthority<Device: BlockDevice = NullDevice> {
    fn resolve(&mut self) -> Result<ProvisionedScope<'_, Device>, Error>;
    fn revalidate(&mut self, registration: Registration, scope: Scope) -> bool;
}

impl<const P: usize, const L: usize, const R: usize, Device: BlockDevice> Runtime<P, L, R, Device> {
    /// Install pins before retain handling, then fail closed on lifecycle races.
    /// Exclusive runtime access prevents requests from observing provisional pins.
    pub fn provision(&mut self, authority: &mut impl ScopeAuthority<Device>) -> Result<Scope, Error> {
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
