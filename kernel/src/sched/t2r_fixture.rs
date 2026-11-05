//! Opt-in native pilot. Entry rendezvous is bounded and holds no lifecycle lock.
//! Registered full identities authorize receipts. No ordinary-build selectors.
use super::*;
use crate::process::{layout, mm2b_state::AddressSpaceIdentity, thread_admission::Error};
use core::sync::atomic::{AtomicUsize, Ordering};

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
    owners: [Identity; 4],
    actors: [Identity; 2],
    ready: usize,
    results: usize,
    successes: usize,
    commits: usize,
    rejects: usize,
    done: usize,
    progress: usize,
    child: Identity,
    child_slot: usize,
    base_heap: usize,
    finished: bool,
}
static GATE: spin::Mutex<Gate> = spin::Mutex::new(Gate {
    owners: [EMPTY; 4],
    actors: [EMPTY; 2],
    ready: 0,
    results: 0,
    successes: 0,
    commits: 0,
    rejects: 0,
    done: 0,
    progress: 0,
    child: EMPTY,
    child_slot: 0,
    base_heap: 0,
    finished: false,
});
static COMMITTED: AtomicUsize = AtomicUsize::new(0);
static ENTERED: AtomicUsize = AtomicUsize::new(0);
static CPUS: [AtomicUsize; 2] = [AtomicUsize::new(usize::MAX), AtomicUsize::new(usize::MAX)];
fn winner() -> usize {
    usize::from(option_env!("SUNLIGHT_T2R_WINNER") == Some("1"))
}
fn case() -> usize {
    match option_env!("SUNLIGHT_T2R_CASE") {
        Some("vm") => 1,
        Some("immediate") => 2,
        _ => 0,
    }
}
fn workers(s: &Scheduler) -> usize {
    s.processes
        .iter()
        .filter(|p| !p.owns_address_space && p.state != ProcessState::Reaped)
        .count()
}
fn resources(s: &Scheduler, label: &str) {
    let live = s
        .processes
        .iter()
        .filter(|p| p.state != ProcessState::Reaped)
        .count();
    let stacks = s
        .processes
        .iter()
        .filter(|p| p.kernel_stack.is_some())
        .count();
    let refs = crate::process::fd_table::thread_description_refs();
    let cached = s
        .processes
        .iter()
        .filter(|p| p.state == ProcessState::Reaped)
        .count();
    let base_heap = GATE.lock().base_heap;
    let cache_bytes = cached * core::mem::size_of::<crate::process::fd_table::FdTable>();
    assert_eq!(
        crate::memory::heap::heap_stats().allocated,
        base_heap + cache_bytes,
        "T2R unattributed heap bytes"
    );
    assert_eq!(stacks, 4);
    assert_eq!(refs, 12);
    assert_eq!(
        s.processes
            .iter()
            .map(|p| p.capabilities.len())
            .sum::<usize>(),
        4
    );
    crate::serial_println!(
        "[T2R] heap baseline={} cached_fd_boxes={} cached_fd_bytes={} reaped_stacks={} drift=0",
        base_heap,
        cached,
        cache_bytes,
        cached
    );

    crate::serial_println!("[T2R] resources point={} workers={} occupied={} slots={} stacks={} fd_refs={} cap_entries={} heap={} reserved=0 pending=0", label, workers(s),live,s.processes.len(),stacks,refs,s.processes.iter().map(|p|p.capabilities.len()).sum::<usize>(),crate::memory::heap::heap_stats().allocated);
    for owner in GATE.lock().owners {
        let n = s
            .processes
            .iter()
            .filter(|p| {
                !p.owns_address_space
                    && p.state != ProcessState::Reaped
                    && p.address_space.identity() == owner.vm
            })
            .count();
        crate::serial_println!(
            "[T2R] vm owner={} incarnation={} identity={:?} borrowers={}",
            owner.pid,
            owner.incarnation,
            owner.vm,
            n
        );
    }
    for (i, p) in s.processes.iter().enumerate() {
        assert!(s.diagnostic_ready_occurrences(i) <= 1);
        if p.state == ProcessState::Reaped {
            assert!(p.kernel_stack.is_none());
            assert!(!s.stack_is_pinned(i));
        }
    }
}
/// Returns only after a bounded outside-lock rendezvous. Same-CPU scheduling
/// cannot trap the kernel: lack of two physical entrants is a fixture failure.
pub(crate) fn enter() {
    if case() == 2 {
        return;
    }
    let actor = {
        let s = SCHEDULER.lock();
        let g = GATE.lock();
        if g.ready != 15 || g.results != 0 {
            return;
        }
        g.actors
            .iter()
            .position(|id| id.matches(s.current_process()))
    };
    let Some(actor) = actor else { return };
    let cpu = current_cpu_id();
    CPUS[actor].store(cpu, Ordering::Release);
    ENTERED.fetch_or(1 << actor, Ordering::AcqRel);
    let mut overlap = false;
    for _ in 0..2_000_000 {
        if ENTERED.load(Ordering::Acquire) == 3 {
            overlap = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !overlap {
        let s = SCHEDULER.lock();
        let g = GATE.lock();
        let other = g.actors[1 - actor];
        if let Some((idx, p)) = s
            .processes
            .iter()
            .enumerate()
            .find(|(_, p)| other.matches(p))
        {
            crate::serial_println!("[T2R] rendezvous-timeout actor={} cpu={} other_pid={} incarnation={} state={:?} owning_core={} queued_on={} affinity={} pinned={} tick={}",actor,cpu,p.pid,p.foreground.incarnation,p.state,p.owning_core,p.queued_on_core,p.cpu_mask,s.stack_is_pinned(idx),s.global_tick);
        }
    }
    assert!(overlap, "T2R pilot no second concurrent entrant (bounded)");
    assert_ne!(
        CPUS[0].load(Ordering::Acquire),
        CPUS[1].load(Ordering::Acquire),
        "T2R entrants must occupy different CPUs"
    );
    crate::serial_println!(
        "[T2R] entry actor={} cpu={} mask=3 outside_lifecycle_lock=1",
        actor,
        cpu
    );
    if actor != winner() {
        let mut committed = false;
        for _ in 0..2_000_000 {
            if COMMITTED.load(Ordering::Acquire) == 1 {
                committed = true;
                break;
            }
            core::hint::spin_loop();
        }
        assert!(committed, "T2R bounded winner did not commit");
    }
}
/// Fixture placement is established at birth: changing affinity on a local
/// queue immediately before a rendezvous can strand the remote actor there.
pub(crate) fn placement(s: &Scheduler, child: &mut Process, arg: u64, cpu: usize) -> usize {
    if case() == 1
        && s.online_cores > 1
        && (arg == 29 || arg == 30)
        && GATE.lock().owners[0].matches(s.current_process())
    {
        let target = (arg - 29) as usize;
        child.cpu_mask = 1 << target;
        target
    } else {
        cpu
    }
}
pub(crate) fn rejected(s: &Scheduler, error: &Error) {
    let mut g = GATE.lock();
    if let Some(a) = g
        .actors
        .iter()
        .position(|id| id.matches(s.current_process()))
    {
        g.rejects += 1;
        crate::serial_println!(
            "[T2R] reject actor={} pid={} incarnation={} error={:?} workers={} reservation=none",
            a,
            s.current_process().pid,
            s.current_process().foreground.incarnation,
            error,
            workers(s)
        );
    }
}
pub(crate) fn committed(s: &Scheduler, idx: usize) {
    let mut g = GATE.lock();
    if let Some(a) = g
        .actors
        .iter()
        .position(|id| id.matches(s.current_process()))
    {
        g.commits += 1;
        let p = &s.processes[idx];
        g.child = Identity::of(p);
        g.child_slot = idx;
        crate::serial_println!("[T2R] commit actor={} parent={} tid={} incarnation={} slot={} vm={:?} stack={} queued={} workers={}",a,s.current_process().pid,p.pid,p.foreground.incarnation,idx,p.address_space.identity(),p.kernel_stack.is_some(),s.diagnostic_ready_occurrences(idx),workers(s));
        assert!(p.kernel_stack.is_some());
        assert_eq!(s.diagnostic_ready_occurrences(idx), 1);
        COMMITTED.store(1, Ordering::Release);
    }
}
pub(crate) fn query(command: &str) -> Option<u64> {
    if !command.starts_with("T2R ") {
        return None;
    }
    let mut s = SCHEDULER.lock();
    let p = s.current_process();
    let mut g = GATE.lock();
    let owner = g.owners.iter().position(|id| id.matches(p));
    let vm = g
        .owners
        .iter()
        .position(|id| id.vm == p.address_space.identity());
    let Some(role) = vm else {
        return Some(u64::MAX);
    };
    Some(match command {
        "T2R child-exit" => {
            assert_eq!(case(), 2);
            assert!(g.child.matches(p));
            crate::serial_println!(
                "[T2R] child-exit tid={} incarnation={} cpu={} stack_live=true",
                p.pid,
                p.foreground.incarnation,
                current_cpu_id()
            );
            assert!(p.kernel_stack.is_some());
            0
        }
        "T2R case" => case() as u64,
        "T2R ready" => {
            assert!(owner.is_some());
            g.ready |= 1 << role;
            0
        }
        "T2R actor" => {
            let a = if case() == 0 {
                assert!(role >= 2 && owner.is_some());
                role - 2
            } else if case() == 1 {
                assert_eq!(role, 0);
                // Birth placement fixes the actor's CPU. Registration arrival
                // order must not exchange their affinities and strand queues.
                current_cpu_id()
            } else {
                assert_eq!(role, 0);
                0
            };
            assert_eq!(g.actors[a].pid, 0);
            g.actors[a] = Identity::of(p);
            crate::serial_println!(
                "[T2R] registered actor={} pid={} incarnation={} vm={:?}",
                a,
                p.pid,
                p.foreground.incarnation,
                p.address_space.identity()
            );
            if case() != 2 && s.online_cores > 1 {
                s.current_process_mut().cpu_mask = 1 << a;
                request_park_after_sysret();
            }
            0
        }
        "T2R go" => {
            u64::from(g.ready == 15 && (case() == 2 || g.actors.iter().all(|a| a.pid != 0)))
        }
        "T2R accepted" | "T2R rejected" => {
            let a = g
                .actors
                .iter()
                .position(|id| id.matches(p))
                .expect("registered actor");
            assert_eq!(g.results & (1 << a), 0);
            g.results |= 1 << a;
            if command == "T2R accepted" {
                g.successes += 1;
            }
            if g.results == if case() == 2 { 1 } else { 3 } {
                assert_eq!(g.successes, 1);
                assert_eq!(g.commits, 1);
                assert_eq!(g.rejects, usize::from(case() != 2));
                crate::serial_println!(
                    "[T2R] outcome case={} successes={} commits={} rejects={} entry_mask={}",
                    case(),
                    g.successes,
                    g.commits,
                    g.rejects,
                    ENTERED.load(Ordering::Acquire)
                );
            }
            0
        }
        "T2R release" => u64::from(g.results == if case() == 2 { 1 } else { 3 }),
        "T2R finish" => {
            assert!(owner.is_some());
            if ((case() == 0 && role < 2) || (case() != 0 && role > 0)) && g.ready == 15 {
                g.progress += 1;
            }
            if workers(&s) != 0 || g.results != if case() == 2 { 1 } else { 3 } {
                0
            } else {
                g.done |= 1 << role;
                if g.done == 15 && !g.finished {
                    g.finished = true;
                    let child = &s.processes[g.child_slot];
                    assert!(g.child.matches(child));
                    assert_eq!(child.state, ProcessState::Reaped);
                    assert!(child.kernel_stack.is_none());
                    assert!(!s.stack_is_pinned(g.child_slot));
                    assert!(g.progress > 0, "unrelated owners made no progress");
                    crate::serial_println!("[T2R] retired tid={} incarnation={} slot={} state=Reaped stack=none pinned=false progress={}",child.pid,child.foreground.incarnation,g.child_slot,g.progress);
                    drop(g);
                    resources(&s, "drained");
                    crate::serial_println!(
                        "[T2R] PILOT PASS case={} cpus={}",
                        case(),
                        s.online_cores
                    );
                    unsafe {
                        core::arch::asm!("out dx, eax",in("dx")0xf4u16,in("eax")0x10u32);
                    }
                }
                0
            }
        }
        "T2R fail" => panic!("T2R ring3 failed role={}", role),
        _ => u64::MAX,
    })
}
pub(crate) fn run(hhdm: x86_64::VirtAddr) {
    let cpus = ONLINE_CORES.load(Ordering::Acquire);
    init_cores(cpus);
    for role in 0..4 {
        let mut pmm = crate::PMM.lock();
        let mut p = unsafe { Process::new(0xE000 + role, 0, "t2r-owner", &mut pmm, hhdm) };
        let entry = crate::process::elf_loader::load_elf(
            include_bytes!(concat!(env!("OUT_DIR"), "/t2r.elf")),
            &mut p,
            &mut pmm,
            hhdm,
        )
        .unwrap();
        crate::process::spawn::map_user_stack(&mut p, &mut pmm, hhdm).unwrap();
        p.env.set("T2R", "inherited");
        let (endpoint, token) = crate::capability::CAP_BROKER
            .lock()
            .create_endpoint(p.pid)
            .unwrap();
        p.capabilities.push(crate::process::Capability {
            token: token.as_u64(),
            endpoint_id: endpoint,
            can_send: true,
            can_recv: true,
            can_grant: false,
        });
        if case() == 0 && role >= 2 && cpus > 1 {
            p.cpu_mask = 1 << (role - 2);
        }
        p.init_context(entry, layout::USER_STACK_TOP);
        p.set_initial_args(role as u64, 0, 0, 0);
        drop(pmm);
        let mut s = SCHEDULER.lock();
        let idx = s.add_process_after_reaping(p);
        GATE.lock().owners[role] = Identity::of(&s.processes[idx]);
    }
    {
        let mut s = SCHEDULER.lock();
        s.reserve_task_storage()
            .expect("T2R bounded storage warmup");
        GATE.lock().base_heap = crate::memory::heap::heap_stats().allocated;
        crate::serial_println!("[T2R] baseline heap={} workers=0 stacks=4 fd_refs=12 cap_entries=4 reserved=0 pending=0",GATE.lock().base_heap);
    }
    crate::serial_println!("[T2R] requested-winner actor={}", winner());
    crate::serial_println!(
        "[T2R] begin case={} cpus={} bounded_entry_spins=2000000",
        case(),
        cpus
    );
    enter_first_process();
}
