//! Isolated post-scheduler fixture. Real ring-3 tasks invoke bounded kernel
//! steps and yield using the existing native scheduler. No filesystem mount.
use crate::process::{layout, Process};
use crate::sfs_reset::{Coordinator, Phase, Probe};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use sunlight_block::contract::{Completion, RequestId};
use sunlight_virtio::VirtioBlk;
use x86_64::VirtAddr;

#[cfg(any(
    feature = "sfs5b1-test",
    feature = "sfs5b2-test",
    feature = "sfs5b3-test"
))]
compile_error!("SFS5B4 requires an isolated storage fixture");

static DEVICE: spin::Once<Coordinator> = spin::Once::new();
static STATUS_ADDRESS: AtomicU64 = AtomicU64::new(0);
static FETCHED: AtomicBool = AtomicBool::new(false);
static REQUEST: spin::Mutex<Option<RequestId>> = spin::Mutex::new(None);
static TICKS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static WORK: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static LOGICAL: AtomicU64 = AtomicU64::new(0);
static SUBMIT_TICK: AtomicU64 = AtomicU64::new(0);
static TIMEOUT: AtomicBool = AtomicBool::new(false);
static CONTESTED: AtomicBool = AtomicBool::new(false);
static CLOSED: AtomicBool = AtomicBool::new(false);
static TERMINAL: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);
static FINALIZING: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicU64 = AtomicU64::new(0);
static START: spin::Mutex<([u64; 4], [u64; 4])> = spin::Mutex::new(([0; 4], [0; 4]));
static RESET_START: spin::Mutex<([u64; 4], [u64; 4])> = spin::Mutex::new(([0; 4], [0; 4]));
static FREE: AtomicU64 = AtomicU64::new(0);
const PID: usize = 0x5b400;

pub fn on_tick(cpu: usize) {
    if let Some(t) = TICKS.get(cpu) {
        t.fetch_add(1, Ordering::Relaxed);
    }
}
fn ticks() -> [u64; 4] {
    core::array::from_fn(|i| TICKS[i].load(Ordering::Acquire))
}
fn work() -> [u64; 4] {
    core::array::from_fn(|i| WORK[i].load(Ordering::Acquire))
}
fn event(role: u64, actor: u64, stage: &str) {
    let seq = LOGICAL.fetch_add(1, Ordering::AcqRel) + 1;
    crate::serial_println!(
        "[SFS5B4] event={} cpu={} pid={} task-generation={} role={} scheduler-ready={} stage={}",
        seq,
        crate::sched::current_cpu_id(),
        PID + role as usize,
        actor,
        role,
        crate::sched::SCHEDULER_READY.load(Ordering::Acquire),
        stage
    );
}
fn load(hhdm: VirtAddr, role: u64, cpu: usize) -> Process {
    let elf = include_bytes!(concat!(env!("OUT_DIR"), "/sfs5b4-probe.elf"));
    let mut pmm = crate::PMM.lock();
    let mut p = unsafe { Process::new(PID + role as usize, 0, "sfs5b4-worker", &mut pmm, hhdm) };
    let entry = crate::process::elf_loader::load_elf(elf, &mut p, &mut pmm, hhdm).unwrap();
    crate::process::spawn::map_user_stack(&mut p, &mut pmm, hhdm).unwrap();
    p.init_context(entry, layout::USER_STACK_TOP);
    p.set_initial_args(role, 0, 0, 0);
    p.cpu_mask = 1 << cpu;
    p
}

