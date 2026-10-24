//! Opt-in observations/faults on real production service calls. No role
//! installation, entropy override, debug syscall or scheduler policy changes.
use super::{CapabilityBroker, CapabilityToken, GrantDelivery, RecipientIdentity};
use crate::process::{Process, ProcessState};
use crate::sched::{Scheduler, SCHEDULER};
use x86_64::VirtAddr;

const BASE: usize = 0xF400;
const MAX: usize = 12;
const ELF: &[u8] = include_bytes!("../../../target/x86_64-unknown-none/release/grant-probe");
#[derive(Clone, Copy)]
struct Entry {
    commits: usize,
    minted: usize,
    consumed: usize,
    delivered: usize,
    owner_retired: usize,
    delivery_retired: usize,
    exec_retired: usize,
    cancelled: bool,
    exec: bool,
    reaped: bool,
    last: CapabilityToken,
}
const EMPTY: Entry = Entry {
    commits: 0,
    minted: 0,
    consumed: 0,
    delivered: 0,
    owner_retired: 0,
    delivery_retired: 0,
    exec_retired: 0,
    cancelled: false,
    exec: false,
    reaped: false,
    last: CapabilityToken::INVALID,
};
static ENTRIES: spin::Mutex<[Entry; MAX]> = spin::Mutex::new([EMPTY; MAX]);
static COMPLETE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static CLIENT_COUNT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
const RACE_DRIVER: usize = 0xF4F0;
const RACE_PRE: usize = 0xF4F1;
const RACE_POST: usize = 0xF4F2;
static RACES: spin::Mutex<RaceState> = spin::Mutex::new(RaceState {
    commits: 0,
    exits: 0,
    reaped: 0,
    recipient: None,
});
struct RaceState {
    commits: usize,
    exits: u8,
    reaped: u8,
    recipient: Option<RecipientIdentity>,
}

fn index(pid: usize) -> Option<usize> {
    pid.checked_sub(BASE)
        .filter(|i| *i < CLIENT_COUNT.load(core::sync::atomic::Ordering::Relaxed))
}

pub fn run(hhdm: VirtAddr) {
    let count = crate::sched::ONLINE_CORES
        .load(core::sync::atomic::Ordering::Acquire)
        .min(MAX);
    CLIENT_COUNT.store(count, core::sync::atomic::Ordering::Relaxed);
    for ordinal in 0..count + 3 {
        let (pid, name, cpu) = if ordinal < count {
            (BASE + ordinal, "grant-probe", ordinal)
        } else {
            let offset = ordinal - count;
            (
                RACE_DRIVER + offset,
                if offset == 0 {
                    "grant-racer"
                } else {
                    "grant-probe"
                },
                if offset == 0 {
                    count - 1
                } else {
                    offset % count
                },
            )
        };
        let mut pmm = crate::PMM.lock();
        let mut process = unsafe { Process::new(pid, 0, name, &mut pmm, hhdm) };
        process.uid = 1000;
        process.gid = 1000;
        let entry = crate::process::elf_loader::load_elf(ELF, &mut process, &mut pmm, hhdm)
            .expect("grant probe ELF");
        crate::process::spawn::map_user_stack(&mut process, &mut pmm, hhdm).unwrap();
        process.init_context(entry, crate::process::layout::USER_STACK_TOP);
        process.cpu_mask = 1 << cpu;
        assert!(!process.trusted_auth_broker && !process.trusted_vfs_service);
        drop(pmm);
        SCHEDULER.lock().add_process(process);
    }
    crate::serial_println!(
        "[GRANT-C] begin real_services=uac,vfs clients={} cpus={}",
        count,
        count
    );
    accounting("start");
}

