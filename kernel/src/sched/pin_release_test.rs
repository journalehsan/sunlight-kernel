use super::*;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use x86_64::VirtAddr;

const ROUNDS: usize = if cfg!(feature = "pin_release_ordering_test") {
    6
} else {
    4
};
static BOUNDARY: AtomicUsize = AtomicUsize::new(0);
static RECHECK_RELEASED: AtomicBool = AtomicBool::new(false);
const FIRST_PID: usize = 0xF200;
static PREPARED: spin::Mutex<alloc::vec::Vec<Process>> = spin::Mutex::new(alloc::vec::Vec::new());
static LAST_GENERATION: AtomicU64 = AtomicU64::new(0);
static ARMED: AtomicBool = AtomicBool::new(false);
static IDLE: AtomicU64 = AtomicU64::new(0);
static ROUND: AtomicUsize = AtomicUsize::new(0);
static PHASE: AtomicUsize = AtomicUsize::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static TASK: AtomicUsize = AtomicUsize::new(usize::MAX);
static REJECTED: AtomicU64 = AtomicU64::new(0);
static ACK: AtomicU64 = AtomicU64::new(0);
static RELEASED: AtomicU64 = AtomicU64::new(0);
static ACCEPTED: AtomicU64 = AtomicU64::new(0);
static HANDLED: AtomicU64 = AtomicU64::new(0);
static PASSES: AtomicU64 = AtomicU64::new(0);
static TERMINAL: AtomicBool = AtomicBool::new(false);
static TERMINAL_ACK: AtomicBool = AtomicBool::new(false);
static EXITED: AtomicBool = AtomicBool::new(false);
static FRAMES: AtomicUsize = AtomicUsize::new(0);
static MASKS: [AtomicU64; MAX_CORES] = [const { AtomicU64::new(0) }; MAX_CORES];
static SENDS: [AtomicUsize; MAX_CORES] = [const { AtomicUsize::new(0) }; MAX_CORES];
static REQUESTS: [AtomicUsize; MAX_CORES] = [const { AtomicUsize::new(0) }; MAX_CORES];

fn ordering_round() -> bool {
    cfg!(feature = "pin_release_ordering_test") && ROUND.load(Ordering::Acquire) >= 4 && cpus() > 1
}

fn execution_cpu() -> usize {
    if ordering_round() {
        1
    } else {
        cpus() - 1
    }
}

#[cfg(feature = "pin_release_ordering_test")]
pub fn ordering_checkpoint(owner: usize, cpu: usize, idx: usize, published: bool) {
    if !active() || !ordering_round() || idx != TASK.load(Ordering::Acquire) {
        return;
    }
    assert_eq!((owner, cpu), (0, 1));
    let after_publication = ROUND.load(Ordering::Acquire) == 5;
    if published != after_publication {
        return;
    }
    assert!(STACK_HANDOFFS.is_pinned(owner, idx));
    if published {
        assert_eq!(STACK_HANDOFFS.waiter_snapshot(owner), 2);
    }
    // CPU0 has dropped S and can pivot/release without taking any lock.
    BOUNDARY.store(if published { 2 } else { 1 }, Ordering::Release);
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    let budget = crate::arch::x86_64::interrupts::tsc_hz().saturating_mul(5);
    assert_ne!(budget, 0);
    while STACK_HANDOFFS.is_pinned(owner, idx) {
        assert!(
            unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) < budget,
            "R2C ordering release timeout"
        );
        core::hint::spin_loop();
    }
}

fn cpus() -> usize {
    ONLINE_CORES.load(Ordering::Acquire)
}

fn remote_mask() -> u64 {
    ((1u64 << cpus()) - 1) & !1
}

fn active() -> bool {
    ARMED.load(Ordering::Acquire)
}

