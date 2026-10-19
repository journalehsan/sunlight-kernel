//! Cooperative D2 adapter. Only scheduled deadlines initiate quiescing. The
//! existing D1 process supervisor supplies reaping observations.
use super::*;
use sunlight_ipc::demand as wire;
use sunlightd::idle::{Deadlines, Key, Leases, Owner};

pub(super) struct IdleManager {
    pub timers: Deadlines,
    leases: Leases,
    // unload count, last private frames, last page tables, PMM free frames
    receipts: [[u64; 4]; MAX_UNITS],
}

#[cfg(test)]
mod tests {
    use super::*;
    fn table() -> ServiceTable {
        let mut t = ServiceTable::new();
        t.add(parse_service_unit(b"[Service]\nExecStart=/sbin/provider\nActivation=on-demand\nType=notify\nUnloadWhenIdle=yes\n").unwrap()).unwrap();
        t.add(
            parse_service_unit(
                b"[Unit]\nRequires=provider.service\n[Service]\nExecStart=/sbin/consumer\n",
            )
            .unwrap(),
        )
        .unwrap();
        t
    }
    #[test]
    fn dependencies_hold_starting_live_and_unreaped_failed_consumers() {
        let mut t = table();
        assert!(!dependent(&t, 0));
        t.get_mut(1).unwrap().activation.request(0).unwrap();
        assert!(dependent(&t, 0));
        t.get_mut(1).unwrap().activation.spawned(8, 12, 10);
        t.get_mut(1).unwrap().activation.fail(wire::DIED, 20);
        assert!(dependent(&t, 0));
        t.get_mut(1).unwrap().activation.runtime = None;
        assert!(!dependent(&t, 0));
        t.get_mut(1).unwrap().mark_running(8, 10);
        assert!(dependent(&t, 0));
        t.get_mut(1).unwrap().mark_stopped();
        assert!(!dependent(&t, 0));
    }
    #[test]
    fn cooperative_stop_and_restart_never_enter_legacy_force_path() {
        let mut t = table();
        t.get_mut(0).unwrap().activation.request(0).unwrap();
        t.get_mut(0).unwrap().activation.spawned(7, 11, 0);
        t.get_mut(0).unwrap().mark_running(7, 0);
        for op in [2, 3] {
            let r = handle_control_message(
                &wire::request(op, "provider").unwrap(),
                &mut t,
                CapabilityToken::INVALID,
            );
            assert_eq!(r.label, REPLY_ERR);
            assert!(t.get(0).unwrap().activation.runtime.is_some());
            assert_eq!(t.get(0).unwrap().state.pid(), Some(7));
        }
    }
}

pub(super) fn is_idle_op(op: u64) -> bool {
    matches!(
        op,
        wire::IDLE_REPORT | wire::LEASE_ACQUIRE | wire::LEASE_RELEASE
    )
}

fn dependent(services: &ServiceTable, provider: usize) -> bool {
    let name = binary_name_of(&services.get(provider).unwrap().unit.exec_start);
    services.dependencies.busy(sunlightd::dependency::Instance {
        unit: provider, generation: services.get(provider).unwrap().activation.generation,
    }) || (0..services.count).any(|i| {
        let e = services.get(i).unwrap();
        e.unit.activation != ActivationPolicy::OnDemand
            && (e.activation.phase == Phase::Starting
            || e.activation.runtime.is_some()
            || e.state.may_still_be_alive())
            && e.unit
                .requires
                .iter()
                .chain(e.unit.after.iter())
                .any(|dep| dep.strip_suffix(".service").unwrap_or(dep) == name)
    })
}

