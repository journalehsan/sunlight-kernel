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
static NATIVE_BORROWERS_CREATED: AtomicUsize = AtomicUsize::new(0);
static NATIVE_BORROWERS_FINISHED: AtomicUsize = AtomicUsize::new(0);
static NATIVE_BORROWERS_REAPED: AtomicUsize = AtomicUsize::new(0);
static BORROWER_USER_RECLAIM_SKIPPED: AtomicUsize = AtomicUsize::new(0);
static STALE_TERMINAL_WAKEUPS_REJECTED: AtomicUsize = AtomicUsize::new(0);
static BORROWER_SLOTS_REUSED: AtomicUsize = AtomicUsize::new(0);
static LIVE_NATIVE_BORROWERS: AtomicUsize = AtomicUsize::new(0);
static MAX_LIVE_NATIVE_BORROWERS: AtomicUsize = AtomicUsize::new(0);

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

const RR_STEAL_BATCH_MAX: usize = 32;

/// Per-core scheduling state: local RoundRobin FIFO, BORE-tiered run queues,
/// and current task. The type is cache-line aligned so adjacent core locks do
/// not share a line and bounce between CPUs on every LAPIC tick.
#[repr(align(64))]
pub struct CoreState {
    /// RoundRobin ready FIFO. A task in this queue has
    /// `Process::queued_on_core == this_cpu`; the current task is never queued.
    pub rr_queue: VecDeque<usize>,
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
    /// Number of successful work-steal batches performed by this core.
    pub steal_count: u64,
    /// Number of stale queue entries dropped by this core.
    pub stale_pops: u64,
    /// Number of times this core found its RR FIFO empty.
    pub empty_pops: u64,
    /// Saved interrupt-frame RSP for this core's idle hlt loop.
    pub idle_context_rsp: u64,
}

impl CoreState {
    pub const fn new() -> Self {
        Self {
            rr_queue: VecDeque::new(),
            run_queue_high: VecDeque::new(),
            run_queue_medium: VecDeque::new(),
            run_queue_low: VecDeque::new(),
            current_task: None,
            current_ticks: 0,
            timer_ticks: 0,
            context_switches: 0,
            steal_count: 0,
            stale_pops: 0,
            empty_pops: 0,
            idle_context_rsp: 0,
        }
    }

    fn total_ready(&self) -> usize {
        self.rr_queue.len()
            + self.run_queue_high.len()
            + self.run_queue_medium.len()
            + self.run_queue_low.len()
    }
}

/// Per-core scheduler state. RoundRobin uses this array for its hot FIFO and
/// current-task metadata; Bore keeps using `Scheduler::cores` so the existing
/// tiered code paths continue compiling.
///
/// Lock ordering: normal enqueue/dequeue takes the process-table lock
/// (`SCHEDULER`) before a single core lock. Work stealing never blocks on a
/// victim: it uses `try_lock()`, drops the victim lock, then updates process
/// queue ownership and pushes leftovers to the thief. No path holds two core
/// locks at once.
pub static CORE_STATES: [spin::Mutex<CoreState>; MAX_CORES] =
    [const { spin::Mutex::new(CoreState::new()) }; MAX_CORES];

// ─── Scheduler ───────────────────────────────────────────────────────────────

pub struct Scheduler {
    pub processes: Vec<Process>,

    /// Per-core scheduling state. Only indices 0..online_cores are active.
    pub cores: [CoreState; MAX_CORES],
    /// Number of online cores (1 until init_cores() is called).
    pub online_cores: usize,

    /// Legacy scheduler-visible global tick counter.
    /// Kept intentionally for backward compatibility; the canonical source of
    /// truth now lives in `crate::timekeeping` and this field mirrors it.
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

    #[inline]
    fn core_current_task(&self, cpu_id: usize) -> Option<usize> {
        match SCHEDULER_MODE {
            SchedulerMode::RoundRobin => CORE_STATES[cpu_id].lock().current_task,
            SchedulerMode::Bore => self.cores[cpu_id].current_task,
        }
    }

    #[inline]
    fn set_core_current_task(&mut self, cpu_id: usize, task: Option<usize>) {
        match SCHEDULER_MODE {
            SchedulerMode::RoundRobin => {
                CORE_STATES[cpu_id].lock().current_task = task;
            }
            SchedulerMode::Bore => {
                self.cores[cpu_id].current_task = task;
            }
        }
    }

    #[inline]
    fn increment_context_switches(&mut self, cpu_id: usize) {
        match SCHEDULER_MODE {
            SchedulerMode::RoundRobin => {
                CORE_STATES[cpu_id].lock().context_switches += 1;
            }
            SchedulerMode::Bore => {
                self.cores[cpu_id].context_switches += 1;
            }
        }
    }

    #[inline]
    fn core_current_ticks(&self, cpu_id: usize) -> u64 {
        match SCHEDULER_MODE {
            SchedulerMode::RoundRobin => CORE_STATES[cpu_id].lock().current_ticks,
            SchedulerMode::Bore => self.cores[cpu_id].current_ticks,
        }
    }

    #[inline]
    fn set_core_current_ticks(&mut self, cpu_id: usize, ticks: u64) {
        match SCHEDULER_MODE {
            SchedulerMode::RoundRobin => {
                CORE_STATES[cpu_id].lock().current_ticks = ticks;
            }
            SchedulerMode::Bore => {
                self.cores[cpu_id].current_ticks = ticks;
            }
        }
    }

    #[inline]
    fn core_idle_context_rsp(&self, cpu_id: usize) -> u64 {
        match SCHEDULER_MODE {
            SchedulerMode::RoundRobin => CORE_STATES[cpu_id].lock().idle_context_rsp,
            SchedulerMode::Bore => self.cores[cpu_id].idle_context_rsp,
        }
    }

    #[inline]
    fn set_core_idle_context_rsp(&mut self, cpu_id: usize, rsp: u64) {
        match SCHEDULER_MODE {
            SchedulerMode::RoundRobin => {
                CORE_STATES[cpu_id].lock().idle_context_rsp = rsp;
            }
            SchedulerMode::Bore => {
                self.cores[cpu_id].idle_context_rsp = rsp;
            }
        }
    }

    fn choose_rr_core(&self, idx: usize) -> usize {
        let online = self.online_cores.min(MAX_CORES).max(1);
        let cpu_mask = if idx < self.processes.len() {
            self.processes[idx].cpu_mask
        } else {
            u64::MAX
        };

        let mut best_core = 0usize;
        let mut best_load = usize::MAX;
        for core_id in 0..online {
            if cpu_mask & (1u64 << core_id) == 0 {
                continue;
            }
            let core = CORE_STATES[core_id].lock();
            let load = core.rr_queue.len() + usize::from(core.current_task.is_some());
            if load < best_load {
                best_load = load;
                best_core = core_id;
            }
        }
        best_core
    }

    pub fn add_process(&mut self, process: Process) -> usize {
        self.reap_finished_processes();
        self.add_process_after_reaping(process)
    }

