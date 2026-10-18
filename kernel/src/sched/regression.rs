//! Opt-in, isolated ring-3 lifecycle gate. No normal services or workload timing.
//! Fixture objects/VMs are allocated before dispatch; the timer only publishes
//! prebuilt replacements, kills workers and checks invariants.
use super::*;
use crate::process::{address_space::SharedAddressSpaceHandle, layout};
use core::sync::atomic::{AtomicUsize, Ordering};
use x86_64::VirtAddr;

const ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/scheduler-probe.elf"));
static DEFERRED: AtomicUsize = AtomicUsize::new(0);
static EXIT_RACES: AtomicUsize = AtomicUsize::new(0);
struct Gate {
    replacements: Vec<Process>,
    second_wave: bool,
    published: usize,
}
static GATE: spin::Mutex<Gate> = spin::Mutex::new(Gate {
    replacements: Vec::new(),
    second_wave: false,
    published: 0,
});

pub fn note_deferred_reap() {
    DEFERRED.fetch_add(1, Ordering::Relaxed);
}

/// Deterministically reproduce a remote terminal transition winning just
/// before self-exit commits. Run with S held; the real external-kill method
/// preserves its result and the normal finisher must still close both clocks.
pub fn race_exit(s: &mut Scheduler, idx: usize) {
    let p = &s.processes[idx];
    if p.name_str() == "gate-worker"
        && p.pid % 2 == 1
        && EXIT_RACES
            .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        let pid = p.pid;
        assert!(s.terminate_process_by_pid(pid, 137, "scheduler-b-exit-race"));
    }
}

pub fn handoff_delay() {
    // Deliberately widen the post-lock/pre-pivot window, without waiting for
    // another CPU, taking locks, allocating or using the UART in that window.
    if unsafe { &*stack_handoff_slot() }.load(Ordering::Acquire) != handoff::EMPTY {
        for _ in 0..1024 {
            core::hint::spin_loop();
        }
    }
}

fn private(pid: usize, role: u64, hhdm: VirtAddr) -> Process {
    let mut pmm = crate::PMM.lock();
    let mut p = unsafe { Process::new(pid, 0, "gate-worker", &mut pmm, hhdm) };
    let entry = crate::process::elf_loader::load_elf(ELF, &mut p, &mut pmm, hhdm)
        .expect("scheduler fixture ELF");
    crate::process::spawn::map_user_stack(&mut p, &mut pmm, hhdm).unwrap();
    p.init_context(entry, layout::USER_STACK_TOP);
    p.set_initial_args(role, 0, 0, 0);
    p
}

fn borrower(pid: usize, owner: usize, vm: SharedAddressSpaceHandle, linux: bool) -> Process {
    let mut p = Process::new_thread(
        pid,
        owner,
        "gate-borrower",
        vm,
        crate::process::fd_table::FdTable::new_boxed(),
        crate::process::env::EnvMap::new(),
        0,
        0,
        0,
        Vec::new(),
        None,
        0,
    );
    if linux {
        p.linux_tgid = Some(owner);
        p.personality =
            crate::process::ProcessPersonality::Linux(crate::process::LinuxProcessState::new());
    } else {
        p.native_thread = true;
        note_native_borrower_created();
    }
    // Separate user stacks within the owner's existing 2 MiB stack region.
    p.init_context(
        layout::USER_CODE_START,
        layout::USER_STACK_TOP - (pid - owner) as u64 * 65536,
    );
    p.set_initial_args(if linux { 0 } else { 1 }, 0, 0, 0);
    p
}

