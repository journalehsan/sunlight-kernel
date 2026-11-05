//! H4 observations only. Coordination remains in ordinary userspace IPC.
//! No token, role, process state or production authority is created here.
use core::sync::atomic::{AtomicU64, Ordering};

static FIXTURE: AtomicU64 = AtomicU64::new(0);
static CONTROLLER: AtomicU64 = AtomicU64::new(0);
static TARGET: AtomicU64 = AtomicU64::new(0);
static CASE: AtomicU64 = AtomicU64::new(0);
static GATE: super::session_h4_gate::Gate = super::session_h4_gate::Gate::new();

fn number(text: &str, name: &str) -> Option<u64> {
    text.split_whitespace().find_map(|field| {
        let (key, value) = field.split_once('=')?;
        (key == name).then(|| value.parse().ok()).flatten()
    })
}

pub(crate) fn observation(text: &str) {
    if text.starts_with("[SESSION-E3-CLIENT] ") && text.contains("checkpoint=fixture-complete ") {
        let sched = crate::sched::SCHEDULER.lock();
        let p = sched.current_process();
        if p.name_str() == "session-race-probe"
            && number(text, "fixture") == Some(FIXTURE.load(Ordering::Acquire))
        {
            super::session_h3_trace::record(
                80,
                p.pid as u64,
                p.foreground.incarnation,
                FIXTURE.load(Ordering::Relaxed),
                CASE.load(Ordering::Relaxed),
                0,
            );
        }
        return;
    }
    if !text.starts_with("[H4-CTL] ") && !text.starts_with("[H4-MANAGER] ") {
        return;
    }
    let sched = crate::sched::SCHEDULER.lock();
    let p = sched.current_process();
    if text.starts_with("[H4-CTL] ")
        && p.name_str() == "sunlightctl"
        && p.uid == 0
        && !p.is_exiting()
    {
        if text.contains("checkpoint=ready ") && FIXTURE.load(Ordering::Acquire) == 0 {
            let (Some(fixture), Some(target)) = (number(text, "fixture"), number(text, "target"))
            else {
                return;
            };
            if fixture == 0 || target == 0 {
                return;
            }
            TARGET.store(target, Ordering::Relaxed);
            CONTROLLER.store(p.pid as u64, Ordering::Relaxed);
            FIXTURE.store(fixture, Ordering::Release);
            super::session_h3_trace::enable();
            super::session_h3_trace::record(
                60,
                p.pid as u64,
                p.foreground.incarnation,
                fixture,
                target,
                0,
            );
        } else if CONTROLLER.load(Ordering::Relaxed) == p.pid as u64
            && text.contains("checkpoint=submitted ")
        {
            super::session_h3_trace::record(
                61,
                p.pid as u64,
                p.foreground.incarnation,
                FIXTURE.load(Ordering::Acquire),
                TARGET.load(Ordering::Relaxed),
                0,
            );
        }
    } else if text.starts_with("[H4-MANAGER] ")
        && sched.active_manager(p)
        && number(text, "sender") == Some(CONTROLLER.load(Ordering::Relaxed))
        && number(text, "target") == Some(TARGET.load(Ordering::Relaxed))
    {
        super::session_h3_trace::record(
            62,
            p.pid as u64,
            p.foreground.incarnation,
            FIXTURE.load(Ordering::Acquire),
            TARGET.load(Ordering::Relaxed),
            CONTROLLER.load(Ordering::Relaxed),
        );
    }
}

pub(crate) fn arm(case: u64) {
    CASE.store(case, Ordering::Release);
}

pub(crate) fn wait_for_forced_exit() -> bool {
    CASE.load(Ordering::Acquire) == 12
}

/// A candidate manager signal entrant is not a reaper. Authorization is
/// observed separately after the ordinary production checks under SCHEDULER.
pub(crate) fn signal_attempt(pid: usize, signal: u64, generation: Option<u64>) {
    if FIXTURE.load(Ordering::Acquire) != 0 && TARGET.load(Ordering::Relaxed) == pid as u64 {
        super::session_h3_trace::record(63, 0, 0, pid as u64, signal, generation.unwrap_or(0));
    }
}

