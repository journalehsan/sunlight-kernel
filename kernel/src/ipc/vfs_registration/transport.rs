//! Copied service pull channel. Lifecycle locks cover authentication and state
//! transition; user memory copying is performed by the syscall after unlock.
use super::*;
use sunlight_ipc::vfs_rpc::retention::{transport as api, wire::FRAME_BYTES};

impl Runtime {
    /// Initial root selection comes solely from boot image provenance. Derived
    /// callers already have a staged attempt and must never enter this route. Called
    /// under scheduler -> broker -> runtime serialization, before NEXT can
    /// expose work. Caller delivery records retain the one-attempt decision
    /// after refusal as well as success, preventing a failing service from
    /// exhausting generations or spinning on the same caller each poll.
    fn stage_selected_roots(&mut self, registration: Registration, sched: &Scheduler) {
        if self.registry.active() != Some(registration) {
            return;
        }
        for (idx, process) in sched.processes.iter().enumerate() {
            if !process.vfs_root_metadata || !process.vfs_boot_root || process.is_exiting() {
                continue;
            }
            let caller = process_identity(process);
            if [
                caller.pid,
                caller.incarnation,
                caller.address_space_generation,
            ]
            .contains(&0)
            {
                continue;
            }
            let attempted = self.remote.iter().flatten().any(|r| {
                r.registration == registration
                    && r.callers.iter().flatten().any(|c| c.caller == caller)
            });
            if attempted {
                continue;
            }
            let identity = AdmissionIdentity {
                caller,
                uid: process.uid,
                gid: process.gid,
                groups: [],
                registration,
            };
            // Capacity failure leaves no published profile and is retryable
            // on a later service turn. Never replace a live caller's binding.
            let _ = self.queue_initial_namespace(
                identity,
                sunlight_ipc::vfs_rpc::context::LOOKUP_METADATA,
                &sched.processes[sched.shared_process_index(idx)].cwd,
            );
        }
    }

    /// Component staging without a root-policy grant. Preinstalled service
    /// policies can consume it; the root provisioning route must refuse it.
    /// No syscall exposes namespace preparation or policy selection.
    #[allow(dead_code)]
    pub(super) fn queue_namespace(
        &mut self,
        identity: AdmissionIdentity,
        grants: u64,
    ) -> Result<(), NamespaceRetentionError> {
        self.queue_namespace_inner(identity, grants, None).map(|_| ())
    }

    pub(super) fn queue_namespace_inner(
        &mut self,
        identity: AdmissionIdentity,
        grants: u64,
        previous: Option<Scope>,
    ) -> Result<u64, NamespaceRetentionError> {
        let index = self
            .remote
            .iter()
            .position(|r| {
                r.as_ref()
                    .is_some_and(|r| r.registration == identity.registration)
            })
            .ok_or(NamespaceRetentionError::Ownership(OwnershipError::State))?;
        let slot = self.remote[index]
            .as_ref()
            .unwrap()
            .unsent
            .iter()
            .position(Option::is_none)
            .ok_or(NamespaceRetentionError::Ownership(OwnershipError::Capacity))?;
        let frame = self.prepare_namespace_inner(identity, grants, previous)?;
        let Frame::Retain { request_id, .. } = frame else {
            unreachable!("namespace preparation always returns a retain frame");
        };
        self.remote[index].as_mut().unwrap().unsent[slot] = Some(frame);
        Ok(request_id)
    }

    /// Root-cwd component fixture. Production snapshots the selected caller's
    /// shared cwd through `queue_initial_namespace` instead.
    #[allow(dead_code)]
    pub(super) fn queue_root_namespace(
        &mut self,
        identity: AdmissionIdentity,
        grants: u64,
    ) -> Result<(), NamespaceRetentionError> {
        self.queue_initial_namespace(identity, grants, "/")
    }