pub(super) fn committed(issuer: &Process, delivery: GrantDelivery, tokens: &[CapabilityToken]) {
    if delivery.recipient.pid == RACE_PRE || delivery.recipient.pid == RACE_POST {
        assert_eq!(
            delivery.recipient.pid, RACE_POST,
            "precommit exit admitted a grant"
        );
        assert_eq!(tokens.len(), 2);
        let mut races = RACES.lock();
        assert_eq!(races.commits, 0);
        races.commits = tokens.len();
        races.recipient = Some(delivery.recipient);
        crate::serial_println!("[GRANT-D] case=exit-after-commit checkpoint=commit issuer={} issuer_inc={} recipient={} incarnation={} vm={} session_generation=none grant_batch=postcommit count=2 cpu={}",
            issuer.pid, issuer.foreground.incarnation, delivery.recipient.pid, delivery.recipient.incarnation, delivery.recipient.vm_generation, crate::sched::current_cpu_id());
        return;
    }
    let Some(index) = index(delivery.recipient.pid) else {
        return;
    };
    assert!(issuer.trusted_auth_broker && !issuer.is_exiting());
    let mut entries = ENTRIES.lock();
    entries[index].commits += 1;
    assert_eq!(
        tokens.len(),
        if entries[index].commits == 5 { 1 } else { 2 }
    );
    entries[index].minted += tokens.len();
    entries[index].last = tokens[0];
    crate::serial_println!("[GRANT-C] commit issuer={} issuer_inc={} role=uac recipient={} incarnation={} vm={} request={} batch={} ordinal={} cpu={}",
        issuer.pid, issuer.foreground.incarnation, delivery.recipient.pid, delivery.recipient.incarnation,
        delivery.recipient.vm_generation, delivery.request, tokens.len(), entries[index].commits, crate::sched::current_cpu_id());
}

pub(crate) fn before_delivery(
    target: crate::process::IpcReplyTarget,
    sched: &mut Scheduler,
    caps: &mut CapabilityBroker,
    bus: &mut crate::ipc::IpcBus,
) {
    let Some(index) = index(target.call.pid) else {
        return;
    };
    let entry = ENTRIES.lock()[index];
    if entry.commits != 3
        || entry.cancelled
        || !caps.vfs_caps.iter().any(|g| {
            g.delivery.is_some_and(|d| {
                !d.delivered
                    && d.matches(
                        target.call.pid,
                        target.call.incarnation,
                        target.call.generation,
                    )
            })
        })
    {
        return;
    }
    let before = caps.recipient_grant_count(target.call.pid);
    assert_eq!(before, 2);
    crate::ipc::handle_ipc_cancel(target.call.pid, sched, caps, bus).unwrap();
    crate::ipc::handle_ipc_cancel(target.call.pid, sched, caps, bus).unwrap();
    assert_eq!(caps.recipient_grant_count(target.call.pid), 0);
    ENTRIES.lock()[index].cancelled = true;
    ENTRIES.lock()[index].delivery_retired += before;
    crate::serial_println!("[GRANT-C] delivery-cancel recipient={} incarnation={} request={} committed=2 retired=2 inventory=0 cpu={}",
        target.call.pid, target.call.incarnation, target.call.generation, crate::sched::current_cpu_id());
}

pub(crate) fn after_delivery(target: crate::process::IpcReplyTarget, caps: &CapabilityBroker) {
    let Some(index) = index(target.call.pid) else {
        return;
    };
    let delivered = caps
        .vfs_caps
        .iter()
        .filter(|g| {
            g.delivery.is_some_and(|d| {
                d.delivered
                    && d.matches(
                        target.call.pid,
                        target.call.incarnation,
                        target.call.generation,
                    )
            })
        })
        .count();
    if delivered != 0 {
        ENTRIES.lock()[index].delivered += delivered;
        crate::serial_println!(
            "[GRANT-C] delivered recipient={} incarnation={} request={} count={} cpu={}",
            target.call.pid,
            target.call.incarnation,
            target.call.generation,
            delivered,
            crate::sched::current_cpu_id()
        );
    }
}

