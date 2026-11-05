//! Bounded observation of one authenticated operation. Never consulted by any
//! authority, allocator, mapping or shootdown decision. No ISR logging/locks.
#[cfg(not(debug_assertions))]
compile_error!("authenticated_exec_test is restricted to debug test images");
use super::{CapabilityBroker, CapabilityToken};
use crate::process::mm2b_state::AddressSpaceIdentity;
use core::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(1);
static TOKEN: AtomicU64 = AtomicU64::new(0);
static GRANT: AtomicU64 = AtomicU64::new(0);
static FRAME: AtomicU64 = AtomicU64::new(0);
static OBJECT: AtomicU64 = AtomicU64::new(0);
static RELEASED: AtomicU64 = AtomicU64::new(0);
static REUSED: AtomicU64 = AtomicU64::new(0);
static CALLER: AtomicU64 = AtomicU64::new(0);
static INC: AtomicU64 = AtomicU64::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static ENDPOINT: AtomicU64 = AtomicU64::new(0);
static UNMAP_VM: AtomicU64 = AtomicU64::new(0);
static UNMAP_VA: AtomicU64 = AtomicU64::new(0);
static ACK_SEQUENCE: [AtomicU64; 64] = [const { AtomicU64::new(0) }; 64];
static ACK_FLUSHED: [AtomicU64; 64] = [const { AtomicU64::new(0) }; 64];

pub(crate) fn ack_produced(request: crate::process::mm2b_state::MailboxRequest, flushed: bool) {
    if request.target.generation != UNMAP_VM.load(Ordering::Relaxed)
        || request.start != UNMAP_VA.load(Ordering::Relaxed)
    {
        return;
    }
    let cpu = crate::sched::current_cpu_id();
    ACK_FLUSHED[cpu].store(u64::from(flushed), Ordering::Relaxed);
    ACK_SEQUENCE[cpu].store(request.sequence, Ordering::Release);
}

pub(crate) fn seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}
pub(crate) fn selected(token: CapabilityToken) -> bool {
    TOKEN.load(Ordering::Relaxed) != 0 && TOKEN.load(Ordering::Relaxed) == token.0
}

#[cfg(any(
    feature = "authenticated_completion_race_test",
    feature = "authenticated_completion_exit_test"
))]
pub(crate) fn selected_call(target: crate::process::IpcReplyTarget) -> Option<u64> {
    (target.call.pid as u64 == CALLER.load(Ordering::Relaxed)
        && target.call.incarnation == INC.load(Ordering::Relaxed)
        && target.call.generation == GENERATION.load(Ordering::Relaxed)
        && u64::from(target.endpoint_id) == ENDPOINT.load(Ordering::Relaxed)
        && GRANT.load(Ordering::Relaxed) != 0)
        .then(|| OBJECT.load(Ordering::Relaxed))
}

