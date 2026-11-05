//! C1.2B single-CPU observation only. Every access to STATE requires the
//! existing SCHEDULER guard; no new locks or scheduler policy mutations.
use core::cell::UnsafeCell;

use crate::process::ProcessState;
use crate::sched::Scheduler;

const N: usize = 4;
// Bounded storage for up to 100 Hz over 10 seconds per worker, plus margin.
const MAX_WAITS: usize = 2_048;

#[derive(Clone, Copy)]
pub struct Worker {
    pid: usize,
    active: bool,
    baseline: u64,
    runtime: u64,
    ready_since: u64,
    dispatches: u64,
    waits: [u64; MAX_WAITS],
    wait_count: usize,
    bursts: [u32; MAX_WAITS],
    dispatch_at: [u64; MAX_WAITS],
    budgets: [u32; MAX_WAITS],
    run_ns: [u64; MAX_WAITS],
    reasons: [u64; MAX_WAITS],
    selected_ns: u64,
    wait_overflow: bool,
    wait_sum: u64,
    wait_max: u64,
    first_ns: u64,
    finish_ns: u64,
    burst_start: u32,
    burst_end: u32,
    nice: i8,
    mask: u64,
    invalid_observations: u64,
}

impl Worker {
    const ZERO: Self = Self {
        pid: 0,
        active: false,
        baseline: 0,
        runtime: 0,
        ready_since: 0,
        dispatches: 0,
        waits: [0; MAX_WAITS],
        wait_count: 0,
        bursts: [0; MAX_WAITS],
        dispatch_at: [0; MAX_WAITS],
        budgets: [0; MAX_WAITS],
        run_ns: [0; MAX_WAITS],
        reasons: [0; MAX_WAITS],
        selected_ns: 0,
        wait_overflow: false,
        wait_sum: 0,
        wait_max: 0,
        first_ns: 0,
        finish_ns: 0,
        burst_start: 0,
        burst_end: 0,
        nice: 0,
        mask: 0,
        invalid_observations: 0,
    };

    fn wait(&mut self, now: u64) {
        let elapsed = now.saturating_sub(self.ready_since);
        self.wait_sum += elapsed;
        self.wait_max = self.wait_max.max(elapsed);
        if self.wait_count < MAX_WAITS {
            self.waits[self.wait_count] = elapsed;
            self.wait_count += 1;
        } else {
            self.wait_overflow = true;
        }
    }
}

#[derive(Clone, Copy)]
struct Service {
    pid: u64,
    baseline: u64,
    cpu_ns: u64,
    ready_since: Option<u64>,
    waits: [u64; MAX_WAITS],
    count: usize,
    overflow: bool,
    ready_count: u64,
    dispatches: u64,
    deadline_count: u64,
    deadline_late_ns: u64,
    deadline_max_ns: u64,
}
impl Service {
    const ZERO: Self = Self {
        pid: 0,
        baseline: 0,
        cpu_ns: 0,
        ready_since: None,
        waits: [0; MAX_WAITS],
        count: 0,
        overflow: false,
        ready_count: 0,
        dispatches: 0,
        deadline_count: 0,
        deadline_late_ns: 0,
        deadline_max_ns: 0,
    };
}

struct State {
    services: [Service; 2],
    workers: [Worker; N],
    start: u64,
    start_tick: u64,
    run: u64,
    switches: u64,
}
struct Protected(UnsafeCell<State>);
// SAFETY: access is serialized by the existing global scheduler lock.
unsafe impl Sync for Protected {}
static STATE: Protected = Protected(UnsafeCell::new(State {
    services: [Service::ZERO, Service::ZERO],
    workers: [Worker::ZERO; N],
    start: 0,
    start_tick: 0,
    run: 0,
    switches: 0,
}));

fn switches(sched: &Scheduler) -> u64 {
    match crate::sched::SCHEDULER_MODE {
        crate::sched::SchedulerMode::RoundRobin => {
            crate::sched::CORE_STATES[0].lock().context_switches
        }
        crate::sched::SchedulerMode::Bore => sched.cores[0].context_switches,
    }
}

