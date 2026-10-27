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
static PID_CHILDREN_REAPED: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);
const RACE_DRIVER: usize = 0xF4F0;
const RACE_PRE: usize = 0xF4F1;
const RACE_POST: usize = 0xF4F2;
const RACE_EXEC: usize = 0xF4F5;
static EXEC_REPLY: spin::Mutex<(Option<RecipientIdentity>, bool, bool, bool)> =
    spin::Mutex::new((None, false, false, false));
static SERVICE_EPOCH: spin::Mutex<(usize, u8)> = spin::Mutex::new((0, 0));
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

struct ConsumeRace {
    identity: Option<RecipientIdentity>,
    token: CapabilityToken,
    exec: bool,
    attempted: bool,
    reaped: bool,
}
const CONSUME_EMPTY: ConsumeRace = ConsumeRace {
    identity: None,
    token: CapabilityToken::INVALID,
    exec: false,
    attempted: false,
    reaped: false,
};
static CONSUME_RACES: spin::Mutex<[ConsumeRace; 2]> =
    spin::Mutex::new([CONSUME_EMPTY, CONSUME_EMPTY]);

fn consume_index(pid: usize) -> Option<usize> {
    pid.checked_sub(RACE_DRIVER + 3).filter(|index| *index < 2)
}

fn index(pid: usize) -> Option<usize> {
    pid.checked_sub(BASE)
        .filter(|i| *i < CLIENT_COUNT.load(core::sync::atomic::Ordering::Relaxed))
}

