//! Isolated opt-in ring-3 issuance/authorization fixture. No normal service
//! migration and no deterministic entropy override on the global broker.
use super::{CapError, CapabilityBroker, CapabilityRights};
use crate::memory::accounting::{class_frame_count, PhysicalMemoryClass};
use crate::process::{Process, ProcessState};
use crate::sched::{Scheduler, ONLINE_CORES, SCHEDULER};
use core::sync::atomic::Ordering;
use x86_64::VirtAddr;

const ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/capability-probe.elf"));
pub(super) static AUTH_COMMITS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
pub(super) static ACCOUNT_COMMITS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

const OWNER: usize = 0xF200;
const READER: usize = OWNER + 1;

struct State {
    baseline: usize,
    data: u64,
    tables: u64,
    workers: usize,
    complete: bool,
    signal_address: u64,
    signalled: bool,
}
static STATE: spin::Mutex<State> = spin::Mutex::new(State {
    baseline: 0,
    data: 0,
    tables: 0,
    workers: 0,
    complete: false,
    signal_address: 0,
    signalled: false,
});

fn private(pid: usize, hhdm: VirtAddr) -> Process {
    let mut pmm = crate::PMM.lock();
    let mut process = unsafe { Process::new(pid, 0, "cap-probe", &mut pmm, hhdm) };
    let entry = crate::process::elf_loader::load_elf(ELF, &mut process, &mut pmm, hhdm)
        .expect("capability fixture ELF");
    crate::process::spawn::map_user_stack(&mut process, &mut pmm, hhdm).unwrap();
    process.init_context(entry, crate::process::layout::USER_STACK_TOP);
    process
}

fn timestamp() -> u64 {
    unsafe {
        core::arch::x86_64::_mm_lfence();
        let value = core::arch::x86_64::_rdtsc();
        core::arch::x86_64::_mm_lfence();
        value
    }
}

fn issuance_samples() {
    let mut caps = CapabilityBroker::new();
    let mut create = [0u64; 128];
    let mut mint = [0u64; 128];
    for index in 0..128 {
        let started = timestamp();
        let (ep, owner) = caps.create_endpoint(1).unwrap();
        create[index] = timestamp().saturating_sub(started);
        let started = timestamp();
        let token = caps.mint(ep, CapabilityRights::SEND).unwrap();
        mint[index] = timestamp().saturating_sub(started);
        caps.revoke(token);
        caps.destroy_endpoint(1, owner).unwrap();
    }
    create.sort_unstable();
    mint.sort_unstable();
    for (name, values) in [("endpoint_create_including_uart", create), ("mint", mint)] {
        crate::serial_println!("[CAP-P] issuance samples={} count=128 units=guest_tsc p50={} p95={} p99={} max={} collisions={} scope=local_broker_no_contention", name, values[63], values[121], values[126], values[127], caps.issuance_stats().collisions);
    }
}

pub fn run(hhdm: VirtAddr) {
    assert!(crate::entropy::is_ready());
    let cpus = ONLINE_CORES.load(Ordering::Acquire);
    issuance_samples();
    {
        let mut pmm = crate::PMM.lock();
        crate::memory::shared_authority_gate::run(&mut pmm, hhdm);
        // Exercise actual SHM allocation with instance-local failure injection.
        // A failed mint happens before backing/PTE allocation, leaves no token
        // or owner view, and does not alter the global entropy stream.
        let mut process = unsafe { Process::new(0xF1FF, 0, "cap-fail", &mut pmm, hhdm) };
        let mut caps = CapabilityBroker::new();
        let free = pmm.free_page_count();
        for reservation in [false, true] {
            caps.test_entropy(&[None]);
            caps.test_fail_reservation(reservation);
            assert!(crate::memory::shared::alloc_shared_region(
                &mut process,
                &mut pmm,
                &mut caps,
                hhdm,
                8192
            )
            .is_err());
            assert_eq!(pmm.free_page_count(), free);
            assert_eq!(caps.shared_region_count(), 0);
            assert!(process.mapped_shared.is_empty() && process.owned_shared.is_empty());
            assert_eq!(process.address_space.region_count(), 0);
        }
        unsafe {
            process
                .address_space
                .reclaim_user_space(&mut pmm, hhdm, true);
        }
        let mut caps = CapabilityBroker::new();
        caps.test_entropy(&[Some(9), Some(9), Some(11)]);
        let (ep, first) = caps.create_endpoint(1).unwrap();
        let (_, second) = caps.create_endpoint(2).unwrap();
        assert_ne!(first, second);
        assert_eq!(caps.check(first, CapabilityRights::SEND), Ok(ep));
        caps.test_entropy(&[Some(9); 16]);
        assert_eq!(caps.create_endpoint(3), Err(CapError::TokenCollisionLimit));
        assert_eq!(caps.issuance_stats().collisions, 17);
        crate::serial_println!("[CAP-P] local fault injection: entropy/reservation/SHM rollback/collision/retry bound OK");
    }
    let workers = 2 * cpus;
    {
        let mut state = STATE.lock();
        state.baseline = crate::PMM.lock().free_page_count();
        state.data = class_frame_count(PhysicalMemoryClass::SharedMemory);
        state.tables = class_frame_count(PhysicalMemoryClass::PageTable);
        state.workers = workers;
    }
    let mut owner = private(OWNER, hhdm);
    owner.trusted_auth_broker = true;
    let mut reader = private(READER, hhdm);
    let (base, shm) = {
        let mut pmm = crate::PMM.lock();
        let mut caps = super::CAP_BROKER.lock();
        crate::memory::shared::alloc_shared_region(&mut owner, &mut pmm, &mut caps, hhdm, 4096)
            .unwrap()
    };
    let page = x86_64::structures::paging::Page::from_start_address(base).unwrap();
    let (physical, _) = unsafe { owner.address_space.lookup_entry(page, hhdm) }.unwrap();
    STATE.lock().signal_address = (hhdm + physical.as_u64()).as_u64();
    unsafe {
        (hhdm + physical.as_u64())
            .as_mut_ptr::<u64>()
            .write_volatile(0x51a7);
    }
    let (owner_cap, send) = {
        let mut caps = super::CAP_BROKER.lock();
        let (_, owner_cap) = caps.create_endpoint(OWNER).unwrap();
        let send = caps.derive(owner_cap, CapabilityRights::SEND).unwrap();
        (owner_cap, send)
    };
    owner.set_initial_args(0, base.as_u64(), owner_cap.as_u64(), READER as u64);
    reader.set_initial_args(2, shm.as_u64(), send.as_u64(),
        u64::from(option_env!("SUNLIGHT_CAP_EXEC_PROBE") == Some("1")));
    {
        let mut sched = SCHEDULER.lock();
        sched.add_process_after_reaping(owner);
        sched.add_process_after_reaping(reader);
    }
    for n in 0..workers {
        let mut attacker = private(READER + 1 + n, hhdm);
        attacker.cpu_mask = 1u64 << (n % cpus);
        attacker.set_initial_args(1, 0, 0, 0);
        SCHEDULER.lock().add_process_after_reaping(attacker);
    }
    crate::serial_println!("[CAP-P] native begin cpus={} workers={} predictions_per_worker=124 issuance_cycles_per_worker=128", cpus, workers);
    crate::sched::enter_first_process()
}

