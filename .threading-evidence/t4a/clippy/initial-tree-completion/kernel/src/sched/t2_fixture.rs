//! Debug-only, kernel-loaded ring-3 controls. The full VM incarnation and
//! publication identity authorize commands; no production selector exists.
use super::*;
use crate::process::mm2b_state::AddressSpaceIdentity;
use crate::process::{layout, thread_admission::Error};

#[derive(Clone, Copy)]
struct Owner {
    pid: usize,
    incarnation: u64,
    vm: AddressSpaceIdentity,
}
const EMPTY: Owner = Owner {
    pid: 0,
    incarnation: 0,
    vm: AddressSpaceIdentity::INVALID,
};
struct Gate {
    owners: [Owner; 4],
    phase: usize,
    full: u8,
    drained: u8,
    cycles: usize,
    pulses: u64,
    observer: usize,
    first_heap: usize,
    arm: usize,
    armed: usize,
    active: bool,
    seen: usize,
    hit: bool,
    baseline: [usize; 6],
    failures: [usize; 2],
    finished: bool,
}
static GATE: spin::Mutex<Gate> = spin::Mutex::new(Gate {
    owners: [EMPTY; 4],
    phase: 0,
    full: 0,
    drained: 0,
    cycles: 0,
    pulses: 0,
    observer: 0,
    first_heap: 0,
    arm: 0,
    armed: 0,
    active: false,
    seen: 0,
    hit: false,
    baseline: [0; 6],
    failures: [0; 2],
    finished: false,
});

pub(crate) struct FaultGuard;
pub(crate) fn begin(parent: &Process) -> FaultGuard {
    let mut g = GATE.lock();
    g.active = g.arm != 0
        && g.owners.get(g.armed).is_some_and(|o| {
            o.pid == parent.pid
                && o.incarnation == parent.foreground.incarnation
                && o.vm == parent.address_space.identity()
        });
    g.seen = 0;
    g.hit = false;
    FaultGuard
}
impl Drop for FaultGuard {
    fn drop(&mut self) {
        GATE.lock().active = false;
    }
}
pub(crate) fn allocation(stage: &'static str) -> Result<(), Error> {
    let mut g = GATE.lock();
    if !g.active {
        return Ok(());
    }
    g.seen += 1;
    assert!(g.seen <= 1024, "unbounded T2 injection");
    if g.seen == g.arm {
        g.hit = true;
        crate::serial_println!(
            "[T2-FAULT] abi={} ordinal={} stage={} rejected=ENOMEM",
            g.armed,
            g.seen,
            stage
        );
        return Err(Error::NoMemory);
    }
    Ok(())
}
fn workers(s: &Scheduler, vm: Option<AddressSpaceIdentity>) -> usize {
    s.processes
        .iter()
        .filter(|p| {
            !p.owns_address_space
                && p.state != ProcessState::Reaped
                && vm.is_none_or(|vm| p.address_space.identity() == vm)
        })
        .count()
}
fn snapshot(s: &Scheduler) -> [usize; 6] {
    [
        crate::memory::heap::heap_stats().allocated,
        workers(s, None),
        s.processes
            .iter()
            .filter(|p| p.kernel_stack.is_some())
            .count(),
        crate::process::fd_table::thread_description_refs(),
        s.processes.iter().map(|p| p.capabilities.len()).sum(),
        (0..s.processes.len())
            .map(|idx| s.diagnostic_ready_occurrences(idx))
            .sum(),
    ]
}