pub(crate) fn observe(text: &str) {
    let Some(event) = text.strip_prefix("[AE-OBSERVE] event=") else {
        return;
    };
    if !matches!(
        event,
        "committed"
            | "failed-exec-preserved"
            | "exec-enter"
            | "replacement"
            | "replacement-complete"
            | "caller-consumed"
            | "worker-ready"
            | "worker-held"
            | "fresh-request"
            | "controller-complete"
    ) {
        return;
    }
    let sched = crate::sched::SCHEDULER.lock();
    let p = sched.current_process();
    let broker = super::CAP_BROKER.lock();
    crate::serial_println!("[AE-SNAPSHOT] seq={} event={} pid={} inc={} vm={} auth={} uid={} gid={} pending={} outcome={} reply={} deferred={} maps={} cpu={}", seq(), event, p.pid, p.foreground.incarnation, p.address_space.identity().generation, u8::from(p.trusted_auth_broker), p.uid, p.gid, u8::from(p.pending_call.is_some()), u8::from(p.ipc_call_outcome.is_some()), u8::from(p.ipc_reply.is_some()), p.deferred_reply_targets.len(), p.mapped_shared.len(), crate::sched::current_cpu_id());
    if event == "fresh-request" {
        if let Some(target) = p.ipc_reply_target {
            crate::serial_println!("[AE-FRESH] seq={} pid={} inc={} vm={} caller={} caller_inc={} endpoint={} generation={} live={} cpu={}", seq(), p.pid, p.foreground.incarnation, p.address_space.identity().generation, target.call.pid, target.call.incarnation, target.endpoint_id, target.call.generation, u8::from(crate::ipc::reply_target_is_live(target, &sched)), crate::sched::current_cpu_id());
        }
    }
    if OBJECT.load(Ordering::Relaxed) >= 3 {
        return;
    }
    if event != "committed"
        || !p.trusted_auth_broker
        || (TOKEN.load(Ordering::Relaxed) != 0 && RELEASED.load(Ordering::Relaxed) == 0)
    {
        return;
    }
    let Some(target) = p.ipc_reply_target else {
        return;
    };
    let Some((grant_token, grant)) = broker.account_grants.iter().find(|(_, g)| {
        g.delivery.is_some_and(|d| {
            d.issuer_pid == p.pid
                && d.issuer_incarnation == p.foreground.incarnation
                && d.issuer_vm_generation == p.address_space.identity().generation
                && d.recipient.incarnation == target.call.incarnation
                && d.request == target.call.generation
                && d.recipient.pid == target.call.pid
                && !d.delivered
        })
    }) else {
        return;
    };
    let Some(&(token, _, _)) = p.mapped_shared.last() else {
        return;
    };
    let Some(object) = broker.shared_region_for_cleanup(token) else {
        return;
    };
    if object.frames.len() != 1 {
        return;
    }
    OBJECT.fetch_add(1, Ordering::Relaxed);
    RELEASED.store(0, Ordering::Relaxed);
    REUSED.store(0, Ordering::Relaxed);
    TOKEN.store(token.0, Ordering::Relaxed);
    GRANT.store(grant_token.0, Ordering::Relaxed);
    FRAME.store(object.frames[0].start_address().as_u64(), Ordering::Relaxed);
    CALLER.store(target.call.pid as u64, Ordering::Relaxed);
    INC.store(target.call.incarnation, Ordering::Relaxed);
    GENERATION.store(target.call.generation, Ordering::Relaxed);
    ENDPOINT.store(u64::from(target.endpoint_id), Ordering::Relaxed);
    crate::serial_println!("[AE-GRANT] seq={} event=selected grant={} object={} frame={} session={} epoch={} issuer={} issuer_inc={} issuer_vm={} caller={} caller_inc={} caller_vm={} endpoint={} generation={} views={} cpu={}", seq(), OBJECT.load(Ordering::Relaxed), OBJECT.load(Ordering::Relaxed), FRAME.load(Ordering::Relaxed), grant.grant.binding.scope.session, grant.grant.binding.scope.session_generation, p.pid, p.foreground.incarnation, p.address_space.identity().generation, target.call.pid, target.call.incarnation, grant.grant.binding.process_generation, target.endpoint_id, target.call.generation, broker.shared_region_map_count(token).unwrap_or(0), crate::sched::current_cpu_id());
    for owner in &sched.processes {
        for &(t, va, size) in &owner.mapped_shared {
            if t == token {
                crate::serial_println!("[AE-MAP] seq={} event=existing object={} pid={} inc={} vm={} va={} bytes={} frame={} cpu={}", seq(), OBJECT.load(Ordering::Relaxed), owner.pid, owner.foreground.incarnation, owner.address_space.identity().generation, va.as_u64(), size, FRAME.load(Ordering::Relaxed), crate::sched::current_cpu_id());
            }
        }
    }
}

