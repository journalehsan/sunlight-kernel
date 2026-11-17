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
    buffer: alloc::vec::Vec<u8>,
}

/// One caller-owned file request. Borrowed paths and payloads stay valid only
/// for the duration of the syscall that admits them.
pub(crate) enum FileOperation<'a> {
    Open {
        path: &'a str,
        access: u64,
    },
    Read {
        token: u64,
        length: usize,
    },
    Write {
        token: u64,
        payload: &'a [u8],
    },
    Seek {
        token: u64,
        whence: u64,
        offset: i64,
    },
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
    fn op(&self) -> u64 {
        match self {
            Self::Open { .. } => api::OPEN,
            Self::Read { .. } => api::READ,
            Self::Write { .. } => api::WRITE,
            Self::Seek { .. } => api::SEEK,
            Self::Stat { .. } => api::STAT,
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
            if !publication.replied
                && now_tick.wrapping_sub(publication.started_tick) >= TIMEOUT_TICKS
            {
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
            if slot
                .as_ref()
                .is_some_and(|publication| {
                    matches!(publication.op, api::TRANSFER | api::CLOSE) && publication.replied
                })
            {
                *slot = None;
            }
        }
    }

    fn caller_publication(&self, caller: Caller) -> Option<usize> {
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

    /// Lexicographically latest retained metadata lease for this caller. The
    /// service still proves the receipt names a live binding before publishing.
    fn caller_receipt(&self, identity: AdmissionIdentity) -> Option<Receipt> {
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

    fn publish_file(
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
            FileOperation::Open { path, access } => {
                let receipt = receipt.ok_or(Error::NotReady)?;
                (
                    api::OPEN,
                    0,
                    *access,
                    0,
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
            FileOperation::Write { token, payload } => (api::WRITE, *token, 0, 0, *payload, None),
            FileOperation::Seek {
                token,
                whence,
                offset,
            } => (api::SEEK, *token, *whence, *offset as u64, &[][..], None),
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
            FileOperation::Open { .. } => (identity.uid, identity.gid),
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
        self.file_publications[slot] = Some(Publication {
            id: request_id,
            identity,
            op,
            replied: false,
            failed: false,
            woken: false,
            started_tick: sched.global_tick,
            buffer,
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
        let request = api::Request {
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
                caller,
                uid: 0,
                gid: 0,
                groups: [],
                registration,
            },
            op: api::CLOSE,
            replied: false,
            failed: false,
            // No caller is parked for this terminal result.
            woken: true,
            started_tick: now_tick,
            buffer,
        });
        Ok(())
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
            .filter(|p| p.identity.registration == registration && !p.replied)
            .min_by_key(|p| p.id)
        else {
            return Ok(None);
        };
        if publication.buffer.len() != api::FRAME_BYTES {
            return Err(Error::Protocol);
        }
        output.copy_from_slice(&publication.buffer);
        Ok(Some(publication.id))
    }

    pub(super) fn reply_file(
        &mut self,
        registration: Registration,
        bytes: &[u8; api::FRAME_BYTES],
        sched: &mut Scheduler,
    ) -> Result<(), Error> {
        let reply = api::Reply::decode(bytes).map_err(|_| Error::Protocol)?;
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
                })
            })
            .ok_or(Error::Identity)?;
        {
            let publication = self.file_publications[slot].as_mut().unwrap();
            publication.buffer.clear();
            publication
                .buffer
                .try_reserve_exact(api::FRAME_BYTES)
                .map_err(|_| Error::Capacity)?;
            publication.buffer.extend_from_slice(bytes);
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
            replied: false,
            failed: false,
            woken: false,
            started_tick: now_tick,
            buffer,
        });
        Ok(())
    }

    fn take_file(&mut self, caller: Caller, op: u64) -> Option<FileCompletion> {
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

    pub(super) fn retire_file_publications(&mut self, expected: Caller) {
        for slot in &mut self.file_publications {
            if slot.as_ref().is_some_and(|p| p.identity.caller == expected) {
                *slot = None;
            }
        }
    }

    pub(super) fn retire_file_registration(&mut self, expected: Registration) {
        for slot in &mut self.file_publications {
            if slot
                .as_ref()
                .is_some_and(|p| p.identity.registration == expected)
            {
                *slot = None;
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
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<FileCall, Error> {
    let routed = sched
        .processes
        .iter()
        .any(|p| p.pid == pid && !p.is_exiting() && p.vfs_file_routed);
    if !routed {
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
    {
        let mut runtime = RUNTIME.lock();
        runtime.expire_files(sched.global_tick);
        if let Some(slot) = runtime.caller_publication(caller) {
            if runtime.file_publications[slot].as_ref().unwrap().op != op {
                return Err(Error::Busy);
            }
            if let Some(completion) = runtime.take_file(caller, op) {
                return Ok(FileCall::Complete(completion));
            }
            runtime.wake_file_callers(sched);
            return Ok(FileCall::Pending);
        }
    }
    let identity = capture(pid, sched, caps).map_err(|_| Error::Identity)?;
    let mut runtime = RUNTIME.lock();
    let receipt = match &operation {
        FileOperation::Open { .. } => {
            Some(runtime.caller_receipt(identity).ok_or(Error::NotReady)?)
        }
        _ => None,
    };
    runtime.publish_file(identity, operation, receipt, sched)?;
    Ok(FileCall::Pending)
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