pub fn on_tick(sched: &mut Scheduler, cpu: usize) {
    if cpu != 0 {
        return;
    }
    let mut state = STATE.lock();
    if state.workers == 0 || state.complete {
        return;
    }
    for p in &sched.processes {
        assert_ne!(
            p.exit_code, 99,
            "native capability fixture rejected security invariant pid={}",
            p.pid
        );
        if p.pid != OWNER && matches!(p.state, ProcessState::Finished | ProcessState::Reaped) {
            assert_eq!(
                p.exit_code, 0,
                "unexpected native fault or failed fixture pid={}",
                p.pid
            );
        }
    }
    if sched
        .processes
        .iter()
        .filter(|p| p.pid != OWNER)
        .all(|p| p.state == ProcessState::Reaped)
    {
        if !state.signalled {
            let caps = super::CAP_BROKER.lock();
            assert!(caps.auth_session_grants.is_empty() && caps.account_grants.is_empty());
            // The reader is fully Reaped. Tell the still-live issuer to attempt
            // both syscalls again against that retired recipient.
            unsafe { (state.signal_address as *mut u64).write_volatile(0xc1ea); }
            state.signalled = true;
        }
    }

    if sched
        .processes
        .iter()
        .all(|p| p.state == ProcessState::Reaped)
    {
        let free = crate::PMM.lock().free_page_count();
        let caps = super::CAP_BROKER.lock();
        assert_eq!(caps.active_tokens(), 0);
        assert_eq!(caps.shared_region_count(), 0);
        assert!(sched.processes.iter().all(|p| p.mapped_shared.is_empty()));
        assert_eq!(free, state.baseline);
        assert_eq!(
            class_frame_count(PhysicalMemoryClass::SharedMemory),
            state.data
        );
        assert_eq!(
            class_frame_count(PhysicalMemoryClass::PageTable),
            state.tables
        );
        assert_eq!(AUTH_COMMITS.load(Ordering::Relaxed), 16);
        assert_eq!(ACCOUNT_COMMITS.load(Ordering::Relaxed), 16);
        assert!(caps.auth_session_grants.is_empty() && caps.account_grants.is_empty());
        crate::serial_println!("[CAP-PR] lifecycle PASS auth=16 account=16 post_reap_records=0 late_auth=denied late_account=denied cpus={}", sched.online_cores);
        state.complete = true;
        crate::serial_println!("[CAP-P] native PASS cpus={} workers={} predictions_denied={} issuance_cycles={} free_before={} free_after={} data_delta=0 pt_delta=0 mappings=0 tokens=0 ticks={}", sched.online_cores, state.workers, state.workers * 124, state.workers * 128, state.baseline, free, sched.global_tick);
        unsafe {
            x86_64::instructions::port::Port::<u32>::new(0xf4).write(0x10);
        }
    }
    assert!(
        sched.global_tick < 12000,
        "native capability fixture timeout"
    );
}
