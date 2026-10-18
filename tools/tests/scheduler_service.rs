// Host fixture for extracted production code; no alternative dispatcher model.
#![allow(dead_code)]
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
};
struct Lock<T>(Mutex<T>);
impl<T> Lock<T> {
    fn lock(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap()
    }
}
struct Core {
    rr_queue: VecDeque<usize>,
    run_queue_high: VecDeque<usize>,
    run_queue_medium: VecDeque<usize>,
    run_queue_low: VecDeque<usize>,
    current_task: Option<usize>,
    slice: quantum::Slice,
    current_ticks: u64,
    empty_pops: u64,
    stale_pops: u64,
}
impl Core {
    const fn new() -> Self {
        Self {
            rr_queue: VecDeque::new(),
            run_queue_high: VecDeque::new(),
            run_queue_medium: VecDeque::new(),
            run_queue_low: VecDeque::new(),
            current_task: None,
            slice: quantum::Slice::new(),
            current_ticks: 0,
            empty_pops: 0,
            stale_pops: 0,
        }
    }
}
static CORE_STATES: [Lock<Core>; 1] = [Lock(Mutex::new(Core::new()))];
static SCHEDULER_READY: AtomicBool = AtomicBool::new(true);
static DISPATCH_NOTIFICATION_MASK: AtomicU64 = AtomicU64::new(0);
static REQUEST: AtomicBool = AtomicBool::new(false);
static CLOCK: AtomicU64 = AtomicU64::new(1);
fn now_ns() -> u64 {
    CLOCK.load(Ordering::Relaxed)
}
fn current_cpu_id() -> usize {
    0
}
fn check_reschedule_on(_: usize) -> bool {
    REQUEST.swap(false, Ordering::AcqRel)
}
fn request_reschedule_on(_: usize) {
    REQUEST.store(true, Ordering::Release);
}
fn reschedule_bit(cpu: usize) -> u64 {
    1 << cpu
}
const MAX_CORES: usize = 1;
#[derive(PartialEq)]
enum SchedulerMode {
    RoundRobin,
    Bore,
}
const SCHEDULER_MODE: SchedulerMode = SchedulerMode::RoundRobin;
#[derive(PartialEq, Debug, Clone, Copy)]
enum ProcessState {
    BlockedOnIpc,
    BlockedOnIo,
    Ready,
    Running,
    BlockedOnTimer,
    Finished,
    Reaped,
}
enum QueueTier {
    High,
    Medium,
    Low,
}
enum BurstReason {
    EarlyBlock,
    FullQuantum,
    Aged,
}
struct Dummy;
impl Dummy {
    unsafe fn activate(&self) {}
    unsafe fn restore(&self) {}
    unsafe fn save(&self) {}
}
struct Process {
    state: ProcessState,
    owning_core: u8,
    queued_on_core: u8,
    cpu_mask: u64,
    nice: i8,
    burst_score: u32,
    quantum_override: Option<u32>,
    timeslice_used: u32,
    slice_credit: Option<quantum::Credit>,
    last_start_ns: u64,
    cpu_charge_since: Option<u64>,
    cpu_runtime_ns: u64,
    last_run_tick: u64,
    aging_boosted_this_pick: bool,
    counter: i32,
    interactive_bonus: i32,
    aging_counter: u32,
    context_rsp: u64,
    kernel_stack_top: u64,
    fs_base: u64,
    address_space: Dummy,
    fx_state: Dummy,
}
impl Process {
    fn get_queue_tier(&self) -> QueueTier {
        QueueTier::Medium
    }
    fn is_exiting(&self) -> bool {
        matches!(self.state, ProcessState::Finished | ProcessState::Reaped)
    }
}
struct Scheduler {
    processes: Vec<Process>,
    cores: [Core; 1],
    global_tick: u64,
}
impl Scheduler {
    fn core_current_task(&self, c: usize) -> Option<usize> {
        CORE_STATES[c].lock().current_task
    }
    fn set_core_current_task(&self, c: usize, t: Option<usize>) {
        CORE_STATES[c].lock().current_task = t;
    }
    fn set_core_idle_context_rsp(&self, _: usize, _: u64) {}
    fn core_idle_context_rsp(&self, _: usize) -> u64 {
        999
    }
    fn increment_context_switches(&self, _: usize) {}
    fn pin_outgoing_stack(&self, _: usize, _: usize) {}
    fn is_live_on_other_core(&self, i: usize, c: usize) -> bool {
        self.processes[i].owning_core != u8::MAX && self.processes[i].owning_core != c as u8
    }
    fn stack_is_pinned_remotely(&self, _: usize, _: usize) -> bool {
        false
    }
    fn context_is_dispatchable(&self, i: usize) -> bool {
        self.processes[i].context_rsp != 0
    }
    fn steal_work(&self, _: usize) -> Option<usize> {
        None
    }
    fn enqueue_process_once(&mut self, i: usize) {
        self.enqueue_process_to_core(i, 0);
    }
    fn remove_from_ready_queues(&mut self, i: usize) {
        CORE_STATES[0].lock().rr_queue.retain(|&idx| idx != i);
        self.processes[i].queued_on_core = u8::MAX;
    }
    fn pick_next(&mut self, c: usize) -> Option<usize> {
        self.pick_next_round_robin(c)
    }
    fn reap_process_resources(&mut self, i: usize) {
        self.processes[i].state = ProcessState::Reaped;
    }
    fn reap_finished_processes(&self) {}
    // PRODUCTION_METHODS
}
mod arch {
    pub mod x86_64 {
        pub mod interrupts {
            pub fn tsc_hz() -> u64 {
                1_000_000_000
            }
        }
        pub mod smp {
            pub fn set_current_cpu_tss_rsp0(_: u64) {}
        }
    }
}
mod x86_64 {
    pub mod registers {
        pub mod model_specific {
            pub struct Msr;
            impl Msr {
                pub fn new(_: u64) -> Self {
                    Self
                }
                pub unsafe fn write(&self, _: u64) {}
                pub unsafe fn read(&self) -> u64 {
                    0
                }
            }
        }
    }
}
mod memory {
    pub mod tlb {
        pub unsafe fn activate_kernel_root() {}
    }
}
mod process {
    pub const KERNEL_STACK_SIZE: usize = 128;
}
mod sunlight_ipc {
    pub(crate) use crate::cpu_accounting;
}
static mut CORE_IDLE_STACKS: [[u8; 128]; 1] = [[0; 128]];
fn build_kernel_idle_frame(_: u64) -> u64 {
    999
}
macro_rules! serial_println { ($($a:tt)*) => { let _ = format_args!($($a)*); }; }
use serial_println;

