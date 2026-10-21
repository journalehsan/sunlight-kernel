//! Debug-only scheduler fixture. Existing ring-3 probe and exit/yield ABI only.
use super::*;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use x86_64::VirtAddr;

static ARMED: AtomicBool = AtomicBool::new(false);
static TARGET_IDLE: AtomicBool = AtomicBool::new(false);
// 4 * 11 address spaces fits the existing 64-entry lifetime ledger on 12 CPUs.
const ROUNDS: usize = 4;
static ROUND: AtomicUsize = AtomicUsize::new(0);
static PUBLISHED_COUNT: [AtomicUsize; MAX_CORES] = [const { AtomicUsize::new(0) }; MAX_CORES];
static EXECUTION_COUNT: AtomicUsize = AtomicUsize::new(0);
static FRAME_BASELINE: AtomicUsize = AtomicUsize::new(0);
static FINAL_SIGNAL: AtomicBool = AtomicBool::new(false);
static PUBLISHED: AtomicU64 = AtomicU64::new(0);
static EXECUTED: AtomicU64 = AtomicU64::new(0);
static HANDLED: AtomicUsize = AtomicUsize::new(0);
static OPPORTUNITIES: AtomicUsize = AtomicUsize::new(0);
static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static ACCEPTED: AtomicUsize = AtomicUsize::new(0);
static INJECTED: AtomicUsize = AtomicUsize::new(0);
static HARDWARE_BUSY: AtomicUsize = AtomicUsize::new(0);
static BUSY: [AtomicUsize; MAX_CORES] = [const { AtomicUsize::new(0) }; MAX_CORES];
const FIRST_PID: usize = 0xF100;

fn checked_apic_base(cpu: usize) -> u64 {
    let base = unsafe { x86_64::registers::model_specific::Msr::new(0x1b).read() };
    assert_ne!(base & (1 << 11), 0, "LAPIC disabled");
    assert_eq!(base & (1 << 10), 0, "fixture requires actual xAPIC mode");
    assert_eq!(crate::arch::x86_64::lapic::local_apic_id() as usize, cpu);
    base
}

fn target() -> usize {
    usize::from(ONLINE_CORES.load(Ordering::Acquire) > 1)
}
fn sources() -> u64 {
    let count = ONLINE_CORES.load(Ordering::Acquire);
    if count == 1 {
        1
    } else {
        ((1u64 << count) - 1) & !2
    }
}

pub fn run(hhdm: VirtAddr) {
    let count = ONLINE_CORES.load(Ordering::Acquire);
    init_cores(count);
    let elf = include_bytes!(concat!(env!("OUT_DIR"), "/scheduler-probe.elf"));
    FRAME_BASELINE.store(crate::PMM.lock().free_page_count(), Ordering::Relaxed);
    for round in 0..ROUNDS {
        for source in 0..count {
            if sources() & (1 << source) == 0 {
                continue;
            }
            let mut pmm = crate::PMM.lock();
            let mut p = unsafe {
                Process::new(
                    FIRST_PID + round * MAX_CORES + source,
                    0,
                    "remote-wake-probe",
                    &mut pmm,
                    hhdm,
                )
            };
            let entry = crate::process::elf_loader::load_elf(elf, &mut p, &mut pmm, hhdm).unwrap();
            crate::process::spawn::map_user_stack(&mut p, &mut pmm, hhdm).unwrap();
            p.init_context(entry, crate::process::layout::USER_STACK_TOP);
            p.set_initial_args(2, 0, 0, 0);
            p.cpu_mask = 1 << target();
            p.state = ProcessState::BlockedOnIo;
            drop(pmm);
            SCHEDULER.lock().add_process_after_reaping(p);
            BUSY[source].store(3, Ordering::Relaxed);
        }
    }
    serial_println!(
        "[F13NR] begin cpus={} target={} mode=xapic apic_base={:#x} injected_busy=3",
        count,
        target(),
        checked_apic_base(current_cpu_id())
    );
    ARMED.store(true, Ordering::Release);
    mark_scheduler_ready();
    flush_dispatch_notifications();
    idle_loop()
}

/// Runs with IF clear immediately before target STI;HLT. The source can now
/// race precisely this idle entry. No target periodic timer can repair a wake.
pub fn idle(cpu: usize) {
    if !ARMED.load(Ordering::Acquire) || cpu != target() {
        return;
    }
    if cpu != 0 {
        unsafe {
            crate::arch::x86_64::lapic::mask_timer_for_remote_wake_test();
        }
    }
    if !TARGET_IDLE.swap(true, Ordering::AcqRel) {
        serial_println!(
            "[F13NR] idle cpu={} apic_base={:#x}",
            cpu,
            checked_apic_base(cpu)
        );
    }
    let completed = EXECUTION_COUNT.load(Ordering::Acquire);
    let per_round = sources().count_ones() as usize;
    let round = ROUND.load(Ordering::Acquire);
    if completed == ROUNDS * per_round {
        // The final exit's self-IPI must first consume its scheduling request.
        if !RESCHEDULE_MASK.pending(cpu) && !FINAL_SIGNAL.swap(true, Ordering::AcqRel) {
            request_reschedule_on(0);
            if cpu == 0 {
                DISPATCH_NOTIFICATION_MASK.fetch_or(1, Ordering::Release);
            }
            flush_dispatch_notifications();
        }
    } else if completed == (round + 1) * per_round {
        ROUND.store(round + 1, Ordering::Release);
        for source in 0..ONLINE_CORES.load(Ordering::Acquire) {
            if sources() & (1 << source) != 0 {
                request_reschedule_on(source);
                if source == cpu {
                    DISPATCH_NOTIFICATION_MASK.fetch_or(1 << source, Ordering::Release);
                }
            }
        }
        flush_dispatch_notifications();
    }
}