pub fn run(hhdm: VirtAddr) {
    assert!(cpus() <= 12);
    assert!(!cfg!(feature = "remote_wake_test"));
    assert!(!cfg!(feature = "scheduler_b_test"));
    init_cores(cpus());
    FRAMES.store(crate::PMM.lock().free_page_count(), Ordering::Relaxed);
    let elf = include_bytes!(concat!(env!("OUT_DIR"), "/scheduler-probe.elf"));
    for round in 0..ROUNDS {
        let mut pmm = crate::PMM.lock();
        let mut process =
            unsafe { Process::new(FIRST_PID + round, 0, "pin-release-probe", &mut pmm, hhdm) };
        let entry = crate::process::elf_loader::load_elf(elf, &mut process, &mut pmm, hhdm)
            .expect("pin-release ELF");
        crate::process::spawn::map_user_stack(&mut process, &mut pmm, hhdm).unwrap();
        process.init_context(entry, crate::process::layout::USER_STACK_TOP);
        process.set_initial_args(
            if cfg!(feature = "pin_release_ordering_test") && round == 3 {
                4
            } else {
                3
            },
            0,
            0,
            0,
        );
        process.cpu_mask = 1;
        process.state = ProcessState::BlockedOnIo;
        drop(pmm);
        if cfg!(feature = "pin_release_ordering_test") {
            PREPARED.lock().push(process);
        } else {
            SCHEDULER.lock().add_process_after_reaping(process);
        }
    }
    serial_println!("[R2V] begin cpus={} rounds={} source=0", cpus(), ROUNDS);
    ARMED.store(true, Ordering::Release);
    mark_scheduler_ready();
    idle_loop()
}

pub fn idle(cpu: usize) {
    if !active() {
        return;
    }
    if cpu != 0 {
        unsafe { crate::arch::x86_64::lapic::mask_timer_for_remote_wake_test() };
    }
    if IDLE.fetch_or(1 << cpu, Ordering::AcqRel) & (1 << cpu) == 0 {
        let apic = crate::arch::x86_64::lapic::local_apic_id() as usize;
        assert_eq!(apic, cpu);
        serial_println!("[R2V] cpu cpu={} apic={}", cpu, apic);
    }
}

pub fn timer(cpu: usize) {
    if active() && cpu != 0 && PHASE.load(Ordering::Acquire) == 2 {
        panic!("R2V periodic timer assistance on cpu={}", cpu);
    }
}

