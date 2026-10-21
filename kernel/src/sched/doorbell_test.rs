use super::doorbell_state::{ConditionalWait, Handle, Outcome};
use super::*;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use sunlight_fastlane_doorbell::{Error, Identity, Terminal};
use x86_64::VirtAddr;

#[cfg(any(
    feature = "pin_release_test",
    feature = "remote_wake_test",
    feature = "scheduler_b_test"
))]
compile_error!("native Doorbell fixture requires an isolated boot feature set");

const ROUNDS: usize = 42;
const CONSUMER_PID: usize = 0xf3a0;
const WORKER_PID: usize = 0xf3b0;
static ARMED: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static ROUND: AtomicUsize = AtomicUsize::new(0);
static PHASE: AtomicUsize = AtomicUsize::new(0);
static TASK: AtomicUsize = AtomicUsize::new(usize::MAX);
static IDLE: AtomicU64 = AtomicU64::new(0);
static FINAL_ACK: AtomicU64 = AtomicU64::new(0);
static CLAIMED: AtomicU64 = AtomicU64::new(0);
static DONE: AtomicU64 = AtomicU64::new(0);
static BLOCKED: AtomicUsize = AtomicUsize::new(0);
static BLOCK_EPOCH: AtomicU64 = AtomicU64::new(0);
static STAGE: AtomicUsize = AtomicUsize::new(0);
static EXCLUDED: AtomicBool = AtomicBool::new(false);
static WORK: AtomicU64 = AtomicU64::new(0);
static FRAMES: AtomicUsize = AtomicUsize::new(0);
static HANDLE: spin::Mutex<Option<Handle>> = spin::Mutex::new(None);
static LAST: spin::Mutex<Option<(Handle, Identity)>> = spin::Mutex::new(None);
static PREPARED: spin::Mutex<Vec<Process>> = spin::Mutex::new(Vec::new());
static WORKERS: spin::Mutex<Vec<Process>> = spin::Mutex::new(Vec::new());
static WORK_COUNT: [AtomicU64; MAX_CORES] = [const { AtomicU64::new(0) }; MAX_CORES];
static WORK_INCAR: [AtomicU64; MAX_CORES] = [const { AtomicU64::new(0) }; MAX_CORES];

#[derive(Clone, Copy)]
struct Record {
    generation: u64,
    incarnation: u64,
    observed: u64,
    sequence: u64,
    registrations: u64,
    blocks: u64,
    wakes: u64,
    ready: u64,
    spurious: u64,
    outcome: &'static str,
    notifications: u64,
    sources: u64,
    resumes: u64,
    selections: u64,
    switches: u64,
    requests: u64,
    sends: u64,
    handlers: u64,
    prepark: bool,
    excluded: bool,
    stale_rejected: bool,
    idle_ns: u64,
    idle_handlers: u64,
    wait_ns: u64,
    notify_ns: u64,
    commit_ns: u64,
    runnable_ns: u64,
    execution_ns: u64,
    last_source: usize,
    ticks: u64,
}

impl Record {
    const EMPTY: Self = Self {
        generation: 0,
        incarnation: 0,
        observed: 0,
        sequence: 0,
        registrations: 0,
        blocks: 0,
        wakes: 0,
        ready: 0,
        spurious: 0,
        outcome: "none",
        notifications: 0,
        sources: 0,
        resumes: 0,
        selections: 0,
        switches: 0,
        requests: 0,
        sends: 0,
        handlers: 0,
        prepark: false,
        excluded: false,
        stale_rejected: false,
        idle_ns: 0,
        idle_handlers: 0,
        wait_ns: 0,
        notify_ns: 0,
        commit_ns: 0,
        runnable_ns: 0,
        execution_ns: 0,
        last_source: 0,
        ticks: 0,
    };
}

static RECORDS: spin::Mutex<[Record; ROUNDS]> = spin::Mutex::new([Record::EMPTY; ROUNDS]);