pub(crate) fn query(command: &str) -> Option<u64> {
    if !command.starts_with("T2 ") {
        return None;
    }
    let s = SCHEDULER.lock();
    let mut g = GATE.lock();
    let p = s.current_process();
    let vm = p.address_space.identity();
    if command == "T2 pulse" && g.owners.iter().any(|o| o.vm == vm) {
        if g.full == 7 {
            g.pulses |= 1u64 << current_cpu_id();
        }
        return Some(0);
    }
    let Some(role) = g
        .owners
        .iter()
        .position(|o| o.pid == p.pid && o.incarnation == p.foreground.incarnation && o.vm == vm)
    else {
        return Some(u64::MAX);
    };
    let result = if command == "T2 phase" {
        if role == 3 {
            g.observer += 1;
        }
        g.phase as u64
    } else if let Some(value) = command.strip_prefix("T2 arm ") {
        let ordinal = value.parse::<usize>().expect("fixture ordinal");
        assert!(role <= 1 && ordinal <= 128 && ordinal != 0 && g.phase == role);
        g.arm = ordinal;
        g.armed = role;
        g.baseline = snapshot(&s);
        0
    } else if command == "T2 check" {
        assert!(role <= 1 && g.armed == role);
        if g.hit {
            let now = snapshot(&s);
            // Queued/current counts can change due to ordinary scheduling;
            // all references, allocations and live objects must be identical.
            assert_eq!(&now[..5], &g.baseline[..5], "failed construction leaked");
            g.failures[role] += 1;
            g.arm = 0;
            1
        } else {
            g.arm = 0;
            0
        }
    } else if command == "T2 fault-done" {
        assert_eq!(g.phase, role);
        assert!(g.failures[role] >= 70);
        g.phase += 1;
        crate::serial_println!(
            "[T2] faults abi={} points={} cleanup=exact",
            role,
            g.failures[role]
        );
        0
    } else if command == "T2 full" {
        assert!(role < 3 && g.phase == 2);
        assert_eq!(
            workers(&s, Some(vm)),
            crate::process::thread_admission::MAX_VM_WORKERS
        );
        g.full |= 1 << role;
        0
    } else if command == "T2 global-ready" {
        u64::from(
            g.phase == 2 && g.full == 7 && g.pulses.count_ones() >= s.online_cores.min(2) as u32,
        )
    } else if command == "T2 pressure" {
        assert_eq!(role, 3);
        assert_eq!(
            workers(&s, None),
            crate::process::thread_admission::MAX_GLOBAL_WORKERS
        );
        assert!(g.observer > 0);
        assert!(g.pulses != 0);
        if s.online_cores > 1 {
            assert!(g.pulses.count_ones() >= 2, "no useful remote CPU work");
        }
        crate::serial_println!(
            "[T2] pressure cycle={} workers=96 vm_workers=32 observer={} useful_cpus={:#x}",
            g.cycles,
            g.observer,
            g.pulses
        );
        crate::serial_println!(
            "[T2] pressure-heap cycle={} allocated={} free={}",
            g.cycles,
            crate::memory::heap::heap_stats().allocated,
            crate::memory::heap::heap_stats().free
        );
        g.phase = 3;
        0
    } else if command == "T2 drained" {
        if workers(&s, Some(vm)) != 0 {
            return Some(0);
        }
        assert!(role < 3);
        g.drained |= 1 << role;
        if g.drained == 7 {
            assert_eq!(workers(&s, None), 0);
            let heap = crate::memory::heap::heap_stats().allocated;
            if g.cycles == 0 {
                g.first_heap = heap;
            } else {
                assert_eq!(heap, g.first_heap, "exhaustion/recovery heap drift");
            }
            g.cycles += 1;
            crate::serial_println!(
                "[T2] recovery cycle={} heap={} drift=0 unreaped_workers=0",
                g.cycles,
                heap
            );
            g.full = 0;
            g.pulses = 0;
            g.drained = 0;
            g.phase = if g.cycles == 3 { 4 } else { 2 };
        }
        1
    } else if command == "T2 quiescent" {
        u64::from(workers(&s, Some(vm)) == 0)
    } else if command == "T2 fail" {
        panic!("T2 userspace fixture failed role={}", role);
    } else {
        return Some(u64::MAX);
    };
    // Every command checks unique queue membership and T1 stack ownership.
    for idx in 0..s.processes.len() {
        assert!(s.diagnostic_ready_occurrences(idx) <= 1);
        let p = &s.processes[idx];
        if p.state == ProcessState::Reaped {
            assert!(p.kernel_stack.is_none());
            assert!(!s.stack_is_pinned(idx));
        }
    }
    Some(result)
}

pub(crate) fn run(hhdm: x86_64::VirtAddr) {
    crate::serial_println!(
        "[T2] resource-layout stack={} process={} fd_table={} inherited_budget={} headroom={}",
        crate::process::KERNEL_STACK_SIZE,
        core::mem::size_of::<Process>(),
        core::mem::size_of::<crate::process::fd_table::FdTable>(),
        crate::process::thread_admission::MAX_INHERITED_BYTES,
        crate::process::thread_admission::THREAD_HEAP_HEADROOM
    );
    let cpus = ONLINE_CORES.load(Ordering::Acquire);
    init_cores(cpus);
    for role in 0..4 {
        let elf = if role == 1 {
            include_bytes!(concat!(env!("OUT_DIR"), "/t2-linux.elf")).as_slice()
        } else {
            include_bytes!(concat!(env!("OUT_DIR"), "/t2-native.elf")).as_slice()
        };
        let mut pmm = crate::PMM.lock();
        let mut p = unsafe { Process::new(0xD000 + role, 0, "t2-owner", &mut pmm, hhdm) };
        let entry = crate::process::elf_loader::load_elf(elf, &mut p, &mut pmm, hhdm).unwrap();
        crate::process::spawn::map_user_stack(&mut p, &mut pmm, hhdm).unwrap();
        p.env.set("A", "one");
        p.env.set("B", "two");
        // Real broker authority makes capability-vector allocation nonempty
        // at the injected clone stage; no synthetic bearer is adopted.
        let (endpoint_id, token) = crate::capability::CAP_BROKER
            .lock()
            .create_endpoint(p.pid)
            .unwrap();
        p.capabilities.push(crate::process::Capability {
            token: token.as_u64(),
            endpoint_id,
            can_send: true,
            can_recv: true,
            can_grant: false,
        });
        if role == 1 {
            p.personality =
                crate::process::ProcessPersonality::Linux(crate::process::LinuxProcessState::new());
            p.linux_tgid = Some(p.pid);
        }
        p.init_context(entry, layout::USER_STACK_TOP);
        p.set_initial_args(role as u64, 0, 0, 0);
        drop(pmm);
        let mut s = SCHEDULER.lock();
        let idx = s.add_process_after_reaping(p);
        let p = &s.processes[idx];
        GATE.lock().owners[role] = Owner {
            pid: p.pid,
            incarnation: p.foreground.incarnation,
            vm: p.address_space.identity(),
        };
    }
    crate::serial_println!("[T2] begin cpus={} limits=32,96,512 cycles=3", cpus);
    enter_first_process();
}

pub(crate) fn on_tick(s: &Scheduler) {
    let mut g = GATE.lock();
    assert!(s.global_tick < 24000, "T2 bounded fixture deadline");
    if g.finished || g.phase != 4 {
        return;
    }
    if g.owners.iter().all(|owner| {
        s.processes
            .iter()
            .find(|p| p.pid == owner.pid)
            .is_some_and(|p| p.state == ProcessState::Reaped && p.exit_code == 0)
    }) {
        g.finished = true;
        crate::serial_println!("[T2] PASS faults_native={} faults_linux={} cycles=3 pressure=96 recovery=exact queues=unique T1_pins=preserved", g.failures[0], g.failures[1]);
        unsafe {
            core::arch::asm!("out dx, eax", in("dx") 0xf4u16, in("eax") 0x10u32);
        }
    }
}
