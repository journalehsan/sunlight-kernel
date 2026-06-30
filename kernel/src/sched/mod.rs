//! SunBrust Distributed Work-Stealing Scheduler (SunlightOS)
//!
//! This module implements the "SunBrust" per-core distributed scheduler with
//! work-stealing. Each logical CPU owns a `CoreState` holding its local run
//! queues and currently-executing task; when a core's local queues are empty
//! it calls `steal_work()` to pop a task from the tail of another core's
//! lowest-priority tier queue, respecting per-process CPU affinity masks.
//!
//! ## Scheduling Model
//!
//! ### Burst Score (0..=1024)
//! - Low score (0-256)   => HIGH tier queue (favors interactive / short-running tasks)
//! - Mid score (257-768) => MEDIUM tier
//! - High score (769-1024)=> LOW tier (CPU-bound tasks get longer quanta but lower dispatch freq)
//!
//! Burst score is adjusted by:
//! - Early block / short run (< ~3 ticks or <2ms) => decrease burst (more interactive credit)
//! - Full quantum consumption => increase burst (CPU-bound behavior)
//! - Periodic aging for tasks waiting >100 ticks => mild decrease (anti-starvation)
//! - Short-burst churn penalty (Phase 4) => increase burst temporarily when a task
//!   runs for extremely short periods and blocks/yields immediately. This prevents
//!   thrashing tasks from causing excessive idle churn.
//!
//! Burst score is always clamped to [BURST_SCORE_MIN, BURST_SCORE_MAX].
//!
//! ### Nice Value (i8)
//! - Negative nice => higher priority. Tasks gain "credit" faster and may be
//!   promoted with a small quantum bonus.
//! - Positive nice => lower priority. Tasks accumulate "debt" and may be skipped
//!   for a cycle.
//! - nice==0 => pure round-robin (counter system bypassed).
//!
//! The effective quantum is:
//!   quantum = calculate_quantum_with_nice(burst_score, nice)
//!   where positive nice inflates the score (shorter slice) and negative deflates it.
//!
//! ### Quantum
//! - Base range: [QUANTUM_MIN, QUANTUM_MAX] ticks (~5..50 at 100 Hz).
//! - Interactive tasks (low burst) receive shorter quanta for responsiveness.
//! - CPU-bound (high burst) receive longer quanta to reduce switch overhead.
//!
//! quantum_override may be set by the nice promotion logic for one shot bonuses.
//!
//! ### Per-Core Tiered Queues (BORE mode)
//! Each `CoreState` maintains three ready queues (high/med/low). pick_next_bore
//! prefers high then medium then low. When all local queues are empty,
//! steal_work() attempts a try_lock-style steal from the tail of another core's
//! lowest-priority non-empty queue, checking cpu_mask affinity before taking.
//!
//! ### Aging & Starvation Prevention
//! - Every AGING_INTERVAL_TICKS, tasks that have waited > AGING_THRESHOLD_TICKS
//!   without running receive a small burst reduction (but never below
//!   MINIMUM_AGED_BURST_SCORE) and a starvation boost in the RR counter system.
//! - This guarantees forward progress for all ready tasks.
//!
//! ### CPU Accounting (for sunlight-top)
//! Each Process tracks:
//!   cpu_runtime_ns: u64   // exact accumulated on-CPU time (committed)
//!   last_start_ns: u64    // when it was last scheduled (monotonic); 0 if not charging
//!
//! On context switch out (deschedule):
//!   if prev was actually running:
//!       delta = now_ns() - prev.last_start_ns
//!       prev.cpu_runtime_ns += delta
//!
//! On context switch in:
//!   next.last_start_ns = now_ns()
//!
//! Effective runtime for sampling (used by telemetry for Running tasks):
//!   if state == Running:
//!       effective = cpu_runtime_ns + (now_ns() - last_start_ns)
//!   else:
//!       effective = cpu_runtime_ns
//!
//! The monotonic clock is a calibrated TSC (or tick fallback) providing ns resolution
//! with very low overhead (rdtsc + mul + shift, no floating point).
//!
//! ### SMP Phase 0 → Phase 1 Transition
//! BSP brings APs online via smp::start_aps(); main.rs then calls
//! sched::init_cores(total_cpu_count) which stores ONLINE_CORES and seeds the
//! online_cores field of the global Scheduler. Until then only CORES[0] (BSP)
//! is used. When per-core LAPIC timers are wired (phase 1 SMP), each AP's
//! timer IRQ will call schedule_tick(current_cpu_id(), saved_rsp) to dispatch
//! tasks from its own per-core queue or steal from neighbours.
//!
//! ## Invariants (Phase 2)
//! - Only Ready or Running processes may reside in the tiered ready queues.
//! - Blocking (IPC, timer, IO, waitpid, yield-to-block, exit, suspend) removes
//!   the task from ready queues before it stops being current.
//! - pick_next_* only returns Ready tasks (or falls back safely).

use crate::arch::x86_64::interrupts::now_ns;
use crate::process::{Process, ProcessState, QueueTier};
use crate::serial_println;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub const TIME_SLICE_TICKS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerMode {
    RoundRobin,
    Bore,
}

pub const SCHEDULER_MODE: SchedulerMode = SchedulerMode::RoundRobin;

// === Phase 3 BORE Scheduler Constants ===
pub const BURST_SCORE_MIN: u32 = 0;
pub const BURST_SCORE_MAX: u32 = 1024;
pub const BURST_SCORE_DEFAULT: u32 = 256;
pub const BURST_SCORE_HIGH: u32 = 256; // Interactive threshold
pub const BURST_SCORE_LOW: u32 = 768; // CPU-bound threshold

pub const BURST_REDUCTION_EARLY_BLOCK: u32 = 64; // ~6% reduction
pub const BURST_INCREASE_FULL_QUANTUM: u32 = 32; // ~3% increase
pub const BURST_REDUCTION_AGING: u32 = 20; // ~2% per aging tick

pub const AGING_INTERVAL_TICKS: u64 = 10;
pub const AGING_THRESHOLD_TICKS: u64 = 100; // Age after 100ms
pub const MINIMUM_AGED_BURST_SCORE: u32 = 256; // Don't starve below HIGH
pub const INTERACTIVE_DETECTION_THRESHOLD: u32 = 3; // Block < 3 ticks = interactive

// === Phase 4: short-burst / churn detection ===
const SHORT_BURST_NS: u64 = 2_000_000; // < 2 ms run is "extremely short"
const SHORT_BURST_PENALTY: u32 = 48; // temporary de-prio by bumping burst (lower prio)

// === Feature 3: Nice-weighted counter system (RoundRobin mode only) ===
pub const MAX_CREDIT: i32 = 6;
pub const MAX_DEBT: i32 = -8; // asymmetric: easier to be demoted
pub const PROMOTE_LIMIT: i32 = 3;
pub const SKIP_LIMIT: i32 = -3;
pub const DECAY_RATE: i32 = 1;
pub const STARVATION_BOOST: i32 = 2;

#[derive(Debug, Clone, Copy)]
pub enum BurstReason {
    EarlyBlock,  // Task blocked early (< 3 ticks)
    FullQuantum, // Task used full 10-tick quantum
    Aged,        // Task hasn't run in 100+ ticks
}

/// Update burst score based on why the task yielded
pub fn update_burst_score(process: &mut Process, reason: BurstReason) {
    match reason {
        BurstReason::EarlyBlock => {
            process.burst_score = process
                .burst_score
                .saturating_sub(BURST_REDUCTION_EARLY_BLOCK);
            process.interactive_bonus = 20;
        }
        BurstReason::FullQuantum => {
            process.burst_score = process
                .burst_score
                .saturating_add(BURST_INCREASE_FULL_QUANTUM)
                .min(BURST_SCORE_MAX);
            process.interactive_bonus = 0;
        }
        BurstReason::Aged => {
            process.burst_score = process
                .burst_score
                .saturating_sub(BURST_REDUCTION_AGING)
                .max(MINIMUM_AGED_BURST_SCORE);
            process.aging_counter += 1;
        }
    }
}