pub fn on_tick(sched: &mut Scheduler, cpu: usize) {
    if !active() || cpu != 0 || IDLE.load(Ordering::Acquire) & remote_mask() != remote_mask() {
        return;
    }
    if cfg!(feature = "pin_release_ordering_test")
        && TERMINAL.load(Ordering::Acquire)
        && !TERMINAL_ACK.load(Ordering::Acquire)
    {
        audit_terminal(sched);
    }
    if EXITED.load(Ordering::Acquire) {
        for remote in 0..cpus() {
            if STACK_HANDOFFS.slot(remote).load(Ordering::SeqCst) != handoff::EMPTY
                || (remote != 0 && RESCHEDULE_MASK.pending(remote))
            {
                return;
            }
        }
        if DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire) != 0 {
            return;
        }
        sched.reap_finished_processes();
        let idx = TASK.load(Ordering::Acquire);
        assert_eq!(sched.processes[idx].state, ProcessState::Reaped);
        assert!(sched.processes[idx].kernel_stack.is_none());
        assert_eq!(sched.diagnostic_ready_occurrences(idx), 0);
        for remote in 0..cpus() {
            assert_eq!(STACK_HANDOFFS.waiter_snapshot(remote), 0);
        }
        let round = ROUND.load(Ordering::Acquire);
        for remote in 1..if ordering_round() { 1 } else { cpus() } {
            let sends = SENDS[remote].load(Ordering::Acquire);
            let requests = REQUESTS[remote].load(Ordering::Acquire);
            assert!((1..=4).contains(&sends));
            assert!((1..=4).contains(&requests));
            serial_println!("[R2V] waiter round={} cpu={} registered_mask={:#x} rejected=1 ack=1 accepted={} handled={} pass={} sends={} requests={}",
                round, remote, MASKS[remote].load(Ordering::Acquire),
                (ACCEPTED.load(Ordering::Acquire) >> remote) & 1,
                (HANDLED.load(Ordering::Acquire) >> remote) & 1,
                (PASSES.load(Ordering::Acquire) >> remote) & 1, sends, requests);
        }
        if ordering_round() {
            assert!(RECHECK_RELEASED.load(Ordering::Acquire));
            assert_eq!(
                BOUNDARY.load(Ordering::Acquire),
                if round == 4 { 1 } else { 2 }
            );
            serial_println!("[R2C] ordering round={} owner=0 waiter=1 task={} generation={} boundary={} recheck=released execution=1 timer_assistance=0 pins=0 waiters=0 queued=0 cleanup=reaped claimed={:#x}",
                round, idx, GENERATION.load(Ordering::Acquire),
                BOUNDARY.load(Ordering::Acquire), RELEASED.load(Ordering::Acquire));
        }
        serial_println!("[R2V] cycle round={} source=0 execution={} rejected={:#x} released={:#x} timer_assistance=0 pins=0 waiters=0 queued=0 cleanup=reaped",
            round, execution_cpu(), REJECTED.load(Ordering::Acquire), RELEASED.load(Ordering::Acquire));
        ROUND.store(round + 1, Ordering::Release);
        PHASE.store(0, Ordering::Release);
        EXITED.store(false, Ordering::Release);
        if round + 1 == ROUNDS {
            let after = crate::PMM.lock().free_page_count();
            assert_eq!(FRAMES.load(Ordering::Acquire), after);
            serial_println!("[R2V] PASS cpus={} rounds={} forced={} frames_before={} frames_after={} metadata_slots={} retired={} timer_assistance=0",
                cpus(), ROUNDS, (if cfg!(feature = "pin_release_ordering_test") { 4 } else { ROUNDS }) * (cpus() - 1), FRAMES.load(Ordering::Acquire), after,
                sched.processes.len(), ROUNDS);
            unsafe { x86_64::instructions::port::Port::<u32>::new(0xf4).write(0x10) };
            return;
        }
    }
    if PHASE.load(Ordering::Acquire) == 0 {
        TERMINAL.store(false, Ordering::Release);
        TERMINAL_ACK.store(false, Ordering::Release);
        BOUNDARY.store(0, Ordering::Release);
        RECHECK_RELEASED.store(false, Ordering::Release);
        REJECTED.store(0, Ordering::Release);
        ACK.store(0, Ordering::Release);
        RELEASED.store(0, Ordering::Release);
        ACCEPTED.store(0, Ordering::Release);
        HANDLED.store(0, Ordering::Release);
        PASSES.store(0, Ordering::Release);
        for remote in 0..cpus() {
            SENDS[remote].store(0, Ordering::Release);
            REQUESTS[remote].store(0, Ordering::Release);
        }
        let pid = FIRST_PID + ROUND.load(Ordering::Acquire);
        if cfg!(feature = "pin_release_ordering_test") {
            let process = PREPARED.lock().remove(0);
            let idx = sched.add_process_after_reaping(process);
            assert_eq!(idx, 0);
            let generation = sched.processes[idx].address_space.identity().generation;
            let old = LAST_GENERATION.swap(generation, Ordering::AcqRel);
            assert_ne!(generation, old);
            if ROUND.load(Ordering::Acquire) > 0 {
                let old_pid = pid - 1;
                sched.wake_io_pid(old_pid);
                sched.wake_pid(old_pid);
                assert_eq!(sched.processes[idx].state, ProcessState::BlockedOnIo);
                assert_eq!(sched.diagnostic_ready_occurrences(idx), 0);
            }
            serial_println!("[R2C] reuse round={} task={} old_generation={} generation={} old_wake=retired pins=0 waiters=0 dispatch=quiescent",
                ROUND.load(Ordering::Acquire), idx, old, generation);
        }
        TASK.store(
            sched.processes.iter().position(|p| p.pid == pid).unwrap(),
            Ordering::Release,
        );
        GENERATION.store(
            sched.processes[TASK.load(Ordering::Acquire)]
                .address_space
                .identity()
                .generation,
            Ordering::Release,
        );
        PHASE.store(1, Ordering::Release);
        sched.wake_io_pid(pid);
    }
}

pub fn before_schedule(sched: &mut Scheduler, cpu: usize) {
    if !active() || cpu != 0 || PHASE.load(Ordering::Acquire) != 1 {
        return;
    }
    let idx = TASK.load(Ordering::Acquire);
    if sched.core_current_task(cpu) == Some(idx)
        && sched.processes[idx].state == ProcessState::Ready
    {
        PHASE.store(2, Ordering::Release);
        if cpus() > 1 {
            sched.processes[idx].state = ProcessState::BlockedOnIo;
            sched.processes[idx].cpu_mask = remote_mask();
        }
    }
}