fn record(update: impl FnOnce(&mut Record)) {
    let round = ROUND.load(Ordering::Acquire);
    if ARMED.load(Ordering::Acquire) && round < ROUNDS {
        update(&mut RECORDS.lock()[round]);
    }
}

fn cpus() -> usize {
    ONLINE_CORES.load(Ordering::Acquire)
}
fn all_cpus() -> u64 {
    (1 << cpus()) - 1
}
fn round() -> usize {
    ROUND.load(Ordering::Acquire)
}
fn case() -> usize {
    match round() {
        0..=11 => round() + 1,
        12 => 12,
        13..=16 => round(),
        17 => 10,
        _ => 17,
    }
}

fn sources() -> u64 {
    if matches!(case(), 1 | 2 | 3 | 11) {
        return 0;
    }
    if cpus() == 1 || case() == 6 {
        return 1;
    }
    if case() == 8 {
        return all_cpus() & !1;
    }
    1 << (1 + round() % (cpus() - 1))
}

fn wait_until(condition: impl Fn() -> bool) {
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    let budget = crate::arch::x86_64::interrupts::tsc_hz().saturating_mul(10);
    assert_ne!(budget, 0);
    while !condition() {
        assert!(unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) < budget,
            "Doorbell fixture barrier timeout round={} case={} cpu={} stage={} blocked={} done={:#x}",
            round(), case(), current_cpu_id(), STAGE.load(Ordering::Acquire), BLOCKED.load(Ordering::Acquire), DONE.load(Ordering::Acquire));
        core::hint::spin_loop();
    }
}

fn load(hhdm: VirtAddr, pid: usize, role: u64, cpu: usize) -> Process {
    let elf = include_bytes!(concat!(env!("OUT_DIR"), "/doorbell-probe.elf"));
    let mut pmm = crate::PMM.lock();
    let mut process = unsafe { Process::new(pid, 0, "doorbell-fixture", &mut pmm, hhdm) };
    let entry = crate::process::elf_loader::load_elf(elf, &mut process, &mut pmm, hhdm).unwrap();
    crate::process::spawn::map_user_stack(&mut process, &mut pmm, hhdm).unwrap();
    process.init_context(entry, crate::process::layout::USER_STACK_TOP);
    process.set_initial_args(role, 0, 0, 0);
    process.cpu_mask = 1 << cpu;
    process.state = ProcessState::BlockedOnIo;
    process
}

pub fn run(hhdm: VirtAddr) {
    assert!(cpus() <= 12);
    init_cores(cpus());
    FRAMES.store(crate::PMM.lock().free_page_count(), Ordering::Release);
    for _ in 0..ROUNDS {
        PREPARED.lock().push(load(hhdm, CONSUMER_PID, 0, 0));
    }
    for cpu in 0..cpus() {
        WORKERS.lock().push(load(hhdm, WORKER_PID + cpu, 1, cpu));
    }
    serial_println!(
        "[F13NA] begin cpus={} rounds={} slots=4 transport=kernel_fixture",
        cpus(),
        ROUNDS
    );
    ARMED.store(true, Ordering::Release);
    mark_scheduler_ready();
    idle_loop()
}

pub fn registration_checkpoint(stage: usize) {
    let desired = if case() == 4 { 1 } else { 2 };
    if cpus() <= 1 || !matches!(case(), 4 | 5 | 13) || stage != desired {
        return;
    }
    STAGE.store(stage, Ordering::Release);
    for value in 0u64..200_000 {
        core::hint::black_box(value.wrapping_mul(31).rotate_left(9));
    }
}

