//! Test-only observations of actual ring-3 calls. No coordination or authority.
use core::sync::atomic::{AtomicU64, Ordering};

static ENDPOINTS: [AtomicU64; 15] = [const { AtomicU64::new(0) }; 15];

pub(crate) fn observation(text: &str) {
    if !text.starts_with("[AUTH-OBSERVE] ") {
        return;
    }
    let number = |key: &str| {
        text.split_whitespace().find_map(|field| {
            let (name, value) = field.split_once('=')?;
            (name == key).then(|| value.parse::<usize>().ok()).flatten()
        })
    };
    let Some(round @ 1..=15) = number("round") else {
        return;
    };
    let event = text
        .split_whitespace()
        .find_map(|s| s.strip_prefix("event="));
    let Some(event @ ("received" | "consumed" | "exec-enter" | "exec-return" | "shutdown-enter")) =
        event
    else {
        return;
    };
    let sched = crate::sched::SCHEDULER.lock();
    let process = sched.current_process();
    crate::serial_println!("[AUTH-KERNEL] round={} event={} pid={} inc={} vm={} uid={} gid={} auth={} pending={} outcome={} reply={} deferred={} cpu={}", round, event, process.pid, process.foreground.incarnation, process.address_space.identity().generation, process.uid, process.gid, u8::from(process.trusted_auth_broker), u8::from(process.pending_call.is_some()), u8::from(process.ipc_call_outcome.is_some()), u8::from(process.ipc_reply.is_some()), process.deferred_reply_targets.len(), crate::sched::current_cpu_id());
    if event != "received" {
        return;
    }
    let Some(target) = process.ipc_reply_target else {
        return;
    };
    let Some(caller) = sched.processes.iter().find(|p| p.pid == target.call.pid) else {
        return;
    };
    ENDPOINTS[round - 1].store(u64::from(target.endpoint_id) + 1, Ordering::Release);
    crate::serial_println!("[AUTH-BINDING] round={} server={} server_inc={} server_vm={} caller={} caller_inc={} caller_vm={} caller_auth={} endpoint={} generation={} live={}", round, process.pid, process.foreground.incarnation, process.address_space.identity().generation, caller.pid, target.call.incarnation, caller.address_space.identity().generation, u8::from(caller.trusted_auth_broker), target.endpoint_id, target.call.generation, u8::from(super::reply_target_is_live(target, &sched)));
}

pub(crate) fn terminal(
    process: &crate::process::Process,
    target: crate::process::IpcReplyTarget,
    outcome: crate::process::IpcCallOutcome,
) {
    use crate::process::IpcCallOutcome;
    let Some(round) = ENDPOINTS
        .iter()
        .position(|id| id.load(Ordering::Acquire) == u64::from(target.endpoint_id) + 1)
    else {
        return;
    };
    let outcome = match outcome {
        IpcCallOutcome::ReplyDelivered(_) => "ReplyDelivered",
        IpcCallOutcome::PeerClosed(_) => "PeerClosed",
        IpcCallOutcome::DeadlineExpired(_) => "DeadlineExpired",
        IpcCallOutcome::ExplicitlyCancelled(_) => "Cancelled",
    };
    crate::serial_println!("[AUTH-TERMINAL] round={} caller={} caller_inc={} caller_vm={} endpoint={} generation={} outcome={} cpu={}", round + 1, process.pid, process.foreground.incarnation, process.address_space.identity().generation, target.endpoint_id, target.call.generation, outcome, crate::sched::current_cpu_id());
}