impl IdleManager {
    pub(super) fn new() -> Self {
        Self {
            timers: Deadlines::default(),
            leases: Leases::default(),
            receipts: [[0; 4]; MAX_UNITS],
        }
    }
    fn pinned(&self, services: &ServiceTable, idx: usize) -> bool {
        self.leases
            .busy(idx, services.get(idx).unwrap().activation.generation)
            || dependent(services, idx)
    }
    pub(super) fn consider(&mut self, services: &mut ServiceTable, idx: usize) {
        let pinned = self.pinned(services, idx);
        let e = services.get_mut(idx).unwrap();
        if let Some(at) = e
            .activation
            .nominate_idle(e.unit.idle, monotonic_millis(), pinned)
        {
            self.timers.set(Key::Unit(idx), at);
            serial_println!(
                "[D2] candidate unit={} generation={} deadline={}",
                binary_name_of(&e.unit.exec_start),
                e.activation.generation,
                at
            );
        }
    }
    /// Triggered by dependency changes, never by a periodic D2 table scan.
    pub(super) fn dependencies_changed(&mut self, services: &mut ServiceTable) {
        for idx in 0..services.count {
            self.consider(services, idx);
        }
    }
    pub(super) fn metrics(&self, idx: usize) -> IpcMsg {
        let r = self.receipts[idx];
        IpcMsg::with_label(wire::OK)
            .word(0, r[0])
            .word(1, r[1])
            .word(2, r[2])
            .word(3, r[3])
    }
    pub(super) fn handle(&mut self, msg: &IpcMsg, services: &mut ServiceTable) -> IpcMsg {
        let Some(caller) = sunlight_ipc::session_query_process(msg.badge) else {
            return wire::error(wire::STALE);
        };
        let owner = Owner {
            pid: caller.pid as u32,
            generation: caller.generation,
        };
        if msg.label == wire::LEASE_RELEASE {
            if msg.word_count != 2 {
                return wire::error(wire::STALE);
            }
            let Some(lease) = self
                .leases
                .slots
                .iter()
                .flatten()
                .find(|l| l.id == msg.words[0])
                .copied()
            else {
                return wire::error(wire::STALE);
            };
            match self.leases.release(msg.words[0], msg.words[1], owner) {
                Ok(slot) => {
                    self.timers.cancel(Key::Lease(slot));
                    self.consider(services, lease.unit);
                    return IpcMsg::with_label(wire::OK);
                }
                Err(code) => return wire::error(code),
            }
        }
        if msg.label == wire::LEASE_ACQUIRE {
            if msg.word_count != 4 {
                return wire::error(wire::STALE);
            }
            let Some(idx) = (0..services.count).find(|i| {
                let e = services.get(*i).unwrap();
                e.unit.idle.unload_when_idle
                    && e.activation.generation == msg.words[0]
                    && matches!(e.activation.phase, Phase::Ready | Phase::IdleCandidate)
                    && e.activation.runtime.is_some_and(|r| {
                        r.pid as u64 == msg.words[1] && r.process_generation == msg.words[2]
                    })
            }) else {
                return wire::error(wire::STALE);
            };
            let e = services.get(idx).unwrap();
            let r = e.activation.runtime.unwrap();
            if !demand_runtime::live(r) || demand_runtime::endpoint(services, idx) != r.endpoint {
                return wire::error(wire::STALE);
            }
            match self.leases.acquire(
                idx,
                e.activation.generation,
                r,
                owner,
                monotonic_millis(),
                msg.words[3],
            ) {
                Ok((slot, l)) => {
                    self.timers.cancel(Key::Unit(idx));
                    services.get_mut(idx).unwrap().activation.phase = Phase::Ready;
                    self.timers.set(Key::Lease(slot), l.expires);
                    return IpcMsg::with_label(wire::OK)
                        .word(0, l.id)
                        .word(1, l.expires);
                }
                Err(code) => return wire::error(code),
            }
        }
        // The kernel supplies the badge. A client, another service, or a
        // restarted process cannot modify this generation's work declaration.
        let Some(idx) = (0..services.count).find(|i| {
            services
                .get(*i)
                .unwrap()
                .activation
                .runtime
                .is_some_and(|r| r.pid == owner.pid && r.process_generation == owner.generation)
        }) else {
            return wire::error(wire::STALE);
        };
        let e = services.get_mut(idx).unwrap();
        if !e.unit.idle.unload_when_idle
            || msg.word_count != 3
            || msg.words[0] != e.activation.generation
            || !matches!(e.activation.phase, Phase::Ready | Phase::IdleCandidate)
        {
            return wire::error(wire::STALE);
        }
        e.activation.idle_reported = msg.words[1] == 0 && msg.words[2] == 0;
        if !e.activation.idle_reported {
            e.activation.phase = Phase::Ready;
            self.timers.cancel(Key::Unit(idx));
        }
        self.consider(services, idx);
        IpcMsg::with_label(wire::OK)
    }

