//! Kernel-private copied stat pilot. Scheduler -> broker -> runtime serialization
//! covers admission, publication, parking and completion. The syscall exposes
//! metadata within the explicitly provisioned root/cwd profile.
use super::*;
use crate::process::ProcessState;
use sunlight_ipc::vfs_rpc::{
    lifecycle::{Failure, State},
    response::ReplyMessage,
};

// Admission starts the budget; delivery and caller retries never extend it.
const TIMEOUT_TICKS: u64 = 5 * crate::timekeeping::TICK_HZ;

pub(super) struct Publication {
    pub(super) id: u64,
    pub(super) identity: AdmissionIdentity,
    delivered: bool,
    woken: bool,
    started_tick: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Identity,
    Busy,
    Capacity,
    Prepare,
    Protocol,
}

impl Runtime {
    fn expire_stats(&mut self, now_tick: u64) {
        for publication in self.publications.iter().flatten() {
            if now_tick.wrapping_sub(publication.started_tick) >= TIMEOUT_TICKS {
                // The common terminal transition preserves a reply/retirement
                // that won serialization first. Resources stay caller-owned.
                let _ = self.pending.fail(publication.id, Failure::TimedOut);
            }
        }
    }

    /// Exact caller-owned result take. Copying to user memory must occur only
    /// after the syscall continuation validates its retained address space.
    pub(super) fn reap_stat(&mut self, caller: Caller, id: u64) -> Option<Retired<Resources>> {
        let slot = self.publications.iter().position(|p| {
            p.as_ref()
                .is_some_and(|p| p.id == id && p.identity.caller == caller && p.woken)
        })?;
        let retired = self.pending.reap(id)?;
        self.publications[slot] = None;
        Some(retired)
    }

    pub(super) fn stat_ready(&self, registration: Registration) -> bool {
        self.publications.iter().flatten().any(|p| {
            p.identity.registration == registration
                && !p.delivered
                && self.pending.state(p.id) == Some(State::Waiting)
        })
    }

    pub(super) fn publish_stat(
        &mut self,
        identity: AdmissionIdentity,
        path: &str,
        sched: &mut Scheduler,
    ) -> Result<Transfer, Error> {
        let idx = sched
            .processes
            .iter()
            .position(|p| {
                process_identity(p) == identity.caller
                    && !p.is_exiting()
                    && p.uid == identity.uid
                    && p.gid == identity.gid
            })
            .ok_or(Error::Identity)?;
        let process = &sched.processes[idx];
        if process.state != ProcessState::Running
            || process.pending_call.is_some()
            || self
                .publications
                .iter()
                .flatten()
                .any(|p| p.identity.caller == identity.caller)
        {
            return Err(Error::Busy);
        }
        let slot = self
            .publications
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let transfer = self
            .admit_stat(identity, path)
            .map_err(|_| Error::Prepare)?;
        // All fallible work precedes these transitions. Completion and teardown
        // cannot observe a published request with an unparked caller.
        self.publications[slot] = Some(Publication {
            id: transfer.words()[1],
            identity,
            delivered: false,
            woken: false,
            started_tick: sched.global_tick,
        });
        sched.processes[idx].state = ProcessState::BlockedOnIpc;
        Ok(transfer)
    }

    /// Copies into caller-provided bounded backing; performs no allocation.
    /// Single delivery: a failed later user copy requires service retirement,
    /// never retransmission of an operation whose execution is uncertain.
    pub(super) fn take_stat(
        &mut self,
        registration: Registration,
        output: &mut [u8],
    ) -> Result<Option<(u64, usize)>, Error> {
        if self.registry.active() != Some(registration) {
            return Err(Error::Identity);
        }
        let Some(publication) = self
            .publications
            .iter_mut()
            .flatten()
            .filter(|p| p.identity.registration == registration && !p.delivered)
            .filter(|p| self.pending.state(p.id) == Some(State::Waiting))
            .min_by_key(|p| p.id)
        else {
            return Ok(None);
        };
        let resources = self
            .pending
            .resources(publication.id)
            .ok_or(Error::Protocol)?;
        resources
            .revalidate_namespace()
            .map_err(|_| Error::Identity)?;
        let input = resources.input();
        if output.len() < input.len() {
            return Err(Error::Capacity);
        }
        output[..input.len()].copy_from_slice(input);
        publication.delivered = true;
        Ok(Some((publication.id, input.len())))
    }