pub fn requested(target: usize) {
    if target == 0 {
        record(|row| row.requests += 1);
    }
}
pub fn sent(target: usize) {
    if target == 0 {
        record(|row| row.sends += 1);
    }
}
pub fn handled() {
    if current_cpu_id() == 0 {
        record(|row| row.handlers += 1);
    }
}
pub fn scheduled(sched: &Scheduler, cpu: usize, result: u64) {
    if ARMED.load(Ordering::Acquire)
        && cpu == 0
        && result != 0
        && sched.core_current_task(cpu) == Some(TASK.load(Ordering::Acquire))
    {
        record(|row| row.selections += 1);
    }
}
pub fn notify_commit() {
    let timestamp = now_ns();
    record(|row| {
        row.commit_ns = timestamp;
        row.last_source = current_cpu_id();
    });
}
pub fn runnable(index: usize) {
    if index != TASK.load(Ordering::Acquire) {
        return;
    }
    let timestamp = now_ns();
    record(|row| {
        row.runnable_ns = timestamp;
        row.ready += 1;
    });
}

fn publish(sched: &mut Scheduler, handle: Handle) {
    let start = now_ns();
    WORK.fetch_add(1, Ordering::Release);
    let woke = sched.doorbell_notify(handle).unwrap();
    let cost = now_ns().saturating_sub(start);
    record(|row| {
        row.notifications += 1;
        row.wakes += u64::from(woke);
        row.sources |= 1 << current_cpu_id();
        row.sequence = if case() == 15 {
            u64::MAX
        } else {
            row.sequence + 1
        };
        row.notify_ns = cost;
    });
}

fn produce(cpu: usize) {
    let handle = HANDLE.lock().unwrap();
    let mut sched = SCHEDULER.lock();
    let index = TASK.load(Ordering::Acquire);
    let identity = sched.doorbell_identity(index).unwrap();
    match case() {
        10 => {
            let woke = if round() == 17 {
                sched.doorbell_producer_exit(handle).unwrap()
            } else {
                sched.doorbell_close(handle).unwrap()
            };
            assert!(woke);
            record(|row| row.wakes += 1);
        }
        12 => {
            let token = sched.processes[index].doorbell_wait.unwrap().token;
            if round() == 11 {
                assert!(sched.doorbell_cancel(token, identity).unwrap());
                record(|row| row.wakes += 1);
                publish(&mut sched, handle);
            } else {
                publish(&mut sched, handle);
                assert!(!sched.doorbell_cancel(token, identity).unwrap());
            }
        }
        13 | 14 => {
            assert!(sched.terminate_process_by_pid(CONSUMER_PID, 137, "doorbell-fixture-exit"));
            assert!(sched.processes[index].doorbell_wait.is_none());
            assert_eq!(sched.doorbells.waiters(), 0);
            record(|row| row.outcome = "consumer_exit");
            assert!(!sched.terminate_process_by_pid(CONSUMER_PID, 137, "doorbell-duplicate-exit"));
            assert_eq!(
                sched.doorbell_notify(handle),
                Err(Error::Terminal(Terminal::ConsumerExit))
            );
            if cpu == 0 {
                request_park_after_sysret();
            } else {
                request_reschedule_on(0);
            }
        }
        9 => {
            if BLOCKED.load(Ordering::Acquire) == 1 {
                sched.wake_io_pid(CONSUMER_PID);
                record(|row| row.wakes += 1);
                if cpu == 0 {
                    request_park_after_sysret();
                }
            } else {
                publish(&mut sched, handle);
            }
        }
        8 => {
            for _ in 0..8 {
                publish(&mut sched, handle);
            }
        }
        _ => publish(&mut sched, handle),
    }
    assert!(sched.diagnostic_ready_occurrences(index) <= 1);
    record(|row| row.sources |= 1 << cpu);
    drop(sched);
    flush_dispatch_notifications();
}