fn setup(workers: usize) -> Scheduler {
    *CORE_STATES[0].lock() = Core::new();
    CLOCK.store(1, Ordering::Relaxed);
    REQUEST.store(false, Ordering::Relaxed);
    let mut s = Scheduler {
        processes: (0..workers)
            .map(|i| Process {
                state: if i == 0 {
                    ProcessState::Running
                } else {
                    ProcessState::Ready
                },
                owning_core: if i == 0 { 0 } else { u8::MAX },
                queued_on_core: u8::MAX,
                cpu_mask: 1,
                nice: 0,
                burst_score: 304,
                quantum_override: None,
                timeslice_used: 0,
                slice_credit: None,
                last_start_ns: 0,
                cpu_charge_since: None,
                cpu_runtime_ns: 0,
                last_run_tick: 0,
                aging_boosted_this_pick: false,
                counter: 0,
                interactive_bonus: 0,
                aging_counter: 0,
                context_rsp: (i + 1) as u64 * 1000,
                kernel_stack_top: 0,
                fs_base: 0,
                address_space: Dummy,
                fx_state: Dummy,
            })
            .collect(),
        cores: [Core::new()],
        global_tick: 0,
    };
    s.set_core_current_task(0, Some(0));
    s.start_charging_runtime(0);
    for i in 1..workers {
        s.queue_process_to_core(i, 0, false);
    }
    s
}
fn dispatch(s: &mut Scheduler, elapsed_ns: u64) -> u64 {
    CLOCK.fetch_add(elapsed_ns, Ordering::Relaxed);
    s.global_tick = now_ns() / quantum::NS_PER_TICK;
    request_reschedule_on(0);
    s.schedule_tick(0, (s.core_current_task(0).unwrap_or(0) + 1) as u64 * 1000)
}
fn queue() -> Vec<usize> {
    CORE_STATES[0].lock().rr_queue.iter().copied().collect()
}
fn runtime(s: &Scheduler, i: usize) -> u64 {
    cpu_accounting::runtime_at(
        s.processes[i].cpu_runtime_ns,
        s.processes[i].cpu_charge_since,
        now_ns(),
    )
}
#[test]
fn unrelated_wakes_keep_turn_and_expiration_rotates_once() {
    let mut s = setup(3);
    for _ in 0..17 {
        assert_eq!(dispatch(&mut s, 10_000_000), 0);
        assert_eq!(s.core_current_task(0), Some(0));
        assert_eq!(queue(), [1, 2]);
        assert_eq!(s.processes[0].burst_score, 304);
    }
    assert_eq!(dispatch(&mut s, 10_000_000), 2000);
    assert_eq!(queue(), [2, 0]);
    assert_eq!(s.processes[0].burst_score, 336);
    assert_eq!(runtime(&s, 0), 180_000_000);
    assert_eq!(s.schedule_tick(0, 2000), 0); // no rearming from rotation
    assert!(s.processes[0].slice_credit.is_none());
}
#[test]
fn priority_interrupt_resumes_fractional_budget_and_original_position() {
    let mut s = setup(3);
    s.processes[2].nice = -1;
    assert_eq!(dispatch(&mut s, 1_500_000), 3000);
    assert_eq!(queue(), [0, 1]);
    assert!(s.processes[0].slice_credit.is_some());
    assert_eq!(s.processes[0].burst_score, 304); // no forced short-burst penalty
    s.processes[2].state = ProcessState::BlockedOnTimer;
    assert_eq!(dispatch(&mut s, 20_000_000), 1000);
    assert!(s.processes[0].slice_credit.is_none()); // taken, never copied
    assert_eq!(dispatch(&mut s, 178_000_000), 0);
    assert_eq!(dispatch(&mut s, 500_000), 2000);
    assert_eq!(runtime(&s, 0), 180_000_000);
    assert_eq!(runtime(&s, 2), 20_000_000);
    assert_eq!(s.processes[0].burst_score, 336); // full-turn feedback exactly once
    assert_eq!(queue(), [0]);
}

