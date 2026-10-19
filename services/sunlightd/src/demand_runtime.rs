//! Runtime adapter for D1. Only authorized calls are retained, as opaque kernel
//! completion tokens, in a fixed 16-entry table. No client operations are queued.
use super::*;
use sunlight_ipc::{demand as wire, nameserver_lookup_timeout};
use sunlightd::activation::Phase;

const MAX_WAITERS: usize = 16;
#[derive(Clone, Copy)]
struct Waiter {
    token: u64,
    unit: usize,
    generation: u64,
}
pub(super) struct DemandManager {
    waiters: [Option<Waiter>; MAX_WAITERS],
    cursor: usize,
    idle: idle_runtime::IdleManager,
}

pub(super) fn is_demand_op(op: u64) -> bool {
    matches!(
        op,
        wire::DISCOVER
            | wire::ACTIVATE
            | wire::HELLO
            | wire::READY
            | wire::METRICS
            | wire::IDLE_METRICS
            | wire::IDLE_ACCOUNT
    ) || idle_runtime::is_idle_op(op)
}

fn demand_unit(services: &ServiceTable, name: &str) -> Option<usize> {
    services
        .find_by_name(name.strip_suffix(".service").unwrap_or(name))
        .filter(|i| {
            services
                .get(*i)
                .is_some_and(|e| e.unit.activation == ActivationPolicy::OnDemand)
        })
}

pub(super) fn live(runtime: sunlightd::activation::Runtime) -> bool {
    sunlight_ipc::session_query_process(runtime.pid as u64)
        .is_some_and(|p| p.generation == runtime.process_generation)
}

pub(super) fn endpoint(services: &ServiceTable, idx: usize) -> Option<u32> {
    let entry = services.get(idx)?;
    let runtime = entry.activation.runtime?;
    let cap = nameserver_lookup_timeout(binary_name_of(&entry.unit.exec_start), 100)?;
    wire::endpoint_identity(cap, runtime.pid, runtime.process_generation)
}

pub(super) fn dependencies_ready(services: &ServiceTable, idx: usize) -> bool {
    let Some(entry) = services.get(idx) else {
        return false;
    };
    for dep in entry.unit.requires.iter().chain(entry.unit.after.iter()) {
        if let Some(i) = demand_unit(services, dep) {
            if !services.get(i).is_some_and(|e| {
                matches!(e.activation.phase, Phase::Ready | Phase::IdleCandidate)
                    && e.activation
                        .runtime
                        .is_some_and(|r| live(r) && endpoint(services, i) == r.endpoint)
            }) {
                return false;
            }
        } else if entry.unit.requires.contains(dep) || unit_is_enabled(services, dep) {
            if nameserver_lookup_timeout(dep_unit_to_ready_name(dep), 100).is_none() {
                return false;
            }
        }
    }
    true
}

/// Validate the complete bounded closure before changing any dependency state.
/// After edges to demand units also require readiness, matching the boot gate.
fn plan(
    services: &ServiceTable,
    idx: usize,
    visiting: &mut [bool; MAX_UNITS],
    selected: &mut [bool; MAX_UNITS],
) -> Result<(), u64> {
    if visiting[idx] {
        return Err(wire::DEPENDENCY);
    }
    if selected[idx] {
        return Ok(());
    }
    let entry = services.get(idx).ok_or(wire::NOT_FOUND)?;
    if matches!(entry.activation.phase, Phase::Quiescing | Phase::Stopping) {
        return Err(wire::BUSY);
    }
    if !entry.enabled {
        return Err(wire::STOPPED);
    }
    if matches!(entry.activation.phase, Phase::Inactive | Phase::Failed)
        && entry.activation.generation == u64::MAX
    {
        return Err(wire::EXHAUSTED);
    }
    if entry.activation.phase == Phase::Failed
        && (entry.activation.runtime.is_some() || monotonic_millis() < entry.activation.retry_at)
    {
        return Err(wire::BUSY);
    }
    visiting[idx] = true;
    for dep in entry.unit.requires.iter().chain(entry.unit.after.iter()) {
        if let Some(i) = demand_unit(services, dep) {
            plan(services, i, visiting, selected)?;
        }
    }
    visiting[idx] = false;
    selected[idx] = true;
    Ok(())
}

