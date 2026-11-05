//! Host model uses the production reservation and publication methods. Hardware
//! pin/queue operations are modeled; native gates supply their physical proof.
#![allow(dead_code)]
extern crate alloc;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Barrier, Mutex, MutexGuard,
};
struct Lock<T>(Mutex<T>);
impl<T> Lock<T> {
    fn lock(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap()
    }
}
const MAX_CORES: usize = 64;
static BORROWER_SLOTS_REUSED: AtomicUsize = AtomicUsize::new(0);
static PROCESS_CREATED: AtomicUsize = AtomicUsize::new(0);
#[derive(PartialEq)]
enum SchedulerMode {
    RoundRobin,
    Bore,
}
const SCHEDULER_MODE: SchedulerMode = SchedulerMode::RoundRobin;
#[derive(Debug, PartialEq, Clone, Copy)]
enum ProcessState {
    Ready,
    Finished,
    Reaped,
}
struct CoreState {
    rr_queue: VecDeque<usize>,
    run_queue_high: VecDeque<usize>,
    run_queue_medium: VecDeque<usize>,
    run_queue_low: VecDeque<usize>,
}
impl CoreState {
    const fn new() -> Self {
        Self {
            rr_queue: VecDeque::new(),
            run_queue_high: VecDeque::new(),
            run_queue_medium: VecDeque::new(),
            run_queue_low: VecDeque::new(),
        }
    }
}
static CORE_STATES: [Lock<CoreState>; MAX_CORES] =
    [const { Lock(Mutex::new(CoreState::new())) }; MAX_CORES];
