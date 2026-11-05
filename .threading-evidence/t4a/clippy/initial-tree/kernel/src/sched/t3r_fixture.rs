//! Opt-in ring-3 migration proof. No queue, pin or saved-context mutation.
//! Commands require the exact registered PID, incarnation and VM identity.
use super::*;
use crate::process::{layout, mm2b_state::AddressSpaceIdentity};

#[derive(Clone, Copy)]
struct Identity {
    pid: usize,
    incarnation: u64,
    vm: AddressSpaceIdentity,
}
const EMPTY: Identity = Identity {
    pid: 0,
    incarnation: 0,
    vm: AddressSpaceIdentity::INVALID,
};
impl Identity {
    fn of(p: &Process) -> Self {
        Self {
            pid: p.pid,
            incarnation: p.foreground.incarnation,
            vm: p.address_space.identity(),
        }
    }
    fn matches(self, p: &Process) -> bool {
        self.pid == p.pid
            && self.incarnation == p.foreground.incarnation
            && self.vm == p.address_space.identity()
    }
}
struct Gate {
    owner: Identity,
    children: [Identity; 2],
    slots: [usize; 2],
    dispatch: [u64; 2],
    executions: u64,
    phase: usize,
    pulses: [usize; 3],
    retired: [usize; 2],
    heap: usize,
    frames: usize,
    stack: u64,
    tls: u64,
    verified: bool,
}
static GATE: spin::Mutex<Gate> = spin::Mutex::new(Gate {
    owner: EMPTY,
    children: [EMPTY; 2],
    slots: [0; 2],
    dispatch: [0; 2],
    executions: 0,
    phase: 0,
    pulses: [0; 3],
    retired: [0; 2],
    heap: 0,
    frames: 0,
    stack: 0,
    tls: 0,
    verified: false,
});

pub(crate) fn placement(s: &Scheduler, thread: &mut Process, role: u64) {
    let g = GATE.lock();
    assert!(g.owner.matches(s.current_process()));
    assert!(role < 2 && g.children[role as usize].pid == 0);
    // Existing affinity mechanism; placement happens before normal publication.
    thread.cpu_mask = 1;
}
pub(crate) fn created(s: &Scheduler, idx: usize, role: u64) {
    let mut g = GATE.lock();
    assert!(g.owner.matches(s.current_process()));
    let role = role as usize;
    let p = &s.processes[idx];
    g.children[role] = Identity::of(p);
    g.slots[role] = idx;
    if role == 0 {
        g.stack = p.kernel_stack_top;
        g.tls = p.fs_base;
    }
    crate::serial_println!("[T3R] created role={} owner={} owner_inc={} tid={} inc={} vm={:?} slot={} stack={:#x} tls={:#x} queued={}",role,g.owner.pid,g.owner.incarnation,p.pid,p.foreground.incarnation,p.address_space.identity(),idx,p.kernel_stack_top,p.fs_base,s.diagnostic_ready_occurrences(idx));
}

