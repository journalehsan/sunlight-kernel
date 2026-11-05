//! Debug-image scheduling coordination for the two real syscall owners.
//! Never publishes a terminal, settles a grant, or changes an authority check.
//! The existing scheduler lock remains the sole lifecycle serialization lock.
#[cfg(not(debug_assertions))]
compile_error!("authenticated_completion_exit_test requires a debug test image");

#[cfg(feature = "authenticated_completion_race_test")]
compile_error!("select one authenticated race fixture per debug image");

use crate::process::IpcReplyTarget;
use crate::sched::{Scheduler, SCHEDULER};
use core::sync::atomic::{AtomicU64, Ordering};

pub(crate) const COMPLETE: u64 = 1;
pub(crate) const EXIT: u64 = 2;
static EXIT_SYSCALL_CPU: AtomicU64 = AtomicU64::new(u64::MAX);
static MODE: AtomicU64 = AtomicU64::new(0);
static OBJECT: AtomicU64 = AtomicU64::new(0);
static ISSUER: AtomicU64 = AtomicU64::new(0);
static ISSUER_INC: AtomicU64 = AtomicU64::new(0);
static CALLER: AtomicU64 = AtomicU64::new(0);
static CALLER_INC: AtomicU64 = AtomicU64::new(0);
static ENDPOINT: AtomicU64 = AtomicU64::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static ENTERED: AtomicU64 = AtomicU64::new(0);
static WINNER: AtomicU64 = AtomicU64::new(0);
static HELD: AtomicU64 = AtomicU64::new(0);
static CONTENDED: AtomicU64 = AtomicU64::new(0);
static FINISHED: AtomicU64 = AtomicU64::new(0);
static COMPLETE_CPU: AtomicU64 = AtomicU64::new(u64::MAX);
static EXIT_CPU: AtomicU64 = AtomicU64::new(u64::MAX);

fn wait(mut ready: impl FnMut() -> bool) {
    let start = crate::timekeeping::monotonic_ms();
    for _ in 0..100_000_000 {
        if ready() {
            return;
        }
        assert!(
            crate::timekeeping::monotonic_ms().saturating_sub(start) < 5000,
            "completion/caller-exit rendezvous timeout"
        );
        core::hint::spin_loop();
    }
    panic!("completion/caller-exit rendezvous exhausted");
}

fn event(event: &str, actor: u64, pid: usize, inc: u64, result: u64) {
    crate::serial_println!("[EX-RACE] seq={} object={} event={} actor={} pid={} inc={} caller={} caller_inc={} endpoint={} generation={} mode={} winner={} result={} cpu={}",
        super::exec_retirement_trace::seq(), OBJECT.load(Ordering::Relaxed), event, actor,
        pid, inc, CALLER.load(Ordering::Relaxed), CALLER_INC.load(Ordering::Relaxed),
        ENDPOINT.load(Ordering::Relaxed), GENERATION.load(Ordering::Relaxed),
        MODE.load(Ordering::Relaxed), WINNER.load(Ordering::Relaxed), result,
        crate::sched::current_cpu_id());
}