pub fn run(mut d: VirtioBlk, queue: u64, request: u64) -> VirtioBlk {
    assert_eq!(d.block_count(), 8192);
    let mut marker = [0; 512];
    assert_eq!(
        unsafe { d.read_typed(0, &mut marker) }.completion,
        Completion::Completed
    );
    assert_eq!(&marker[..34], b"SFS5A-ONLY-7b93143a-disposable-v1!");
    // Normal I/O baseline before the held sector; flush/readback remain real.
    assert_eq!(
        unsafe { d.write_typed(62, &[0x62; 512]) }.completion,
        Completion::Completed
    );
    assert_eq!(unsafe { d.flush_typed() }.completion, Completion::Completed);
    assert_eq!(
        unsafe { d.read_typed(62, &mut marker) }.completion,
        Completion::Completed
    );
    assert_eq!(marker, [0x62; 512]);
    assert_eq!(
        unsafe { d.read_typed(61, &mut marker) }.completion,
        Completion::Completed
    );
    CANCEL.store(marker[0] == 1, Ordering::Release);
    crate::serial_println!(
        "[SFS5B4] BASELINE features={:?} queue={:#x} request={:#x} pages=5",
        d.features(),
        queue,
        request
    );
    STATUS_ADDRESS.store(
        crate::HHDM_REQ.response().unwrap().offset + request + 528,
        Ordering::Release,
    );
    DEVICE.call_once(|| Coordinator::new(d));
    unsafe {
        crate::syscall::setup_syscall_msrs(VirtAddr::new(
            crate::syscall::syscall_entry as *const () as u64,
        ));
    }
    crate::memory::tlb::register_kernel_root();
    let mp = crate::MP_REQ.response().expect("MP response");
    let cpus = mp.cpus().len();
    assert!((1..=4).contains(&cpus));
    crate::arch::x86_64::smp::start_aps(mp.cpus(), mp.bsp_lapic_id);
    crate::sched::init_cores(cpus);
    let hhdm = VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
    for (role, cpu) in [
        (0, 0),
        (1, usize::from(cpus > 1)),
        (2, if cpus == 4 { 2 } else { 0 }),
    ] {
        let p = load(hhdm, role, cpu);
        let mut scheduler = crate::sched::SCHEDULER.lock();
        let index = scheduler.add_process_after_reaping(p);
        scheduler.enqueue_ready(index);
    }
    for cpu in 0..cpus {
        let p = load(hhdm, 10 + cpu as u64, cpu);
        let mut scheduler = crate::sched::SCHEDULER.lock();
        let index = scheduler.add_process_after_reaping(p);
        scheduler.enqueue_ready(index);
    }
    FREE.store(
        crate::PMM.lock().free_page_count() as u64,
        Ordering::Release,
    );
    crate::sched::mark_scheduler_ready();
    crate::sched::flush_dispatch_notifications();
    crate::sched::core_idle_entry()
}

