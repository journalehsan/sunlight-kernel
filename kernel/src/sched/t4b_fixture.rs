//! Debug-only T4B observations of the production waiter lifecycle.
//! SCHEDULER -> GATE; no waiter owner or production scheduling policy here.
use super::*;
use crate::process::{layout, mm2b_state::AddressSpaceIdentity};
use core::sync::atomic::Ordering;
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
const ACTORS: usize = 32;
struct Gate {
    owner: Identity,
    actors: [Identity; ACTORS],
    slots: [usize; ACTORS],
    stacks: [u64; ACTORS],
    tls: [u64; ACTORS],
    registered: [usize; ACTORS],
    completed: [usize; ACTORS],
    results: [u64; ACTORS],
    dispatch: [u64; ACTORS],
    case: usize,
    verified: usize,
    wakes: usize,
    base_heap: usize,
    stable_heap: usize,
    base_frames: usize,
    cached: usize,
    address: u64,
}
static GATE: spin::Mutex<Gate> = spin::Mutex::new(Gate {
    owner: EMPTY,
    actors: [EMPTY; ACTORS],
    slots: [0; ACTORS],
    stacks: [0; ACTORS],
    tls: [0; ACTORS],
    registered: [0; ACTORS],
    completed: [0; ACTORS],
    results: [u64::MAX; ACTORS],
    dispatch: [0; ACTORS],
    case: 0,
    verified: 0,
    wakes: 0,
    base_heap: 0,
    stable_heap: 0,
    base_frames: 0,
    cached: 0,
    address: 0,
});
pub(crate) fn syscall_enter(_: u64, _: u64, _: u64) {}
pub(crate) fn point(_: u64, _: u64, _: usize) {}
pub(crate) fn wake_done(_: u64, _: u64) {}
pub(crate) fn placement(s: &Scheduler, p: &mut Process, role: u64) {
    let g = GATE.lock();
    assert!(g.owner.matches(s.current_process()));
    assert!(role < ACTORS as u64);
    p.cpu_mask = if role == 0 {
        1
    } else if role == 1 && s.online_cores > 1 {
        2
    } else {
        (1 << s.online_cores) - 1
    };
}
pub(crate) fn created(s: &Scheduler, idx: usize, role: u64) {
    let mut g = GATE.lock();
    let r = role as usize;
    assert!(g.owner.matches(s.current_process()));
    assert_eq!(g.actors[r].pid, 0);
    let p = &s.processes[idx];
    g.actors[r] = Identity::of(p);
    g.slots[r] = idx;
    g.stacks[r] = p.kernel_stack_top;
    g.tls[r] = p.fs_base;
    crate::serial_println!(
        "[T4B] created case={} role={} tid={} inc={} vm={} stack={:#x} tls={:#x}",
        g.case,
        r,
        p.pid,
        p.foreground.incarnation,
        p.address_space.identity().generation,
        p.kernel_stack_top,
        p.fs_base
    );
}
pub(crate) fn dispatched(s: &Scheduler, idx: usize, cpu: usize) {
    let mut g = GATE.lock();
    let p = &s.processes[idx];
    let Some(role) = g.actors.iter().position(|id| id.matches(p)) else {
        return;
    };
    assert_eq!(p.kernel_stack_top, g.stacks[role]);
    assert_eq!(p.fs_base, g.tls[role]);
    assert_eq!(
        crate::arch::x86_64::smp::current_cpu_tss_rsp0(),
        p.kernel_stack_top
    );
    let fs = unsafe { x86_64::registers::model_specific::Msr::new(0xc0000100).read() };
    assert_eq!(fs, p.fs_base);
    assert!(!s.stack_is_pinned_remotely(idx, cpu));
    g.dispatch[role] |= 1 << cpu;
}
pub(crate) fn registered(p: &Process) {
    let mut g = GATE.lock();
    let Some(r) = g.actors.iter().position(|id| id.matches(p)) else {
        return;
    };
    assert_eq!(g.registered[r], g.completed[r]);
    g.registered[r] += 1;
    crate::serial_println!(
        "[T4B] registered case={} role={} seq={} tid={} inc={}",
        g.case,
        r,
        g.registered[r],
        p.pid,
        p.foreground.incarnation
    );
}
pub(crate) fn completing(p: &Process, result: u64) {
    if p.native_futex_wait.is_none() {
        return;
    }
    let mut g = GATE.lock();
    let Some(r) = g.actors.iter().position(|id| id.matches(p)) else {
        return;
    };
    assert_eq!(g.registered[r], g.completed[r] + 1);
    g.completed[r] += 1;
    g.results[r] = result;
    crate::serial_println!(
        "[T4B] terminal case={} role={} seq={} tid={} inc={} result={}",
        g.case,
        r,
        g.completed[r],
        p.pid,
        p.foreground.incarnation,
        result
    );
}
pub(crate) fn waking(s: &Scheduler, idx: usize) {
    let mut g = GATE.lock();
    let p = &s.processes[idx];
    // Expiry has already detached before entering its wake-placement helper.
    if p.native_futex_wait.is_some() && g.actors.iter().any(|id| id.matches(p)) {
        g.wakes += 1;
    }
}
fn num(w: &mut core::str::SplitWhitespace<'_>) -> u64 {
    w.next().unwrap().parse().unwrap()
}
pub(crate) fn query(command: &str) -> Option<u64> {
    if !command.starts_with("T4B ") {
        return None;
    }
    let mut w = command.split_whitespace();
    w.next();
    let op = w.next().unwrap();
    let mut s = SCHEDULER.lock();
    let mut g = GATE.lock();
    let owner = g.owner.matches(s.current_process());
    let role = g
        .actors
        .iter()
        .position(|id| id.matches(s.current_process()));
    assert!(owner || role.is_some());
    Some(match op {
        "yield" => {
            s.account_and_apply_churn_penalty();
            if s.current_process().state == ProcessState::Running {
                s.current_process_mut().state = ProcessState::Ready;
            }
            drop(g);
            drop(s);
            request_reschedule_after_sysret();
            return Some(0);
        }
        "baseline" => {
            assert!(owner);
            g.base_frames = crate::PMM.lock().free_page_count();
            0
        }
        "arm" => {
            assert!(owner);
            let case = num(&mut w) as usize;
            assert_eq!(case, g.verified);
            g.case = case;
            g.address = num(&mut w);
            g.actors = [EMPTY; ACTORS];
            g.registered = [0; ACTORS];
            g.completed = [0; ACTORS];
            g.results = [u64::MAX; ACTORS];
            g.dispatch = [0; ACTORS];
            g.wakes = 0;
            0
        }
        "ready" => {
            assert!(owner || role == Some(1));
            if g.case == 1 || g.case == 9 {
                u64::from(g.completed[0] == if g.case == 9 { 16 } else { 1 })
            } else {
                let count = if g.case == 8 { 32 } else { 1 };
                u64::from((0..count).all(|r| {
                    let p = &s.processes[g.slots[r]];
                    g.actors[r].pid != 0
                        && g.actors[r].matches(p)
                        && p.state == ProcessState::BlockedOnIo
                        && p.native_futex_wait.is_some()
                        && p.owning_core == u8::MAX
                        && !s.stack_is_pinned(g.slots[r])
                }))
            }
        }
        "cancel" | "kill" | "spurious" => {
            assert_eq!(role, Some(1));
            let pid = g.actors[0].pid;
            drop(g);
            if op == "kill" {
                assert!(s.terminate_process_by_pid(pid, 37, "t4b-waiting-exit"));
            } else {
                s.wake_for_signal(pid);
            }
            return Some(0);
        }
        "migrate" => {
            assert_eq!(role, Some(1));
            let idx = g.slots[0];
            assert_eq!(s.processes[idx].owning_core, u8::MAX);
            assert!(!s.stack_is_pinned(idx));
            s.processes[idx].cpu_mask = if s.online_cores > 1 { 2 } else { 1 };
            0
        }
        "remaining" => {
            assert!(owner);
            g.actors
                .iter()
                .enumerate()
                .filter(|(r, id)| {
                    id.pid != 0 && s.processes[g.slots[*r]].native_futex_wait.is_some()
                })
                .count() as u64
        }
        "waker" => {
            assert!(role.is_some());
            let cpu = usize::from(s.online_cores > 1);
            s.current_process_mut().cpu_mask = 1 << cpu;
            let ready = current_cpu_id() == cpu;
            if ready {
                crate::serial_println!(
                    "[T4B] contender role={} tid={} inc={} worker_cpu={} owner_cpu=0",
                    role.unwrap(),
                    s.current_process().pid,
                    s.current_process().foreground.incarnation,
                    cpu
                );
            }
            u64::from(ready)
        }
        "other-ready" => {
            assert!(owner);
            let address = num(&mut w);
            u64::from(g.actors.iter().enumerate().any(|(r, id)| {
                id.pid != 0
                    && s.processes[g.slots[r]].native_futex_wait == Some(address)
                    && s.processes[g.slots[r]].owning_core == u8::MAX
                    && !s.stack_is_pinned(g.slots[r])
            }))
        }
        "cardinality" => {
            assert!(owner || role.is_some());
            let limit = num(&mut w);
            let selected = num(&mut w);
            assert!(selected <= limit);
            crate::serial_println!(
                "[T4B] wake case={} role={} limit={} selected={}",
                g.case,
                role.unwrap_or(64),
                limit,
                selected
            );
            0
        }
        "verify" => {
            assert!(owner);
            let actors = if g.case == 8 { 32 } else { 3 };
            if g.actors.iter().filter(|id| id.pid != 0).count() != actors {
                return Some(0);
            }
            for r in 0..ACTORS {
                if g.actors[r].pid == 0 {
                    continue;
                }
                let idx = g.slots[r];
                let p = &s.processes[idx];
                if p.state != ProcessState::Reaped {
                    return Some(0);
                }
                assert!(g.actors[r].matches(p));
                assert_eq!(p.exit_code, if g.case == 6 && r == 0 { 37 } else { 0 });
                assert_eq!(g.registered[r], g.completed[r]);
                assert!(
                    p.native_futex_wait.is_none()
                        && p.native_futex_deadline.is_none()
                        && p.native_futex_result.is_none()
                        && p.futex_return_pending.is_none()
                );
                assert_eq!(p.linux_state().and_then(|st| st.futex_wait), None);
                assert!(
                    p.kernel_stack.is_none()
                        && p.kernel_stack_top == 0
                        && p.context_rsp == 0
                        && p.fs_base == 0
                );
                assert_eq!((p.owning_core, p.queued_on_core), (u8::MAX, u8::MAX));
                assert_eq!(s.diagnostic_ready_occurrences(idx), 0);
                assert!(!s.stack_is_pinned(idx));
                assert_ne!(g.dispatch[r], 0);
            }
            let expected = if g.case == 8 {
                33
            } else if g.case == 9 {
                16
            } else {
                1
            };
            assert_eq!(g.registered.iter().sum::<usize>(), expected);
            if g.case == 7 {
                assert_eq!(g.dispatch[0], if s.online_cores > 1 { 3 } else { 1 });
            }
            let heap = crate::memory::heap::heap_stats().allocated;
            let frames = crate::PMM.lock().free_page_count();
            if g.case == 0 {
                g.stable_heap = heap;
            }
            g.cached = g.cached.max(actors);
            assert_eq!(
                heap,
                g.stable_heap
                    + (g.cached - 3) * core::mem::size_of::<crate::process::fd_table::FdTable>()
            );
            assert_eq!(frames, g.base_frames);
            assert_eq!(crate::process::fd_table::thread_description_refs(), 3);
            for cpu in 0..s.online_cores {
                assert_eq!(
                    STACK_HANDOFFS.slot(cpu).load(Ordering::Acquire),
                    handoff::EMPTY
                );
                assert_eq!(STACK_HANDOFFS.waiter_snapshot(cpu), 0);
            }
            crate::serial_println!("[T4B] case={} registered={} completed={} selected={} result={} dispatch0={:#x} heap={} frames={} active=0 deadlines=0 receipts=0 queues=0 pins=0 refs=3",g.case,expected,g.completed.iter().sum::<usize>(),g.wakes,g.results[0],g.dispatch[0],heap,frames);
            g.verified += 1;
            1
        }
        "progress" => {
            assert!(owner);
            let case = num(&mut w);
            let progress = num(&mut w);
            let selected = num(&mut w);
            assert_eq!(case as usize + 1, g.verified);
            assert!(progress > 0);
            assert_eq!(selected, if g.case == 8 { 32 } else { 0 });
            crate::serial_println!(
                "[T4B] progress case={} ticks={} userspace_selected={}",
                case,
                progress,
                selected
            );
            0
        }
        "finish" => {
            assert!(owner);
            assert_eq!(g.verified, 11);
            crate::serial_println!(
                "[T4B] PASS cpus={} cases=11 resources=stable",
                s.online_cores
            );
            unsafe {
                core::arch::asm!("out dx,eax",in("dx")0xf4u16,in("eax")0x10u32);
            }
            1
        }
        "fail" => panic!("T4B userspace failure case={} role={:?}", g.case, role),
        _ => panic!("unknown T4B control"),
    })
}
pub(crate) fn run(hhdm: x86_64::VirtAddr) {
    let cpus = ONLINE_CORES.load(Ordering::Acquire);
    assert!(cpus >= 1);
    init_cores(cpus);
    let mut pmm = crate::PMM.lock();
    let mut p = unsafe { Process::new(0xe400, 0, "t4b-owner", &mut pmm, hhdm) };
    let entry = crate::process::elf_loader::load_elf(
        include_bytes!(concat!(env!("OUT_DIR"), "/t4a.elf")),
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
    g.base_heap = crate::memory::heap::heap_stats().allocated;
    g.base_frames = crate::PMM.lock().free_page_count();
    crate::serial_println!(
        "[T4B] begin cpus={} owner={} inc={} vm={:?} heap={} frames={}",
        cpus,
        g.owner.pid,
        g.owner.incarnation,
        g.owner.vm,
        g.base_heap,
        g.base_frames
    );
    drop(g);
    drop(s);
    enter_first_process();
}