/// Account alone arms its exact already-authenticated, committed deferred call.
/// Wake schedules the blocked caller without changing its pending call/result.
pub(crate) fn query(text: &str) -> Option<u64> {
    let mode = match text.strip_prefix("[EX-ARM] mode=")?.parse::<u64>() {
        Ok(mode @ 11..=13) => mode,
        _ => return Some(u64::MAX),
    };
    let mut sched = SCHEDULER.lock();
    let issuer = sched.current_process();
    if !issuer.trusted_auth_broker
        || issuer.is_exiting()
        || issuer.deferred_reply_targets.len() != 1
    {
        return Some(u64::MAX);
    }
    let target = issuer.deferred_reply_targets[0].target;
    let Some(object) = super::exec_retirement_trace::selected_call(target) else {
        return Some(u64::MAX);
    };
    if !crate::ipc::reply_target_is_live(target, &sched)
        || MODE.load(Ordering::Acquire) != 0 && FINISHED.load(Ordering::Acquire) != 3
    {
        return Some(u64::MAX);
    }
    EXIT_SYSCALL_CPU.store(u64::MAX, Ordering::Relaxed);
    OBJECT.store(object, Ordering::Relaxed);
    ISSUER.store(issuer.pid as u64, Ordering::Relaxed);
    ISSUER_INC.store(issuer.foreground.incarnation, Ordering::Relaxed);
    CALLER.store(target.call.pid as u64, Ordering::Relaxed);
    CALLER_INC.store(target.call.incarnation, Ordering::Relaxed);
    ENDPOINT.store(u64::from(target.endpoint_id), Ordering::Relaxed);
    GENERATION.store(target.call.generation, Ordering::Relaxed);
    ENTERED.store(0, Ordering::Relaxed);
    FINISHED.store(0, Ordering::Relaxed);
    HELD.store(0, Ordering::Relaxed);
    CONTENDED.store(0, Ordering::Relaxed);
    COMPLETE_CPU.store(u64::MAX, Ordering::Relaxed);
    EXIT_CPU.store(u64::MAX, Ordering::Relaxed);
    WINNER.store(
        match mode {
            11 => COMPLETE,
            12 => EXIT,
            _ => 0,
        },
        Ordering::Relaxed,
    );
    MODE.store(mode, Ordering::Release);
    event("armed", 0, issuer.pid, issuer.foreground.incarnation, 0);
    sched.wake_pid(target.call.pid);
    Some(0)
}

fn matches_target(target: IpcReplyTarget) -> bool {
    target.call.pid as u64 == CALLER.load(Ordering::Relaxed)
        && target.call.incarnation == CALLER_INC.load(Ordering::Relaxed)
        && target.call.generation == GENERATION.load(Ordering::Relaxed)
        && u64::from(target.endpoint_id) == ENDPOINT.load(Ordering::Relaxed)
}

/// Arrive inside the actual syscall, then compete for its existing lock.
/// The loser proves a real failed try_lock while the winner holds that lock.
/// Both invocations remain active until after the first terminal transition.
pub(crate) fn acquire(actor: u64, token: u64) -> (spin::MutexGuard<'static, Scheduler>, bool) {
    let sched = SCHEDULER.lock();
    let p = sched.current_process();
    let selected = MODE.load(Ordering::Acquire) != 0
        && FINISHED.load(Ordering::Acquire) & actor == 0
        && if actor == COMPLETE {
            p.pid as u64 == ISSUER.load(Ordering::Relaxed)
                && p.foreground.incarnation == ISSUER_INC.load(Ordering::Relaxed)
                && p.trusted_auth_broker
                && p.deferred_reply_targets
                    .iter()
                    .any(|e| e.token == token && matches_target(e.target))
        } else {
            EXIT_SYSCALL_CPU.load(Ordering::Acquire) == crate::sched::current_cpu_id() as u64
                && p.pid as u64 == CALLER.load(Ordering::Relaxed)
                && p.foreground.incarnation == CALLER_INC.load(Ordering::Relaxed)
                && p.pending_call.is_some_and(|pending| {
                    matches_target(IpcReplyTarget {
                        endpoint_id: pending.endpoint_id,
                        call: crate::process::IpcCallId {
                            pid: p.pid,
                            incarnation: p.foreground.incarnation,
                            generation: pending.generation,
                        },
                    })
                })
        };
    if !selected {
        return (sched, false);
    }
    let pid = p.pid;
    let inc = p.foreground.incarnation;
    let cpu = crate::sched::current_cpu_id() as u64;
    event("entered", actor, pid, inc, 0);
    if actor == COMPLETE {
        COMPLETE_CPU.store(cpu, Ordering::Release);
    } else {
        EXIT_CPU.store(cpu, Ordering::Release);
    }
    drop(sched);
    assert_eq!(ENTERED.fetch_or(actor, Ordering::AcqRel) & actor, 0);
    wait(|| ENTERED.load(Ordering::Acquire) == 3);
    assert_ne!(
        COMPLETE_CPU.load(Ordering::Acquire),
        EXIT_CPU.load(Ordering::Acquire),
        "actors did not overlap on separate CPUs"
    );
    let _ = WINNER.compare_exchange(0, actor, Ordering::AcqRel, Ordering::Acquire);
    if WINNER.load(Ordering::Acquire) == actor {
        let sched = SCHEDULER.lock();
        event("lock-held", actor, pid, inc, 0);
        HELD.store(actor, Ordering::Release);
        wait(|| CONTENDED.load(Ordering::Acquire) != 0);
        (sched, true)
    } else {
        wait(|| HELD.load(Ordering::Acquire) != 0);
        assert!(
            SCHEDULER.try_lock().is_none(),
            "terminal lock contention not observed"
        );
        event("lock-contended", actor, pid, inc, 0);
        CONTENDED.store(actor, Ordering::Release);
        wait(|| FINISHED.load(Ordering::Acquire) & WINNER.load(Ordering::Acquire) != 0);
        (SCHEDULER.lock(), true)
    }
}