pub fn syscall(role: u64) -> u64 {
    assert!(
        CALLS.fetch_add(1, Ordering::Relaxed) < 2_000_000,
        "SFS5B4 work budget exhausted"
    );
    let cpu = crate::sched::current_cpu_id();
    let (actor, state) = {
        let s = crate::sched::SCHEDULER.lock();
        let p = s.current_process();
        assert_eq!(p.pid, PID + role as usize);
        (p.foreground.incarnation, p.state)
    };
    assert!(crate::sched::SCHEDULER_READY.load(Ordering::Acquire));
    if READY.load(Ordering::Acquire) {
        return 1;
    }
    let c = DEVICE.get().unwrap();
    if role >= 10 {
        assert_eq!(role as usize - 10, cpu);
        let count = WORK[cpu].fetch_add(1, Ordering::AcqRel);
        if count == 0 {
            event(role, actor, "INDEPENDENT_WORKER_RUNNING");
        }
        if TERMINAL.load(Ordering::Acquire)
            && CLOSED.load(Ordering::Acquire)
            && CONTESTED.load(Ordering::Acquire)
        {
            let start = *START.lock();
            let cpus = crate::sched::ONLINE_CORES.load(Ordering::Acquire);
            if (0..cpus).all(|i| ticks()[i] > start.0[i] + 3 && work()[i] > start.1[i] + 3)
                && FINALIZING
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                event(role, actor, "FORWARD_PROGRESS");
                crate::serial_println!("[SFS5B4] READY cpus={} state={:?} start-ticks={:?} ticks={:?} start-work={:?} work={:?} free-before={} free-after={} pages-pinned=5 released=0 new-epoch=none inventory={:?}",
                    cpus, state, start.0, ticks(), start.1, work(), FREE.load(Ordering::Acquire), crate::PMM.lock().free_page_count(), c.inventory());
                READY.store(true, Ordering::Release);
            }
        }
        return 0;
    }
    if role == 0 {
        let id = *REQUEST.lock();
        if let Some(id) = id {
            if TIMEOUT.load(Ordering::Acquire) {
                if matches!(unsafe { c.start_write(64, &[0xcc; 512]) }, Probe::Closed)
                    && !CLOSED.swap(true, Ordering::AcqRel)
                {
                    event(role, actor, "ADMISSION_CLOSED");
                }
            } else {
                // QEMU independently publishes this byte only after its actual
                // payload copy. It does not advance used.idx or claim completion.
                if unsafe { (STATUS_ADDRESS.load(Ordering::Acquire) as *const u8).read_volatile() }
                    == 0xfe
                    && !FETCHED.swap(true, Ordering::AcqRel)
                {
                    event(role, actor, "DEVICE_ACCESS_HANDSHAKE");
                }
                match unsafe { c.poll_write(id) } {
                    Probe::Value(Some(outcome)) => {
                        event(role, actor, "NORMAL_COMPLETION");
                        assert_eq!(outcome.completion, Completion::Completed);
                        // Normal release controls still exercise idle terminal reset.
                        *START.lock() = (ticks(), work());
                        TIMEOUT.store(true, Ordering::Release);
                    }
                    Probe::Value(None)
                        if FETCHED.load(Ordering::Acquire)
                            && ticks()[0] > SUBMIT_TICK.load(Ordering::Acquire) + 20 =>
                    {
                        let Probe::Value(Some(outcome)) = c.timeout_write(id) else {
                            panic!("timeout ownership");
                        };
                        assert_eq!(outcome.completion, Completion::Uncertain);
                        event(role, actor, "TIMEOUT_QUARANTINE");
                        crate::serial_println!("[SFS5B4] uncertainty={:?}", outcome);
                        *START.lock() = (ticks(), work());
                        TIMEOUT.store(true, Ordering::Release);
                    }
                    _ => {}
                }
            }
        } else {
            event(role, actor, "SUBMISSION_INITIATED");
            if let Probe::Value(result) = unsafe { c.start_write(63, &[0xb4; 512]) } {
                let id = result.unwrap();
                *REQUEST.lock() = Some(id);
                SUBMIT_TICK.store(ticks()[0], Ordering::Release);
                event(role, actor, "SUBMITTED");
                crate::serial_println!("[SFS5B4] request={:?} state={:?}", id, state);
            }
        }
        return 0;
    }
    if TERMINAL.load(Ordering::Acquire) {
        return 0;
    }
    if TIMEOUT.load(Ordering::Acquire) {
        let ticket = c.ticket(actor);
        match c.request(ticket) {
            Probe::Closed => {
                if !CONTESTED.swap(true, Ordering::AcqRel) {
                    event(role, actor, "RESET_OWNER_CONTESTED");
                }
            }
            Probe::Value(()) => {
                if !CONTESTED.load(Ordering::Acquire) {
                    return 0;
                }
                let at_owner = matches!(
                    c.inventory(),
                    Probe::Value((Phase::ResetOwnerEstablished, _, _))
                );
                if at_owner {
                    let start = *RESET_START.lock();
                    let cpus = crate::sched::ONLINE_CORES.load(Ordering::Acquire);
                    if !(0..cpus).all(|i| ticks()[i] > start.0[i] + 3 && work()[i] > start.1[i] + 3)
                    {
                        return 0; // yield in ring 3, holding no coordinator/scheduler lock
                    }
                    event(role, actor, "CONTENTION_PROGRESS");
                    crate::serial_println!("[SFS5B4] contention-start-ticks={:?} ticks={:?} contention-start-work={:?} work={:?} inventory={:?}", start.0, ticks(), start.1, work(), c.inventory());
                    if !CANCEL.load(Ordering::Acquire) {
                        event(role, actor, "RESET_REQUESTED");
                    }
                }
                let step = if CANCEL.load(Ordering::Acquire) && at_owner {
                    c.cancel(ticket)
                } else {
                    unsafe { c.step(ticket) }
                };
                match step {
                    Probe::Value(phase) => {
                        if phase == Phase::ResetOwnerEstablished {
                            *RESET_START.lock() = (ticks(), work());
                        }
                        event(
                            role,
                            actor,
                            match phase {
                                Phase::Quiescing => "QUIESCING",
                                Phase::ResetOwnerEstablished => "RESET_OWNER_ESTABLISHED",
                                Phase::ResetPending => "RESET_PENDING",
                                Phase::ResetAcknowledged => "RESET_ACKNOWLEDGED",
                                Phase::FenceUnqualified => "FENCE_UNQUALIFIED",
                                Phase::TerminalQuarantine => "TERMINAL_QUARANTINE",
                                Phase::Running => "RUNNING",
                            },
                        );
                        if phase == Phase::TerminalQuarantine {
                            TERMINAL.store(true, Ordering::Release);
                        }
                    }
                    Probe::Busy => event(role, actor, "DEVICE_TRY_LOCK_BUSY"),
                    _ => panic!("reset ticket lost"),
                }
            }
            _ => panic!("reset request invalid"),
        }
    }
    0
}