pub fn run(hhdm: VirtAddr) {
    let cpus = ONLINE_CORES.load(Ordering::Acquire);
    init_cores(cpus);
    SCHEDULER.lock().processes.reserve(128);
    let mut pid = 0xE000;
    for n in 0..2 * cpus {
        let mut p = private(pid, (n % 2) as u64, hhdm);
        if n % 2 == 1 {
            p.nice = 8 + (n % 3) as i8;
        }
        SCHEDULER.lock().add_process_after_reaping(p);
        pid += 1;
    }
    for linux in [false, true] {
        let mut owner = private(pid, 0, hhdm);
        owner.name[..10].copy_from_slice(b"gate-owner");
        if linux {
            owner.linux_tgid = Some(pid);
            owner.personality =
                crate::process::ProcessPersonality::Linux(crate::process::LinuxProcessState::new());
        }
        let shared = owner.address_space.shared_handle();
        SCHEDULER.lock().add_process_after_reaping(owner);
        for offset in 1..=2 {
            let p = borrower(pid + offset, pid, shared.clone(), linux);
            SCHEDULER.lock().add_process_after_reaping(p);
        }
        pid += 3;
    }
    // Replacement allocations happen at boot, never from a dispatch/IRQ path.
    let mut replacements = Vec::with_capacity(cpus);
    for _ in 0..cpus {
        replacements.push(private(pid, 1, hhdm));
        pid += 1;
    }
    {
        let mut g = GATE.lock();
        g.replacements = replacements;
        g.published = 2 * cpus + 6;
    }
    serial_println!(
        "[SCHED-B] begin cpus={} tasks={} replacements={}",
        cpus,
        2 * cpus + 6,
        cpus
    );
    enter_first_process()
}

pub fn on_tick(s: &mut Scheduler, cpu: usize) {
    // Run assertions on every CPU, including while remote epilogues hold pins.
    if let Some(idx) = s.core_current_task(cpu) {
        assert_eq!(s.processes[idx].owning_core as usize, cpu);
        assert_eq!(s.processes[idx].queued_on_core, u8::MAX);
    }
    if cpu != 0 {
        return;
    }
    for idx in 0..s.processes.len() {
        assert!(s.diagnostic_ready_occurrences(idx) <= 1);
        let p = &s.processes[idx];
        assert_ne!(p.exit_code, 99, "register/SSE cookie corrupted");
        if p.state == ProcessState::Reaped {
            assert!(
                p.exit_code == 0 || p.exit_code == 137,
                "unexpected fixture fault/exit"
            );
            assert!(!s.stack_is_pinned(idx));
            assert!(p.kernel_stack.is_none());
            assert_eq!(p.context_rsp, 0);
        }
        // Yielding tasks self-exit. Spin tasks exercise external and timer
        // fatal termination, but only after actually receiving execution.
        if s.global_tick >= 10
            && p.state == ProcessState::Running
            && (p.cpu_charge_since.is_some() || p.cpu_runtime_ns != 0)
        {
            let owner = p.name_str().starts_with("gate-owner");
            if owner && s.live_address_space_borrowers(idx) != 0 {
                continue;
            }
            // Ordinary odd workers are yielding (and must finish themselves).
            if p.name_str() == "gate-worker" && p.pid % 2 == 1 {
                continue;
            }
            let pid = p.pid;
            if pid % 2 == 0 {
                assert!(s.terminate_process_by_pid(pid, 137, "scheduler-b-external"));
            } else {
                s.processes[idx].foreground.forced_exit = Some(137);
                if let Some(owner) = s.live_owner_core(idx) {
                    request_reschedule_on(owner);
                }
            }
        }
    }
    let mut g = GATE.lock();
    if s.processes.iter().all(|p| p.state == ProcessState::Reaped) {
        if !g.second_wave {
            g.second_wave = true;
            while let Some(p) = g.replacements.pop() {
                let idx = s.add_process_after_reaping(p);
                assert!(idx < g.published, "expected safe Reaped slot reuse");
                s.enqueue_ready(idx);
            }
        } else {
            assert_eq!(EXIT_RACES.load(Ordering::Relaxed), 1);
            assert!(
                DEFERRED.load(Ordering::Relaxed) > 0,
                "outgoing reap guard not exercised"
            );
            for c in 0..s.online_cores {
                let core = CORE_STATES[c].lock();
                assert!(core.timer_ticks > 0);
                assert!(core.context_switches > 0);
                assert!(core.rr_queue.is_empty());
            }
            serial_println!(
                "[SCHED-B] PASS cpus={} deferred_reaps={} reused={} exit_races={}",
                s.online_cores,
                DEFERRED.load(Ordering::Relaxed),
                s.online_cores,
                EXIT_RACES.load(Ordering::Relaxed)
            );
            unsafe {
                x86_64::instructions::port::Port::<u32>::new(0xf4).write(0x10);
            }
        }
    }
    assert!(s.global_tick < 12000, "scheduler lifecycle gate timeout");
}