pub(crate) fn finished(actor: u64, selected: bool, sched: &Scheduler, result: u64) {
    if !selected {
        return;
    }
    let pid = if actor == COMPLETE {
        ISSUER.load(Ordering::Relaxed)
    } else {
        CALLER.load(Ordering::Relaxed)
    };
    let inc = if actor == COMPLETE {
        ISSUER_INC.load(Ordering::Relaxed)
    } else {
        CALLER_INC.load(Ordering::Relaxed)
    };
    event(
        if actor == COMPLETE {
            "returned"
        } else {
            "exit-committed"
        },
        actor,
        pid as usize,
        inc,
        result,
    );
    let caller = sched
        .processes
        .iter()
        .find(|p| p.pid as u64 == CALLER.load(Ordering::Relaxed))
        .expect("selected caller");
    let broker = super::CAP_BROKER.lock();
    let counts = broker.session_accounting();
    crate::serial_println!("[EX-OUTCOME] seq={} object={} actor={} caller={} inc={} endpoint={} generation={} pending={} reply={} outcome={:?} state={:?} grants={} receipts={} deliveries={} maps={} cpu={}",
        super::exec_retirement_trace::seq(), OBJECT.load(Ordering::Relaxed), actor, caller.pid,
        caller.foreground.incarnation, ENDPOINT.load(Ordering::Relaxed), GENERATION.load(Ordering::Relaxed),
        u8::from(caller.pending_call.is_some()), u8::from(caller.ipc_reply.is_some()), caller.ipc_call_outcome, caller.state, counts[7], counts[6],
        broker.account_grants.iter().filter(|(_, g)| g.delivery.is_some_and(|d| !d.delivered)).count(),
        caller.mapped_shared.len(), crate::sched::current_cpu_id());
    FINISHED.fetch_or(actor, Ordering::Release);
}

/// Called only at the real userspace ProcessExit syscall entry. Other kernel
/// finish/kill paths cannot participate, even when they name their reason exit.
pub(crate) fn exit_syscall(code: i32) {
    let sched = SCHEDULER.lock();
    let p = sched.current_process();
    if MODE.load(Ordering::Acquire) != 0
        && FINISHED.load(Ordering::Acquire) & EXIT == 0
        && p.pid as u64 == CALLER.load(Ordering::Relaxed)
        && p.foreground.incarnation == CALLER_INC.load(Ordering::Relaxed)
    {
        event(
            "exit-syscall",
            EXIT,
            p.pid,
            p.foreground.incarnation,
            code as u64,
        );
        EXIT_SYSCALL_CPU.store(crate::sched::current_cpu_id() as u64, Ordering::Release);
    }
}

/// Observation immediately after the production Finished state publication,
/// before ordinary endpoint/grant retirement. No lifecycle decision uses it.
pub(crate) fn marked(selected: bool, p: &crate::process::Process) {
    if selected {
        event("exit-marked", EXIT, p.pid, p.foreground.incarnation, 0);
    }
}
