//! Isolated opt-in ring-3 issuance/authorization fixture. No normal service
//! migration and no deterministic entropy override on the global broker.
use super::{CapError, CapabilityBroker, CapabilityRights};
use crate::memory::accounting::{class_frame_count, PhysicalMemoryClass};
use crate::process::{Process, ProcessState};
use crate::sched::{Scheduler, ONLINE_CORES, SCHEDULER};
use core::sync::atomic::Ordering;
use x86_64::VirtAddr;

const ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/capability-probe.elf"));
pub(super) static AUTH_COMMITS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);
static ACCOUNT_DENIALS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

pub(crate) fn account_denied(sched: &Scheduler, error: sunlight_ipc::accounts::Error) {
    use sunlight_ipc::accounts::Error;
    assert!(matches!(
        error,
        Error::SessionAuthorityUnavailable | Error::Denied
    ));
    ACCOUNT_DENIALS.fetch_add(1, Ordering::Relaxed);
    let issuer = sched.current_process();
    crate::serial_println!("[SESSION-E] checkpoint=account-denied issuer={} issuer_inc={} vm={} session_epoch=UNESTABLISHED broker=boot-global commit=none error={} cpu={}",
        issuer.pid, issuer.foreground.incarnation, issuer.address_space.identity().generation,
        error as u64, crate::sched::current_cpu_id());
}

const OWNER: usize = 0xF200;
const READER: usize = OWNER + 1;
static READER_ENDPOINT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static EXEC_COMMITS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

pub(crate) fn assert_exec_authority(
    process: &Process,
    old: crate::process::mm2b_state::AddressSpaceIdentity,
) {
    assert_eq!(process.pid, READER);
    assert_ne!(process.foreground.incarnation, 0);
    assert_ne!(old.generation, process.address_space.identity().generation);
    assert!(!process.trusted_auth_broker);
    assert!(!process.trusted_service_manager);
    assert!(!process.trusted_lock_service);
    assert!(!process.trusted_tty_session_service);
    assert_eq!(process.uid, 0);
    let caps = super::CAP_BROKER.lock();
    assert_eq!(caps.recipient_grant_count(READER), 0);
    let token = super::CapabilityToken(READER_ENDPOINT.load(Ordering::Relaxed));
    assert_eq!(
        caps.token_owner(token, CapabilityRights::RECV_ONLY)
            .map(|(_, owner)| owner),
        Ok(READER)
    );
    let index = EXEC_COMMITS.fetch_add(1, Ordering::Relaxed);
    if index == 0 {
        assert_eq!(process.mapped_shared.len(), 1);
    }
    crate::serial_println!("[EXEC-R3] authority commit={} pid={} incarnation={} old_generation={} new_generation={} roles=withdrawn endpoint=preserved uid=preserved old_views={}",
        index + 1, process.pid, process.foreground.incarnation, old.generation,
        process.address_space.identity().generation, process.mapped_shared.len());
}

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
    super::history_boundary::run();
    super::sessions::native_sequential_probe();
    {
        // Isolated production broker only; never override global entropy or
        // install a trusted role on a live process.
        let mut caps = CapabilityBroker::new();
        caps.test_entropy(&[Some(41)]);
        let (_, old) = caps.create_endpoint(1).unwrap();
        let capacity = caps.capabilities.capacity();
        caps.destroy_endpoint(1, old).unwrap();
        caps.test_entropy(&[Some(41), Some(43)]);
        let (endpoint, new) = caps.create_endpoint(2).unwrap();
        assert!(old != new);
        assert_eq!(caps.capabilities.capacity(), capacity);
        assert_eq!(
            caps.check(old, CapabilityRights::SEND),
            Err(CapError::InvalidToken)
        );
        assert_eq!(caps.check(new, CapabilityRights::SEND), Ok(endpoint));
        caps.test_entropy(&[Some(41); 16]);
        assert_eq!(caps.create_endpoint(3), Err(CapError::TokenCollisionLimit));
        caps.destroy_endpoint(2, new).unwrap();
        assert_eq!(caps.active_tokens(), 0);
        crate::serial_println!("[GRANT-D] retired-token local_broker recycled_storage=observed repeated_candidate=rejected stuck_source=failed_closed old_lookup=denied active=0 scope=isolated_production_broker");
    }
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
    reader.trusted_lock_service = true;
    if option_env!("SUNLIGHT_CAP_EXEC_PROBE") == Some("1") {
        // Synthetic launch provenance, isolated from production services.
        reader.trusted_auth_broker = true;
        reader.trusted_lock_service = true;
        reader.trusted_tty_session_service = true;
        let (_, token) = super::CAP_BROKER.lock().create_endpoint(READER).unwrap();
        READER_ENDPOINT.store(token.as_u64(), Ordering::Relaxed);
    }
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
    reader.set_initial_args(
        2,
        shm.as_u64(),
        send.as_u64(),
        u64::from(option_env!("SUNLIGHT_CAP_EXEC_PROBE") == Some("1")),
    );
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
            unsafe {
                (state.signal_address as *mut u64).write_volatile(0xc1ea);
            }
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
        assert_eq!(AUTH_COMMITS.load(Ordering::Relaxed), 17);
        assert_eq!(AUTH_CONSUMES.load(Ordering::Relaxed), 15);
        assert!(DELIVERY_BOUNDARIES.load(Ordering::Relaxed));
        crate::serial_println!("[CAP-R41] COMPLETE delivered_auth_consumed=15 stale_incarnation=denied real_call_cancel=retired pending=0 active_auth_account=0 cpus={}", sched.online_cores);
        assert_eq!(ACCOUNT_DENIALS.load(Ordering::Relaxed), 35);
        assert!(caps.auth_session_grants.is_empty() && caps.account_grants.is_empty());
        crate::serial_println!("[CAP-PR] lifecycle PASS auth=17 account=0 account_denied=35 post_reap_records=0 late_auth=denied late_account=denied cpus={}", sched.online_cores);
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