/// Per-core reschedule request mask.
///
/// A single global bit is not sufficient with per-core LAPIC timers: a timer
/// on one CPU can consume another CPU's block/yield wakeup request, leaving the
/// target CPU running a task that just transitioned to `BlockedOnIpc` or
/// leaving a woken live task `Ready` but not enqueued. Each CPU consumes only
/// its own bit; cross-core wakeups set the owner CPU's bit.
static RESCHEDULE_MASK: AtomicU64 = AtomicU64::new(0);

/// Diagnostic: track first time sunlightd (pid 6) is picked by scheduler.
static SUNLIGHTD_FIRST_SCHED: AtomicBool = AtomicBool::new(false);

/// === Diagnostic counters for process leak detection ===
static PROCESS_CREATED: AtomicUsize = AtomicUsize::new(0);
static PROCESS_FINISHED: AtomicUsize = AtomicUsize::new(0);

pub const QUANTUM_MIN: u32 = 5;
pub const QUANTUM_MAX: u32 = 50;

fn calculate_quantum_with_nice(burst_score: u32, nice: i8) -> u32 {
    let nice_modifier = (nice as i32) * 16;
    let effective_score =
        (burst_score as i32 + nice_modifier).clamp(0, BURST_SCORE_MAX as i32) as u32;
    let bonus_quantum = (effective_score * (QUANTUM_MAX - QUANTUM_MIN)) / BURST_SCORE_MAX;
    QUANTUM_MIN + bonus_quantum
}

/// Returns the quantum (in ticks) a process should run for, honoring any
/// promotion override set by the RoundRobin counter system. [FEAT-3]
fn quantum_ticks(process: &Process) -> u32 {
    process
        .quantum_override
        .unwrap_or_else(|| calculate_quantum_with_nice(process.burst_score, process.nice))
}

/// Accumulate the nice-weighted counter for a candidate process. [FEAT-3]
fn accumulate_counter(process: &mut Process) {
    if process.nice == 0 {
        return;
    }
    let factor: i32 = if process.burst_score > BURST_SCORE_LOW {
        2
    } else {
        1
    };
    if process.nice < 0 {
        let gain = ((-process.nice as i32) / 4 + 1) * factor;
        process.counter = (process.counter + gain).min(MAX_CREDIT);
    } else {
        let loss = (process.nice as i32 / 4 + 1) * factor;
        process.counter = (process.counter - loss).max(MAX_DEBT);
    }
}

/// Post-run housekeeping for the RoundRobin counter system. [FEAT-3]
fn on_task_ran(process: &mut Process, ticks_used: u64) {
    process.aging_boosted_this_pick = false;
    let q = quantum_ticks(process);
    process.quantum_override = None;
    if ticks_used < (q / 2) as u64 {
        process.counter /= 2;
    } else {
        if process.counter > 0 {
            process.counter -= DECAY_RATE;
        } else if process.counter < 0 {
            process.counter += DECAY_RATE;
        }
    }
}

// ─── Per-Core Work-Stealing Infrastructure ───────────────────────────────────

/// Maximum number of logical CPUs supported by the per-core scheduler.
pub const MAX_CORES: usize = 64;

/// Number of cores that have completed phase-0 init and can receive tasks.
/// Initialised to 1 (BSP only); updated by init_cores() after SMP bring-up.
pub static ONLINE_CORES: AtomicUsize = AtomicUsize::new(1);

/// Gate that allows AP cores to begin dispatching tasks.
///
/// APs `sti` and arm their LAPIC timers during `smp::start_aps`, but the BSP
/// then continues single-threaded boot (spawning init/vfs/tty, seeding the
/// run queues) with interrupts disabled. Until the BSP finishes and calls
/// `mark_scheduler_ready()` just before entering the first process, an AP timer
/// tick must NOT pull a task — the run queues are not seeded yet and the BSP is
/// still mutating process state. AP `schedule_tick` returns early until this is
/// set. The BSP (core 0) is never gated: it runs with IF=0 during boot so its
/// own timer cannot preempt it anyway.
pub static SCHEDULER_READY: AtomicBool = AtomicBool::new(false);

/// Signal that the BSP has finished boot and seeded the run queues, so AP
/// cores may begin dispatching tasks on their next timer tick.
pub fn mark_scheduler_ready() {
    SCHEDULER_READY.store(true, Ordering::Release);
}

/// Per-core scheduling state: local BORE-tiered run queues and current task.
///
/// All fields are protected by the global `SCHEDULER` spinlock during BSP-only
/// operation (SMP Phase 0). When per-core LAPIC timers are wired (Phase 1),
/// each CoreState gains its own `spin::Mutex` and steal_work() uses try_lock()
/// to avoid blocking on a busy victim — the current single-lock design is
/// correct because only the BSP acquires SCHEDULER.
pub struct CoreState {
    /// Highest-priority ready tasks (burst_score 0–256, interactive).
    pub run_queue_high: VecDeque<usize>,
    /// Medium-priority ready tasks (burst_score 257–768).
    pub run_queue_medium: VecDeque<usize>,
    /// Lowest-priority ready tasks (burst_score 769–1024, CPU-bound).
    /// Work-stealing steals from the *back* of this queue first, so the
    /// longest-waiting, least-urgent tasks migrate preferentially.
    pub run_queue_low: VecDeque<usize>,
    /// Index into Scheduler::processes for the task currently running on this
    /// core. None when the core is idle.
    pub current_task: Option<usize>,
    /// Ticks accumulated by current_task within its current quantum.
    pub current_ticks: u64,
    /// Total timer IRQ ticks handled on this core.
    pub timer_ticks: u64,
    /// Number of times a different task was switched onto this core.
    pub context_switches: u64,
}

impl CoreState {
    pub const fn new() -> Self {
        Self {
            run_queue_high: VecDeque::new(),
            run_queue_medium: VecDeque::new(),
            run_queue_low: VecDeque::new(),
            current_task: None,
            current_ticks: 0,
            timer_ticks: 0,
            context_switches: 0,
        }
    }

    fn total_ready(&self) -> usize {
        self.run_queue_high.len() + self.run_queue_medium.len() + self.run_queue_low.len()
    }
}

// ─── Scheduler ───────────────────────────────────────────────────────────────

pub struct Scheduler {
    pub processes: Vec<Process>,

    /// Per-core scheduling state. Only indices 0..online_cores are active.
    pub cores: [CoreState; MAX_CORES],
    /// Number of online cores (1 until init_cores() is called).
    pub online_cores: usize,

