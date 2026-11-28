//! Copied regular-file operation channel. Scheduler -> broker -> runtime
//! serialization covers admission, publication, parking and completion. Every
//! request carries the full authenticated caller identity; the service must
//! still check it against each descriptor it touches.
use super::*;
use crate::process::ProcessState;
use sunlight_ipc::vfs_rpc::file_transport as api;

/// Admission starts the budget; delivery and caller retries never extend it.
pub(super) const TIMEOUT_TICKS: u64 = 5 * crate::timekeeping::TICK_HZ;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Identity,
    Busy,
    Capacity,
    Protocol,
    NotReady,
}

pub(super) struct Publication {
    id: u64,
    pub(super) identity: AdmissionIdentity,
    pub(super) op: u64,
    pub(super) replied: bool,
    failed: bool,
    woken: bool,
    started_tick: u64,
    token: u64,
    compensation: bool,
    executable: bool,
    buffer: alloc::vec::Vec<u8>,
    #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
    observation: Option<(Receipt, alloc::vec::Vec<u8>, u64)>,
    dispatched: bool,
}

/// One caller-owned file request. Borrowed paths and payloads stay valid only
/// for the duration of the syscall that admits them.
pub(crate) enum FileOperation<'a> {
    #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
    ObserveMount { path: Option<&'a str> },
    #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
    MutateNamespace { op: u64, flags: u64, anchor: u64, paths: &'a str, storage_admin: bool },
    #[cfg(feature = "vfs_c1e1_test")]
    PrepareStorage { source: u64, path: &'a str },
    #[cfg(feature = "vfs_c1e2_test")]
    AbortStorage { token: u64 },
    #[cfg(feature = "vfs_c1e2_test")]
    PublishStorage { token: u64 },
    #[cfg(feature = "vfs_c1e2_test")]
    InventoryStorage { index: u64 },
    #[cfg(feature = "vfs_c1e2_test")]
    UnmountStorage { token: u64 },
    Open {
        anchor: u64,
        path: &'a str,
        access: u64,
        mode: u64,
    },
    Read {
        token: u64,
        length: usize,
    },
    #[cfg(any(test, feature = "vfs_c2e_candidate", all(debug_assertions, any(feature = "vfs_d1b_test", feature = "vfs_service_private_mmap"))))]
    Pread { token: u64, offset: u64, length: usize },
    Write {
        token: u64,
        payload: &'a [u8],
    },
    Seek {
        token: u64,
        whence: u64,
        offset: i64,
    },
    #[cfg(feature = "vfs_c2e_candidate")]
    StatAt { token: u64, path: &'a str },
    Stat {
        token: u64,
    },
    Truncate {
        token: u64,
    },
    Close {
        token: u64,
    },
    Dup {
        token: u64,
    },
}

impl FileOperation<'_> {
    fn token(&self) -> u64 {
        match *self {
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            Self::ObserveMount { .. } => 0,
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            Self::MutateNamespace { anchor, .. } => anchor,
            #[cfg(feature = "vfs_c1e1_test")]
            Self::PrepareStorage { source, .. } => source,
            #[cfg(feature = "vfs_c1e2_test")]
            Self::AbortStorage { token } | Self::PublishStorage { token }
            | Self::UnmountStorage { token } => token,
            #[cfg(feature = "vfs_c1e2_test")]
            Self::InventoryStorage { index } => index,
            Self::Open { anchor, .. } => anchor,
            #[cfg(any(test, feature = "vfs_c2e_candidate", all(debug_assertions, any(feature = "vfs_d1b_test", feature = "vfs_service_private_mmap"))))]
            Self::Pread { token, .. } => token,
            #[cfg(feature = "vfs_c2e_candidate")]
            Self::StatAt { token, .. } => token,
            Self::Read { token, .. }
            | Self::Write { token, .. }
            | Self::Seek { token, .. }
            | Self::Stat { token }
            | Self::Truncate { token }
            | Self::Close { token }
            | Self::Dup { token } => token,
        }
    }

    fn op(&self) -> u64 {
        match self {
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            Self::ObserveMount { path: None } => api::MOUNTINFO,
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            Self::ObserveMount { path: Some(_) } => api::STATFS,
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            Self::MutateNamespace { op, .. } => *op,
            #[cfg(feature = "vfs_c1e1_test")]
            Self::PrepareStorage { .. } => api::PREPARE_STORAGE,
            #[cfg(feature = "vfs_c1e2_test")]
            Self::AbortStorage { .. } => api::ABORT_STORAGE,
            #[cfg(feature = "vfs_c1e2_test")]
            Self::PublishStorage { .. } => api::PUBLISH_STORAGE,
            #[cfg(feature = "vfs_c1e2_test")]
            Self::InventoryStorage { .. } => api::INVENTORY_STORAGE,
            #[cfg(feature = "vfs_c1e2_test")]
            Self::UnmountStorage { .. } => api::UNMOUNT_STORAGE,
            Self::Open { .. } => api::OPEN,
            Self::Read { .. } => api::READ,
            #[cfg(any(test, feature = "vfs_c2e_candidate", all(debug_assertions, any(feature = "vfs_d1b_test", feature = "vfs_service_private_mmap"))))]
            Self::Pread { .. } => api::PREAD,
            Self::Write { .. } => api::WRITE,
            Self::Seek { .. } => api::SEEK,
            Self::Stat { .. } => api::STAT,
            #[cfg(feature = "vfs_c2e_candidate")]
            Self::StatAt { .. } => api::STAT_AT,
            Self::Truncate { .. } => api::TRUNCATE,
            Self::Close { .. } => api::CLOSE,
            Self::Dup { .. } => api::DUP,
        }
    }
}

/// Caller-owned terminal result. Payload is valid until dropped.
pub(crate) struct FileCompletion {
    pub(crate) op: u64,
    pub(crate) errno: u64,
    pub(crate) result0: u64,
    pub(crate) result1: u64,
    pub(crate) registration_epoch: u64,
    pub(crate) payload: alloc::vec::Vec<u8>,
}

pub(crate) enum FileCall {
    Pending,
    Complete(FileCompletion),
}

impl Runtime {
    #[cfg(all(not(test), debug_assertions, feature = "vfs_d2b_test"))]
    pub(crate) fn fixture_mapping_read(
        &self,
        caller: Caller,
        service: Caller,
        id: u64,
    ) -> Option<(u64, u64)> {
        let registration = self.registry.active()?;
        if !owner_matches(registration.owner, service) {
            return None;
        }
        self.file_publications.iter().flatten().find_map(|p| {
            (p.id == id
                && p.identity.caller == caller
                && p.identity.registration == registration
                && p.op == api::PREAD
                && !p.replied)
                .then_some((registration.epoch, p.token))
        })
    }

    fn check_descriptor(
        &self,
        identity: AdmissionIdentity,
        descriptor: Option<(Caller, u64)>,
    ) -> Result<(), Error> {
        if self.registry.active() == Some(identity.registration)
            && descriptor == Some((identity.caller, identity.registration.epoch))
        {
            Ok(())
        } else {
            Err(Error::NotReady)
        }
    }

    /// Hold the file-owner barrier until the kernel has published every
    /// surviving descriptor transfer for this caller. Set before retirement so
    /// the service cannot observe an empty transfer set in the publication gap.
    pub(super) fn expect_file_transfer(&mut self, caller: Caller) {
        for remote in self.remote.iter_mut().flatten() {
            for delivery in remote.callers.iter_mut().flatten() {
                if delivery.caller == caller {
                    delivery.file_transfer_pending = true;
                }
            }
        }
    }

    /// Publication is complete (successfully or fail-closed). Pending transfer
    /// publications, if any, still keep the barrier closed until they reply.
    pub(super) fn finish_file_transfer(&mut self, caller: Caller) {
        for remote in self.remote.iter_mut().flatten() {
            for delivery in remote.callers.iter_mut().flatten() {
                if delivery.caller == caller {
                    delivery.file_transfer_pending = false;
                }
            }
        }
    }

    fn expire_files(&mut self, now_tick: u64) {
        for publication in self.file_publications.iter_mut().flatten() {
            if !publication.compensation
                && !publication.replied
                && now_tick.wrapping_sub(publication.started_tick) >= TIMEOUT_TICKS
            {
                #[cfg(feature = "vfs_c1e1_test")]
                publication.cancel_storage();
                publication.failed = true;
                publication.replied = true;
                publication.buffer.clear();
            }
        }
        // Kernel-origin transfers and fire-and-forget closes have no caller to
        // take their terminal result. Once terminal (success, refusal or
        // timeout) the slot is free; the barrier may then proceed and close
        // anything left untransferred.
        for slot in &mut self.file_publications {
            if slot.as_ref().is_some_and(|publication| {
                matches!(publication.op, api::TRANSFER | api::CLOSE) && publication.replied
                    && !publication.executable
            }) {
                *slot = None;
            }
        }
    }

    pub(super) fn caller_publication(&self, caller: Caller) -> Option<usize> {
        self.file_publications
            .iter()
            // A fire-and-forget close names the live caller but does not own
            // it: the fd is already consumed and the caller may proceed with a
            // new operation while the service finishes the teardown.
            .position(|p| {
                p.as_ref()
                    .is_some_and(|p| p.identity.caller == caller && p.op != api::CLOSE)
            })
    }

    /// A late PREAD has no cursor side effect and its reply no longer names
    /// a publication. Keep this narrower than whole-image retirement.
    #[cfg(any(test, feature = "vfs_c2e_candidate", all(debug_assertions, feature = "vfs_service_private_mmap")))]
    fn cancel_mapping_read(&mut self, caller: Caller, epoch: u64, token: u64) {
        if let Some(slot) = self.caller_publication(caller) {
            let publication = self.file_publications[slot].as_ref().unwrap();
            if publication.op == api::PREAD
                && publication.token == token
                && publication.identity.registration.epoch == epoch
            {
                self.file_publications[slot] = None;
            }
        }
    }

    #[cfg(feature = "vfs_c2e_candidate")]
    pub(super) fn settle_executable(&mut self, caller: Caller) {
        if let Some(slot) = self.caller_publication(caller) {
            let pending = self.file_publications[slot].as_ref().unwrap();
            if pending.op == api::OPEN && pending.executable {
                if pending.replied && !pending.failed {
                    let _ = self.finish_open(slot, |_, _| Err(125));
                } else {
                    self.file_publications[slot] = None;
                }
            }
        }
        if let Some(close) = self.file_publications.iter_mut().flatten().find(|p| {
            p.identity.caller == caller && p.op == api::CLOSE && p.executable
        }) {
            let (epoch, token) = (close.identity.registration.epoch, close.token);
            close.executable = false;
            close.replied = false;
            self.cancel_mapping_read(caller, epoch, token);
        }
    }

    /// Lexicographically latest retained metadata lease for this caller. The
    /// service still proves the receipt names a live binding before publishing.
    pub(super) fn caller_receipt(&self, identity: AdmissionIdentity) -> Option<Receipt> {
        self.remote
            .iter()
            .flatten()
            .find(|r| r.registration == identity.registration)
            .into_iter()
            .flat_map(|r| r.local.iter().flatten())
            .filter_map(|lease| match lease {
                LocalLease::Bound { receipt, .. } if receipt.scope().caller == identity.caller => {
                    Some(*receipt)
                }
                _ => None,
            })
            .max_by_key(|receipt| receipt.request_id())
    }

    pub(super) fn publish_file(
        &mut self,
        identity: AdmissionIdentity,
        operation: FileOperation<'_>,
        receipt: Option<Receipt>,
        sched: &mut Scheduler,
    ) -> Result<(), Error> {
        if self.registry.active() != Some(identity.registration) {
            return Err(Error::NotReady);
        }
        let idx = sched
            .processes
            .iter()
            .position(|p| process_identity(p) == identity.caller && !p.is_exiting())
            .ok_or(Error::Identity)?;
        if sched.processes[idx].state != ProcessState::Running
            || sched.processes[idx].pending_call.is_some()
            || sched.processes[idx].ipc_reply_target.is_some()
            || self.caller_publication(identity.caller).is_some()
        {
            return Err(Error::Busy);
        }
        let slot = self
            .file_publications
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let request_id = self.file_ids.allocate().ok_or(Error::Capacity)?;
        let (op, token, arg0, arg1, payload, receipt_words) = match &operation {
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            FileOperation::ObserveMount { path } => {
                let receipt = receipt.ok_or(Error::NotReady)?;
                (operation.op(), 0, 0, 0, path.unwrap_or("").as_bytes(),
                    Some((receipt.scope().namespace_id, receipt.scope().namespace_generation,
                        receipt.request_id(), receipt.generation())))
            }
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            FileOperation::MutateNamespace { op, flags, anchor, paths, storage_admin } => {
                let receipt = receipt.ok_or(Error::NotReady)?;
                if !api::namespace_mutation(*op) {
                    return Err(Error::Protocol);
                }
                let distinct = *op == api::RENAME && *anchor != 0 && *flags != 0;
                let options = u64::from(*storage_admin) | if distinct { api::RENAME_DISTINCT_DIRS } else { 0 };
                (*op, *anchor, *flags, options, paths.as_bytes(),
                    Some((receipt.scope().namespace_id, receipt.scope().namespace_generation,
                        receipt.request_id(), receipt.generation())))
            }
            #[cfg(feature = "vfs_c1e1_test")]
            FileOperation::PrepareStorage { source, path } => {
                (api::PREPARE_STORAGE, *source, 0, 0, path.as_bytes(), None)
            }
            #[cfg(feature = "vfs_c1e2_test")]
            FileOperation::AbortStorage { token } => (api::ABORT_STORAGE, *token, 0, 0, &[][..], None),
            #[cfg(feature = "vfs_c1e2_test")]
            FileOperation::PublishStorage { token } => {
                (api::PUBLISH_STORAGE, *token, 0, 0, &[][..], None)
            }
            #[cfg(feature = "vfs_c1e2_test")]
            FileOperation::InventoryStorage { index } => {
                (api::INVENTORY_STORAGE, *index, 0, 0, &[][..], None)
            }
            #[cfg(feature = "vfs_c1e2_test")]
            FileOperation::UnmountStorage { token } => {
                (api::UNMOUNT_STORAGE, *token, 0, 0, &[][..], None)
            }
            FileOperation::Open { anchor, path, access, mode } => {
                let receipt = receipt.ok_or(Error::NotReady)?;
                (
                    api::OPEN,
                    *anchor,
                    *access,
                    *mode,
                    path.as_bytes(),
                    Some((
                        receipt.scope().namespace_id,
                        receipt.scope().namespace_generation,
                        receipt.request_id(),
                        receipt.generation(),
                    )),
                )
            }
            FileOperation::Read { token, length } => {
                (api::READ, *token, 0, *length as u64, &[][..], None)
            }
            #[cfg(any(test, feature = "vfs_c2e_candidate", all(debug_assertions, any(feature = "vfs_d1b_test", feature = "vfs_service_private_mmap"))))]
            FileOperation::Pread { token, offset, length } => {
                (api::PREAD, *token, *offset, *length as u64, &[][..], None)
            }
            FileOperation::Write { token, payload } => (api::WRITE, *token, 0, 0, *payload, None),
            FileOperation::Seek {
                token,
                whence,
                offset,
            } => (api::SEEK, *token, *whence, *offset as u64, &[][..], None),
            #[cfg(feature = "vfs_c2e_candidate")]
            FileOperation::StatAt { token, path } => {
                let receipt = receipt.ok_or(Error::NotReady)?;
                (api::STAT_AT, *token, 0, 0, path.as_bytes(),
                    Some((receipt.scope().namespace_id, receipt.scope().namespace_generation,
                        receipt.request_id(), receipt.generation())))
            }
            FileOperation::Stat { token } => (api::STAT, *token, 0, 0, &[][..], None),
            FileOperation::Truncate { token } => (api::TRUNCATE, *token, 0, 0, &[][..], None),
            FileOperation::Close { token } => (api::CLOSE, *token, 0, 0, &[][..], None),
            FileOperation::Dup { token } => (api::DUP, *token, 0, 0, &[][..], None),
        };
        let (namespace_id, namespace_generation, receipt_id, receipt_generation) =
            receipt_words.unwrap_or((0, 0, 0, 0));
        // Credentials are carried only by OPEN: it admits the description and
        // must prove the caller. Every later op is authorized by the exact
        // authenticated caller against an already-open token, and the codec
        // rejects credential words on those operations.
        let (uid, gid) = match &operation {
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            FileOperation::ObserveMount { .. } | FileOperation::MutateNamespace { .. } => (identity.uid, identity.gid),
            FileOperation::Open { .. } => (identity.uid, identity.gid),
            #[cfg(feature = "vfs_c2e_candidate")]
            FileOperation::StatAt { .. } => (identity.uid, identity.gid),
            #[cfg(feature = "vfs_c1e2_test")]
            FileOperation::InventoryStorage { .. } => (identity.uid, identity.gid),
            _ => (0, 0),
        };
        let request = api::Request {
            registration: identity.registration,
            caller: identity.caller,
            request_id,
            op,
            token,
            arg0,
            arg1,
            length: payload.len(),
            namespace_id,
            namespace_generation,
            receipt_id,
            receipt_generation,
            uid,
            gid,
            payload,
        };
        let bytes = request.encode().map_err(|_| Error::Protocol)?;
        let mut buffer = alloc::vec::Vec::new();
        buffer
            .try_reserve_exact(api::FRAME_BYTES)
            .map_err(|_| Error::Capacity)?;
        buffer.extend_from_slice(&bytes);
        #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
        let observation = if api::retained_path(op) {
            let mut path = alloc::vec::Vec::new();
            path.try_reserve_exact(payload.len()).map_err(|_| Error::Capacity)?;
            path.extend_from_slice(payload);
            Some((receipt.ok_or(Error::NotReady)?, path, arg0))
        } else {
            None
        };
        self.file_publications[slot] = Some(Publication {
            id: request_id,
            identity,
            op,
            replied: false,
            failed: false,
            woken: false,
            started_tick: sched.global_tick,
            token: operation.token(),
            compensation: false,
            executable: op == api::OPEN && arg0 & api::OPEN_EXECUTABLE != 0,
            buffer,
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            observation,
            dispatched: false,
        });
        // Completion and teardown cannot observe a published request with an
        // unparked caller: publication and the parking transition are atomic
        // under the scheduler lock, exactly as the stat and chdir channels.
        sched.processes[idx].state = ProcessState::BlockedOnIpc;
        Ok(())
    }

    /// Publish a fire-and-forget close for a descriptor whose final kernel
    /// reference was just dropped. The caller has already consumed the fd and
    /// must not park: re-executing `close(2)` would find the fd gone. The
    /// service still owns the exact token and performs the teardown; no caller
    /// takes the terminal result, so the slot is released on reply or timeout.
    pub(super) fn publish_file_close(
        &mut self,
        registration: Registration,
        caller: Caller,
        token: u64,
        now_tick: u64,
    ) -> Result<(), Error> {
        if self.registry.active() != Some(registration) {
            return Err(Error::NotReady);
        }
        let slot = self
            .file_publications
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let request_id = self.file_ids.allocate().ok_or(Error::Capacity)?;
        let bytes = Self::file_close_frame(registration, caller, token, request_id)?;
        let mut buffer = alloc::vec::Vec::new();
        buffer
            .try_reserve_exact(api::FRAME_BYTES)
            .map_err(|_| Error::Capacity)?;
        buffer.extend_from_slice(&bytes);
        self.file_publications[slot] = Some(Publication {
            id: request_id,
            identity: AdmissionIdentity {
                caller,
                uid: 0,
                gid: 0,
                groups: [],
                registration,
            },
            op: api::CLOSE,
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            observation: None,
            dispatched: false,
            replied: false,
            failed: false,
            woken: true,
            started_tick: now_tick,
            token,
            compensation: false,
            executable: false,
            buffer,
        });
        Ok(())
    }

    fn file_close_frame(
        registration: Registration,
        caller: Caller,
        token: u64,
        request_id: u64,
    ) -> Result<[u8; api::FRAME_BYTES], Error> {
        api::Request {
            registration,
            caller,
            request_id,
            op: api::CLOSE,
            token,
            arg0: 0,
            arg1: 0,
            length: 0,
            namespace_id: 0,
            namespace_generation: 0,
            receipt_id: 0,
            receipt_generation: 0,
            uid: 0,
            gid: 0,
            payload: &[],
        }
        .encode()
        .map_err(|_| Error::Protocol)
    }

    pub(super) fn next_file(
        &mut self,
        registration: Registration,
        output: &mut [u8; api::FRAME_BYTES],
    ) -> Result<Option<u64>, Error> {
        if !self
            .remote
            .iter()
            .flatten()
            .any(|remote| remote.registration == registration)
        {
            return Err(Error::Identity);
        }
        let Some(publication) = self
            .file_publications
            .iter_mut()
            .flatten()
            .filter(|p| p.identity.registration == registration && !p.replied && !p.dispatched)
            .min_by_key(|p| p.id)
        else {
            return Ok(None);
        };
        if publication.buffer.len() != api::FRAME_BYTES {
            return Err(Error::Protocol);
        }
        output.copy_from_slice(&publication.buffer);
        publication.dispatched =
            api::namespace_mutation(publication.op) || publication.op == api::PUBLISH_STORAGE;
        let id = publication.id;
        #[cfg(feature = "vfs_c1e10_test")]
        if publication.op == api::PUBLISH_STORAGE {
            crate::serial_println!("[VFS-C1E10] pulled publication {}", id);
        }
        #[cfg(feature = "vfs_c1e2_test")]
        if api::mount_mutation(publication.op) {
            for pending in self.file_publications.iter_mut().flatten() {
                if pending.identity.registration == registration
                    && pending.replied && api::mount_observation(pending.op)
                {
                    pending.failed = true;
                }
            }
        }
        Ok(Some(id))
    }

    #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
    pub(super) fn revalidate_storage_files(&mut self, sched: &mut Scheduler) {
        for slot in 0..self.file_publications.len() {
            let invalid = self.file_publications[slot].as_ref().is_some_and(|publication| {
                (api::namespace_mutation(publication.op) || publication.op == api::STAT_AT) && !publication.replied
                    && !self.namespace_mutation_live(publication, sched)
            });
            if invalid {
                let publication = self.file_publications[slot].as_mut().unwrap();
                publication.replied = true;
                publication.failed = true;
            }
        }
        #[cfg(feature = "vfs_c1e2_test")]
        for publication in self.file_publications.iter_mut().flatten() {
            if !publication.replied
                && (publication.op == api::INVENTORY_STORAGE || api::mount_mutation(publication.op))
                && !storage_view_live(sched, publication.identity, publication.op)
            {
                publication.replied = true;
                publication.failed = true;
            }
        }
        self.wake_file_callers(sched);
    }

    #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
    fn namespace_mutation_live(&self, publication: &Publication, sched: &Scheduler) -> bool {
        let identity = publication.identity;
        self.registry.active() == Some(identity.registration)
            && publication.observation.as_ref().is_some_and(|(receipt, _, _)| {
                self.caller_receipt(identity) == Some(*receipt)
            })
            && sched.processes.iter().enumerate().any(|(idx, process)| {
                !process.is_exiting() && process_identity(process) == identity.caller
                    && process.uid == identity.uid && process.gid == identity.gid
                    && sched.shared_process_index(idx) == idx
            })
    }

    pub(super) fn reply_file(
        &mut self,
        registration: Registration,
        bytes: &[u8; api::FRAME_BYTES],
        sched: &mut Scheduler,
    ) -> Result<(), Error> {
        let reply = api::Reply::decode(bytes).map_err(|_| Error::Protocol)?;
        #[cfg(feature = "vfs_c1e10_test")]
        if reply.op == api::PUBLISH_STORAGE {
            crate::serial_println!("[VFS-C1E10] publication reply {} errno={}", reply.request_id, reply.errno);
        }
        if reply.registration != registration {
            return Err(Error::Identity);
        }
        let slot = self
            .file_publications
            .iter()
            .position(|p| {
                p.as_ref().is_some_and(|p| {
                    p.identity.registration == registration
                        && p.identity.caller == reply.caller
                        && p.id == reply.request_id
                        && p.op == reply.op
                        && !p.replied
                        && (!(api::namespace_mutation(p.op) || p.op == api::PUBLISH_STORAGE)
                            || p.dispatched)
                })
            })
            .ok_or(Error::Identity)?;
        #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
        if (api::namespace_mutation(reply.op) || reply.op == api::STAT_AT)
            && !self.namespace_mutation_live(self.file_publications[slot].as_ref().unwrap(), sched)
        {
            self.revalidate_storage_files(sched);
            return Err(Error::Identity);
        }
        #[cfg(feature = "vfs_c1e2_test")]
        if (reply.op == api::INVENTORY_STORAGE || api::mount_mutation(reply.op))
            && !storage_view_live(
                sched, self.file_publications[slot].as_ref().unwrap().identity, reply.op,
            )
        {
            self.revalidate_storage_files(sched);
            return Err(Error::Identity);
        }
        #[cfg(feature = "vfs_c1e1_test")]
        if reply.op == api::PREPARE_STORAGE {
            let publication = self.file_publications[slot].as_ref().unwrap();
            if !storage_caller_live(sched, reply.caller)
                || self.registry.active() != Some(registration)
                || !crate::vfs_n1j_test::storage_reply(
                    registration, reply.caller, reply.request_id, publication.token,
                    reply.result0, reply.errno, false,
                )
            {
                return Err(Error::Identity);
            }
        }
        #[cfg(feature = "vfs_c1e2_test")]
        if reply.op == api::PUBLISH_STORAGE {
            let publication = self.file_publications[slot].as_ref().unwrap();
            if !storage_caller_live(sched, reply.caller)
                || self.registry.active() != Some(registration)
                || (matches!(reply.errno, 0 | 95) && !crate::vfs_n1j_test::validate_storage_publication(
                    registration, reply.caller, publication.token,
                    (reply.errno == 0).then_some(reply.result1),
                ))
            {
                return Err(Error::Identity);
            }
        }
        #[cfg(all(not(test), debug_assertions, feature = "vfs_d1b_test"))]
        {
            let publication = self.file_publications[slot].as_ref().unwrap();
            if publication.compensation {
                use core::sync::atomic::{AtomicU64, Ordering};
                static REJECTED_ID: AtomicU64 = AtomicU64::new(0);
                if REJECTED_ID.swap(reply.request_id, Ordering::Relaxed) != reply.request_id {
                    crate::serial_println!(
                        "[VFS-D1B] compensation acknowledgement withheld id={} token={} epoch={} charge=1",
                        reply.request_id, publication.token, registration.epoch,
                    );
                    return Err(Error::Protocol);
                }
                crate::serial_println!(
                    "[VFS-D1B] compensation retry id={} token={} epoch={} errno={}",
                    reply.request_id, publication.token, registration.epoch, reply.errno,
                );
            }
        }
        if self.file_publications[slot].as_ref().unwrap().compensation
            && !matches!(reply.errno, 0 | 9)
        {
            return Err(Error::Protocol);
        }
        {
            let publication = self.file_publications[slot].as_mut().unwrap();
            publication.buffer.copy_from_slice(bytes);
            publication.replied = true;
        }
        let caller = self.file_publications[slot]
            .as_ref()
            .unwrap()
            .identity
            .caller;
        let kernel_origin = self.file_publications[slot]
            .as_ref()
            .is_some_and(|publication| matches!(publication.op, api::TRANSFER | api::CLOSE));
        if kernel_origin {
            // No caller owns this terminal result; the reply itself discharges
            // the transfer/close obligation and releases the publication slot.
            // A fire-and-forget close names a live caller only to authorize the
            // teardown, so it must not wake anyone: that caller may already be
            // parked on a later operation, and waking it here would resume the
            // wrong continuation before its own reply arrives.
            self.file_publications[slot] = None;
        } else if let Some(idx) = sched
            .processes
            .iter()
            .position(|p| process_identity(p) == caller && !p.is_exiting())
        {
            if sched.processes[idx].pending_call.is_none()
                && sched.processes[idx].ipc_reply_target.is_none()
                && sched.processes[idx].state == ProcessState::BlockedOnIpc
            {
                super::super::wake_terminal(sched, idx);
            }
        }
        Ok(())
    }

    /// Publish one kernel-origin ownership transfer. The old caller is already
    /// retired, so this intentionally does not require a live caller process and
    /// never parks one. The service rekeys the exact token before the
    /// `FileOwnerRetired` barrier is allowed to run.
    pub(super) fn publish_file_transfer(
        &mut self,
        registration: Registration,
        old: Caller,
        new: Caller,
        token: u64,
        now_tick: u64,
    ) -> Result<(), Error> {
        if self.registry.active() != Some(registration) {
            return Err(Error::NotReady);
        }
        if !(old.pid == new.pid && old.incarnation == new.incarnation
            || old.address_space_generation == new.address_space_generation)
            || old == new
        {
            return Err(Error::Identity);
        }
        let slot = self
            .file_publications
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let request_id = self.file_ids.allocate().ok_or(Error::Capacity)?;
        let payload = api::encode_caller(new);
        let request = api::Request {
            registration,
            caller: old,
            request_id,
            op: api::TRANSFER,
            token,
            arg0: 0,
            arg1: 0,
            length: payload.len(),
            namespace_id: 0,
            namespace_generation: 0,
            receipt_id: 0,
            receipt_generation: 0,
            uid: 0,
            gid: 0,
            payload: &payload,
        };
        let bytes = request.encode().map_err(|_| Error::Protocol)?;
        let mut buffer = alloc::vec::Vec::new();
        buffer
            .try_reserve_exact(api::FRAME_BYTES)
            .map_err(|_| Error::Capacity)?;
        buffer.extend_from_slice(&bytes);
        self.file_publications[slot] = Some(Publication {
            id: request_id,
            identity: AdmissionIdentity {
                caller: old,
                uid: 0,
                gid: 0,
                groups: [],
                registration,
            },
            op: api::TRANSFER,
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            observation: None,
            dispatched: false,
            replied: false,
            failed: false,
            woken: false,
            started_tick: now_tick,
            token,
            compensation: false,
            executable: false,
            buffer,
        });
        Ok(())
    }

    pub(super) fn take_file(&mut self, caller: Caller, op: u64) -> Option<FileCompletion> {
        let slot = self.file_publications.iter().position(|p| {
            p.as_ref()
                .is_some_and(|p| p.identity.caller == caller && p.op == op && p.replied)
        })?;
        let publication = self.file_publications[slot].take().unwrap();
        if publication.failed {
            return Some(FileCompletion {
                op,
                errno: 5,
                result0: 0,
                result1: 0,
                registration_epoch: publication.identity.registration.epoch,
                payload: alloc::vec::Vec::new(),
            });
        }
        let reply = api::Reply::decode(&publication.buffer).ok()?;
        Some(FileCompletion {
            op,
            errno: reply.errno,
            result0: reply.result0,
            result1: reply.result1,
            registration_epoch: publication.identity.registration.epoch,
            payload: reply.payload.to_vec(),
        })
    }

    pub(super) fn finish_open(
        &mut self,
        slot: usize,
        install: impl FnOnce(u64, u64) -> Result<u64, u64>,
    ) -> Result<FileCompletion, Error> {
        let publication = self.file_publications[slot].as_mut().unwrap();
        let caller = publication.identity.caller;
        if publication.failed {
            return self.take_file(caller, api::OPEN).ok_or(Error::Protocol);
        }
        let reply = api::Reply::decode(&publication.buffer).map_err(|_| Error::Protocol)?;
        if reply.errno != 0 {
            return self.take_file(caller, api::OPEN).ok_or(Error::Protocol);
        }
        let token = reply.result0;
        let size = reply.result1;
        let registration = publication.identity.registration;
        let result = if !publication.executable && size != 0 {
            Err(5)
        } else {
            install(token, registration.epoch)
        };
        let (errno, result0) = match result {
            Ok(value) if !publication.executable => {
                self.file_publications[slot] = None;
                (0, value)
            }
            result => {
                #[cfg(all(not(test), debug_assertions, feature = "vfs_d1b_test"))]
                if let Err(errno) = result {
                    crate::serial_println!(
                        "[VFS-D1B] failed install pid={} incarnation={} image={} epoch={} id={} token={} errno={}",
                        caller.pid, caller.incarnation, caller.address_space_generation,
                        registration.epoch, publication.id, token, errno,
                    );
                }
                // Reuse the authenticated OPEN slot and frame allocation for
                // CLOSE, including when the rest of the channel is full.
                let bytes = Self::file_close_frame(registration, caller, token, publication.id)
                    .expect("validated OPEN reply supplies a valid close identity");
                publication.buffer.copy_from_slice(&bytes);
                publication.op = api::CLOSE;
                publication.token = token;
                publication.compensation = true;
                publication.executable = result.is_ok();
                publication.replied = result.is_ok(); // Held until the executable reader settles.
                publication.woken = true;
                match result {
                    Ok(value) => (0, value),
                    Err(errno) => (errno, 0),
                }
            }
        };
        Ok(FileCompletion {
            op: api::OPEN,
            errno,
            result0,
            result1: if errno == 0 { size } else { 0 },
            registration_epoch: registration.epoch,
            payload: alloc::vec::Vec::new(),
        })
    }

    pub(super) fn retire_file_publications(&mut self, expected: Caller) {
        for slot in 0..self.file_publications.len() {
            let Some(publication) = self.file_publications[slot]
                .as_ref()
                .filter(|publication| publication.identity.caller == expected)
            else {
                continue;
            };
            if publication.compensation {
                if publication.executable {
                    let publication = self.file_publications[slot].as_mut().unwrap();
                    publication.executable = false;
                    publication.replied = false;
                }
                continue;
            }
            #[cfg(feature = "vfs_c1e1_test")]
            publication.cancel_storage();
            if publication.op == api::OPEN && publication.replied && !publication.failed {
                let _ = self.finish_open(slot, |_, _| Err(125));
            } else {
                self.file_publications[slot] = None;
            }
        }
    }

    pub(super) fn retire_file_registration(&mut self, expected: Registration) {
        for slot in &mut self.file_publications {
            if let Some(publication) = slot
                .as_mut()
                .filter(|p| p.identity.registration == expected)
            {
                if matches!(publication.op, api::CLOSE | api::TRANSFER) {
                    *slot = None;
                } else {
                    publication.failed = true;
                    publication.replied = true;
                    publication.buffer.clear();
                }
            }
        }
    }

    pub(super) fn wake_file_callers(&mut self, sched: &mut Scheduler) {
        for publication in self.file_publications.iter_mut().flatten() {
            if publication.woken || !publication.replied {
                continue;
            }
            if let Some(idx) = sched
                .processes
                .iter()
                .position(|p| process_identity(p) == publication.identity.caller && !p.is_exiting())
            {
                if sched.processes[idx].pending_call.is_some()
                    || sched.processes[idx].ipc_reply_target.is_some()
                {
                    continue;
                }
                if sched.processes[idx].state == ProcessState::BlockedOnIpc {
                    super::super::wake_terminal(sched, idx);
                }
            }
            publication.woken = true;
        }
    }
}

/// Kernel-origin entry only. The caller must yield through the scheduler and
/// retain its exact operation until terminal delivery or cleanup.
pub(crate) fn file_call(
    pid: usize,
    operation: FileOperation<'_>,
    descriptor: Option<(Caller, u64)>,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
    install: impl FnOnce(&mut Scheduler, u64, u64) -> Result<u64, u64>,
) -> Result<FileCall, Error> {
    let routed = sched
        .processes
        .iter()
        .any(|p| p.pid == pid && !p.is_exiting() && p.vfs_file_routed);
    let storage = (cfg!(feature = "vfs_c1e1_test") && operation.op() == api::PREPARE_STORAGE)
        || (cfg!(feature = "vfs_c1e2_test")
            && matches!(operation.op(), api::ABORT_STORAGE | api::PUBLISH_STORAGE
                | api::UNMOUNT_STORAGE | api::INVENTORY_STORAGE));
    let candidate_path = cfg!(feature = "vfs_c2e_candidate")
        && (operation.op() == api::OPEN || api::retained_path(operation.op()) || descriptor.is_some());
    if !routed && !storage && !candidate_path {
        return Err(Error::Identity);
    }
    if sched
        .processes
        .iter()
        .find(|p| p.pid == pid)
        .is_some_and(|p| p.pending_call.is_some() || p.ipc_reply_target.is_some())
    {
        return Err(Error::Busy);
    }
    let op = operation.op();
    let idx = sched
        .processes
        .iter()
        .position(|p| p.pid == pid && !p.is_exiting())
        .ok_or(Error::Identity)?;
    let caller = process_identity(&sched.processes[idx]);
    #[cfg(feature = "vfs_c1e1_test")]
    if storage && op != api::INVENTORY_STORAGE && !storage_caller_live(sched, caller) {
        return Err(Error::Identity);
    }
    {
        let mut runtime = RUNTIME.lock();
        live_registration(&mut runtime, sched, caps);
        runtime.expire_files(sched.global_tick);
        if let Some((owner, _)) = descriptor {
            if owner != caller {
                return Err(Error::Identity);
            }
        }
        if let Some(slot) = runtime.caller_publication(caller) {
            let publication = runtime.file_publications[slot].as_ref().unwrap();
            if publication.op != op
                || publication.token != operation.token()
                || descriptor.is_some_and(|(_, epoch)| epoch != publication.identity.registration.epoch)
                || (op != api::OPEN && !storage && !api::retained_path(op)
                    && descriptor != Some((caller, publication.identity.registration.epoch)))
            {
                return Err(Error::Busy);
            }
            #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
            if let Some((path, flags)) = match &operation {
                FileOperation::ObserveMount { path } => Some((path.unwrap_or(""), 0)),
                FileOperation::MutateNamespace { paths, flags, .. } => Some((*paths, *flags)),
                #[cfg(feature = "vfs_c2e_candidate")]
                FileOperation::StatAt { path, .. } => Some((*path, 0)),
                _ => None,
            } {
                let valid = publication.observation.as_ref().is_some_and(|(receipt, original, original_flags)| {
                    original == path.as_bytes()
                        && *original_flags == flags
                        && runtime.registry.active() == Some(publication.identity.registration)
                        && runtime.caller_receipt(publication.identity) == Some(*receipt)
                        && sched.processes[idx].uid == publication.identity.uid
                        && sched.processes[idx].gid == publication.identity.gid
                        && sched.shared_process_index(idx) == idx
                });
                if !valid {
                    runtime.file_publications[slot] = None;
                    return Err(Error::Identity);
                }
            }
            if op == api::OPEN && publication.replied {
                return runtime
                    .finish_open(slot, |token, epoch| install(sched, token, epoch))
                    .map(FileCall::Complete);
            }
            #[cfg(feature = "vfs_c1e1_test")]
            if op == api::PREPARE_STORAGE && publication.replied && !publication.failed {
                let valid = runtime.registry.active() == Some(publication.identity.registration)
                    && api::Reply::decode(&publication.buffer).is_ok_and(|reply| {
                        crate::vfs_n1j_test::storage_reply(
                            reply.registration, caller, reply.request_id, publication.token,
                            reply.result0, reply.errno, true,
                        )
                    });
                if !valid {
                    publication.cancel_storage();
                    runtime.file_publications[slot].as_mut().unwrap().failed = true;
                }
            }
            if let Some(completion) = runtime.take_file(caller, op) {
                return Ok(FileCall::Complete(completion));
            }
            runtime.wake_file_callers(sched);
            return Ok(FileCall::Pending);
        }
    }
    let identity = capture(pid, sched, caps).map_err(|_| Error::Identity)?;
    if api::retained_path(op) && sched.shared_process_index(idx) != idx {
        return Err(Error::Identity);
    }
    #[cfg(feature = "vfs_c1e2_test")]
    if (op == api::INVENTORY_STORAGE || api::mount_mutation(op))
        && !storage_view_live(sched, identity, op)
    {
        return Err(Error::Identity);
    }
    let mut runtime = RUNTIME.lock();
    if descriptor.is_some() || (op != api::OPEN && !storage && !api::retained_path(op)) {
        runtime.check_descriptor(identity, descriptor)?;
    }
    let receipt = match &operation {
        #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
        FileOperation::ObserveMount { .. } | FileOperation::MutateNamespace { .. } => {
            Some(runtime.caller_receipt(identity).ok_or(Error::NotReady)?)
        }
        #[cfg(feature = "vfs_c2e_candidate")]
        FileOperation::StatAt { .. } => {
            Some(runtime.caller_receipt(identity).ok_or(Error::NotReady)?)
        }
        FileOperation::Open { .. } => {
            Some(runtime.caller_receipt(identity).ok_or(Error::NotReady)?)
        }
        _ => None,
    };
    if (op == api::OPEN || op == api::STAT_AT || api::namespace_mutation(op))
        && operation.token() != 0 && descriptor.is_none()
    {
        return Err(Error::Identity);
    }
    runtime.publish_file(identity, operation, receipt, sched)?;
    Ok(FileCall::Pending)
}

#[cfg(feature = "vfs_c1e1_test")]
impl Publication {
    fn cancel_storage(&self) {
        if self.op == api::PREPARE_STORAGE {
            crate::vfs_n1j_test::cancel_storage(self.identity.registration, self.identity.caller, self.id);
        }
    }
}

#[cfg(feature = "vfs_c1e1_test")]
fn storage_caller_live(sched: &Scheduler, caller: Caller) -> bool {
    sched.processes.iter().any(|process| {
        !process.is_exiting() && process_identity(process) == caller
            && sunlight_ipc::storage::authorized(process.uid, process.service_lookup_restrictions)
    })
}

#[cfg(feature = "vfs_c1e2_test")]
fn storage_view_live(sched: &Scheduler, identity: AdmissionIdentity, op: u64) -> bool {
    sched.processes.iter().enumerate().any(|(index, process)| {
        !process.is_exiting() && !process.vfs_root_rebased
            && process_identity(process) == identity.caller
            && !sched.processes[sched.shared_process_index(index)].vfs_root_rebased
            && process.uid == identity.uid && process.gid == identity.gid
            && (!api::mount_mutation(op)
                || sunlight_ipc::storage::authorized(process.uid, process.service_lookup_restrictions))
    })
}

#[cfg(feature = "vfs_c1e1_test")]
pub(crate) fn acquire_storage(
    service: Caller,
    epoch: u64,
    request: u64,
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Option<u64> {
    let mut runtime = RUNTIME.lock();
    let registration = live_registration(&mut runtime, sched, caps)?;
    if registration.epoch != epoch || registration.owner.pid != service.pid
        || registration.owner.incarnation != service.incarnation
        || registration.owner.address_space_generation != service.address_space_generation
    {
        return None;
    }
    let publication = runtime.file_publications.iter().flatten().find(|p| {
        p.id == request && p.op == api::PREPARE_STORAGE && !p.replied
            && p.identity.registration == registration
            && storage_caller_live(sched, p.identity.caller)
    })?;
    crate::vfs_n1j_test::acquire_storage(
        registration, publication.identity.caller, request, publication.token,
    ).ok()
}

#[cfg(feature = "vfs_c1e2_test")]
pub(crate) fn retire_storage_fixture_caller(
    request: u64,
    epoch: u64,
    sched: &mut Scheduler,
) -> bool {
    let service = process_identity(sched.current_process());
    let caller = {
        let caps = crate::capability::CAP_BROKER.lock();
        let mut runtime = RUNTIME.lock();
        let Some(registration) = live_registration(&mut runtime, sched, &caps) else {
            return false;
        };
        if registration.epoch != epoch || registration.owner.pid != service.pid
            || registration.owner.incarnation != service.incarnation
            || registration.owner.address_space_generation != service.address_space_generation
        {
            return false;
        }
        runtime.file_publications.iter().flatten().find(|entry| {
            entry.id == request && entry.op == api::PREPARE_STORAGE && !entry.replied
                && !entry.failed && entry.identity.registration == registration
                && entry.identity.caller.pid == sunlight_ipc::storage::FIXTURE_RETIRING_PID
        }).map(|entry| entry.identity.caller)
    };
    let Some(caller) = caller else { return false };
    let pid = caller.pid as usize;
    if !sched.process_index_by_pid(pid).is_some_and(|index| {
        process_identity(&sched.processes[index]) == caller
            && sched.processes[index].state != ProcessState::Running
    }) {
        return false;
    }
    sched.terminate_process_by_pid(pid, 125, "StorageCtl parked fixture")
}

/// Drop the exact cancelled mapping read without touching ordinary Close.
#[cfg(all(not(test), debug_assertions, feature = "vfs_service_private_mmap"))]
pub(crate) fn cancel_mapping_read(caller: Caller, epoch: u64, token: u64) {
    RUNTIME.lock().cancel_mapping_read(caller, epoch, token);
}

/// Expire and wake pending file operations at timer or syscall-return points.
pub(crate) fn wake_file_callers(sched: &mut Scheduler, now_tick: u64) {
    let mut runtime = RUNTIME.lock();
    runtime.expire_files(now_tick);
    runtime.wake_file_callers(sched);
}

#[cfg(test)]
#[path = "file_tests.rs"]
mod tests;