/// Diagnostic Ready publication under SCHEDULER; no allocation or logging.
pub fn service_ready(name: &str, now: u64) {
    let state = unsafe { &mut *STATE.0.get() };
    if state.start == 0 || !state.workers.iter().any(|w| w.active) {
        return;
    }
    let slot = match name {
        "timer_server" => 0,
        "tty_server" => 1,
        _ => return,
    };
    let service = &mut state.services[slot];
    service.ready_count = service.ready_count.saturating_add(1);
    service.ready_since.get_or_insert(now);
}

/// Record the BSP timer's benchmark-relative tick phase against the clock.
/// global_tick is itself derived from calibrated elapsed time and catches up
/// after delayed delivery. This phase remainder is not independent evidence
/// of timely IRQ delivery. C1.2E records handler-entry interarrival gaps too.
pub fn timer_deadline(global_tick: u64, now: u64) {
    let state = unsafe { &mut *STATE.0.get() };
    if state.start == 0 {
        return;
    }
    let service = &mut state.services[0];
    let expected = state.start.saturating_add(
        global_tick
            .saturating_sub(state.start_tick)
            .saturating_mul(10_000_000),
    );
    let late = now.saturating_sub(expected);
    service.deadline_count = service.deadline_count.saturating_add(1);
    service.deadline_late_ns = service.deadline_late_ns.saturating_add(late);
    service.deadline_max_ns = service.deadline_max_ns.max(late);
}

/// Call only while holding SCHEDULER, immediately after schedule_tick. A
/// nonzero result means a selected dispatch, including same-task reselection.
/// No logging or allocation occurs here; existing scheduler read accessors
/// preserve the established SCHEDULER-to-core lock order.
pub fn observe(sched: &Scheduler, outgoing: Option<usize>, result: u64, reason: u64) {
    let state = unsafe { &mut *STATE.0.get() };
    if state.start == 0 {
        return;
    }
    let now = sched.now_ns();
    let selected = sched.diagnostic_current_pid(0);
    #[cfg(feature = "scheduler_timer_trace")]
    if result != 0 {
        if let Some(p) = sched.processes.iter().find(|p| Some(p.pid) == selected) {
            crate::scheduler_timer_trace::event(
                sched,
                4,
                p.pid,
                outgoing.unwrap_or(0) as u64,
                reason,
                (p.nice as i64 + 20) as u64,
            );
        }
    }
    if result != 0 {
        if let Some(p) = sched.processes.iter().find(|p| Some(p.pid) == selected) {
            let slot = match p.name_str() {
                "timer_server" => Some(0),
                "tty_server" => Some(1),
                _ => None,
            };
            if let Some(slot) = slot {
                let service = &mut state.services[slot];
                service.dispatches = service.dispatches.saturating_add(1);
                if let Some(ready) = service.ready_since.take() {
                    if service.count < MAX_WAITS {
                        service.waits[service.count] = now.saturating_sub(ready);
                        service.count += 1;
                    } else {
                        service.overflow = true;
                    }
                }
            }
        }
    }
    for w in &mut state.workers {
        if !w.active {
            continue;
        }
        let Some(p) = sched.processes.iter().find(|p| p.pid == w.pid) else {
            w.invalid_observations += 1;
            continue;
        };
        if !matches!(p.state, ProcessState::Ready | ProcessState::Running)
            || p.nice != w.nice
            || p.cpu_mask & 1 == 0
            || p.context_rsp == 0
            || p.exit_cleanup_pending
        {
            w.invalid_observations += 1;
        }
        if result == 0 {
            continue;
        }
        if outgoing == Some(w.pid) {
            if w.wait_count > 0 {
                let index = w.wait_count - 1;
                w.run_ns[index] += now.saturating_sub(w.selected_ns);
                w.reasons[index] = reason;
            }
            w.ready_since = now;
        }
        if selected == Some(w.pid) {
            let sample = w.wait_count;
            if sample < MAX_WAITS {
                w.bursts[sample] = p.burst_score;
                w.dispatch_at[sample] = now.saturating_sub(state.start);
                w.budgets[sample] = p.quantum_override.unwrap_or(5 + p.burst_score * 45 / 1024);
            }
            w.wait(now);
            w.selected_ns = now;
            w.dispatches += 1;
            if w.first_ns == 0 {
                w.first_ns = now;
            }
        }
    }
}