pub(crate) fn grant_retired(broker: &CapabilityBroker) {
    let token = GRANT.load(Ordering::Relaxed);
    if token != 0
        && !broker.account_grants.iter().any(|(t, _)| t.0 == token)
        && GRANT
            .compare_exchange(token, 0, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        let counts = broker.session_accounting();
        let (sessions, receipts, grants) = (counts[0], counts[6], counts[7]);
        crate::serial_println!("[AE-GRANT] seq={} event=retired grant={} object={} sessions={} receipts={} grants={} pending_deliveries={} cpu={}", seq(), OBJECT.load(Ordering::Relaxed), OBJECT.load(Ordering::Relaxed), sessions, receipts, grants, broker.account_grants.iter().filter(|(_,g)| g.delivery.is_some_and(|d| !d.delivered)).count(), crate::sched::current_cpu_id());
    }
}

pub(crate) fn terminal(
    target: crate::process::IpcReplyTarget,
    outcome: crate::process::IpcCallOutcome,
) {
    if target.call.pid as u64 != CALLER.load(Ordering::Relaxed)
        || target.call.incarnation != INC.load(Ordering::Relaxed)
        || target.call.generation != GENERATION.load(Ordering::Relaxed)
        || u64::from(target.endpoint_id) != ENDPOINT.load(Ordering::Relaxed)
    {
        return;
    }
    crate::serial_println!("[AE-TERMINAL] seq={} grant={} caller={} inc={} endpoint={} generation={} outcome={:?} cpu={}", seq(), OBJECT.load(Ordering::Relaxed), target.call.pid, target.call.incarnation, target.endpoint_id, target.call.generation, outcome, crate::sched::current_cpu_id());
}

pub(crate) fn unmap(
    process: &crate::process::Process,
    token: CapabilityToken,
    va: u64,
    event: &str,
    views: usize,
) {
    if !selected(token) {
        return;
    }
    if event == "request" {
        UNMAP_VM.store(
            process.address_space.identity().generation,
            Ordering::Relaxed,
        );
        UNMAP_VA.store(va, Ordering::Relaxed);
    }
    crate::serial_println!(
        "[AE-UNMAP] seq={} event={} object={} frame={} pid={} inc={} vm={} va={} views={} cpu={}",
        seq(),
        event,
        OBJECT.load(Ordering::Relaxed),
        FRAME.load(Ordering::Relaxed),
        process.pid,
        process.foreground.incarnation,
        process.address_space.identity().generation,
        va,
        views,
        crate::sched::current_cpu_id()
    );
    if event == "complete" {
        UNMAP_VM.store(0, Ordering::Relaxed);
    }
}

pub(crate) fn tlb(
    identity: AddressSpaceIdentity,
    va: u64,
    event: &str,
    request: u64,
    targets: u64,
    ack_cpu: usize,
) {
    if identity.generation != UNMAP_VM.load(Ordering::Relaxed)
        || va != UNMAP_VA.load(Ordering::Relaxed)
    {
        return;
    }
    let produced = if event == "ack-observed" {
        ACK_SEQUENCE[ack_cpu].load(Ordering::Acquire)
    } else {
        0
    };
    let flushed = if event == "ack-observed" {
        ACK_FLUSHED[ack_cpu].load(Ordering::Relaxed)
    } else {
        0
    };
    crate::serial_println!("[AE-TLB] seq={} event={} object={} frame={} vm={} va={} request={} targets={} ack_cpu={} produced={} flushed={} cpu={}", seq(), event, OBJECT.load(Ordering::Relaxed), FRAME.load(Ordering::Relaxed), identity.generation, va, request, targets, ack_cpu, produced, flushed, crate::sched::current_cpu_id());
}

pub(crate) fn physical(address: u64, event: &str, class: u8) {
    if address == 0 || address != FRAME.load(Ordering::Relaxed) {
        return;
    }
    if event == "claim" && REUSED.swap(1, Ordering::Relaxed) != 0 {
        return;
    }
    if event == "free" && REUSED.load(Ordering::Relaxed) != 0 {
        return;
    }
    if event == "free" {
        RELEASED.store(1, Ordering::Relaxed);
    }
    crate::serial_println!(
        "[AE-PMM] seq={} event={} object={} frame={} class={} cpu={}",
        seq(),
        event,
        OBJECT.load(Ordering::Relaxed),
        address,
        class,
        crate::sched::current_cpu_id()
    );
}