pub(crate) fn signal_authorized(
    p: &crate::process::Process,
    target: &crate::process::Process,
    signal: u64,
) {
    if TARGET.load(Ordering::Relaxed) == target.pid as u64 {
        super::session_h3_trace::record(
            64,
            p.pid as u64,
            p.foreground.incarnation,
            target.pid as u64,
            signal,
            target.owning_core as u64,
        );
    }
}

/// Snapshot only; release the guard before recording the exit pre-acquisition
/// checkpoint. There is no barrier on this path, including on timeout/failure.
pub(crate) fn exit_attempt() -> Option<(usize, u64)> {
    let identity = {
        let sched = crate::sched::SCHEDULER.lock();
        let p = sched.current_process();
        (p.pid as u64 == TARGET.load(Ordering::Relaxed))
            .then_some((p.pid, p.foreground.incarnation))
    };
    if let Some((pid, incarnation)) = identity {
        super::session_h3_trace::record(70, pid as u64, incarnation, 1, 0, 0);
        if session_owner_case() {
            GATE.publish(super::session_h4_gate::EXIT_READY);
            wait(false, pid as u64, incarnation);
        }
    }
    identity
}

pub(crate) fn exit_releasing(p: &crate::process::Process) {
    if p.pid as u64 == TARGET.load(Ordering::Relaxed) {
        super::session_h3_trace::record(74, p.pid as u64, p.foreground.incarnation, 0, 0, 0);
    }
}

pub(crate) fn exit_acquired(p: &crate::process::Process) {
    if p.pid as u64 == TARGET.load(Ordering::Relaxed) {
        super::session_h3_trace::record(
            71,
            p.pid as u64,
            p.foreground.incarnation,
            1,
            u64::from(p.is_exiting()),
            p.owning_core as u64,
        );
    }
}

pub(crate) fn forced_exit(p: &crate::process::Process) {
    if p.pid as u64 == TARGET.load(Ordering::Relaxed) {
        super::session_h3_trace::record(
            67,
            p.pid as u64,
            p.foreground.incarnation,
            p.owning_core as u64,
            u64::from(p.is_exiting()),
            p.address_space.identity().generation,
        );
    }
}

fn session_owner_case() -> bool {
    matches!(CASE.load(Ordering::Acquire), 13..=15)
}

fn wait(grant: bool, pid: u64, incarnation: u64) {
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    let budget = crate::arch::x86_64::interrupts::tsc_hz().saturating_mul(5);
    for i in 0..100_000_000u64 {
        if GATE.released(CASE.load(Ordering::Acquire), grant) {
            return;
        }
        if i % 4096 == 0 && unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) >= budget {
            break;
        }
        core::hint::spin_loop();
    }
    GATE.abort();
    super::session_h3_trace::record(90, pid, incarnation, u64::from(grant), 0, 0);
}

pub(crate) fn grant_boundary(p: &crate::process::Process) -> bool {
    if !session_owner_case() {
        return false;
    }
    // This function is called with a process borrow only to publish identity.
    // Waiting happens separately, after its scheduler guard has been dropped.
    GATE.publish(super::session_h4_gate::GRANT_READY);
    super::session_h3_trace::record(
        68,
        p.pid as u64,
        p.foreground.incarnation,
        CASE.load(Ordering::Acquire),
        0,
        0,
    );
    true
}

pub(crate) fn grant_wait(pid: u64, incarnation: u64) {
    wait(true, pid, incarnation);
}

pub(crate) fn grant_done() {
    if session_owner_case() {
        GATE.publish(super::session_h4_gate::GRANT_DONE);
    }
}

pub(crate) fn retired(pid: usize) {
    if session_owner_case() && pid as u64 == TARGET.load(Ordering::Relaxed) {
        GATE.publish(super::session_h4_gate::RETIRED);
    }
}