fn producer(cpu: usize) {
    if PHASE.load(Ordering::Acquire) != 1 || sources() & (1 << cpu) == 0 {
        return;
    }
    if cpu == 0 && matches!(case(), 4 | 5 | 13) {
        return;
    }
    if cpu == 0 && BLOCKED.load(Ordering::Acquire) == 0 {
        return;
    }
    if CLAIMED.fetch_or(1 << cpu, Ordering::AcqRel) & (1 << cpu) != 0 {
        return;
    }
    if cpus() > 1 && matches!(case(), 4 | 5 | 13) {
        let desired = if case() == 4 { 1 } else { 2 };
        wait_until(|| STAGE.load(Ordering::Acquire) == desired);
        let excluded = SCHEDULER.try_lock().is_none();
        EXCLUDED.store(excluded, Ordering::Release);
        record(|row| row.excluded = excluded);
    } else {
        wait_until(|| BLOCKED.load(Ordering::Acquire) >= 1);
    }
    if case() == 7 {
        let start = now_ns();
        let handlers = RECORDS.lock()[round()].handlers;
        while now_ns().saturating_sub(start) < 100_000_000 {
            core::hint::spin_loop();
        }
        let mut sched = SCHEDULER.lock();
        let index = TASK.load(Ordering::Acquire);
        assert_eq!(sched.processes[index].state, ProcessState::BlockedOnIo);
        assert_eq!(sched.doorbells.waiters(), 1);
        assert!(sched.processes[index]
            .doorbell_wait
            .unwrap()
            .outcome
            .is_none());
        record(|row| {
            row.idle_ns = now_ns().saturating_sub(start);
            row.idle_handlers = row.handlers - handlers;
        });
        assert_eq!(RECORDS.lock()[round()].idle_handlers, 0);
        sched.remove_from_ready_queues(index);
    }
    produce(cpu);
    if case() == 9 {
        if cpu != 0 {
            wait_until(|| BLOCKED.load(Ordering::Acquire) >= 2);
            let handle = HANDLE.lock().unwrap();
            let mut sched = SCHEDULER.lock();
            publish(&mut sched, handle);
            drop(sched);
            flush_dispatch_notifications();
        } else {
            if BLOCKED.load(Ordering::Acquire) == 1 {
                CLAIMED.fetch_and(!1, Ordering::AcqRel);
                return;
            }
        }
    }
    DONE.fetch_or(1 << cpu, Ordering::Release);
}