    pub(super) fn complete_stat(
        &mut self,
        registration: Registration,
        reply: ReplyMessage<'_>,
    ) -> Result<(), Error> {
        if self.registry.active() != Some(registration) {
            return Err(Error::Identity);
        }
        let publication = self
            .publications
            .iter()
            .flatten()
            .find(|p| p.id == reply.words[1])
            .ok_or(Error::Identity)?;
        if publication.identity.registration != registration
            || !publication.delivered
            || self.pending.state(publication.id) != Some(State::Waiting)
        {
            return Err(Error::Identity);
        }
        let id = publication.id;
        // Validation terminalizes malformed replies from the bound service.
        // Only validated initialized bytes reach retained output backing.
        self.pending
            .complete(
                registration.service(),
                ReplyMessage {
                    label: reply.label,
                    words: reply.words,
                    word_count: reply.word_count,
                    cap_count: reply.cap_count,
                    output: reply.output,
                },
            )
            .map_err(|_| Error::Protocol)?;
        let output = self
            .pending
            .resources_mut(id)
            .ok_or(Error::Protocol)?
            .output();
        output.fill(0);
        output[..reply.output.len()].copy_from_slice(reply.output);
        Ok(())
    }

    pub(super) fn wake_stat_callers(&mut self, sched: &mut Scheduler) {
        for publication in self.publications.iter_mut().flatten() {
            if publication.woken || self.pending.state(publication.id) == Some(State::Waiting) {
                continue;
            }
            if let Some(idx) = sched
                .processes
                .iter()
                .position(|p| process_identity(p) == publication.identity.caller && !p.is_exiting())
            {
                // An outbound call owns its own blocked continuation. An
                // inbound reply target only records a server's obligation to
                // its client; stat may run while servicing that request and
                // must leave the reply target intact.
                if sched.processes[idx].pending_call.is_some() {
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

/// Kernel-origin entry only. The caller must subsequently yield through the
/// scheduler and retain its request ID until terminal delivery/cleanup.
#[allow(dead_code)] // General pathname adapter remains private.
pub(super) fn publish_stat(
    identity: AdmissionIdentity,
    path: &str,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<Transfer, Error> {
    revalidate(identity, sched, caps).map_err(|_| Error::Identity)?;
    RUNTIME.lock().publish_stat(identity, path, sched)
}

/// Service PID comes from syscall entry, never from a message payload. Only
/// exact-size copied syscall envelopes may reach these entrypoints.
pub(crate) fn take_stat(
    pid: usize,
    epoch: u64,
    output: &mut [u8],
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<Option<(u64, usize)>, Error> {
    let mut runtime = RUNTIME.lock();
    runtime.expire_stats(sched.global_tick);
    let registration = super::transport::authenticate(&mut runtime, pid, epoch, sched, caps)
        .map_err(|_| Error::Identity)?;
    runtime.take_stat(registration, output)
}

pub(crate) fn complete_stat(
    pid: usize,
    epoch: u64,
    reply: ReplyMessage<'_>,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<(), Error> {
    let mut runtime = RUNTIME.lock();
    runtime.expire_stats(sched.global_tick);
    let registration = super::transport::authenticate(&mut runtime, pid, epoch, sched, caps)
        .map_err(|_| Error::Identity)?;
    let errno = reply.words[2];
    let output_bytes = reply.words[3];
    let result = runtime.complete_stat(registration, reply);
    #[cfg(not(test))]
    crate::serial_println!(
        "[VFS-RPC] root stat reply validated={} errno={} bytes={}",
        result.is_ok(),
        errno,
        output_bytes
    );
    #[cfg(test)]
    let _ = (errno, output_bytes);
    runtime.wake_stat_callers(sched);
    result
}

pub(super) fn wake_stat_callers(sched: &mut Scheduler, now_tick: u64) {
    #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_r1_test"))]
    super::spawn::fixture::retire_checkpoint_service(sched);
    let mut runtime = RUNTIME.lock();
    runtime.expire_stats(now_tick);
    runtime.wake_stat_callers(sched);
    runtime.poll_chdirs(sched, now_tick);
    runtime.poll_spawns(sched, now_tick);
    #[cfg(feature = "vfs_c2e_candidate")]
    runtime.poll_shared_cwd_joins(sched, now_tick);
}

#[cfg(test)]
#[path = "stat_tests.rs"]
mod tests;

/// Own completion backing beyond scheduler/broker/runtime guards. Dropping this
/// value releases quota and namespace pins only after those guards are gone.
pub(crate) struct StatCompletion(Retired<Resources>);
impl StatCompletion {
    pub(crate) fn caller(&self) -> Caller {
        self.0.resources.caller()
    }
    pub(crate) fn native_bytes(&mut self) -> Result<[u8; 24], u64> {
        native_record(&self.bytes().map_err(native_error)?)
    }
    pub(crate) fn linux_bytes(&mut self) -> Result<[u8; 144], u64> {
        linux_record(&self.bytes()?)
    }
    pub(crate) fn bytes(
        &mut self,
    ) -> Result<[u8; sunlight_ipc::vfs_rpc::response::METADATA_BYTES], u64> {
        use sunlight_ipc::vfs_rpc::{lifecycle::Failure, stat_transport as api};
        match self.0.state {
            State::Completed(completion) if completion.errno == 0 => {
                let mut bytes = [0; sunlight_ipc::vfs_rpc::response::METADATA_BYTES];
                bytes.copy_from_slice(self.0.resources.output());
                Ok(bytes)
            }
            State::Completed(completion) => Err(0u64.wrapping_sub(u64::from(completion.errno))),
            State::Failed(Failure::CallerRetired) => Err((-125i64) as u64),
            State::Failed(Failure::TimedOut) => {
                Err(sunlight_ipc::vfs_rpc::stat_transport::TIMED_OUT)
            }
            _ => Err(api::IO_ERROR),
        }
    }
}
pub(crate) enum StatCall {
    Pending,
    Complete(StatCompletion),
}

/// The syscall selects the current caller; its path supplies no identity or policy.
/// Pending calls are consumed before looking up the service, so service loss
/// cannot hide a caller-owned terminal result. Scheduler -> broker are held.
pub(crate) fn stat_call(
    pid: usize,
    path: &str,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<StatCall, Error> {
    let idx = sched
        .processes
        .iter()
        .position(|p| p.pid == pid && !p.is_exiting())
        .ok_or(Error::Identity)?;
    #[cfg(feature = "vfs_c2e_candidate")]
    if !RUNTIME.lock().join_shared_cwd(idx, sched, caps).map_err(|_| Error::Identity)? {
        return Ok(StatCall::Pending);
    }
    if !sched.processes[idx].vfs_root_metadata {
        return Err(Error::Identity);
    }
    if sched.processes[idx].pending_call.is_some() {
        return Err(Error::Busy);
    }
    let caller = process_identity(&sched.processes[idx]);
    if RUNTIME.lock().chdir_busy(idx, sched) {
        return Err(Error::Busy);
    }
    {
        let mut runtime = RUNTIME.lock();
        runtime.expire_stats(sched.global_tick);
        runtime.wake_stat_callers(sched);
        if let Some(id) = runtime
            .publications
            .iter()
            .flatten()
            .find(|p| p.identity.caller == caller)
            .map(|p| p.id)
        {
            if !runtime
                .pending
                .resources(id)
                .is_some_and(|r| r.matches_stat_path(path))
            {
                return Err(Error::Busy);
            }
            if let Some(retired) = runtime.reap_stat(caller, id) {
                return Ok(StatCall::Complete(StatCompletion(retired)));
            }
            // Signals may wake a waiting caller. A retry parks the same owned
            // request rather than allocating a second operation.
            sched.processes[idx].state = ProcessState::BlockedOnIpc;
            return Ok(StatCall::Pending);
        }
    }
    let identity = capture(pid, sched, caps).map_err(|_| Error::Identity)?;
    let mut runtime = RUNTIME.lock();
    let transfer = runtime.publish_stat(identity, path, sched)?;
    #[cfg(not(test))]
    crate::serial_println!(
        "[VFS-RPC] root stat parked caller={} request={} epoch={}",
        caller.pid,
        transfer.words()[1],
        identity.registration.epoch
    );
    #[cfg(test)]
    let _ = transfer;
    Ok(StatCall::Pending)
}

/// Pull consumed delivery before a user copy failed: terminate the epoch rather
/// than retrying an operation whose remote execution is uncertain.
pub(crate) fn copy_failed(pid: usize, epoch: u64, sched: &mut Scheduler, caps: &CapabilityBroker) {
    let mut runtime = RUNTIME.lock();
    if let Ok(registration) = super::transport::authenticate(&mut runtime, pid, epoch, sched, caps)
    {
        runtime.retire_registration(registration);
        runtime.wake_stat_callers(sched);
    }
}

// Native libc uses historical sentinels rather than negated POSIX errno.
fn native_error(error: u64) -> u64 {
    match 0u64.wrapping_sub(error) {
        1 | 13 => u64::MAX - 3,
        2 => u64::MAX - 2,
        9 => u64::MAX - 4,
        11 => u64::MAX - 1,
        12 => u64::MAX - 12,
        20 => u64::MAX - 7,
        21 => u64::MAX - 6,
        22 | 36 => u64::MAX - 5,
        28 => u64::MAX - 11,
        _ => u64::MAX - 8, // Includes service loss, timeout and cancellation.
    }
}

fn native_record(bytes: &[u8]) -> Result<[u8; 24], u64> {
    use sunlight_ipc::vfs_rpc::{
        codec::Request,
        response::{Kind, Reply, ReplyContract},
        Completion,
    };
    let Ok(Reply::Metadata(metadata)) = ReplyContract::for_request(Request::Stat { path: "/" })
        .unwrap()
        .validate(
            Completion {
                errno: 0,
                output_bytes: bytes.len() as u64,
            },
            bytes,
        )
    else {
        return Err(u64::MAX - 8);
    };
    let mut record = [0; 24];
    record[..8].copy_from_slice(&metadata.size.to_le_bytes());
    record[8..12].copy_from_slice(&metadata.uid.to_le_bytes());
    record[12..16].copy_from_slice(&metadata.gid.to_le_bytes());
    let kind = match metadata.kind {
        Kind::File => 0o100000,
        Kind::Directory => 0o040000,
    };
    record[16..18].copy_from_slice(&(metadata.mode | kind).to_le_bytes());
    record[18] = metadata.kind as u8;
    record[20..24].copy_from_slice(&metadata.links.to_le_bytes());
    Ok(record)
}

// Linux x86_64 struct stat. Like the existing compatibility ABI, timestamps
// are zero and block counts describe the logical size, not allocated storage.
fn linux_record(bytes: &[u8]) -> Result<[u8; 144], u64> {
    use sunlight_ipc::vfs_rpc::{
        codec::Request,
        response::{Kind, Reply, ReplyContract},
        Completion,
    };
    let Ok(Reply::Metadata(metadata)) = ReplyContract::for_request(Request::Stat { path: "/" })
        .unwrap()
        .validate(
            Completion {
                errno: 0,
                output_bytes: bytes.len() as u64,
            },
            bytes,
        )
    else {
        return Err((-5i64) as u64);
    };
    if metadata.size > i64::MAX as u64 {
        return Err((-75i64) as u64); // EOVERFLOW: st_size is signed.
    }
    let mut record = [0; 144];
    record[..8].copy_from_slice(&metadata.provenance.mount_id.to_le_bytes());
    record[8..16].copy_from_slice(&metadata.provenance.object_id.to_le_bytes());
    record[16..24].copy_from_slice(&u64::from(metadata.links).to_le_bytes());
    let kind = match metadata.kind {
        Kind::File => 0o100000,
        Kind::Directory => 0o040000,
    };
    record[24..28].copy_from_slice(&(u32::from(metadata.mode) | kind).to_le_bytes());
    record[28..32].copy_from_slice(&metadata.uid.to_le_bytes());
    record[32..36].copy_from_slice(&metadata.gid.to_le_bytes());
    record[48..56].copy_from_slice(&metadata.size.to_le_bytes());
    record[56..64].copy_from_slice(&4096u64.to_le_bytes());
    record[64..72].copy_from_slice(&metadata.size.div_ceil(512).to_le_bytes());
    Ok(record)
}