pub(super) fn consumed(issuer: &Process, recipient: RecipientIdentity, allowed: bool) {
    let Some(index) = index(recipient.pid) else {
        return;
    };
    assert!(issuer.trusted_vfs_service && !issuer.is_exiting());
    if allowed {
        ENTRIES.lock()[index].consumed += 1;
    }
    crate::serial_println!("[GRANT-C] consume service={} service_inc={} role=vfs recipient={} incarnation={} vm={} allowed={} cpu={}",
        issuer.pid, issuer.foreground.incarnation, recipient.pid, recipient.incarnation, recipient.vm_generation, allowed, crate::sched::current_cpu_id());
}

pub(crate) fn exec(
    process: &Process,
    old_vm: crate::process::mm2b_state::AddressSpaceIdentity,
    old_grants: usize,
    caps: &mut CapabilityBroker,
) {
    let Some(index) = index(process.pid) else {
        return;
    };
    let token = ENTRIES.lock()[index].last;
    assert_eq!(old_grants, 2);
    assert_eq!(caps.recipient_grant_count(process.pid), 0);
    assert!(!caps.consume_vfs_grant(token, RecipientIdentity::of(process), "/bin/true", 0));
    assert_ne!(
        old_vm.generation,
        process.address_space.identity().generation
    );
    assert!(!process.trusted_auth_broker && !process.trusted_vfs_service);
    ENTRIES.lock()[index].exec = true;
    ENTRIES.lock()[index].exec_retired += old_grants;
    crate::serial_println!("[GRANT-C] exec recipient={} incarnation={} old_vm={} new_vm={} old_token=denied inventory=0 cpu={}",
        process.pid, process.foreground.incarnation, old_vm.generation, process.address_space.identity().generation, crate::sched::current_cpu_id());
}

pub(super) fn owner_cancelled(pid: usize, before: usize, caps: &CapabilityBroker) {
    let Some(index) = index(pid) else { return };
    let after = caps.recipient_grant_count(pid);
    let retired = before
        .checked_sub(after)
        .expect("cancel cannot create authority");
    assert!(retired <= 1);
    ENTRIES.lock()[index].owner_retired += retired;
    crate::serial_println!(
        "[GRANT-C] owner-cancel recipient={} retired={} inventory={} cpu={}",
        pid,
        retired,
        after,
        crate::sched::current_cpu_id()
    );
}

/// Record the real reaper transition before normal process-slot reuse can
/// overwrite the dead record. Timer sampling cannot reliably observe Reaped.
pub(crate) fn reaped(process: &Process) {
    if (RACE_DRIVER..=RACE_POST).contains(&process.pid) {
        assert_eq!(process.state, ProcessState::Reaped);
        if process.pid == RACE_DRIVER {
            assert_eq!(process.exit_code, 0);
        } else {
            assert_ne!(process.exit_code, 0);
        }
        let mut races = RACES.lock();
        let bit = 1 << (process.pid - RACE_DRIVER);
        assert_eq!(races.reaped & bit, 0);
        races.reaped |= bit;
        crate::serial_println!(
            "[GRANT-D] checkpoint=reaped recipient={} incarnation={} exit={} cpu={}",
            process.pid,
            process.foreground.incarnation,
            process.exit_code,
            crate::sched::current_cpu_id()
        );
        return;
    }
    let Some(index) = index(process.pid) else {
        return;
    };
    assert_eq!(process.state, ProcessState::Reaped);
    assert_eq!(process.exit_code, 0);
    let mut entries = ENTRIES.lock();
    assert!(entries[index].exec);
    assert!(!entries[index].reaped);
    entries[index].reaped = true;
    crate::serial_println!(
        "[GRANT-C] reaped recipient={} incarnation={} exit=0 cpu={}",
        process.pid,
        process.foreground.incarnation,
        crate::sched::current_cpu_id()
    );
}