    fn queue_initial_namespace(
        &mut self,
        identity: AdmissionIdentity,
        grants: u64,
        cwd: &str,
    ) -> Result<(), NamespaceRetentionError> {
        api::encode_cwd(cwd)
            .map_err(|_| NamespaceRetentionError::Ownership(OwnershipError::State))?;
        let mut directory = alloc::string::String::new();
        directory
            .try_reserve_exact(cwd.len())
            .map_err(|_| NamespaceRetentionError::Ownership(OwnershipError::Capacity))?;
        directory.push_str(cwd);
        let id = self.queue_namespace_inner(identity, grants, None)?;
        let remote = self
            .remote
            .iter_mut()
            .flatten()
            .find(|r| r.registration == identity.registration)
            .unwrap();
        let staged = remote
            .local
            .iter_mut()
            .flatten()
            .find(|lease| lease.id() == id)
            .unwrap();
        let LocalLease::Preparing {
            root_policy,
            initial_cwd,
            ..
        } = staged
        else {
            unreachable!()
        };
        *root_policy = true;
        *initial_cwd = Some(directory);
        Ok(())
    }

    fn authorize_root(
        &self,
        registration: Registration,
        bytes: &[u8],
        sched: &Scheduler,
    ) -> Result<&str, IpcError> {
        let Frame::Retain {
            registration: claimed,
            scope,
            request_id,
        } = Frame::decode(bytes).map_err(|_| IpcError::InvalidCapability)?
        else {
            return Err(IpcError::InvalidCapability);
        };
        if claimed != registration || self.registry.active() != Some(registration) {
            return Err(IpcError::InvalidCapability);
        }
        let remote = self
            .remote
            .iter()
            .flatten()
            .find(|r| r.registration == registration && !r.retired)
            .ok_or(IpcError::InvalidCapability)?;
        if remote.delivered != Some(request_id)
            || remote.callers.iter().flatten().any(CallerDelivery::pending)
        {
            return Err(IpcError::InvalidCapability);
        }
        let (identity, directory) = remote
            .local
            .iter()
            .flatten()
            .find_map(|l| match l {
                LocalLease::Preparing {
                    id,
                    identity,
                    scope: expected,
                    root_policy: true,
                    initial_cwd: Some(directory),
                    ..
                } if *id == request_id && *expected == scope => Some((identity, directory)),
                _ => None,
            })
            .ok_or(IpcError::InvalidCapability)?;
        if !sched.processes.iter().enumerate().any(|(idx, p)| {
            process_identity(p) == identity.caller
                && !p.is_exiting()
                && p.vfs_root_metadata
                && p.vfs_boot_root
                && p.uid == identity.uid
                && p.gid == identity.gid
                && sched.processes[sched.shared_process_index(idx)].cwd == *directory
        }) {
            return Err(IpcError::InvalidCapability);
        }
        Ok(directory)
    }

    pub(super) fn next_control(
        &mut self,
        registration: Registration,
    ) -> Result<Option<Frame>, IpcError> {
        let Runtime {
            remote,
            file_publications,
            ..
        } = self;
        let remote = remote
            .iter_mut()
            .flatten()
            .find(|r| r.registration == registration)
            .ok_or(IpcError::InvalidCapability)?;
        // Retry the same barrier until its exact delivered acknowledgement.
        // No later retain crosses an unacknowledged exit/exec event. A retain
        // fetched before retirement may still reply and own its compensation.
        if let Some(caller) = remote
            .callers
            .iter_mut()
            .flatten()
            .filter(|c| c.retired && !c.metadata_acked)
            .min_by_key(|c| c.id)
        {
            caller.delivered = true;
            return Ok(Some(Frame::CallerRetired {
                registration,
                caller: caller.caller,
                event_id: caller.id,
            }));
        }
        // File lifetime delivery survives metadata acknowledgement. It uses a
        // distinct opcode/ack so credential withdrawal cannot close files.
        // Surviving non-CLOEXEC descriptors must be rekeyed first: the barrier
        // waits for both the publication gap and every pending transfer reply.
        let transfer_pending = |caller| {
            file_publications.iter().flatten().any(|publication| {
                publication.op == sunlight_ipc::vfs_rpc::file_transport::TRANSFER
                    && publication.identity.caller == caller
                    && !publication.replied
            })
        };
        if let Some(caller) = remote
            .callers
            .iter_mut()
            .flatten()
            .filter(|c| c.file_retired && !c.file_transfer_pending)
            .filter(|c| !transfer_pending(c.caller))
            .min_by_key(|c| c.id)
        {
            caller.file_delivered = true;
            return Ok(Some(Frame::FileOwnerRetired {
                registration,
                caller: caller.caller,
                event_id: caller.id,
            }));
        }
        // Allocation order is the replay order, regardless of slot reuse.
        if let Some((slot, id)) = remote
            .unsent
            .iter()
            .enumerate()
            // Copies happen after lifecycle unlock. A second service thread
            // must not process a higher retain ID before the first completes.
            .filter(|_| remote.delivered.is_none())
            .filter_map(|(i, f)| {
                if let Some(Frame::Retain { request_id, .. }) = f {
                    Some((i, *request_id))
                } else {
                    None
                }
            })
            .min_by_key(|(_, id)| *id)
        {
            #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_1_test"))]
            if spawn::fixture::awaiting_observer(registration, id) {
                return Ok(None);
            }
            // Reserve delivery bookkeeping before consuming the single-send
            // record. Copy failure does not authorize retain retransmission.
            remote.delivered = Some(id);
            return Ok(remote.unsent[slot].take());
        }
        Ok(remote.ownership.releases().next())
    }

