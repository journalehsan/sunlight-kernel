//! SunBrust RR Scheduler (SunlightOS)
//!
//! This module implements the "SunBrust" round-robin scheduler with BORE-inspired
//! burst tracking, nice-weighted counters, and tiered ready queues.
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
//! ### Tiered Queues (BORE mode)
//! Three ready queues (high/med/low) are maintained. pick_next_bore prefers
//! high then medium then low. Within a tier, simple FIFO with "skip current"
//! to avoid immediate re-run of the just-preempted task.
//!
//! In RoundRobin mode the tier queues are ignored; pick_next_round_robin performs
//! a single linear scan over processes with nice-weighted promotion and skip logic.
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
//! sunlight-top reads cpu_ticks (effective runtime at publish) and previous values
//! it stores locally, then computes machine-normalized deltas using:
//!   capacity_delta = interval_ns * online_cpu_count
//!   process_bp   = process_delta_ns * 10000 / capacity_delta
//!   used_bp      = sum(non_idle_deltas) * 10000 / capacity_delta
//!   idle_bp      = 10000 - used_bp (or derived from idle if tracked)
//!
//! ## Invariants (Phase 2)
//! - Only Ready or Running processes may reside in the tiered ready queues.
//! - Blocking (IPC, timer, IO, waitpid, yield-to-block, exit, suspend) removes
//!   the task from ready queues before it stops being current.
//! - pick_next_* only returns Ready tasks (or falls back safely).
//!
//! Overhead in the hot path is deliberately kept minimal: the dominant cost on
//! context switch is rdtsc + a few branches and the existing queue/tier logic.

use crate::arch::x86_64::interrupts::now_ns;
use crate::process::{Process, ProcessState, QueueTier};
use crate::serial_println;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

/// Flag set by timer IRQ when a reschedule is needed.
static NEEDS_RESCHEDULE: AtomicBool = AtomicBool::new(false);

/// Diagnostic: track first time sunlightd (pid 6) is picked by scheduler.
static SUNLIGHTD_FIRST_SCHED: AtomicBool = AtomicBool::new(false);

/// === Diagnostic counters for process leak detection ===
static PROCESS_CREATED: AtomicUsize = AtomicUsize::new(0);
static PROCESS_FINISHED: AtomicUsize = AtomicUsize::new(0);
// Adjust the timeslice constants to your needs
pub const QUANTUM_MIN: u32 = 5;
pub const QUANTUM_MAX: u32 = 50;