pub fn on_tick(sched: &mut Scheduler, cpu: usize) {
    use core::sync::atomic::Ordering;
    if cpu != 0 || COMPLETE.load(Ordering::Relaxed) {
        return;
    }
    let count = sched.online_cores.min(MAX);
    let entries = *ENTRIES.lock();
    if !entries[..count].iter().all(|e| e.exec && e.reaped) || RACES.lock().reaped != 7 {
        return;
    }
    {
        let races = RACES.lock();
        assert_eq!(races.commits, 2);
        assert_eq!(races.exits, 3);
    }
    for index in 0..count {
        assert_eq!(entries[index].commits, 6);
        assert_eq!(entries[index].minted, 11);
        assert_eq!(entries[index].consumed, 4);
        assert_eq!(entries[index].delivered, 9);
        assert_eq!(entries[index].owner_retired, 3);
        assert_eq!(entries[index].delivery_retired, 2);
        assert_eq!(entries[index].exec_retired, 2);
        assert_eq!(
            entries[index].minted,
            entries[index].consumed
                + entries[index].owner_retired
                + entries[index].delivery_retired
                + entries[index].exec_retired
        );
        assert!(entries[index].cancelled);
    }
    let caps = super::CAP_BROKER.lock();
    assert_eq!(caps.recipient_grant_count(RACE_PRE), 0);
    assert_eq!(caps.recipient_grant_count(RACE_POST), 0);
    for index in 0..count {
        assert_eq!(caps.recipient_grant_count(BASE + index), 0);
    }
    crate::serial_println!("[GRANT-C] metadata vfs_active={} vfs_capacity={} vfs_entry_bytes={} auth_active={} auth_capacity={} allocator_drift=UNMEASURED",
        caps.vfs_caps.len(), caps.vfs_caps.capacity(), core::mem::size_of::<super::VfsGrant>(),
        caps.auth_session_grants.len(), caps.auth_session_grants.capacity());
    drop(caps);
    accounting("final");
    crate::serial_println!("[GRANT-D] lifecycle COMPLETE precommit_commits=0 postcommit_commits=2 inventory=0 reaped=3 pid_reuse=NOT_FORCED restart=NOT_FORCED");
    COMPLETE.store(true, Ordering::Relaxed);
    crate::serial_println!("[GRANT-C] COMPLETE PASS cpus={} clients={} committed={} delivered={} consumed={} owner_retired={} delivery_retired={} exec_retired={} inventory=0 pending=0 issuer_refs=0 recipient_refs=0 remote_race=NOT_FORCED",
        count, count, count * 11, count * 9, count * 4, count * 3, count * 2, count * 2);
    unsafe {
        x86_64::instructions::port::Port::<u32>::new(0xf4).write(0x10);
    }
}

/// Observation on the ordinary signal path, immediately before the real
/// terminal transition. No waiting, forged identity, or policy bypass.
pub(crate) fn before_race_exit(sched: &Scheduler, target: usize) {
    let process = &sched.processes[target];
    if sched.current_process().pid != RACE_DRIVER
        || (process.pid != RACE_PRE && process.pid != RACE_POST)
    {
        return;
    }
    let caps = super::CAP_BROKER.lock();
    let inventory = caps
        .vfs_caps
        .iter()
        .filter(|g| g.delivery.is_some_and(|d| d.recipient.pid == process.pid))
        .count();
    let post = process.pid == RACE_POST;
    assert_eq!(inventory, if post { 2 } else { 0 });
    // Before exchange the delivered password proof still belongs to the
    // recipient. After exchange it is consumed and replaced by two VFS rows.
    assert_eq!(
        caps.recipient_grant_count(process.pid),
        if post { 2 } else { 1 }
    );
    let mut races = RACES.lock();
    if post {
        assert_eq!(races.recipient, Some(RecipientIdentity::of(process)));
    }
    let bit = if post { 2 } else { 1 };
    assert_eq!(races.exits & bit, 0);
    races.exits |= bit;
    crate::serial_println!("[GRANT-D] case={} checkpoint=signal-before-exit issuer_driver={} driver_inc={} recipient={} incarnation={} vm={} session_generation=none inventory={} cpu={}",
        if post { "exit-after-commit" } else { "exit-before-commit" }, sched.current_process().pid,
        sched.current_process().foreground.incarnation, process.pid, process.foreground.incarnation,
        process.address_space.identity().generation, inventory, crate::sched::current_cpu_id());
}