#[test]
fn higher_priority_wake_wins_when_worker_slice_expires_same_tick() {
    let mut s = setup(3);
    s.processes[2].nice = -5;
    // Queue order is [worker 1, timer service].  At the exact 180 ms
    // expiration, the service must not lose its wake to ordinary FIFO.
    assert_eq!(dispatch(&mut s, 180_000_000), 3000);
    assert_eq!(s.core_current_task(0), Some(2));
    assert_eq!(s.processes[0].burst_score, 336);
}

#[test]
fn yield_sleep_wake_termination_discard_credit_and_reject_duplicate_enqueue() {
    let mut s = setup(3);
    s.processes[2].nice = -1;
    dispatch(&mut s, 1_000_000);
    s.processes[2].state = ProcessState::BlockedOnTimer;
    dispatch(&mut s, 10_000_000);
    CLOCK.fetch_add(1_000_000, Ordering::Relaxed);
    s.account_and_apply_churn_penalty(); // actual syscall yield checkpoint
    s.processes[0].state = ProcessState::Ready;
    assert_eq!(dispatch(&mut s, 0), 2000);
    assert_eq!(s.processes[0].burst_score, 352); // voluntary churn still applies
    assert!(s.processes[0].slice_credit.is_none());
    assert_eq!(runtime(&s, 0), 2_000_000); // checkpoint + close do not double charge
    s.processes[1].state = ProcessState::BlockedOnTimer;
    assert_eq!(dispatch(&mut s, 1_000_000), 1000);
    s.processes[1].state = ProcessState::Ready;
    s.enqueue_process_to_core(1, 0);
    s.enqueue_process_to_core(1, 0);
    assert_eq!(queue(), [1]);
    assert_eq!(dispatch(&mut s, 1_000_000), 0); // equal-priority wake preserves turn
    s.processes[0].state = ProcessState::Finished;
    assert_eq!(dispatch(&mut s, 1_000_000), 2000);
    assert_eq!(s.processes[0].state, ProcessState::Reaped);
    assert!(s.processes[0].slice_credit.is_none());
    assert!(s.processes[0].cpu_charge_since.is_none());
    assert_eq!(runtime(&s, 0), 4_000_000);
    assert!(queue().is_empty());
}
#[test]
fn frequent_background_wakes_produce_equal_service_and_stable_bursts() {
    let mut s = setup(5);
    // Background service blocks on each dispatch, becomes Ready each next tick.
    for _ in 0..10000 {
        let current = s.core_current_task(0).unwrap();
        if current == 4 {
            s.processes[4].state = ProcessState::BlockedOnTimer;
        } else if s.processes[4].state == ProcessState::BlockedOnTimer {
            s.processes[4].state = ProcessState::Ready;
            s.enqueue_process_to_core(4, 0);
        }
        dispatch(&mut s, 10_000_000);
        assert_eq!(
            queue().len(),
            queue()
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
        );
    }
    let times: Vec<f64> = (0..4).map(|i| runtime(&s, i) as f64).collect();
    let jain = times.iter().sum::<f64>().powi(2) / (4.0 * times.iter().map(|x| x * x).sum::<f64>());
    assert!(jain > 0.999, "service Jain={jain}, times={times:?}");
    let scores: Vec<_> = (0..4).map(|i| s.processes[i].burst_score).collect();
    assert!(
        scores.iter().all(|&score| (1004..=1024).contains(&score)),
        "{scores:?}"
    ); // aging remains active
}