#[cfg(feature = "authenticated_cancel_test")]
pub(crate) fn cancel_query(text: &str) -> Option<u64> {
    if text != "[CE-QUERY] caller-reaped" {
        return None;
    }
    // Observation only: never drive reaping, grant settlement or VM retirement.
    // waitpid reports Finished too, which does not establish resource cleanup.
    let sched = crate::sched::SCHEDULER.lock();
    Some(u64::from(sched.processes.iter().any(|p| {
        p.pid as u64 == CALLER.load(Ordering::Relaxed)
            && p.foreground.incarnation == INC.load(Ordering::Relaxed)
            && p.ppid == sched.current_process().pid
            && p.state == crate::process::ProcessState::Reaped
            && !p.exit_cleanup_pending
    })))
}

#[cfg(feature = "authenticated_cancel_test")]
pub(crate) fn cancel_observe(text: &str) {
    let Some(event) = text.strip_prefix("[CE-OBSERVE] event=") else {
        return;
    };
    if !matches!(
        event,
        "committed"
            | "deferred"
            | "published"
            | "late-denied"
            | "fresh"
            | "account-unmapped"
            | "caller-dead"
            | "cancel-observed"
            | "supervisor-clean"
            | "first-return"
            | "exit-with-reply"
            | "cancel-after-commit-before-collect"
            | "cancel-preserved-reply"
            | "caller-consumed"
            | "caller-unmapped"
            | "endpoint-closed"
            | "race-late-cancel-before"
            | "race-late-cancel-after"
    ) {
        return;
    }
    let sched = crate::sched::SCHEDULER.lock();
    let p = sched.current_process();
    let broker = super::CAP_BROKER.lock();
    let counts = broker.session_accounting();
    crate::serial_println!("[CE-STATE] seq={} object={} event={} pid={} inc={} vm={} auth={} pending={} outcome={:?} reply={} deferred={} maps={} sessions={} receipts={} grants={} deliveries={} cpu={}", seq(), OBJECT.load(Ordering::Relaxed), event, p.pid, p.foreground.incarnation, p.address_space.identity().generation, u8::from(p.trusted_auth_broker), u8::from(p.pending_call.is_some()), p.ipc_call_outcome, u8::from(p.ipc_reply.is_some()), p.deferred_reply_targets.len(), p.mapped_shared.len(), counts[0], counts[6], counts[7], broker.account_grants.iter().filter(|(_,g)| g.delivery.is_some_and(|d| !d.delivered)).count(), crate::sched::current_cpu_id());
    if let Some(target) = p.ipc_reply_target {
        crate::serial_println!("[CE-TARGET] seq={} object={} event={} pid={} caller={} inc={} endpoint={} generation={} live={}", seq(), OBJECT.load(Ordering::Relaxed), event, p.pid, target.call.pid, target.call.incarnation, target.endpoint_id, target.call.generation, u8::from(crate::ipc::reply_target_is_live(target, &sched)));
    }
}

#[cfg(feature = "authenticated_cancel_test")]
pub(crate) fn caller_reap(p: &crate::process::Process, event: &str, retired_vm: u64) {
    if p.pid as u64 != CALLER.load(Ordering::Relaxed)
        || p.foreground.incarnation != INC.load(Ordering::Relaxed)
    {
        return;
    }
    crate::serial_println!("[CE-REAP] seq={} object={} event={} pid={} inc={} vm={} maps={} pending={} outcome={} reply={} deferred={} active={} owning={} queued={} endpoints={} cpu={}", seq(), OBJECT.load(Ordering::Relaxed), event, p.pid, p.foreground.incarnation, retired_vm, p.mapped_shared.len(), u8::from(p.pending_call.is_some()), u8::from(p.ipc_call_outcome.is_some()), u8::from(p.ipc_reply.is_some()), p.deferred_reply_targets.len(), crate::memory::tlb::active_cpu_mask(p.address_space.identity()), p.owning_core, p.queued_on_core, super::CAP_BROKER.lock().endpoints_owned_by(p.pid).len(), crate::sched::current_cpu_id());
}