fn accounting(phase: &str) {
    // Acquire/release PMM before CAP_BROKER; never invert SHM's lock order.
    let free_frames = crate::PMM.lock().free_page_count();
    let heap = crate::memory::heap::heap_stats();
    let caps = super::CAP_BROKER.lock();
    let mut pending = 0;
    let mut delivered = 0;
    let mut legacy = 0;
    let deliveries = caps
        .auth_session_grants
        .iter()
        .map(|g| g.delivery)
        .chain(caps.account_grants.iter().map(|(_, g)| g.delivery))
        .chain(caps.vfs_caps.iter().map(|g| g.delivery));
    for delivery in deliveries {
        match delivery {
            Some(d) if d.delivered => delivered += 1,
            Some(_) => pending += 1,
            None => legacy += 1,
        }
    }
    assert_eq!(
        pending + delivered + legacy + caps.capabilities.len() + caps.shared_regions.len(),
        caps.active_tokens()
    );
    let (history, history_capacity) = caps.token_history.usage();
    let tokens = caps
        .capabilities
        .iter()
        .map(|(t, _, _)| t.0)
        .chain(caps.shared_regions.iter().map(|g| g.token.0))
        .chain(caps.vfs_caps.iter().map(|g| g.token.0))
        .chain(caps.auth_session_grants.iter().map(|g| g.token.0))
        .chain(caps.account_grants.iter().map(|(t, _)| t.0));
    assert!(tokens.clone().all(|t| caps.token_history.contains(t)));
    for (index, token) in tokens.clone().enumerate() {
        assert!(!tokens.clone().skip(index + 1).any(|t| t == token));
    }
    crate::serial_println!("[GRANT-D] tables phase={} endpoint_size={} endpoint_len={} endpoint_capacity={} ipc_size={} ipc_capacity={} shm_size={} shm_capacity={} auth_size={} auth_len={} auth_capacity={} account_size={} account_len={} account_capacity={} vfs_size={} vfs_len={} vfs_capacity={} shm_frame_vector_capacity_bytes={}",
        phase, core::mem::size_of::<super::Endpoint>(), caps.endpoints.len(), caps.endpoints.capacity(),
        core::mem::size_of::<(CapabilityToken, u32, super::CapabilityRights)>(), caps.capabilities.capacity(),
        core::mem::size_of::<super::ShmEntry>(), caps.shared_regions.capacity(),
        core::mem::size_of::<super::AuthSessionGrant>(), caps.auth_session_grants.len(), caps.auth_session_grants.capacity(),
        core::mem::size_of::<(CapabilityToken, super::AccountGrant)>(), caps.account_grants.len(), caps.account_grants.capacity(),
        core::mem::size_of::<super::VfsGrant>(), caps.vfs_caps.len(), caps.vfs_caps.capacity(),
        caps.shared_regions.iter().map(|g| g.obj.frames.capacity() * core::mem::size_of::<x86_64::structures::paging::PhysFrame>()).sum::<usize>());
    crate::serial_println!("[GRANT-D] accounting phase={} broker_live={} ipc={} shm={} pending={} delivered={} legacy={} history={} history_capacity={} history_bytes={} history_capacity_bytes={} heap_used={} heap_free={} free_frames={} global_reconciliation=INCOMPLETE",
        phase, caps.active_tokens(), caps.capabilities.len(), caps.shared_regions.len(), pending, delivered, legacy,
        history, history_capacity, history * 8, history_capacity * 8, heap.allocated, heap.free, free_frames);
}
