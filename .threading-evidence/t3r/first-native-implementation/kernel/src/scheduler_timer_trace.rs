//! C1.2E observer. All calls require SCHEDULER, including begin/report.
//! Single CPU only; fixed storage, no allocation or serial output while active.
use core::cell::UnsafeCell;

use crate::sched::Scheduler;

const CAPACITY: usize = 65_536;
struct State {
    active: bool,
    count: usize,
    overflow: bool,
    invalid_clock: bool,
    last: u64,
    pending_timer: usize,
    events: [[u64; 7]; CAPACITY],
}
struct Protected(UnsafeCell<State>);
// SAFETY: every access is serialized by the existing global scheduler lock.
unsafe impl Sync for Protected {}
static STATE: Protected = Protected(UnsafeCell::new(State {
    active: false,
    count: 0,
    overflow: false,
    invalid_clock: false,
    last: 0,
    pending_timer: 0,
    events: [[0; 7]; CAPACITY],
}));

pub fn begin(sched: &Scheduler) {
    let state = unsafe { &mut *STATE.0.get() };
    state.active = sched.online_cores == 1;
    state.count = 0;
    state.overflow = false;
    state.invalid_clock = crate::arch::x86_64::interrupts::tsc_hz() == 0;
    state.last = 0;
    state.pending_timer = 0;
}

/// Row: kind, timestamp ns, CPU, subject pid, a, b, c.
/// 1 first Rust IRQ-handler timestamp after GPR save (a=entry ns,b=global tick);
/// 2 tick publication;
/// 3 queue publication (a=notify,b=nice+20,c=current pid);
/// 4 selection (a=outgoing pid,b=reason,c=nice+20);
/// 5 request (a=caller,b=generation,c=label);
/// 6 receive (a=caller/0,b=generation/tick,c=attributed service CPU ns);
/// 7 reply commit (a=caller,b=generation,c=service CPU ns);
/// 8 caller consumes reply (a=generation,b=label); 9 dispatch decision
/// (a=current pid,b=current nice+20,c=request|expired<<1|running<<2).
/// 11 actual BlockedOnIpc-to-Ready wake (a=nice+20,b=current pid,c=pending call).
/// 12 dispatch/reap checkpoint (a=stage, subject=process pid);
/// 13 Rust handler tail before dropping SCHEDULER (a=vector,b=switched,c=request);
/// 14 retained priority request (subject=selected pid,a=candidate pid).
pub fn event(sched: &Scheduler, kind: u64, pid: usize, a: u64, b: u64, c: u64) {
    let state = unsafe { &mut *STATE.0.get() };
    if !state.active {
        return;
    }
    let cpu = crate::sched::current_cpu_id();
    let now = sched.now_ns();
    state.invalid_clock |= cpu != 0 || sched.online_cores != 1 || now < state.last;
    state.last = now;
    if state.count == CAPACITY {
        state.overflow = true;
        return;
    }
    state.events[state.count] = [kind, now, cpu as u64, pid as u64, a, b, c];
    state.count += 1;
    if kind == 11 {
        state.pending_timer = pid;
    } else if kind == 4 && state.pending_timer == pid {
        state.pending_timer = 0;
    }
}

/// Dispatch checkpoints only while a timer wake is outstanding. Reap stages
/// (20+) are sparse and retained even outside a wake, to expose IRQ-off work.
pub fn checkpoint(sched: &Scheduler, stage: u64, pid: usize) {
    let state = unsafe { &*STATE.0.get() };
    if state.active && (state.pending_timer != 0 || stage >= 20) {
        event(sched, 12, pid, stage, 0, 0);
    }
}

pub fn receive(sched: &Scheduler, server_pid: usize, caller: usize, generation: u64) {
    if let Some(idx) = sched
        .processes
        .iter()
        .position(|p| p.pid == server_pid && p.name_str() == "timer_server")
    {
        event(
            sched,
            6,
            server_pid,
            caller as u64,
            generation,
            sched.effective_runtime_at(idx, sched.now_ns()),
        );
    }
}

pub fn finish() -> (alloc::vec::Vec<[u64; 7]>, bool, bool) {
    let state = unsafe { &mut *STATE.0.get() };
    state.active = false;
    (
        state.events[..state.count].to_vec(),
        state.overflow,
        state.invalid_clock,
    )
}