    pub global_tick: u64,
    pub idle_context_rsp: u64,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            processes: Vec::new(),
            cores: [const { CoreState::new() }; MAX_CORES],
            online_cores: 1,
            global_tick: 0,
            idle_context_rsp: 0,
        }
    }

    // ── Process management ───────────────────────────────────────────────────

    pub fn add_process(&mut self, process: Process) -> usize {
        let created_count = PROCESS_CREATED.fetch_add(1, Ordering::Relaxed);
        let online = self.online_cores;

        // Collect which process slots are currently running on any core so we
        // don't reclaim them while a core still has a reference.
        let mut in_use = [usize::MAX; MAX_CORES];
        for c in 0..online {
            in_use[c] = self.cores[c].current_task.unwrap_or(usize::MAX);
        }

        if let Some(id) = self
            .processes
            .iter()
            .enumerate()
            .find(|(idx, p)| {
                p.state == ProcessState::Finished && in_use[..online].iter().all(|&cur| cur != *idx)
            })
            .map(|(idx, _)| idx)
        {
            self.remove_from_ready_queues(id);
            serial_println!(
                "[SCHED] CREATED process #{} '{}' idx={} burst_score={} tier={:?} (reused slot)",
                created_count + 1,
                process.name_str(),
                id,
                process.burst_score,
                process.get_queue_tier()
            );
            self.processes[id] = process;
            return id;
        }

        let id = self.processes.len();
        serial_println!(
            "[SCHED] CREATED process #{} '{}' idx={} burst_score={} tier={:?}",
            created_count + 1,
            process.name_str(),
            id,
            process.burst_score,
            process.get_queue_tier()
        );
        self.processes.push(process);
        id
    }

    /// Choose the least-loaded core that the process's cpu_mask permits.
    fn target_core_for_process(&self, idx: usize) -> usize {
        let online = self.online_cores;
        let cpu_mask = if idx < self.processes.len() {
            self.processes[idx].cpu_mask
        } else {
            u64::MAX
        };

        let mut best_core = 0;
        let mut best_len = usize::MAX;
        for core_id in 0..online {
            if cpu_mask & (1u64 << core_id) == 0 {
                continue;
            }
            let len = self.cores[core_id].total_ready();
            if len < best_len {
                best_len = len;
                best_core = core_id;
            }
        }
        best_core
    }

    /// Enqueue a Ready process onto a specific core's tier queue.
    fn enqueue_process_to_core(&mut self, idx: usize, core_id: usize) {
        if idx >= self.processes.len() || !matches!(self.processes[idx].state, ProcessState::Ready)
        {
            return;
        }
        let tier = self.processes[idx].get_queue_tier();
        self.processes[idx].queued_on_core = core_id as u8;
        let core = &mut self.cores[core_id];
        match tier {
            QueueTier::High => core.run_queue_high.push_back(idx),
            QueueTier::Medium => core.run_queue_medium.push_back(idx),
            QueueTier::Low => core.run_queue_low.push_back(idx),
        }
    }

    /// Enqueue a Ready process to the least-loaded eligible core.
    pub fn enqueue_process(&mut self, idx: usize) {
        if idx >= self.processes.len() {
            return;
        }
        if !matches!(self.processes[idx].state, ProcessState::Ready) {
            return;
        }
        self.remove_from_ready_queues(idx);
        let core_id = self.target_core_for_process(idx);
        self.enqueue_process_to_core(idx, core_id);
    }

    /// Enqueue once, skipping if already present in any core's queue.
    pub fn enqueue_process_once(&mut self, idx: usize) {
        if idx >= self.processes.len()
            || !matches!(self.processes[idx].state, ProcessState::Ready)
            || self.is_queued(idx)
        {
            return;
        }
        self.enqueue_process(idx);
    }

    /// Remove a process index from ready queues.
    ///
    /// Uses `queued_on_core` to target the single core that owns this entry
    /// (O(queue_len / online_cores) instead of O(cores × queue_len)).
    pub fn remove_from_ready_queues(&mut self, idx: usize) {
        if idx >= self.processes.len() {
            return;
        }
        let qcore = self.processes[idx].queued_on_core;
        if qcore == u8::MAX {
            return;
        }
        let core_id = qcore as usize;
        if core_id < self.online_cores.min(MAX_CORES) {
            let core = &mut self.cores[core_id];
            core.run_queue_high.retain(|&q| q != idx);
            core.run_queue_medium.retain(|&q| q != idx);
            core.run_queue_low.retain(|&q| q != idx);
        }
        self.processes[idx].queued_on_core = u8::MAX;
    }

    /// True if `idx` is the live `current_task` of any online core.
    ///
    /// Such a task is still physically executing (or about to be descheduled)
    /// on that core, even if its bookkeeping `state` has already flipped to
    /// `Ready` (e.g. a cross-core wake landed between `block_on_recv` setting
    /// `BlockedOnIpc` and the owning core's next timer tick). It must NOT be
    /// enqueued by another core's wake path, or a second core could pick it up
    /// and run the same context concurrently — corrupting its saved RSP. The
    /// owning core re-enqueues it on its next `schedule_tick` deschedule.
    pub(crate) fn is_live_on_core(&self, idx: usize) -> bool {
        idx < self.processes.len() && self.processes[idx].owning_core != u8::MAX
    }

    /// Return the CPU that still owns `idx` as its current task, if any.
    pub(crate) fn live_owner_core(&self, idx: usize) -> Option<usize> {
        if idx < self.processes.len() {
            let c = self.processes[idx].owning_core;
            if c != u8::MAX {
                return Some(c as usize);
            }
        }
        None
    }

    /// Like `is_live_on_core`, but ignores `self_cpu`. O(1) via `owning_core`.
    pub(crate) fn is_live_on_other_core(&self, idx: usize, self_cpu: usize) -> bool {
        if idx >= self.processes.len() {
            return false;
        }
        let c = self.processes[idx].owning_core;
        c != u8::MAX && c as usize != self_cpu
    }

    fn is_queued(&self, idx: usize) -> bool {
        idx < self.processes.len() && self.processes[idx].queued_on_core != u8::MAX
    }

    /// Clear all core queues, then distribute all Ready processes (except
    /// `running_idx`) across the online cores.
    pub fn seed_ready_queues_except(&mut self, running_idx: usize) {
        let online = self.online_cores;
        for core_id in 0..online {
            let core = &mut self.cores[core_id];
            core.run_queue_high.clear();
            core.run_queue_medium.clear();
            core.run_queue_low.clear();
        }
        for idx in 0..self.processes.len() {
            if idx != running_idx && matches!(self.processes[idx].state, ProcessState::Ready) {
                self.enqueue_process(idx);
            }
        }
    }

    /// Set the idle thread's context RSP.
    pub fn set_idle_context(&mut self, rsp: u64) {
        self.idle_context_rsp = rsp;
    }

    /// Set a per-process watchdog period. [FEAT-1]
    pub fn set_process_watchdog(&mut self, pid: usize, period_ticks: u64) {
        if let Some(p) = self.process_mut_by_pid(pid) {
            p.wd_period_ticks = Some(period_ticks);
        }
    }

    /// Disable the per-process watchdog. [FEAT-1]
    pub fn clear_process_watchdog(&mut self, pid: usize) {
        if let Some(p) = self.process_mut_by_pid(pid) {
            p.wd_period_ticks = None;
        }
    }

    /// Reset a process's nice-weighted counter. [FEAT-3]
    pub fn reset_counter(&mut self, pid: usize) {
        if let Some(p) = self.process_mut_by_pid(pid) {
            p.counter = 0;
        }
    }

    #[inline]
    pub fn now_ns(&self) -> u64 {
        now_ns()
    }

    // ── CPU accounting ───────────────────────────────────────────────────────

    /// Stop charging runtime to the currently running process on this CPU and
    /// accumulate the elapsed delta. Returns bytes charged (0 if none).
    pub fn account_current_runtime(&mut self) -> u64 {
        let cpu_id = current_cpu_id();
        let current = match self.cores[cpu_id].current_task {
            Some(idx) => idx,
            None => return 0,
        };
        if current >= self.processes.len() {
            return 0;
        }
        let now = now_ns();
        let p = &mut self.processes[current];
        let mut delta: u64 = 0;
        if p.last_start_ns != 0 {
            delta = now.saturating_sub(p.last_start_ns);
            p.cpu_runtime_ns = p.cpu_runtime_ns.saturating_add(delta);
        }
        p.last_start_ns = 0;
        delta
    }

    /// Account runtime and apply the Phase 4 short-burst churn penalty.
    pub fn account_and_apply_churn_penalty(&mut self) -> u64 {
        let delta = self.account_current_runtime();
        if delta > 0 && delta < SHORT_BURST_NS {
            let cpu_id = current_cpu_id();
            if let Some(current) = self.cores[cpu_id].current_task {
                if current < self.processes.len() {
                    let p = &mut self.processes[current];
                    p.burst_score = p
                        .burst_score
                        .saturating_add(SHORT_BURST_PENALTY)
                        .min(BURST_SCORE_MAX);
                }
            }
        }
        delta
    }

    /// Begin charging CPU time to the given process.
    pub fn start_charging_runtime(&mut self, idx: usize) {
        if idx >= self.processes.len() {
            return;
        }
        self.processes[idx].last_start_ns = now_ns();
    }

    /// Compute effective runtime including uncommitted time for a Running task.
    #[inline]
    pub fn effective_runtime_ns(&self, idx: usize) -> u64 {
        if idx >= self.processes.len() {
            return 0;
        }
        let p = &self.processes[idx];
        if p.state == ProcessState::Running && p.last_start_ns != 0 {
            let now = now_ns();
            p.cpu_runtime_ns
                .saturating_add(now.saturating_sub(p.last_start_ns))
        } else {
            p.cpu_runtime_ns
        }
    }

    // ── State management ─────────────────────────────────────────────────────

    pub fn set_state(&mut self, idx: usize, new_state: ProcessState) {
        if idx >= self.processes.len() {
            return;
        }
        self.processes[idx].state = new_state;
        let runnable = matches!(new_state, ProcessState::Ready | ProcessState::Running);
        if !runnable {
            self.remove_from_ready_queues(idx);
        }
    }

    // ── Timer tick ───────────────────────────────────────────────────────────

    /// Called from the timer IRQ on each tick. Updates quantum tracking and
    /// requests a local reschedule when the current task's quantum expires.
    pub fn tick(&mut self) {
        let cpu_id = current_cpu_id();
        // Only the BSP (CPU 0) drives the global tick counter.
        // With N cores each firing at 100 Hz, incrementing on every core would
        // advance global_tick N× faster, making aging and the diagnostic report
        // fire far too often and misrepresenting elapsed time to userland.
        if cpu_id == 0 {
            self.global_tick += 1;
        }
        self.cores[cpu_id].current_ticks += 1;
        self.cores[cpu_id].timer_ticks += 1;

        let current = match self.cores[cpu_id].current_task {
            Some(idx) => idx,
            None => {
                request_reschedule_on(cpu_id);
                return;
            }
        };

        if current >= self.processes.len() {
            return;
        }

        if !matches!(self.processes[current].state, ProcessState::Running) {
            self.cores[cpu_id].current_ticks = 0;
            request_reschedule_on(cpu_id);
            return;
        }

        let quantum = quantum_ticks(&self.processes[current]) as u64;

        if self.cores[cpu_id].current_ticks >= quantum {
            let ticks_used = self.cores[cpu_id].current_ticks;
            let current_proc = &mut self.processes[current];
            current_proc.timeslice_used = ticks_used as u32;

            update_burst_score(current_proc, BurstReason::FullQuantum);

            if SCHEDULER_MODE == SchedulerMode::RoundRobin {
                on_task_ran(current_proc, ticks_used);
            } else {
                current_proc.quantum_override = None;
                current_proc.aging_boosted_this_pick = false;
            }

            // [FEAT-1] Per-process watchdog.
            if let Some(wd) = current_proc.wd_period_ticks {
                if ticks_used > wd {
                    self.account_current_runtime();
                    self.processes[current].state = ProcessState::Suspended;
                    self.remove_from_ready_queues(current);
                    serial_println!(
                        "[WD] pid={} '{}' exceeded watchdog: ran={} limit={} ticks — suspended",
                        self.processes[current].pid,
                        self.processes[current].name_str(),
                        ticks_used,
                        wd
                    );
                }
            }

            self.age_ready_tasks();
            self.cores[cpu_id].current_ticks = 0;
            request_reschedule_on(cpu_id);
        }

        if self.global_tick.is_multiple_of(1000) {
            self.diagnostic_report();
        }
    }

    fn age_ready_tasks(&mut self) {
        if !self.global_tick.is_multiple_of(AGING_INTERVAL_TICKS) {
            return;
        }
        for idx in 0..self.processes.len() {
            let p = &mut self.processes[idx];
            if !matches!(p.state, ProcessState::Ready) {
                continue;
            }
            let ticks_since_run = self.global_tick - p.last_run_tick;
            if ticks_since_run > AGING_THRESHOLD_TICKS {
                update_burst_score(p, BurstReason::Aged);
            }
        }
    }

    // ── Work stealing ────────────────────────────────────────────────────────

    /// Search a queue (back to front) for a process whose cpu_mask permits
    /// `thief_id`. Returns the queue index (not process index) if found.
    fn find_stealable(
        queue: &VecDeque<usize>,
        processes: &[Process],
        thief_id: usize,
    ) -> Option<usize> {
        for i in (0..queue.len()).rev() {
            let pidx = queue[i];
            if pidx < processes.len()
                && matches!(processes[pidx].state, ProcessState::Ready)
                && processes[pidx].cpu_mask & (1u64 << thief_id) != 0
            {
                return Some(i);
            }
        }
        None
    }

    /// Iterate all other online cores and steal one task from the tail of their
    /// lowest-priority non-empty queue. Respects cpu_mask affinity.
    ///
    /// In a future SMP Phase 1 design each CoreState will have its own
    /// `spin::Mutex`; stealing will use `try_lock()` to skip busy victims
    /// without blocking the thief's timer handler. Under the current
    /// single-lock design (all CoreState under SCHEDULER's lock) the
    /// "skip if empty or insufficient" check plays the same role.
    pub fn steal_work(&mut self, thief_id: usize) -> Option<usize> {
        let online = self.online_cores;
        for victim_id in 0..online {
            if victim_id == thief_id {
                continue;
            }
            // Phase 1: inspect victim's queues under shared SCHEDULER lock.
            // We need separate borrows of cores[victim_id] and processes.
            let low_pos = {
                let core = &self.cores[victim_id];
                // Only steal if victim has more than one task (leave them work).
                if core.total_ready() <= 1 {
                    None
                } else {
                    Self::find_stealable(&core.run_queue_low, &self.processes, thief_id)
                        .map(|p| (2u8, p))
                        .or_else(|| {
                            Self::find_stealable(&core.run_queue_medium, &self.processes, thief_id)
                                .map(|p| (1u8, p))
                        })
                        .or_else(|| {
                            Self::find_stealable(&core.run_queue_high, &self.processes, thief_id)
                                .map(|p| (0u8, p))
                        })
                }
            };

            // Phase 2: perform the steal.
            if let Some((tier, pos)) = low_pos {
                let core = &mut self.cores[victim_id];
                let stolen = match tier {
                    2 => core.run_queue_low.remove(pos),
                    1 => core.run_queue_medium.remove(pos),
                    _ => core.run_queue_high.remove(pos),
                };
                if let Some(pidx) = stolen {
                    if let Some(p) = self.processes.get_mut(pidx) {
                        p.queued_on_core = u8::MAX;
                    }
                    return Some(pidx);
                }
            }
        }
        None
    }

    // ── Pick-next ────────────────────────────────────────────────────────────

    /// Pick the next Ready process using BORE tiered queues for `cpu_id`.
    /// Falls back to work-stealing if local queues are empty.
    pub fn pick_next_bore(&mut self, cpu_id: usize) -> Option<usize> {
        let current = self.cores[cpu_id].current_task.unwrap_or(usize::MAX);
        let mut skipped_high: Option<usize> = None;
        let mut skipped_medium: Option<usize> = None;
        let mut skipped_low: Option<usize> = None;

        if let Some(idx) = pop_ready_excluding_current(
            &mut self.cores[cpu_id].run_queue_high,
            &self.processes,
            current,
            &mut skipped_high,
        ) {
            if let Some(c) = skipped_high {
                self.enqueue_process_once(c);
            }
            self.processes[idx].queued_on_core = u8::MAX;
            return Some(idx);
        }

        if let Some(idx) = pop_ready_excluding_current(
            &mut self.cores[cpu_id].run_queue_medium,
            &self.processes,
            current,
            &mut skipped_medium,
        ) {
            if let Some(c) = skipped_high {
                self.enqueue_process_once(c);
            }
            if let Some(c) = skipped_medium {
                self.enqueue_process_once(c);
            }
            self.processes[idx].queued_on_core = u8::MAX;
            return Some(idx);
        }

        if let Some(idx) = pop_ready_excluding_current(
            &mut self.cores[cpu_id].run_queue_low,
            &self.processes,
            current,
            &mut skipped_low,
        ) {
            if let Some(c) = skipped_high {
                self.enqueue_process_once(c);
            }
            if let Some(c) = skipped_medium {
                self.enqueue_process_once(c);
            }
            if let Some(c) = skipped_low {
                self.enqueue_process_once(c);
            }
            self.processes[idx].queued_on_core = u8::MAX;
            return Some(idx);
        }

        if let Some(c) = skipped_high.or(skipped_medium).or(skipped_low) {
            // This item was popped from the queue (skipped because it's `current`),
            // then immediately re-enqueued — queued_on_core is already set by re-enqueue.
            return Some(c);
        }

        // Local queues empty — attempt work stealing.
        if let Some(stolen) = self.steal_work(cpu_id) {
            return Some(stolen);
        }

        // Last-resort linear scan (safety net for races during queue migration).
        let len = self.processes.len();
        if len == 0 {
            return None;
        }
        let start = (current.wrapping_add(1)) % len;
        let mut idx = start;
        loop {
            // Skip any process that is the live `current_task` of another core.
            // A woken-while-live task is Ready but deliberately not enqueued (it
            // is still executing/owned by its core); the run queues won't offer
            // it, but this raw state scan would — and dispatching it here would
            // run the same context on two cores at once. Let its owning core
            // re-dispatch it.
            if matches!(self.processes[idx].state, ProcessState::Ready)
                && !self.is_live_on_other_core(idx, cpu_id)
            {
                serial_println!(
                    "[SCHED] WARNING: pick_next_bore fallback linear search idx={}",
                    idx
                );
                return Some(idx);
            }
            idx = (idx + 1) % len;
            if idx == start {
                break;
            }
        }
        None
    }

    /// Pick the next Ready process for RoundRobin mode for `cpu_id`. [FEAT-3]
    pub fn pick_next_round_robin(&mut self, cpu_id: usize) -> Option<usize> {
        let len = self.processes.len();
        if len == 0 {
            return None;
        }

        let current = self.cores[cpu_id].current_task.unwrap_or(0);
        let mut attempts = len;
        let start = (current + 1) % len;
        let mut idx = start;

        loop {
            if attempts == 0 {
                break;
            }
            attempts -= 1;

            // Not eligible if it isn't Ready, or if it is still the live
            // `current_task` of another core. RoundRobin ignores the run queues
            // and selects purely by state, so without the live-on-core check a
            // process woken (set Ready) while still executing on its owning core
            // would be dispatched here too — running the same context on two
            // cores at once. Its owning core re-selects it on its next tick.
            if !matches!(self.processes[idx].state, ProcessState::Ready)
                || self.is_live_on_other_core(idx, cpu_id)
            {
                idx = (idx + 1) % len;
                if idx == start && attempts == 0 {
                    break;
                }
                continue;
            }

            // Starvation rescue BEFORE accumulate_counter.
            let ticks_waiting = self
                .global_tick
                .saturating_sub(self.processes[idx].last_run_tick);
            if ticks_waiting > AGING_THRESHOLD_TICKS && !self.processes[idx].aging_boosted_this_pick
            {
                self.processes[idx].counter =
                    (self.processes[idx].counter + STARVATION_BOOST).min(MAX_CREDIT);
                self.processes[idx].aging_boosted_this_pick = true;
                update_burst_score(&mut self.processes[idx], BurstReason::Aged);
            }

            accumulate_counter(&mut self.processes[idx]);

            // High priority promotion.
            if self.processes[idx].nice < 0
                && self.processes[idx].counter >= PROMOTE_LIMIT
                && self.processes[idx].quantum_override.is_none()
            {
                self.processes[idx].counter =
                    0_i32.max(self.processes[idx].counter - PROMOTE_LIMIT);
                let base_q = calculate_quantum_with_nice(
                    self.processes[idx].burst_score,
                    self.processes[idx].nice,
                );
                let quantum_override_value = ((base_q * 110) / 100).min(QUANTUM_MAX);
                self.processes[idx].quantum_override = Some(quantum_override_value);
                self.processes[idx].aging_boosted_this_pick = false;
                return Some(idx);
            }

            // Low priority skip.
            if self.processes[idx].nice > 0 && self.processes[idx].counter <= SKIP_LIMIT {
                self.processes[idx].counter += DECAY_RATE;
                self.processes[idx].aging_boosted_this_pick = false;
                idx = (idx + 1) % len;
                if idx == start && attempts == 0 {
                    break;
                }
                continue;
            }

            self.processes[idx].aging_boosted_this_pick = false;
            return Some(idx);
        }

        // All tasks skipped — find least-indebted, or try stealing.
        let mut best_idx = None;
        let mut best_counter = i32::MIN;
        let mut scan = start;
        for _ in 0..len {
            if matches!(self.processes[scan].state, ProcessState::Ready)
                && !self.is_live_on_other_core(scan, cpu_id)
                && self.processes[scan].counter > best_counter
            {
                best_counter = self.processes[scan].counter;
                best_idx = Some(scan);
            }
            scan = (scan + 1) % len;
        }

        if let Some(idx) = best_idx {
            serial_println!(
                "[SCHED-RR] fallback: all tasks in debt, picking least-indebted idx={} counter={}",
                idx,
                best_counter
            );
            return Some(idx);
        }

        // Absolutely nothing local — try stealing.
        self.steal_work(cpu_id)
    }

    pub fn pick_next(&mut self, cpu_id: usize) -> Option<usize> {
        let next = match SCHEDULER_MODE {
            SchedulerMode::RoundRobin => self.pick_next_round_robin(cpu_id),
            SchedulerMode::Bore => self.pick_next_bore(cpu_id),
        };
        if let Some(idx) = next {
            let next_pid = self.processes[idx].pid;
            if next_pid == 6 && !SUNLIGHTD_FIRST_SCHED.swap(true, Ordering::SeqCst) {
                serial_println!(
                    "[SUNLIGHTD-SCHED] First time scheduled, rip=0x{:x} idx={}",
                    self.processes[idx].entry_point,
                    idx
                );
            }
        }
        next
    }

    // ── Per-core dispatch ────────────────────────────────────────────────────

    /// Central per-core scheduling dispatch.
    ///
    /// Called from the timer handler with the ID of the CPU whose timer fired
    /// and the kernel RSP saved by the naked interrupt entry stub. Returns the
    /// RSP of the next process to resume (0 = stay on current context).
    pub fn schedule_tick(&mut self, cpu_id: usize, saved_rsp: u64) -> u64 {
        // AP cores stay idle until the BSP has finished boot and seeded the run
        // queues. Without this, an AP timer tick during boot would dispatch a
        // not-yet-seeded task (and contend with the BSP's PMM-heavy boot).
        // Core 0 (BSP) is never gated.
        if cpu_id != 0 && !SCHEDULER_READY.load(Ordering::Acquire) {
            return 0;
        }

        if !check_reschedule_on(cpu_id) {
            return 0;
        }

        // ── Idle-AP fast path ─────────────────────────────────────────────────
        // When an AP has no current task (it was in its idle hlt loop), try to
        // pick a task immediately.  There is nothing to save or re-queue.
        if self.cores[cpu_id].current_task.is_none() {
            if let Some(next) = self.pick_next(cpu_id) {
                let next_rsp       = self.processes[next].context_rsp;
                let next_stack_top = self.processes[next].kernel_stack_top;
                let next_fs_base   = self.processes[next].fs_base;
                self.cores[cpu_id].current_task   = Some(next);
                self.cores[cpu_id].context_switches += 1;
                self.processes[next].state          = ProcessState::Running;
                self.processes[next].last_run_tick  = self.global_tick;
                self.processes[next].owning_core    = cpu_id as u8;
                // Do NOT clear queued_on_core here. In BORE mode pick_next_bore
                // already cleared it when it popped the process. In RR mode the
                // process is never popped from the queue, so queued_on_core must
                // remain valid so remove_from_ready_queues can find and evict it
                // when the process later blocks.
                self.start_charging_runtime(next);
                unsafe {
                    self.processes[next].address_space.activate();
                    x86_64::registers::model_specific::Msr::new(0xC0000100).write(next_fs_base);
                }
                crate::arch::x86_64::smp::set_current_cpu_tss_rsp0(next_stack_top);
                return next_rsp;
            }
            return 0;
        }

        let current = self.cores[cpu_id].current_task.unwrap();

        if current >= self.processes.len() {
            return 0;
        }

        self.account_and_apply_churn_penalty();

        // Save interrupted context.
        self.processes[current].context_rsp = saved_rsp;
        self.processes[current].fs_base =
            unsafe { x86_64::registers::model_specific::Msr::new(0xC0000100).read() };

        // Maintain runnable-only queue invariant.
        let was_runnable = matches!(
            self.processes[current].state,
            ProcessState::Ready | ProcessState::Running
        );
        if self.processes[current].state == ProcessState::Running {
            self.processes[current].state = ProcessState::Ready;
        }
        if matches!(self.processes[current].state, ProcessState::Ready) {
            if SCHEDULER_MODE == SchedulerMode::Bore {
                self.enqueue_process_once(current);
            }
        } else if !was_runnable || !matches!(self.processes[current].state, ProcessState::Ready) {
            self.remove_from_ready_queues(current);
        }

        if let Some(next) = self.pick_next(cpu_id) {
            let next_rsp = self.processes[next].context_rsp;
            let next_stack_top = self.processes[next].kernel_stack_top;
            let next_fs_base = self.processes[next].fs_base;
            let prev = current;

            if next != prev {
                self.cores[cpu_id].context_switches += 1;
                self.processes[prev].owning_core = u8::MAX;
            }
            self.cores[cpu_id].current_task = Some(next);
            self.processes[next].state = ProcessState::Running;
            self.processes[next].last_run_tick = self.global_tick;
            self.processes[next].owning_core = cpu_id as u8;
            // queued_on_core is not cleared here — see idle-fast-path comment.
            self.start_charging_runtime(next);

            unsafe {
                self.processes[next].address_space.activate();
                x86_64::registers::model_specific::Msr::new(0xC0000100).write(next_fs_base);
            }
            // Update this core's TSS RSP0 so the next interrupt from ring-3
            // delivers to the correct kernel stack.  Uses the per-core TSS
            // (BSP → global TSS; APs → AP_TSS_STORE[cpu_id-1]).
            crate::arch::x86_64::smp::set_current_cpu_tss_rsp0(next_stack_top);

            if self.processes[prev].state == ProcessState::Finished {
                self.reap_process_resources(prev);
            }

            next_rsp
        } else {
            0
        }
    }

    // ── Current process accessors ────────────────────────────────────────────

    pub fn current_process(&self) -> &Process {
        let cpu_id = current_cpu_id();
        &self.processes[self.cores[cpu_id].current_task.unwrap_or(0)]
    }

    pub fn current_process_mut(&mut self) -> &mut Process {
        let cpu_id = current_cpu_id();
        let idx = self.cores[cpu_id].current_task.unwrap_or(0);
        &mut self.processes[idx]
    }

    // ── Wake / unblock ───────────────────────────────────────────────────────

    /// Return the PID of the process currently running on this CPU (0 if none).
    pub fn current_pid(&self) -> usize {
        let cpu_id = current_cpu_id();
        self.cores[cpu_id]
            .current_task
            .and_then(|idx| self.processes.get(idx))
            .map(|p| p.pid)
            .unwrap_or(0)
    }

    pub fn is_blocked_on_recv(&self, pid: usize) -> bool {
        self.processes
            .iter()
            .any(|p| p.pid == pid && p.state == ProcessState::BlockedOnIpc)
    }

    pub fn wake_pid(&mut self, pid: usize) {
        let idx = match self.processes.iter().position(|p| p.pid == pid) {
            Some(i) => i,
            None => return,
        };

        if self.processes[idx].state == ProcessState::BlockedOnIpc {
            let ticks_blocked = self.global_tick - self.processes[idx].block_start_tick;
            if ticks_blocked < INTERACTIVE_DETECTION_THRESHOLD as u64 {
                update_burst_score(&mut self.processes[idx], BurstReason::EarlyBlock);
            }
            self.processes[idx].state = ProcessState::Ready;
            self.remove_from_ready_queues(idx);
            if let Some(cpu_id) = self.live_owner_core(idx) {
                request_reschedule_on(cpu_id);
            } else {
                self.enqueue_process(idx);
            }
        }
    }

    pub fn wake_timer_pid(&mut self, pid: usize) {
        let idx = match self.processes.iter().position(|p| p.pid == pid) {
            Some(i) => i,
            None => return,
        };
        if self.processes[idx].state == ProcessState::BlockedOnTimer {
            let ticks_blocked = self.global_tick - self.processes[idx].block_start_tick;
            if ticks_blocked < INTERACTIVE_DETECTION_THRESHOLD as u64 {
                update_burst_score(&mut self.processes[idx], BurstReason::EarlyBlock);
            }
            self.processes[idx].state = ProcessState::Ready;
            self.remove_from_ready_queues(idx);
            if let Some(cpu_id) = self.live_owner_core(idx) {
                request_reschedule_on(cpu_id);
            } else {
                self.enqueue_process(idx);
            }
        }
    }

    pub fn wake_io_pid(&mut self, pid: usize) {
        let idx = match self.processes.iter().position(|p| p.pid == pid) {
            Some(i) => i,
            None => return,
        };
        if self.processes[idx].state == ProcessState::BlockedOnIo {
            let ticks_blocked = self.global_tick - self.processes[idx].block_start_tick;
            if ticks_blocked < INTERACTIVE_DETECTION_THRESHOLD as u64 {
                update_burst_score(&mut self.processes[idx], BurstReason::EarlyBlock);
            }
            self.processes[idx].state = ProcessState::Ready;
            self.remove_from_ready_queues(idx);
            if let Some(cpu_id) = self.live_owner_core(idx) {
                request_reschedule_on(cpu_id);
            } else {
                self.enqueue_process(idx);
            }
        }
    }

    pub fn process_mut_by_pid(&mut self, pid: usize) -> Option<&mut Process> {
        self.processes.iter_mut().find(|p| p.pid == pid)
    }

    pub fn process_index_by_pid(&self, pid: usize) -> Option<usize> {
        self.processes.iter().position(|p| p.pid == pid)
    }

    pub fn wake_for_signal(&mut self, pid: usize) {
        let Some(idx) = self.process_index_by_pid(pid) else {
            return;
        };
        match self.processes[idx].state {
            ProcessState::BlockedOnIpc
            | ProcessState::BlockedOnTimer
            | ProcessState::BlockedOnIo
            | ProcessState::Suspended => {
                self.processes[idx].state = ProcessState::Ready;
                self.processes[idx].block_start_tick = self.global_tick;
                self.remove_from_ready_queues(idx);
                if let Some(cpu_id) = self.live_owner_core(idx) {
                    request_reschedule_on(cpu_id);
                } else {
                    self.enqueue_process(idx);
                }
            }
            _ => {}
        }
    }

    // ── Process lifecycle ────────────────────────────────────────────────────

    fn address_space_is_shared(&self, idx: usize) -> bool {
        let pml4 = self.processes[idx].address_space.pml4_phys;
        self.processes.iter().enumerate().any(|(other_idx, proc)| {
            other_idx != idx
                && proc.state != ProcessState::Finished
                && proc.address_space.pml4_phys == pml4
        })
    }

    pub fn reap_process_resources(&mut self, idx: usize) {
        if idx >= self.processes.len() || !self.processes[idx].exit_cleanup_pending {
            return;
        }

        let pid = self.processes[idx].pid;
        let free_root = !self.address_space_is_shared(idx);
        let hhdm_offset = match crate::HHDM_REQ.response() {
            Some(resp) => x86_64::VirtAddr::new(resp.offset),
            None => return,
        };

        crate::memory::swap::untrack_process(pid);

        let endpoint_ids = {
            let caps = crate::capability::CAP_BROKER.lock();
            caps.endpoints_owned_by(pid)
        };

        {
            crate::ipc::for_all_shards(|bus| {
                bus.remove_pid_references(pid);
            });
            for endpoint_id in &endpoint_ids {
                crate::ipc::with_shard(*endpoint_id, |bus| bus.remove_endpoint(*endpoint_id));
            }
        }

        {
            let mut pmm = crate::PMM.lock();
            let mut caps = crate::capability::CAP_BROKER.lock();
            caps.revoke_endpoints_owned_by(pid);
            crate::memory::shared::cleanup_shared_pages(
                &mut self.processes[idx],
                &mut *pmm,
                &mut *caps,
            );
        }

        for proc in self.processes.iter_mut() {
            if proc
                .ipc_reply_target
                .is_some_and(|(_, client_pid)| client_pid == pid)
            {
                proc.ipc_reply_target = None;
            }
        }

        let reclaim = {
            let mut pmm = crate::PMM.lock();
            unsafe {
                self.processes[idx].address_space.reclaim_user_space(
                    &mut *pmm,
                    hhdm_offset,
                    free_root,
                )
            }
        };

        self.processes[idx].ipc_queue.clear();
        self.processes[idx].ipc_reply = None;
        self.processes[idx].ipc_endpoint = None;
        self.processes[idx].pending_call = None;
        self.processes[idx].pending_reply_wait = None;
        self.processes[idx].ipc_reply_target = None;
        self.processes[idx].capabilities.clear();
        self.processes[idx].exit_cleanup_pending = false;

        serial_println!(
            "[SCHED] reaped pid={} user_frames={} page_tables={} swap_blocks={}",
            pid,
            reclaim.user_frames,
            reclaim.page_tables,
            reclaim.swap_blocks
        );
        crate::PMM.lock().diagnostic_report_pid(pid as u32);
    }

    pub fn terminate_process_by_pid(&mut self, pid: usize, code: i32, reason: &str) -> bool {
        let Some(idx) = self.process_index_by_pid(pid) else {
            return false;
        };
        // Refuse to terminate a task that is currently executing on any core.
        let is_running_on_core = self.processes[idx].owning_core != u8::MAX;
        if is_running_on_core || self.processes[idx].state == ProcessState::Finished {
            return false;
        }

        self.processes[idx].exit_code = code;
        self.processes[idx].state = ProcessState::Finished;
        self.processes[idx].exit_cleanup_pending = true;
        self.remove_from_ready_queues(idx);
        note_process_finished(pid, self.processes[idx].name_str());
        serial_println!(
            "[SCHED] terminating pid={} name='{}' reason={}",
            pid,
            self.processes[idx].name_str(),
            reason
        );

        let parent_pid = self.processes[idx].ppid;
        let parent_waiting = self
            .process_mut_by_pid(parent_pid)
            .is_some_and(|parent| parent.wait_child == Some(pid));
        if parent_waiting {
            self.wake_pid(parent_pid);
        }

        self.reap_process_resources(idx);
        true
    }

    pub fn get_process_burst_info(&self, pid: usize) -> Option<(u32, ProcessState)> {
        self.processes
            .iter()
            .find(|p| p.pid == pid)
            .map(|p| (p.burst_score, p.state))
    }

    // ── Entry point ──────────────────────────────────────────────────────────

    /// Enter the first ready process; never returns.
    pub fn run_forever(&mut self) -> ! {
        let mut first = None;
        for (i, p) in self.processes.iter().enumerate() {
            serial_println!(
                "[SCHED] process {} '{}' state={:?}",
                i,
                p.name_str(),
                p.state
            );
            if matches!(p.state, ProcessState::Ready) {
                first = Some(i);
                break;
            }
        }

        if let Some(idx) = first {
            self.cores[0].current_task = Some(idx);
            self.processes[idx].state = ProcessState::Running;
            self.processes[idx].last_run_tick = self.global_tick;
            self.processes[idx].owning_core = 0;
            self.processes[idx].queued_on_core = u8::MAX;
            self.start_charging_runtime(idx);

            for i in 0..self.processes.len() {
                if i != idx && matches!(self.processes[i].state, ProcessState::Ready) {
                    self.enqueue_process_once(i);
                }
            }

            let rsp = self.processes[idx].context_rsp;
            serial_println!(
                "[SCHED] Entering process {} '{}' at rsp={:#x}",
                idx,
                self.processes[idx].name_str(),
                rsp
            );
            unsafe {
                self.processes[idx].address_space.activate();
            }
            unsafe {
                context::iretq_to_context(rsp);
            }
        }

        serial_println!("[SCHED] No user processes, entering idle");
        idle_loop();
    }

    // ── Diagnostics ──────────────────────────────────────────────────────────

    pub fn diagnostic_report(&self) {
        let created = PROCESS_CREATED.load(Ordering::Relaxed);
        let finished = PROCESS_FINISHED.load(Ordering::Relaxed);
        let alive = self
            .processes
            .iter()
            .filter(|p| p.state != ProcessState::Finished)
            .count();
        let finished_slots = self.processes.len().saturating_sub(alive);

        let ready_high: usize = (0..self.online_cores)
            .map(|c| self.cores[c].run_queue_high.len())
            .sum();
        let ready_mid: usize = (0..self.online_cores)
            .map(|c| self.cores[c].run_queue_medium.len())
            .sum();
        let ready_low: usize = (0..self.online_cores)
            .map(|c| self.cores[c].run_queue_low.len())
            .sum();

        let blocked_ipc = self
            .processes
            .iter()
            .filter(|p| p.state == ProcessState::BlockedOnIpc)
            .count();
        let blocked_timer = self
            .processes
            .iter()
            .filter(|p| p.state == ProcessState::BlockedOnTimer)
            .count();
        let blocked_io = self
            .processes
            .iter()
            .filter(|p| p.state == ProcessState::BlockedOnIo)
            .count();

        serial_println!(
            "[SCHED-DIAG] created={} finished={} alive={} finished_slots={} ready_queues=({},{},{}) delta_created-finished={} blocked_ipc={} blocked_timer={} blocked_io={} online_cores={}",
            created, finished, alive, finished_slots, ready_high, ready_mid, ready_low,
            created.saturating_sub(finished),
            blocked_ipc, blocked_timer, blocked_io, self.online_cores
        );
        for core_id in 0..self.online_cores {
            match self.cores[core_id].current_task {
                Some(idx) if idx < self.processes.len() => {
                    let process = &self.processes[idx];
                    serial_println!(
                        "[SCHED-DIAG] core={} current_idx={} pid={} name='{}' state={:?} ticks={} queues=({},{},{})",
                        core_id,
                        idx,
                        process.pid,
                        process.name_str(),
                        process.state,
                        self.cores[core_id].current_ticks,
                        self.cores[core_id].run_queue_high.len(),
                        self.cores[core_id].run_queue_medium.len(),
                        self.cores[core_id].run_queue_low.len()
                    );
                }
                _ => {
                    serial_println!(
                        "[SCHED-DIAG] core={} current_idx=none ticks={} queues=({},{},{})",
                        core_id,
                        self.cores[core_id].current_ticks,
                        self.cores[core_id].run_queue_high.len(),
                        self.cores[core_id].run_queue_medium.len(),
                        self.cores[core_id].run_queue_low.len()
                    );
                }
            }
        }

        if alive > 0 && blocked_ipc == alive
            || (ready_high + ready_mid + ready_low == 0 && blocked_ipc > 0)
        {
            serial_println!("[SCHED-DIAG] IPC wait dump:");
            let caps = crate::capability::CAP_BROKER.lock();
            for p in self
                .processes
                .iter()
                .filter(|p| p.state != ProcessState::Finished)
            {
                let (pending_call_cap, pending_call_label, resolved_ep, resolved_owner) =
                    match p.pending_call {
                        Some((cap, msg)) => {
                            let resolved = caps.debug_resolve_ipc(
                                crate::capability::CapabilityToken(cap),
                                crate::capability::CapabilityRights::SEND,
                            );
                            match resolved {
                                Some((ep, owner, _rights)) => (cap, msg.label, ep, owner),
                                None => (cap, msg.label, u32::MAX, usize::MAX),
                            }
                        }
                        None => (0, 0, u32::MAX, usize::MAX),
                    };
                let (receiver_waiting_ep, endpoint_queue_len, waiting_receiver, pending_callers) =
                    if resolved_ep != u32::MAX {
                        crate::ipc::with_shard(resolved_ep, |bus| {
                            (
                                resolved_ep,
                                bus.pending_count(resolved_ep),
                                bus.waiting_receiver_pid(resolved_ep, self)
                                    .unwrap_or(usize::MAX),
                                bus.pending_callers_count(resolved_ep),
                            )
                        })
                    } else {
                        (u32::MAX, 0, usize::MAX, 0)
                    };
                if pending_call_cap != 0
                    && waiting_receiver == resolved_owner
                    && endpoint_queue_len > 0
                    && pending_callers > 0
                    && self.global_tick.saturating_sub(p.block_start_tick) > 50
                {
                    serial_println!(
                        "[IPC-DIAG] stuck rendezvous caller={} server={} ep={} label={:#x}",
                        p.pid,
                        resolved_owner,
                        resolved_ep,
                        pending_call_label
                    );
                };
                let pending_reply_wait = match p.pending_reply_wait {
                    Some((ep, msg)) => (ep, msg.label),
                    None => (0, 0),
                };
                serial_println!(
                    "[SCHED-DIAG] pid={} name='{}' state={:?} ipc_ep={:?} pending_call_cap={:#x} pending_call_label={:#x} pending_call_resolved_ep={} pending_call_resolved_owner_pid={} receiver_waiting_ep={} endpoint_queue_len={} endpoint_waiting_receiver_pid={} pending_callers={} reply_wait_ep={} reply_wait_label={} blocked_ticks={}",
                    p.pid,
                    p.name_str(),
                    p.state,
                    p.ipc_endpoint,
                    pending_call_cap,
                    pending_call_label,
                    resolved_ep,
                    resolved_owner,
                    receiver_waiting_ep,
                    endpoint_queue_len,
                    waiting_receiver,
                    pending_callers,
                    pending_reply_wait.0,
                    pending_reply_wait.1,
                    self.global_tick.saturating_sub(p.block_start_tick)
                );
            }
            for (ep, owner) in caps.debug_endpoints() {
                crate::ipc::with_shard(ep, |bus| {
                    serial_println!(
                        "[IPC-DIAG] ep={} owner={} waiting_receiver={} pending_callers={}",
                        ep,
                        owner,
                        bus.waiting_receiver_pid(ep, self).unwrap_or(usize::MAX),
                        bus.pending_callers_count(ep)
                    );
                });
            }
        }
    }
}

