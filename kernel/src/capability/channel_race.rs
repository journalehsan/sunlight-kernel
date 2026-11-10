//! Bounded test timing only. Actual authenticated syscall actors contend for
//! the existing lifecycle lock; these hooks never attach/revoke/retire roles.
use super::{
    channel::{Error, Handle, State},
    delivery::RecipientIdentity,
    CapabilityBroker, CAP_BROKER,
};
use crate::sched::{Scheduler, SCHEDULER};
use core::sync::atomic::{AtomicU64, Ordering::*};
// Release observations are captured by the guard owner, independently of
// finished(). This module exists only in explicitly enabled debug fixtures.
static RELEASES: [[AtomicU64; 5]; 2] = [const { [const { AtomicU64::new(0) }; 5] }; 2];
pub(crate) struct ObservedGuard {
    guard: Option<spin::MutexGuard<'static, Scheduler>>,
    actor: u64,
    generation: u64,
    cpu: u64,
}
impl ObservedGuard {
    fn new(guard: spin::MutexGuard<'static, Scheduler>, actor: u64) -> Self {
        Self { guard: Some(guard), actor, generation: GENERATION.load(Acquire),
            cpu: crate::sched::current_cpu_id() as u64 }
    }
}
impl core::ops::Deref for ObservedGuard {
    type Target = Scheduler;
    fn deref(&self) -> &Scheduler { self.guard.as_ref().unwrap() }
}
impl core::ops::DerefMut for ObservedGuard {
    fn deref_mut(&mut self) -> &mut Scheduler { self.guard.as_mut().unwrap() }
}
impl Drop for ObservedGuard {
    fn drop(&mut self) {
        // The real spin guard performs its Release unlock first. No allocation,
        // logging, blocking acquisition or handshake belongs in this path.
        drop(self.guard.take());
        if self.actor != 0 {
            let receipt = &RELEASES[(self.actor - 1) as usize];
            receipt[1].store(self.generation, Relaxed);
            receipt[2].store(core::ptr::addr_of!(SCHEDULER) as u64, Relaxed);
            receipt[3].store(self.cpu, Relaxed);
            receipt[4].store(self.actor, Relaxed);
            // This is an immediate post-unlock observation, not the mutex's
            // internal unlock instruction. Consumers must retain that limit.
            receipt[0].store(SEQ.fetch_add(1, Relaxed), Release);
        }
    }
}
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
// Additional private continuation witness: PID, incarnation, VM, stack top,
// first pre/post-yield RSP, checked resumes. Printed only after both actors end.
static WAITS: [[AtomicU64; 17]; 8] = [const { [const { AtomicU64::new(0) }; 17] }; 8];
static READY: AtomicU64 = AtomicU64::new(0);
static READY_CPU: AtomicU64 = AtomicU64::new(u64::MAX);
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
fn wait(actor: u64, phase: &str, owner: RecipientIdentity, top: u64, mut f: impl FnMut() -> bool) {
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
    for (field, value) in trace[10..14].iter().zip([
        owner.pid as u64,
        owner.incarnation,
        owner.vm_generation,
        top,
    ]) {
        field.store(value, Relaxed);
    }
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
        // SAFETY: no live guard; ordinary entry owns this private stack.
        // The helper retains it and verifies ownership before/after scheduling.
        let before: u64;
        let after: u64;
        unsafe {
            core::arch::asm!("mov {}, rsp", out(reg) before, options(nomem, nostack, preserves_flags));
            assert_eq!(
                yield_on_kernel_stack(owner.incarnation, owner.vm_generation),
                top
            );
            core::arch::asm!("mov {}, rsp", out(reg) after, options(nomem, nostack, preserves_flags));
        }
        assert_eq!(before, after, "M1 continuation stack pointer changed");
        trace[16].store(attempt + 1, Relaxed);
        let flags = u64::from(x86_64::instructions::interrupts::are_enabled());
        assert_eq!(flags, 0, "M1 continuation resumed with IF set");
        if attempt == 0 {
            trace[14].store(before, Relaxed);
            trace[15].store(after, Relaxed);
            trace[4].store(crate::sched::current_cpu_id() as u64, Relaxed);
            trace[7].store(flags, Relaxed);
        }
    }
    panic!("channel race exhausted");
}