    /// Receipt observation is serialized with activation replies. Keep all
    /// identities until this exact incarnation is fully reaped.
    pub(super) fn reaped(
        &mut self,
        services: &mut ServiceTable,
        idx: usize,
        r: sunlightd::activation::Runtime,
        frames: u64,
        pts: u64,
        free: u64,
        exit_code: i32,
    ) {
        self.timers.cancel(Key::Unit(idx));
        for slot in 0..self.leases.slots.len() {
            if self.leases.slots[slot].is_some_and(|l| {
                l.unit == idx
                    && l.runtime.pid == r.pid
                    && l.runtime.process_generation == r.process_generation
            }) {
                self.leases.slots[slot] = None;
                self.timers.cancel(Key::Lease(slot));
            }
        }
        let e = services.get_mut(idx).unwrap();
        if exit_code == 0 && e.activation.unloaded(r).is_ok() {
            self.receipts[idx] = [self.receipts[idx][0].saturating_add(1), frames, pts, free];
            e.mark_stopped();
            serial_println!("[D2] unloaded unit={} generation={} pid={} private_frames={} page_tables={} free_frames={} shutdown_ms={}",
                binary_name_of(&e.unit.exec_start),e.activation.generation,r.pid,frames,pts,free,
                monotonic_millis().saturating_sub(e.activation.quiesce_at));
        } else {
            if e.activation.phase != Phase::Failed {
                e.activation.fail(wire::DIED, monotonic_millis());
            }
            e.activation.runtime = None;
            e.mark_failed(-1, monotonic_millis());
        }
        // Releasing only after recording the receipt permits safe slot reuse.
        #[cfg(feature = "recovery-test")]
        recovery_test_fault(3, services, idx);
        assert!(wire::release_reap_receipt(r.pid, r.process_generation));
        demand_runtime::cleanup_dependencies(services);
        self.dependencies_changed(services);
    }
    fn quarantine(&mut self, services: &mut ServiceTable, idx: usize, code: u64) {
        self.timers.cancel(Key::Unit(idx));
        let e = services.get_mut(idx).unwrap();
        e.activation.fail(code, monotonic_millis());
        e.diagnostics.termination_unconfirmed = true;
        serial_println!(
            "[D2] retained unit={} generation={} failure={}",
            binary_name_of(&e.unit.exec_start),
            e.activation.generation,
            code
        );
    }
    pub(super) fn tick(&mut self, services: &mut ServiceTable) {
        let mut handled_unit = false;
        while let Some(event) = self.timers.due(monotonic_millis()) {
            if matches!(event.key, Key::Unit(_)) {
                if handled_unit {
                    self.timers.set(event.key, event.at);
                    break;
                }
                handled_unit = true;
            }
            let idx = match event.key {
                Key::Lease(slot) => {
                    if let Some(l) = self.leases.slots[slot].take() {
                        self.consider(services, l.unit);
                    }
                    continue;
                }
                Key::Unit(idx) => idx,
            };
            let phase = services.get(idx).unwrap().activation.phase;
            if phase == Phase::Stopping {
                let r = services.get(idx).unwrap().activation.runtime.unwrap();
                if let Some(wire::InstanceStatus::Reaped {
                    private_frames,
                    page_tables,
                    free_frames,
                    exit_code,
                }) = wire::instance_status(r.pid, r.process_generation)
                {
                    self.reaped(
                        services,
                        idx,
                        r,
                        private_frames,
                        page_tables,
                        free_frames,
                        exit_code,
                    );
                } else {
                    self.quarantine(services, idx, wire::TIMED_OUT);
                }
                continue;
            }
            if phase != Phase::IdleCandidate {
                continue;
            }
            let pinned = self.pinned(services, idx);
            if pinned {
                services.get_mut(idx).unwrap().activation.phase = Phase::Ready;
                continue;
            }
            let e = services.get(idx).unwrap();
            let Some(r) = e.activation.runtime else {
                continue;
            };
            let generation = e.activation.generation;
            let deadline = monotonic_millis().saturating_add(e.unit.idle.shutdown_timeout_ms);
            let cap =
                sunlight_ipc::nameserver_lookup_timeout(binary_name_of(&e.unit.exec_start), 100);
            let Some(cap) = cap.filter(|cap| {
                wire::endpoint_identity(*cap, r.pid, r.process_generation) == r.endpoint
            }) else {
                self.quarantine(services, idx, wire::STALE);
                continue;
            };
            let Ok(nonce) = services
                .get_mut(idx)
                .unwrap()
                .activation
                .begin_quiesce(false)
            else {
                self.quarantine(services, idx, wire::EXHAUSTED);
                continue;
            };
            services.get_mut(idx).unwrap().activation.quiesce_at = monotonic_millis();
            let result = sunlight_ipc::ipc_call_timeout(
                cap,
                IpcMsg::with_label(wire::QUIESCE)
                    .word(0, generation)
                    .word(1, nonce),
                deadline.saturating_sub(monotonic_millis()).max(1),
            );
            let Ok(ack) = result else {
                self.quarantine(services, idx, wire::TIMED_OUT);
                continue;
            };
            if ack.label == wire::ERROR && ack.words[0] == wire::BUSY {
                // Explicit refusal guarantees admission was never closed.
                let e = services.get_mut(idx).unwrap();
                e.activation.phase = Phase::Ready;
                e.activation.idle_reported = false;
                continue;
            }
            if ack.label != wire::DRAINED
                || ack.word_count != 4
                || ack.words != [generation, nonce, 0, 0, 0, 0, 0, 0]
                || monotonic_millis() >= deadline
                || !demand_runtime::live(r)
                || wire::endpoint_identity(cap, r.pid, r.process_generation) != r.endpoint
                || self.pinned(services, idx)
            {
                self.quarantine(services, idx, wire::STALE);
                continue;
            }
            #[cfg(feature = "recovery-test")]
            recovery_test_fault(2, services, idx);
            let e = services.get_mut(idx).unwrap();
            if e.activation.drained(generation, nonce, false).is_err() {
                self.quarantine(services, idx, wire::STALE);
                continue;
            }
            e.mark_stopping(r.pid, e.activation.spawned_at, monotonic_millis(), false);
            // Commit is irrevocable. Transport closure may be the successful
            // service exit; neither ACK nor PeerClosed is a reaping receipt.
            let committed = sunlight_ipc::ipc_call_timeout(
                cap,
                IpcMsg::with_label(wire::COMMIT_EXIT)
                    .word(0, generation)
                    .word(1, nonce),
                deadline.saturating_sub(monotonic_millis()).max(1),
            );
            services.get_mut(idx).unwrap().activation.stop_ack = committed.is_ok_and(|ack| {
                ack.label == wire::OK
                    && ack.word_count == 2
                    && ack.words[0] == generation
                    && ack.words[1] == nonce
            });
            self.timers.set(Key::Unit(idx), deadline);
        }
    }
}