// ─── Queue helpers ────────────────────────────────────────────────────────────

fn pop_ready_excluding_current(
    queue: &mut VecDeque<usize>,
    processes: &[Process],
    current: usize,
    skipped_current: &mut Option<usize>,
) -> Option<usize> {
    let mut remaining = queue.len();
    while remaining > 0 {
        let Some(idx) = queue.pop_front() else {
            break;
        };
        remaining -= 1;

        if idx >= processes.len() || !matches!(processes[idx].state, ProcessState::Ready) {
            continue;
        }

        if idx == current && skipped_current.is_none() {
            *skipped_current = Some(idx);
            continue;
        }

        return Some(idx);
    }
    None
}

fn idle_loop() -> ! {
    loop {
        x86_64::instructions::interrupts::enable();
        x86_64::instructions::hlt();
    }
}

// ─── Scheduler reschedule requests ────────────────────────────────────────────

#[inline]
fn reschedule_bit(cpu_id: usize) -> u64 {
    1u64 << cpu_id.min(63)
}

/// Check if a reschedule is needed for `cpu_id` and clear only that CPU's bit.
pub fn check_reschedule_on(cpu_id: usize) -> bool {
    let bit = reschedule_bit(cpu_id);
    RESCHEDULE_MASK.fetch_and(!bit, Ordering::SeqCst) & bit != 0
}