fn calculate_quantum_with_nice(burst_score: u32, nice: i8) -> u32 {
    // 1. Convert to i32 for safe calculations (supports negative numbers)
    // Each 1 nice unit is equivalent to 16 jump units in the burst graph
    let nice_modifier = (nice as i32) * 16;

    // 2. Algebraic sum of dynamic score with static priority
    // Positive nice (lower priority) -> score increases -> timeslice decreases
    // Negative nice (higher priority) -> score decreases -> timeslice increases
    let effective_score = (burst_score as i32 + nice_modifier)
        // With clamp, we make sure we don't go outside the allowed range [0, 1024]
        .clamp(0, BURST_SCORE_MAX as i32) as u32;

    // 3. Continue calculating weight and quantum as before
    // [FIX-4] Interactive tasks (low score) get short quanta; CPU-bound tasks
    // (high score) get long quanta. Previously this was inverted.
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
/// nice == 0 tasks are pure round-robin and never have their counter touched.
fn accumulate_counter(process: &mut Process) {
    if process.nice == 0 {
        return; // [FEAT-3] pure RR for neutral tasks
    }

    // CPU-bound tasks feel nice value MORE strongly than interactive ones.
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
/// Called after a process's quantum ends, before it is re-enqueued.
fn on_task_ran(process: &mut Process, ticks_used: u64) {
    process.aging_boosted_this_pick = false; // [FEAT-3] reset guard

    let q = quantum_ticks(process);
    process.quantum_override = None; // [FEAT-3] consume override

    if ticks_used < (q / 2) as u64 {
        // Interactive: used less than half the quantum. Fast decay toward
        // neutral, preserving sign (halving doesn't flip polarity).
        process.counter /= 2; // [FEAT-3]
    } else {
        // CPU-bound: slow linear decay.
        if process.counter > 0 {
            process.counter -= DECAY_RATE; // [FEAT-3]
        } else if process.counter < 0 {
            process.counter += DECAY_RATE; // [FEAT-3]
        }
    }
}

pub struct Scheduler {
    pub processes: Vec<Process>,

    // BORE: Tiered ready queues by priority
    pub ready_queue_high: VecDeque<usize>, // Burst 0-256 (interactive)
    pub ready_queue_medium: VecDeque<usize>, // Burst 257-768
    pub ready_queue_low: VecDeque<usize>,  // Burst 769-1024 (CPU-bound)

    pub current: usize,
    pub current_ticks: u64,
    pub global_tick: u64, // Ever-incrementing counter
    pub idle_context_rsp: u64,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            processes: Vec::new(),
            ready_queue_high: VecDeque::new(),
            ready_queue_medium: VecDeque::new(),
            ready_queue_low: VecDeque::new(),
            current: 0,
            current_ticks: 0,
            global_tick: 0,
            idle_context_rsp: 0,
        }
    }

    /// Add a process to the scheduler.
    pub fn add_process(&mut self, process: Process) -> usize {
        let created_count = PROCESS_CREATED.fetch_add(1, Ordering::Relaxed);

        // Reuse a finished slot when possible so we avoid growing Vec<Process>
        // unboundedly under spawn/exit churn. `Process` is large, so a Vec
        // growth reallocation can fail even when only one additional process
        // is being created.
        if let Some(id) = self
            .processes
            .iter()
            .enumerate()
            .find(|(idx, p)| *idx != self.current && p.state == ProcessState::Finished)
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

        // Don't queue here - let enqueue_process() handle it
        // This avoids duplicates when a process is first started in run_forever()

        id
    }

    /// Enqueue a Ready process to the appropriate tier queue
    pub fn enqueue_process(&mut self, idx: usize) {
        if idx >= self.processes.len() {
            return;
        }
        if !matches!(self.processes[idx].state, ProcessState::Ready) {
            return;
        }
        self.remove_from_ready_queues(idx);
        let tier = self.processes[idx].get_queue_tier();
        match tier {
            QueueTier::High => self.ready_queue_high.push_back(idx),
            QueueTier::Medium => self.ready_queue_medium.push_back(idx),
            QueueTier::Low => self.ready_queue_low.push_back(idx),
        }
    }

    /// Enqueue a Ready process once, avoiding stale duplicate queue entries.
    pub fn enqueue_process_once(&mut self, idx: usize) {
        if idx >= self.processes.len()
            || !matches!(self.processes[idx].state, ProcessState::Ready)
            || self.is_queued(idx)
        {
            return;
        }
        self.enqueue_process(idx);
    }

    pub fn remove_from_ready_queues(&mut self, idx: usize) {
        self.ready_queue_high.retain(|&queued| queued != idx);
        self.ready_queue_medium.retain(|&queued| queued != idx);
        self.ready_queue_low.retain(|&queued| queued != idx);
    }

    fn is_queued(&self, idx: usize) -> bool {
        self.ready_queue_high.iter().any(|&queued| queued == idx)
            || self.ready_queue_medium.iter().any(|&queued| queued == idx)
            || self.ready_queue_low.iter().any(|&queued| queued == idx)
    }

    /// Seed all currently Ready processes except the one already running.
    pub fn seed_ready_queues_except(&mut self, running_idx: usize) {
        self.ready_queue_high.clear();
        self.ready_queue_medium.clear();
        self.ready_queue_low.clear();

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

    /// Set a per-process watchdog: if a single quantum runs longer than
    /// `period_ticks`, the process is suspended. [FEAT-1]
    pub fn set_process_watchdog(&mut self, pid: usize, period_ticks: u64) {
        if let Some(p) = self.process_mut_by_pid(pid) {
            p.wd_period_ticks = Some(period_ticks); // [FEAT-1]
        }
    }

    /// Disable the per-process watchdog. [FEAT-1]
    pub fn clear_process_watchdog(&mut self, pid: usize) {
        if let Some(p) = self.process_mut_by_pid(pid) {
            p.wd_period_ticks = None; // [FEAT-1]
        }
    }

    /// Reset a process's nice-weighted counter (e.g. when nice changes at runtime). [FEAT-3]
    pub fn reset_counter(&mut self, pid: usize) {
        if let Some(p) = self.process_mut_by_pid(pid) {
            p.counter = 0; // [FEAT-3]
        }
    }

    /// Sample the monotonic kernel clock (nanoseconds).
    #[inline]
    pub fn now_ns(&self) -> u64 {
        now_ns()
    }

    /// Stop charging CPU time to the current process and accumulate elapsed.
    /// Safe to call when current is valid. Returns the delta that was charged (0 if none).
    pub fn account_current_runtime(&mut self) -> u64 {
        let now = now_ns();
        if self.current >= self.processes.len() {
            return 0;
        }
        let p = &mut self.processes[self.current];
        let mut delta: u64 = 0;
        if p.last_start_ns != 0 {
            delta = now.saturating_sub(p.last_start_ns);
            p.cpu_runtime_ns = p.cpu_runtime_ns.saturating_add(delta);
        }
        p.last_start_ns = 0;
        delta
    }

    /// Account runtime for current and, if the just-run burst was extremely short,
    /// apply a temporary priority penalty (increase burst_score) to reduce idle churn.
    /// Called on the deschedule path for Phase 4.
    pub fn account_and_apply_churn_penalty(&mut self) -> u64 {
        let delta = self.account_current_runtime();
        if delta > 0 && delta < SHORT_BURST_NS {
            if self.current < self.processes.len() {
                let p = &mut self.processes[self.current];
                // Bump burst score => appears more CPU-bound => lower scheduling priority
                // for a while. This decays via normal full-quantum/aging paths.
                p.burst_score = p
                    .burst_score
                    .saturating_add(SHORT_BURST_PENALTY)
                    .min(BURST_SCORE_MAX);
            }
        }
        delta
    }

    /// Begin charging CPU time to the given process (mark start time).
    pub fn start_charging_runtime(&mut self, idx: usize) {
        if idx >= self.processes.len() {
            return;
        }
        self.processes[idx].last_start_ns = now_ns();
    }

    /// Compute effective runtime for a process, including uncommitted time
    /// if the process is currently Running on CPU.
    ///
    /// Per CPU accounting rules: a task's committed cpu_runtime_ns is updated
    /// on context switch-out. If the task is currently running, add the
    /// elapsed time since last_start_ns.
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

    /// Set state and ensure ready queues only contain runnable tasks.
    /// Non-runnable tasks are removed from all tier queues.
    pub fn set_state(&mut self, idx: usize, new_state: ProcessState) {
        if idx >= self.processes.len() {
            return;
        }
        let old_state = self.processes[idx].state;
        self.processes[idx].state = new_state;

        let runnable = matches!(new_state, ProcessState::Ready | ProcessState::Running);
        if !runnable {
            self.remove_from_ready_queues(idx);
        } else if matches!(old_state, ProcessState::Ready | ProcessState::Running)
            && matches!(new_state, ProcessState::Ready)
        {
            // Will be (re)enqueued by enqueue path as needed for tier correctness.
        }
    }

    /// Called from timer IRQ — may set the reschedule flag.
    pub fn tick(&mut self) {
        self.global_tick += 1;
        self.current_ticks += 1;

        // This is the key line! We get the timeslice based on the current behavior of the process:
        let current = self.current;
        let quantum = quantum_ticks(&self.processes[current]) as u64; // [FEAT-3]
                                                                      // if for sshl set to less than 5 reset to -1 :)

        // print quantum in serial
        // serial_println!(
        //     "[SCHED] Quantum {} ticks (process #{} '{}')",
        //     quantum,
        //     current,
        //     self.processes[current].pid
        // );

        if self.current_ticks >= quantum {
            // Process used full quantum
            let ticks_used = self.current_ticks;
            let current_proc = &mut self.processes[current];
            current_proc.timeslice_used = ticks_used as u32;

            // Update burst score for full quantum usage
            update_burst_score(current_proc, BurstReason::FullQuantum);

            // [FEAT-3] Counter decay / override consumption is RoundRobin-only.
            if SCHEDULER_MODE == SchedulerMode::RoundRobin {
                on_task_ran(current_proc, ticks_used);
            } else {
                current_proc.quantum_override = None;
                current_proc.aging_boosted_this_pick = false;
            }

            // [FEAT-1] Per-process watchdog: suspend if this quantum ran too long.
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

            // Age processes that haven't run recently
            self.age_ready_tasks();

            // Request reschedule
            self.current_ticks = 0;
            NEEDS_RESCHEDULE.store(true, Ordering::SeqCst);
        }

        if self.global_tick.is_multiple_of(1000) {
            self.diagnostic_report();
        }
    }

    fn age_ready_tasks(&mut self) {
        // [FIX-1] Only age on multiples of AGING_INTERVAL_TICKS; previously
        // this condition was inverted and aged on ~99% of ticks.
        if !self.global_tick.is_multiple_of(AGING_INTERVAL_TICKS) {
            return; // [FIX-1] skip unless this is an aging tick
        }

        for idx in 0..self.processes.len() {
            let p = &mut self.processes[idx];

            // Only age Ready (not Running/Blocked*) processes
            if !matches!(p.state, ProcessState::Ready) {
                continue;
            }

            // Check if process has been waiting too long
            let ticks_since_run = self.global_tick - p.last_run_tick;
            if ticks_since_run > AGING_THRESHOLD_TICKS {
                update_burst_score(p, BurstReason::Aged);
            }
        }
    }

    /// Pick the next Ready process using BORE tiered queues
    pub fn pick_next_bore(&mut self) -> Option<usize> {
        // [FIX-2] Each queue pop gets its own isolated "skipped current"
        // variable so a value popped from one tier doesn't leak into the
        // re-enqueue decision for another tier.
        let mut skipped_high: Option<usize> = None;
        let mut skipped_medium: Option<usize> = None;
        let mut skipped_low: Option<usize> = None;

        if let Some(idx) = pop_ready_excluding_current(
            &mut self.ready_queue_high,
            &self.processes,
            self.current,
            &mut skipped_high,
        ) {
            if let Some(current) = skipped_high {
                self.enqueue_process_once(current);
            }
            return Some(idx);
        }

        if let Some(idx) = pop_ready_excluding_current(
            &mut self.ready_queue_medium,
            &self.processes,
            self.current,
            &mut skipped_medium,
        ) {
            if let Some(current) = skipped_high {
                self.enqueue_process_once(current);
            }
            if let Some(current) = skipped_medium {
                self.enqueue_process_once(current);
            }
            return Some(idx);
        }

        if let Some(idx) = pop_ready_excluding_current(
            &mut self.ready_queue_low,
            &self.processes,
            self.current,
            &mut skipped_low,
        ) {
            if let Some(current) = skipped_high {
                self.enqueue_process_once(current);
            }
            if let Some(current) = skipped_medium {
                self.enqueue_process_once(current);
            }
            if let Some(current) = skipped_low {
                self.enqueue_process_once(current);
            }
            return Some(idx);
        }

        if let Some(current) = skipped_high.or(skipped_medium).or(skipped_low) {
            return Some(current);
        }

        // Fallback: if queues are empty but processes exist, do a linear search (safety net)
        let len = self.processes.len();
        if len == 0 {
            return None;
        }
        let start = (self.current + 1) % len;
        let mut idx = start;
        loop {
            if matches!(self.processes[idx].state, ProcessState::Ready) {
                serial_println!(
                    "[SCHED] WARNING: pick_next_bore fallback to linear search, idx={}",
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

    /// Pick the next Ready process for RoundRobin mode. [FEAT-3]
    ///
    /// Turn ORDER remains round-robin (every Ready task gets a turn), but
    /// nice != 0 tasks can be promoted earlier or deferred within a cycle via
    /// the nice-weighted counter system. nice == 0 tasks are pure RR.
    pub fn pick_next_round_robin(&mut self) -> Option<usize> {
        let len = self.processes.len();
        if len == 0 {
            return None;
        }

        // Safety bound: exactly one full pass through the queue.
        // Do NOT use len*2. A task enqueued-back cannot be seen again
        // within len attempts, preventing double accumulation.
        let mut attempts = len;
        let start = (self.current + 1) % len;
        let mut idx = start;

        loop {
            if attempts == 0 {
                break;
            }
            attempts -= 1;

            // Skip non-Ready processes silently
            if !matches!(self.processes[idx].state, ProcessState::Ready) {
                idx = (idx + 1) % len;
                if idx == start && attempts == 0 {
                    break;
                }
                continue;
            }

            // ── Starvation rescue BEFORE accumulate_counter ──────────────
            // Must run before accumulate so deeply-penalized starving tasks
            // still get rescued (otherwise skip fires before boost applies).
            let ticks_waiting = self
                .global_tick
                .saturating_sub(self.processes[idx].last_run_tick);
            if ticks_waiting > AGING_THRESHOLD_TICKS && !self.processes[idx].aging_boosted_this_pick
            {
                self.processes[idx].counter =
                    (self.processes[idx].counter + STARVATION_BOOST).min(MAX_CREDIT); // [FEAT-3] capped
                self.processes[idx].aging_boosted_this_pick = true; // one-shot per pick
                update_burst_score(&mut self.processes[idx], BurstReason::Aged);
            }

            // ── Accumulate counter for this candidate ────────────────────
            accumulate_counter(&mut self.processes[idx]); // [FEAT-3]

            // ── High priority promotion ───────────────────────────────────
            if self.processes[idx].nice < 0 && self.processes[idx].counter >= PROMOTE_LIMIT {
                // Partial spend: keep residual credit, don't reset to zero.
                self.processes[idx].counter =
                    0_i32.max(self.processes[idx].counter - PROMOTE_LIMIT); // [FEAT-3]

                // 10% quantum bonus via quantum_override (fixed-point: *110/100)
                let base_q = calculate_quantum_with_nice(
                    self.processes[idx].burst_score,
                    self.processes[idx].nice,
                );
                let quantum_override_value = ((base_q * 110) / 100).min(QUANTUM_MAX);
                self.processes[idx].quantum_override = Some(quantum_override_value); // [FEAT-3]

                serial_println!(
                    "[SCHED-RR] promoted pid={} nice={} counter={} quantum_override={}",
                    self.processes[idx].pid,
                    self.processes[idx].nice,
                    self.processes[idx].counter,
                    quantum_override_value
                );

                self.processes[idx].aging_boosted_this_pick = false;
                return Some(idx);
            }

            // ── Low priority skip (debt zone) ─────────────────────────────
            if self.processes[idx].nice > 0 && self.processes[idx].counter <= SKIP_LIMIT {
                // Partial debt recovery each time we skip
                self.processes[idx].counter += DECAY_RATE; // [FEAT-3]

                serial_println!(
                    "[SCHED-RR] skipped pid={} nice={} counter={}",
                    self.processes[idx].pid,
                    self.processes[idx].nice,
                    self.processes[idx].counter
                );

                self.processes[idx].aging_boosted_this_pick = false;
                // Move to next — this task deferred for this cycle
                idx = (idx + 1) % len;
                if idx == start && attempts == 0 {
                    break;
                }
                continue;
            }

            // ── Normal pick (nice==0 or counter in neutral zone) ─────────
            self.processes[idx].aging_boosted_this_pick = false;
            return Some(idx);
        }

        // ── Fallback: all tasks were skipped ─────────────────────────────
        // Bounded debt means this is rare. Find the least-indebted Ready task.
        // This prevents livelock when every task is in debt simultaneously.
        let mut best_idx = None;
        let mut best_counter = i32::MIN;
        let mut scan = start;
        for _ in 0..len {
            if matches!(self.processes[scan].state, ProcessState::Ready)
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
        }

        best_idx // [FEAT-3] pick least-indebted rather than blind front
    }

    pub fn pick_next(&mut self) -> Option<usize> {
        let next = match SCHEDULER_MODE {
            SchedulerMode::RoundRobin => self.pick_next_round_robin(),
            SchedulerMode::Bore => self.pick_next_bore(),
        };
        // Diagnostic 1b: one-time log the FIRST time sunlightd's pid is picked for execution.
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

    /// Get the currently running process.
    pub fn current_process(&self) -> &Process {
        &self.processes[self.current]
    }

    pub fn current_process_mut(&mut self) -> &mut Process {
        &mut self.processes[self.current]
    }

    pub fn is_blocked_on_recv(&self, pid: usize) -> bool {
        self.processes
            .iter()
            .any(|p| p.pid == pid && p.state == ProcessState::BlockedOnIpc)
    }

    pub fn wake_pid(&mut self, pid: usize) {
        // Find the process by PID and get its index
        let idx = match self.processes.iter().position(|p| p.pid == pid) {
            Some(i) => i,
            None => return,
        };

        if self.processes[idx].state == ProcessState::BlockedOnIpc {
            // Calculate how long was blocked
            let ticks_blocked = self.global_tick - self.processes[idx].block_start_tick;

            // Early block = high interactivity
            if ticks_blocked < INTERACTIVE_DETECTION_THRESHOLD as u64 {
                update_burst_score(&mut self.processes[idx], BurstReason::EarlyBlock);
            }

            // Update state and enqueue. [FIX-3] update_burst_score may change
            // this process's queue tier, so force-remove from any tier queue
            // before re-enqueuing at the (possibly new) correct tier — using
            // enqueue_process_once() here could no-op if it's still queued
            // under its old tier.
            self.processes[idx].state = ProcessState::Ready;
            self.remove_from_ready_queues(idx); // [FIX-3] force remove from any tier
            self.enqueue_process(idx); // then enqueue fresh at correct tier
        }
    }

    /// Wake a process blocked on a timer/sleep. [FEAT-2]
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
            self.remove_from_ready_queues(idx); // [FIX-3] / [FEAT-2]
            self.enqueue_process(idx);
        }
    }

    /// Wake a process blocked on I/O completion. [FEAT-2]
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
            self.remove_from_ready_queues(idx); // [FIX-3] / [FEAT-2]
            self.enqueue_process(idx);
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
                self.enqueue_process(idx);
            }
            _ => {}
        }
    }

    fn address_space_is_shared(&self, idx: usize) -> bool {
        let pml4 = self.processes[idx].address_space.pml4_phys;
        self.processes
            .iter()
            .enumerate()
            .any(|(other_idx, proc)| other_idx != idx && proc.state != ProcessState::Finished && proc.address_space.pml4_phys == pml4)
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
            let mut bus = crate::ipc::IPC_BUS.lock();
            bus.remove_pid_references(pid);
            for endpoint_id in &endpoint_ids {
                bus.remove_endpoint(*endpoint_id);
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
            if proc.ipc_reply_target.is_some_and(|(_, client_pid)| client_pid == pid) {
                proc.ipc_reply_target = None;
            }
        }

        let reclaim = {
            let mut pmm = crate::PMM.lock();
            unsafe {
                self.processes[idx]
                    .address_space
                    .reclaim_user_space(&mut *pmm, hhdm_offset, free_root)
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
        if idx == self.current || self.processes[idx].state == ProcessState::Finished {
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

    /// Get BORE diagnostics for a process
    pub fn get_process_burst_info(&self, pid: usize) -> Option<(u32, ProcessState)> {
        self.processes
            .iter()
            .find(|p| p.pid == pid)
            .map(|p| (p.burst_score, p.state))
    }

    /// Run the scheduler — enter the first process and never return.
    pub fn run_forever(&mut self) -> ! {
        // Find first Ready process.
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
            self.current = idx;
            self.processes[idx].state = ProcessState::Running;
            self.processes[idx].last_run_tick = self.global_tick;
            self.start_charging_runtime(idx);

            // Enqueue other Ready processes (idx might not be first in order)
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
            // Switch to the process's address space before entering user space.
            unsafe {
                self.processes[idx].address_space.activate();
            }
            // SAFETY: rsp points to a valid context frame on the process's kernel stack.
            unsafe {
                context::iretq_to_context(rsp);
            }
        }

        // No user processes — enter idle loop directly.
        serial_println!("[SCHED] No user processes, entering idle");
        idle_loop();
    }

    /// Print diagnostic information about process lifecycle
    pub fn diagnostic_report(&self) {
        let created = PROCESS_CREATED.load(Ordering::Relaxed);
        let finished = PROCESS_FINISHED.load(Ordering::Relaxed);
        let alive = self
            .processes
            .iter()
            .filter(|p| p.state != ProcessState::Finished)
            .count();
        let finished_slots = self.processes.len().saturating_sub(alive);
        let ready_high = self.ready_queue_high.len();
        let ready_mid = self.ready_queue_medium.len();
        let ready_low = self.ready_queue_low.len();

        // [FEAT-2] Per-blocked-state diagnostics.
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
            "[SCHED-DIAG] created={} finished={} alive={} finished_slots={} ready_queues=({},{},{}) delta_created-finished={} blocked_ipc={} blocked_timer={} blocked_io={}",
            created, finished, alive, finished_slots, ready_high, ready_mid, ready_low,
            created.saturating_sub(finished),
            blocked_ipc, blocked_timer, blocked_io
        );

        if alive > 0 && blocked_ipc == alive
            || (ready_high + ready_mid + ready_low == 0 && blocked_ipc > 0)
        {
            serial_println!("[SCHED-DIAG] IPC wait dump:");
            let caps = crate::capability::CAP_BROKER.lock();
            let bus = crate::ipc::IPC_BUS.lock();
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
                        (
                            resolved_ep,
                            bus.pending_count(resolved_ep),
                            bus.waiting_receiver_pid(resolved_ep, self)
                                .unwrap_or(usize::MAX),
                            bus.pending_callers_count(resolved_ep),
                        )
                    } else {
                        (u32::MAX, 0, usize::MAX, 0)
                    };
                // Stuck rendezvous: caller is blocked calling endpoint E,
                // the server that owns E is also blocked waiting to receive
                // on E, yet no delivery happened. This should be impossible
                // with a correct multi-caller queue: the kernel must match
                // callers to a waiting receiver immediately.
                //
                // False positive guard: we check waiting_receiver == resolved_owner
                // (server actively blocked on ipc_recv/reply_wait for THIS endpoint).
                // If the server is processing a previous message or is blocked on a
                // different endpoint, waiting_receiver != resolved_owner and this
                // warning will not fire — so tty_server busy while sshl handles a
                // command does NOT trigger this.
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
                serial_println!(
                    "[IPC-DIAG] ep={} owner={} waiting_receiver={} pending_callers={}",
                    ep,
                    owner,
                    bus.waiting_receiver_pid(ep, self).unwrap_or(usize::MAX),
                    bus.pending_callers_count(ep)
                );
            }
        }
    }
}

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

/// Idle loop — runs when no user process is Ready.
fn idle_loop() -> ! {
    loop {
        x86_64::instructions::interrupts::enable();
        x86_64::instructions::hlt();
    }
}

/// Check if a reschedule is needed and clear the flag.
pub fn check_reschedule() -> bool {
    NEEDS_RESCHEDULE.swap(false, Ordering::SeqCst)
}

/// Set the reschedule flag.
pub fn request_reschedule() {
    NEEDS_RESCHEDULE.store(true, Ordering::SeqCst);
}

pub fn note_process_finished(pid: usize, name: &str) {
    PROCESS_FINISHED.fetch_add(1, Ordering::Relaxed);
    serial_println!("[SCHED] FINISHED process pid={} name='{}'", pid, name);
}

/// Mark the current process finished and release resources shared with the
/// normal exit path. Returns the process kernel stack top for the final idle
/// loop before the timer switches away.
pub fn finish_current_process(code: i32, reason: &str) -> u64 {
    let mut sched = SCHEDULER.lock();
    if sched.current >= sched.processes.len() {
        return 0;
    }

    let cur = sched.current;
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

/// Global scheduler instance.
pub static SCHEDULER: spin::Mutex<Scheduler> = spin::Mutex::new(Scheduler::new());

/// Access the global scheduler.
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
            sched.current = idx;
            sched.processes[idx].state = ProcessState::Running;
            sched.processes[idx].last_run_tick = sched.global_tick;
            sched.start_charging_runtime(idx);
            sched.seed_ready_queues_except(idx);
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
        crate::arch::x86_64::interrupts::set_tss_rsp0(kernel_stack_top);
        context::iretq_to_context(rsp);
    }
}

/// Access the global scheduler and return the current process's context_rsp.
pub fn current_process_rsp() -> u64 {
    let sched = SCHEDULER.lock();
    sched.processes[sched.current].context_rsp
}

pub mod context;