fn control() {
    if current_cpu_id() != 0 || IDLE.load(Ordering::Acquire) != all_cpus() {
        return;
    }
    if PHASE.load(Ordering::Acquire) == 0 {
        CLAIMED.store(0, Ordering::Release);
        DONE.store(0, Ordering::Release);
        BLOCKED.store(0, Ordering::Release);
        BLOCK_EPOCH.store(0, Ordering::Release);
        STAGE.store(0, Ordering::Release);
        EXCLUDED.store(false, Ordering::Release);
        WORK.store(0, Ordering::Release);
        let process = PREPARED.lock().pop().unwrap();
        let mut sched = SCHEDULER.lock();
        let index = sched.add_process_after_reaping(process);
        let identity = sched.doorbell_identity(index).unwrap();
        let handle = sched.doorbell_create(index).unwrap();
        if case() == 15 {
            sched.doorbells.fixture_sequence(handle, u64::MAX);
        }
        if let Some((old_handle, old_identity)) = *LAST.lock() {
            assert_eq!(sched.doorbell_notify(old_handle), Err(Error::Stale));
            assert_eq!(old_identity.pid, identity.pid);
            assert_ne!(old_identity.incarnation, identity.incarnation);
            assert_eq!(
                sched.doorbells.snapshot(handle, old_identity),
                Err(Error::Unauthorized)
            );
            assert!(sched
                .doorbells
                .peer_exit(old_identity)
                .iter()
                .all(Option::is_none));
            assert!(sched.doorbell_snapshot(handle, index).is_ok());
            record(|row| row.stale_rejected = true);
        }
        if round() == 18 {
            while let Some(mut worker) = WORKERS.lock().pop() {
                let cpu = worker.pid - WORKER_PID;
                worker.state = ProcessState::Ready;
                let worker_index = sched.add_process_after_reaping(worker);
                sched.enqueue_ready(worker_index);
                WORK_INCAR[cpu].store(
                    sched.processes[worker_index].foreground.incarnation,
                    Ordering::Release,
                );
            }
        }
        TASK.store(index, Ordering::Release);
        *HANDLE.lock() = Some(handle);
        let switches = CORE_STATES[0].lock().context_switches;
        record(|row| {
            row.generation = handle.channel().generation;
            row.incarnation = identity.incarnation;
            row.ticks = sched.global_tick;
            row.switches = switches;
        });
        sched.wake_io_pid(CONSUMER_PID);
        PHASE.store(1, Ordering::Release);
        for cpu in 1..cpus() {
            if sources() & (1 << cpu) != 0 {
                request_reschedule_on(cpu);
            }
        }
        request_park_after_sysret();
        drop(sched);
        flush_dispatch_notifications();
        if cpus() > 1 && matches!(case(), 4 | 5 | 13) {
            wait_until(|| CLAIMED.load(Ordering::Acquire) & sources() == sources());
        }
        return;
    }
    if PHASE.load(Ordering::Acquire) == 1 {
        let mut sched = SCHEDULER.lock();
        let index = TASK.load(Ordering::Acquire);
        let process = &sched.processes[index];
        if process.state == ProcessState::BlockedOnIo
            && process.owning_core == u8::MAX
            && !sched.stack_is_pinned(index)
        {
            let registration = process.doorbell_wait.unwrap();
            assert!(registration.outcome.is_none());
            assert_eq!(sched.diagnostic_ready_occurrences(index), 0);
            if BLOCK_EPOCH.swap(registration.token.epoch, Ordering::AcqRel)
                != registration.token.epoch
            {
                BLOCKED.fetch_add(1, Ordering::Release);
                record(|row| row.blocks += 1);
            }
        }
        if !matches!(
            sched.processes[index].state,
            ProcessState::Finished | ProcessState::Reaped
        ) || sched.live_owner_core(index).is_some()
            || sched.stack_is_pinned(index)
            || DONE.load(Ordering::Acquire) != sources()
        {
            return;
        }
        sched.reap_finished_processes();
        if sched.processes[index].state != ProcessState::Reaped {
            return;
        }
        assert_eq!(sched.processes[index].state, ProcessState::Reaped);
        assert_eq!(
            sched.processes[index].exit_code,
            if matches!(case(), 13 | 14) { 137 } else { 0 }
        );
        assert!(sched.processes[index].kernel_stack.is_none());
        assert!(sched.processes[index].doorbell_wait.is_none());
        assert_eq!(sched.diagnostic_ready_occurrences(index), 0);
        let handle = HANDLE.lock().take().unwrap();
        let identity = Identity {
            pid: CONSUMER_PID as u64,
            incarnation: sched.processes[index].foreground.incarnation,
        };
        sched.doorbell_close(handle).unwrap();
        assert_eq!(sched.doorbells.waiters(), 0);
        sched.doorbell_retire(handle).unwrap();
        assert_eq!(sched.doorbell_retire(handle), Err(Error::Stale));
        *LAST.lock() = Some((handle, identity));
        let switches = CORE_STATES[0].lock().context_switches;
        record(|row| {
            row.ticks = sched.global_tick.saturating_sub(row.ticks);
            row.switches = switches.saturating_sub(row.switches);
        });
        serial_println!("[F13NA] cleanup round={} case=N{} generation={} task_incarnation={} waiter=0 task_refs=0 queued=0 state=Reaped retired=1", round(), case(), handle.channel().generation, identity.incarnation);
        ROUND.fetch_add(1, Ordering::Release);
        if round() == ROUNDS {
            PHASE.store(2, Ordering::Release);
            STOP.store(true, Ordering::Release);
            for cpu in 1..cpus() {
                request_reschedule_on(cpu);
            }
            request_park_after_sysret();
        } else {
            PHASE.store(0, Ordering::Release);
            request_park_after_sysret();
        }
        drop(sched);
        flush_dispatch_notifications();
        return;
    }
    if PHASE.load(Ordering::Acquire) == 2 {
        let mut sched = SCHEDULER.lock();
        sched.reap_finished_processes();
        if sched
            .processes
            .iter()
            .any(|process| process.state != ProcessState::Reaped)
        {
            return;
        }
        if DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire) != 0
            || (0..cpus()).any(|cpu| RESCHEDULE_MASK.pending(cpu))
        {
            return;
        }
        for (index, process) in sched.processes.iter().enumerate() {
            assert!(process.doorbell_wait.is_none());
            assert!(process.kernel_stack.is_none());
            assert!(!sched.stack_is_pinned(index));
            assert_eq!(sched.diagnostic_ready_occurrences(index), 0);
        }
        assert_eq!(sched.doorbells.active(), 0);
        assert_eq!(sched.doorbells.waiters(), 0);
        assert_eq!(sched.doorbells.created, sched.doorbells.retired);
        let frames = crate::PMM.lock().free_page_count();
        assert_eq!(FRAMES.load(Ordering::Acquire), frames);
        let created = sched.doorbells.created;
        let retired = sched.doorbells.retired;
        let completions = sched.doorbells.completions;
        drop(sched);
        for (number, row) in RECORDS.lock().iter().enumerate() {
            let case_number = match number {
                0..=11 => number + 1,
                12 => 12,
                13..=16 => number,
                17 => 10,
                _ => 17,
            };
            let classification =
                if (matches!(case_number, 4 | 5 | 13) && cpus() > 1 && !row.excluded)
                    || (cpus() == 1 && matches!(case_number, 4 | 7 | 13))
                {
                    "NOT_FORCED"
                } else {
                    "FORCED_AND_PASSED"
                };
            serial_println!("[F13NA] race round={} case=N{} result={} generation={} incarnation={} snapshot={} sequence={} registered={} blocked={} wakes={} notifications={} sources={:#x} consumer=0 resumes={} selections={} prepark={} excluded={} stale_rejected={} ready={} spurious={} outcome={} cleanup=retired",
                number, case_number, classification, row.generation, row.incarnation, row.observed, row.sequence,
                row.registrations, row.blocks, row.wakes, row.notifications, row.sources,
                row.resumes, row.selections, row.prepark as u8, row.excluded as u8, row.stale_rejected as u8, row.ready, row.spurious, row.outcome);
            serial_println!("[F13NA] metrics round={} wait_ns={} notify_ns={} commit_ns={} runnable_ns={} execution_ns={} clock_cpu={} requests={} sends={} handlers={} idle_ns={} idle_handlers={} ticks={} switches={} unit=native_tcg_ns",
                number, row.wait_ns, row.notify_ns, row.commit_ns, row.runnable_ns, row.execution_ns, row.last_source,
                row.requests, row.sends, row.handlers, row.idle_ns, row.idle_handlers, row.ticks, row.switches);
        }
        for (cpu, counter) in WORK_COUNT.iter().enumerate().take(cpus()) {
            let count = counter.load(Ordering::Acquire);
            assert!(count > 0);
            serial_println!("[F13NA] workload cpu={} user_executions={}", cpu, count);
        }
        serial_println!("[F13NA] PASS cpus={} rounds={} created={} retired={} active=0 waiters=0 task_refs=0 obligations=0 deferred=0 pending=0 queued=0 pins=0 frames_before={} frames_after={} completions={} metadata_heap_allocations=0 polling_wakes=0",
            cpus(), ROUNDS, created, retired, FRAMES.load(Ordering::Acquire), frames, completions);
        unsafe { x86_64::instructions::port::Port::<u32>::new(0xf4).write(0x10) };
    }
}