/// Read the CPU's loaded TSS, rather than trusting the logical-array accessor.
fn loaded_tss() -> (u64, u64, u64, u64) {
    let gdtr = x86_64::instructions::tables::sgdt();
    let selector: u16;
    unsafe {
        core::arch::asm!("str {0:x}", out(reg) selector, options(nomem, nostack));
        assert_eq!(selector & 4, 0);
        let offset = (selector & !7) as u64;
        assert!(offset + 15 <= gdtr.limit as u64);
        let lo = ((gdtr.base.as_u64() + offset) as *const u64).read_unaligned();
        let hi = ((gdtr.base.as_u64() + offset + 8) as *const u64).read_unaligned();
        let base = ((lo >> 16) & 0xffff)
            | ((lo >> 32) & 0xff) << 16
            | ((lo >> 56) & 0xff) << 24
            | (hi & 0xffff_ffff) << 32;
        // Architectural 64-bit TSS offsets: RSP0=4, IST1=36, IST2=44.
        (
            base,
            ((base + 4) as *const u64).read_unaligned(),
            ((base + 36) as *const u64).read_unaligned(),
            ((base + 44) as *const u64).read_unaligned(),
        )
    }
}
fn ownership(p: &Process, executing: bool) -> (u64, u64, u64, u64) {
    let state = loaded_tss();
    assert_eq!(state.1, p.kernel_stack_top, "loaded RSP0 owner");
    assert_eq!(state.1, crate::arch::x86_64::smp::current_cpu_tss_rsp0());
    assert_eq!(
        state.3,
        crate::arch::x86_64::syscall_stack::nmi_stack_top(current_cpu_id())
    );
    assert!(state.2 != 0 && state.2 != state.3 && state.2 != state.1);
    if executing {
        let rsp: u64;
        unsafe {
            core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack));
        }
        assert!(
            rsp < p.kernel_stack_top
                && rsp >= p.kernel_stack_top - crate::process::KERNEL_STACK_SIZE as u64
        );
    }
    state
}
pub(crate) fn dispatched(s: &Scheduler, idx: usize, cpu: usize) {
    let mut g = GATE.lock();
    let p = &s.processes[idx];
    let Some(role) = g.children.iter().position(|id| id.matches(p)) else {
        return;
    };
    assert_eq!(p.owning_core as usize, cpu);
    assert!(!s.stack_is_pinned_remotely(idx, cpu));
    let state = ownership(p, false);
    let fs = unsafe { x86_64::registers::model_specific::Msr::new(0xC0000100).read() };
    assert_eq!(fs, p.fs_base);
    if g.dispatch[role] & (1 << cpu) == 0 {
        crate::serial_println!("[T3R] dispatch role={} tid={} inc={} cpu={} apic={} stack={:#x} tls={:#x} tss={:#x} rsp0={:#x} df={:#x} nmi={:#x} remote_pin=false",role,p.pid,p.foreground.incarnation,cpu,crate::arch::x86_64::lapic::local_apic_id(),p.kernel_stack_top,fs,state.0,state.1,state.2,state.3);
    }
    g.dispatch[role] |= 1 << cpu;
}
pub(crate) fn woken(s: &Scheduler, idx: usize) {
    let g = GATE.lock();
    let p = &s.processes[idx];
    if !g.children[0].matches(p) {
        return;
    }
    assert_eq!(g.phase, 1);
    assert_eq!(current_cpu_id(), 0);
    assert_eq!(p.queued_on_core, 1);
    assert_eq!(p.owning_core, u8::MAX);
    assert!(peek_reschedule_on(1));
    crate::serial_println!(
        "[T3R] wake tid={} inc={} from=0 queued=1 request=true wait=cleared pinned={}",
        p.pid,
        p.foreground.incarnation,
        s.stack_is_pinned(idx)
    );
}
pub(crate) fn retired(s: &Scheduler, idx: usize) {
    let mut g = GATE.lock();
    let p = &s.processes[idx];
    if let Some(role) = g.children.iter().position(|id| id.matches(p)) {
        g.retired[role] += 1;
        assert_eq!(g.retired[role], 1, "duplicate retirement");
        crate::serial_println!(
            "[T3R] retire role={} tid={} inc={} count=1",
            role,
            p.pid,
            p.foreground.incarnation
        );
    }
}

