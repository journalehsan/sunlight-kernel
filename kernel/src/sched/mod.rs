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
    let factor: i32 = if process.burst_score > BURST_SCORE_LOW { 2 } else { 1 };

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
                process.name,
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
            process.name,
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

    fn remove_from_ready_queues(&mut self, idx: usize) {
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

    /// Called from timer IRQ — may set the reschedule flag.
    pub fn tick(&mut self) {
        self.global_tick += 1;
        self.current_ticks += 1;

        // This is the key line! We get the timeslice based on the current behavior of the process:
        let current = self.current;
        let quantum = quantum_ticks(&self.processes[current]) as u64; // [FEAT-3]

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
                    current_proc.state = ProcessState::Suspended;
                    serial_println!(
                        "[WD] pid={} '{}' exceeded watchdog: ran={} limit={} ticks — suspended",
                        current_proc.pid,
                        current_proc.name,
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
            if ticks_waiting > AGING_THRESHOLD_TICKS
                && !self.processes[idx].aging_boosted_this_pick
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
            serial_println!("[SCHED] process {} '{}' state={:?}", i, p.name, p.state);
            if matches!(p.state, ProcessState::Ready) {
                first = Some(i);
                break;
            }
        }

        if let Some(idx) = first {
            self.current = idx;
            self.processes[idx].state = ProcessState::Running;
            self.processes[idx].last_run_tick = self.global_tick;

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
                self.processes[idx].name,
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

pub fn note_process_finished(pid: usize, name: &'static str) {
    PROCESS_FINISHED.fetch_add(1, Ordering::Relaxed);
    serial_println!("[SCHED] FINISHED process pid={} name='{}'", pid, name);
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
    let (rsp, pml4_phys) = {
        let mut sched = SCHEDULER.lock();
        let mut first = None;
        for (i, p) in sched.processes.iter().enumerate() {
            serial_println!("[SCHED] process {} '{}' state={:?}", i, p.name, p.state);
            if matches!(p.state, ProcessState::Ready) {
                first = Some(i);
                break;
            }
        }

        if let Some(idx) = first {
            sched.current = idx;
            sched.processes[idx].state = ProcessState::Running;
            sched.processes[idx].last_run_tick = sched.global_tick;
            sched.seed_ready_queues_except(idx);
            let rsp = sched.processes[idx].context_rsp;
            let pml4_phys = sched.processes[idx].address_space.pml4_phys;
            serial_println!(
                "[SCHED] Entering process {} '{}' at rsp={:#x}",
                idx,
                sched.processes[idx].name,
                rsp
            );
            (rsp, pml4_phys)
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
        context::iretq_to_context(rsp);
    }
}

/// Access the global scheduler and return the current process's context_rsp.
pub fn current_process_rsp() -> u64 {
    let sched = SCHEDULER.lock();
    sched.processes[sched.current].context_rsp
}

pub mod context;
