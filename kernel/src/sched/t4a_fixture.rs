//! Isolated, debug-only ring-3 futex race fixture. SCHEDULER -> GATE.
//! Bounded rendezvous inside a critical section are test controls only.
use super::*;
use crate::process::{layout, mm2b_state::AddressSpaceIdentity};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

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
    actors: [Identity; 32],
    slots: [usize; 32],
    stacks: [u64; 32],
    tls: [u64; 32],
    registered: [usize; 32],
    completed: [usize; 32],
    dispatch: [u64; 32],
    case: usize,
    epoch: usize,
    verified: usize,
    wakes: usize,
    live_wakes: usize,
    base_heap: usize,
    base_frames: usize,
    stable_heap: usize,
    cardinal_steps: usize,
}
static GATE: spin::Mutex<Gate> = spin::Mutex::new(Gate {
    owner: EMPTY,
    actors: [EMPTY; 32],
    slots: [0; 32],
    stacks: [0; 32],
    tls: [0; 32],
    registered: [0; 32],
    completed: [0; 32],
    dispatch: [0; 32],
    case: 0,
    epoch: 0,
    verified: 0,
    wakes: 0,
    live_wakes: 0,
    base_heap: 0,
    base_frames: 0,
    stable_heap: 0,
    cardinal_steps: 0,
});
static ADDRESS: AtomicU64 = AtomicU64::new(0);
static POINT: AtomicUsize = AtomicUsize::new(usize::MAX);
static ENTERED: AtomicUsize = AtomicUsize::new(0);
static WAKE_DONE: AtomicUsize = AtomicUsize::new(0);
static WAITER_CPU: AtomicUsize = AtomicUsize::new(usize::MAX);
static WAKER_CPU: AtomicUsize = AtomicUsize::new(usize::MAX);
static USER_GATE: AtomicU64 = AtomicU64::new(0);