    pub fn add_process_after_reaping(&mut self, process: Process) -> usize {
        let created_count = PROCESS_CREATED.fetch_add(1, Ordering::Relaxed);
        let online = self.online_cores;

        // Collect which process slots are currently running on any core so we
        // don't reclaim them while a core still has a reference.
        let mut in_use = [usize::MAX; MAX_CORES];
        for c in 0..online {
            in_use[c] = self.core_current_task(c).unwrap_or(usize::MAX);
        }

        // Only reuse slots that are fully Reaped (or Finished that became reaped above).
        // Never overwrite a still-Finished slot until its resources are cleaned.
        if let Some(id) = self
            .processes
            .iter()
            .enumerate()
            .find(|(idx, p)| {
                (p.state == ProcessState::Reaped
                    || (p.state == ProcessState::Finished && !p.exit_cleanup_pending))
                    && in_use[..online].iter().all(|&cur| cur != *idx)
            })
            .map(|(idx, _)| idx)
        {
            let reused_borrower = self.processes[id].native_thread
                && self.processes[id].state == ProcessState::Reaped;
            self.remove_from_ready_queues(id);
            serial_println!(
                "[SCHED] process_slot_reused idx={} pid={} (reused reaped slot)",
                id,
                process.pid
            );
            // Drop old (reaped) contents by overwrite; new process owns fresh resources.
            self.processes[id] = process;
            if reused_borrower {
                BORROWER_SLOTS_REUSED.fetch_add(1, Ordering::Relaxed);
            }
            serial_println!(
                "[SCHED] CREATED process #{} '{}' idx={} burst_score={} tier={:?} (reused reaped slot)",
                created_count + 1,
                self.processes[id].name_str(),
                id,
                self.processes[id].burst_score,
                self.processes[id].get_queue_tier()
            );
            return id;
        }

        // Fallback: if there are stale Finished (not safe to reap yet), do not overwrite them.
        // Search only for Reaped again (after the reap attempt above).
        if let Some(id) = self
            .processes
            .iter()
            .enumerate()
            .find(|(idx, p)| {
                p.state == ProcessState::Reaped && in_use[..online].iter().all(|&cur| cur != *idx)
            })
            .map(|(idx, _)| idx)
        {
            let reused_borrower = self.processes[id].native_thread;
            self.remove_from_ready_queues(id);
            serial_println!("[SCHED] process_slot_reused idx={} (reaped)", id);
            self.processes[id] = process;
            if reused_borrower {
                BORROWER_SLOTS_REUSED.fetch_add(1, Ordering::Relaxed);
            }
            serial_println!(
                "[SCHED] CREATED process #{} '{}' idx={} burst_score={} tier={:?} (reused reaped)",
                created_count + 1,
                self.processes[id].name_str(),
                id,
                self.processes[id].burst_score,
                self.processes[id].get_queue_tier()
            );
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
        if SCHEDULER_MODE == SchedulerMode::RoundRobin {
            return self.choose_rr_core(idx);
        }

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
        if self.processes[idx].owning_core != u8::MAX {
            return;
        }
        if SCHEDULER_MODE == SchedulerMode::RoundRobin {
            self.processes[idx].queued_on_core = core_id as u8;
            CORE_STATES[core_id].lock().rr_queue.push_back(idx);
            request_reschedule_on(core_id);
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

    /// Public Ready-transition helper. Call this exactly once after changing a
    /// non-running task to `ProcessState::Ready`; it is idempotent via
    /// `queued_on_core` and refuses to queue a task still owned by a CPU.
    pub fn enqueue_ready(&mut self, idx: usize) {
        self.enqueue_process_once(idx);
    }

    pub(crate) fn diagnostic_ready_occurrences(&self, idx: usize) -> usize {
        if SCHEDULER_MODE == SchedulerMode::RoundRobin {
            return (0..self.online_cores.min(MAX_CORES))
                .map(|core_id| {
                    CORE_STATES[core_id]
                        .lock()
                        .rr_queue
                        .iter()
                        .filter(|&&queued| queued == idx)
                        .count()
                })
                .sum();
        }
        self.cores[..self.online_cores.min(MAX_CORES)]
            .iter()
            .map(|core| {
                core.run_queue_high
                    .iter()
                    .chain(core.run_queue_medium.iter())
                    .chain(core.run_queue_low.iter())
                    .filter(|&&queued| queued == idx)
                    .count()
            })
            .sum()
    }

    /// Enqueue a freshly created Ready task onto a preferred CPU for its first
    /// dispatch. Once it has run and been preempted, normal scheduler paths
    /// are free to rebalance it across cores.
    pub fn enqueue_ready_on_cpu(&mut self, idx: usize, preferred_cpu: usize) {
        if idx >= self.processes.len() || !matches!(self.processes[idx].state, ProcessState::Ready)
        {
            return;
        }
        if self.processes[idx].owning_core != u8::MAX {
            return;
        }

        self.remove_from_ready_queues(idx);

        let online = self.online_cores.min(MAX_CORES).max(1);
        let preferred_cpu = preferred_cpu.min(online - 1);
        let cpu_mask = self.processes[idx].cpu_mask;
        let target_cpu = if cpu_mask & (1u64 << preferred_cpu) != 0 {
            preferred_cpu
        } else {
            self.target_core_for_process(idx)
        };

        self.enqueue_process_to_core(idx, target_cpu);
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
            if SCHEDULER_MODE == SchedulerMode::RoundRobin {
                CORE_STATES[core_id].lock().rr_queue.retain(|&q| q != idx);
            } else {
                let core = &mut self.cores[core_id];
                core.run_queue_high.retain(|&q| q != idx);
                core.run_queue_medium.retain(|&q| q != idx);
                core.run_queue_low.retain(|&q| q != idx);
            }
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

    fn context_is_dispatchable(&self, idx: usize) -> bool {
        if idx >= self.processes.len() {
            return false;
        }

        let rsp = self.processes[idx].context_rsp;
        let stack_top = self.processes[idx].kernel_stack_top;
        let stack_bottom = stack_top.saturating_sub(crate::process::KERNEL_STACK_SIZE as u64);
        if rsp < stack_bottom || rsp.saturating_add(160) > stack_top {
            return false;
        }

        unsafe { ((rsp + 120) as *const u64).read_volatile() != 0 }
    }

    /// Clear all core queues, then distribute all Ready processes (except
    /// `running_idx`) across the online cores.
    pub fn seed_ready_queues_except(&mut self, running_idx: usize) {
        let online = self.online_cores;
        for core_id in 0..online {
            if SCHEDULER_MODE == SchedulerMode::RoundRobin {
                CORE_STATES[core_id].lock().rr_queue.clear();
            } else {
                let core = &mut self.cores[core_id];
                core.run_queue_high.clear();
                core.run_queue_medium.clear();
                core.run_queue_low.clear();
            }
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
        let current = match self.core_current_task(cpu_id) {
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
            if let Some(current) = self.core_current_task(cpu_id) {
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
    pub fn tick(&mut self, cpu_id: usize, global_tick: u64) {
        // Compatibility mirror: all legacy scheduler logic still reads
        // `self.global_tick`, but only the centralized timekeeper advances it.
        self.global_tick = self.global_tick.max(global_tick);
        if cpu_id == crate::timekeeping::TIMEKEEPER_CORE_ID {
            crate::ipc::expire_deadlines(self, self.global_tick);
        }
        if SCHEDULER_MODE == SchedulerMode::RoundRobin {
            let mut core = CORE_STATES[cpu_id].lock();
            core.current_ticks += 1;
            core.timer_ticks += 1;
        } else {
            self.cores[cpu_id].current_ticks += 1;
            self.cores[cpu_id].timer_ticks += 1;
        }

        let current = match self.core_current_task(cpu_id) {
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
            self.set_core_current_ticks(cpu_id, 0);
            request_reschedule_on(cpu_id);
            return;
        }

        // Fatal pending signals (SIGKILL always; Default SIGTERM/SIGINT/…)
        // must be noticed even when the task never enters a syscall. Timer
        // preemption is a safe kernel/user return point for that check.
        if self.apply_pending_fatal_signal(current) {
            self.set_core_current_ticks(cpu_id, 0);
            request_reschedule_on(cpu_id);
            return;
        }

        let quantum = quantum_ticks(&self.processes[current]) as u64;

        if self.core_current_ticks(cpu_id) >= quantum {
            let ticks_used = self.core_current_ticks(cpu_id);
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
            self.set_core_current_ticks(cpu_id, 0);
            request_reschedule_on(cpu_id);
        }

        // Periodic opportunistic reaping of Finished (cheap no-op if none).
        // 50 ticks ~ 0.5s at 100Hz; keeps Finished from lingering without depending on spawn/switch.
        if self.global_tick % 50 == 0 {
            self.reap_finished_processes();
        }

        // The diagnostic dump writes per-core + per-process state over the slow
        // UART while the global scheduler lock is held — every other core spins
        // on the lock for the duration. Compile it out unless explicitly enabled.
        if cfg!(feature = "verbose_diag") && self.global_tick.is_multiple_of(100) {
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
            let ticks_since_run = self.global_tick.saturating_sub(p.last_run_tick);
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
        if SCHEDULER_MODE == SchedulerMode::RoundRobin {
            return self.steal_work_round_robin(thief_id);
        }

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

    /// RoundRobin work stealing: take up to half of a victim FIFO, bounded by a
    /// small fixed stack batch, using try_lock so a timer interrupt never blocks
    /// behind another core's queue lock. One valid task is returned immediately;
    /// the rest are moved to the thief's FIFO to amortize future steals.
    pub fn steal_work_round_robin(&mut self, thief_id: usize) -> Option<usize> {
        let online = self.online_cores.min(MAX_CORES).max(1);
        let mut batch = [usize::MAX; RR_STEAL_BATCH_MAX];

        for victim_id in 0..online {
            if victim_id == thief_id {
                continue;
            }

            let mut count = 0usize;
            if let Some(mut victim) = CORE_STATES[victim_id].try_lock() {
                let available = victim.rr_queue.len();
                if available <= 1 {
                    continue;
                }
                let to_take = (available / 2).min(RR_STEAL_BATCH_MAX);
                while count < to_take {
                    let Some(idx) = victim.rr_queue.pop_back() else {
                        break;
                    };
                    batch[count] = idx;
                    count += 1;
                }
            } else {
                continue;
            }

            if count == 0 {
                continue;
            }

            let mut picked = None;
            let mut moved = 0usize;
            for &idx in batch[..count].iter() {
                if idx >= self.processes.len() {
                    continue;
                }

                if self.processes[idx].queued_on_core == victim_id as u8 {
                    self.processes[idx].queued_on_core = u8::MAX;
                }

                let eligible = matches!(self.processes[idx].state, ProcessState::Ready)
                    && !self.is_live_on_other_core(idx, thief_id)
                    && self.processes[idx].cpu_mask & (1u64 << thief_id) != 0
                    && self.context_is_dispatchable(idx);

                if !eligible {
                    CORE_STATES[thief_id].lock().stale_pops += 1;
                    if matches!(self.processes[idx].state, ProcessState::Ready)
                        && !self.is_live_on_other_core(idx, thief_id)
                    {
                        self.processes[idx].queued_on_core = victim_id as u8;
                        CORE_STATES[victim_id].lock().rr_queue.push_back(idx);
                    }
                    continue;
                }

                if picked.is_none() {
                    picked = Some(idx);
                } else {
                    self.processes[idx].queued_on_core = thief_id as u8;
                    CORE_STATES[thief_id].lock().rr_queue.push_back(idx);
                    moved += 1;
                }
            }

            if picked.is_some() || moved > 0 {
                CORE_STATES[thief_id].lock().steal_count += 1;
            }
            if picked.is_some() {
                return picked;
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
        let mut remaining = CORE_STATES[cpu_id].lock().rr_queue.len();
        while remaining > 0 {
            let idx = {
                let mut core = CORE_STATES[cpu_id].lock();
                match core.rr_queue.pop_front() {
                    Some(idx) => idx,
                    None => {
                        core.empty_pops += 1;
                        return self.steal_work(cpu_id);
                    }
                }
            };
            remaining -= 1;

            if idx >= self.processes.len() {
                CORE_STATES[cpu_id].lock().stale_pops += 1;
                continue;
            }

            if self.processes[idx].queued_on_core == cpu_id as u8 {
                self.processes[idx].queued_on_core = u8::MAX;
            }

            if !matches!(self.processes[idx].state, ProcessState::Ready)
                || self.is_live_on_other_core(idx, cpu_id)
                || self.processes[idx].cpu_mask & (1u64 << cpu_id) == 0
                || !self.context_is_dispatchable(idx)
            {
                CORE_STATES[cpu_id].lock().stale_pops += 1;
                if matches!(self.processes[idx].state, ProcessState::Ready)
                    && !self.is_live_on_other_core(idx, cpu_id)
                {
                    self.enqueue_process_once(idx);
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
                self.processes[idx].queued_on_core = cpu_id as u8;
                CORE_STATES[cpu_id].lock().rr_queue.push_back(idx);
                continue;
            }

            self.processes[idx].aging_boosted_this_pick = false;
            return Some(idx);
        }

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
        if self.core_current_task(cpu_id).is_none() {
            self.set_core_idle_context_rsp(cpu_id, saved_rsp);
            if let Some(next) = self.pick_next(cpu_id) {
                let next_rsp = self.processes[next].context_rsp;
                let next_stack_top = self.processes[next].kernel_stack_top;
                let next_fs_base = self.processes[next].fs_base;
                self.set_core_current_task(cpu_id, Some(next));
                self.increment_context_switches(cpu_id);
                self.processes[next].state = ProcessState::Running;
                self.processes[next].last_run_tick = self.global_tick;
                self.processes[next].owning_core = cpu_id as u8;
                self.processes[next].queued_on_core = u8::MAX;
                self.start_charging_runtime(next);
                unsafe {
                    self.processes[next].address_space.activate();
                    x86_64::registers::model_specific::Msr::new(0xC0000100).write(next_fs_base);
                }
                crate::arch::x86_64::smp::set_current_cpu_tss_rsp0(next_stack_top);
                return next_rsp;
            }
            unsafe {
                crate::memory::tlb::activate_kernel_root();
            }
            return 0;
        }

        let current = self.core_current_task(cpu_id).unwrap();

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
            self.processes[current].owning_core = u8::MAX;
            self.set_core_current_task(cpu_id, None);
            self.enqueue_process_to_core(current, cpu_id);
        } else if !was_runnable || !matches!(self.processes[current].state, ProcessState::Ready) {
            self.processes[current].owning_core = u8::MAX;
            self.set_core_current_task(cpu_id, None);
            self.remove_from_ready_queues(current);
        }

        if let Some(next) = self.pick_next(cpu_id) {
            let next_rsp = self.processes[next].context_rsp;
            let next_stack_top = self.processes[next].kernel_stack_top;
            let next_fs_base = self.processes[next].fs_base;
            let prev = current;

            if next != prev {
                self.increment_context_switches(cpu_id);
                self.processes[prev].owning_core = u8::MAX;
            }
            self.set_core_current_task(cpu_id, Some(next));
            self.processes[next].state = ProcessState::Running;
            self.processes[next].last_run_tick = self.global_tick;
            self.processes[next].owning_core = cpu_id as u8;
            self.processes[next].queued_on_core = u8::MAX;
            self.start_charging_runtime(next);

            unsafe {
                self.processes[next].address_space.activate();
                x86_64::registers::model_specific::Msr::new(0xC0000100).write(next_fs_base);
            }
            // Update this core's TSS RSP0 so the next interrupt from ring-3
            // delivers to the correct kernel stack.  Uses the per-core TSS
            // (BSP → global TSS; APs → AP_TSS_STORE[cpu_id-1]).
            crate::arch::x86_64::smp::set_current_cpu_tss_rsp0(next_stack_top);

            if matches!(
                self.processes[prev].state,
                ProcessState::Finished | ProcessState::Reaped
            ) {
                if self.processes[prev].state == ProcessState::Finished {
                    self.reap_process_resources(prev);
                }
            }

            // Opportunistic global reaping of any other safe Finished processes.
            self.reap_finished_processes();

            next_rsp
        } else {
            if current < self.processes.len()
                && matches!(self.processes[current].state, ProcessState::Ready)
            {
                self.remove_from_ready_queues(current);
                self.set_core_current_task(cpu_id, Some(current));
                self.processes[current].state = ProcessState::Running;
                self.processes[current].owning_core = cpu_id as u8;
                self.processes[current].queued_on_core = u8::MAX;
                self.start_charging_runtime(current);
                return 0;
            }

            let idle_rsp = self.core_idle_context_rsp(cpu_id);
            if idle_rsp != 0 {
                unsafe {
                    crate::memory::tlb::activate_kernel_root();
                }
                return idle_rsp;
            }
            let stack_top = unsafe {
                core::ptr::addr_of!(CORE_IDLE_STACKS[cpu_id][crate::process::KERNEL_STACK_SIZE - 1])
                    as u64
                    + 1
            };
            let rsp = build_kernel_idle_frame(stack_top);
            self.set_core_idle_context_rsp(cpu_id, rsp);
            unsafe {
                crate::memory::tlb::activate_kernel_root();
            }
            serial_println!(
                "[SCHED] WARN: cpu={} lazily initialized idle context",
                cpu_id
            );
            rsp
        }
    }

    // ── Current process accessors ────────────────────────────────────────────

    pub fn current_process(&self) -> &Process {
        let cpu_id = current_cpu_id();
        &self.processes[self.core_current_task(cpu_id).unwrap_or(0)]
    }

    pub fn current_process_index(&self) -> Option<usize> {
        self.core_current_task(current_cpu_id())
    }

    pub fn current_process_mut(&mut self) -> &mut Process {
        let cpu_id = current_cpu_id();
        let idx = self.core_current_task(cpu_id).unwrap_or(0);
        &mut self.processes[idx]
    }

    // ── Wake / unblock ───────────────────────────────────────────────────────

    /// Return the PID of the process currently running on this CPU (0 if none).
    pub fn current_pid(&self) -> usize {
        self.process_index_for_cpu(current_cpu_id())
            .and_then(|idx| self.processes.get(idx))
            .map(|p| p.pid)
            .unwrap_or(0)
    }

    /// Resolve the live process on `cpu_id` from per-core bookkeeping, falling
    /// back to the active page table when a user context is running without a
    /// `current_task` entry (SMP bookkeeping bug recovery).
    pub(crate) fn process_index_for_cpu(&self, cpu_id: usize) -> Option<usize> {
        if let Some(idx) = self.core_current_task(cpu_id) {
            return Some(idx);
        }
        let cr3 = x86_64::registers::control::Cr3::read()
            .0
            .start_address()
            .as_u64();
        self.processes.iter().position(|p| {
            !matches!(p.state, ProcessState::Finished | ProcessState::Reaped)
                && p.address_space.pml4_phys.as_u64() == cr3
        })
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
            let ticks_blocked = self
                .global_tick
                .saturating_sub(self.processes[idx].block_start_tick);
            if ticks_blocked < INTERACTIVE_DETECTION_THRESHOLD as u64 {
                update_burst_score(&mut self.processes[idx], BurstReason::EarlyBlock);
            }
            self.processes[idx].state = ProcessState::Ready;
            self.remove_from_ready_queues(idx);
            if let Some(cpu_id) = self.live_owner_core(idx) {
                request_reschedule_on(cpu_id);
            } else {
                self.enqueue_ready(idx);
            }
        } else if matches!(
            self.processes[idx].state,
            ProcessState::Finished | ProcessState::Reaped
        ) {
            STALE_TERMINAL_WAKEUPS_REJECTED.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn wake_timer_pid(&mut self, pid: usize) {
        let idx = match self.processes.iter().position(|p| p.pid == pid) {
            Some(i) => i,
            None => return,
        };
        if self.processes[idx].state == ProcessState::BlockedOnTimer {
            let ticks_blocked = self
                .global_tick
                .saturating_sub(self.processes[idx].block_start_tick);
            if ticks_blocked < INTERACTIVE_DETECTION_THRESHOLD as u64 {
                update_burst_score(&mut self.processes[idx], BurstReason::EarlyBlock);
            }
            self.processes[idx].state = ProcessState::Ready;
            self.remove_from_ready_queues(idx);
            if let Some(cpu_id) = self.live_owner_core(idx) {
                request_reschedule_on(cpu_id);
            } else {
                self.enqueue_ready(idx);
            }
        }
    }

    pub fn wake_io_pid(&mut self, pid: usize) {
        let idx = match self.processes.iter().position(|p| p.pid == pid) {
            Some(i) => i,
            None => return,
        };
        if self.processes[idx].state == ProcessState::BlockedOnIo {
            let ticks_blocked = self
                .global_tick
                .saturating_sub(self.processes[idx].block_start_tick);
            if ticks_blocked < INTERACTIVE_DETECTION_THRESHOLD as u64 {
                update_burst_score(&mut self.processes[idx], BurstReason::EarlyBlock);
            }
            self.processes[idx].state = ProcessState::Ready;
            self.remove_from_ready_queues(idx);
            if let Some(cpu_id) = self.live_owner_core(idx) {
                request_reschedule_on(cpu_id);
            } else {
                self.enqueue_ready(idx);
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
                    self.enqueue_ready(idx);
                }
            }
            _ => {}
        }
    }

    // ── Process lifecycle ────────────────────────────────────────────────────

    fn live_address_space_borrowers(&self, idx: usize) -> usize {
        let identity = self.processes[idx].address_space.identity();
        self.processes
            .iter()
            .enumerate()
            .filter(|(other_idx, proc)| {
                *other_idx != idx
                    && !proc.owns_address_space
                    && proc.state != ProcessState::Reaped
                    && proc.address_space.identity() == identity
            })
            .count()
    }

    pub fn current_address_space_has_borrowers(&self) -> bool {
        self.current_process_index()
            .is_some_and(|idx| self.live_address_space_borrowers(idx) != 0)
    }

    fn terminate_address_space_borrowers(&mut self, owner_idx: usize, reason: &str) {
        let identity = self.processes[owner_idx].address_space.identity();
        for idx in 0..self.processes.len() {
            if idx == owner_idx
                || self.processes[idx].owns_address_space
                || self.processes[idx].address_space.identity() != identity
                || matches!(
                    self.processes[idx].state,
                    ProcessState::Finished | ProcessState::Reaped
                )
            {
                continue;
            }
            self.processes[idx].exit_code = 128 + 9;
            self.processes[idx].state = ProcessState::Finished;
            self.processes[idx].exit_cleanup_pending = true;
            if self.processes[idx].native_thread {
                note_native_borrower_finished();
            }
            self.remove_from_ready_queues(idx);
            if self.processes[idx].owning_core != u8::MAX {
                request_reschedule_on(self.processes[idx].owning_core as usize);
            }
            serial_println!(
                "[SCHED] shared-address-space borrower pid={} terminated reason={}",
                self.processes[idx].pid,
                reason
            );
        }
    }

    fn close_owned_ipc_endpoints(&mut self, pid: usize) {
        let endpoint_ids = {
            let mut caps = crate::capability::CAP_BROKER.lock();
            let endpoint_ids = caps.endpoints_owned_by(pid);
            caps.revoke_endpoints_owned_by(pid);
            endpoint_ids
        };
        for endpoint_id in endpoint_ids {
            crate::arch::x86_64::keyboard::unregister_kbd_endpoint(endpoint_id);
            crate::arch::x86_64::mouse::unregister_mouse_endpoint(endpoint_id);
            let calls = crate::ipc::with_shard(endpoint_id, |bus| bus.remove_endpoint(endpoint_id));
            crate::ipc::finish_peer_closed_calls(endpoint_id, calls, self);
        }
    }

    pub fn reap_process_resources(&mut self, idx: usize) {
        if idx >= self.processes.len() {
            return;
        }
        if !self.processes[idx].exit_cleanup_pending {
            // Already reaped or never marked for cleanup. Ensure state reflects it.
            if self.processes[idx].state == ProcessState::Finished {
                // If no pending work was recorded, treat as reaped to allow reuse.
                self.processes[idx].state = ProcessState::Reaped;
            }
            return;
        }

        serial_println!(
            "[SCHED] process_reap_attempt idx={} pid={} name='{}'",
            idx,
            self.processes[idx].pid,
            self.processes[idx].name_str()
        );

        // Safety: never reap while still current on a core or queued.
        if self.processes[idx].owning_core != u8::MAX {
            serial_println!(
                "[SCHED] process_reap_blocked_reason idx={} reason=owning_core cpu={}",
                idx,
                self.processes[idx].owning_core
            );
            return;
        }
        if self.processes[idx].queued_on_core != u8::MAX {
            serial_println!(
                "[SCHED] process_reap_blocked_reason idx={} reason=queued_on_core",
                idx
            );
            return;
        }

        let pid = self.processes[idx].pid;
        let name: alloc::string::String = self.processes[idx].name_str().into();
        let is_native_borrower = self.processes[idx].native_thread;
        if self.processes[idx].owns_address_space {
            let borrowers = self.live_address_space_borrowers(idx);
            if borrowers != 0 {
                self.terminate_address_space_borrowers(idx, "address-space-owner-exit");
                serial_println!(
                    "[SCHED] process_reap_blocked_reason idx={} reason=live_address_space_borrowers count={}",
                    idx,
                    borrowers
                );
                return;
            }
        }
        let hhdm_offset = match crate::HHDM_REQ.response() {
            Some(resp) => x86_64::VirtAddr::new(resp.offset),
            None => {
                serial_println!(
                    "[SCHED] process_reap_blocked_reason idx={} reason=no_hhdm",
                    idx
                );
                return;
            }
        };

        // Additional safety: do not reap the live current on this or other cores (double-check).
        for c in 0..self.online_cores {
            if self.core_current_task(c) == Some(idx) {
                serial_println!(
                    "[SCHED] process_reap_blocked_reason idx={} reason=current_on_cpu={}",
                    idx,
                    c
                );
                return;
            }
        }

        if self.processes[idx].owns_address_space {
            let active_mask =
                crate::memory::tlb::active_cpu_mask(self.processes[idx].address_space.identity());
            if active_mask != 0 {
                serial_println!(
                    "[SCHED] process_reap_blocked_reason idx={} reason=active_address_space mask={:#x}",
                    idx,
                    active_mask
                );
                return;
            }
        }

        crate::memory::swap::untrack_process(pid);
        let swap_admin_identity = self.processes[idx].address_space.identity();
        crate::memory::zram::revoke_admin(pid, swap_admin_identity.generation);

        let endpoint_ids = {
            let caps = crate::capability::CAP_BROKER.lock();
            caps.endpoints_owned_by(pid)
        };

        let ipc_pending_before = {
            let mut n = 0usize;
            crate::ipc::for_all_shards(|bus| {
                n += bus.pending_count_for_pid(pid);
            });
            n
        };

        {
            crate::ipc::for_all_shards(|bus| {
                bus.remove_pid_references(pid);
            });
            for endpoint_id in &endpoint_ids {
                crate::arch::x86_64::keyboard::unregister_kbd_endpoint(*endpoint_id);
                crate::arch::x86_64::mouse::unregister_mouse_endpoint(*endpoint_id);
                let calls =
                    crate::ipc::with_shard(*endpoint_id, |bus| bus.remove_endpoint(*endpoint_id));
                crate::ipc::finish_peer_closed_calls(*endpoint_id, calls, self);
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

        let reclaim = if self.processes[idx].owns_address_space {
            let mut pmm = crate::PMM.lock();
            unsafe {
                self.processes[idx]
                    .address_space
                    .reclaim_user_space(&mut *pmm, hhdm_offset, true)
            }
        } else {
            if is_native_borrower {
                BORROWER_USER_RECLAIM_SKIPPED.fetch_add(1, Ordering::Relaxed);
            }
            crate::process::address_space::ReclaimStats::default()
        };

        // Clear per-process kernel structures (idempotent clears).
        let ipc_q_len = self.processes[idx].ipc_queue.len();
        self.processes[idx].ipc_queue.clear();
        self.processes[idx].ipc_reply = None;
        self.processes[idx].ipc_endpoint = None;
        self.processes[idx].pending_call = None;
        self.processes[idx].ipc_call_outcome = None;
        self.processes[idx].ipc_next_deadline_tick = None;
        self.processes[idx].ipc_deadline = None;
        self.processes[idx].ipc_recv_deadline = None;
        self.processes[idx].ipc_recv_timeout = None;
        self.processes[idx].pending_reply_wait = None;
        self.processes[idx].ipc_reply_target = None;
        self.processes[idx].deferred_reply_targets.clear();
        self.processes[idx].next_deferred_reply_token = 0;
        let dead_pid = self.processes[idx].pid;
        for process in &mut self.processes {
            process
                .deferred_reply_targets
                .retain(|entry| entry.target.call.pid != dead_pid);
        }
        self.processes[idx].capabilities.clear();
        // The dying context has already pivoted to its per-core static idle
        // stack. Drop the task-local kernel stack before exposing Reaped.
        self.processes[idx].kernel_stack = None;
        self.processes[idx].kernel_stack_top = 0;
        self.processes[idx].context_rsp = 0;
        self.processes[idx].fs_base = 0;
        self.processes[idx].exit_cleanup_pending = false;

        // Mark as fully reaped: slot is now safe for add_process reuse.
        self.processes[idx].state = ProcessState::Reaped;
        if is_native_borrower {
            NATIVE_BORROWERS_REAPED.fetch_add(1, Ordering::Relaxed);
        }

        serial_println!(
            "[SCHED] process_reaped pid={} name='{}' user_frames={} page_tables={} swap_blocks={} ipc_cleared={}",
            pid,
            name,
            reclaim.user_frames,
            reclaim.page_tables,
            reclaim.swap_blocks,
            ipc_q_len
        );
        serial_println!(
            "[SCHED] process_resource_cleanup_summary pid={} frames={} pts={} swap={} ipc_entries_cleared={}",
            pid, reclaim.user_frames, reclaim.page_tables, reclaim.swap_blocks, ipc_pending_before
        );
        serial_println!(
            "[SCHED] ipc_entries_reaped_for_process pid={} cleared_msgs={}",
            pid,
            ipc_pending_before
        );
        crate::PMM.lock().diagnostic_report_pid(pid as u32);
        if is_native_borrower {
            let (created, finished, reaped, skipped, stale_wakes, reused, max_live) =
                native_borrower_lifecycle_counts();
            // One aggregate marker when the current borrower population has
            // drained is enough to prove the lifecycle without logging an
            // additional diagnostic for every reaped worker.
            if reaped == created {
                serial_println!(
                    "[THREAD-LIFECYCLE] created={} finished={} reaped={} borrower_reclaim_skipped={} stale_wake_rejected={} borrower_slots_reused={} max_live={}",
                    created,
                    finished,
                    reaped,
                    skipped,
                    stale_wakes,
                    reused,
                    max_live
                );
            }
        }
    }

    /// Centralized reaper. Walks Finished processes and reaps those that are
    /// safe (not current on any core, not queued, not mid-transition).
    /// A reaped slot transitions to ProcessState::Reaped and may be reused by
    /// add_process. Never reaps the current task of any CPU.
    pub fn reap_finished_processes(&mut self) {
        let online = self.online_cores;
        let mut in_use = [usize::MAX; MAX_CORES];
        for c in 0..online.min(MAX_CORES) {
            in_use[c] = self.core_current_task(c).unwrap_or(usize::MAX);
        }

        for idx in 0..self.processes.len() {
            if self.processes[idx].state != ProcessState::Finished {
                continue;
            }
            if !self.processes[idx].exit_cleanup_pending {
                // Stale Finished without pending flag: promote to Reaped for reuse safety.
                self.processes[idx].state = ProcessState::Reaped;
                continue;
            }
            // Safety checks per requirements
            if self.processes[idx].owning_core != u8::MAX {
                serial_println!(
                    "[SCHED] process_reap_blocked_reason pid={} idx={} reason=owning_core",
                    self.processes[idx].pid,
                    idx
                );
                continue;
            }
            if self.processes[idx].queued_on_core != u8::MAX {
                serial_println!(
                    "[SCHED] process_reap_blocked_reason pid={} idx={} reason=queued",
                    self.processes[idx].pid,
                    idx
                );
                continue;
            }
            if in_use[..online].iter().any(|&cur| cur == idx) {
                serial_println!(
                    "[SCHED] process_reap_blocked_reason pid={} idx={} reason=in_use_on_core",
                    self.processes[idx].pid,
                    idx
                );
                continue;
            }
            // Do not reap if this idx is still the current on its last known owner (paranoia).
            if let Some(c) = self.live_owner_core(idx) {
                if c < online {
                    serial_println!(
                        "[SCHED] process_reap_blocked_reason pid={} idx={} reason=live_owner",
                        self.processes[idx].pid,
                        idx
                    );
                    continue;
                }
            }

            serial_println!(
                "[SCHED] process_reap_attempt pid={} idx={} name='{}'",
                self.processes[idx].pid,
                idx,
                self.processes[idx].name_str()
            );
            // Perform the resource cleanup; it will set Reaped on success.
            self.reap_process_resources(idx);
        }
    }

    /// Force-terminate a process by pid (SIGKILL / external kill path).
    ///
    /// Idempotent: returns `false` only when the pid is unknown or already
    /// Finished/Reaped. A task that is currently executing on a core is marked
    /// Finished and that core is asked to reschedule; reaping waits until the
    /// owner core has dropped the task (same pattern as address-space borrower
    /// teardown). Returns `true` once termination has been accepted.
    pub fn terminate_process_by_pid(&mut self, pid: usize, code: i32, reason: &str) -> bool {
        serial_println!(
            "[SCHED] process_exit_begin external pid={} reason={}",
            pid,
            reason
        );
        let Some(idx) = self.process_index_by_pid(pid) else {
            return false;
        };
        if matches!(
            self.processes[idx].state,
            ProcessState::Finished | ProcessState::Reaped
        ) {
            return false;
        }

        serial_println!(
            "[SCHED] process_mark_finished pid={} name='{}' reason={} code={} (external)",
            pid,
            self.processes[idx].name_str(),
            reason,
            code
        );

        self.processes[idx].exit_code = code;
        self.processes[idx].state = ProcessState::Finished;
        self.processes[idx].exit_cleanup_pending = true;
        if self.processes[idx].native_thread {
            note_native_borrower_finished();
        }
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

        // If still live on a core, leave owning_core set so the owner core's
        // schedule_tick deschedules it safely; request a preemption.
        let owning = self.processes[idx].owning_core;
        if owning != u8::MAX {
            request_reschedule_on(owning as usize);
            return true;
        }

        self.reap_process_resources(idx);
        true
    }

    /// Apply a fatal pending signal on a live process slot without requiring a
    /// syscall return. Used from the timer tick so CPU-bound userspace loops
    /// cannot dodge Default-disposition termination forever.
    ///
    /// Returns true if the process was marked Finished.
    pub fn apply_pending_fatal_signal(&mut self, idx: usize) -> bool {
        if idx >= self.processes.len() {
            return false;
        }
        if matches!(
            self.processes[idx].state,
            ProcessState::Finished | ProcessState::Reaped
        ) {
            return false;
        }
        let Some(code) = self.processes[idx].signal_state.take_fatal_exit_code() else {
            return false;
        };
        let pid = self.processes[idx].pid;
        serial_println!(
            "[SCHED] process_mark_finished pid={} name='{}' reason=pending-fatal-signal code={}",
            pid,
            self.processes[idx].name_str(),
            code
        );
        self.processes[idx].exit_code = code;
        self.processes[idx].state = ProcessState::Finished;
        self.processes[idx].exit_cleanup_pending = true;
        if self.processes[idx].native_thread {
            note_native_borrower_finished();
        }
        self.remove_from_ready_queues(idx);
        note_process_finished(pid, self.processes[idx].name_str());

        let parent_pid = self.processes[idx].ppid;
        let parent_waiting = self
            .process_mut_by_pid(parent_pid)
            .is_some_and(|parent| parent.wait_child == Some(pid));
        if parent_waiting {
            self.wake_pid(parent_pid);
        }

        let owning = self.processes[idx].owning_core;
        if owning != u8::MAX {
            request_reschedule_on(owning as usize);
        } else {
            self.reap_process_resources(idx);
        }
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
            self.set_core_current_task(0, Some(idx));
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
        let ipc = crate::ipc::diagnostic_snapshot();
        let caps = crate::capability::diagnostic_snapshot();
        crate::memory::security::diagnostic_report();
        serial_println!(
            "[IPC-DIAG] depth={} high_water={} enqueue={} dequeue={} full={} coalesced={} deadline={} cancel={} late_reply={} peer_closed={} public_send={} rights_escalation={} unauthorized_register={} live_conflict={} stale_remove={} dead_replace={} stale_lookup={} registry_full={} send_only_reject={}",
            ipc.current_queue_depth,
            ipc.high_watermark,
            ipc.enqueue_count,
            ipc.dequeue_count,
            ipc.queue_full_count,
            ipc.coalesced_notification_count,
            ipc.deadline_expired_count,
            ipc.explicit_cancel_count,
            ipc.late_reply_drop_count,
            ipc.peer_closed_wake_count,
            caps.public_send_derivations,
            caps.rejected_rights_escalations,
            ipc.unauthorized_register_count,
            ipc.conflicting_live_registration_count,
            ipc.stale_registration_removal_count,
            ipc.successful_dead_replacement_count,
            ipc.stale_lookup_count,
            ipc.registry_full_rejection_count,
            ipc.send_only_management_reject_count
        );
        let created = PROCESS_CREATED.load(Ordering::Relaxed);
        let finished = PROCESS_FINISHED.load(Ordering::Relaxed);
        let alive = self
            .processes
            .iter()
            .filter(|p| !matches!(p.state, ProcessState::Finished | ProcessState::Reaped))
            .count();
        let finished_slots = self
            .processes
            .iter()
            .filter(|p| p.state == ProcessState::Finished)
            .count();
        let _reaped_slots = self
            .processes
            .iter()
            .filter(|p| p.state == ProcessState::Reaped)
            .count();

        let (ready_high, ready_mid, ready_low) = if SCHEDULER_MODE == SchedulerMode::RoundRobin {
            let rr_ready: usize = (0..self.online_cores)
                .map(|c| CORE_STATES[c].lock().rr_queue.len())
                .sum();
            (0, rr_ready, 0)
        } else {
            (
                (0..self.online_cores)
                    .map(|c| self.cores[c].run_queue_high.len())
                    .sum(),
                (0..self.online_cores)
                    .map(|c| self.cores[c].run_queue_medium.len())
                    .sum(),
                (0..self.online_cores)
                    .map(|c| self.cores[c].run_queue_low.len())
                    .sum(),
            )
        };

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

        // Minimal post-fix ownership invariant validation (diagnostics only).
        // Reports on the five listed bad states.
        for (_idx, p) in self.processes.iter().enumerate() {
            if matches!(p.state, ProcessState::Finished | ProcessState::Reaped) {
                continue;
            }
            let oc = p.owning_core;
            let qc = p.queued_on_core;
            let is_running = p.state == ProcessState::Running;
            let is_ready = p.state == ProcessState::Ready;
            if is_running && qc != u8::MAX {
                serial_println!(
                    "[SCHED-INV] VIOLATION running task queued pid={} qc={}",
                    p.pid,
                    qc
                );
            }
            if oc != u8::MAX && qc != u8::MAX && oc != qc {
                serial_println!(
                    "[SCHED-INV] VIOLATION owning!=queued pid={} oc={} qc={} state={:?}",
                    p.pid,
                    oc,
                    qc,
                    p.state
                );
            }
            if is_running && oc == u8::MAX {
                serial_println!("[SCHED-INV] VIOLATION running no owner pid={}", p.pid);
            }
            if is_ready && qc != u8::MAX {
                // Verify presence would require full queue scan; flag flag vs core only here.
            }
            if oc != u8::MAX && is_ready {
                // A Ready with owner set means live window (pre-desched wakeup); ok if not enqueued elsewhere.
            }
        }
        // Also verify no duplicates across queues by counting presence vs flag (lightweight).
        let mut queued_flag_count = 0usize;
        for p in self.processes.iter() {
            if p.queued_on_core != u8::MAX {
                queued_flag_count += 1;
            }
        }
        let total_in_queues = ready_high + ready_mid + ready_low;
        if queued_flag_count != total_in_queues {
            serial_println!(
                "[SCHED-INV] VIOLATION queued_flag_count={} != queues_len={}",
                queued_flag_count,
                total_in_queues
            );
        }

        // Cross check: any running task present in any queue, or queued flag pointing to wrong core's queues.
        for c in 0..self.online_cores {
            let queue_iter: alloc::vec::Vec<usize> = if SCHEDULER_MODE == SchedulerMode::RoundRobin
            {
                let core = CORE_STATES[c].lock();
                core.rr_queue.iter().copied().collect()
            } else {
                let core = &self.cores[c];
                core.run_queue_high
                    .iter()
                    .chain(core.run_queue_medium.iter())
                    .chain(core.run_queue_low.iter())
                    .copied()
                    .collect()
            };
            for q in queue_iter {
                if q < self.processes.len() {
                    let p = &self.processes[q];
                    if p.state == ProcessState::Running {
                        serial_println!("[SCHED-INV] VIOLATION running task in queue pid={} in_core={} owning={}", p.pid, c, p.owning_core);
                    }
                    if p.queued_on_core as usize != c {
                        serial_println!("[SCHED-INV] VIOLATION queued flag mismatch pid={} flag={} actual_queue_core={}", p.pid, p.queued_on_core, c);
                    }
                }
            }
        }
        // Check running tasks are not duplicated in foreign queues
        for (idx, p) in self.processes.iter().enumerate() {
            if p.state == ProcessState::Running && p.owning_core != u8::MAX {
                let owner = p.owning_core as usize;
                // scan other cores queues
                for oc in 0..self.online_cores {
                    if oc == owner {
                        continue;
                    }
                    let present = if SCHEDULER_MODE == SchedulerMode::RoundRobin {
                        CORE_STATES[oc].lock().rr_queue.iter().any(|&x| x == idx)
                    } else {
                        let co = &self.cores[oc];
                        co.run_queue_high
                            .iter()
                            .chain(&co.run_queue_medium)
                            .chain(&co.run_queue_low)
                            .any(|&x| x == idx)
                    };
                    if present {
                        serial_println!("[SCHED-INV] VIOLATION running on {} present in foreign queue {} pid={}", owner, oc, p.pid);
                    }
                }
            }
        }

        serial_println!(
            "[SCHED-DIAG] created={} finished={} alive={} finished_slots={} ready_queues=({},{},{}) delta_created-finished={} blocked_ipc={} blocked_timer={} blocked_io={} online_cores={}",
            created, finished, alive, finished_slots, ready_high, ready_mid, ready_low,
            created.saturating_sub(finished),
            blocked_ipc, blocked_timer, blocked_io, self.online_cores
        );
        for core_id in 0..self.online_cores {
            let (
                current_task,
                current_ticks,
                q_high,
                q_mid,
                q_low,
                timer_ticks,
                context_switches,
                steal_count,
                stale_pops,
                empty_pops,
            ) = if SCHEDULER_MODE == SchedulerMode::RoundRobin {
                let core = CORE_STATES[core_id].lock();
                (
                    core.current_task,
                    core.current_ticks,
                    0,
                    core.rr_queue.len(),
                    0,
                    core.timer_ticks,
                    core.context_switches,
                    core.steal_count,
                    core.stale_pops,
                    core.empty_pops,
                )
            } else {
                let core = &self.cores[core_id];
                (
                    core.current_task,
                    core.current_ticks,
                    core.run_queue_high.len(),
                    core.run_queue_medium.len(),
                    core.run_queue_low.len(),
                    core.timer_ticks,
                    core.context_switches,
                    core.steal_count,
                    core.stale_pops,
                    core.empty_pops,
                )
            };

            match current_task {
                Some(idx) if idx < self.processes.len() => {
                    let process = &self.processes[idx];
                    serial_println!(
                        "[SCHED-DIAG] core={} current_idx={} pid={} name='{}' state={:?} ticks={} queues=({},{},{})",
                        core_id,
                        idx,
                        process.pid,
                        process.name_str(),
                        process.state,
                        current_ticks,
                        q_high,
                        q_mid,
                        q_low
                    );
                }
                _ => {
                    serial_println!(
                        "[SCHED-DIAG] core={} current_idx=none ticks={} queues=({},{},{})",
                        core_id,
                        current_ticks,
                        q_high,
                        q_mid,
                        q_low
                    );
                }
            }
            // Minimal extra for runtime matrix: expose ticks/switches per core.
            serial_println!(
                "[SCHED-DIAG] core={} timer_ticks={} ctx_switches={} steals={} stale_pops={} empty_pops={}",
                core_id,
                timer_ticks,
                context_switches,
                steal_count,
                stale_pops,
                empty_pops
            );
        }

        if alive > 0 && blocked_ipc == alive
            || (ready_high + ready_mid + ready_low == 0 && blocked_ipc > 0)
        {
            serial_println!("[SCHED-DIAG] IPC wait dump:");
            let caps = crate::capability::CAP_BROKER.lock();
            for p in self
                .processes
                .iter()
                .filter(|p| !matches!(p.state, ProcessState::Finished | ProcessState::Reaped))
            {
                let (pending_call_cap, pending_call_label, resolved_ep, resolved_owner) =
                    match p.pending_call {
                        Some(pending) => {
                            let cap = pending.target_cap;
                            let msg = pending.msg;
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

        // ── Accounting / leak-detection telemetry (per spec) ───────────────────
        let live_process_count = self
            .processes
            .iter()
            .filter(|p| !matches!(p.state, ProcessState::Finished | ProcessState::Reaped))
            .count();
        let finished_process_count = self
            .processes
            .iter()
            .filter(|p| p.state == ProcessState::Finished)
            .count();
        let reaped_process_count = self
            .processes
            .iter()
            .filter(|p| p.state == ProcessState::Reaped)
            .count();
        let process_slots_used = self.processes.len();
        let process_slots_finished = finished_process_count + reaped_process_count;

        // Best-effort kernel heap stats if available (heap module may export).
        // We log what we can; real numbers come from PMM + allocator diagnostics.
        let kernel_heap_info = crate::memory::heap::heap_stats();

        // PMM free pages (approx).
        let pmm_free = crate::PMM.lock().free_page_count();

        // Rough display surface/window counts are userspace-owned; we expose process stats here.
        // IPC queue/wait snapshot (sampled).
        let mut ipc_wait_entries: usize = 0;
        let mut ipc_queue_entries: usize = 0;
        for p in self.processes.iter() {
            ipc_queue_entries += p.ipc_queue.len();
        }
        crate::ipc::for_all_shards(|bus| {
            ipc_wait_entries += bus.total_waiter_count();
        });

        serial_println!(
            "[SCHED-ACCT] live_process_count={} finished_process_count={} reaped_process_count={} process_slots_used={} process_slots_finished={} ipc_wait_entries={} ipc_queue_entries={}",
            live_process_count,
            finished_process_count,
            reaped_process_count,
            process_slots_used,
            process_slots_finished,
            ipc_wait_entries,
            ipc_queue_entries
        );
        serial_println!(
            "[SCHED-ACCT] kernel_heap_allocated={} kernel_heap_free={} kernel_heap_reusable={} pmm_free_pages={}",
            kernel_heap_info.allocated,
            kernel_heap_info.free,
            kernel_heap_info.reusable,
            pmm_free
        );
    }
}

pub fn run_mm0_address_space_lifecycle_test(hhdm_offset: x86_64::VirtAddr) {
    use x86_64::structures::paging::{Page, PhysFrame, Size4KiB};

    fn borrower(
        pid: usize,
        owner_pid: usize,
        shared_address_space: crate::process::address_space::SharedAddressSpaceHandle,
    ) -> Process {
        crate::process::Process::new_thread(
            pid,
            owner_pid,
            "mm0-thread",
            shared_address_space,
            crate::process::fd_table::FdTable::new_boxed(),
            crate::process::env::EnvMap::new(),
            0,
            0,
            0,
            alloc::vec::Vec::new(),
            None,
        )
    }

    let free_before = crate::PMM.lock().free_page_count();
    let mut owner = {
        let mut pmm = crate::PMM.lock();
        unsafe { crate::process::Process::new(0xA001, 0, "mm0-owner", &mut pmm, hhdm_offset) }
    };
    let pml4 = owner.address_space.pml4_phys;
    let identity = owner.address_space.identity();
    let shared_address_space = owner.address_space.shared_handle();
    let owner_pid = owner.pid;
    let sentinel_region = crate::process::region::MappingRegion::new(
        0x40_0000,
        0x40_1000,
        crate::process::region::RegionProtection::READ_WRITE,
        crate::process::region::MappingKind::InternalUserMapping,
        crate::process::region::RegionPolicy::SYSTEM
            .union(crate::process::region::RegionPolicy::OWNER_MANAGED),
        crate::process::region::RegionBacking::Internal(0x4D4D_30),
    )
    .expect("MM-0 sentinel ledger range");
    let sentinel_reservation = owner
        .address_space
        .preflight_region(sentinel_region)
        .expect("MM-0 sentinel ledger reservation");

    // Give the owner a real writable/NX user mapping and sentinel. Borrower
    // teardown must preserve the frame and every lower page-table level.
    let sentinel_frame = {
        let mut pmm = crate::PMM.lock();
        let frame_addr = pmm
            .alloc_frame_owned(owner_pid as u32)
            .expect("MM-0 sentinel frame");
        assert!(crate::memory::security::sanitize_user_frame(
            frame_addr,
            hhdm_offset
        ));
        let page = Page::<Size4KiB>::from_start_address(x86_64::VirtAddr::new(0x40_0000))
            .expect("MM-0 sentinel page");
        let frame = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };
        unsafe {
            owner
                .address_space
                .map_page(
                    page,
                    frame,
                    crate::memory::security::user_stack_flags(),
                    &mut pmm,
                    hhdm_offset,
                )
                .expect("MM-0 owner sentinel mapping");
            owner
                .address_space
                .commit_region(sentinel_reservation)
                .expect("MM-0 sentinel ledger commit");
            (hhdm_offset + frame_addr.as_u64())
                .as_mut_ptr::<u64>()
                .write_volatile(0x534C_4D4D_3054_4852);
        }
        frame_addr
    };
    let free_with_owner_mapping = crate::PMM.lock().free_page_count();

    let mut sched = Scheduler::new();
    sched.processes.push(owner);

    // Generation one: twelve concurrent borrower records all finish and reap.
    for offset in 0..12 {
        sched
            .processes
            .push(borrower(0xA100 + offset, owner_pid, shared_address_space));
    }
    assert_eq!(sched.live_address_space_borrowers(0), 12);
    for borrower in &sched.processes[1..=12] {
        assert_eq!(borrower.address_space.region_count(), 1);
        assert_eq!(
            borrower.address_space.lookup_region(0x40_0000),
            Some(sentinel_region)
        );
    }
    let stale_wakes_before = STALE_TERMINAL_WAKEUPS_REJECTED.load(Ordering::Relaxed);
    for idx in 1usize..=12 {
        let pid = sched.processes[idx].pid;
        sched.processes[idx].state = ProcessState::Finished;
        sched.processes[idx].exit_cleanup_pending = true;
        sched.reap_process_resources(idx);
        assert_eq!(sched.processes[idx].state, ProcessState::Reaped);
        assert!(sched.processes[idx].kernel_stack.is_none());
        assert_eq!(sched.processes[idx].queued_on_core, u8::MAX);
        sched.enqueue_ready(idx);
        sched.enqueue_ready(idx);
        assert_eq!(sched.diagnostic_ready_occurrences(idx), 0);
        sched.wake_pid(pid);
        assert_eq!(sched.processes[idx].state, ProcessState::Reaped);
        assert_eq!(sched.processes[0].address_space.pml4_phys, pml4);
        assert_eq!(sched.processes[0].address_space.region_count(), 1);
        let sentinel = unsafe {
            (hhdm_offset + sentinel_frame.as_u64())
                .as_ptr::<u64>()
                .read_volatile()
        };
        assert_eq!(sentinel, 0x534C_4D4D_3054_4852);
    }
    assert_eq!(
        STALE_TERMINAL_WAKEUPS_REJECTED.load(Ordering::Relaxed),
        stale_wakes_before + 12
    );
    assert_eq!(crate::PMM.lock().free_page_count(), free_with_owner_mapping);

    // Generation two must reuse the twelve Reaped task slots without growing
    // the table or allowing a generation-one terminal state to wake/run.
    let slots_before = sched.processes.len();
    let mut second_generation_slots = alloc::vec::Vec::new();
    for offset in 0..12 {
        let idx = sched.add_process(borrower(0xB100 + offset, owner_pid, shared_address_space));
        assert!(!second_generation_slots.contains(&idx));
        second_generation_slots.push(idx);
    }
    assert_eq!(sched.processes.len(), slots_before);
    assert_eq!(sched.live_address_space_borrowers(0), 12);
    for idx in second_generation_slots {
        sched.processes[idx].state = ProcessState::Finished;
        sched.processes[idx].exit_cleanup_pending = true;
        sched.reap_process_resources(idx);
        assert_eq!(sched.processes[idx].state, ProcessState::Reaped);
    }
    assert_eq!(crate::PMM.lock().free_page_count(), free_with_owner_mapping);

    // Owner exit still contains a live borrower and delays address-space
    // reclamation until that borrower is terminal and Reaped.
    let live_idx = sched.add_process(borrower(0xC100, owner_pid, shared_address_space));
    sched.processes[0].state = ProcessState::Finished;
    sched.processes[0].exit_cleanup_pending = true;
    sched.reap_process_resources(0);
    assert_eq!(sched.processes[0].state, ProcessState::Finished);
    assert_eq!(sched.processes[live_idx].state, ProcessState::Finished);
    sched.reap_process_resources(live_idx);
    assert_eq!(sched.processes[live_idx].state, ProcessState::Reaped);
    sched.reap_process_resources(0);
    assert_eq!(sched.processes[0].state, ProcessState::Reaped);
    assert_eq!(crate::PMM.lock().free_page_count(), free_before);

    let mut replacement = {
        let mut pmm = crate::PMM.lock();
        unsafe { crate::process::Process::new(0xA002, 0, "mm0-reuse", &mut pmm, hhdm_offset) }
    };
    assert_ne!(replacement.address_space.identity(), identity);
    assert_eq!(replacement.address_space.region_count(), 0);
    {
        let mut pmm = crate::PMM.lock();
        unsafe {
            replacement
                .address_space
                .reclaim_user_space(&mut pmm, hhdm_offset, true);
        }
    }
    crate::serial_println!(
        "[MM-0] native borrower lifecycle: 12 finished/reaped + 12 recreated/reaped: OK"
    );
    crate::serial_println!("[MM-0] thread address-space lifecycle: OK");
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

/// Per-core idle loop target for synthetic kernel interrupt frames.
#[no_mangle]
extern "C" fn core_idle_entry() -> ! {
    loop {
        x86_64::instructions::interrupts::enable();
        x86_64::instructions::hlt();
    }
}

/// Build a saved interrupt frame that `iretq`s into [`core_idle_entry`] on a
/// dedicated kernel stack. Used so a core with no runnable task never resumes a
/// descheduled (blocked) user context when `idle_context_rsp` was unset.
fn build_kernel_idle_frame(stack_top: u64) -> u64 {
    const FRAME_SIZE: u64 = 160;
    let frame_base = stack_top - FRAME_SIZE;
    // SAFETY: `frame_base` lies within the per-core idle stack allocated in
    // `init_core_idle_contexts`.
    unsafe {
        let base = frame_base as *mut u64;
        for i in 0..15 {
            base.add(i).write_volatile(0);
        }
        base.add(15)
            .write_volatile(core_idle_entry as *const () as usize as u64);
        base.add(16).write_volatile(0x08); // kernel 64-bit code selector
        base.add(17).write_volatile(0x202); // IF=1
        base.add(18).write_volatile(stack_top.saturating_sub(8));
        base.add(19).write_volatile(0x10); // kernel data selector
    }
    frame_base
}

static mut CORE_IDLE_STACKS: [[u8; crate::process::KERNEL_STACK_SIZE]; MAX_CORES] =
    [[0; crate::process::KERNEL_STACK_SIZE]; MAX_CORES];

/// Top-of-stack address of the per-core static idle kernel stack for `cpu_id`.
///
/// This is the safer resting place for a CPU that has just called
/// `finish_current_process` (or `terminate_current_user_process`) — it must
/// `sti; hlt` waiting to be descheduled, but should do so on a static stack
/// rather than the dying process's heap-allocated `Box<[u8; 32K]> kernel_stack`.
/// A subsequent cross-core `Scheduler::add_process` can reuse the Finished slot
/// (`self.processes[id] = process` in this file) which drops the old `Box` and
/// frees that heap; if a CPU is still halted on the dying kernel stack, the
/// next timer IRQ pushes the IRET frame into the now-freed heap, and the
/// resulting `iretq` reads a zeroed/garbage CS and raises `#GP` (err=0) — the
/// desktop appears frozen because the gpf_handler loops on `hlt`.
pub(crate) fn core_idle_stack_top(cpu_id: usize) -> u64 {
    let cpu = cpu_id.min(MAX_CORES - 1);
    unsafe {
        core::ptr::addr_of!(CORE_IDLE_STACKS[cpu][crate::process::KERNEL_STACK_SIZE - 1]) as u64 + 1
    }
}

fn init_core_idle_contexts(online: usize) {
    for core_id in 0..online.min(MAX_CORES) {
        let stack_top = unsafe {
            core::ptr::addr_of!(CORE_IDLE_STACKS[core_id][crate::process::KERNEL_STACK_SIZE - 1])
                as u64
                + 1
        };
        let idle_rsp = build_kernel_idle_frame(stack_top);
        CORE_STATES[core_id].lock().idle_context_rsp = idle_rsp;
    }
}

fn idle_loop() -> ! {
    core_idle_entry();
}

// ─── Scheduler reschedule requests ────────────────────────────────────────────

#[inline]
fn reschedule_bit(cpu_id: usize) -> u64 {
    1u64 << cpu_id.min(63)
}

/// Check if a reschedule is needed for `cpu_id` and clear only that CPU's bit.
/// Uses AcqRel: cheaper than SeqCst (no store-load fence) but still correct
/// because only the owning core ever clears its own bit.
pub fn check_reschedule_on(cpu_id: usize) -> bool {
    let bit = reschedule_bit(cpu_id);
    RESCHEDULE_MASK.fetch_and(!bit, Ordering::AcqRel) & bit != 0
}

/// Non-consuming peek: returns true if the reschedule bit is set for `cpu_id`
/// without clearing it. Safe to call without holding the scheduler lock —
/// each core owns its own bit and no other core clears it.
#[inline]
pub fn peek_reschedule_on(cpu_id: usize) -> bool {
    RESCHEDULE_MASK.load(Ordering::Relaxed) & reschedule_bit(cpu_id) != 0
}

/// Set the reschedule flag for a specific CPU.
pub fn request_reschedule_on(cpu_id: usize) {
    RESCHEDULE_MASK.fetch_or(reschedule_bit(cpu_id), Ordering::AcqRel);
}

/// Set the reschedule flag for the current CPU.
pub fn request_reschedule() {
    request_reschedule_on(current_cpu_id());
}

pub fn note_process_finished(pid: usize, name: &str) {
    PROCESS_FINISHED.fetch_add(1, Ordering::Relaxed);
    serial_println!("[SCHED] FINISHED process pid={} name='{}'", pid, name);
}

pub fn note_native_borrower_created() {
    NATIVE_BORROWERS_CREATED.fetch_add(1, Ordering::Relaxed);
    let live = LIVE_NATIVE_BORROWERS.fetch_add(1, Ordering::Relaxed) + 1;
    MAX_LIVE_NATIVE_BORROWERS.fetch_max(live, Ordering::Relaxed);
}

fn note_native_borrower_finished() {
    NATIVE_BORROWERS_FINISHED.fetch_add(1, Ordering::Relaxed);
    let _ = LIVE_NATIVE_BORROWERS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |live| {
        Some(live.saturating_sub(1))
    });
}

pub fn native_borrower_lifecycle_counts() -> (usize, usize, usize, usize, usize, usize, usize) {
    (
        NATIVE_BORROWERS_CREATED.load(Ordering::Relaxed),
        NATIVE_BORROWERS_FINISHED.load(Ordering::Relaxed),
        NATIVE_BORROWERS_REAPED.load(Ordering::Relaxed),
        BORROWER_USER_RECLAIM_SKIPPED.load(Ordering::Relaxed),
        STALE_TERMINAL_WAKEUPS_REJECTED.load(Ordering::Relaxed),
        BORROWER_SLOTS_REUSED.load(Ordering::Relaxed),
        MAX_LIVE_NATIVE_BORROWERS.load(Ordering::Relaxed),
    )
}

/// Mark the current process finished and release resources.
///
/// Returns the **per-core static idle stack top** (NOT the dying process's
/// heap-allocated `kernel_stack_top`) for the caller's final `sti; hlt` loop.
/// Callers (`process_exit` in `arch/x86_64/syscall.rs` and
/// `terminate_current_user_process` in `arch/x86_64/interrupts.rs`) pivot RSP
/// to this value, then halt until the next timer IRQ deschedules them.
///
/// Returning the dying process's own `kernel_stack_top` here would leave the
/// CPU halted on a `Box`-owned per-process stack whose memory is freed when
/// `Scheduler::add_process` later reuses the Finished slot via
/// `self.processes[id] = process`. The next timer IRQ would push an IRET
/// frame into freed heap and the resulting `iretq` would load a zeroed CS
/// and raise `#GP` (err=0) — the desktop "freeze after several app launches
/// from the file manager" symptom.
pub fn finish_current_process(code: i32, reason: &str) -> u64 {
    serial_println!("[SCHED] process_exit_begin reason={}", reason);
    let mut sched = SCHEDULER.lock();
    let cpu_id = current_cpu_id();
    let idle_top = core_idle_stack_top(cpu_id);
    let cur = match sched.process_index_for_cpu(cpu_id) {
        Some(idx) => idx,
        None => {
            serial_println!(
                "[SCHED] WARN: finish_current_process with no current_task on cpu={}",
                cpu_id
            );
            return idle_top;
        }
    };
    if cur >= sched.processes.len() {
        serial_println!(
            "[SCHED] WARN: finish_current_process invalid current idx={} cpu={}",
            cur,
            cpu_id
        );
        return idle_top;
    }

    if matches!(
        sched.processes[cur].state,
        crate::process::ProcessState::Finished | crate::process::ProcessState::Reaped
    ) {
        return idle_top;
    }

    let my_pid = sched.processes[cur].pid;
    let my_name: alloc::string::String = sched.processes[cur].name_str().into();
    serial_println!(
        "[SCHED] process_mark_finished pid={} name='{}' reason={} code={}",
        my_pid,
        my_name,
        reason,
        code
    );

    sched.account_current_runtime();
    let is_borrower = sched.processes[cur].native_thread;
    {
        let process = &mut sched.processes[cur];
        process.exit_code = code;
        process.state = crate::process::ProcessState::Finished;
        process.exit_cleanup_pending = true;
        process.owning_core = u8::MAX;
        process.queued_on_core = u8::MAX;
    }
    if is_borrower {
        note_native_borrower_finished();
    }
    sched.set_core_current_task(cpu_id, None);
    sched.set_core_current_ticks(cpu_id, 0);
    sched.remove_from_ready_queues(cur);
    if sched.processes[cur].owns_address_space {
        sched.terminate_address_space_borrowers(cur, "address-space-owner-exit");
    }
    sched.close_owned_ipc_endpoints(my_pid);

    serial_println!(
        "[SCHED] terminating pid={} name='{}' reason={}",
        my_pid,
        my_name,
        reason
    );
    note_process_finished(my_pid, &my_name);

    let parent_pid = sched.processes[cur].ppid;
    let parent_waiting = sched
        .process_mut_by_pid(parent_pid)
        .is_some_and(|parent| parent.wait_child == Some(my_pid));
    if parent_waiting {
        sched.wake_pid(parent_pid);
    }

    idle_top
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
    let (rsp, fs_base, kernel_stack_top) = {
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
            sched.set_core_current_task(0, Some(idx));
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
            let fs_base = sched.processes[idx].fs_base;
            let kernel_stack_top = sched.processes[idx].kernel_stack_top;
            unsafe {
                sched.processes[idx].address_space.activate();
            }
            serial_println!(
                "[SCHED] Entering process {} '{}' at rsp={:#x}",
                idx,
                sched.processes[idx].name_str(),
                rsp
            );
            (rsp, fs_base, kernel_stack_top)
        } else {
            serial_println!("[SCHED] No user processes, entering idle");
            drop(sched);
            idle_loop();
        }
    };

    unsafe {
        x86_64::registers::model_specific::Msr::new(0xC0000100).write(fs_base);
        crate::arch::x86_64::smp::set_current_cpu_tss_rsp0(kernel_stack_top);
        context::iretq_to_context(rsp);
    }
}

pub fn current_process_rsp() -> u64 {
    let sched = SCHEDULER.lock();
    let cpu_id = current_cpu_id();
    match sched.core_current_task(cpu_id) {
        Some(idx) => sched.processes[idx].context_rsp,
        None => 0,
    }
}

/// Best-effort current PID for exception diagnostics. Never spins on
/// `SCHEDULER`; fault handlers may run while that lock is already held.
pub fn try_current_pid() -> usize {
    SCHEDULER
        .try_lock()
        .map(|sched| sched.current_pid())
        .unwrap_or(usize::MAX)
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
    init_core_idle_contexts(count);
    {
        let mut sched = SCHEDULER.lock();
        sched.online_cores = count;
        for core_id in 0..count {
            let idle_rsp = CORE_STATES[core_id].lock().idle_context_rsp;
            sched.cores[core_id].idle_context_rsp = idle_rsp;
        }
    }
    for core_id in 0..count {
        CORE_STATES[core_id].lock().rr_queue.reserve(128);
    }
    serial_println!(
        "[SCHED] Per-core work-stealing scheduler: {} online core(s)",
        count
    );
}

pub mod context;