pub fn after_schedule(sched: &mut Scheduler, cpu: usize, result: u64) {
    if !active() || PHASE.load(Ordering::Acquire) != 2 {
        return;
    }
    let idx = TASK.load(Ordering::Acquire);
    if cpu == 0 && cpus() > 1 && STACK_HANDOFFS.is_pinned(0, idx) {
        assert_ne!(result, 0);
        assert!(sched.core_current_task(0).is_none());
        assert_eq!(sched.processes[idx].state, ProcessState::BlockedOnIo);
        sched.wake_io_pid(sched.processes[idx].pid);
        sched.enqueue_ready_on_cpu(idx, 1);
        request_reschedule_on(1);
    } else if cpu != 0 {
        if ordering_round()
            && RECHECK_RELEASED.load(Ordering::Acquire)
            && PASSES.load(Ordering::Acquire) & (1 << cpu) == 0
        {
            assert_eq!(cpu, 1);
            assert_ne!(result, 0);
            assert_eq!(sched.core_current_task(cpu), Some(idx));
            PASSES.fetch_or(1 << cpu, Ordering::Release);
            return;
        }
        if REJECTED.load(Ordering::Acquire) & (1 << cpu) != 0
            && ACK.load(Ordering::Acquire) & (1 << cpu) == 0
        {
            assert_eq!(result, 0);
            assert!(sched.core_current_task(cpu).is_none());
            assert!(!RESCHEDULE_MASK.pending(cpu));
            assert_eq!(sched.diagnostic_ready_occurrences(idx), 1);
            if cpu + 1 < cpus() {
                sched.enqueue_ready_on_cpu(idx, cpu + 1);
                request_reschedule_on(cpu + 1);
            } else {
                sched.processes[idx].cpu_mask = 1 << cpu;
            }
        }
        if RELEASED.load(Ordering::Acquire) & (1 << cpu) != 0 {
            PASSES.fetch_or(1 << cpu, Ordering::Release);
        }
    }
}

pub fn registered(owner: usize, cpu: usize, idx: usize, held: bool) {
    if !active() || idx != TASK.load(Ordering::Acquire) || PHASE.load(Ordering::Acquire) != 2 {
        return;
    }
    assert_eq!(owner, 0);
    assert_ne!(cpu, 0);
    if ordering_round() {
        assert!(!held);
        RECHECK_RELEASED.store(true, Ordering::Release);
        return;
    }
    assert!(held);
    let mask = STACK_HANDOFFS.waiter_snapshot(owner);
    assert_ne!(mask & (1 << cpu), 0);
    MASKS[cpu].store(mask, Ordering::Release);
    REJECTED.fetch_or(1 << cpu, Ordering::Release);
}

pub fn handoff_checkpoint(cpu: usize) {
    if !active() || PHASE.load(Ordering::Acquire) != 2 || cpus() == 1 {
        return;
    }
    if cpu != 0 {
        if REJECTED.load(Ordering::Acquire) & (1 << cpu) != 0 {
            ACK.fetch_or(1 << cpu, Ordering::Release);
        }
        return;
    }
    if !STACK_HANDOFFS.is_pinned(0, TASK.load(Ordering::Acquire)) {
        return;
    }
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    let budget = crate::arch::x86_64::interrupts::tsc_hz().saturating_mul(5);
    assert_ne!(budget, 0);
    while if ordering_round() {
        BOUNDARY.load(Ordering::Acquire) == 0
    } else {
        ACK.load(Ordering::Acquire) != remote_mask()
    } {
        assert!(
            unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) < budget,
            "R2V rejection checkpoint timeout"
        );
        core::hint::spin_loop();
    }
    assert_eq!(
        STACK_HANDOFFS.waiter_snapshot(0),
        if ordering_round() {
            if ROUND.load(Ordering::Acquire) == 4 {
                0
            } else {
                2
            }
        } else {
            remote_mask()
        }
    );
}

pub fn released(mask: u64) {
    if active() && current_cpu_id() == 0 && PHASE.load(Ordering::Acquire) == 2 && cpus() > 1 {
        assert_eq!(
            STACK_HANDOFFS.slot(0).load(Ordering::SeqCst),
            handoff::EMPTY
        );
        if ordering_round() {
            assert_eq!(ROUND.load(Ordering::Acquire), 5);
            assert!(mask == 0 || mask == 2);
            RELEASED.store(mask, Ordering::Release);
            return;
        }
        assert_eq!(mask, remote_mask());
        assert_eq!(ACK.load(Ordering::Acquire), mask);
        RELEASED.store(mask, Ordering::Release);
    }
}