mod memory {
    pub mod heap {
        pub struct Stats {
            pub free: usize,
        }
        std::thread_local! { static FREE: core::cell::Cell<usize> = const { core::cell::Cell::new(usize::MAX) }; }
        pub fn set_free(value: usize) {
            FREE.with(|free| free.set(value));
        }
        pub fn heap_stats() -> Stats {
            Stats {
                free: FREE.with(|free| free.get()),
            }
        }
    }
}
mod process {
    pub const KERNEL_STACK_SIZE: usize = 32 * 1024;
    pub mod fd_table {
        pub struct FdTable;
    }
    // PRODUCTION_HELPERS
    pub struct Capability([u8; 16]);
    pub mod foreground {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        #[derive(Default, Clone, Copy)]
        pub struct Foreground {
            pub incarnation: u64,
            pub parent_generation: u64,
            pub owned: u64,
            pub member: u64,
            pub session_owner: bool,
            pub cancel: u8,
            pub forced_exit: Option<i32>,
        }
        impl Foreground {
            pub fn execution(&self) -> u64 {
                if self.owned != 0 {
                    self.owned
                } else {
                    self.member
                }
            }
            pub fn allocate_incarnation() -> Option<u64> {
                NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
                    .ok()
            }
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Identity {
    root: u64,
    generation: u64,
}
impl Identity {
    fn is_valid(self) -> bool {
        self.root != 0 && self.generation != 0
    }
}
struct AddressSpace(Identity);
impl AddressSpace {
    fn identity(&self) -> Identity {
        self.0
    }
}
struct FdSnapshot;
impl FdSnapshot {
    fn retain_thread_snapshot(&self) {}
}
struct Process {
    pid: usize,
    ppid: usize,
    linux_tgid: Option<usize>,
    native_thread: bool,
    owns_address_space: bool,
    state: ProcessState,
    address_space: AddressSpace,
    foreground: process::foreground::Foreground,
    env: process::env::EnvMap,
    capabilities: Vec<process::Capability>,
    cwd: String,
    service_reap_watcher: Option<u64>,
    owning_core: u8,
    queued_on_core: u8,
    pinned: bool,
    fd_table: FdSnapshot,
}
impl Process {
    fn new(owner: bool, generation: u64) -> Self {
        Self {
            pid: 1,
            ppid: 0,
            linux_tgid: None,
            native_thread: !owner,
            owns_address_space: owner,
            state: ProcessState::Ready,
            address_space: AddressSpace(Identity {
                root: generation * 4096,
                generation,
            }),
            foreground: process::foreground::Foreground::default(),
            env: process::env::EnvMap::new(),
            capabilities: Vec::new(),
            cwd: String::from("/"),
            service_reap_watcher: None,
            owning_core: u8::MAX,
            queued_on_core: u8::MAX,
            pinned: false,
            fd_table: FdSnapshot,
        }
    }
    fn is_exiting(&self) -> bool {
        self.state != ProcessState::Ready
    }
}
struct Scheduler {
    processes: Vec<Process>,
    cores: [CoreState; MAX_CORES],
    online_cores: usize,
}
impl Scheduler {
    fn new(global: usize, vm: usize) -> Self {
        let mut s = Self {
            processes: vec![Process::new(true, 1)],
            cores: [const { CoreState::new() }; MAX_CORES],
            online_cores: 2,
        };
        for group in 0..global.saturating_sub(vm).div_ceil(32) {
            s.processes.push(Process::new(true, 2 + group as u64));
        }
        for n in 0..global {
            s.processes.push(Process::new(
                false,
                if n < vm {
                    1
                } else {
                    2 + ((n - vm) / 32) as u64
                },
            ));
        }
        s
    }
    fn current_process_index(&self) -> Option<usize> {
        Some(0)
    }
    fn current_process(&self) -> &Process {
        &self.processes[0]
    }
    fn stack_is_pinned(&self, idx: usize) -> bool {
        self.processes[idx].pinned
    }
    fn core_current_task(&self, _: usize) -> Option<usize> {
        None
    }
    fn enqueue_ready_on_cpu(&mut self, idx: usize, _: usize) {
        self.processes[idx].queued_on_core = 0;
    }
    fn create(&mut self) -> Result<usize, process::thread_admission::Error> {
        let slot = self.reserve_thread_slot()?;
        let mut child = Process::new(
            false,
            self.current_process().address_space.identity().generation,
        );
        child.address_space = AddressSpace(self.current_process().address_space.identity());
        process::thread_admission::checkpoint("publication")?;
        Ok(self.publish_thread(slot, child, 0))
    }
}
mod production {
    // PRODUCTION_ADMISSION
}
#[test]
fn two_creators_race_for_final_global_and_vm_slots() {
    for (global, vm) in [(95, 0), (31, 31)] {
        let s = Arc::new(Mutex::new(Scheduler::new(global, vm)));
        let start = Arc::new(Barrier::new(3));
        let mut joins = Vec::new();
        for _ in 0..2 {
            let s = s.clone();
            let start = start.clone();
            joins.push(std::thread::spawn(move || {
                start.wait();
                s.lock().unwrap().create().is_ok()
            }));
        }
        start.wait();
        assert_eq!(
            joins
                .into_iter()
                .map(|j| usize::from(j.join().unwrap()))
                .sum::<usize>(),
            1
        );
        let mut s = s.lock().unwrap();
        let idx = s.processes.len() - 1;
        s.processes[idx].state = ProcessState::Finished;
        assert!(s.create().is_err(), "Finished still owns resources");
        s.processes[idx].state = ProcessState::Reaped;
        s.processes[idx].queued_on_core = u8::MAX;
        assert!(
            s.create().is_ok(),
            "physical reap restores exactly one slot"
        );
    }
}
#[test]
fn cancellation_exit_teardown_and_generation_reuse_serialize_with_create() {
    for kind in 0..4 {
        for _ in 0..32 {
            let s = Arc::new(Mutex::new(Scheduler::new(0, 0)));
            let start = Arc::new(Barrier::new(2));
            let peer = s.clone();
            let ready = start.clone();
            let join = std::thread::spawn(move || {
                ready.wait();
                peer.lock().unwrap().create().is_ok()
            });
            start.wait();
            {
                let mut s = s.lock().unwrap();
                match kind {
                    0 => s.processes[0].foreground.cancel = 1,
                    1 => s.processes[0].state = ProcessState::Finished,
                    2 => s.processes[0].owns_address_space = false,
                    _ => {
                        s.processes[0].address_space.0.generation = 2;
                        s.processes[0].state = ProcessState::Finished;
                    }
                }
            }
            let committed = join.join().unwrap();
            assert_eq!(
                s.lock().unwrap().processes.len(),
                if committed { 2 } else { 1 }
            );
            assert!(s.lock().unwrap().create().is_err());
        }
    }
}
#[test]
fn storage_is_bounded_and_pinned_or_watched_slots_are_not_reused() {
    let mut s = Scheduler::new(0, 0);
    for _ in 1..512 {
        let mut p = Process::new(true, 3);
        p.state = ProcessState::Reaped;
        p.pinned = true;
        s.processes.push(p);
    }
    assert!(s.create().is_err());
    s.processes[500].pinned = false;
    s.processes[500].service_reap_watcher = Some(7);
    assert!(s.create().is_err());
    s.processes[500].service_reap_watcher = None;
    assert_eq!(s.create().unwrap(), 500);
    assert_eq!(s.processes.len(), 512);
}
#[test]
fn every_reservation_failure_rolls_back_without_publication() {
    let mut s = Scheduler::new(0, 0);
    for ordinal in 1..=(MAX_CORES
        * if SCHEDULER_MODE == SchedulerMode::RoundRobin {
            1
        } else {
            3
        }
        + 2)
    {
        // task table, all active-mode CPU queues, publication
        process::thread_admission::host_fault::arm(ordinal);
        assert!(s.create().is_err());
        assert_eq!(s.processes.len(), 1);
        process::thread_admission::host_fault::arm(0);
    }
    assert!(s.create().is_ok());
}

#[test]
fn low_heap_preserves_existing_process_and_storage() {
    let mut s = Scheduler::new(0, 0);
    memory::heap::set_free(process::thread_admission::THREAD_HEAP_HEADROOM);
    assert!(matches!(
        s.create(),
        Err(process::thread_admission::Error::NoMemory)
    ));
    assert_eq!(s.processes.len(), 1);
    assert_eq!(s.processes[0].state, ProcessState::Ready);
    memory::heap::set_free(usize::MAX);
    assert!(s.create().is_ok());
}
#[test]
fn exec_eligibility_and_admission_have_one_vm_generation() {
    for _ in 0..32 {
        let s = Arc::new(Mutex::new(Scheduler::new(0, 0)));
        let start = Arc::new(Barrier::new(2));
        let other = s.clone();
        let ready = start.clone();
        let join = std::thread::spawn(move || {
            ready.wait();
            other.lock().unwrap().create().unwrap()
        });
        start.wait();
        let replaced = {
            let mut s = s.lock().unwrap();
            // The predicate is extracted from the actual exec/lifecycle code.
            if s.current_address_space_has_borrowers() {
                false
            } else {
                s.processes[0].address_space.0.generation = 2;
                true
            }
        };
        let idx = join.join().unwrap();
        let s = s.lock().unwrap();
        assert_eq!(
            s.processes[idx].address_space.identity(),
            s.processes[0].address_space.identity()
        );
        assert_eq!(
            s.processes[idx].address_space.0.generation,
            if replaced { 2 } else { 1 }
        );
    }
}

#[test]
fn inherited_state_budgets_reject_before_publication() {
    let mut s = Scheduler::new(0, 0);
    for n in 0..129 {
        s.processes[0].env.set(&format!("K{n}"), "value");
    }
    assert!(matches!(
        s.create(),
        Err(process::thread_admission::Error::Capacity)
    ));
    assert_eq!(s.processes.len(), 1);
    s.processes[0].env = process::env::EnvMap::new();
    s.processes[0].cwd = "x".repeat(process::thread_admission::MAX_INHERITED_BYTES + 1);
    assert!(matches!(
        s.create(),
        Err(process::thread_admission::Error::Capacity)
    ));
    s.processes[0].cwd.pop();
    assert!(s.create().is_ok());
}

#[test]
fn reaping_and_slot_reuse_race_with_concurrent_creation() {
    for _ in 0..32 {
        let s = Arc::new(Mutex::new(Scheduler::new(32, 32)));
        let dead = s.lock().unwrap().processes.len() - 1;
        {
            let mut s = s.lock().unwrap();
            s.processes[dead].state = ProcessState::Finished;
            s.processes[dead].pinned = true;
        }
        let start = Arc::new(Barrier::new(4));
        let mut creators = Vec::new();
        for _ in 0..2 {
            let s = s.clone();
            let start = start.clone();
            creators.push(std::thread::spawn(move || {
                start.wait();
                s.lock().unwrap().create().is_ok()
            }));
        }
        let other = s.clone();
        let ready = start.clone();
        let reaper = std::thread::spawn(move || {
            ready.wait();
            let mut s = other.lock().unwrap();
            s.processes[dead].pinned = false;
            s.processes[dead].state = ProcessState::Reaped;
        });
        start.wait();
        reaper.join().unwrap();
        let admitted = creators
            .into_iter()
            .map(|j| usize::from(j.join().unwrap()))
            .sum::<usize>();
        assert!(admitted <= 1);
        let mut s = s.lock().unwrap();
        if admitted == 0 {
            assert_eq!(s.create().unwrap(), dead);
        }
        assert!(s.create().is_err());
        assert_eq!(s.processes.len(), 33);
    }
}

#[test]
fn recycled_root_does_not_adopt_a_different_vm_generation() {
    let mut s = Scheduler::new(0, 0);
    s.processes[0].owns_address_space = false;
    let mut owner = Process::new(true, 2);
    owner.address_space.0.root = s.processes[0].address_space.0.root;
    s.processes.push(owner);
    assert!(matches!(
        s.create(),
        Err(process::thread_admission::Error::Cancelled)
    ));
    assert_eq!(s.processes.len(), 2);
    s.processes[1].address_space.0.generation = 1;
    assert!(s.create().is_ok());
}

#[test]
fn exited_linux_leader_retains_admission_only_for_its_live_group() {
    let mut s = Scheduler::new(0, 0);
    s.processes[0].owns_address_space = false;
    s.processes[0].linux_tgid = Some(7);
    let mut leader = Process::new(true, 1);
    leader.pid = 7;
    leader.linux_tgid = Some(7);
    leader.state = ProcessState::Finished;
    s.processes.push(leader);
    assert!(s.create().is_ok());
    s.processes[1].linux_tgid = Some(8);
    assert!(matches!(
        s.create(),
        Err(process::thread_admission::Error::Cancelled)
    ));
    s.processes[1].linux_tgid = Some(7);
    s.processes[0].state = ProcessState::Finished;
    assert!(matches!(
        s.create(),
        Err(process::thread_admission::Error::Cancelled)
    ));
}