pub fn run(hhdm: VirtAddr) {
    crate::process::pid::native_boundary();
    let count = crate::sched::ONLINE_CORES
        .load(core::sync::atomic::Ordering::Acquire)
        .min(MAX);
    CLIENT_COUNT.store(count, core::sync::atomic::Ordering::Relaxed);
    for ordinal in 0..count + 6 {
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
    super::reconciliation::snapshot(&SCHEDULER.lock(), "start");
}

pub(super) fn committed(issuer: &Process, delivery: GrantDelivery, tokens: &[CapabilityToken]) {
    if delivery.recipient.pid == RACE_EXEC {
        assert_eq!(tokens.len(), 2);
        let mut state = EXEC_REPLY.lock();
        assert!(state.0.is_none());
        state.0 = Some(delivery.recipient);
        crate::serial_println!("[GRANT-G] exec-reply checkpoint=commit pid={} incarnation={} vm={} request={} count=2 cpu={}", delivery.recipient.pid, delivery.recipient.incarnation, delivery.recipient.vm_generation, delivery.request, crate::sched::current_cpu_id());
        return;
    }
    if delivery.recipient.pid == RACE_DRIVER {
        crate::serial_println!("[GRANT-G] cycle-commit epoch={} pid={} incarnation={} vm={} request={} count={} cpu={}", SERVICE_EPOCH.lock().0 + 1, delivery.recipient.pid, delivery.recipient.incarnation, delivery.recipient.vm_generation, delivery.request, tokens.len(), crate::sched::current_cpu_id());
    }
    if let Some(index) = consume_index(delivery.recipient.pid) {
        assert_eq!(tokens.len(), 2);
        let mut races = CONSUME_RACES.lock();
        assert!(races[index].identity.is_none());
        races[index].identity = Some(delivery.recipient);
        races[index].token = tokens[0];
        crate::serial_println!("[GRANT-F] lifecycle case={} checkpoint=commit pid={} incarnation={} vm={} issuer={} issuer_inc={} issuer_vm={} request={} fingerprint=boot-local-{} cpu={}", if index == 0 { "exit-consume" } else { "exec-consume" }, delivery.recipient.pid, delivery.recipient.incarnation, delivery.recipient.vm_generation, issuer.pid, issuer.foreground.incarnation, issuer.address_space.identity().generation, delivery.request, super::reconciliation::fingerprint(tokens[0]), crate::sched::current_cpu_id());
        return;
    }
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
    crate::serial_println!(
        "[GRANT-F] commit pid={} incarnation={} vm={} request={} fingerprint=boot-local-{} cpu={}",
        delivery.recipient.pid,
        delivery.recipient.incarnation,
        delivery.recipient.vm_generation,
        delivery.request,
        super::reconciliation::fingerprint(tokens[0]),
        crate::sched::current_cpu_id()
    );
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
    if target.call.pid == RACE_EXEC && EXEC_REPLY.lock().1 {
        assert_eq!(caps.recipient_grant_count(RACE_EXEC), 0);
        let mut state = EXEC_REPLY.lock();
        assert!(!state.3);
        state.3 = true;
        crate::serial_println!("[GRANT-G] exec-reply checkpoint=late-settlement pid={} incarnation={} request={} inventory=0 cpu={}", target.call.pid, target.call.incarnation, target.call.generation, crate::sched::current_cpu_id());
    }
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

pub(super) fn census_after_consume(sched: &Scheduler, token: CapabilityToken) {
    let pid = sched.current_process().ipc_reply_target.map(|t| t.call.pid);
    if pid == Some(RACE_DRIVER) && token == CapabilityToken::INVALID {
        let (epoch, ack) = *SERVICE_EPOCH.lock();
        if ack == 3 {
            let caps = super::CAP_BROKER.lock();
            assert_eq!(caps.recipient_grant_count(RACE_DRIVER), 0);
            assert!((BASE..BASE + MAX)
                .chain(RACE_PRE..=RACE_EXEC)
                .all(|pid| caps.recipient_grant_count(pid) == 0));
            drop(caps);
            accounting("cycle");
            crate::serial_println!("[GRANT-G] boundary epoch={} services=uac,vfs scope=completed-fixture-operations sessiond=UNMEASURED fixture_grants=0 cpu={}", epoch, crate::sched::current_cpu_id());
            super::reconciliation::snapshot(sched, "operation");
        }
    }
    if pid.and_then(index).is_some() {
        super::reconciliation::snapshot(sched, "operation");
    }
}

pub(crate) fn exec(
    process: &Process,
    old_vm: crate::process::mm2b_state::AddressSpaceIdentity,
    old_grants: usize,
    caps: &mut CapabilityBroker,
) {
    if process.pid == RACE_EXEC {
        let mut state = EXEC_REPLY.lock();
        let old = state.0.expect("grant committed before recipient exec");
        assert_eq!(old.vm_generation, old_vm.generation);
        assert_eq!(old.incarnation, process.foreground.incarnation);
        assert_ne!(
            old_vm.generation,
            process.address_space.identity().generation
        );
        // A blocked recipient cannot exec until its IPC call ends. The real
        // deadline cancels Pending delivery before userspace resumes for exec.
        assert_eq!(old_grants, 0);
        assert_eq!(caps.recipient_grant_count(process.pid), 0);
        state.1 = true;
        crate::serial_println!("[GRANT-G] exec-reply checkpoint=exec pid={} incarnation={} old_vm={} new_vm={} retired=2 exec_retired=0 retirement=ipc-cancel-before-exec cpu={}", process.pid, process.foreground.incarnation, old_vm.generation, process.address_space.identity().generation, crate::sched::current_cpu_id());
        return;
    }
    if let Some(index) = consume_index(process.pid) {
        assert_eq!(index, 1);
        let mut races = CONSUME_RACES.lock();
        let race = &mut races[index];
        let old = race.identity.expect("committed recipient");
        assert_eq!(old.pid, process.pid);
        assert_eq!(old.incarnation, process.foreground.incarnation);
        assert_eq!(old.vm_generation, old_vm.generation);
        assert_ne!(
            old_vm.generation,
            process.address_space.identity().generation
        );
        assert_eq!(old_grants, 2);
        assert_eq!(caps.recipient_grant_count(process.pid), 0);
        assert!(!caps.consume_vfs_grant(
            race.token,
            RecipientIdentity::of(process),
            "/bin/true",
            0
        ));
        assert!(!race.exec);
        race.exec = true;
        crate::serial_println!("[GRANT-F] lifecycle case=exec-consume checkpoint=exec pid={} incarnation={} old_vm={} new_vm={} retired=2 replay=denied cpu={}", process.pid, process.foreground.incarnation, old_vm.generation, process.address_space.identity().generation, crate::sched::current_cpu_id());
        return;
    }
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
    if process.pid == RACE_EXEC {
        assert_eq!(process.exit_code, 0);
        let mut state = EXEC_REPLY.lock();
        assert!(state.1 && !state.2);
        state.2 = true;
        crate::serial_println!(
            "[GRANT-G] exec-reply checkpoint=reaped pid={} incarnation={} cpu={}",
            process.pid,
            process.foreground.incarnation,
            crate::sched::current_cpu_id()
        );
        return;
    }
    if process.ppid == RACE_DRIVER {
        assert_eq!(process.exit_code, 0);
        assert_eq!(process.state, ProcessState::Reaped);
        PID_CHILDREN_REAPED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    if let Some(index) = consume_index(process.pid) {
        let mut races = CONSUME_RACES.lock();
        let race = &mut races[index];
        assert_eq!(process.state, ProcessState::Reaped);
        assert_eq!(process.exit_code == 0, index == 1);
        assert_eq!(race.exec, index == 1);
        assert!(!race.reaped);
        race.reaped = true;
        crate::serial_println!("[GRANT-F] lifecycle case={} checkpoint=reaped pid={} incarnation={} exit={} pending=0 reply=0 deferred=0 cpu={}", if index == 0 { "exit-consume" } else { "exec-consume" }, process.pid, process.foreground.incarnation, process.exit_code, crate::sched::current_cpu_id());
        return;
    }
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
    if !{
        let state = EXEC_REPLY.lock();
        state.1 && state.2 && state.3
    } || SERVICE_EPOCH.lock().0 != 4
    {
        return;
    }
    let entries = *ENTRIES.lock();
    if !entries[..count].iter().all(|e| e.exec && e.reaped) || RACES.lock().reaped != 7 {
        return;
    }
    if !CONSUME_RACES.lock().iter().all(|r| r.reaped && r.attempted) {
        return;
    }
    if PID_CHILDREN_REAPED.load(Ordering::Relaxed) != 8 {
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
    super::reconciliation::snapshot(sched, "final");
    crate::serial_println!("[GRANT-D] lifecycle COMPLETE precommit_commits=0 postcommit_commits=2 inventory=0 reaped=3 pid_reuse=NOT_FORCED restart=NOT_FORCED");
    COMPLETE.store(true, Ordering::Relaxed);
    crate::serial_println!("[GRANT-C] COMPLETE PASS cpus={} clients={} committed={} delivered={} consumed={} owner_retired={} delivery_retired={} exec_retired={} inventory=0 pending=0 issuer_refs=0 recipient_refs=0 remote_race=NOT_FORCED",
        count, count, count * 11, count * 9, count * 4, count * 3, count * 2, count * 2);
    #[cfg(feature = "grant_history_test")]
    history::arm(count);
    #[cfg(feature = "grant_history_transport_test")]
    {
        assert_eq!(TRANSPORT_ATTEMPTS.load(Ordering::Relaxed), 2);
        let broker = super::CAP_BROKER.lock();
        assert_eq!(
            broker.token_history.usage().0,
            super::token_history::MAX_PREPARED_TOKENS
        );
        assert_eq!(broker.recipient_grant_count(RACE_DRIVER), 0);
        crate::serial_println!("[HISTORY-GT] COMPLETE PASS capacity={} history={} attempts=2 partial=0 proof=exec-retired error_transport=explicit exhaustion=permanent", super::token_history::MAX_PREPARED_TOKENS, broker.token_history.usage().0);
    }
    #[cfg(not(feature = "grant_history_test"))]
    unsafe {
        x86_64::instructions::port::Port::<u32>::new(0xf4).write(0x10);
    }
}

/// Entry observation on the real VFS consume syscall. No locks are retained
/// across a checkpoint: retirement has already won in userspace rendezvous.
pub(super) fn consumption_attempt(sched: &Scheduler) {
    let issuer = sched.current_process();
    let Some(target) = issuer.ipc_reply_target else {
        return;
    };
    let Some(index) = consume_index(target.call.pid) else {
        return;
    };
    assert!(issuer.trusted_vfs_service && !issuer.is_exiting());
    let recipient = sched.processes.iter().find(|p| p.pid == target.call.pid);
    let caps = super::CAP_BROKER.lock();
    let inventory = caps.recipient_grant_count(target.call.pid);
    assert!(recipient.is_none_or(|p| p.is_exiting()));
    assert!(inventory <= if index == 0 { 2 } else { 0 });
    let mut races = CONSUME_RACES.lock();
    let race = &mut races[index];
    assert_eq!(target.call.incarnation, race.identity.unwrap().incarnation);
    assert_eq!(race.exec, index == 1);
    assert!(!race.attempted);
    race.attempted = true;
    crate::serial_println!("[GRANT-F] lifecycle case={} checkpoint=consume-attempt pid={} incarnation={} old_vm={} issuer={} issuer_inc={} issuer_vm={} inventory={} recipient=retiring-or-unpublished cpu={}", if index == 0 { "exit-consume" } else { "exec-consume" }, target.call.pid, target.call.incarnation, race.identity.unwrap().vm_generation, issuer.pid, issuer.foreground.incarnation, issuer.address_space.identity().generation, inventory, crate::sched::current_cpu_id());
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

/// Observe replies at the real role-checked service IPC boundary. Diagnostic
/// values are service observations, never used for production authorization.
pub(crate) fn service_ack(
    sched: &Scheduler,
    target: crate::process::IpcReplyTarget,
    reply: crate::ipc::IpcMsg,
) {
    if target.call.pid != RACE_DRIVER || reply.label != 0xF11B07 {
        return;
    }
    let server = sched.current_process();
    let bit = if server.trusted_auth_broker {
        1
    } else if server.trusted_vfs_service {
        2
    } else {
        panic!("untrusted census ack")
    };
    let epoch = reply.words[0] as usize;
    let mut state = SERVICE_EPOCH.lock();
    if bit == 1 {
        assert_eq!(epoch, state.0 + 1);
        *state = (epoch, 1);
    } else {
        assert_eq!(*state, (epoch, 1));
        state.1 = 3;
    }
    crate::serial_println!("[GRANT-G] service-ack epoch={} service={} pid={} incarnation={} vm={} caller={} caller_inc={} request={} records={} active_or_boot_handles={} heap_used={} boundary=handler-returned cpu={}", epoch, if bit == 1 { "uac" } else { "vfs" }, server.pid, server.foreground.incarnation, server.address_space.identity().generation, target.call.pid, target.call.incarnation, target.call.generation, reply.words[1], reply.words[2], reply.words[3], crate::sched::current_cpu_id());
}

#[cfg(feature = "grant_history_test")]
#[path = "global_history_gate.rs"]
pub(crate) mod history;

#[cfg(feature = "grant_history_transport_test")]
static TRANSPORT_ATTEMPTS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

#[cfg(feature = "grant_history_transport_test")]
pub(super) fn history_transport_prepare(broker: &mut CapabilityBroker, delivery: GrantDelivery) {
    if delivery.recipient.pid != RACE_DRIVER || SERVICE_EPOCH.lock().0 != 4 {
        return;
    }
    if TRANSPORT_ATTEMPTS.load(core::sync::atomic::Ordering::Relaxed) == 0 {
        let baseline = broker.active_tokens();
        while broker.token_history.usage().0 < super::token_history::MAX_PREPARED_TOKENS - 1 {
            drop(broker.prepare_shared_region().unwrap());
        }
        assert_eq!(broker.active_tokens(), baseline);
        crate::serial_println!("[HISTORY-GT] prefill history={} capacity={} authority_added=0 entropy=production cpu={}", broker.token_history.usage().0, super::token_history::MAX_PREPARED_TOKENS, crate::sched::current_cpu_id());
    }
}

#[cfg(feature = "grant_history_transport_test")]
pub(super) fn history_transport_result(
    broker: &CapabilityBroker,
    delivery: GrantDelivery,
    proof: CapabilityToken,
    result: &Result<heapless::Vec<CapabilityToken, 2>, super::CapError>,
) {
    if delivery.recipient.pid != RACE_DRIVER || SERVICE_EPOCH.lock().0 != 4 {
        return;
    }
    assert!(matches!(result, Err(super::CapError::TokenHistoryFull)));
    assert_eq!(
        broker.token_history.usage().0,
        super::token_history::MAX_PREPARED_TOKENS
    );
    assert_eq!(broker.recipient_grant_count(RACE_DRIVER), 1);
    assert!(broker
        .auth_session_grants
        .iter()
        .any(|g| g.token == proof && g.delivery.is_some_and(|d| d.usable(delivery.recipient))));
    assert!(!broker.vfs_caps.iter().any(|g| g
        .delivery
        .is_some_and(|d| d.recipient == delivery.recipient)));
    let attempt = TRANSPORT_ATTEMPTS.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1;
    crate::serial_println!("[HISTORY-GT] rollback attempt={} pid={} incarnation={} vm={} request={} history={} vfs_partial=0 proof=retained error=TokenHistoryFull cpu={}", attempt, delivery.recipient.pid, delivery.recipient.incarnation, delivery.recipient.vm_generation, delivery.request, broker.token_history.usage().0, crate::sched::current_cpu_id());
}

/// Observe the actual terminal IPC transition, before the old VM can resume.
pub(crate) fn pending_cancelled(
    target: crate::process::IpcReplyTarget,
    before: usize,
    broker: &CapabilityBroker,
    deadline: bool,
) {
    if target.call.pid != RACE_EXEC {
        return;
    }
    let state = EXEC_REPLY.lock();
    if let Some(identity) = state.0 {
        assert!(!state.1);
        assert_eq!(target.call.incarnation, identity.incarnation);
        assert!(deadline);
        assert_eq!(before, 2);
        assert_eq!(broker.recipient_grant_count(RACE_EXEC), 0);
        crate::serial_println!("[GRANT-G] exec-reply checkpoint=cancel pid={} incarnation={} vm={} request={} retired=2 terminal=deadline inventory=0 cpu={}", target.call.pid, target.call.incarnation, identity.vm_generation, target.call.generation, crate::sched::current_cpu_id());
    }
}