/// Set the reschedule flag for a specific CPU.
pub fn request_reschedule_on(cpu_id: usize) {
    RESCHEDULE_MASK.fetch_or(reschedule_bit(cpu_id), Ordering::SeqCst);
}

/// Set the reschedule flag for the current CPU.
pub fn request_reschedule() {
    request_reschedule_on(current_cpu_id());
}

pub fn note_process_finished(pid: usize, name: &str) {
    PROCESS_FINISHED.fetch_add(1, Ordering::Relaxed);
    serial_println!("[SCHED] FINISHED process pid={} name='{}'", pid, name);
}

/// Mark the current process finished and release resources.
/// Returns the process kernel stack top for the final idle loop.
pub fn finish_current_process(code: i32, reason: &str) -> u64 {
    let mut sched = SCHEDULER.lock();
    let cpu_id = current_cpu_id();
    let cur = match sched.cores[cpu_id].current_task {
        Some(idx) => idx,
        None => return 0,
    };
    if cur >= sched.processes.len() {
        return 0;
    }

    let kstack_top = sched.processes[cur].kernel_stack_top;
    if sched.processes[cur].state == crate::process::ProcessState::Finished {
        return kstack_top;
    }

    sched.account_current_runtime();
    {
        let process = &mut sched.processes[cur];
        process.exit_code = code;
        process.state = crate::process::ProcessState::Finished;
        process.exit_cleanup_pending = true;
    }
    sched.remove_from_ready_queues(cur);

    let my_pid = sched.processes[cur].pid;
    let parent_pid = sched.processes[cur].ppid;
    serial_println!(
        "[SCHED] terminating pid={} name='{}' reason={}",
        my_pid,
        sched.processes[cur].name_str(),
        reason
    );
    note_process_finished(my_pid, sched.processes[cur].name_str());

    let parent_waiting = sched
        .process_mut_by_pid(parent_pid)
        .is_some_and(|parent| parent.wait_child == Some(my_pid));
    if parent_waiting {
        sched.wake_pid(parent_pid);
    }

    kstack_top
}