impl DemandManager {
    pub(super) fn new() -> Self {
        Self {
            waiters: [None; MAX_WAITERS],
            cursor: 0,
            idle: idle_runtime::IdleManager::new(),
        }
    }

    pub(super) fn receive_timeout(&self) -> u64 {
        self.idle
            .timers
            .next()
            .map_or(50, |at| at.saturating_sub(monotonic_millis()).clamp(1, 50))
    }

    pub(super) fn control_changed(&mut self, services: &mut ServiceTable) {
        self.idle.dependencies_changed(services);
    }

    pub(super) fn handle(&mut self, msg: &IpcMsg, services: &mut ServiceTable) -> IpcMsg {
        let now = monotonic_millis();
        if idle_runtime::is_idle_op(msg.label) {
            return self.idle.handle(msg, services);
        }

        if matches!(msg.label, wire::HELLO | wire::READY) {
            let Some(idx) = find_service_idx_by_pid(services, msg.badge as u32) else {
                return wire::error(wire::STALE);
            };
            let entry = services.get(idx).unwrap();
            let Some(runtime) = entry.activation.runtime else {
                return wire::error(wire::STALE);
            };
            if entry.unit.activation != ActivationPolicy::OnDemand
                || entry.activation.phase != Phase::Starting
                || !live(runtime)
            {
                return wire::error(wire::STALE);
            }
            if msg.label == wire::HELLO {
                return IpcMsg::with_label(wire::OK)
                    .word(0, entry.activation.generation)
                    .word(1, libc::getpid());
            }
            let Some(ep) = endpoint(services, idx) else {
                return wire::error(wire::STALE);
            };
            let entry = services.get_mut(idx).unwrap();
            let mut runtime = runtime;
            runtime.endpoint = Some(ep);
            if let Err(code) = entry
                .activation
                .ready(msg.words[0], runtime, monotonic_millis())
            {
                return wire::error(code);
            }
            entry.mark_running(runtime.pid, entry.activation.spawned_at);
            serial_println!("[D1] ready unit={} generation={} pid={} process_generation={} endpoint={} cold_ms={} ready_ms={}",
                binary_name_of(&entry.unit.exec_start), entry.activation.generation, runtime.pid,
                runtime.process_generation, ep, entry.activation.metrics.cold_ms, entry.activation.metrics.ready_ms);
            return IpcMsg::with_label(wire::OK);
        }
        if msg.word_count != 4 {
            return wire::error(wire::NOT_FOUND);
        }
        let name = extract_unit_name(msg);
        let Some(idx) = demand_unit(services, &name) else {
            return wire::error(wire::NOT_FOUND);
        };
        if matches!(
            services.get(idx).unwrap().activation.phase,
            Phase::Ready | Phase::IdleCandidate
        ) {
            let r = services.get(idx).unwrap().activation.runtime.unwrap();
            if endpoint(services, idx) != r.endpoint || !live(r) {
                services
                    .get_mut(idx)
                    .unwrap()
                    .activation
                    .fail(wire::STALE, now);
            }
        }
        if msg.label == wire::DISCOVER {
            let e = services.get(idx).unwrap();
            return IpcMsg::with_label(wire::OK)
                .word(0, e.activation.phase as u64)
                .word(1, e.activation.generation)
                .word(2, e.activation.failure)
                .word(3, e.enabled as u64);
        }
        if msg.label == wire::IDLE_METRICS {
            return self.idle.metrics(idx);
        }
        if msg.label == wire::IDLE_ACCOUNT {
            if let Some(r) = services.get(idx).unwrap().activation.runtime {
                if let Some(a) = wire::instance_account(r.pid, r.process_generation) {
                    return IpcMsg::with_label(wire::OK)
                        .word(0, a[0])
                        .word(1, a[1])
                        .word(2, a[2])
                        .word(3, a[3]);
                }
            }
            return wire::error(wire::STALE);
        }
        if msg.label == wire::METRICS {
            let m = &services.get(idx).unwrap().activation.metrics;
            return IpcMsg::with_label(wire::OK)
                .word(0, m.requests)
                .word(1, m.starts)
                .word(2, m.coalesced)
                .word(
                    3,
                    m.failures.min(u32::MAX as u64) | (m.timeouts.min(u32::MAX as u64) << 32),
                );
        }
        // Kernel mediation checks the immediate caller's real profile. Ready
        // identity was revalidated above; no operation capability is returned.
        if matches!(
            services.get(idx).unwrap().activation.phase,
            Phase::Ready | Phase::IdleCandidate
        ) {
            let e = services.get_mut(idx).unwrap();
            let _ = e.activation.request(now);
            self.idle.timers.cancel(sunlightd::idle::Key::Unit(idx));
            let reply = e.activation.resolution();
            self.idle.consider(services, idx);
            return reply;
        }
        self.reap_cancelled();
        let Some(slot) = self.waiters.iter().position(Option::is_none) else {
            return wire::error(wire::BUSY);
        };
        let mut selected = [false; MAX_UNITS];
        if let Err(code) = plan(services, idx, &mut [false; MAX_UNITS], &mut selected) {
            return wire::error(code);
        }
        // Defer before reserving: kernel capacity exhaustion cannot start work.
        let Ok(token) = sunlight_ipc::ipc_defer_reply() else {
            return wire::error(wire::BUSY);
        };
        for (i, chosen) in selected.iter().enumerate().take(services.count) {
            if *chosen {
                if let Err(code) = services.get_mut(i).unwrap().activation.request(now) {
                    let _ = sunlight_ipc::ipc_complete_deferred_reply(token, wire::error(code));
                    return IpcMsg::empty();
                }
            }
        }
        self.idle.dependencies_changed(services);
        let generation = services.get(idx).unwrap().activation.generation;
        self.waiters[slot] = Some(Waiter {
            token,
            unit: idx,
            generation,
        });
        IpcMsg::empty()
    }