#[test]
fn high_priority_timer_service_wake_is_bounded_without_losing_worker_credit() {
    let mut s = setup(5);
    s.processes[4].nice = -5; // sunlight-niced High service policy
    for _ in 0..1_000 {
        if s.core_current_task(0) == Some(4) {
            s.processes[4].state = ProcessState::BlockedOnTimer;
            dispatch(&mut s, 10_000_000);
            continue;
        }
        if s.processes[4].state == ProcessState::BlockedOnTimer {
            s.processes[4].state = ProcessState::Ready;
            s.enqueue_process_to_core(4, 0);
            dispatch(&mut s, 10_000_000);
            let mut waited = 10_000_000;
            while s.core_current_task(0) != Some(4) {
                assert!(waited <= 200_000_000, "timer service wait={waited} ns");
                dispatch(&mut s, 10_000_000);
                waited += 10_000_000;
            }
            continue;
        }
        dispatch(&mut s, 10_000_000);
    }
    for i in 0..4 {
        assert!(runtime(&s, i) > 0, "worker {i} made no progress");
    }
}

#[test]
fn low_nice_workers_progress_under_higher_priority_background_wakes() {
    let mut s = setup(4);
    for i in 0..3 {
        s.processes[i].nice = 8 + i as i8;
    }
    for _ in 0..10000 {
        let current = s.core_current_task(0);
        if current == Some(3) {
            s.processes[3].state = ProcessState::BlockedOnTimer;
        } else if s.processes[3].state == ProcessState::BlockedOnTimer {
            s.processes[3].state = ProcessState::Ready;
            s.enqueue_process_to_core(3, 0);
        }
        dispatch(&mut s, 10_000_000);
    }
    for i in 0..3 {
        assert!(
            runtime(&s, i) > 0,
            "nice {} runtime {}",
            s.processes[i].nice,
            runtime(&s, i)
        );
    }
}

#[test]
fn priority_candidates_must_be_eligible_and_burst_is_not_wake_priority() {
    let mut s = setup(4);
    s.processes[1].burst_score = 0;
    assert!(s.priority_preemption_candidate(0, 0).is_none());
    s.processes[1].nice = -1;
    s.processes[1].cpu_mask = 0;
    s.processes[2].nice = -1;
    s.processes[2].context_rsp = 0;
    s.processes[3].nice = -1;
    s.processes[3].state = ProcessState::Finished;
    assert!(s.priority_preemption_candidate(0, 0).is_none());
    assert_eq!(dispatch(&mut s, 1_000_000), 0);
    s.processes[1].cpu_mask = 1;
    assert_eq!(dispatch(&mut s, 1_000_000), 2000);
    assert_eq!(queue(), [0, 2, 3]);
}

#[test]
fn residual_turn_bypasses_readmission_debt_and_retires_override_once() {
    let mut s = setup(2);
    s.stop_execution_clock(0, false);
    s.processes[0].nice = 10;
    s.processes[0].counter = MAX_DEBT;
    s.processes[0].quantum_override = Some(25);
    s.start_charging_runtime(0);
    assert_eq!(dispatch(&mut s, 1_000_000), 2000);
    assert_eq!(s.processes[0].counter, MAX_DEBT);
    assert_eq!(s.processes[0].quantum_override, Some(25));
    s.processes[1].state = ProcessState::BlockedOnTimer;
    assert_eq!(dispatch(&mut s, 10_000_000), 1000);
    assert_eq!(s.processes[0].counter, MAX_DEBT);
    assert_eq!(s.processes[0].quantum_override, Some(25));
    assert_eq!(dispatch(&mut s, 248_000_000), 0);
    dispatch(&mut s, 1_000_000);
    assert_eq!(s.processes[0].quantum_override, None);
    assert_eq!(s.processes[0].burst_score, 336);
    assert_eq!(runtime(&s, 0), 250_000_000);
    assert!(s.processes[0].slice_credit.is_none());
}

#[test]
fn direct_io_block_keeps_voluntary_feedback_without_double_checkpoint() {
    for checkpointed in [false, true] {
        let mut s = setup(2);
        CLOCK.fetch_add(1_000_000, Ordering::Relaxed);
        if checkpointed {
            s.account_and_apply_churn_penalty();
        }
        s.processes[0].state = ProcessState::BlockedOnIo;
        assert_eq!(dispatch(&mut s, 0), 2000);
        assert_eq!(s.processes[0].burst_score, 352);
        assert_eq!(runtime(&s, 0), 1_000_000);
        assert!(s.processes[0].slice_credit.is_none());
    }
}