// ─── Globals and wrappers ─────────────────────────────────────────────────────

/// Global scheduler instance.
pub static SCHEDULER: spin::Mutex<Scheduler> = spin::Mutex::new(Scheduler::new());

pub fn with_scheduler<F, R>(f: F) -> R
where
    F: FnOnce(&mut Scheduler) -> R,
{
    f(&mut SCHEDULER.lock())
}

/// Enter the first ready user process without holding the scheduler lock across
/// the privilege transition.
pub fn enter_first_process() -> ! {
    let (rsp, pml4_phys, fs_base, kernel_stack_top) = {
        let mut sched = SCHEDULER.lock();
        let mut first = None;
        for (i, p) in sched.processes.iter().enumerate() {
            serial_println!(
                "[SCHED] process {} '{}' state={:?}",
                i,
                p.name_str(),
                p.state
            );
            if matches!(p.state, ProcessState::Ready) {
                first = Some(i);
                break;
            }
        }

        if let Some(idx) = first {
            sched.cores[0].current_task = Some(idx);
            sched.processes[idx].state = ProcessState::Running;
            sched.processes[idx].last_run_tick = sched.global_tick;
            sched.processes[idx].owning_core = 0;
            sched.processes[idx].queued_on_core = u8::MAX;
            sched.start_charging_runtime(idx);
            sched.seed_ready_queues_except(idx);
            // Run queues are now seeded and core 0 has its first task. Release
            // the AP cores so their next LAPIC timer tick can steal/dispatch.
            // APs still block on SCHEDULER.lock() until this function drops it.
            mark_scheduler_ready();
            let rsp = sched.processes[idx].context_rsp;
            let pml4_phys = sched.processes[idx].address_space.pml4_phys;
            let fs_base = sched.processes[idx].fs_base;
            let kernel_stack_top = sched.processes[idx].kernel_stack_top;
            serial_println!(
                "[SCHED] Entering process {} '{}' at rsp={:#x}",
                idx,
                sched.processes[idx].name_str(),
                rsp
            );
            (rsp, pml4_phys, fs_base, kernel_stack_top)
        } else {
            serial_println!("[SCHED] No user processes, entering idle");
            drop(sched);
            idle_loop();
        }
    };

    unsafe {
        x86_64::registers::control::Cr3::write(
            x86_64::structures::paging::PhysFrame::from_start_address_unchecked(pml4_phys),
            x86_64::registers::control::Cr3Flags::empty(),
        );
        x86_64::registers::model_specific::Msr::new(0xC0000100).write(fs_base);
        crate::arch::x86_64::smp::set_current_cpu_tss_rsp0(kernel_stack_top);
        context::iretq_to_context(rsp);
    }
}

