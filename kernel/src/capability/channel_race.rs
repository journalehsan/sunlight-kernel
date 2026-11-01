//! Bounded test timing only. Actual authenticated syscall actors contend for
//! the existing lifecycle lock; these hooks never attach/revoke/retire roles.
use super::{
    channel::{Error, Handle, State},
    delivery::RecipientIdentity,
    CapabilityBroker, CAP_BROKER,
};
use crate::sched::{Scheduler, SCHEDULER};
use core::sync::atomic::{AtomicU64, Ordering::*};
static MODE: AtomicU64 = AtomicU64::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static P: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static C: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static ENTERED: AtomicU64 = AtomicU64::new(0);
static HELD: AtomicU64 = AtomicU64::new(0);
static CONTENDED: AtomicU64 = AtomicU64::new(0);
static FINISHED: AtomicU64 = AtomicU64::new(3);
static CPUS: [AtomicU64; 2] = [const { AtomicU64::new(u64::MAX) }; 2];
static SEQ: AtomicU64 = AtomicU64::new(1);
fn event(event: &str, actor: u64, who: RecipientIdentity, result: u64) {
    crate::serial_println!("[FL-RACE] seq={} event={} generation={} mode={} actor={} pid={} inc={} vm={} result={} cpu={}",
        SEQ.fetch_add(1,Relaxed),event,GENERATION.load(Relaxed),MODE.load(Relaxed),actor,
        who.pid,who.incarnation,who.vm_generation,result,crate::sched::current_cpu_id());
}
fn load(a: &[AtomicU64; 3]) -> RecipientIdentity {
    RecipientIdentity {
        pid: a[0].load(Relaxed) as usize,
        incarnation: a[1].load(Relaxed),
        vm_generation: a[2].load(Relaxed),
    }
}
fn save(a: &[AtomicU64; 3], p: RecipientIdentity) {
    for (v, n) in a.iter().zip([p.pid as u64, p.incarnation, p.vm_generation]) {
        v.store(n, Relaxed);
    }
}
fn wait(mut f: impl FnMut() -> bool) {
    let start = crate::timekeeping::monotonic_ms();
    for _ in 0..100_000_000 {
        if f() {
            return;
        }
        assert!(
            crate::timekeeping::monotonic_ms().saturating_sub(start) < 5000,
            "channel race deadline"
        );
        core::hint::spin_loop();
    }
    panic!("channel race exhausted");
}
pub(super) fn arm(
    b: &CapabilityBroker,
    h: Handle,
    who: RecipientIdentity,
    mode: u64,
) -> Result<(), Error> {
    if !(1..=4).contains(&mode) || FINISHED.load(Acquire) != 3 {
        return Err(Error::Denied);
    }
    let row = b
        .channels
        .rows
        .iter()
        .find(|r| r.handle == h && r.creator == who && r.state == State::Created)
        .ok_or(Error::Denied)?;
    let c = row.consumer.ok_or(Error::Incomplete)?;
    save(&P, who);
    save(&C, c);
    GENERATION.store(h.generation, Relaxed);
    ENTERED.store(0, Relaxed);
    HELD.store(0, Relaxed);
    CONTENDED.store(0, Relaxed);
    FINISHED.store(0, Relaxed);
    for cpu in &CPUS {
        cpu.store(u64::MAX, Relaxed);
    }
    MODE.store(mode, Release);
    event("armed", 0, who, 0);
    Ok(())
}
pub(crate) fn acquire(op: u64, generation: u64) -> (spin::MutexGuard<'static, Scheduler>, u64) {
    let sched = SCHEDULER.lock();
    let who = RecipientIdentity::of(sched.current_process());
    let mode = MODE.load(Acquire);
    let actor = if mode <= 2 {
        if op == 3 && who == load(&C) {
            1
        } else if op == 4 && who == load(&P) {
            2
        } else {
            0
        }
    } else if op == 3 && who == load(&P) {
        1
    } else if op == 20 && who == load(&C) {
        2
    } else {
        0
    };
    if actor == 0
        || mode == 0
        || FINISHED.load(Acquire) & actor != 0
        || op != 20 && generation != GENERATION.load(Relaxed)
    {
        return (sched, 0);
    }
    event(
        if op == 20 { "exit-syscall" } else { "entered" },
        actor,
        who,
        0,
    );
    CPUS[(actor - 1) as usize].store(crate::sched::current_cpu_id() as u64, Release);
    drop(sched);
    assert_eq!(ENTERED.fetch_or(actor, AcqRel) & actor, 0);
    wait(|| ENTERED.load(Acquire) == 3);
    assert_ne!(
        CPUS[0].load(Acquire),
        CPUS[1].load(Acquire),
        "channel actors did not overlap across CPUs"
    );
    let winner = if mode % 2 == 1 { 1 } else { 2 };
    if actor == winner {
        let sched = SCHEDULER.lock();
        event("lock-held", actor, who, 0);
        HELD.store(actor, Release);
        wait(|| CONTENDED.load(Acquire) != 0);
        (sched, actor)
    } else {
        wait(|| HELD.load(Acquire) != 0);
        assert!(SCHEDULER.try_lock().is_none(), "channel contention missing");
        event("lock-contended", actor, who, 0);
        CONTENDED.store(actor, Release);
        wait(|| FINISHED.load(Acquire) & winner != 0);
        (SCHEDULER.lock(), actor)
    }
}
pub(crate) fn finished(actor: u64, result: u64) {
    if actor == 0 {
        return;
    }
    let mode = MODE.load(Relaxed);
    let who = if (mode <= 2 && actor == 1) || (mode >= 3 && actor == 2) {
        load(&C)
    } else {
        load(&P)
    };
    let broker = CAP_BROKER.lock();
    let row = broker
        .channels
        .rows
        .iter()
        .find(|r| r.handle.generation == GENERATION.load(Relaxed))
        .expect("race channel retained");
    event("returned", actor, who, result);
    crate::serial_println!("[FL-OUTCOME] seq={} generation={} actor={} state={} terminal={} producer={} consumer={} cpu={}",
        SEQ.fetch_add(1,Relaxed),GENERATION.load(Relaxed),actor,row.state as u64,row.terminal.map_or(0,|t|t as u64),
        u8::from(row.producer_role==super::channel::RoleState::Attached),u8::from(row.consumer_role==super::channel::RoleState::Attached),crate::sched::current_cpu_id());
    FINISHED.fetch_or(actor, Release);
}
