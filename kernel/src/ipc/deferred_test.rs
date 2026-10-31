//! Opt-in boot-method tests with real Process/VM records, not ring-3 races.
use super::*;
use crate::process::{foreground::Foreground, DeferredIpcReply};

fn start(
    sched: &mut Scheduler,
    caller: usize,
    server: usize,
    cap: CapabilityToken,
    endpoint: u32,
    caps: &mut CapabilityBroker,
    bus: &mut IpcBus,
) {
    let caller_pid = sched.processes[caller].pid;
    let server_pid = sched.processes[server].pid;
    assert!(sched.processes[caller].pending_call.is_none());
    assert!(sched.processes[caller].ipc_call_outcome.is_none());
    assert!(matches!(
        handle_ipc_call(caller_pid, cap, IpcMsg::with_label(77), caps, sched, bus),
        Err(IpcError::WouldBlock)
    ));
    assert_eq!(
        handle_ipc_recv(server_pid, endpoint, sched, bus)
            .unwrap()
            .label,
        77
    );
}

fn terminal(sched: &mut Scheduler, caller: usize, expected: Result<u64, IpcError>) {
    let result = take_terminal_result(sched, caller).map(|r| r.map(|m| m.label));
    assert_eq!(result, Some(expected));
    assert!(take_terminal_result(sched, caller).is_none());
    assert!(sched.processes[caller].pending_call.is_none());
    sched.remove_from_ready_queues(caller);
}