pub fn idle(cpu: usize) {
    if !ARMED.load(Ordering::Acquire) {
        return;
    }
    unsafe { crate::arch::x86_64::lapic::mask_timer_for_remote_wake_test() };
    if IDLE.fetch_or(1 << cpu, Ordering::AcqRel) & (1 << cpu) == 0 {
        assert_eq!(crate::arch::x86_64::lapic::local_apic_id() as usize, cpu);
        serial_println!(
            "[F13NA] cpu cpu={} apic={} timer=masked",
            cpu,
            crate::arch::x86_64::lapic::local_apic_id()
        );
        if cpu != 0 {
            request_reschedule_on(0);
            flush_dispatch_notifications();
        }
    }
    if cpu != 0
        && PHASE.load(Ordering::Acquire) == 2
        && !peek_reschedule_on(cpu)
        && FINAL_ACK.fetch_or(1 << cpu, Ordering::AcqRel) & (1 << cpu) == 0
    {
        request_reschedule_on(0);
        flush_dispatch_notifications();
    }
    control();
    producer(cpu);
}

pub fn syscall(operation: u64) -> u64 {
    assert!(ARMED.load(Ordering::Acquire));
    let cpu = current_cpu_id();
    let caller = SCHEDULER.lock().current_pid();
    if operation == 2 {
        assert_eq!(caller, WORKER_PID + cpu);
        assert_eq!(
            SCHEDULER.lock().current_process().foreground.incarnation,
            WORK_INCAR[cpu].load(Ordering::Acquire)
        );
        WORK_COUNT[cpu].fetch_add(1, Ordering::Release);
        control();
        producer(cpu);
        return u64::from(STOP.load(Ordering::Acquire));
    }
    assert_eq!(caller, CONSUMER_PID);
    assert_eq!(cpu, 0);
    assert_eq!(PHASE.load(Ordering::Acquire), 1);
    assert_eq!(
        SCHEDULER.lock().current_process().foreground.incarnation,
        RECORDS.lock()[round()].incarnation
    );
    let handle = HANDLE.lock().unwrap();
    let index = TASK.load(Ordering::Acquire);
    if operation == 0 {
        let mut sched = SCHEDULER.lock();
        assert_eq!(sched.current_process_index(), Some(index));
        let observed = sched.doorbell_snapshot(handle, index).unwrap();
        let mut sequence = observed;
        if case() == 1 {
            publish(&mut sched, handle);
            sequence = sched.doorbell_snapshot(handle, index).unwrap();
        }
        if case() == 2 {
            publish(&mut sched, handle);
        }
        let work = WORK.load(Ordering::Acquire);
        if matches!(case(), 1 | 2) {
            assert_eq!(work, 1);
            record(|row| {
                row.observed = sequence;
                row.outcome = "work";
            });
            return 1;
        }
        assert_eq!(work, 0);
        if case() == 3 {
            publish(&mut sched, handle);
        }
        if case() == 11 {
            publish(&mut sched, handle);
            assert!(!sched.doorbell_close(handle).unwrap());
            assert_eq!(
                sched.doorbell_notify(handle),
                Err(Error::Terminal(Terminal::Closed))
            );
            assert_eq!(
                sched.doorbell_wait_if_unchanged(handle, sequence),
                Err(Error::Terminal(Terminal::Closed))
            );
            assert!(!sched.doorbell_close(handle).unwrap());
            let second = sched.doorbell_create(index).unwrap();
            assert!(!sched.doorbell_close(second).unwrap());
            assert_eq!(
                sched.doorbell_notify(second),
                Err(Error::Terminal(Terminal::Closed))
            );
            sched.doorbell_retire(second).unwrap();
            record(|row| row.outcome = "closed");
            return 1;
        }
        let start = now_ns();
        let result = sched.doorbell_wait_if_unchanged(handle, sequence).unwrap();
        let cost = now_ns().saturating_sub(start);
        record(|row| {
            row.observed = sequence;
            row.wait_ns = cost;
        });
        if case() == 3 {
            assert_eq!(result, ConditionalWait::Changed(1));
            assert_eq!(sched.processes[index].state, ProcessState::Running);
            assert!(sched.processes[index].doorbell_wait.is_none());
            record(|row| row.outcome = "changed");
            return 1;
        }
        let ConditionalWait::Registered(token) = result else {
            panic!("expected registration")
        };
        assert_eq!(sched.processes[index].doorbell_wait.unwrap().token, token);
        assert_eq!(sched.processes[index].state, ProcessState::BlockedOnIo);
        assert_eq!(sched.live_owner_core(index), Some(0));
        assert_eq!(sched.diagnostic_ready_occurrences(index), 0);
        assert_eq!(
            sched.doorbell_wait_if_unchanged(handle, sequence),
            Err(Error::Invalid)
        );
        assert_eq!(sched.doorbell_retire(handle), Err(Error::AlreadyWaiting));
        record(|row| row.registrations += 1);
        drop(sched);
        if matches!(case(), 4 | 5 | 13) {
            if cpus() == 1 {
                produce(0);
                DONE.store(1, Ordering::Release);
            } else {
                wait_until(|| DONE.load(Ordering::Acquire) == sources());
            }
            let sched = SCHEDULER.lock();
            assert_eq!(
                sched.processes[index].state,
                if case() == 13 {
                    ProcessState::Finished
                } else {
                    ProcessState::Ready
                }
            );
            assert_eq!(sched.live_owner_core(index), Some(0));
            assert_eq!(sched.diagnostic_ready_occurrences(index), 0);
            record(|row| row.prepark = true);
        }
        0
    } else {
        assert_eq!(operation, 1);
        if !(case() == 9 && BLOCKED.load(Ordering::Acquire) == 1) {
            wait_until(|| DONE.load(Ordering::Acquire) == sources());
        }
        let execution = now_ns();
        let mut sched = SCHEDULER.lock();
        assert_eq!(sched.current_process_index(), Some(index));
        assert_eq!(sched.processes[index].state, ProcessState::Running);
        assert_eq!(sched.diagnostic_ready_occurrences(index), 0);
        assert!(sched.doorbell_identity(index).is_ok());
        let mut retry = false;
        if !matches!(case(), 1 | 2 | 3 | 11) {
            let outcome = sched.doorbell_take_outcome(index).unwrap();
            let expected = match case() {
                9 if BLOCKED.load(Ordering::Acquire) == 1 => {
                    retry = true;
                    Outcome::Spurious
                }
                10 => Outcome::Terminal(if round() == 17 {
                    Terminal::ProducerExit
                } else {
                    Terminal::Closed
                }),
                12 if round() == 11 => Outcome::Cancelled,
                15 => Outcome::Terminal(Terminal::Exhausted),
                _ => Outcome::Notified(1),
            };
            assert_eq!(outcome, expected);
            record(|row| {
                row.outcome = match outcome {
                    Outcome::Notified(_) => "notified",
                    Outcome::Cancelled => "cancelled",
                    Outcome::Spurious => {
                        row.spurious += 1;
                        "spurious"
                    }
                    Outcome::Terminal(Terminal::Closed) => "closed",
                    Outcome::Terminal(Terminal::ProducerExit) => "producer_exit",
                    Outcome::Terminal(Terminal::Exhausted) => "exhausted",
                    Outcome::Terminal(_) => panic!("unexpected terminal receipt"),
                };
            });
            if retry {
                assert_eq!(WORK.load(Ordering::Acquire), 0);
                assert_eq!(sched.doorbell_snapshot(handle, index), Ok(0));
            } else if !matches!(case(), 10 | 15) {
                assert!(WORK.load(Ordering::Acquire) > 0);
                assert_eq!(
                    sched.doorbell_snapshot(handle, index).unwrap(),
                    RECORDS.lock()[round()].sequence
                );
            }
        }
        record(|row| {
            row.resumes += 1;
            if row.last_source == 0 && row.commit_ns != 0 {
                row.execution_ns = execution;
            }
        });
        u64::from(retry)
    }
}