fn bounded(mut ready: impl FnMut() -> bool) {
    for _ in 0..200_000_000 {
        if ready() {
            return;
        }
        core::hint::spin_loop();
    }
    panic!(
        "T4A rendezvous expired point={}",
        POINT.load(Ordering::Acquire)
    );
}
/// No scheduler access: the competing CPU can announce syscall entry while
/// the waiter owns SCHEDULER. Actual wake still follows normal dispatch/locks.
pub(crate) fn syscall_enter(number: u64, address: u64, op: u64) {
    if matches!(number, 155 | 202) && op & 0x7f == 1 && ADDRESS.load(Ordering::Acquire) == address {
        WAKER_CPU.store(current_cpu_id(), Ordering::Release);
        ENTERED.fetch_add(1, Ordering::AcqRel);
    }
}
pub(crate) fn point(address: u64, op: u64, point: usize) {
    if op != 0
        || ADDRESS.load(Ordering::Acquire) != address
        || POINT.load(Ordering::Acquire) != point
    {
        return;
    }
    // The gate is a validated writable word in the owner's static ELF .bss.
    // It is never unmapped/replaced by this fixture. ARM resolves the HHDM
    // alias under SCHEDULER; the owner outlives all actors and gate use.
    WAITER_CPU.store(current_cpu_id(), Ordering::Release);
    let pointer = USER_GATE.load(Ordering::Acquire) as *const core::sync::atomic::AtomicU32;
    unsafe { (*pointer).store(1, Ordering::Release) };
    if point == 0 || point == 4 {
        bounded(|| WAKE_DONE.load(Ordering::Acquire) != 0);
    } else {
        bounded(|| ENTERED.load(Ordering::Acquire) != 0);
    }
    assert_ne!(
        WAITER_CPU.load(Ordering::Acquire),
        WAKER_CPU.load(Ordering::Acquire)
    );
}
pub(crate) fn wake_done(address: u64, _count: u64) {
    if ADDRESS.load(Ordering::Acquire) == address {
        WAKE_DONE.fetch_add(1, Ordering::AcqRel);
    }
}
pub(crate) fn placement(s: &Scheduler, thread: &mut Process, role: u64) {
    let g = GATE.lock();
    assert!(g.owner.matches(s.current_process()));
    assert!(role < 32);
    thread.cpu_mask = if g.case >= 15 {
        (1 << s.online_cores) - 1
    } else if role == 1 {
        2
    } else {
        1
    };
}
pub(crate) fn created(s: &Scheduler, idx: usize, role: u64) {
    let mut g = GATE.lock();
    let role = role as usize;
    assert!(g.owner.matches(s.current_process()));
    assert_eq!(g.actors[role].pid, 0);
    let p = &s.processes[idx];
    g.actors[role] = Identity::of(p);
    g.slots[role] = idx;
    g.stacks[role] = p.kernel_stack_top;
    g.tls[role] = p.fs_base;
    crate::serial_println!(
        "[T4A] created case={} role={} tid={} inc={} stack={:#x} tls={:#x}",
        g.epoch,
        role,
        p.pid,
        p.foreground.incarnation,
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
    let Some(role) = g.actors.iter().position(|id| id.matches(p)) else {
        return;
    };
    assert_eq!(
        g.registered[role], g.completed[role],
        "duplicate futex registration"
    );
    g.registered[role] += 1;
}
pub(crate) fn completing(p: &Process, _result: u64) {
    if p.native_futex_wait.is_none() && p.linux_state().and_then(|s| s.futex_wait).is_none() {
        return;
    }
    let mut g = GATE.lock();
    let Some(role) = g.actors.iter().position(|id| id.matches(p)) else {
        return;
    };
    assert_eq!(
        g.registered[role],
        g.completed[role] + 1,
        "duplicate completion"
    );
    g.completed[role] += 1;
}
/// Called before normal wake detaches a matching waiter.
pub(crate) fn waking(s: &Scheduler, idx: usize) {
    let mut g = GATE.lock();
    if g.actors.iter().any(|id| id.matches(&s.processes[idx])) {
        g.wakes += 1;
        if s.live_owner_core(idx).is_some() {
            g.live_wakes += 1;
        }
    }
}
fn number(words: &mut core::str::SplitWhitespace<'_>) -> u64 {
    words.next().unwrap().parse().unwrap()
}

pub(crate) fn query(command: &str) -> Option<u64> {
    if !command.starts_with("T4A ") {
        return None;
    }
    let mut words = command.split_whitespace();
    words.next();
    let operation = words.next().unwrap();
    let mut s = SCHEDULER.lock();
    let mut g = GATE.lock();
    let owner = g.owner.matches(s.current_process());
    let role = g
        .actors
        .iter()
        .position(|id| id.matches(s.current_process()));
    assert!(owner || role.is_some(), "unregistered T4A caller");
    Some(match operation {
        "linux" | "native" => {
            let tgid = g.owner.pid;
            assert!(owner || role.is_some());
            let p = s.current_process_mut();
            assert!(
                p.native_futex_wait.is_none()
                    && p.linux_state().and_then(|state| state.futex_wait).is_none()
            );
            if operation == "linux" {
                p.personality = crate::process::ProcessPersonality::Linux(
                    crate::process::LinuxProcessState::new(),
                );
                p.linux_tgid = Some(tgid);
            } else {
                p.personality = crate::process::ProcessPersonality::Native;
                p.linux_tgid = None;
            }
            0
        }
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
        "unsupported" => {
            assert!(owner);
            let hhdm = x86_64::VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
            let mut pmm = crate::PMM.lock();
            let mut caps = crate::capability::CAP_BROKER.lock();
            let p = s.current_process_mut();
            let (first, token) =
                crate::memory::shared::alloc_shared_region(p, &mut pmm, &mut caps, hhdm, 4096)
                    .unwrap();
            let alias = crate::memory::shared::map_shared_page(p, token, &mut pmm, &mut caps, hhdm)
                .unwrap();
            assert_ne!(first, alias);
            for address in [first.as_u64(), alias.as_u64()] {
                assert!(crate::memory::user::read_process_futex_u32(p, hhdm, address).is_err());
            }
            crate::memory::shared::free_shared_page(p, token, &mut pmm, &mut caps, hhdm).unwrap();
            crate::memory::shared::free_shared_page(p, token, &mut pmm, &mut caps, hhdm).unwrap();
            crate::serial_println!(
                "[T4A] unsupported SHM aliases=2 rejected=true backing_released=true"
            );
            0
        }
        "baseline" => {
            assert!(owner);
            g.base_frames = crate::PMM.lock().free_page_count();
            0
        }
        "arm" => {
            assert!(owner);
            let case = number(&mut words) as usize;
            let address = number(&mut words);
            let user_gate = number(&mut words);
            assert_eq!(case, g.verified);
            g.case = case % 17;
            g.epoch = case;
            g.actors = [EMPTY; 32];
            g.registered = [0; 32];
            g.completed = [0; 32];
            g.dispatch = [0; 32];
            g.wakes = 0;
            g.live_wakes = 0;
            g.cardinal_steps = 0;
            let hhdm = x86_64::VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
            let p = s.current_process();
            crate::memory::user::validate_process_write(p, hhdm, user_gate, 4).unwrap();
            assert_eq!(user_gate & 3, 0);
            let page = x86_64::structures::paging::Page::containing_address(x86_64::VirtAddr::new(
                user_gate,
            ));
            let (physical, _) = unsafe { p.address_space.lookup_entry(page, hhdm) }.unwrap();
            USER_GATE.store(
                hhdm.as_u64() + physical.as_u64() + (user_gate & 4095),
                Ordering::Release,
            );
            ENTERED.store(0, Ordering::Release);
            WAKE_DONE.store(0, Ordering::Release);
            WAITER_CPU.store(usize::MAX, Ordering::Release);
            WAKER_CPU.store(usize::MAX, Ordering::Release);
            POINT.store(
                if (1..=5).contains(&g.case) {
                    g.case - 1
                } else {
                    usize::MAX
                },
                Ordering::Release,
            );
            ADDRESS.store(address, Ordering::Release);
            crate::serial_println!(
                "[T4A] arm case={} address={:#x} point={}",
                case,
                address,
                POINT.load(Ordering::Acquire)
            );
            0
        }
        "ready-all" => {
            assert!(owner && g.case >= 15);
            u64::from(
                g.registered.iter().sum::<usize>() == 32
                    && (0..32).all(|r| {
                        let idx = g.slots[r];
                        g.actors[r].matches(&s.processes[idx])
                            && s.processes[idx].state == ProcessState::BlockedOnIo
                            && (s.processes[idx].native_futex_wait.is_some()
                                || s.processes[idx]
                                    .linux_state()
                                    .and_then(|state| state.futex_wait)
                                    .is_some())
                            && s.processes[idx].owning_core == u8::MAX
                            && !s.stack_is_pinned(idx)
                    }),
            )
        }
        "cardinality" => {
            assert!(owner && g.case == 15);
            let limit = number(&mut words);
            let actual = number(&mut words) as usize;
            let resumed = number(&mut words) as usize;
            let (want_limit, want_actual, total) = [
                (0, 0, 0),
                (1, 1, 1),
                (4, 4, 5),
                (
                    if g.epoch >= 17 {
                        i32::MAX as u64
                    } else {
                        u32::MAX as u64
                    },
                    27,
                    32,
                ),
                (1, 0, 32),
            ][g.cardinal_steps];
            assert_eq!((limit, actual), (want_limit, want_actual));
            assert_eq!(g.wakes, total);
            assert_eq!(g.completed.iter().sum::<usize>(), total);
            assert!(resumed <= total);
            let waiting = s
                .processes
                .iter()
                .filter(|p| {
                    p.address_space.identity() == g.owner.vm
                        && (p.native_futex_wait.is_some()
                            || p.linux_state().and_then(|state| state.futex_wait).is_some())
                })
                .count();
            assert_eq!(waiting, 32 - total);
            crate::serial_println!("[T4A] cardinality case={} step={} limit={} actual={} selected_total={} resumed={} remaining={} unique=true",g.epoch,g.cardinal_steps,limit,actual,total,resumed,waiting);
            g.cardinal_steps += 1;
            0
        }
        "ready" => {
            assert_eq!(role, Some(1));
            let count = if g.case == 7 { 2 } else { 1 };
            let quiescent = [0, 2].into_iter().take(count).all(|r| {
                let idx = g.slots[r];
                let p = &s.processes[idx];
                g.actors[r].pid != 0
                    && g.actors[r].matches(p)
                    && p.state == ProcessState::BlockedOnIo
                    && (p.native_futex_wait.is_some()
                        || p.linux_state().and_then(|state| state.futex_wait).is_some())
                    && p.owning_core == u8::MAX
                    && !s.stack_is_pinned(idx)
            });
            if g.case == 8 {
                u64::from(if g.epoch >= 17 {
                    true
                } else {
                    g.registered[0] == 1 && g.completed[0] == 1
                })
            } else {
                if quiescent && g.case == 14 {
                    // Existing affinity on a quiescent waiter. Normal wake and
                    // dispatch perform all queueing/ownership changes.
                    s.processes[g.slots[0]].cpu_mask = 1 << (g.registered[0] % 2);
                }
                u64::from(quiescent)
            }
        }
        "cancel" | "kill" => {
            assert_eq!(role, Some(1));
            let pid = g.actors[0].pid;
            drop(g);
            if operation == "kill" {
                assert!(s.terminate_process_by_pid(pid, 37, "t4a-waiting-exit"));
            } else {
                s.wake_for_signal(pid);
            }
            return Some(0);
        }
        "remaining" => {
            assert!(owner || role == Some(1));
            g.actors
                .iter()
                .enumerate()
                .filter(|(r, id)| {
                    id.pid != 0
                        && (s.processes[g.slots[*r]].native_futex_wait.is_some()
                            || s.processes[g.slots[*r]]
                                .linux_state()
                                .and_then(|state| state.futex_wait)
                                .is_some())
                })
                .count() as u64
        }
        "verify" => {
            assert!(owner);
            let actors = if g.case >= 15 {
                32
            } else if g.case == 7 {
                3
            } else {
                2
            };
            if g.actors.iter().filter(|id| id.pid != 0).count() != actors {
                return Some(0);
            }
            for r in 0..32 {
                if g.actors[r].pid == 0 {
                    continue;
                }
                let idx = g.slots[r];
                let p = &s.processes[idx];
                if p.state != ProcessState::Reaped {
                    return Some(0);
                }
                assert!(g.actors[r].matches(p));
                assert_eq!(p.exit_code, if g.case == 13 && r == 0 { 37 } else { 0 });
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
                assert_eq!(g.registered[r], g.completed[r]);
                assert_ne!(g.dispatch[r], 0);
            }
            if g.case == 15 {
                assert_eq!(g.cardinal_steps, 5);
            }
            let expected = match g.case {
                0 | 1 => 0,
                8 if g.epoch >= 17 => 0,
                7 => 2,
                14 => 128,
                15 | 16 => 32,
                _ => 1,
            };
            assert_eq!(g.registered.iter().sum::<usize>(), expected);
            if (2..=5).contains(&g.case) {
                assert_ne!(ENTERED.load(Ordering::Acquire), 0);
            }
            if g.case == 5 {
                assert_eq!(g.live_wakes, 1);
            }
            if (1..=5).contains(&g.case) {
                crate::serial_println!("[T4A] ordering case={} point={} waiter_cpu={} waker_cpu={} entered={} completed={}",g.epoch,g.case-1,WAITER_CPU.load(Ordering::Acquire),WAKER_CPU.load(Ordering::Acquire),ENTERED.load(Ordering::Acquire),WAKE_DONE.load(Ordering::Acquire));
            }
            if g.case == 14 {
                assert_eq!(g.dispatch[0], 3);
            }
            let heap = crate::memory::heap::heap_stats().allocated;
            let frames = crate::PMM.lock().free_page_count();
            if g.epoch == 0 {
                g.stable_heap = heap;
            }
            // The three-worker case retains one additional existing FD cache.
            let extra = (if g.epoch >= 15 {
                30
            } else if g.case >= 7 {
                1
            } else {
                0
            }) * core::mem::size_of::<crate::process::fd_table::FdTable>();
            assert_eq!(heap, g.stable_heap + extra);
            assert_eq!(frames, g.base_frames);
            for cpu in 0..s.online_cores {
                assert_eq!(
                    STACK_HANDOFFS.slot(cpu).load(Ordering::Acquire),
                    handoff::EMPTY
                );
                assert_eq!(STACK_HANDOFFS.waiter_snapshot(cpu), 0);
            }
            crate::serial_println!("[T4A] case={} registered={} completed={} wakes={} live_wakes={} dispatch0={:#x} heap={} frames={} stale=0 pins=0 queues=0 ownership=ok",g.epoch,expected,g.completed.iter().sum::<usize>(),g.wakes,g.live_wakes,g.dispatch[0],heap,frames);
            g.verified += 1;
            ADDRESS.store(0, Ordering::Release);
            POINT.store(usize::MAX, Ordering::Release);
            1
        }
        "finish" => {
            assert!(owner);
            assert_eq!(g.verified, 34);
            assert_eq!(crate::process::fd_table::thread_description_refs(), 3);
            crate::serial_println!(
                "[T4A] PASS cpus={} cases=34 waiter_records=0 resources=stable",
                s.online_cores
            );
            unsafe {
                core::arch::asm!("out dx,eax",in("dx")0xf4u16,in("eax")0x10u32);
            }
            1
        }
        "fail" => panic!("T4A userspace failure case={} role={:?}", g.case, role),
        _ => panic!("unknown T4A command"),
    })
}
pub(crate) fn run(hhdm: x86_64::VirtAddr) {
    let cpus = ONLINE_CORES.load(Ordering::Acquire);
    assert!(cpus >= 2);
    init_cores(cpus);
    let mut pmm = crate::PMM.lock();
    let mut p = unsafe { Process::new(0xe400, 0, "t4a-owner", &mut pmm, hhdm) };
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
        "[T4A] begin cpus={} owner={} inc={} vm={:?} heap={} frames={}",
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
