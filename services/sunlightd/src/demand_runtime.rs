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
            | wire::DEPENDENCY_METRICS
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

/// Bounded transitive snapshot. Runtime validation happens outside all kernel
/// locks. A consumer with a process must already own bound provider identities.
pub(super) fn dependencies_ready(services: &ServiceTable, idx: usize) -> bool {
    let mut selected = [false; MAX_UNITS];
    if plan(services, idx, &mut [false; MAX_UNITS], &mut selected).is_err() {
        return false;
    }
    for consumer in 0..services.count {
        if !selected[consumer] {
            continue;
        }
        let entry = services.get(consumer).unwrap();
        for dep in entry.unit.requires.iter().chain(entry.unit.after.iter()) {
            if let Some(i) = demand_unit(services, dep) {
                let e = services.get(i).unwrap();
                let Some(r) = e.activation.runtime else {
                    return false;
                };
                if !matches!(e.activation.phase, Phase::Ready | Phase::IdleCandidate)
                    || !live(r)
                    || endpoint(services, i) != r.endpoint
                    || (entry.activation.runtime.is_some()
                        && !services.dependencies.matches(
                            instance(services, consumer),
                            instance(services, i),
                            r,
                        ))
                {
                    return false;
                }
            } else if entry.unit.requires.contains(dep) || unit_is_enabled(services, dep) {
                if nameserver_lookup_timeout(dep_unit_to_ready_name(dep), 100).is_none() {
                    return false;
                }
            }
        }
    }
    true
}