    pub(super) fn control_reply(
        &mut self,
        authenticated: Registration,
        bytes: &[u8],
        sched: &Scheduler,
    ) -> Result<(), IpcError> {
        let decoded = Frame::decode(bytes).map_err(|_| IpcError::InvalidCapability)?;
        if let Frame::CallerRetiredAck {
            registration,
            caller,
            event_id,
        }
        | Frame::FileOwnerRetiredAck {
            registration,
            caller,
            event_id,
        } = decoded
        {
            let file_ack = matches!(decoded, Frame::FileOwnerRetiredAck { .. });
            if registration != authenticated {
                return Err(IpcError::InvalidCapability);
            }
            let remote = self
                .remote
                .iter_mut()
                .flatten()
                .find(|r| r.registration == authenticated)
                .ok_or(IpcError::InvalidCapability)?;
            let slot = remote
                .callers
                .iter()
                .position(|c| {
                    c.as_ref().is_some_and(|c| {
                        c.caller == caller
                            && c.id == event_id
                            && if file_ack {
                                c.file_retired && c.file_delivered && c.metadata_acked
                            } else {
                                c.retired && c.delivered && !c.metadata_acked
                            }
                    })
                })
                .ok_or(IpcError::InvalidCapability)?;
            if file_ack {
                remote.callers[slot] = None;
            } else {
                // Keep the lifetime reservation until independent exit/exec.
                remote.callers[slot].as_mut().unwrap().metadata_acked = true;
            }
            self.collect_remote();
            return Ok(());
        }
        let id = match decoded {
            Frame::Retained(r) | Frame::Released(r) => r.request_id(),
            Frame::Refused { request, .. } => {
                let Frame::Retain { request_id, .. } = request.request() else {
                    unreachable!()
                };
                request_id
            }
            _ => return Err(IpcError::InvalidCapability),
        };
        if matches!(decoded, Frame::Released(_)) {
            return self
                .released(authenticated, id, bytes)
                .map_err(|_| IpcError::InvalidCapability);
        }
        let remote = self
            .remote
            .iter()
            .flatten()
            .find(|r| r.registration == authenticated)
            .ok_or(IpcError::InvalidCapability)?;
        if remote.delivered != Some(id)
            || self.chdirs.iter().flatten().any(|c| {
                c.id == id && c.identity.registration == authenticated && c.result.is_none()
            })
        {
            return Err(IpcError::InvalidCapability);
        }
        if self.spawn_request(authenticated, id).is_some() {
            return self.reply_spawn(authenticated, id, decoded, bytes, sched);
        }
        let live = remote
            .local
            .iter()
            .flatten()
            .find_map(|local| match local {
                LocalLease::Preparing {
                    id: pending,
                    identity,
                    root_policy,
                    initial_cwd,
                    ..
                } if *pending == id => Some((*identity, *root_policy, initial_cwd.as_deref())),
                _ => None,
            })
            .filter(|(identity, root_policy, directory)| {
                sched.processes.iter().enumerate().any(|(idx, p)| {
                    process_identity(p) == identity.caller
                        && !p.is_exiting()
                        && (!*root_policy || (p.vfs_root_metadata && p.vfs_boot_root))
                        && p.uid == identity.uid
                        && p.gid == identity.gid
                        && (directory.is_none()
                            || *directory
                                == Some(
                                    sched.processes[sched.shared_process_index(idx)]
                                        .cwd
                                        .as_str(),
                                ))
                })
            })
            .map(|(identity, _, _)| identity);
        let live = if remote.local.iter().flatten().any(|l| {
            matches!(l,
            LocalLease::Preparing { id: pending, derivation: Some(_), .. } if *pending == id)
        }) {
            let request = remote
                .local
                .iter()
                .flatten()
                .find_map(|l| match l {
                    LocalLease::Preparing {
                        id: pending, scope, ..
                    } if *pending == id => Some(Frame::Retain {
                        registration: authenticated,
                        scope: *scope,
                        request_id: id,
                    }),
                    _ => None,
                })
                .unwrap()
                .encode()
                .unwrap();
            live.filter(|_| {
                self.authorize_derivation(authenticated, &request, sched)
                    .is_ok()
            })
        } else {
            live
        };
        let result = match decoded {
            Frame::Retained(_) => self
                .install_retained(authenticated, id, bytes, live)
                .map(|_| ()),
            Frame::Refused { .. } => self
                .refused(authenticated, id, bytes)
                .map(|_| ())
                .map_err(NamespaceRetentionError::Ownership),
            _ => unreachable!(),
        };
        #[cfg(not(test))]
        if result.is_ok() && matches!(decoded, Frame::Retained(_)) {
            if let Some(identity) = live.filter(|identity| {
                sched
                    .processes
                    .iter()
                    .any(|p| p.vfs_root_metadata && process_identity(p) == identity.caller)
            }) {
                if self.namespaces.lookup(identity).is_ok() {
                    crate::serial_println!(
                        "[VFS-RPC] boot root policy installed caller={} epoch={} transport=copy",
                        identity.caller.pid,
                        authenticated.epoch
                    );
                }
            }
        }
        if let Some(remote) = self
            .remote
            .iter_mut()
            .flatten()
            .find(|r| r.registration == authenticated)
        {
            // Invalid responses leave preparation intact. Installation errors
            // that already own compensation must stop accepting retain replies.
            if !remote.local.iter().flatten().any(|l| {
                matches!(l,
                LocalLease::Preparing { id: pending, .. } if *pending == id)
            }) {
                remote.delivered = None;
            }
        }
        // Settling a derivation drops its source reference; publish any newly
        // drained source lease release before the next service pull.
        self.reconcile_namespaces();
        result.map_err(|_| IpcError::InvalidCapability)
    }
}