pub fn accepted(cpu: usize) {
    if active() && RELEASED.load(Ordering::Acquire) & (1 << cpu) != 0 {
        SENDS[cpu].fetch_add(1, Ordering::Relaxed);
        ACCEPTED.fetch_or(1 << cpu, Ordering::Release);
    }
}

pub fn requested(cpu: usize) {
    if active() && RELEASED.load(Ordering::Acquire) & (1 << cpu) != 0 {
        REQUESTS[cpu].fetch_add(1, Ordering::Relaxed);
    }
}

pub fn handled() {
    let cpu = current_cpu_id();
    if active() && RELEASED.load(Ordering::Acquire) & (1 << cpu) != 0 {
        HANDLED.fetch_or(1 << cpu, Ordering::Release);
    }
}

pub fn exited(code: i32) {
    if !active() {
        return;
    }
    assert_eq!(
        code,
        if cfg!(feature = "pin_release_ordering_test") && ROUND.load(Ordering::Acquire) == 3 {
            132
        } else {
            0
        }
    );
    assert_eq!(PHASE.load(Ordering::Acquire), 2);
    let cpu = current_cpu_id();
    assert_eq!(cpu, execution_cpu());
    assert_eq!(
        SCHEDULER.lock().current_pid(),
        FIRST_PID + ROUND.load(Ordering::Acquire)
    );
    if cpus() > 1 && !ordering_round() {
        let bit = 1 << cpu;
        assert_ne!(RELEASED.load(Ordering::Acquire) & bit, 0);
        assert_ne!(HANDLED.load(Ordering::Acquire) & bit, 0);
        assert_ne!(PASSES.load(Ordering::Acquire) & bit, 0);
    }
    if ordering_round() {
        assert_ne!(PASSES.load(Ordering::Acquire) & (1 << cpu), 0);
    }
    assert!(!EXITED.load(Ordering::Acquire));
}

fn audit_terminal(sched: &mut Scheduler) {
    let idx = TASK.load(Ordering::Acquire);
    let owner = execution_cpu();
    assert_eq!(sched.processes[idx].state, ProcessState::Finished);
    assert!(STACK_HANDOFFS.is_pinned(owner, idx));
    assert!(sched.processes[idx].kernel_stack.is_some());
    assert_eq!(sched.diagnostic_ready_occurrences(idx), 0);
    let identity = sched.processes[idx].address_space.identity();
    let pid = sched.processes[idx].pid;
    sched.wake_io_pid(pid);
    sched.wake_pid(pid);
    sched.reap_finished_processes();
    assert_eq!(sched.processes[idx].state, ProcessState::Finished);
    assert!(sched.processes[idx].kernel_stack.is_some());
    assert_eq!(sched.processes[idx].address_space.identity(), identity);
    assert_eq!(sched.diagnostic_ready_occurrences(idx), 0);
    assert!(!sched.terminate_process_by_pid(pid, 137, "R2C duplicate terminal"));
    serial_println!("[R2C] terminal round={} task={} generation={} owner={} auditor={} pin=held state=Finished stack=retained queued=0 late_wake=retired duplicate_exit=rejected path={}",
        ROUND.load(Ordering::Acquire), idx, identity.generation, owner, current_cpu_id(),
        if ROUND.load(Ordering::Acquire) == 3 { "terminate_current_user_process" } else { "process_exit" });
    TERMINAL_ACK.store(true, Ordering::Release);
}

pub fn finished() {
    if active() {
        if cfg!(feature = "pin_release_ordering_test") {
            TERMINAL.store(true, Ordering::Release);
            if current_cpu_id() == 0 {
                audit_terminal(&mut SCHEDULER.lock());
            } else {
                let start = unsafe { core::arch::x86_64::_rdtsc() };
                let budget = crate::arch::x86_64::interrupts::tsc_hz().saturating_mul(5);
                assert_ne!(budget, 0);
                while !TERMINAL_ACK.load(Ordering::Acquire) {
                    assert!(
                        unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) < budget,
                        "R2C terminal audit timeout"
                    );
                    core::hint::spin_loop();
                }
            }
        }
        assert!(!EXITED.swap(true, Ordering::AcqRel));
    }
}
