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

struct State {
    workers: [Worker; N],
    start: u64,
    run: u64,
}
struct Protected(UnsafeCell<State>);
// SAFETY: access is serialized by the existing global scheduler lock.
unsafe impl Sync for Protected {}
static STATE: Protected = Protected(UnsafeCell::new(State {
    workers: [Worker::ZERO; N],
    start: 0,
    run: 0,
}));

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
    for w in &mut state.workers {
        if !w.active {
            continue;
        }
        let Some(p) = sched.processes.iter().find(|p| p.pid == w.pid) else {
            w.invalid_observations += 1;
            continue;
        };
        if !matches!(p.state, ProcessState::Ready | ProcessState::Running)
            || p.nice != 0
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
            state.start = 0;
        }
        1 if value < N as u64 => {
            let w = &mut state.workers[value as usize];
            if w.pid != 0 || current.nice != 0 || current.cpu_mask & 1 == 0 {
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
            state.run = value;
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
            let reaped = core::array::from_fn::<_, N, _>(|slot| {
                let w = &workers[slot];
                sched
                    .processes
                    .iter()
                    .find(|p| p.pid == w.pid)
                    .map(|p| p.state == ProcessState::Reaped)
                    .unwrap_or(true)
            });
            drop(sched);
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
