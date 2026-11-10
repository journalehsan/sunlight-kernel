//! KFP-2A.6C: observation only. Lifecycle mutations, arm and dump use SCHEDULER.
//! The failed pre-lock probe and post-drop observation never reacquire it.
//! No allocation, IPC, serial, waiting or lock acquisition while recording.
use super::lifecycle_receipts::Receipts;
use super::{
    channel::{Channel, Error, Handle},
    delivery::RecipientIdentity,
    CapabilityBroker,
};
// Fixed 120 KiB receipt budget. Overflow remains a hard proof rejection.
const CAPACITY: usize = 1024;
static LOG: Receipts<CAPACITY> = Receipts::new();
pub(crate) const EXIT_STARTED: u64 = 1;
pub(crate) const EXIT_GUARD_ACQUIRED: u64 = 2;
pub(crate) const SWEEP_OBSERVED: u64 = 3;
pub(crate) const IDENTITY_VALIDATED: u64 = 4;
pub(crate) const RETIREMENT_COMMITTED: u64 = 5;
pub(crate) const ENDPOINT_CLOSED: u64 = 6;
pub(crate) const TERMINAL_CANDIDATE: u64 = 7;
pub(crate) const TERMINAL_COMMITTED: u64 = 8;
pub(crate) const CLEANUP_STARTED: u64 = 9;
pub(crate) const CLEANUP_COMPLETED: u64 = 10;
pub(crate) const ENDPOINT_CLOSE_ENTERED: u64 = 11;
pub(crate) const ENDPOINT_BROKER_REMOVED: u64 = 12;
pub(crate) const EXIT_NESTED_REJECTED: u64 = 13;
pub(crate) const EXIT_GUARD_RELEASED: u64 = 14;
pub(crate) const RETIRE_PROCESS_ENTERED: u64 = 15;
pub(crate) const TERMINAL_REJECTED: u64 = 16;
pub(crate) const RETIREMENT_CONTENDER_ACQUIRED: u64 = 17;
pub(crate) const RETIREMENT_CONTENDER_REJECTED: u64 = 18;
pub(crate) const RETIREMENT_CONTENDER_PROBED: u64 = 19;

pub(crate) fn arm(
    broker: &CapabilityBroker,
    h: Handle,
    who: RecipientIdentity,
    generation: u64,
    operation: u64,
) -> Result<(), Error> {
    let row = broker
        .channels
        .rows
        .iter()
        .find(|r| r.handle == h && r.creator == who && r.live())
        .ok_or(Error::Denied)?;
    let p = row.consumer.ok_or(Error::Incomplete)?;
    if !LOG.arm([
        generation,
        operation,
        p.pid as u64,
        p.incarnation,
        p.vm_generation,
        row.session.id,
        row.session.epoch,
        h.generation,
    ]) {
        return Err(Error::Duplicate);
    }
    Ok(())
}
fn record(event: u64, outcome: u64, state: u64, detail: u64) {
    if let Some(c) = LOG.context() {
        LOG.append(
            c[0],
            [
                event,
                crate::sched::current_cpu_id() as u64,
                outcome,
                state,
                detail,
                u64::from(x86_64::instructions::interrupts::are_enabled()),
            ],
        );
    }
}
pub(crate) fn process(who: RecipientIdentity, event: u64, outcome: u64, state: u64, detail: u64) {
    if LOG
        .context()
        .is_some_and(|c| [c[2], c[3], c[4]] == [who.pid as u64, who.incarnation, who.vm_generation])
    {
        record(event, outcome, state, detail);
    }
}
pub(super) fn row(row: &Channel, event: u64, outcome: u64, detail: u64) {
    if LOG.context().is_some_and(|c| {
        c[7] == row.handle.generation && c[5] == row.session.id && c[6] == row.session.epoch
    }) {
        record(event, outcome, row.state as u64, detail);
    }
}
pub(crate) fn participant(
    meta: (u64, sunlight_ipc::account_session::SessionRef, u64),
    who: RecipientIdentity,
    event: u64,
    outcome: u64,
    detail: u64,
) {
    // Identity observation concerns live-channel classification. After retirement
    // every actual terminal candidate/rejection is still captured by row().
    if meta.2 == super::channel::State::Retired as u64 {
        return;
    }

    if LOG.context().is_some_and(|c| {
        c[7] == meta.0
            && c[5] == meta.1.id
            && c[6] == meta.1.epoch
            && [c[2], c[3], c[4]] == [who.pid as u64, who.incarnation, who.vm_generation]
    }) {
        record(event, outcome, meta.2, detail);
    }
}
pub(crate) fn retirement_contender(p: &crate::process::Process, event: u64) {
    let who = RecipientIdentity::of(p);
    let old_vm = if p.state == crate::process::ProcessState::Reaped
        && !p.exit_cleanup_pending
        && p.kernel_stack.is_none()
    {
        Some(p.service_reap_receipt[0])
    } else {
        None
    };
    if LOG.context().is_some_and(|c| {
        super::lifecycle_receipts::same_owner(
            [c[2], c[3], c[4]],
            [who.pid as u64, who.incarnation, who.vm_generation],
            old_vm,
        )
    }) {
        record(
            event,
            u64::from(event == RETIREMENT_CONTENDER_REJECTED),
            p.state as u64,
            who.vm_generation,
        );
    }
}
pub(crate) fn retire_process(pid: usize) {
    if LOG.context().is_some_and(|c| c[2] == pid as u64) {
        record(RETIRE_PROCESS_ENTERED, 2, 0, 0);
    }
}
pub(crate) fn ready(
    broker: &CapabilityBroker,
    h: Handle,
    who: RecipientIdentity,
) -> Result<u64, Error> {
    let c = LOG.context().ok_or(Error::Denied)?;
    if !broker
        .channels
        .rows
        .iter()
        .any(|r| r.handle == h && r.creator == who)
        || c[7] != h.generation
    {
        return Err(Error::Denied);
    }
    Ok(u64::from((0..CAPACITY).any(|i| {
        LOG.read(i).is_some_and(|r| r[8] == CLEANUP_COMPLETED)
    })))
}
pub(crate) fn dump(
    broker: &CapabilityBroker,
    h: Handle,
    who: RecipientIdentity,
) -> Result<u64, Error> {
    if ready(broker, h, who)? != 1 {
        return Err(Error::Pending);
    }
    let c = LOG.context().ok_or(Error::Denied)?;
    if !broker
        .channels
        .rows
        .iter()
        .any(|r| r.handle == h && r.creator == who)
        || c[7] != h.generation
    {
        return Err(Error::Denied);
    }
    // Called by the surviving creator after waitpid; never called by a recording boundary.
    crate::serial_println!(
        "[KFP-LC-HEADER] generation={} operation={} overflow={} build={}",
        c[0],
        c[1],
        u8::from(LOG.overflowed()),
        env!("KFP2A6C_BUILD_ID")
    );
    let mut count = 0;
    for i in 0..CAPACITY {
        if let Some(r) = LOG.read(i) {
            crate::serial_println!("[KFP-LC] seq={} generation={} operation={} pid={} inc={} vm={} session={} epoch={} channel={} event={} cpu={} outcome={} state={} detail={} if={}",
            i+1,r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7],r[8],r[9],r[10],r[11],r[12],r[13]);
            count += 1;
        }
    }
    crate::serial_println!(
        "[KFP-LC-END] generation={} count={} overflow={}",
        c[0],
        count,
        u8::from(LOG.overflowed())
    );
    Ok(count)
}