pub(super) fn authenticate(
    runtime: &mut Runtime,
    pid: usize,
    epoch: u64,
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<Registration, IpcError> {
    let process = sched
        .processes
        .iter()
        .find(|p| p.pid == pid && !p.is_exiting() && p.trusted_vfs_service)
        .ok_or(IpcError::InvalidCapability)?;
    let caller = process_identity(process);
    let registration = if epoch == 0 {
        live_registration(runtime, sched, caps)
    } else {
        runtime
            .remote
            .iter()
            .flatten()
            .find(|r| r.registration.epoch == epoch)
            .map(|r| r.registration)
    }
    .ok_or(IpcError::InvalidCapability)?;
    if !owner_matches(registration.owner, caller)
        || !u32::try_from(registration.owner.endpoint)
            .ok()
            .is_some_and(|ep| caps.endpoint_owner(ep) == Some(pid))
    {
        return Err(IpcError::InvalidCapability);
    }
    Ok(registration)
}

/// Copied file-channel pull/reply. The service authenticates through the same
/// independently captured registration as the retention channel. The buffer is
/// copied to and from user memory by the syscall before reaching this point.
pub(crate) enum FileControl {
    Empty,
    Filled,
    Acked,
}

pub(crate) fn file_control(
    pid: usize,
    operation: u64,
    epoch: u64,
    bytes: &mut [u8; sunlight_ipc::vfs_rpc::file_transport::FRAME_BYTES],
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<FileControl, IpcError> {
    use sunlight_ipc::vfs_rpc::file_transport as file;
    let mut runtime = RUNTIME.lock();
    let registration = authenticate(&mut runtime, pid, epoch, sched, caps)?;
    #[cfg(any(feature = "vfs_c1e2_test", feature = "vfs_c2d_candidate", feature = "vfs_c2e_candidate"))]
    if operation == file::NEXT {
        runtime.revalidate_storage_files(sched);
    }
    match operation {
        file::NEXT => match runtime.next_file(registration, bytes) {
            Ok(Some(_)) => Ok(FileControl::Filled),
            Ok(None) => Ok(FileControl::Empty),
            Err(_) => Err(IpcError::InvalidCapability),
        },
        file::REPLY => runtime
            .reply_file(registration, bytes, sched)
            .map(|_| FileControl::Acked)
            .map_err(|_| IpcError::InvalidCapability),
        _ => Err(IpcError::InvalidCapability),
    }
}

/// Hold scheduler -> broker throughout. The PID comes from syscall entry.
/// Replies are copied into immutable kernel memory before reaching this call.
pub(crate) fn control(
    pid: usize,
    operation: u64,
    epoch: u64,
    input: &[u8],
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<Option<[u8; FRAME_BYTES]>, IpcError> {
    let mut runtime = RUNTIME.lock();
    control_runtime(&mut runtime, pid, operation, epoch, input, sched, caps)
}

pub(crate) fn initial_cwd(
    pid: usize,
    epoch: u64,
    input: &[u8],
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<[u8; api::CWD_BYTES], IpcError> {
    if epoch == 0 {
        return Err(IpcError::InvalidCapability);
    }
    let mut runtime = RUNTIME.lock();
    let registration = authenticate(&mut runtime, pid, epoch, sched, caps)?;
    api::encode_cwd(runtime.authorize_root(registration, input, sched)?)
        .map_err(|_| IpcError::InvalidCapability)
}

fn control_runtime(
    runtime: &mut Runtime,
    pid: usize,
    operation: u64,
    epoch: u64,
    input: &[u8],
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<Option<[u8; FRAME_BYTES]>, IpcError> {
    if (operation == api::SNAPSHOT && epoch != 0)
        || (operation != api::SNAPSHOT && epoch == 0)
        || (!api::has_input(operation) && !input.is_empty())
    {
        return Err(IpcError::InvalidCapability);
    }
    let registration = authenticate(runtime, pid, epoch, sched, caps)?;
    match operation {
        api::SNAPSHOT => {
            runtime.stage_selected_roots(registration, sched);
            let mut bytes = [0; FRAME_BYTES];
            bytes[..api::REGISTRATION_BYTES]
                .copy_from_slice(&api::encode_registration(registration));
            Ok(Some(bytes))
        }
        api::STATUS => {
            let retired = runtime
                .remote
                .iter()
                .flatten()
                .find(|r| r.registration == registration)
                .ok_or(IpcError::InvalidCapability)?
                .retired;
            let mut bytes = [0; FRAME_BYTES];
            bytes[..api::REGISTRATION_BYTES]
                .copy_from_slice(&api::encode_registration(registration));
            bytes[api::REGISTRATION_BYTES..api::STATUS_BYTES]
                .copy_from_slice(&u64::from(retired).to_le_bytes());
            Ok(Some(bytes))
        }
        api::NEXT => runtime
            .next_control(registration)?
            .map(|f| f.encode().map_err(|_| IpcError::InvalidCapability))
            .transpose(),
        api::DERIVE_SOURCE => {
            #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_adapter_accounting_test"))]
            if spawn::fixture::withhold_source(registration, input) {
                return Err(IpcError::InvalidCapability);
            }
            Frame::Retained(runtime.authorize_derivation(registration, input, sched)?)
                .encode()
                .map(Some)
                .map_err(|_| IpcError::InvalidCapability)
        }
        api::AUTHORIZE_ROOT => {
            if runtime.authorize_root(registration, input, sched)? != "/" {
                return Err(IpcError::InvalidCapability);
            }
            Ok(None)
        }
        api::REPLY => {
            #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_r5_test"))]
            spawn::fixture::observe_reply(runtime, registration, input, sched);
            #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_r4_test"))]
            if spawn::fixture::hold_reply(registration, input) {
                return Ok(None);
            }
            runtime.control_reply(registration, input, sched)?;
            #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_r5_test"))]
            spawn::fixture::observe_release(registration, input);
            Ok(None)
        }
        _ => Err(IpcError::InvalidCapability),
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