pub(crate) fn query(command: &str) -> Option<u64> {
    if !command.starts_with("T3R ") {
        return None;
    }
    let mut s = SCHEDULER.lock();
    let p = s.current_process();
    let mut g = GATE.lock();
    let owner = g.owner.matches(p);
    let role = g.children.iter().position(|id| id.matches(p));
    if !owner && role.is_none() {
        return Some(u64::MAX);
    }
    Some(match command {
        "T3R source" | "T3R destination" => {
            assert_eq!(role, Some(0));
            let cpu = usize::from(command == "T3R destination");
            if cpu == 0 && g.pulses[0] == 0 {
                return Some(1);
            }
            assert_eq!(current_cpu_id(), cpu);
            assert_eq!(p.kernel_stack_top, g.stack);
            assert_eq!(p.fs_base, g.tls);
            let fs = unsafe { x86_64::registers::model_specific::Msr::new(0xC0000100).read() };
            assert_eq!(fs, g.tls);
            assert_ne!(g.dispatch[0] & (1 << cpu), 0);
            let state = ownership(p, true);
            assert_eq!(p.owning_core as usize, cpu);
            assert_ne!(g.executions & (1 << cpu), 1 << cpu);
            g.executions |= 1 << cpu;
            crate::serial_println!("[T3R] execution tid={} inc={} cpu={} apic={} stack={:#x} tls={:#x} tss={:#x} rsp0={:#x} df={:#x} nmi={:#x}",p.pid,p.foreground.incarnation,cpu,crate::arch::x86_64::lapic::local_apic_id(),p.kernel_stack_top,fs,state.0,state.1,state.2,state.3);
            if cpu == 0 {
                g.phase = 1;
            } else {
                assert!(g.pulses[1] > 0);
                g.phase = 2;
            }
            0
        }
        "T3R pulse" => {
            assert_eq!(role, Some(1));
            ownership(p, true);
            assert_eq!(current_cpu_id(), 0);
            let phase = g.phase;
            g.pulses[phase] += 1;
            let child = &s.processes[g.slots[0]];
            if phase == 1
                && child.state == ProcessState::BlockedOnIo
                && child.native_futex_wait.is_some()
                && child.owning_core == u8::MAX
                && !s.stack_is_pinned(g.slots[0])
            {
                assert!(g.children[0].matches(child));
                assert_eq!(s.diagnostic_ready_occurrences(g.slots[0]), 0);
                // The registered sibling requests affinity for a quiescent task.
                // Normal futex WAKE performs all queue placement and notification.
                let id = g.children[0];
                s.processes[g.slots[0]].cpu_mask = 2;
                crate::serial_println!(
                    "[T3R] request tid={} inc={} source=0 destination=1 blocked=true pinned=false",
                    id.pid,
                    id.incarnation
                );
                1
            } else if phase == 2 {
                2
            } else {
                0
            }
        }
        "T3R state-ok" => {
            assert_eq!(role, Some(0));
            assert_eq!(g.executions, 3);
            assert_eq!(g.phase, 2);
            crate::serial_println!("[T3R] state tid={} inc={} tls=true fs=true gpr=true x87=true xmm=true mxcsr=true canary=true",p.pid,p.foreground.incarnation);
            g.verified = true;
            0
        }
        "T3R finish" => {
            assert!(owner);
            if g.retired != [1, 1] {
                return Some(0);
            }
            assert_eq!(g.executions, 3);
            assert!(g.verified);
            assert!(g.pulses.iter().all(|n| *n > 0));
            for role in 0..2 {
                let idx = g.slots[role];
                let p = &s.processes[idx];
                assert!(g.children[role].matches(p));
                assert_eq!(p.state, ProcessState::Reaped);
                assert_eq!(p.exit_code, 0);
                assert!(
                    p.kernel_stack.is_none()
                        && p.kernel_stack_top == 0
                        && p.context_rsp == 0
                        && p.fs_base == 0
                );
                assert!(!p.exit_cleanup_pending && p.native_futex_wait.is_none());
                assert_eq!((p.owning_core, p.queued_on_core), (u8::MAX, u8::MAX));
                assert_eq!(s.diagnostic_ready_occurrences(idx), 0);
                assert!(!s.stack_is_pinned(idx));
                assert!((0..s.online_cores).all(|cpu| s.core_current_task(cpu) != Some(idx)));
                crate::serial_println!("[T3R] reaped role={} tid={} inc={} stack=0 tls=0 current=0 queued=0 pins=0 pending=0 exit=0",role,p.pid,p.foreground.incarnation);
            }
            assert_eq!(
                s.processes
                    .iter()
                    .filter(|p| !p.owns_address_space && p.state != ProcessState::Reaped)
                    .count(),
                0
            );
            for cpu in 0..s.online_cores {
                assert_eq!(
                    STACK_HANDOFFS.slot(cpu).load(Ordering::Acquire),
                    handoff::EMPTY
                );
                assert_eq!(STACK_HANDOFFS.waiter_snapshot(cpu), 0);
            }
            assert_eq!(crate::process::fd_table::thread_description_refs(), 3);
            let cache = 2 * core::mem::size_of::<crate::process::fd_table::FdTable>();
            assert_eq!(crate::memory::heap::heap_stats().allocated, g.heap + cache);
            assert_eq!(crate::PMM.lock().free_page_count(), g.frames);
            crate::serial_println!("[T3R] resources heap_base={} heap_final={} cache_bytes={} frame_base={} frame_final={} workers=0 reservations=0 stacks=1 fd_refs=3 handoff_pins=0 handoff_waiters=0 user_backing=owner_static pulses={},{},{}",g.heap,crate::memory::heap::heap_stats().allocated,cache,g.frames,crate::PMM.lock().free_page_count(),g.pulses[0],g.pulses[1],g.pulses[2]);
            crate::serial_println!(
                "[T3R] PASS cpus={} executions=0,1 physical_distinct=true",
                s.online_cores
            );
            unsafe {
                core::arch::asm!("out dx, eax",in("dx")0xf4u16,in("eax")0x10u32);
            }
            1
        }
        "T3R fail" => panic!("T3R user state check failed role={:?}", role),
        _ => u64::MAX,
    })
}
pub(crate) fn run(hhdm: x86_64::VirtAddr) {
    let cpus = ONLINE_CORES.load(Ordering::Acquire);
    assert!(cpus >= 2);
    init_cores(cpus);
    let mut pmm = crate::PMM.lock();
    let mut p = unsafe { Process::new(0xE300, 0, "t3r-owner", &mut pmm, hhdm) };
    let entry = crate::process::elf_loader::load_elf(
        include_bytes!(concat!(env!("OUT_DIR"), "/t3r.elf")),
        &mut p,
        &mut pmm,
        hhdm,
    )
    .unwrap();
    crate::process::spawn::map_user_stack(&mut p, &mut pmm, hhdm).unwrap();
    p.cpu_mask = 1;
    p.init_context(entry, layout::USER_STACK_TOP);
    drop(pmm);
    let mut s = SCHEDULER.lock();
    let idx = s.add_process_after_reaping(p);
    s.reserve_task_storage().unwrap();
    let mut g = GATE.lock();
    g.owner = Identity::of(&s.processes[idx]);
    g.heap = crate::memory::heap::heap_stats().allocated;
    g.frames = crate::PMM.lock().free_page_count();
    crate::serial_println!(
        "[T3R] begin cpus={} owner={} inc={} vm={:?}",
        cpus,
        g.owner.pid,
        g.owner.incarnation,
        g.owner.vm
    );
    drop(g);
    drop(s);
    enter_first_process();
}
