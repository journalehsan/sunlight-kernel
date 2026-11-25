//! One waiter per parent, one request per private child. Registration owns
//! the remote lease; the scheduler owns the child and its PID claim.
use super::*;
use crate::process::{
    pending_state::{Identity, Parent, RemoteClaim},
    Process, ProcessState,
};
use sunlight_ipc::vfs_rpc::retention::wire::Refusal;

const WAIT_TICKS: u64 = 5 * crate::timekeeping::TICK_HZ;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LaunchAttribution {
    pub(super) parent: Parent,
    session_broker: u64,
    session: u64,
    session_generation: u64,
    pub(super) execution: u64,
    stat_routed: bool,
    root_rebased: bool,
}

impl LaunchAttribution {
    fn capture(process: &Process) -> Option<Self> {
        let foreground = &process.foreground;
        if foreground.incarnation == 0 || foreground.cancel != 0 || foreground.forced_exit.is_some() {
            return None;
        }
        Some(Self {
            parent: parent(process_identity(process)),
            session_broker: foreground.session_broker,
            session: foreground.session,
            session_generation: foreground.generation,
            execution: foreground.execution(),
            stat_routed: process.vfs_stat_routed,
            root_rebased: process.vfs_root_rebased,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Terminal {
    Retained(Receipt),
    Refused(Refusal),
    Transport,
    Timeout,
    Cancelled,
    InvalidAcknowledgement,
    AuthorityRetired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Identity,
    Busy,
    Capacity,
    Preparation,
}

pub(super) struct Call {
    claim: RemoteClaim,
    launch: LaunchAttribution,
    parent: AdmissionIdentity,
    source: Scope,
    source_receipt: Receipt,
    grants: u64,
    child: AdmissionIdentity,
    started: u64,
    parked: bool,
    result: Option<Terminal>,
    capture: Option<Inheritance>,
    detached: bool,
    #[cfg(not(test))]
    owner: Option<crate::process::pending::Child>,
}

fn caller(child: Identity) -> Caller {
    Caller {
        pid: child.ticket.pid as u64,
        incarnation: child.incarnation,
        address_space_generation: child.image_generation,
    }
}

fn parent(caller: Caller) -> Parent {
    Parent {
        pid: caller.pid as usize,
        incarnation: caller.incarnation,
        image_generation: caller.address_space_generation,
    }
}

impl Runtime {
    pub(super) fn spawn_request(&self, registration: Registration, id: u64) -> Option<&Call> {
        self.spawns
            .iter()
            .flatten()
            .find(|call| call.claim.registration == registration && call.claim.request_id == id)
    }

    pub(super) fn spawn_live(&self, call: &Call, sched: &Scheduler) -> bool {
        let child = call.claim.child;
        let Some(process) = sched.pending_children.private(child) else {
            return false;
        };
        self.registry.active() == Some(call.claim.registration)
            && sched.pending_children.remote(child) == Some(call.claim)
            && sched.pending_children.parent(child.ticket) == Ok(call.launch.parent)
            && process_identity(process) == call.child.caller
            && process.uid == call.child.uid
            && process.gid == call.child.gid
            && if call.detached {
                call.capture.as_ref().and_then(|pin| self.source_receipt(pin))
                    == Some(call.source_receipt)
            } else {
                sched.processes.iter().any(|waiter| {
                process_identity(waiter) == call.parent.caller
                    && !waiter.is_exiting()
                    && waiter.vfs_root_metadata
                    && waiter.uid == call.parent.uid
                    && waiter.gid == call.parent.gid
                    && waiter.cwd == process.cwd
            })
            && self
                .namespaces
                .lookup(call.parent)
                .and_then(|binding| binding.validate(call.parent))
                .is_ok_and(|namespace| {
                    namespace.id == call.source.namespace_id
                        && namespace.generation == call.source.namespace_generation
                })
            }
    }

    pub(super) fn stage_spawn(
        &mut self,
        idx: usize,
        child: Identity,
        sched: &mut Scheduler,
        caps: &CapabilityBroker,
    ) -> Result<RemoteClaim, Error> {
        self.stage_spawn_inner(idx, child, sched, caps, false)
    }

    pub(super) fn stage_spawn_inner(
        &mut self,
        idx: usize,
        child: Identity,
        sched: &mut Scheduler,
        caps: &CapabilityBroker,
        capture: bool,
    ) -> Result<RemoteClaim, Error> {
        let waiter = sched.processes.get(idx).ok_or(Error::Identity)?;
        if waiter.is_exiting()
            || !waiter.vfs_root_metadata
            || sched.shared_process_index(idx) != idx
        {
            return Err(Error::Identity);
        }
        let exact = process_identity(waiter);
        let launch = LaunchAttribution::capture(waiter).ok_or(Error::Identity)?;
        if sched.pending_children.parent(child.ticket) != Ok(launch.parent) {
            return Err(Error::Identity);
        }
        let process = sched
            .pending_children
            .private(child)
            .ok_or(Error::Identity)?;
        if process_identity(process) != caller(child) || process.cwd != waiter.cwd {
            return Err(Error::Identity);
        }
        if !matches!(waiter.state, ProcessState::Ready | ProcessState::Running)
            || waiter.pending_call.is_some()
            || waiter.ipc_reply_target.is_some()
            || sched.pending_children.remote(child).is_some()
            || self
                .spawns
                .iter()
                .flatten()
                .any(|call| !call.detached && call.parent.caller == exact)
            || self
                .chdirs
                .iter()
                .flatten()
                .any(|call| call.waiter == exact)
            || self
                .publications
                .iter()
                .flatten()
                .any(|call| call.identity.caller == exact)
        {
            return Err(Error::Busy);
        }
        let slot = self
            .spawns
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let registration = live_registration(self, sched, caps).ok_or(Error::Identity)?;
        let parent = AdmissionIdentity {
            caller: exact,
            uid: waiter.uid,
            gid: waiter.gid,
            groups: [],
            registration,
        };
        let source = self.capture_inheritance(parent).ok_or(Error::Identity)?;
        let source_receipt = self.source_receipt(&source).ok_or(Error::Identity)?;
        let namespace = self
            .namespaces
            .lookup(parent)
            .and_then(|binding| binding.validate(parent))
            .map_err(|_| Error::Identity)?;
        let grants = self.namespaces.lookup(parent).map_err(|_| Error::Identity)?.grants();
        let identity = AdmissionIdentity {
            caller: caller(child),
            uid: process.uid,
            gid: process.gid,
            groups: [],
            registration,
        };
        let pin = capture.then(|| source.clone());
        let request_id = self
            .stage_inheritance(source, identity, &process.cwd)
            .map_err(|error| match error {
                NamespaceRetentionError::Ownership(OwnershipError::Capacity)
                | NamespaceRetentionError::Resource(
                    ResourceError::NamespaceCapacity | ResourceError::Allocation,
                ) => Error::Capacity,
                _ => Error::Identity,
            })?;
        let claim = RemoteClaim {
            child,
            registration,
            request_id,
        };
        sched.pending_children.attach_remote(claim);
        self.spawns[slot] = Some(Call {
            claim,
            launch,
            parent,
            source: Scope {
                caller: exact,
                namespace_id: namespace.id,
                namespace_generation: namespace.generation,
            },
            child: identity,
            source_receipt,
            grants,
            started: sched.global_tick,
            parked: false,
            result: None,
            capture: pin,
            detached: false,
            #[cfg(not(test))]
            owner: None,
        });
        Ok(claim)
    }

    pub(super) fn wait_spawn(
        &mut self,
        idx: usize,
        claim: RemoteClaim,
        sched: &mut Scheduler,
    ) -> Result<Option<Terminal>, Error> {
        self.wait_spawn_inner(idx, claim, sched, false)
    }

    fn wait_spawn_inner(
        &mut self,
        idx: usize,
        claim: RemoteClaim,
        sched: &mut Scheduler,
        keep_retained: bool,
    ) -> Result<Option<Terminal>, Error> {
        let waiter = sched
            .processes
            .get(idx)
            .filter(|waiter| !waiter.is_exiting())
            .ok_or(Error::Identity)?;
        let exact = process_identity(waiter);
        let slot = self
            .spawns
            .iter()
            .position(|call| {
                call.as_ref()
                    .is_some_and(|call| !call.detached && call.claim == claim && call.parent.caller == exact)
            })
            .ok_or(Error::Identity)?;
        if waiter.pending_call.is_some() || waiter.ipc_reply_target.is_some() {
            return Err(Error::Busy);
        }
        self.expire_spawns(sched.global_tick);
        let call = self.spawns[slot].as_ref().unwrap();
        if matches!(call.result, None | Some(Terminal::Retained(_)))
            && !self.spawn_live(call, sched)
        {
            self.finish_spawn(claim, Terminal::InvalidAcknowledgement);
        }
        let call = self.spawns[slot].as_mut().unwrap();
        let keep_retained = keep_retained || call.capture.is_some();
        if let Some(result) = call.result {
            if keep_retained && call.parked {
                return Ok(None);
            }
            if !keep_retained || !matches!(result, Terminal::Retained(_)) {
                #[cfg(not(test))]
                if call.owner.is_some() {
                    call.detached = true;
                    return Ok(Some(result));
                }
                self.spawns[slot] = None;
            }
            return Ok(Some(result));
        }
        call.parked = true;
        sched.processes[idx].state = ProcessState::BlockedOnIpc;
        Ok(None)
    }

    pub(super) fn publish_spawn(
        &mut self,
        idx: usize,
        claim: RemoteClaim,
        sched: &mut Scheduler,
        caps: &CapabilityBroker,
        prepare: impl FnOnce(&mut Scheduler) -> Result<(), Error>,
        publish: impl FnOnce(&mut Scheduler, Identity, LaunchAttribution) -> usize,
    ) -> Result<usize, Error> {
        let slot = self.spawns.iter().position(|entry| {
            entry.as_ref().is_some_and(|call| call.claim == claim)
        }).ok_or(Error::Identity)?;
        if live_registration(self, sched, caps) != Some(claim.registration) {
            return Err(Error::Identity);
        }
        let call = self.spawns[slot].as_ref().unwrap();
        let Some(Terminal::Retained(receipt)) = call.result else {
            return Err(Error::Identity);
        };
        if call.detached || call.parked
            || sched.global_tick.wrapping_sub(call.started) >= WAIT_TICKS
            || !sched.processes.get(idx).is_some_and(|process| {
                process_identity(process) == call.parent.caller
                    && sched.shared_process_index(idx) == idx
                    && LaunchAttribution::capture(process) == Some(call.launch)
            })
            || !self.spawn_live(call, sched)
            || self.capture_inheritance(call.parent)
                .and_then(|source| self.source_receipt(&source)) != Some(call.source_receipt)
            || receipt.registration() != claim.registration
            || receipt.request_id() != claim.request_id
            || receipt.scope().caller != call.child.caller
        {
            return Err(Error::Identity);
        }
        let identity = call.child;
        let launch = call.launch;
        let grants = call.grants;
        let remote = self.remote.iter_mut().flatten()
            .find(|remote| remote.registration == claim.registration)
            .ok_or(Error::Identity)?;
        if !remote.ownership.holds(receipt) {
            return Err(Error::Identity);
        }
        let local = remote.local.iter_mut().find(|slot| slot.is_none()).ok_or(Error::Capacity)?;
        prepare(sched)?;
        let binding = self.namespaces.install_receipt(identity, receipt, grants)
            .map_err(|_| Error::Preparation)?;
        *local = Some(LocalLease::Bound { receipt, binding });
        #[cfg(not(test))]
        if let Some(owner) = self.spawns[slot].as_mut().unwrap().owner.take() {
            owner.disarm();
        }
        self.spawns[slot] = None;
        Ok(publish(sched, claim.child, launch))
    }

    pub(super) fn reply_spawn(
        &mut self,
        registration: Registration,
        id: u64,
        decoded: Frame,
        bytes: &[u8],
        sched: &Scheduler,
    ) -> Result<(), IpcError> {
        self.expire_spawns(sched.global_tick);
        let call = self
            .spawn_request(registration, id)
            .ok_or(IpcError::InvalidCapability)?;
        let claim = call.claim;
        let live = call.result.is_none()
            && self.spawn_live(call, sched)
            && self
                .validate_derivation(registration, &bytes_for_request(self, claim)?, sched, call.detached)
                .is_ok();
        let result = match decoded {
            Frame::Retained(_) => {
                match self
                    .retained(registration, id, bytes)
                    .map_err(|_| IpcError::InvalidCapability)?
                {
                    Retained::Install(receipt) if live => Terminal::Retained(receipt),
                    Retained::Install(_) | Retained::Release(_) => Terminal::InvalidAcknowledgement,
                }
            }
            Frame::Refused { .. } => {
                let reason = self
                    .refused(registration, id, bytes)
                    .map_err(|_| IpcError::InvalidCapability)?;
                if live {
                    Terminal::Refused(reason)
                } else {
                    Terminal::InvalidAcknowledgement
                }
            }
            _ => return Err(IpcError::InvalidCapability),
        };
        if let Some(remote) = self
            .remote
            .iter_mut()
            .flatten()
            .find(|remote| remote.registration == registration)
        {
            remote.delivered = None;
            for lease in &mut remote.local {
                if lease.as_ref().is_some_and(|lease| lease.id() == id) {
                    *lease = None;
                }
            }
        }
        if let Some(call) = self
            .spawns
            .iter_mut()
            .flatten()
            .find(|call| call.claim == claim)
        {
            if call.result.is_none() {
                call.result = Some(result);
            }
        }
        if matches!(result, Terminal::InvalidAcknowledgement) {
            self.handoff_spawn(claim);
        }
        self.reconcile_namespaces();
        #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_r2_test"))]
        fixture::observe_registration_result(claim, result);
        Ok(())
    }

    pub(super) fn handoff_spawn(&mut self, claim: RemoteClaim) {
        let Some(remote) = self
            .remote
            .iter_mut()
            .flatten()
            .find(|remote| remote.registration == claim.registration)
        else {
            return;
        };
        let id = claim.request_id;
        let local = remote
            .local
            .iter_mut()
            .find(|lease| lease.as_ref().is_some_and(|lease| lease.id() == id));
        let queued = remote.unsent.iter_mut().find(
            |frame| matches!(frame, Some(Frame::Retain { request_id, .. }) if *request_id == id),
        );
        let lifecycle = remote.callers.iter_mut().find(|slot| {
            slot.as_ref()
                .is_some_and(|entry| entry.caller == caller(claim.child))
        });
        if let Some(queued) = queued {
            remote
                .ownership
                .discard_unsent(queued.take().unwrap())
                .expect("unsent spawn retain");
            *local.expect("unsent spawn preparation") = None;
            *lifecycle.expect("reserved spawn caller delivery") = None;
        } else {
            if let Some(LocalLease::Preparing { scope, .. }) = local.and_then(Option::as_mut) {
                remote
                    .ownership
                    .abandon(id, *scope)
                    .expect("delivered spawn retain");
            } else if remote.ownership.contains(id) {
                remote.ownership.release(id).expect("unbound spawn receipt");
            }
            if let Some(entry) = lifecycle.and_then(Option::as_mut) {
                entry.retired = true;
                entry.file_retired = true;
            }
        }
    }

    fn finish_spawn(&mut self, claim: RemoteClaim, result: Terminal) {
        self.handoff_spawn(claim);
        if let Some(call) = self
            .spawns
            .iter_mut()
            .flatten()
            .find(|call| call.claim == claim)
        {
            if matches!(call.result, None | Some(Terminal::Retained(_))) {
                call.result = Some(result);
            }
        }
    }

    pub(super) fn retire_spawns(&mut self, registration: Registration) {
        for slot in 0..self.spawns.len() {
            if let Some(call) = self.spawns[slot]
                .as_ref()
                .filter(|call| call.claim.registration == registration)
            {
                self.finish_spawn(call.claim, Terminal::AuthorityRetired);
            }
        }
    }

    pub(super) fn cancel_spawns_for(&mut self, caller: Caller) {
        for slot in 0..self.spawns.len() {
            if let Some(call) = self.spawns[slot]
                .as_ref()
                .filter(|call| !call.detached && call.parent.caller == caller)
            {
                self.finish_spawn(call.claim, Terminal::Cancelled);
            }
        }
    }

    pub(super) fn detach_spawns_for(&mut self, caller: Caller) {
        for call in self.spawns.iter_mut().flatten().filter(|call| call.parent.caller == caller) {
            if call.capture.is_some() {
                call.detached = true;
                call.parked = false;
            }
        }
    }

    pub(super) fn expire_spawns(&mut self, now: u64) {
        for slot in 0..self.spawns.len() {
            if let Some(call) = self.spawns[slot].as_ref().filter(|call| {
                call.result.is_none() && now.wrapping_sub(call.started) >= WAIT_TICKS
            }) {
                let claim = call.claim;
                self.finish_spawn(claim, Terminal::Timeout);
                self.retire_registration(claim.registration);
            }
        }
    }

    pub(super) fn poll_spawns(&mut self, sched: &mut Scheduler, now: u64) {
        #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_parent_lifecycle_test"))]
        fixture::parent_lifecycle::cancel_detached(self, sched);
        #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_1_test"))]
        fixture::poll_observer(sched);
        #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_2_test"))]
        if let Some((registration, request)) = fixture::pending_cancellation() {
            if let Some(call) = self.spawn_request(registration, request) {
                #[cfg(feature = "vfs_l1c_adapter_accounting_test")]
                if call.result.is_none() {
                    fixture::accounting::before_unsent_cancel(self, call.claim);
                }
                self.finish_spawn(call.claim, Terminal::Cancelled);
            }
        }
        self.expire_spawns(now);
        for slot in 0..self.spawns.len() {
            let Some(call) = self.spawns[slot].as_ref() else {
                continue;
            };
            if call.detached {
                if matches!(call.result, Some(Terminal::Retained(_)))
                    && now.wrapping_sub(call.started) >= WAIT_TICKS
                {
                    self.finish_spawn(call.claim, Terminal::Timeout);
                }
                continue;
            }
            let waiter = sched.processes.iter().position(|waiter| {
                process_identity(waiter) == call.parent.caller && !waiter.is_exiting()
            });
            let Some(idx) = waiter else {
                let claim = call.claim;
                if call.capture.is_some() {
                    self.detach_spawns_for(call.parent.caller);
                    continue;
                }
                self.spawns[slot] = None;
                self.handoff_spawn(claim);
                continue;
            };
            if call.parked && call.result.is_some() {
                #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_r1_test"))]
                if fixture::awaiting_service_exit(call.claim) {
                    continue;
                }
                let waiter = &sched.processes[idx];
                if waiter.pending_call.is_none() && waiter.ipc_reply_target.is_none() {
                    super::super::wake_terminal(sched, idx);
                    self.spawns[slot].as_mut().unwrap().parked = false;
                }
            }
        }
        sched.pending_children.reap_remote(|claim| {
            let settled = self
                .remote
                .iter()
                .flatten()
                .find(|remote| remote.registration == claim.registration)
                .is_none_or(|remote| {
                    (remote.image_retired && self.spawn_request(claim.registration, claim.request_id)
                        .is_some_and(|call| call.claim == claim && call.detached && call.capture.is_some()))
                    || (!remote.ownership.contains(claim.request_id)
                        && !remote
                            .callers
                            .iter()
                            .flatten()
                            .any(|entry| entry.caller == caller(claim.child)))
                });
            #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_1_test"))]
            if settled {
                crate::serial_println!(
                    "[VFS-L1C-1] settled child={} retain={} epoch={}",
                    claim.child.ticket.pid,
                    claim.request_id,
                    claim.registration.epoch
                );
            }
            settled
        });
        let mut released_pin = false;
        for slot in &mut self.spawns {
            if slot.as_ref().is_some_and(|call| {
                call.detached
                    && sched.pending_children.remote(call.claim.child).is_none()
                    && matches!(call.result, Some(result) if !matches!(result, Terminal::Retained(_)))
            }) {
                *slot = None;
                released_pin = true;
            }
        }
        if released_pin {
            self.reconcile_namespaces();
            self.collect_remote();
        }
        #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_r5_test"))]
        fixture::accounting::poll(self, sched);
    }

    pub(super) fn fail_spawn_copy(&mut self, registration: Registration, request_id: u64) {
        let Some(call) = self.spawn_request(registration, request_id) else {
            return;
        };
        if call.result.is_none()
            && self.remote.iter().flatten().any(|remote| {
                remote.registration == registration && remote.delivered == Some(request_id)
            })
        {
            self.finish_spawn(call.claim, Terminal::Transport);
        }
    }
}

#[cfg(not(test))]
#[allow(dead_code)]
pub(crate) fn stage_owned(
    idx: usize,
    owner: &mut Option<crate::process::pending::Child>,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<RemoteClaim, Error> {
    let child = owner.as_ref().ok_or(Error::Identity)?.private_identity();
    let mut runtime = RUNTIME.lock();
    let claim = runtime.stage_spawn_inner(idx, child, sched, caps, true)?;
    runtime.spawns.iter_mut().flatten().find(|call| call.claim == claim)
        .expect("staged transaction").owner = owner.take();
    Ok(claim)
}

#[cfg(not(test))]
pub(crate) fn reap_owned() {
    let (owner, _claim) = {
        let Some(mut runtime) = RUNTIME.try_lock() else { return };
        let Some(call) = runtime.spawns.iter_mut().flatten().find(|call| {
            call.detached && call.owner.is_some()
                && matches!(call.result, Some(result) if !matches!(result, Terminal::Retained(_)))
        }) else { return };
        (call.owner.take(), call.claim)
    };
    drop(owner);
    #[cfg(all(debug_assertions, feature = "vfs_l1c_parent_lifecycle_test"))]
    fixture::parent_lifecycle::disposed(_claim);
}

fn bytes_for_request(
    runtime: &Runtime,
    claim: RemoteClaim,
) -> Result<[u8; sunlight_ipc::vfs_rpc::retention::wire::FRAME_BYTES], IpcError> {
    runtime
        .remote
        .iter()
        .flatten()
        .find(|remote| remote.registration == claim.registration)
        .and_then(|remote| {
            remote.local.iter().flatten().find_map(|lease| match lease {
                LocalLease::Preparing { id, scope, .. } if *id == claim.request_id => {
                    Frame::Retain {
                        registration: claim.registration,
                        scope: *scope,
                        request_id: *id,
                    }
                    .encode()
                    .ok()
                }
                _ => None,
            })
        })
        .ok_or(IpcError::InvalidCapability)
}

#[allow(dead_code)]
pub(crate) fn stage(
    idx: usize,
    child: Identity,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<RemoteClaim, Error> {
    RUNTIME.lock().stage_spawn(idx, child, sched, caps)
}

#[allow(dead_code)]
pub(crate) fn wait(
    idx: usize,
    claim: RemoteClaim,
    sched: &mut Scheduler,
) -> Result<Option<Terminal>, Error> {
    RUNTIME.lock().wait_spawn(idx, claim, sched)
}

#[cfg(not(test))]
#[allow(dead_code)]
pub(crate) fn wait_for_publication(
    idx: usize,
    claim: RemoteClaim,
    sched: &mut Scheduler,
) -> Result<Option<Terminal>, Error> {
    RUNTIME.lock().wait_spawn_inner(idx, claim, sched, true)
}

#[cfg(not(test))]
#[allow(dead_code)]
pub(crate) fn publish(
    idx: usize,
    claim: RemoteClaim,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<usize, Error> {
    RUNTIME.lock().publish_spawn(
        idx, claim, sched, caps,
        |sched| {
            #[cfg(all(debug_assertions, feature = "vfs_l1c_adapter_accounting_test"))]
            if fixture::reject_preparation(claim) {
                return Err(Error::Preparation);
            }
            sched.reserve_private_publication().map_err(|_| Error::Preparation)
        },
        |sched, identity, launch| {
            #[cfg(all(debug_assertions, feature = "vfs_l1c_1_test"))]
            fixture::publication_event(identity, "binding");
            sched.pending_children.eligible(identity, true).expect("validated private owner");
            let mut process = sched.pending_children.transfer(identity, true)
                .expect("exclusive scheduler publication owner");
            process.vfs_root_metadata = true;
            process.vfs_stat_routed = launch.stat_routed;
            process.vfs_root_rebased = launch.root_rebased;
            debug_assert_eq!(process.ppid, launch.parent.pid);
            process.foreground.parent_generation = launch.parent.incarnation;
            process.foreground.session_broker = launch.session_broker;
            process.foreground.session = launch.session;
            process.foreground.generation = launch.session_generation;
            process.foreground.member = launch.execution;
            process.child_wait = crate::process::ChildWait::Pending;
            sched.pending_children.published(identity.ticket);
            let slot = sched.insert_attributed_process(*process);
            #[cfg(all(debug_assertions, feature = "vfs_l1c_1_test"))]
            fixture::publication_event(identity, "committed");
            sched.enqueue_ready(slot);
            debug_assert_eq!(sched.diagnostic_ready_occurrences(slot), 1);
            #[cfg(all(debug_assertions, feature = "vfs_l1c_1_test"))]
            fixture::publication_event(identity, "enqueue");
            identity.ticket.pid
        },
    )
}

#[cfg(not(test))]
pub(crate) fn cancel_child(claim: RemoteClaim) {
    RUNTIME.lock().finish_spawn(claim, Terminal::Cancelled);
}

pub(crate) fn copy_failed(bytes: &[u8]) {
    if let Ok(Frame::Retain {
        registration,
        request_id,
        ..
    }) = Frame::decode(bytes)
    {
        RUNTIME.lock().fail_spawn_copy(registration, request_id);
    }
}

#[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_1_test"))]
pub(crate) mod fixture {
    use super::*;
    #[cfg(feature = "vfs_l1c_r3_test")]
    pub(in crate::ipc::vfs_registration) mod r3;
    #[cfg(feature = "vfs_l1c_r5_test")]
    pub(super) mod accounting;
    #[cfg(feature = "vfs_l1c_parent_lifecycle_test")]
    pub(crate) mod parent_lifecycle;
    use crate::capability::CAP_BROKER;
    use crate::process::pending::{self, Child, Image};
    use crate::sched::SCHEDULER;
    use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    use spin::Mutex;

    static CALL: Mutex<Option<(Child, RemoteClaim)>> = Mutex::new(None);
    static PRIVATE: AtomicUsize = AtomicUsize::new(0);
    static PUBLICATION: AtomicUsize = AtomicUsize::new(0);
    static MODE: AtomicUsize = AtomicUsize::new(0);
    #[cfg(feature = "vfs_l1c_2_test")]
    static METADATA: AtomicUsize = AtomicUsize::new(0);
    static WAITER: AtomicUsize = AtomicUsize::new(0);
    static OBSERVER: AtomicUsize = AtomicUsize::new(0);
    static OTHER: AtomicUsize = AtomicUsize::new(0);
    static OBSERVER_EXITED: AtomicUsize = AtomicUsize::new(0);
    static REQUEST: Mutex<Option<(Registration, u64)>> = Mutex::new(None);
    #[cfg(feature = "vfs_l1c_r4_test")]
    type ReplyBytes = [u8; sunlight_ipc::vfs_rpc::retention::wire::FRAME_BYTES];
    #[cfg(feature = "vfs_l1c_r4_test")]
    static HELD: Mutex<Option<ReplyBytes>> = Mutex::new(None);
    #[cfg(feature = "vfs_l1c_r5_test")]
    static COMPLETED: Mutex<Option<(RemoteClaim, usize, ReplyBytes)>> = Mutex::new(None);
    #[cfg(feature = "vfs_l1c_r5_test")]
    static RELEASES: Mutex<[Option<Receipt>; 2]> = Mutex::new([None; 2]);

    #[cfg(feature = "vfs_l1c_r5_test")]
    pub(in crate::ipc::vfs_registration) fn observe_release(registration: Registration, input: &[u8]) {
        let Ok(Frame::Released(receipt)) = Frame::decode(input) else {
            return;
        };
        assert_eq!(receipt.registration(), registration);
        if let Some(slot) = RELEASES.lock().iter_mut().find(|slot| **slot == Some(receipt)) {
            *slot = None;
            crate::serial_println!("[VFS-R5] release child={} retain={} epoch={}",
                receipt.scope().caller.pid, receipt.request_id(), registration.epoch);
        }
    }

    #[cfg(feature = "vfs_l1c_r5_test")]
    pub(in crate::ipc::vfs_registration) fn observe_reply(
        runtime: &mut Runtime,
        registration: Registration,
        input: &[u8],
        sched: &Scheduler,
    ) {
        let Ok(Frame::Retained(receipt)) = Frame::decode(input) else {
            return;
        };
        if *REQUEST.lock() != Some((registration, receipt.request_id())) {
            return;
        }
        let slot = runtime.spawns.iter().position(|entry| entry.as_ref().is_some_and(|call| {
            call.claim.registration == registration && call.claim.request_id == receipt.request_id()
        })).expect("fixture continuation slot");
        let claim = runtime.spawns[slot].as_ref().unwrap().claim;
        let mode = MODE.load(Relaxed);
        if !matches!(mode, 0 | 2..=4)
            && !(cfg!(feature = "vfs_l1c_parent_lifecycle_test") && matches!(mode, 13 | 16))
        {
            return;
        }
        let ordinal = usize::from(mode != 0);
        assert!(RELEASES.lock()[ordinal].replace(receipt).is_none());
        crate::serial_println!(
            "[VFS-R5] receipt child={} incarnation={} vm={} retain={} service={} service_incarnation={} service_vm={} epoch={}",
            claim.child.ticket.pid, claim.child.incarnation, claim.child.image_generation,
            claim.request_id, registration.owner.pid, registration.owner.incarnation,
            registration.owner.address_space_generation, registration.epoch
        );
        if matches!(mode, 2 | 3 | 13 | 16) {
            return;
        }
        if mode == 0 {
            assert!(COMPLETED.lock().replace((claim, slot, input.try_into().unwrap())).is_none());
            return;
        }
        let (old, old_slot, bytes) = COMPLETED.lock().take().expect("original completion");
        assert_eq!(old_slot, slot);
        assert_eq!(old.registration, registration);
        assert_ne!(old.request_id, claim.request_id);
        assert_ne!(old.child, claim.child);
        let current = runtime.spawns[slot].as_ref().unwrap();
        assert!(current.parked && current.result.is_none());
        assert_eq!(runtime.control_reply(registration, &bytes, sched), Err(IpcError::InvalidCapability));
        let current = runtime.spawns[slot].as_ref().unwrap();
        assert_eq!(current.claim, claim);
        assert!(current.parked && current.result.is_none());
        assert!(runtime.spawn_live(current, sched));
        crate::serial_println!(
            "[VFS-R5] reused slot={} old_child={} old_retain={} child={} retain={} epoch={} rejected=1",
            slot, old.child.ticket.pid, old.request_id, claim.child.ticket.pid,
            claim.request_id, registration.epoch
        );
    }

    #[cfg(feature = "vfs_l1c_r5_test")]
    fn replay_published(idx: usize, claim: RemoteClaim, sched: &mut Scheduler) {
        let saved = COMPLETED.lock();
        let (original, slot, bytes) = saved.as_ref().unwrap();
        assert_eq!(*original, claim);
        let Frame::Retained(receipt) = Frame::decode(bytes).unwrap() else { unreachable!() };
        let caps = CAP_BROKER.lock();
        {
            let mut runtime = RUNTIME.lock();
            assert_eq!(live_registration(&mut runtime, sched, &caps), Some(claim.registration));
            let process = &sched.processes[sched.process_index_by_pid(claim.child.ticket.pid).unwrap()];
            assert_eq!(process_identity(process), caller(claim.child));
            assert!(runtime.spawns[*slot].is_none());
            assert_eq!(runtime.control_reply(claim.registration, bytes, sched), Err(IpcError::InvalidCapability));
            let remote = runtime.remote.iter().flatten()
                .find(|remote| remote.registration == claim.registration && !remote.retired)
                .unwrap();
            assert!(remote.local.iter().flatten().any(|lease| {
                matches!(lease, LocalLease::Bound { receipt: bound, .. } if *bound == receipt)
            }));
            assert!(remote.ownership.holds(receipt));
            assert!(runtime.spawns[*slot].is_none());
        }
        assert!(sched.pending_children.private(claim.child).is_none());
        assert_eq!(wait_for_publication(idx, claim, sched), Err(Error::Identity));
        assert_eq!(publish(idx, claim, sched, &caps), Err(Error::Identity));
        crate::serial_println!("[VFS-R5] published child={} retain={} slot={} rejected=1",
            claim.child.ticket.pid, claim.request_id, slot);
    }
    #[cfg(feature = "vfs_l1c_r1_test")]
    static EXIT_REQUESTED: AtomicUsize = AtomicUsize::new(0);
    #[cfg(feature = "vfs_l1c_r1_test")]
    static EXIT_PROVEN: AtomicUsize = AtomicUsize::new(0);

    #[cfg(feature = "vfs_l1c_r1_test")]
    pub(in crate::ipc::vfs_registration) fn retire_checkpoint_service(sched: &mut Scheduler) {
        if EXIT_REQUESTED.load(Relaxed) != 0 {
            return;
        }
        let before_acquisition = match MODE.load(Relaxed) {
            5 if HELD.lock().is_some() => false,
            6 if cfg!(feature = "vfs_l1c_r1b_test") && OBSERVER_EXITED.load(Relaxed) != 0 => true,
            _ => return,
        };
        let (registration, request) = REQUEST.lock().expect("R1 checkpoint request");
        {
            let runtime = RUNTIME.lock();
            let call = runtime.spawn_request(registration, request).expect("pending continuation");
            assert!(call.parked && call.result.is_none());
            let remote = runtime.remote.iter().flatten().find(|remote| {
                remote.registration == registration && !remote.retired
            }).expect("active checkpoint registration");
            assert!(remote.ownership.contains(request));
            if before_acquisition {
                let child = caller(call.claim.child);
                assert!(remote.unsent.iter().flatten().any(|frame| {
                    matches!(frame, Frame::Retain { request_id, scope, .. }
                        if *request_id == request && scope.caller == child)
                }));
                assert_ne!(remote.delivered, Some(request));
                assert!(remote.local.iter().flatten().any(|lease| {
                    matches!(lease, LocalLease::Preparing { .. } if lease.id() == request)
                }));
                assert!(remote.callers.iter().flatten().any(|entry| entry.caller == child));
                assert!(HELD.lock().is_none());
                crate::serial_println!(
                    "[VFS-R1B] checkpoint service={} child={} retain={} epoch={} unsent=1 delivered=0 preparing=1 caller_reserved=1",
                    registration.owner.pid, child.pid, request, registration.epoch
                );
            } else {
                assert_eq!(remote.delivered, Some(request));
            }
        }
        let service = registration.owner.pid as usize;
        EXIT_REQUESTED.store(service, Relaxed);
        let reason = if before_acquisition {
            "vfs-r1b-unsent-registration"
        } else {
            "vfs-r1-held-registration"
        };
        assert!(sched.terminate_process_by_pid(service, 0, reason));
    }

    #[cfg(feature = "vfs_l1c_r1_test")]
    pub(in crate::ipc::vfs_registration) fn observe_service_exit(
        image: crate::capability::delivery::RecipientIdentity,
    ) {
        if EXIT_REQUESTED.load(Relaxed) != image.pid {
            return;
        }
        #[cfg(feature = "vfs_l1c_r3_test")]
        if MODE.load(Relaxed) == 8 {
            r3::proof(image);
            return;
        }
        let (registration, request) = REQUEST.lock().expect("R1 retirement interest");
        assert_eq!(registration.owner.incarnation, image.incarnation);
        assert_eq!(registration.owner.address_space_generation, image.vm_generation);
        let runtime = RUNTIME.lock();
        let call = runtime.spawn_request(registration, request).expect("R1 terminal continuation");
        assert!(matches!(call.result, Some(Terminal::AuthorityRetired)));
        assert!(runtime.remote.iter().flatten().all(|remote| {
            remote.registration != registration || (remote.retired && remote.image_retired)
        }));
        assert_eq!(EXIT_PROVEN.swap(image.pid, Relaxed), 0);
        let label = if r2_mode() { "VFS-R2" } else { "VFS-R1" };
        crate::serial_println!(
            "[{}] proof service={} incarnation={} vm={} epoch={} retain={} row_retained={}",
            label, image.pid, image.incarnation, image.vm_generation, registration.epoch, request,
            runtime.remote.iter().flatten().any(|remote| remote.registration == registration)
        );
    }

    #[cfg(feature = "vfs_l1c_r1_test")]
    pub(super) fn awaiting_service_exit(claim: RemoteClaim) -> bool {
        (MODE.load(Relaxed) == 5 || before_acquisition_mode())
            && EXIT_PROVEN.load(Relaxed) == 0
            && *REQUEST.lock() == Some((claim.registration, claim.request_id))
    }

    #[cfg(feature = "vfs_l1c_r2_test")]
    pub(super) fn observe_registration_result(claim: RemoteClaim, result: Terminal) {
        if r2_mode() && matches!(result, Terminal::Retained(_))
            && *REQUEST.lock() == Some((claim.registration, claim.request_id))
        {
            crate::serial_println!(
                "[VFS-R2] authenticated service={} child={} retain={} epoch={}",
                claim.registration.owner.pid, claim.child.ticket.pid,
                claim.request_id, claim.registration.epoch
            );
        }
    }

    #[cfg(feature = "vfs_l1c_r2_test")]
    pub(in crate::ipc::vfs_registration) fn observe_withdrawal(runtime: &Runtime, registration: Registration) {
        if !r2_mode() || EXIT_REQUESTED.load(Relaxed) != registration.owner.pid as usize {
            return;
        }
        let (expected, request) = REQUEST.lock().expect("R2 retirement interest");
        assert_eq!(registration, expected);
        let call = runtime.spawn_request(registration, request).expect("retired continuation");
        assert_eq!(call.result, Some(Terminal::AuthorityRetired));
        assert_ne!(runtime.registry.active(), Some(registration));
        let remote = runtime.remote.iter().flatten()
            .find(|remote| remote.registration == registration && remote.retired)
            .expect("registration still owned until image proof");
        assert!(!remote.image_retired);
        let child = caller(call.claim.child);
        assert!(remote.ownership.releases().any(|frame| {
            matches!(frame, Frame::Release(receipt)
                if receipt.request_id() == request && receipt.scope().caller == child)
        }));
        crate::serial_println!(
            "[VFS-R2] withdrawn service={} child={} retain={} epoch={} release_pending=1",
            registration.owner.pid, child.pid, request, registration.epoch
        );
    }

    fn r2_mode() -> bool {
        cfg!(feature = "vfs_l1c_r2_test") && MODE.load(Relaxed) == 7
    }

    fn before_acquisition_mode() -> bool {
        cfg!(feature = "vfs_l1c_r1b_test") && MODE.load(Relaxed) == 6
    }

    #[cfg(feature = "vfs_l1c_r4_test")]
    pub(crate) fn tracks_child(pid: usize) -> bool {
        (MODE.load(Relaxed) == 4 || (cfg!(feature = "vfs_l1c_r1_test") && MODE.load(Relaxed) == 5)
            || before_acquisition_mode() || r2_mode()
            || (cfg!(feature = "vfs_l1c_adapter_accounting_test")
                && matches!(MODE.load(Relaxed), 1..=3 | 11))
            || (cfg!(feature = "vfs_l1c_parent_lifecycle_test")
                && matches!(MODE.load(Relaxed), 12 | 13 | 16)))
            && PRIVATE.load(Relaxed) == pid
    }

    #[cfg(feature = "vfs_l1c_r4_test")]
    pub(in crate::ipc::vfs_registration) fn hold_reply(registration: Registration, input: &[u8]) -> bool {
        let mode = MODE.load(Relaxed);
        if mode != 4 && !(cfg!(feature = "vfs_l1c_r1_test") && mode == 5) {
            return false;
        }
        let Ok(Frame::Retained(receipt)) = Frame::decode(input) else {
            return false;
        };
        if *REQUEST.lock() != Some((registration, receipt.request_id()))
            || receipt.registration() != registration
        {
            return false;
        }
        assert!(HELD.lock().replace(input.try_into().unwrap()).is_none());
        crate::serial_println!("[{}] B held service={} child={} retain={} epoch={}",
            if mode == 5 { "VFS-R1" } else { "VFS-R4" },
            registration.owner.pid, receipt.scope().caller.pid, receipt.request_id(), registration.epoch);
        true
    }

    #[cfg(feature = "vfs_l1c_r4_test")]
    fn release_reply(claim: RemoteClaim, sched: &Scheduler) {
        let held = HELD.lock().take().expect("original authenticated acknowledgement");
        let caps = CAP_BROKER.lock();
        let mut runtime = RUNTIME.lock();
        let registration = super::super::transport::authenticate(
            &mut runtime, claim.registration.owner.pid as usize, claim.registration.epoch, sched, &caps,
        ).expect("same authorized service");
        assert_eq!(registration, claim.registration);
        assert!(runtime.spawn_request(registration, claim.request_id).is_none());
        let slot = runtime.remote.iter().position(|entry| entry.as_ref().is_some_and(|remote| {
            remote.registration == registration && !remote.retired
                && remote.delivered == Some(claim.request_id)
                && remote.ownership.contains(claim.request_id)
        })).expect("registration owns unresolved retain");
        assert_eq!(sched.pending_children.remote(claim.child), Some(claim));
        assert!(sched.pending_children.pids().any(|pid| pid == claim.child.ticket.pid));
        crate::serial_println!("[VFS-R4] C reclaimed child={} retain={} slot={} pid_reserved=1 continuation=0",
            claim.child.ticket.pid, claim.request_id, slot);
        runtime.control_reply(registration, &held, sched).expect("process original late acknowledgement");
        #[cfg(feature = "vfs_l1c_r5_test")]
        {
            assert_eq!(runtime.control_reply(registration, &held, sched), Err(IpcError::InvalidCapability));
            assert!(sched.process_index_by_pid(claim.child.ticket.pid).is_none());
            assert!(sched.pending_children.private(claim.child).is_none());
            crate::serial_println!("[VFS-R5] reclaimed child={} retain={} rejected=1",
                claim.child.ticket.pid, claim.request_id);
        }
        let remote = runtime.remote[slot].as_ref().unwrap();
        assert_eq!(remote.registration, registration);
        assert_ne!(remote.delivered, Some(claim.request_id));
        assert!(remote.ownership.releases().any(|frame| {
            matches!(frame, Frame::Release(receipt) if receipt.request_id() == claim.request_id)
        }));
        assert!(remote.local.iter().flatten().all(|lease| lease.id() != claim.request_id));
        assert!(runtime.spawn_request(registration, claim.request_id).is_none());
        crate::serial_println!("[VFS-R4] D late child={} retain={} slot={} compensation=1 continuation=0",
            claim.child.ticket.pid, claim.request_id, slot);
    }

    pub(crate) fn dispose_orphan() {
        #[cfg(feature = "vfs_l1c_parent_lifecycle_test")]
        if matches!(MODE.load(Relaxed), 12 | 13 | 16) {
            return;
        }
        let abandoned = x86_64::instructions::interrupts::without_interrupts(|| {
            let mut slot = CALL.try_lock()?;
            let (child, _) = slot.as_ref()?;
            if child.identity() != Err(crate::process::pending_state::Error::ParentGone) {
                return None;
            }
            *REQUEST.lock() = None;
            PRIVATE.store(0, Relaxed);
            slot.take()
        });
        drop(abandoned);
    }

    #[cfg(feature = "vfs_l1c_2_test")]
    pub(super) fn pending_cancellation() -> Option<(Registration, u64)> {
        #[cfg(feature = "vfs_l1c_r4_test")]
        if MODE.load(Relaxed) == 4 && HELD.lock().is_some() {
            return *REQUEST.lock();
        }
        if MODE.load(Relaxed) == 1 && OBSERVER_EXITED.load(Relaxed) != 0 {
            *REQUEST.lock()
        } else {
            None
        }
    }

    #[cfg(feature = "vfs_l1c_adapter_accounting_test")]
    pub(super) fn reject_preparation(claim: RemoteClaim) -> bool {
        MODE.load(Relaxed) == 3 && PRIVATE.load(Relaxed) == claim.child.ticket.pid
    }

    #[cfg(feature = "vfs_l1c_adapter_accounting_test")]
    pub(in crate::ipc::vfs_registration) fn withhold_source(
        registration: Registration,
        input: &[u8],
    ) -> bool {
        if MODE.load(Relaxed) != 11 {
            return false;
        }
        let Ok(Frame::Retain {
            registration: claimed,
            request_id,
            ..
        }) = Frame::decode(input) else {
            return false;
        };
        claimed == registration && *REQUEST.lock() == Some((registration, request_id))
    }

    pub(crate) fn publication_event(identity: Identity, event: &str) {
        let pid = identity.ticket.pid;
        if PRIVATE.load(Relaxed) == pid {
            if event == "enqueue" {
                PUBLICATION.store(pid, Relaxed);
            }
            crate::serial_println!("[VFS-L1C-2] {} child={}", event, pid);
        }
    }

    #[cfg(feature = "vfs_l1c_2_test")]
    pub(crate) fn observe_metadata(caller: Caller) {
        if PUBLICATION.load(Relaxed) == caller.pid as usize
            && METADATA.swap(caller.pid as usize, Relaxed) != caller.pid as usize
        {
            crate::serial_println!("[VFS-L1C-2] metadata completed child={}", caller.pid);
        }
    }

    pub(crate) fn awaiting_observer(registration: Registration, id: u64) -> bool {
        *REQUEST.lock() == Some((registration, id))
            && (before_acquisition_mode()
                || (!matches!(MODE.load(Relaxed), 13..=16)
                    && OBSERVER_EXITED.load(Relaxed) == 0))
    }

    pub(crate) fn poll_observer(sched: &Scheduler) {
        let observer = OTHER.load(Relaxed);
        if observer != 0
            && sched
                .processes
                .iter()
                .any(|process| process.pid == observer && process.is_exiting())
        {
            OBSERVER_EXITED.store(observer, Relaxed);
        }
    }

    pub(crate) fn observe_dispatch(sched: &Scheduler, idx: usize) {
        let child = PRIVATE.load(Relaxed);
        if child == 0 {
            return;
        }
        let pid = sched.processes[idx].pid;
        if pid == child {
            assert_eq!(PUBLICATION.load(Relaxed), child);
            PRIVATE.store(0, Relaxed);
            crate::serial_println!("[VFS-L1C-2] first dispatch selected child={}", pid);
        }
        let waiter = WAITER.load(Relaxed);
        if pid == OBSERVER.load(Relaxed)
            && sched
                .processes
                .iter()
                .any(|process| process.pid == waiter && process.state == ProcessState::BlockedOnIpc)
        {
            OTHER.store(pid, Relaxed);
        }
    }

    #[cfg(feature = "vfs_l1c_r5_test")]
    pub(crate) use accounting::heap_checkpoint;
    #[cfg(feature = "vfs_l1c_file_ipc_test")]
    pub(crate) use accounting::file_ipc_checkpoint;

    pub(crate) fn run(mode: u64) -> Option<u64> {
        #[cfg(feature = "vfs_l1c_parent_lifecycle_test")]
        if matches!(mode, 12..=18) {
            return Some(parent_lifecycle::stage(mode));
        }
        #[cfg(feature = "vfs_l1c_r3_test")]
        if let Some(result) = r3::resume(mode) {
            return Some(result);
        }
        let (idx, exact, uid, gid) = {
            let sched = SCHEDULER.lock();
            let idx = sched.current_process_index().expect("fixture caller");
            let process = &sched.processes[idx];
            if process.pid != 1 || !process.vfs_root_metadata {
                return Some(u64::MAX);
            }
            (idx, process_identity(process), process.uid, process.gid)
        };
        let mut fixture = CALL.lock();
        let initial = fixture.is_none();
        #[cfg(feature = "vfs_l1c_r4_test")]
        if initial && mode == 4 {
            assert!(HELD.lock().is_none());
            crate::serial_println!("[VFS-R4] A before construction adapter=empty");
        }
        #[cfg(feature = "vfs_l1c_r5_test")]
        let baseline = initial.then(|| accounting::AllocatorSample::read_baseline(&SCHEDULER.lock(), mode));
        let child = if initial {
            let child = pending::construct(
                parent(exact),
                Image {
                    name: "vfs-l1c-private",
                    bytes: {
                        #[cfg(feature = "vfs_l1c_r3_test")]
                        let bytes = (mode == 8).then_some(r3::IMAGE);
                        #[cfg(not(feature = "vfs_l1c_r3_test"))]
                        let bytes: Option<&[u8]> = None;
                        bytes.unwrap_or(if cfg!(feature = "vfs_l1c_2_test") {
                            crate::SUNLIGHT_UTILS_ELF_BYTES
                        } else {
                            crate::SUNLIGHT_TRUE_ELF_BYTES
                        })
                    },
                    argv: if cfg!(feature = "vfs_l1c_r3_test") && mode == 8 {
                        &[b"vfs-r3-stat", b"/tmp"]
                    } else if cfg!(feature = "vfs_l1c_2_test") {
                        &[b"stat", b"/tmp"]
                    } else {
                        &[b"true"]
                    },
                    envp: &[],
                    uid,
                    gid,
                    stdio: [None; 3],
                },
            )
            .expect("private fixture image");
            let identity = child.identity().unwrap();
            Some((child, identity))
        } else {
            None
        };
        let mut sched = SCHEDULER.lock();
        if let Some((child, identity)) = child {
            MODE.store(mode as usize, Relaxed);
            PUBLICATION.store(0, Relaxed);
            #[cfg(feature = "vfs_l1c_r5_test")]
            let constructed = accounting::AllocatorSample::read(&sched);
            let mut pmm = crate::PMM.lock();
            let mut caps = CAP_BROKER.lock();
            let claim = stage(idx, identity, &mut sched, &caps).unwrap_or_else(|failure| {
                let waiter = &sched.processes[idx];
                let runtime = RUNTIME.lock();
                panic!(
                    "stage fixture: {:?} state={:?} call={} reply={} spawn={} chdir={} root={}",
                    failure, waiter.state, waiter.pending_call.is_some(),
                    waiter.ipc_reply_target.is_some(),
                    runtime.spawns.iter().flatten().any(|call| call.parent.caller == exact),
                    runtime.chdirs.iter().flatten().any(|call| call.waiter == exact),
                    runtime.publications.iter().flatten().any(|call| call.identity.caller == exact)
                )
            });
            #[cfg(feature = "vfs_l1c_r5_test")]
            accounting::stage(&RUNTIME.lock(), &sched, claim, mode, baseline.unwrap(), constructed);
            let hhdm = x86_64::VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
            let observer = crate::process::spawn::spawn_from_path(
                "/bin/true",
                &[],
                &mut pmm,
                &mut sched,
                &mut caps,
                hhdm,
                uid,
                gid,
            )
            .expect("spawn independent runnable fixture observer");
            if cfg!(feature = "vfs_l1c_2_test") && mode != 0 {
                let index = sched.process_index_by_pid(observer).unwrap();
                sched.processes[index].child_wait = crate::process::ChildWait::Pending;
            }
            WAITER.store(exact.pid as usize, Relaxed);
            OBSERVER.store(observer, Relaxed);
            OBSERVER_EXITED.store(0, Relaxed);
            *REQUEST.lock() = Some((claim.registration, claim.request_id));
            OTHER.store(0, Relaxed);
            PRIVATE.store(identity.ticket.pid, Relaxed);
            *fixture = Some((child, claim));
            crate::serial_println!("[VFS-L1C-2B] acquire child={} retain={} epoch={} mode={}",
                identity.ticket.pid, claim.request_id, claim.registration.epoch, mode);
        }
        let claim = fixture.as_ref().unwrap().1;
        let pid = claim.child.ticket.pid;
        assert!(sched.process_index_by_pid(pid).is_none());
        assert!(sched.processes.iter().all(|process| process.pid != pid));
        #[cfg(feature = "vfs_l1c_r2_test")]
        if r2_mode() && EXIT_REQUESTED.load(Relaxed) != 0 {
            if EXIT_PROVEN.load(Relaxed) == 0 {
                return Some(0);
            }
            assert_eq!(EXIT_PROVEN.load(Relaxed), claim.registration.owner.pid as usize);
            assert_eq!(
                publish(idx, claim, &mut sched, &CAP_BROKER.lock()),
                Err(Error::Identity)
            );
            assert!(sched.pending_children.private(claim.child).is_some());
            assert!(sched.process_index_by_pid(pid).is_none());
            crate::serial_println!(
                "[VFS-R2] rejected child={} retain={} error=Identity private=1",
                pid, claim.request_id
            );
            assert_eq!(
                wait_for_publication(idx, claim, &mut sched),
                Ok(Some(Terminal::AuthorityRetired))
            );
            assert_eq!(wait_for_publication(idx, claim, &mut sched), Err(Error::Identity));
            crate::serial_println!("[VFS-R2] terminal child={} authority_retired=1 continuation=0", pid);
            let (child, _) = fixture.take().unwrap();
            *REQUEST.lock() = None;
            drop(sched);
            child.cancel().expect("reclaim rejected R2 child");
            assert!(child.cancel().is_err());
            drop(child);
            #[cfg(feature = "vfs_l1c_adapter_accounting_test")]
            accounting::dispose(claim);
            let sched = SCHEDULER.lock();
            assert!(sched.pending_children.private(claim.child).is_none());
            assert!(sched.process_index_by_pid(pid).is_none());
            PRIVATE.store(0, Relaxed);
            crate::serial_println!(
                "[VFS-L1C-2B] dispose child={} retain={} owner=reclaimed",
                pid, claim.request_id
            );
            return Some(u64::MAX);
        }
        let result = if cfg!(feature = "vfs_l1c_2_test") {
            wait_for_publication(idx, claim, &mut sched)
        } else {
            wait(idx, claim, &mut sched)
        };
        match result.expect("exact fixture continuation") {
            None => {
                sched.account_and_apply_churn_penalty();
                sched.processes[idx].block_start_tick = sched.global_tick;
                sched.remove_from_ready_queues(idx);
                if initial {
                    crate::serial_println!(
                        "[VFS-L1C-1] parked parent={} child={} retain={} epoch={}",
                        exact.pid,
                        pid,
                        claim.request_id,
                        claim.registration.epoch
                    );
                }
                None
            }
            Some(outcome) => {
                assert!(!initial);
                #[cfg(feature = "vfs_l1c_r3_test")]
                if mode == 8 {
                    return Some(r3::publish_child(idx, claim, outcome, &mut sched));
                }
                #[cfg(feature = "vfs_l1c_r2_test")]
                if r2_mode() {
                    let Terminal::Retained(receipt) = outcome else {
                        panic!("R2 registration failed: {:?}", outcome);
                    };
                    let service = claim.registration.owner.pid as usize;
                    let observer = OBSERVER.load(Relaxed);
                    assert_eq!(OTHER.load(Relaxed), observer);
                    {
                        let caps = CAP_BROKER.lock();
                        let mut runtime = RUNTIME.lock();
                        assert_eq!(live_registration(&mut runtime, &sched, &caps), Some(claim.registration));
                        let call = runtime.spawn_request(claim.registration, claim.request_id)
                            .expect("retained publication continuation");
                        assert!(!call.parked);
                        assert!(runtime.spawn_live(call, &sched));
                        assert_eq!(receipt.registration(), claim.registration);
                        assert_eq!(receipt.request_id(), claim.request_id);
                        assert_eq!(receipt.scope().caller, caller(claim.child));
                        assert!(runtime.namespaces.lookup(call.child).is_err());
                        let remote = runtime.remote.iter().flatten()
                            .find(|remote| remote.registration == claim.registration && !remote.retired)
                            .expect("active registration row");
                        assert!(remote.ownership.holds(receipt));
                        assert_ne!(remote.delivered, Some(claim.request_id));
                        assert!(remote.local.iter().flatten().all(|lease| lease.id() != claim.request_id));
                        assert!(remote.callers.iter().flatten().any(|entry| entry.caller == caller(claim.child)));
                    }
                    assert!(sched.pending_children.private(claim.child).is_some());
                    crate::serial_println!(
                        "[VFS-R2] consumed service={} child={} retain={} epoch={} observer={} held_receipt=1 binding=0 private=1",
                        service, pid, claim.request_id, claim.registration.epoch, observer
                    );
                    EXIT_REQUESTED.store(service, Relaxed);
                    assert!(sched.terminate_process_by_pid(service, 0, "vfs-r2-retained-registration"));
                    return Some(observer as u64);
                }
                let (child, _) = fixture.take().unwrap();
                assert!(sched.pending_children.private(claim.child).is_some());
                if cfg!(feature = "vfs_l1c_2_test")
                    && (mode == 0 || (cfg!(feature = "vfs_l1c_adapter_accounting_test") && mode == 3))
                    && matches!(outcome, Terminal::Retained(_))
                {
                    assert_eq!(OTHER.load(Relaxed), OBSERVER.load(Relaxed));
                    *REQUEST.lock() = None;
                    let published = publish(idx, claim, &mut sched, &CAP_BROKER.lock());
                    let Ok(published) = published else {
                        #[cfg(feature = "vfs_l1c_adapter_accounting_test")]
                        if mode == 3 {
                            assert_eq!(published, Err(Error::Preparation));
                            crate::serial_println!(
                                "[VFS-ADAPTER] preparation rejected child={} retain={} authority=live",
                                pid, claim.request_id
                            );
                        }
                        drop(sched);
                        child.cancel().expect("reclaim failed publication");
                        drop(child);
                        let mut sched = SCHEDULER.lock();
                        assert!(matches!(
                            wait_for_publication(idx, claim, &mut sched),
                            Ok(Some(Terminal::Cancelled | Terminal::AuthorityRetired))
                                | Err(Error::Identity)
                        ));
                        #[cfg(feature = "vfs_l1c_adapter_accounting_test")]
                        if mode == 3 {
                            assert!(sched.pending_children.private(claim.child).is_none());
                            assert!(sched.process_index_by_pid(pid).is_none());
                            accounting::dispose(claim);
                        }
                        PRIVATE.store(0, Relaxed);
                        crate::serial_println!("[VFS-L1C-2B] dispose child={} retain={} owner=reclaimed", pid, claim.request_id);
                        #[cfg(feature = "vfs_l1c_adapter_accounting_test")]
                        if mode == 3 {
                            return Some(OBSERVER.load(Relaxed) as u64);
                        }
                        return Some(u64::MAX);
                    };
                    assert_eq!(published, pid);
                    #[cfg(feature = "vfs_l1c_adapter_accounting_test")]
                    assert_ne!(mode, 3);
                    #[cfg(feature = "vfs_l1c_r5_test")]
                    replay_published(idx, claim, &mut sched);
                    drop(sched);
                    assert!(child.cancel().is_err());
                    #[cfg(feature = "vfs_l1c_r5_test")]
                    {
                        drop(child);
                        accounting::dispose(claim);
                    }
                    crate::serial_println!("[VFS-L1C-2B] dispose child={} retain={} owner=scheduler", pid, claim.request_id);
                    return Some(published as u64);
                }
                if cfg!(feature = "vfs_l1c_2_test") && mode != 0 {
                    assert_eq!(OTHER.load(Relaxed), OBSERVER.load(Relaxed));
                    assert!(matches!(
                        (mode, outcome),
                        (1 | 4, Terminal::Cancelled) | (2, Terminal::Retained(_))
                            | (5 | 6, Terminal::AuthorityRetired)
                            | (11, Terminal::Refused(_))
                    ));
                    #[cfg(feature = "vfs_l1c_adapter_accounting_test")]
                    if let (11, Terminal::Refused(reason)) = (mode, outcome) {
                        crate::serial_println!(
                            "[VFS-ADAPTER] service refused child={} retain={} reason={:?}",
                            pid, claim.request_id, reason
                        );
                    }
                    #[cfg(feature = "vfs_l1c_r1_test")]
                    if mode == 5 || before_acquisition_mode() {
                        assert_eq!(EXIT_PROVEN.load(Relaxed), claim.registration.owner.pid as usize);
                        let discarded = usize::from(HELD.lock().take().is_some());
                        assert_eq!(discarded, usize::from(mode == 5));
                        let runtime = RUNTIME.lock();
                        assert!(runtime.spawn_request(claim.registration, claim.request_id).is_none());
                        crate::serial_println!("[VFS-R1] terminal child={} authority_retired=1 continuation=0 held_copy_discarded={}", pid, discarded);
                    }
                    *REQUEST.lock() = None;
                    drop(sched);
                    #[cfg(feature = "vfs_l1c_adapter_accounting_test")]
                    if let (2, Terminal::Retained(receipt)) = (mode, outcome) {
                        accounting::before_consumed_cancel(&RUNTIME.lock(), claim, receipt);
                    }
                    child.cancel().expect("reclaim cancelled fixture child");
                    assert!(child.cancel().is_err());
                    #[cfg(feature = "vfs_l1c_r5_test")]
                    {
                        drop(child);
                        accounting::dispose(claim);
                    }
                    let mut sched = SCHEDULER.lock();
                    assert!(sched.process_index_by_pid(pid).is_none());
                    assert!(sched.pending_children.private(claim.child).is_none());
                    if mode == 2 {
                        assert_eq!(
                            wait_for_publication(idx, claim, &mut sched),
                            Ok(Some(Terminal::Cancelled))
                        );
                    }
                    assert!(publish(idx, claim, &mut sched, &CAP_BROKER.lock()).is_err());
                    crate::serial_println!(
                        "[VFS-L1C-2] cancelled mode={} child={} other={} reclaimed=1",
                        mode, pid, OTHER.load(Relaxed)
                    );
                    PRIVATE.store(0, Relaxed);
                    crate::serial_println!("[VFS-L1C-2B] dispose child={} retain={} owner=reclaimed", pid, claim.request_id);
                    #[cfg(feature = "vfs_l1c_r4_test")]
                    if mode == 4 {
                        release_reply(claim, &sched);
                    }
                    return Some(OBSERVER.load(Relaxed) as u64);
                }
                PRIVATE.store(0, Relaxed);
                *REQUEST.lock() = None;
                drop(sched);
                child.cancel().expect("reclaim private fixture image");
                assert!(SCHEDULER
                    .lock()
                    .pending_children
                    .private(claim.child)
                    .is_none());
                match outcome {
                    Terminal::Retained(receipt) => {
                        assert_eq!(receipt.scope().caller, caller(claim.child));
                        assert_eq!(receipt.request_id(), claim.request_id);
                        assert_eq!(receipt.registration(), claim.registration);
                        crate::serial_println!("[VFS-L1C-1] retained child={} retain={} other_dispatch={} local_reclaimed=1 unpublished=1",
                            pid, claim.request_id, OTHER.load(Relaxed));
                        let observer = OBSERVER.load(Relaxed);
                        assert_eq!(OTHER.load(Relaxed), observer);
                        Some(observer as u64)
                    }
                    failure => {
                        crate::serial_println!("[VFS-L1C-1] terminal {:?}", failure);
                        Some(u64::MAX)
                    }
                }
            }
        }
    }
}