    fn reap_cancelled(&mut self) {
        for slot in &mut self.waiters {
            if slot.is_some_and(|w| !sunlight_ipc::ipc_deferred_reply_is_live(w.token)) {
                *slot = None;
            }
        }
    }

    pub(super) fn tick(&mut self, services: &mut ServiceTable, spawn_cap: CapabilityToken) {
        let now = monotonic_millis();
        self.reap_cancelled();
        for idx in 0..services.count {
            let entry = services.get(idx).unwrap();
            if entry.unit.activation != ActivationPolicy::OnDemand {
                continue;
            }
            if let Some(r) = entry.activation.runtime {
                if entry.unit.idle.unload_when_idle {
                    match wire::instance_status(r.pid, r.process_generation) {
                        Some(wire::InstanceStatus::Reaped {
                            private_frames,
                            page_tables,
                            free_frames,
                            exit_code,
                        }) => {
                            self.idle.reaped(
                                services,
                                idx,
                                r,
                                private_frames,
                                page_tables,
                                free_frames,
                                exit_code,
                            );
                        }
                        Some(wire::InstanceStatus::Exiting) | None => {
                            let e = services.get_mut(idx).unwrap();
                            if !matches!(e.activation.phase, Phase::Stopping | Phase::Failed) {
                                e.activation.fail(wire::DIED, now);
                            }
                        }
                        Some(wire::InstanceStatus::Live) => {}
                    }
                } else if !live(r) {
                    let e = services.get_mut(idx).unwrap();
                    // Preserve timeout/stop failure rather than replacing it with death.
                    if matches!(e.activation.phase, Phase::Starting | Phase::Ready) {
                        e.activation.fail(wire::DIED, now);
                    }
                    e.activation.runtime = None;
                    if e.state.is_active() {
                        e.mark_failed(-1, now);
                    }
                    self.idle.dependencies_changed(services);
                }
            }
            let e = services.get(idx).unwrap();
            if e.activation.phase == Phase::Starting
                && e.activation.spawn_attempted
                && matches!(e.state, ServiceState::Failed { .. })
            {
                services
                    .get_mut(idx)
                    .unwrap()
                    .activation
                    .fail(wire::START_FAILED, now);
                self.idle.dependencies_changed(services);
            }
            let e = services.get(idx).unwrap();
            if e.activation.expired(now) {
                let e = services.get_mut(idx).unwrap();
                e.activation.fail(wire::TIMED_OUT, now);
                e.record_result(FAILURE_STARTUP, FAILURE_STARTUP, wire::TIMED_OUT as u32);
                if let Some(r) = e.activation.runtime {
                    // Failed initialization cleanup, not D2 idle unloading.
                    // live() above validated the exact process incarnation.
                    if !e.unit.idle.unload_when_idle {
                        let _ = wire::kill_instance(r.pid, r.process_generation, SIGKILL);
                    }
                    e.mark_stopping(r.pid, e.activation.spawned_at, now, false);
                } else {
                    e.mark_failed(-1, now);
                }
                serial_println!(
                    "[D1] timeout unit={} generation={}",
                    binary_name_of(&e.unit.exec_start),
                    e.activation.generation
                );
            }
        }
        // At most one bounded dependency/spawn operation per turn. A missing
        // external provider cannot multiply per-lookup timeouts by MAX_UNITS
        // before timeout and cancellation supervision runs again.
        for offset in 0..services.count {
            let idx = (self.cursor + offset) % services.count;
            let e = services.get(idx).unwrap();
            if e.unit.activation != ActivationPolicy::OnDemand
                || e.activation.phase != Phase::Starting
                || e.activation.spawn_attempted
            {
                continue;
            }
            self.cursor = (idx + 1) % services.count;
            let failed_dep = e
                .unit
                .requires
                .iter()
                .chain(e.unit.after.iter())
                .any(|dep| {
                    demand_unit(services, dep)
                        .is_some_and(|i| services.get(i).unwrap().activation.phase == Phase::Failed)
                });
            if failed_dep {
                services
                    .get_mut(idx)
                    .unwrap()
                    .activation
                    .fail(wire::DEPENDENCY, monotonic_millis());
                self.idle.dependencies_changed(services);
            } else {
                match spawn_service_at(services, idx, spawn_cap) {
                    Ok(_) | Err(DETAIL_TRANSITION_BUSY) => {}
                    Err(_) => {
                        services
                            .get_mut(idx)
                            .unwrap()
                            .activation
                            .fail(wire::START_FAILED, monotonic_millis());
                        self.idle.dependencies_changed(services);
                    }
                }
            }
            break;
        }
        self.idle.tick(services);
        for slot in &mut self.waiters {
            let Some(w) = *slot else {
                continue;
            };
            let e = services.get(w.unit).unwrap();
            let reply = if e.activation.generation != w.generation {
                Some(wire::error(wire::STALE))
            } else {
                match e.activation.phase {
                    Phase::Ready | Phase::IdleCandidate => Some(e.activation.resolution()),
                    Phase::Quiescing | Phase::Stopping => Some(wire::error(wire::BUSY)),
                    Phase::Failed => Some(wire::error(e.activation.failure)),
                    Phase::Inactive => Some(wire::error(wire::STOPPED)),
                    Phase::Starting => None,
                }
            };
            if let Some(reply) = reply {
                let _ = sunlight_ipc::ipc_complete_deferred_reply(w.token, reply);
                *slot = None;
            }
        }
    }
}