// Ordinary syscall entry already owns this task's private kernel stack.
// Retain the live dispatcher and its return chain below RSP. In particular,
// never pivot back to stack_top - 256: that is occupied by syscall_entry.
// RBX carries the owned stack top across the ordinary scheduler
// handoff; no scheduler/broker guard survives the interrupt.
#[unsafe(naked)]
unsafe extern "C" fn yield_on_kernel_stack(_incarnation: u64, _vm: u64) -> u64 {
    core::arch::naked_asm!(
        "push rbx",
        "mov rdx, rsp",
        "call {prepare}",
        "mov rbx, rax",
        "int {vector}",
        "mov rdi, rsp",
        "mov rsi, rbx",
        "call {resume}",
        "pop rbx",
        "ret",
        prepare = sym prepare_yield,
        resume = sym resume_yield,
        vector = const crate::arch::x86_64::interrupts::RESCHEDULE_VECTOR,
    );
}
fn assert_private_continuation(p: &crate::process::Process, rsp: u64) {
    crate::arch::x86_64::syscall_stack::validate(p);
    let top = p.kernel_stack_top;
    assert!(
        rsp >= top - crate::process::KERNEL_STACK_SIZE as u64 && rsp < top - 256,
        "M1 yield cannot overwrite a live private-stack continuation"
    );
    assert_eq!(p.state, crate::process::ProcessState::Running);
    assert_eq!(p.owning_core as usize, crate::sched::current_cpu_id());
    assert_eq!(p.queued_on_core, u8::MAX);
    assert_ne!(p.foreground.incarnation, 0);
    assert!(!x86_64::instructions::interrupts::are_enabled());
}
extern "C" fn prepare_yield(incarnation: u64, vm: u64, rsp: u64) -> u64 {
    let mut s = SCHEDULER.lock();
    let p = s.current_process_mut();
    assert_private_continuation(p, rsp);
    assert_eq!(
        p.foreground.incarnation, incarnation,
        "M1 continuation owner changed"
    );
    assert_eq!(
        p.address_space.identity().generation,
        vm,
        "M1 continuation VM changed"
    );
    let top = p.kernel_stack_top;
    p.state = crate::process::ProcessState::Ready;
    crate::sched::request_reschedule();
    top
}
extern "C" fn resume_yield(rsp: u64, top: u64) -> u64 {
    // A lifecycle lock may now be held by the other actor. Do not acquire it
    // before the caller's real contention probe. The saved top is supervisor
    // state retained in RBX by this exact continuation, not a user argument.
    assert!(
        rsp >= top - crate::process::KERNEL_STACK_SIZE as u64 && rsp < top - 256,
        "M1 resumed outside its private-stack continuation"
    );
    assert!(!x86_64::instructions::interrupts::are_enabled());
    top
}
pub(super) fn arm(
    b: &CapabilityBroker,
    h: Handle,
    who: RecipientIdentity,
    mode: u64,
) -> Result<(), Error> {
    if !((1..=4).contains(&mode) || (cfg!(feature = "fastlane_lifecycle_trace") && mode == 6)) || FINISHED.load(Acquire) != 3 {
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
    READY_CPU.store(u64::MAX, Relaxed);
    OBSERVATIONS.store(0, Relaxed);
    PROBE.store(0, Relaxed);
    WINNER_RETURN.store(0, Relaxed);
    for trace in &WAITS {
        for field in trace {
            field.store(0, Relaxed);
        }
    }
    for receipt in &RELEASES {
        for field in receipt { field.store(0, Relaxed); }
    }
    FINISHED.store(0, Relaxed);
    for cpu in &CPUS {
        cpu.store(u64::MAX, Relaxed);
    }
    MODE.store(mode, Release);
    event("armed", 0, who, 0);
    Ok(())
}
pub(crate) fn acquire(op: u64, generation: u64) -> (ObservedGuard, u64) {
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
    } else if (op == 3 || (mode == 6 && op == 21 && generation == load(&C).pid as u64)) && who == load(&P) {
        1
    } else if op == 20 && who == load(&C) {
        2
    } else {
        0
    };
    if actor == 0
        || mode == 0
        || FINISHED.load(Acquire) & actor != 0
        || !matches!(op,20|21) && generation != GENERATION.load(Relaxed)
    {
        return (ObservedGuard::new(sched, 0), 0);
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
    let stack_top = sched.current_process().kernel_stack_top;
    drop(sched);
    assert_eq!(ENTERED.fetch_or(actor, AcqRel) & actor, 0);
    wait(actor, "entry", who, stack_top, || {
        ENTERED.load(Acquire) == 3
    });
    assert_ne!(
        CPUS[0].load(Acquire),
        CPUS[1].load(Acquire),
        "channel actors did not overlap across CPUs"
    );
    let winner = if mode % 2 == 1 { 1 } else { 2 };
    if actor == winner {
        // READY now certifies an IF-clear remote continuation which will not
        // yield, take a blocking lock, or log before publishing its probe.
        wait(actor, "ready", who, stack_top, || READY.load(Acquire) != 0);
        let sched = SCHEDULER.lock();
        assert_eq!(RecipientIdentity::of(sched.current_process()), who);
        assert!(!x86_64::instructions::interrupts::are_enabled());
        assert_ne!(READY_CPU.load(Acquire), crate::sched::current_cpu_id() as u64);
        let held_seq = event("lock-held", actor, who, 0);
        HELD.store(actor, Release);
        // The remote path below depends only on these atomics and a failing
        // nonblocking probe. In particular it does NOT call prepare_yield,
        // which takes SCHEDULER. The finite budget is a failure bound, never
        // evidence of contention. No terminal operation runs on timeout.
        boundary_wait(|| CONTENDED.load(Acquire) != 0);
        assert!(PROBE.load(Acquire) > held_seq);
        (ObservedGuard::new(sched, actor), actor)
    } else {
        assert!(!x86_64::instructions::interrupts::are_enabled());
        READY_CPU.store(crate::sched::current_cpu_id() as u64, Relaxed);
        READY.store(actor, Release);
        // Once READY is visible, this CPU cannot migrate or enter scheduling
        // before the witness. Do not add serial output or a yielding wait.
        boundary_wait(|| HELD.load(Acquire) != 0);
        let contended = SCHEDULER.try_lock().is_none();
        let seq = SEQ.fetch_add(1, Relaxed);
        if contended {
            #[cfg(all(debug_assertions, feature = "fastlane_lifecycle_trace"))]
            if mode == 6 {
                super::lifecycle_trace::process(load(&C),super::lifecycle_trace::RETIREMENT_CONTENDER_PROBED,1,0,seq);
            }
            PROBE.store(seq, Release);
            CONTENDED.store(actor, Release);
        }
        // Acknowledge before taking the serial lock: the holder's progress
        // must not depend on serial output from this CPU.
        crate::serial_println!("[FL-RACE] seq={} event={} generation={} mode={} actor={} pid={} inc={} vm={} result=0 cpu={}",
            seq,
            if contended {
                "lock-contended"
            } else {
                "lock-missed"
            },
            GENERATION.load(Relaxed), mode, actor, who.pid, who.incarnation,
            who.vm_generation, crate::sched::current_cpu_id());
        // Ordinary serialization decides the result; the winner never waits
        // for this observation. A missed contention window fails explicitly.
        let mut acquired = None;
        wait(actor, "unlock", who, stack_top, || {
            acquired = SCHEDULER.try_lock();
            acquired.is_some()
        });
        assert_ne!(FINISHED.load(Acquire) & winner, 0);
        assert_eq!(
            RecipientIdentity::of(acquired.as_ref().unwrap().current_process()),
            who
        );
        (ObservedGuard::new(acquired.unwrap(), actor), actor)
    }
}

// Both uses have a finite instruction budget even if the timekeeper CPU is
// unavailable. No scheduler, allocator, serial, broker, IRQ or wakeup dependency.
// Budget exhaustion is COORDINATION_BLOCKED, not a contention witness.
fn boundary_wait(mut ready: impl FnMut() -> bool) {
    for _ in 0..100_000_000 {
        if ready() {
            return;
        }
        core::hint::spin_loop();
    }
    panic!("COORDINATION_BLOCKED: remote M1 acquisition boundary unavailable");
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
            let v: [u64; 17] = core::array::from_fn(|i| trace[i].load(Relaxed));
            observation(format_args!("event=wait generation={} actor={} phase={} start={} deadline={} entry_cpu={} exit_cpu={} resume_cpu={} yields={} entry_if={} resume_if={} end={} status={} entry_sched_owned=0 yield_sched_owned=0 exit_sched_owned={} broker_owned=0 untraced_yields={} pid={} inc={} vm={} stack_top={} rsp_before={} rsp_after={} checked_resumes={}",
                GENERATION.load(Relaxed), index / 4 + 1, index % 4, v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8], v[9], u8::from(index % 4 == 3), v[5].saturating_sub(1), v[10], v[11], v[12], v[13], v[14], v[15], v[16]));
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
    for receipt in &RELEASES {
        let seq = receipt[0].load(Acquire);
        let v: [u64; 4] = core::array::from_fn(|i| receipt[i + 1].load(Relaxed));
        crate::serial_println!("[FL-UNLOCK] seq={} generation={} lock={} cpu={} actor={} boundary=post_guard_drop", seq, v[0], v[1], v[2], v[3]);
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
