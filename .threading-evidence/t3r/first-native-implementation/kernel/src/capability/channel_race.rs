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
static PROBE: AtomicU64 = AtomicU64::new(0);
static WINNER_RETURN: AtomicU64 = AtomicU64::new(0);
static OBSERVATIONS: AtomicU64 = AtomicU64::new(0);
// Per actor/phase: start, deadline, entry/exit/first-resume CPU, yield count,
// entry/resume IF, end, status (0 unused, 1 waiting, 2 complete).
static WAITS: [[AtomicU64; 10]; 8] = [const { [const { AtomicU64::new(0) }; 10] }; 8];
static READY: AtomicU64 = AtomicU64::new(0);
static SEQ: AtomicU64 = AtomicU64::new(1);
fn event(event: &str, actor: u64, who: RecipientIdentity, result: u64) -> u64 {
    let seq = SEQ.fetch_add(1, Relaxed);
    crate::serial_println!("[FL-RACE] seq={} event={} generation={} mode={} actor={} pid={} inc={} vm={} result={} cpu={}",
        seq,event,GENERATION.load(Relaxed),MODE.load(Relaxed),actor,
        who.pid,who.incarnation,who.vm_generation,result,crate::sched::current_cpu_id());
    seq
}
// Per-generation budget. Four modes, two actors, and four wait phases
// bound the normal records further. Overflow is never silently discarded.
fn observation(args: core::fmt::Arguments<'_>) {
    let n = OBSERVATIONS.fetch_add(1, Relaxed);
    if n < 24 {
        crate::serial_println!("[FL-COORD] {}", args);
    } else if n == 24 {
        crate::serial_println!(
            "[FL-COORD] event=overflow generation={} limit=24",
            GENERATION.load(Relaxed)
        );
    }
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
// No guard is held at a yield. Keep scalar observations in actor-private,
// fixed slots: serial output here can itself make the contention window vanish.
fn wait(actor: u64, phase: &str, mut f: impl FnMut() -> bool) {
    #[cfg(feature = "fastlane_coord_spin")]
    if MODE.load(Relaxed) == 2 && phase == "entry" {
        // Exact old entry behavior: IF-clear atomic spin, five-second deadline,
        // 100M iteration bound; no scheduler yield or lifecycle guard.
        let start = crate::timekeeping::monotonic_ms();
        diagnostic_snapshot("spin-start");
        for _ in 0..100_000_000 {
            if f() {
                diagnostic_snapshot("spin-complete");
                return;
            }
            if crate::timekeeping::monotonic_ms().saturating_sub(start) >= 5000 {
                diagnostic_snapshot("spin-deadline");
                crate::serial_println!("[FL-RACE-DEADLINE] entered={} cpus=({},{}) current_cpu={} mode=2 held={} contended={} finished={} actor={} phase=entry deadline={} if={} sched_owned=0 broker_owned=0",
                    ENTERED.load(Acquire), CPUS[0].load(Acquire), CPUS[1].load(Acquire), crate::sched::current_cpu_id(),
                    HELD.load(Acquire), CONTENDED.load(Acquire), FINISHED.load(Acquire), actor, start+5000,
                    u8::from(x86_64::instructions::interrupts::are_enabled()));
                panic!("channel race deadline");
            }
            core::hint::spin_loop();
        }
        panic!("channel race exhausted");
    }
    let phase_id = match phase {
        "entry" => 0,
        "ready" => 1,
        "held" => 2,
        "unlock" => 3,
        _ => unreachable!(),
    };
    let trace = &WAITS[(actor as usize - 1) * 4 + phase_id];
    assert_eq!(trace[9].load(Relaxed), 0, "duplicate coordination wait");
    let start = crate::timekeeping::monotonic_ms();
    trace[0].store(start, Relaxed);
    trace[1].store(start + 5000, Relaxed);
    trace[2].store(crate::sched::current_cpu_id() as u64, Relaxed);
    trace[4].store(u64::MAX, Relaxed);
    trace[6].store(
        u64::from(x86_64::instructions::interrupts::are_enabled()),
        Relaxed,
    );
    trace[7].store(u64::MAX, Relaxed);
    trace[9].store(1, Release);
    for attempt in 0u64..100_000 {
        if crate::timekeeping::monotonic_ms().saturating_sub(start) >= 5000 {
            crate::serial_println!("[FL-RACE-DEADLINE] entered={} cpus=({},{}) current_cpu={} mode={} held={} contended={} finished={} actor={} phase={} deadline={} yields={}",
                ENTERED.load(Acquire), CPUS[0].load(Acquire), CPUS[1].load(Acquire),
                crate::sched::current_cpu_id(), MODE.load(Relaxed), HELD.load(Acquire),
                CONTENDED.load(Acquire), FINISHED.load(Acquire), actor, phase, start + 5000, attempt);
            panic!("channel race deadline");
        }
        if f() {
            trace[3].store(crate::sched::current_cpu_id() as u64, Relaxed);
            trace[5].store(attempt, Relaxed);
            trace[8].store(crate::timekeeping::monotonic_ms(), Relaxed);
            trace[9].store(2, Release);
            return;
        }
        // SAFETY: no live guard, process is live, and this helper owns the
        // otherwise unused private kernel stack until this invocation resumes.
        unsafe {
            yield_on_kernel_stack();
        }
        let flags = u64::from(x86_64::instructions::interrupts::are_enabled());
        assert_eq!(flags, 0, "M1 continuation resumed with IF set");
        if attempt == 0 {
            trace[4].store(crate::sched::current_cpu_id() as u64, Relaxed);
            trace[7].store(flags, Relaxed);
        }
    }
    panic!("channel race exhausted");
}

// Same private-stack lifetime and saved-IF contract as doorbell_stack, but
// only the yield executes there. All channel/exit authority stays on its
// existing path. Callee-saved RBX and the original stack are restored only
// after this same process resumes; no userspace data pointer is dereferenced.
#[unsafe(naked)]
unsafe extern "C" fn yield_on_kernel_stack() {
    core::arch::naked_asm!(
        "push rbx",
        "mov rdi, rsp",
        "call {stack_top}",
        "mov rbx, rsp",
        "mov rsp, rax",
        "sub rsp, 256",
        "and rsp, -16",
        "push rbx",
        "sub rsp, 8",
        "call {prepare}",
        "int {vector}",
        "add rsp, 8",
        "pop rsp",
        "pop rbx",
        "ret",
        stack_top = sym stack_top,
        prepare = sym prepare_yield,
        vector = const crate::arch::x86_64::interrupts::RESCHEDULE_VECTOR,
    );
}
extern "C" fn stack_top(old_rsp: u64) -> u64 {
    let top = SCHEDULER.lock().current_process().kernel_stack_top;
    assert_ne!(top, 0);
    assert!(
        old_rsp < top - crate::process::KERNEL_STACK_SIZE as u64 || old_rsp >= top,
        "M1 yield cannot overwrite a live private-stack continuation"
    );
    top
}
extern "C" fn prepare_yield() {
    let mut s = SCHEDULER.lock();
    let p = s.current_process_mut();
    assert_eq!(p.state, crate::process::ProcessState::Running);
    p.state = crate::process::ProcessState::Ready;
    crate::sched::request_reschedule();
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
    READY.store(0, Relaxed);
    OBSERVATIONS.store(0, Relaxed);
    PROBE.store(0, Relaxed);
    WINNER_RETURN.store(0, Relaxed);
    for trace in &WAITS {
        for field in trace {
            field.store(0, Relaxed);
        }
    }
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
    // Eight bounded snapshots per fixture (two per mode). No tokens or
    // authorization material; the scheduler guard makes eligibility coherent.
    let peer = if who == load(&P) { load(&C) } else { load(&P) };
    if let Some(p) = sched
        .processes
        .iter()
        .find(|p| RecipientIdentity::of(p) == peer)
    {
        observation(format_args!("event=entry generation={} mode={} actor={} pid={} inc={} vm={} cpu={} if={} sched_owned=1 broker_owned=0 peer={} peer_inc={} peer_vm={} peer_state={:?} peer_mask={} peer_owner={} peer_queue={} deadline_ms=5000",
            GENERATION.load(Relaxed), mode, actor, who.pid, who.incarnation, who.vm_generation,
            crate::sched::current_cpu_id(), u8::from(x86_64::instructions::interrupts::are_enabled()),
            peer.pid, peer.incarnation, peer.vm_generation, p.state, p.cpu_mask, p.owning_core, p.queued_on_core));
        let own_mask = sched.current_process().cpu_mask;
        #[cfg(feature = "fastlane_coord_diag")]
        if mode == 2 {
            diagnostic_locked("entry", &sched);
        }
        assert!(
            !(own_mask.count_ones() == 1 && own_mask == p.cpu_mask)
                || cfg!(feature = "fastlane_coord_spin"),
            "channel race placement impossible: both actors require one CPU"
        );
    } else {
        panic!("channel race peer missing");
    }
    drop(sched);
    assert_eq!(ENTERED.fetch_or(actor, AcqRel) & actor, 0);
    wait(actor, "entry", || ENTERED.load(Acquire) == 3);
    assert_ne!(
        CPUS[0].load(Acquire),
        CPUS[1].load(Acquire),
        "channel actors did not overlap across CPUs"
    );
    let winner = if mode % 2 == 1 { 1 } else { 2 };
    if actor == winner {
        // The contender is independently running before lock acquisition.
        // No peer wait occurs while the lifecycle lock is owned.
        wait(actor, "ready", || READY.load(Acquire) != 0);
        let sched = SCHEDULER.lock();
        assert_eq!(RecipientIdentity::of(sched.current_process()), who);
        HELD.store(actor, Release);
        event("lock-held", actor, who, 0);
        (sched, actor)
    } else {
        READY.store(actor, Release);
        wait(actor, "held", || HELD.load(Acquire) != 0);
        let contended = SCHEDULER.try_lock().is_none();
        let seq = event(
            if contended {
                "lock-contended"
            } else {
                "lock-missed"
            },
            actor,
            who,
            0,
        );
        if contended {
            PROBE.store(seq, Release);
            CONTENDED.store(actor, Release);
        }
        // Ordinary serialization decides the result; the winner never waits
        // for this observation. A missed contention window fails explicitly.
        let mut acquired = None;
        wait(actor, "unlock", || {
            acquired = SCHEDULER.try_lock();
            acquired.is_some()
        });
        assert_ne!(FINISHED.load(Acquire) & winner, 0);
        assert_eq!(
            RecipientIdentity::of(acquired.as_ref().unwrap().current_process()),
            who
        );
        (acquired.unwrap(), actor)
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
    let seq = event("returned", actor, who, result);
    if actor == if mode % 2 == 1 { 1 } else { 2 } {
        WINNER_RETURN.store(seq, Release);
    }
    crate::serial_println!("[FL-OUTCOME] seq={} generation={} actor={} state={} terminal={} producer={} consumer={} cpu={}",
        SEQ.fetch_add(1,Relaxed),GENERATION.load(Relaxed),actor,row.state as u64,row.terminal.map_or(0,|t|t as u64),
        u8::from(row.producer_role==super::channel::RoleState::Attached),u8::from(row.consumer_role==super::channel::RoleState::Attached),crate::sched::current_cpu_id());
    if (FINISHED.fetch_or(actor, AcqRel) | actor) == 3 {
        for (index, trace) in WAITS.iter().enumerate() {
            if trace[9].load(Acquire) == 0 {
                continue;
            }
            let v: [u64; 10] = core::array::from_fn(|i| trace[i].load(Relaxed));
            observation(format_args!("event=wait generation={} actor={} phase={} start={} deadline={} entry_cpu={} exit_cpu={} resume_cpu={} yields={} entry_if={} resume_if={} end={} status={} entry_sched_owned=0 yield_sched_owned=0 exit_sched_owned={} broker_owned=0 untraced_yields={}",
                GENERATION.load(Relaxed), index / 4 + 1, index % 4, v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8], v[9], u8::from(index % 4 == 3), v[5].saturating_sub(1)));
        }
        let n = OBSERVATIONS.load(Relaxed);
        crate::serial_println!("[FL-COORD] event=complete generation={} mode={} entries={} held={} contended={} finished=3 records={} overflow={} missing={} if={} cpu={} qualified={}",
            GENERATION.load(Relaxed), mode, ENTERED.load(Acquire), HELD.load(Acquire), CONTENDED.load(Acquire),
            n.min(24), u8::from(n > 24), n.saturating_sub(24),
            u8::from(x86_64::instructions::interrupts::are_enabled()), crate::sched::current_cpu_id(), u8::from(qualified()));
    }
}

fn qualified() -> bool {
    let probe = PROBE.load(Acquire);
    probe != 0 && probe < WINNER_RETURN.load(Acquire)
}
// Non-authorizing test receipt, readable only by the exact channel creator.
// A missed natural contention window is explicit and never counts as coverage.
pub(super) fn observed(
    b: &CapabilityBroker,
    h: Handle,
    who: RecipientIdentity,
) -> Result<bool, Error> {
    if GENERATION.load(Acquire) != h.generation
        || FINISHED.load(Acquire) != 3
        || !b
            .channels
            .rows
            .iter()
            .any(|row| row.handle == h && row.creator == who)
    {
        return Err(Error::Denied);
    }
    Ok(qualified())
}

// Only compiled with an explicit debug diagnostic feature. Scalar observations
// of real actors; no scheduling policy, authority or persistent lock added.
#[cfg(feature = "fastlane_coord_diag")]
static CPU0_DISPATCHES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "fastlane_coord_diag")]
static CPU0_PID: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "fastlane_coord_diag")]
static CPU0_IF: AtomicU64 = AtomicU64::new(u64::MAX);
#[cfg(feature = "fastlane_coord_diag")]
pub(crate) fn diagnostic_dispatch(pid: usize, cpu: usize) {
    if cpu == 0 {
        CPU0_PID.store(pid as u64, Relaxed);
        CPU0_IF.store(
            u64::from(x86_64::instructions::interrupts::are_enabled()),
            Relaxed,
        );
        CPU0_DISPATCHES.fetch_add(1, Release);
    }
}
#[cfg(feature = "fastlane_coord_diag")]
fn diagnostic_locked(event: &str, s: &Scheduler) {
    for (role, who) in [("producer", load(&P)), ("consumer", load(&C))] {
        if let Some((idx, p)) = s
            .processes
            .iter()
            .enumerate()
            .find(|(_, p)| RecipientIdentity::of(p) == who)
        {
            crate::serial_println!("[FL-COORD-DIAG] event={} role={} generation={} pid={} inc={} vm={} state={:?} mask={} owner={} queue={} occurrences={} cpu0_current={} cpu0_dispatches={} cpu0_last_pid={} cpu0_dispatch_if={} observer_cpu={} observer_if={} sched_owned=1 broker_owned=0 ms={}",
                event, role, GENERATION.load(Relaxed), who.pid, who.incarnation, who.vm_generation,
                p.state, p.cpu_mask, p.owning_core, p.queued_on_core, s.diagnostic_ready_occurrences(idx),
                s.diagnostic_current_pid(0).unwrap_or(0), CPU0_DISPATCHES.load(Acquire), CPU0_PID.load(Relaxed), CPU0_IF.load(Relaxed),
                crate::sched::current_cpu_id(), u8::from(x86_64::instructions::interrupts::are_enabled()), crate::timekeeping::monotonic_ms());
        }
    }
}
#[cfg(feature = "fastlane_coord_spin")]
fn diagnostic_snapshot(event: &str) {
    crate::serial_println!("[FL-COORD-DIAG] event={} cpu0_dispatches={} cpu0_last_pid={} cpu0_dispatch_if={} observer_cpu={} observer_if={} ms={} sched_owned=0 broker_owned=0",
        event, CPU0_DISPATCHES.load(Acquire), CPU0_PID.load(Relaxed), CPU0_IF.load(Relaxed),
        crate::sched::current_cpu_id(), u8::from(x86_64::instructions::interrupts::are_enabled()), crate::timekeeping::monotonic_ms());
    if let Some(s) = SCHEDULER.try_lock() {
        diagnostic_locked(event, &s);
    } else {
        crate::serial_println!(
            "[FL-COORD-DIAG] event={} snapshot=unavailable sched_owned=other-or-transient",
            event
        );
    }
}

#[cfg(feature = "fastlane_coord_diag")]
pub(crate) fn diagnostic_arm(s: &mut Scheduler) {
    // Called only after the ordinary exact-creator arm succeeded. Placement
    // changes only these two authenticated actors, before the IPC release.
    let mask = if cfg!(feature = "fastlane_coord_same_cpu") {
        2
    } else {
        u64::MAX
    };
    for who in [load(&P), load(&C)] {
        let p = s
            .processes
            .iter_mut()
            .find(|p| RecipientIdentity::of(p) == who)
            .expect("armed actor");
        p.cpu_mask = mask;
    }
    CPU0_DISPATCHES.store(0, Release);
    CPU0_PID.store(0, Relaxed);
    CPU0_IF.store(u64::MAX, Relaxed);
    crate::sched::request_park_after_sysret();
    crate::serial_println!("[FL-COORD-CONTROL] mode=2 producer_mask={} consumer_mask={} artificial={} spin={} deadline_ms=5000",
        mask, mask, u8::from(cfg!(feature = "fastlane_coord_same_cpu")), u8::from(cfg!(feature = "fastlane_coord_spin")));
    diagnostic_locked("armed", s);
}