/// No IPC or clock sampling: consumes the existing supervisor's observations.
/// Repeat bounded propagation so table order cannot leave a transitive Ready.
fn propagate_dependency_failure(services: &mut ServiceTable, now: u64) -> bool {
    let mut changed = false;
    for _ in 0..services.count {
        let mut pass = false;
        for idx in 0..services.count {
            let e = services.get(idx).unwrap();
            if e.unit.activation != ActivationPolicy::OnDemand
                || !matches!(
                    e.activation.phase,
                    Phase::Starting | Phase::Ready | Phase::IdleCandidate
                )
            {
                continue;
            }
            let invalid = e
                .unit
                .requires
                .iter()
                .chain(e.unit.after.iter())
                .any(|dep| {
                    let Some(i) = demand_unit(services, dep) else {
                        return false;
                    };
                    let provider = services.get(i).unwrap();
                    if matches!(
                        provider.activation.phase,
                        Phase::Failed | Phase::Inactive | Phase::Quiescing | Phase::Stopping
                    ) {
                        return true;
                    }
                    if e.activation.runtime.is_none() {
                        return false;
                    }
                    !provider.activation.runtime.is_some_and(|r| {
                        services.dependencies.matches(
                            instance(services, idx),
                            instance(services, i),
                            r,
                        )
                    })
                });
            if invalid {
                services
                    .get_mut(idx)
                    .unwrap()
                    .activation
                    .fail(wire::DEPENDENCY, now);
                pass = true;
                changed = true;
            }
        }
        if !pass {
            break;
        }
    }
    changed
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

fn instance(services: &ServiceTable, idx: usize) -> sunlightd::dependency::Instance {
    sunlightd::dependency::Instance {
        unit: idx,
        generation: services.get(idx).unwrap().activation.generation,
    }
}

/// Build first, then atomically insert. Predicted generations are safe because
/// the same event-loop turn performs all requests without yielding ownership.
pub(super) fn reserve_dependencies(
    services: &mut ServiceTable,
    selected: &[bool; MAX_UNITS],
    predicted: bool,
) -> Result<(), u64> {
    use sunlightd::dependency::{Instance, Lease, MAX_DEPENDENCY_LEASES};
    let identity = |i: usize| -> Result<Instance, u64> {
        let e = services.get(i).unwrap();
        let mut id = instance(services, i);
        if predicted && selected[i] && matches!(e.activation.phase, Phase::Inactive | Phase::Failed)
        {
            id.generation = id.generation.checked_add(1).ok_or(wire::EXHAUSTED)?;
        }
        Ok(id)
    };
    let mut edges = heapless::Vec::<Lease, MAX_DEPENDENCY_LEASES>::new();
    for idx in 0..services.count {
        if !selected[idx] {
            continue;
        }
        let e = services.get(idx).unwrap();
        for dep in e.unit.requires.iter().chain(e.unit.after.iter()) {
            if let Some(i) = demand_unit(services, dep) {
                let edge = Lease {
                    consumer: identity(idx)?,
                    provider: identity(i)?,
                    runtime: None,
                };
                if !edges.contains(&edge) {
                    edges.push(edge).map_err(|_| wire::BUSY)?;
                }
            }
        }
    }
    services.dependencies.reserve(&edges)
}

pub(super) fn bind_dependencies(services: &mut ServiceTable, idx: usize) -> Result<(), u64> {
    let consumer = instance(services, idx);
    for i in 0..services.count {
        let e = services.get(idx).unwrap();
        if !e
            .unit
            .requires
            .iter()
            .chain(e.unit.after.iter())
            .any(|dep| demand_unit(services, dep) == Some(i))
        {
            continue;
        }
        let provider = instance(services, i);
        let e = services.get(i).unwrap();
        if !matches!(e.activation.phase, Phase::Ready | Phase::IdleCandidate) {
            return Err(wire::DEPENDENCY);
        }
        let runtime = e.activation.runtime.ok_or(wire::STALE)?;
        services.dependencies.bind(consumer, provider, runtime)?;
    }
    Ok(())
}

pub(super) fn cleanup_dependencies(services: &mut ServiceTable) -> bool {
    let before = services.dependencies.count();
    let alive: [Option<u64>; MAX_UNITS] = core::array::from_fn(|idx| {
        services
            .get(idx)
            .filter(|e| e.activation.phase == Phase::Starting || e.activation.runtime.is_some())
            .map(|e| e.activation.generation)
    });
    services.dependencies.retain(|id| {
        alive
            .get(id.unit)
            .is_some_and(|g| *g == Some(id.generation))
    });
    services.dependencies.count() != before
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
            if !dependencies_ready(services, idx) {
                services
                    .get_mut(idx)
                    .unwrap()
                    .activation
                    .fail(wire::DEPENDENCY, now);
                cleanup_dependencies(services);
                self.idle.dependencies_changed(services);
                return wire::error(wire::DEPENDENCY);
            }
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
            if endpoint(services, idx) != r.endpoint
                || !live(r)
                || !dependencies_ready(services, idx)
            {
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
        if msg.label == wire::DEPENDENCY_METRICS {
            let (outgoing, incoming) = services.dependencies.counts(idx);
            let retained = (0..services.count)
                .filter(|i| {
                    let e = services.get(*i).unwrap();
                    e.activation.runtime.is_some() || e.state.may_still_be_alive()
                })
                .count();
            return IpcMsg::with_label(wire::OK)
                .word(0, services.dependencies.count() as u64)
                .word(1, outgoing as u64)
                .word(2, incoming as u64)
                .word(3, retained as u64);
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
        cleanup_dependencies(services);
        if let Err(code) = reserve_dependencies(services, &selected, true) {
            let _ = sunlight_ipc::ipc_complete_deferred_reply(token, wire::error(code));
            return IpcMsg::empty();
        }
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
        if propagate_dependency_failure(services, now) {
            cleanup_dependencies(services);
            self.idle.dependencies_changed(services);
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
        if cleanup_dependencies(services) {
            self.idle.dependencies_changed(services);
        }
        self.idle.tick(services);
        for slot in &mut self.waiters {
            let Some(w) = *slot else {
                continue;
            };
            if matches!(
                services.get(w.unit).unwrap().activation.phase,
                Phase::Ready | Phase::IdleCandidate
            ) && !dependencies_ready(services, w.unit)
            {
                services
                    .get_mut(w.unit)
                    .unwrap()
                    .activation
                    .fail(wire::DEPENDENCY, monotonic_millis());
            }
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
    #[cfg(feature = "dependency-test")]
    for (name, dependencies) in [
        ("d3-leaf", "d2-fixture.service"),
        ("d3-parent", "d3-leaf.service d2-fixture.service"),
    ] {
        use core::fmt::Write;
        let mut definition = heapless::String::<512>::new();
        write!(definition, "[Unit]\nRequires={}\n[Service]\nType=notify\nActivation=on-demand\nUnloadWhenIdle=yes\nIdleGraceMs=500\nMinimumUptimeMs=500\nExecStart=/sbin/{}\nCapability=service-notify\nCapability=logging\n", dependencies, name).unwrap();
        services
            .add(parse_service_unit(definition.as_bytes()).unwrap())
            .unwrap();
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
    fn chain() -> ServiceTable {
        table(&[
            b"[Unit]\nRequires=b.service c.service\nAfter=b.service\n[Service]\nExecStart=/sbin/a\nActivation=on-demand\nType=notify\n",
            b"[Unit]\nRequires=c.service\n[Service]\nExecStart=/sbin/b\nActivation=on-demand\nType=notify\n",
            b"[Service]\nExecStart=/sbin/c\nActivation=on-demand\nType=notify\n",
        ])
    }
    #[test]
    fn closure_reservations_deduplicate_and_bind_before_consumer_spawn() {
        let mut t = chain();
        let mut selected = [false; MAX_UNITS];
        plan(&t, 0, &mut [false; MAX_UNITS], &mut selected).unwrap();
        reserve_dependencies(&mut t, &selected, true).unwrap();
        assert_eq!(t.dependencies.count(), 3);
        for i in 0..3 {
            t.get_mut(i).unwrap().activation.request(0).unwrap();
        }
        assert_eq!(bind_dependencies(&mut t, 0), Err(wire::DEPENDENCY));
        for i in [2, 1] {
            let e = t.get_mut(i).unwrap();
            e.activation.spawned(i as u32 + 10, i as u64 + 20, 1);
            let mut r = e.activation.runtime.unwrap();
            r.endpoint = Some(i as u32 + 30);
            e.activation.ready(1, r, 2).unwrap();
        }
        bind_dependencies(&mut t, 0).unwrap();
        assert_eq!(t.dependencies.count(), 3);
    }
    #[test]
    fn failed_unspawned_parent_releases_only_its_own_holds() {
        let mut t = chain();
        let selected = core::array::from_fn(|i| i < 3);
        reserve_dependencies(&mut t, &selected, true).unwrap();
        for i in 0..3 {
            t.get_mut(i).unwrap().activation.request(0).unwrap();
        }
        t.get_mut(0).unwrap().activation.fail(wire::DEPENDENCY, 1);
        assert!(cleanup_dependencies(&mut t));
        assert_eq!(t.dependencies.count(), 1);
        assert!(!cleanup_dependencies(&mut t));
        t.get_mut(1).unwrap().activation.fail(wire::TIMED_OUT, 2);
        cleanup_dependencies(&mut t);
        assert_eq!(t.dependencies.count(), 0);
    }
    #[test]
    fn retained_consumer_keeps_dependencies_through_every_shutdown_phase() {
        let mut t = chain();
        let selected = core::array::from_fn(|i| i < 3);
        reserve_dependencies(&mut t, &selected, true).unwrap();
        for i in 0..3 {
            t.get_mut(i).unwrap().activation.request(0).unwrap();
        }
        t.get_mut(0).unwrap().activation.spawned(10, 20, 1);
        for phase in [Phase::Quiescing, Phase::Stopping, Phase::Failed] {
            t.get_mut(0).unwrap().activation.phase = phase;
            cleanup_dependencies(&mut t);
            assert_eq!(t.dependencies.count(), 3);
        }
        t.get_mut(0).unwrap().activation.runtime = None;
        cleanup_dependencies(&mut t);
        assert_eq!(t.dependencies.count(), 1);
    }
    #[test]
    fn stopping_dependency_rejects_plan_without_reserving_or_restarting() {
        let mut t = chain();
        t.get_mut(2).unwrap().activation.phase = Phase::Stopping;
        assert_eq!(
            plan(&t, 0, &mut [false; MAX_UNITS], &mut [false; MAX_UNITS]),
            Err(wire::BUSY)
        );
        assert_eq!(t.dependencies.count(), 0);
        assert_eq!(t.get(0).unwrap().activation.generation, 0);
    }
    #[test]
    fn provider_failure_propagates_transitively_without_restart_or_rebinding() {
        let mut t = chain();
        let selected = core::array::from_fn(|i| i < 3);
        reserve_dependencies(&mut t, &selected, true).unwrap();
        for i in 0..3 {
            let a = &mut t.get_mut(i).unwrap().activation;
            a.request(0).unwrap();
            a.spawned(i as u32 + 10, i as u64 + 20, 1);
            let mut r = a.runtime.unwrap();
            r.endpoint = Some(i as u32 + 30);
            a.ready(1, r, 2).unwrap();
        }
        bind_dependencies(&mut t, 1).unwrap();
        bind_dependencies(&mut t, 0).unwrap();
        assert!(!propagate_dependency_failure(&mut t, 3));
        t.get_mut(2).unwrap().activation.fail(wire::DIED, 4);
        assert!(propagate_dependency_failure(&mut t, 4));
        for i in 0..2 {
            assert_eq!(t.get(i).unwrap().activation.phase, Phase::Failed);
        }
        assert!(!propagate_dependency_failure(&mut t, 5));
        cleanup_dependencies(&mut t);
        assert_eq!(t.dependencies.count(), 3); // live quarantined parents retain holds
        assert_eq!(t.get(0).unwrap().activation.metrics.starts, 1);
    }
    #[test]
    fn replacement_generation_is_not_adopted_and_shutdown_keeps_holds() {
        let mut t = chain();
        let selected = core::array::from_fn(|i| i < 3);
        reserve_dependencies(&mut t, &selected, true).unwrap();
        for i in 0..3 {
            let a = &mut t.get_mut(i).unwrap().activation;
            a.request(0).unwrap();
            a.spawned(i as u32 + 10, i as u64 + 20, 1);
            let mut r = a.runtime.unwrap();
            r.endpoint = Some(i as u32 + 30);
            a.ready(1, r, 2).unwrap();
        }
        bind_dependencies(&mut t, 1).unwrap();
        bind_dependencies(&mut t, 0).unwrap();
        t.get_mut(0).unwrap().activation.phase = Phase::Stopping;
        t.get_mut(2).unwrap().activation.generation = 2;
        assert!(propagate_dependency_failure(&mut t, 4));
        assert_eq!(t.get(1).unwrap().activation.phase, Phase::Failed);
        assert_eq!(t.get(0).unwrap().activation.phase, Phase::Stopping);
        cleanup_dependencies(&mut t);
        assert_eq!(t.dependencies.count(), 3);
    }
    #[test]
    fn advisory_wants_neither_activates_nor_pins() {
        let mut t = table(&[
            b"[Unit]\nWants=b.service\n[Service]\nExecStart=/sbin/a\nActivation=on-demand\nType=notify\n",
            b"[Service]\nExecStart=/sbin/b\nActivation=on-demand\nType=notify\n",
        ]);
        let mut selected = [false; MAX_UNITS];
        plan(&t, 0, &mut [false; MAX_UNITS], &mut selected).unwrap();
        assert!(!selected[1]);
        reserve_dependencies(&mut t, &selected, true).unwrap();
        assert_eq!(t.dependencies.count(), 0);
    }
    #[test]
    fn maximum_depth_is_bounded_and_three_node_cycle_is_atomic() {
        use core::fmt::Write;
        let mut t = ServiceTable::new();
        for i in 0..MAX_UNITS {
            let mut def = heapless::String::<256>::new();
            if i + 1 < MAX_UNITS {
                write!(def, "[Unit]\nRequires=n{}.service\n", i + 1).unwrap();
            }
            write!(
                def,
                "[Service]\nExecStart=/sbin/n{}\nActivation=on-demand\nType=notify\n",
                i
            )
            .unwrap();
            t.add(parse_service_unit(def.as_bytes()).unwrap()).unwrap();
        }
        let mut selected = [false; MAX_UNITS];
        plan(&t, 0, &mut [false; MAX_UNITS], &mut selected).unwrap();
        assert!(selected.iter().all(|s| *s));
        reserve_dependencies(&mut t, &selected, true).unwrap();
        assert_eq!(t.dependencies.count(), MAX_UNITS - 1);
        let mut t = chain();
        t.get_mut(2)
            .unwrap()
            .unit
            .requires
            .push(heapless::String::try_from("a.service").unwrap())
            .unwrap();
        assert_eq!(
            plan(&t, 0, &mut [false; MAX_UNITS], &mut [false; MAX_UNITS]),
            Err(wire::DEPENDENCY)
        );
        assert_eq!(t.dependencies.count(), 0);
        assert_eq!(t.get(0).unwrap().activation.generation, 0);
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