pub(crate) fn run(sched: &mut Scheduler, caller: usize, server: usize) {
    let caller_pid = sched.processes[caller].pid;
    let server_pid = sched.processes[server].pid;
    let original_incarnation = sched.processes[caller].foreground.incarnation;
    sched.processes[caller].foreground.incarnation = Foreground::allocate_incarnation().unwrap();
    let incarnation = sched.processes[caller].foreground.incarnation;
    let vm = sched.processes[caller].address_space.identity().generation;
    let mut caps = CapabilityBroker::new();
    let (endpoint, cap) = caps.create_endpoint(server_pid).unwrap();
    let mut bus = IpcBus::new();
    let depth = diagnostic_snapshot().current_queue_depth;
    assert!(sched.processes[server].deferred_reply_targets.is_empty());
    assert!(matches!(
        handle_ipc_call(
            caller_pid,
            CapabilityToken(0),
            IpcMsg::empty(),
            &mut caps,
            sched,
            &mut bus
        ),
        Err(IpcError::InvalidCapability)
    ));
    assert!(sched.processes[caller].pending_call.is_none());
    crate::serial_println!("[IPC-DEFER] case=invalid-request-cap result=PASS");

    start(sched, caller, server, cap, endpoint, &mut caps, &mut bus);
    let target = sched.processes[server].ipc_reply_target;
    let server_state = sched.processes[server].state;
    sched.processes[server].state = ProcessState::Finished;
    assert!(defer_current_reply(server_pid, sched).is_err());
    assert_eq!(sched.processes[server].ipc_reply_target, target);
    sched.processes[server].state = server_state;
    let stale = defer_current_reply(server_pid, sched).unwrap();
    sched.processes[server].state = ProcessState::Finished;
    assert!(!deferred_reply_is_live(server_pid, stale, sched));
    assert!(complete_deferred_reply(
        server_pid,
        stale,
        IpcMsg::empty(),
        sched,
        &mut caps,
        &mut bus
    )
    .is_err());
    sched.processes[server].state = server_state;
    assert!(deferred_reply_is_live(server_pid, stale, sched));
    assert!(!deferred_reply_is_live(caller_pid, stale, sched));
    assert!(deferred_reply_endpoint(caller_pid, stale, sched).is_err());
    assert!(complete_deferred_reply(
        caller_pid,
        stale,
        IpcMsg::empty(),
        sched,
        &mut caps,
        &mut bus
    )
    .is_err());
    assert!(deferred_reply_is_live(server_pid, stale, sched));
    handle_ipc_cancel(caller_pid, sched, &mut caps, &mut bus).unwrap();
    terminal(sched, caller, Err(IpcError::Cancelled));
    assert!(!deferred_reply_is_live(server_pid, stale, sched));
    crate::serial_println!("[IPC-DEFER] case=owner-isolation-cancel result=PASS");

    // A stale token cannot publish into the next actual pending call.
    start(sched, caller, server, cap, endpoint, &mut caps, &mut bus);
    let fresh = defer_current_reply(server_pid, sched).unwrap();
    assert_ne!(fresh, stale);
    assert!(complete_deferred_reply(
        server_pid,
        stale,
        IpcMsg::empty(),
        sched,
        &mut caps,
        &mut bus
    )
    .is_err());
    assert!(deferred_reply_is_live(server_pid, fresh, sched));
    complete_deferred_reply(
        server_pid,
        fresh,
        IpcMsg::with_label(88),
        sched,
        &mut caps,
        &mut bus,
    )
    .unwrap();
    handle_ipc_cancel(caller_pid, sched, &mut caps, &mut bus).unwrap();
    let calls = bus.remove_endpoint(endpoint);
    finish_peer_closed_calls(endpoint, calls, sched);
    terminal(sched, caller, Ok(88));
    assert!(complete_deferred_reply(
        server_pid,
        fresh,
        IpcMsg::empty(),
        sched,
        &mut caps,
        &mut bus
    )
    .is_err());
    crate::serial_println!("[IPC-DEFER] case=stale-token-reply-first result=PASS");

    start(sched, caller, server, cap, endpoint, &mut caps, &mut bus);
    let token = defer_current_reply(server_pid, sched).unwrap();
    let calls = bus.remove_endpoint(endpoint);
    assert_eq!(calls.len(), 1);
    finish_peer_closed_calls(endpoint, calls, sched);
    assert!(complete_deferred_reply(
        server_pid,
        token,
        IpcMsg::empty(),
        sched,
        &mut caps,
        &mut bus
    )
    .is_err());
    terminal(sched, caller, Err(IpcError::PeerClosed));
    crate::serial_println!("[IPC-DEFER] case=close-first result=PASS");

    // Controlled identity replacement proves rejection, not allocator PID reuse.
    start(sched, caller, server, cap, endpoint, &mut caps, &mut bus);
    let token = defer_current_reply(server_pid, sched).unwrap();
    sched.processes[caller].foreground.incarnation = Foreground::allocate_incarnation().unwrap();
    assert!(!deferred_reply_is_live(server_pid, token, sched));
    assert!(complete_deferred_reply(
        server_pid,
        token,
        IpcMsg::empty(),
        sched,
        &mut caps,
        &mut bus,
    )
    .is_err());
    assert!(sched.processes[caller].ipc_reply.is_none());
    assert!(sched.processes[caller].ipc_call_outcome.is_none());
    sched.processes[caller].foreground.incarnation = incarnation;
    handle_ipc_cancel(caller_pid, sched, &mut caps, &mut bus).unwrap();
    terminal(sched, caller, Err(IpcError::Cancelled));
    crate::serial_println!(
        "[IPC-DEFER] case=stale-incarnation result=PASS numeric_pid_reuse=NOT_RUN"
    );

    start(sched, caller, server, cap, endpoint, &mut caps, &mut bus);
    let token = defer_current_reply(server_pid, sched).unwrap();
    sched.processes[caller].state = ProcessState::Finished;
    assert!(!deferred_reply_is_live(server_pid, token, sched));
    assert!(complete_deferred_reply(
        server_pid,
        token,
        IpcMsg::empty(),
        sched,
        &mut caps,
        &mut bus,
    )
    .is_err());
    assert_eq!(sched.processes[caller].state, ProcessState::Finished);
    assert!(sched.processes[caller].ipc_call_outcome.is_none());
    sched.processes[caller].state = ProcessState::BlockedOnIpc;
    handle_ipc_cancel(caller_pid, sched, &mut caps, &mut bus).unwrap();
    terminal(sched, caller, Err(IpcError::Cancelled));
    crate::serial_println!("[IPC-DEFER] case=exiting-caller result=PASS real_exit_race=NOT_RUN");

    start(sched, caller, server, cap, endpoint, &mut caps, &mut bus);
    let target = sched.processes[server].ipc_reply_target.unwrap();
    let previous = sched.processes[server].next_deferred_reply_token;
    // Inject only table saturation. No synthetic entry is counted as a live call.
    for offset in 0..DEFERRED_REPLY_CAPACITY {
        sched.processes[server]
            .deferred_reply_targets
            .push_back(DeferredIpcReply {
                token: previous + 1 + offset as u64,
                target,
            });
    }
    let entries = sched.processes[server].deferred_reply_targets.clone();
    assert_eq!(
        defer_current_reply(server_pid, sched),
        Err(IpcError::QueueFull)
    );
    assert_eq!(sched.processes[server].ipc_reply_target, Some(target));
    assert_eq!(sched.processes[server].next_deferred_reply_token, previous);
    assert_eq!(sched.processes[server].deferred_reply_targets, entries);
    sched.processes[server].deferred_reply_targets.clear();
    handle_ipc_reply(
        server_pid,
        IpcMsg::with_label(89),
        sched,
        &mut caps,
        &mut bus,
    )
    .unwrap();
    terminal(sched, caller, Ok(89));
    crate::serial_println!("[IPC-DEFER] case=capacity-rollback result=PASS saturation=injected");

    // Boundary injection avoids pretending to execute 2^63 deferrals.
    sched.processes[server].next_deferred_reply_token = u64::MAX - 1;
    start(sched, caller, server, cap, endpoint, &mut caps, &mut bus);
    let last = defer_current_reply(server_pid, sched).unwrap();
    assert_eq!(last, u64::MAX);
    complete_deferred_reply(
        server_pid,
        last,
        IpcMsg::with_label(90),
        sched,
        &mut caps,
        &mut bus,
    )
    .unwrap();
    terminal(sched, caller, Ok(90));
    start(sched, caller, server, cap, endpoint, &mut caps, &mut bus);
    let target = sched.processes[server].ipc_reply_target;
    for _ in 0..64 {
        assert_eq!(
            defer_current_reply(server_pid, sched),
            Err(IpcError::QueueFull)
        );
        assert_eq!(sched.processes[server].ipc_reply_target, target);
        assert_eq!(sched.processes[server].next_deferred_reply_token, u64::MAX);
        assert!(sched.processes[server].deferred_reply_targets.is_empty());
        assert!(sched.processes[caller].ipc_call_outcome.is_none());
    }
    assert!(complete_deferred_reply(
        server_pid,
        stale,
        IpcMsg::empty(),
        sched,
        &mut caps,
        &mut bus
    )
    .is_err());
    handle_ipc_reply(
        server_pid,
        IpcMsg::with_label(91),
        sched,
        &mut caps,
        &mut bus,
    )
    .unwrap();
    terminal(sched, caller, Ok(91));
    crate::serial_println!("[IPC-DEFER] case=exhaustion-rollback result=PASS boundary=injected retries=64 ordinary_reply=PASS");

    let generation = sched.processes[caller].ipc_call_generation;
    sched.processes[caller].ipc_call_generation = u64::MAX;
    assert!(matches!(
        handle_ipc_call(caller_pid, cap, IpcMsg::empty(), &mut caps, sched, &mut bus),
        Err(IpcError::InvalidArgument)
    ));
    assert!(sched.processes[caller].pending_call.is_none());
    assert_eq!(bus.pending_count(endpoint), 0);
    sched.processes[caller].ipc_call_generation = generation;
    assert!(bus.remove_endpoint(endpoint).is_empty());
    assert_eq!(diagnostic_snapshot().current_queue_depth, depth);
    assert!(sched.processes[server].ipc_reply_target.is_none());
    assert!(sched.processes[server].deferred_reply_targets.is_empty());
    assert!(sched.processes[caller].ipc_reply.is_none());
    sched.processes[caller].foreground.incarnation = original_incarnation;
    crate::serial_println!(
        "[IPC-DEFER] case=call-generation-exhaustion result=PASS boundary=injected"
    );
    crate::serial_println!("[IPC-DEFER] PASS cases=9 caller_inc={} caller_vm={} pending=0 deferred=0 queued=0 scope=boot-method concurrent_race=NOT_RUN", incarnation, vm);
}