static DELIVERY_BOUNDARIES: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static AUTH_CONSUMES: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
pub(crate) fn note_auth_consumption(pid: usize) {
    if pid == READER {
        AUTH_CONSUMES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Bounded cancellation injection on the isolated fixture's first real reply.
/// Runs at the existing serialized delivery boundary; no waiting under locks.
pub(crate) fn before_delivery(
    server: usize,
    target: crate::process::IpcReplyTarget,
    reply: crate::ipc::IpcMsg,
    sched: &mut Scheduler,
    caps: &mut CapabilityBroker,
    bus: &mut crate::ipc::IpcBus,
) {
    if server != OWNER || DELIVERY_BOUNDARIES.swap(true, Ordering::Relaxed) {
        return;
    }
    let recipient = sched
        .processes
        .iter()
        .find(|p| p.pid == target.call.pid)
        .unwrap();
    let identity = super::delivery::RecipientIdentity::of(recipient);
    let before = caps.recipient_grant_count(READER);
    assert_eq!(before, 1);
    crate::serial_println!("[CAP-R41] START op=real-auth-reply-account-denied pid={} incarnation={} vm={} request={} pending=1", identity.pid, identity.incarnation, identity.vm_generation, target.call.generation);
    let mut stale = target;
    stale.call.incarnation = stale.call.incarnation.checked_add(1).unwrap();
    crate::ipc::deliver_reply_to_target(stale, reply, sched, caps, bus).unwrap();
    assert_eq!(caps.recipient_grant_count(READER), before);
    assert!(sched
        .processes
        .iter()
        .find(|p| p.pid == READER)
        .unwrap()
        .pending_call
        .is_some());
    crate::ipc::handle_ipc_cancel(READER, sched, caps, bus).unwrap();
    crate::ipc::handle_ipc_cancel(READER, sched, caps, bus).unwrap();
    assert_eq!(caps.recipient_grant_count(READER), 0);
    let recipient = sched.processes.iter().find(|p| p.pid == READER).unwrap();
    assert!(recipient.pending_call.is_none());
    assert!(
        matches!(recipient.ipc_call_outcome, Some(crate::process::IpcCallOutcome::ExplicitlyCancelled(g)) if g == target.call.generation)
    );
    crate::serial_println!("[CAP-R41] boundary stale-incarnation=denied real-call-cancel=retired duplicate=stable pending=0");
}
