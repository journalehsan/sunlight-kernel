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
                .saturating_sub(BURST_REDUCTION_EARLY_BLOCK)
                .max(BURST_SCORE_MIN);
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

/// === Diagnostic counters for process leak detection ===
static PROCESS_CREATED: AtomicUsize = AtomicUsize::new(0);
static PROCESS_FINISHED: AtomicUsize = AtomicUsize::new(0);

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

    /// Called from timer IRQ — may set the reschedule flag.
    pub fn tick(&mut self) {
        self.global_tick += 1;
        self.current_ticks += 1;

        // Check if current process has finished and reap it
        if self.current < self.processes.len() {
            if self.processes[self.current].state == ProcessState::Finished {
                let pid = self.processes[self.current].pid;
                let name = self.processes[self.current].name;
                PROCESS_FINISHED.fetch_add(1, Ordering::Relaxed);
                serial_println!("[SCHED] FINISHED process pid={} name='{}' still in vector (LEAK!)", pid, name);
            }
        }

        if self.current_ticks >= TIME_SLICE_TICKS {
            // Process used full quantum
            let current_proc = &mut self.processes[self.current];
            current_proc.timeslice_used = self.current_ticks as u32;

            // Update burst score for full quantum usage
            update_burst_score(current_proc, BurstReason::FullQuantum);

            // Age processes that haven't run recently
            self.age_ready_tasks();

            // Request reschedule
            self.current_ticks = 0;
            NEEDS_RESCHEDULE.store(true, Ordering::SeqCst);
        }

        // Every 1000 ticks, report process diagnostics
        if self.global_tick % 1000 == 0 {
            self.diagnostic_report();
        }
    }

    fn age_ready_tasks(&mut self) {
        if self.global_tick % AGING_INTERVAL_TICKS != 0 {
            return; // Only age every AGING_INTERVAL_TICKS
        }

        for idx in 0..self.processes.len() {
            let p = &mut self.processes[idx];

            // Only age Ready (not Running/BlockedOnIpc) processes
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
        let mut skipped_current = None;

        if let Some(idx) = pop_ready_excluding_current(
            &mut self.ready_queue_high,
            &self.processes,
            self.current,
            &mut skipped_current,
        ) {
            if let Some(current) = skipped_current {
                self.enqueue_process_once(current);
            }
            return Some(idx);
        }

        if let Some(idx) = pop_ready_excluding_current(
            &mut self.ready_queue_medium,
            &self.processes,
            self.current,
            &mut skipped_current,
        ) {
            if let Some(current) = skipped_current {
                self.enqueue_process_once(current);
            }
            return Some(idx);
        }

        if let Some(idx) = pop_ready_excluding_current(
            &mut self.ready_queue_low,
            &self.processes,
            self.current,
            &mut skipped_current,
        ) {
            if let Some(current) = skipped_current {
                self.enqueue_process_once(current);
            }
            return Some(idx);
        }

        if let Some(current) = skipped_current {
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

    /// Pick the next Ready process using the original stable round-robin scan.
    pub fn pick_next_round_robin(&self) -> Option<usize> {
        let len = self.processes.len();
        if len == 0 {
            return None;
        }

        let start = (self.current + 1) % len;
        let mut idx = start;
        loop {
            if matches!(self.processes[idx].state, ProcessState::Ready) {
                return Some(idx);
            }
            idx = (idx + 1) % len;
            if idx == start {
                break;
            }
        }

        None
    }

    pub fn pick_next(&mut self) -> Option<usize> {
        match SCHEDULER_MODE {
            SchedulerMode::RoundRobin => self.pick_next_round_robin(),
            SchedulerMode::Bore => self.pick_next_bore(),
        }
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

            // Update state and enqueue
            self.processes[idx].state = ProcessState::Ready;
            self.enqueue_process_once(idx);
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
        let alive = self.processes.len();
        let ready_high = self.ready_queue_high.len();
        let ready_mid = self.ready_queue_medium.len();
        let ready_low = self.ready_queue_low.len();

        serial_println!(
            "[SCHED-DIAG] created={} finished={} alive={} ready_queues=({},{},{}) delta_created-finished={}",
            created, finished, alive, ready_high, ready_mid, ready_low,
            created.saturating_sub(finished)
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