#[cfg(feature = "demand-test")]
pub(super) fn add_test_units(services: &mut ServiceTable) {
    #[cfg(feature = "idle-test")]
    {
        let driver=b"[Service]\nExecStart=/sbin/d2-driver\nCapability=service-lifecycle\nCapability=demand-fixture\nCapability=spawn-user\nCapability=time-sync\nCapability=logging\n";
        services.add(parse_service_unit(driver).unwrap()).unwrap();
        let fixture=b"[Service]\nType=notify\nActivation=on-demand\nUnloadWhenIdle=yes\nIdleGraceMs=500\nMinimumUptimeMs=500\nShutdownTimeoutMs=1000\nExecStart=/sbin/d2-fixture\nCapability=service-notify\nCapability=logging\n";
        services.add(parse_service_unit(fixture).unwrap()).unwrap();
        let consumer=b"[Unit]\nRequires=d2-fixture.service\nAfter=d2-fixture.service\n[Service]\nType=notify\nActivation=on-demand\nExecStart=/sbin/d2-dependent\nCapability=service-notify\nCapability=logging\n";
        services.add(parse_service_unit(consumer).unwrap()).unwrap();
    }
    let driver = b"[Service]\nExecStart=/sbin/d1-driver\nCapability=service-lifecycle\nCapability=demand-fixture\nCapability=spawn-user\nCapability=time-sync\nCapability=logging\n";
    let _ = services.add(parse_service_unit(driver).expect("D1 driver"));
    for name in [
        "d1-fixture",
        "d1-timeout",
        "d1-crash",
        "d1-missing",
        "d1-dependent",
    ] {
        let mut definition = heapless::String::<512>::new();
        use core::fmt::Write;
        let _ = write!(definition, "[Unit]\nDescription=D1 synthetic experiment\n{}
[Service]\nType=notify\nActivation=on-demand\nExecStart=/sbin/{}\nRestart=no\nUser=root\nCapability=service-notify\nCapability=logging\n", if name == "d1-dependent" { "Requires=d1-fixture.service\nAfter=d1-fixture.service\n" } else { "" }, name);
        let _ = services.add(parse_service_unit(definition.as_bytes()).expect("D1 unit"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn table(definitions: &[&[u8]]) -> ServiceTable {
        let mut table = ServiceTable::new();
        for d in definitions {
            table.add(parse_service_unit(d).unwrap()).unwrap();
        }
        table
    }
    #[test]
    fn boot_only_reserves_enabled_residents() {
        let table = table(&[
            b"[Service]\nExecStart=/sbin/resident\n",
            b"[Service]\nExecStart=/sbin/lazy\nActivation=on-demand\nType=notify\n",
        ]);
        let boot = BootStartup::new(&table);
        assert!(boot.is_pending(0));
        assert!(!boot.is_pending(1));
        assert_eq!(table.get(1).unwrap().activation.phase, Phase::Inactive);
        assert_eq!(table.get(1).unwrap().activation.metrics.starts, 0);
    }
    #[test]
    fn dependency_cycles_rejected_before_reservation() {
        let table = table(&[
            b"[Unit]\nRequires=b.service\n[Service]\nExecStart=/sbin/a\nActivation=on-demand\nType=notify\n",
            b"[Unit]\nRequires=a.service\n[Service]\nExecStart=/sbin/b\nActivation=on-demand\nType=notify\n",
        ]);
        assert!(build_dep_graph(&table).is_err());
        assert_eq!(
            plan(&table, 0, &mut [false; MAX_UNITS], &mut [false; MAX_UNITS]),
            Err(wire::DEPENDENCY)
        );
        assert_eq!(table.get(0).unwrap().activation.generation, 0);
        assert_eq!(table.get(1).unwrap().activation.generation, 0);
    }
    #[test]
    fn dependency_plan_selects_bounded_closure_without_spawning() {
        let table = table(&[
            b"[Unit]\nRequires=b.service\n[Service]\nExecStart=/sbin/a\nActivation=on-demand\nType=notify\n",
            b"[Service]\nExecStart=/sbin/b\nActivation=on-demand\nType=notify\n",
            b"[Service]\nExecStart=/sbin/unrelated\n",
        ]);
        let mut selected = [false; MAX_UNITS];
        plan(&table, 0, &mut [false; MAX_UNITS], &mut selected).unwrap();
        assert!(selected[0] && selected[1] && !selected[2]);
        assert_eq!(table.get(0).unwrap().activation.metrics.starts, 0);
        assert_eq!(table.get(1).unwrap().activation.metrics.starts, 0);
    }
}