pub fn current_process_rsp() -> u64 {
    let sched = SCHEDULER.lock();
    let cpu_id = current_cpu_id();
    match sched.cores[cpu_id].current_task {
        Some(idx) => sched.processes[idx].context_rsp,
        None => 0,
    }
}

// ─── SMP helpers ──────────────────────────────────────────────────────────────

/// Return the current logical CPU index using the initial APIC ID from CPUID.
///
/// BSP has APIC ID 0 on all supported platforms. APs are parked until SMP
/// Phase 1 LAPIC timers are wired; until then this always returns 0 on the
/// BSP. When per-core timers fire, each AP's CPUID gives its own APIC ID,
/// mapping directly to an index into CORES[].
#[inline]
pub fn current_cpu_id() -> usize {
    // CPUID leaf 1, EBX[31:24] = initial APIC ID (valid for up to 256 logical CPUs).
    let apic_id = core::arch::x86_64::__cpuid(1).ebx as usize >> 24;
    apic_id.min(MAX_CORES - 1)
}

/// Initialise the per-core scheduler after SMP bring-up.
///
/// Called from main.rs after smp::start_aps() with the total number of
/// logical CPUs (BSP + APs). Sets ONLINE_CORES and the Scheduler's
/// online_cores field so that enqueue_process(), steal_work(), and
/// schedule_tick() are aware of all available cores.
pub fn init_cores(total_cpus: usize) {
    let count = total_cpus.min(MAX_CORES).max(1);
    ONLINE_CORES.store(count, Ordering::Release);
    {
        let mut sched = SCHEDULER.lock();
        sched.online_cores = count;
    }
    serial_println!(
        "[SCHED] Per-core work-stealing scheduler: {} online core(s)",
        count
    );
}

pub mod context;