/// Feature-private diagnostic ABI: reset, register-current(slot), begin(run),
/// finish-current(slot), report(run). Reject SMP and unrelated callers.
pub fn syscall(action: u64, value: u64) -> u64 {
    let sched = crate::sched::SCHEDULER.lock();
    if sched.online_cores != 1 {
        return u64::MAX;
    }
    let current = sched.current_process();
    if !current.name.starts_with(b"schedbench") {
        return u64::MAX;
    }
    let state = unsafe { &mut *STATE.0.get() };
    let now = sched.now_ns();
    match action {
        0 => {
            if state.workers.iter().any(|w| w.active) {
                return u64::MAX;
            }
            // All fields admit zero. Initialize in place: copying the complete
            // trace array through a temporary exceeds a 32 KiB kernel stack.
            unsafe {
                core::ptr::write_bytes(state.workers.as_mut_ptr(), 0, N);
            }
            for service in &mut state.services {
                service.pid = 0;
                service.baseline = 0;
                service.cpu_ns = 0;
                service.ready_since = None;
                service.count = 0;
                service.overflow = false;
                service.ready_count = 0;
                service.dispatches = 0;
                service.deadline_count = 0;
                service.deadline_late_ns = 0;
                service.deadline_max_ns = 0;
            }
            state.start = 0;
        }
        1 if value < N as u64 => {
            let w = &mut state.workers[value as usize];
            if w.pid != 0 || !matches!(current.nice, 0 | 8..=10) || current.cpu_mask & 1 == 0 {
                return u64::MAX;
            }
            w.pid = current.pid;
            w.nice = current.nice;
            w.mask = current.cpu_mask;
        }
        2 => {
            if state.workers.iter().any(|w| w.pid == 0) {
                return u64::MAX;
            }
            state.start = now;
            state.switches = switches(&sched);
            #[cfg(feature = "scheduler_timer_trace")]
            crate::scheduler_timer_trace::begin(&sched);
            state.start_tick = sched.global_tick;
            state.run = value;
            for (slot, service) in state.services.iter_mut().enumerate() {
                let name = ["timer_server", "tty_server"][slot];
                if let Some(idx) = sched.processes.iter().position(|p| p.name_str() == name) {
                    service.pid = sched.processes[idx].pid as u64;
                    service.baseline = sched.effective_runtime_at(idx, now);
                }
            }
            for w in &mut state.workers {
                let Some(idx) = sched.processes.iter().position(|p| p.pid == w.pid) else {
                    return u64::MAX;
                };
                let p = &sched.processes[idx];
                if p.state != ProcessState::Ready {
                    return u64::MAX;
                }
                w.active = true;
                w.baseline = sched.effective_runtime_at(idx, now);
                w.ready_since = now;
                w.burst_start = p.burst_score;
            }
        }
        3 if value < N as u64 => {
            let idx = sched.current_process_index().unwrap();
            let w = &mut state.workers[value as usize];
            if current.pid != w.pid || !w.active {
                return u64::MAX;
            }
            if w.wait_count > 0 {
                let index = w.wait_count - 1;
                w.run_ns[index] += now.saturating_sub(w.selected_ns);
                w.reasons[index] = 64; // worker reached its wall deadline
            }
            w.runtime = sched
                .effective_runtime_at(idx, now)
                .saturating_sub(w.baseline);
            w.finish_ns = now;
            w.burst_end = current.burst_score;
            w.active = false;
        }
        4 => {
            if value != state.run || state.workers.iter().any(|w| w.active || w.finish_ns == 0) {
                return u64::MAX;
            }
            // Reporting is after measurement. Copy directly to bounded heap
            // storage, keeping the IRQ/syscall stack independent of trace size.
            let workers = state.workers.to_vec();
            let start = state.start;
            let mut services = state.services;
            for service in &mut services {
                if let Some(idx) = sched
                    .processes
                    .iter()
                    .position(|p| p.pid == service.pid as usize)
                {
                    service.cpu_ns = sched
                        .effective_runtime_at(idx, now)
                        .saturating_sub(service.baseline);
                }
            }
            let reaped = core::array::from_fn::<_, N, _>(|slot| {
                let w = &workers[slot];
                sched
                    .processes
                    .iter()
                    .find(|p| p.pid == w.pid)
                    .map(|p| p.state == ProcessState::Reaped)
                    .unwrap_or(true)
            });
            #[cfg(feature = "scheduler_timer_trace")]
            let (events, overflow, invalid_clock) = crate::scheduler_timer_trace::finish();
            let context_switches = switches(&sched).saturating_sub(state.switches);
            let elapsed_ns = now.saturating_sub(start);
            drop(sched);
            crate::serial_println!("[SCHED-BENCH] COST {{\"run\":\"1-{}-1\",\"elapsed_ns\":{},\"context_switches\":{}}}", value, elapsed_ns, context_switches);
            #[cfg(feature = "scheduler_timer_trace")]
            {
                crate::serial_println!("[SCHED-TIMER] TRACE {{\"run\":{},\"count\":{},\"overflow\":{},\"invalid_clock\":{}}}", value, events.len(), overflow, invalid_clock);
                for row in events {
                    crate::serial_println!("[SCHED-TIMER] EVENT {:?}", row);
                }
            }
            for (slot, service) in services.iter().enumerate() {
                let name = ["timer_server", "tty_server"][slot];
                crate::serial_println!("[SCHED-BENCH] SERVICE {{\"run\":\"1-{}-1\",\"name\":\"{}\",\"pid\":{},\"cpu_ns\":{},\"ready_count\":{},\"dispatches\":{},\"deadline_count\":{},\"deadline_late_ns\":{},\"deadline_max_ns\":{},\"overflow\":{},\"waits_ns\":{:?}}}", value, name, service.pid, service.cpu_ns, service.ready_count, service.dispatches, service.deadline_count, service.deadline_late_ns, service.deadline_max_ns, service.overflow, &service.waits[..service.count]);
            }
            for (slot, w) in workers.iter().enumerate() {
                crate::serial_println!("[SCHED-BENCH] WORKER {{\"run\":\"1-{}-1\",\"slot\":{},\"pid\":{},\"cpu_ns\":{},\"dispatches\":{},\"wait_ns\":{},\"wait_max_ns\":{},\"wait_count\":{},\"wait_overflow\":{},\"first_ns\":{},\"finish_ns\":{},\"burst_start\":{},\"burst_end\":{},\"nice\":{},\"mask\":{},\"invalid_observations\":{},\"reaped\":{}}}",
                    value, slot, w.pid, w.runtime, w.dispatches, w.wait_sum, w.wait_max,
                    w.wait_count, w.wait_overflow, w.first_ns.saturating_sub(start),
                    w.finish_ns.saturating_sub(start), w.burst_start, w.burst_end,
                    w.nice, w.mask, w.invalid_observations, reaped[slot]);
                for (index, ns) in w.waits[..w.wait_count].iter().enumerate() {
                    crate::serial_println!("[SCHED-BENCH] WAIT {{\"run\":\"1-{}-1\",\"slot\":{},\"index\":{},\"ns\":{},\"at_ns\":{},\"burst\":{},\"budget_ticks\":{},\"run_ns\":{},\"reason\":{}}}", value, slot, index, ns, w.dispatch_at[index], w.bursts[index], w.budgets[index], w.run_ns[index], w.reasons[index]);
                }
            }
            return 0;
        }
        _ => return u64::MAX,
    }
    0
}