pub fn on_tick(s: &mut Scheduler, cpu: usize) {
    if !ARMED.load(Ordering::Acquire)
        || !TARGET_IDLE.load(Ordering::Acquire)
        || sources() & (1 << cpu) == 0
    {
        return;
    }
    if FINAL_SIGNAL.load(Ordering::Acquire) && cpu == 0 {
        // Explicit consumption of a fixture control request, no task effect.
        RESCHEDULE_MASK.take(cpu);
        s.reap_finished_processes();
        for (idx, p) in s.processes.iter().enumerate() {
            assert_eq!(p.state, ProcessState::Reaped);
            assert!(p.kernel_stack.is_none());
            assert_eq!(s.diagnostic_ready_occurrences(idx), 0);
            assert!(!s.stack_is_pinned(idx));
        }
        assert_eq!(DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire), 0);
        for cpu in 0..ONLINE_CORES.load(Ordering::Acquire) {
            assert!(!RESCHEDULE_MASK.pending(cpu));
        }
        serial_println!("[F13NR] PASS cpus={} sources={:#x} target={} requests={} attempts={} accepted={} handled={} opportunities={} executed={} deferred=0 unresolved=0 hardware_busy={} injected_busy={} rounds={} cleanup=reaped frames_before={} frames_after={}",
            ONLINE_CORES.load(Ordering::Acquire), sources(), target(), ROUNDS * sources().count_ones() as usize,
            ATTEMPTS.load(Ordering::Relaxed), ACCEPTED.load(Ordering::Relaxed), HANDLED.load(Ordering::Relaxed),
            OPPORTUNITIES.load(Ordering::Relaxed), EXECUTION_COUNT.load(Ordering::Acquire),
            HARDWARE_BUSY.load(Ordering::Relaxed), INJECTED.load(Ordering::Relaxed), ROUNDS,
            FRAME_BASELINE.load(Ordering::Relaxed), crate::PMM.lock().free_page_count());
        unsafe {
            x86_64::instructions::port::Port::<u32>::new(0xf4).write(0x10);
        }
        return;
    }
    let round = ROUND.load(Ordering::Acquire);
    if PUBLISHED_COUNT[cpu].load(Ordering::Acquire) != round {
        return;
    }
    let apic_base = checked_apic_base(cpu);
    assert!(s.core_current_task(cpu).is_none());
    s.reap_finished_processes();
    BUSY[cpu].store(3, Ordering::Relaxed);
    s.wake_io_pid(FIRST_PID + round * MAX_CORES + cpu);
    PUBLISHED_COUNT[cpu].fetch_add(1, Ordering::Release);
    PUBLISHED.fetch_or(1 << cpu, Ordering::Release);
    unsafe {
        crate::arch::x86_64::lapic::mask_timer_for_remote_wake_test();
    }
    serial_println!(
        "[F13NR] publish source={} target={} round={} idle_published=true apic_base={:#x}",
        cpu,
        target(),
        round,
        apic_base
    );
}

pub fn attempt(cpu: usize) -> bool {
    if !ARMED.load(Ordering::Acquire) || cpu != target() {
        return false;
    }
    ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    let source = current_cpu_id();
    if BUSY[source]
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_sub(1))
        .is_ok()
    {
        INJECTED.fetch_add(1, Ordering::Relaxed);
        true
    } else {
        false
    }
}
pub fn accepted(cpu: usize) {
    if ARMED.load(Ordering::Acquire) && cpu == target() {
        ACCEPTED.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn hardware_busy(cpu: usize) {
    if ARMED.load(Ordering::Acquire) && cpu == target() {
        HARDWARE_BUSY.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn handled(cpu: usize) {
    if ARMED.load(Ordering::Acquire) && cpu == target() {
        HANDLED.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn opportunity(cpu: usize) {
    if ARMED.load(Ordering::Acquire) && cpu == target() {
        OPPORTUNITIES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Existing ProcessExit proves ring-3 execution on the physical target CPU.
/// Reporting occurs outside measured paths. This does not claim post-pivot reap.
pub fn exited(pid: usize, code: i32) {
    if !ARMED.load(Ordering::Acquire) {
        return;
    }
    let identity = pid.checked_sub(FIRST_PID).expect("fixture pid");
    let source = identity % MAX_CORES;
    let round = identity / MAX_CORES;
    assert_eq!(round, ROUND.load(Ordering::Acquire));
    assert!(source < ONLINE_CORES.load(Ordering::Acquire));
    assert_eq!(code, 0);
    assert_eq!(current_cpu_id(), target());
    if EXECUTION_COUNT.load(Ordering::Acquire) % sources().count_ones() as usize == 0 {
        EXECUTED.store(0, Ordering::Release);
    }
    assert_eq!(
        EXECUTED.fetch_or(1 << source, Ordering::AcqRel) & (1 << source),
        0
    );
    EXECUTION_COUNT.fetch_add(1, Ordering::Release);
    serial_println!(
        "[F13NR] executed pid={} source={} target={} round={}",
        pid,
        source,
        current_cpu_id(),
        round
    );
}
