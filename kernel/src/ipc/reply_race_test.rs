//! Native SMP production-method races. Kernel test drivers own the fixture
//! capabilities; these are not independent authenticated ring-3 Account peers.
use super::*;
use crate::process::{foreground::Foreground, Process};
use crate::sched::SCHEDULER;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use x86_64::VirtAddr;

#[path = "reply_race_gate.rs"]
mod gate;
static GATE: gate::Gate = gate::Gate::new();
static ROUND: AtomicU64 = AtomicU64::new(0);
static CLAIM: AtomicU64 = AtomicU64::new(0);
static SEQ: AtomicU64 = AtomicU64::new(1);
static OWNER: AtomicU64 = AtomicU64::new(0);
static ENDPOINT: AtomicU64 = AtomicU64::new(0);
static SERVER: AtomicUsize = AtomicUsize::new(0);
static CALLER: AtomicUsize = AtomicUsize::new(0);

fn event(case: u64, kind: &str) {
    crate::serial_println!(
        "[IPC-RACE] seq={} case={} event={} cpu={}",
        SEQ.fetch_add(1, Ordering::Relaxed),
        case,
        kind,
        crate::sched::current_cpu_id()
    );
}

fn wait(mut ready: impl FnMut() -> bool) {
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    let budget = crate::arch::x86_64::interrupts::tsc_hz().saturating_mul(5);
    for i in 0..100_000_000u64 {
        if ready() {
            return;
        }
        if i % 4096 == 0 && unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) >= budget {
            break;
        }
        core::hint::spin_loop();
    }
    panic!("IPC-RACE bounded coordination timeout");
}

fn winner(case: u64) -> u64 {
    if case % 2 == 1 {
        gate::PUBLISH
    } else {
        gate::CLOSE
    }
}

/// AP1 executes an owner-capability checked endpoint retirement. The BSP's
/// operation is already in flight before either contender may acquire its
/// production serialization guard. No guard spans this rendezvous.
pub(crate) fn idle(cpu: usize) {
    if cpu != 1 {
        return;
    }
    let case = ROUND.load(Ordering::Acquire);
    if case == 0 || CLAIM.swap(case, Ordering::AcqRel) == case {
        return;
    }
    event(case, "close-enter");
    GATE.enter(gate::CLOSE);
    wait(|| GATE.may_run(gate::CLOSE, winner(case)));
    let mut sched = SCHEDULER.lock();
    let server_pid = SERVER.load(Ordering::Relaxed);
    let endpoint = ENDPOINT.load(Ordering::Relaxed) as u32;
    let owner = CapabilityToken(OWNER.load(Ordering::Relaxed));
    {
        let mut caps = crate::capability::CAP_BROKER.lock();
        assert_eq!(
            caps.token_owner(owner, CapabilityRights::RECV_ONLY),
            Ok((endpoint, server_pid))
        );
        assert_eq!(caps.destroy_endpoint(server_pid, owner), Ok(endpoint));
        assert!(caps.destroy_endpoint(server_pid, owner).is_err());
    }
    let calls = with_shard(endpoint, |bus| bus.remove_endpoint(endpoint));
    finish_peer_closed_calls(endpoint, calls, &mut sched);
    assert!(with_shard(endpoint, |bus| bus.remove_endpoint(endpoint)).is_empty());
    event(case, "close-committed");
    GATE.finish(gate::CLOSE);
}

pub(crate) fn run(hhdm: VirtAddr) {
    assert!(!crate::sched::SCHEDULER_READY.load(Ordering::Acquire));
    let frames = crate::PMM.lock().free_page_count();
    let depth = diagnostic_snapshot().current_queue_depth;
    let mut indices = [0; 2];
    for (offset, name) in ["reply-race-caller", "reply-race-server"]
        .into_iter()
        .enumerate()
    {
        let mut sched = SCHEDULER.lock();
        assert!(sched.online_cores >= 2);
        let pid = crate::process::pid::next_pid(sched.processes.iter().map(|p| p.pid)).unwrap();
        let mut pmm = crate::PMM.lock();
        let mut process = unsafe { Process::new(pid, 0, name, &mut pmm, hhdm) };
        drop(pmm);
        process.foreground.incarnation = Foreground::allocate_incarnation().unwrap();
        process.state = ProcessState::BlockedOnIo;
        indices[offset] = sched.processes.len();
        sched.processes.push(process);
        if offset == 0 {
            CALLER.store(pid, Ordering::Relaxed);
        } else {
            SERVER.store(pid, Ordering::Relaxed);
        }
    }
    let [caller, server] = indices;
    let caller_pid = CALLER.load(Ordering::Relaxed);
    let server_pid = SERVER.load(Ordering::Relaxed);
    for case in 1..=4 {
        let mut sched = SCHEDULER.lock();
        let mut caps = crate::capability::CAP_BROKER.lock();
        let (endpoint, owner) = caps.create_endpoint(server_pid).unwrap();
        let send = caps.derive(owner, CapabilityRights::SEND_ONLY).unwrap();
        assert!(caps.destroy_endpoint(caller_pid, owner).is_err());
        assert!(caps.destroy_endpoint(server_pid, send).is_err());
        with_shard(endpoint, |bus| {
            assert!(matches!(
                handle_ipc_call(
                    caller_pid,
                    send,
                    IpcMsg::with_label(case),
                    &mut caps,
                    &mut sched,
                    bus
                ),
                Err(IpcError::WouldBlock)
            ));
            assert_eq!(
                handle_ipc_recv(server_pid, endpoint, &mut sched, bus)
                    .unwrap()
                    .label,
                case
            );
        });
        let target = sched.processes[server].ipc_reply_target.unwrap();
        let token = if case >= 3 {
            Some(defer_current_reply(server_pid, &mut sched).unwrap())
        } else {
            None
        };
        crate::serial_println!("[IPC-RACE] case={} event=identity caller={} caller_inc={} caller_vm={} server={} server_inc={} server_vm={} endpoint={} generation={} scope=kernel-method account_auth=NOT_RUN", case, caller_pid, target.call.incarnation, sched.processes[caller].address_space.identity().generation, server_pid, sched.processes[server].foreground.incarnation, sched.processes[server].address_space.identity().generation, endpoint, target.call.generation);
        drop(caps);
        drop(sched);
        ENDPOINT.store(endpoint as u64, Ordering::Relaxed);
        OWNER.store(owner.0, Ordering::Relaxed);
        GATE.reset();
        ROUND.store(case, Ordering::Release);
        event(case, "publish-enter");
        GATE.enter(gate::PUBLISH);
        wait(|| GATE.may_run(gate::PUBLISH, winner(case)));
        let mut sched = SCHEDULER.lock();
        let mut published = None;
        if let Some(token) = token {
            let mut caps = crate::capability::CAP_BROKER.lock();
            let result = with_shard(endpoint, |bus| {
                complete_deferred_reply(
                    server_pid,
                    token,
                    IpcMsg::with_label(99),
                    &mut sched,
                    &mut caps,
                    bus,
                )
            });
            assert_eq!(result.is_ok(), case == 3);
        } else {
            let before = sched.processes[server].ipc_reply_target;
            let result = defer_current_reply(server_pid, &mut sched);
            crate::serial_println!(
                "[IPC-RACE] case={} event=defer-result accepted={} cpu=0",
                case,
                u8::from(result.is_ok())
            );
            if case == 1 {
                published = Some(result.unwrap());
            } else {
                assert!(matches!(result, Err(IpcError::InvalidArgument)));
                assert_eq!(sched.processes[server].ipc_reply_target, before);
            }
        }
        event(case, "publish-finished");
        GATE.finish(gate::PUBLISH);
        drop(sched);
        wait(|| GATE.done());
        ROUND.store(0, Ordering::Release);
        let mut sched = SCHEDULER.lock();
        let mut caps = crate::capability::CAP_BROKER.lock();
        assert!(!caps.debug_endpoints().iter().any(|(id, _)| *id == endpoint));
        assert!(caps.check(send, CapabilityRights::SEND_ONLY).is_err());
        assert!(caps.check(owner, CapabilityRights::RECV_ONLY).is_err());
        assert!(sched.processes[server].deferred_reply_targets.is_empty());
        for stale in [token, published].into_iter().flatten() {
            assert!(!deferred_reply_is_live(server_pid, stale, &sched));
            assert!(with_shard(endpoint, |bus| complete_deferred_reply(
                server_pid,
                stale,
                IpcMsg::empty(),
                &mut sched,
                &mut caps,
                bus
            ))
            .is_err());
        }
        // An ordinary reply left by failed deferral must not overwrite closure.
        with_shard(endpoint, |bus| {
            handle_ipc_reply(server_pid, IpcMsg::empty(), &mut sched, &mut caps, bus)
        })
        .unwrap();
        let terminal = take_terminal_result(&mut sched, caller)
            .unwrap()
            .map(|m| m.label);
        assert_eq!(
            terminal,
            if case == 3 {
                Ok(99)
            } else {
                Err(IpcError::PeerClosed)
            }
        );
        assert!(take_terminal_result(&mut sched, caller).is_none());
        assert!(sched.processes[caller].pending_call.is_none());
        assert!(sched.processes[server].ipc_reply_target.is_none());
        assert_eq!(
            with_shard(endpoint, |bus| bus.pending_callers_count(endpoint)),
            0
        );
        assert_eq!(with_shard(endpoint, |bus| bus.pending_count(endpoint)), 0);
        sched.remove_from_ready_queues(caller);
        sched.processes[caller].state = ProcessState::BlockedOnIo;
        crate::serial_println!("[IPC-RACE] case={} event=verified terminal={} count=1 duplicate=denied late=denied pending=0 deferred=0 queued=0 waiters=0 endpoint_caps=0", case, if case == 3 { "ReplyDelivered" } else { "PeerClosed" });
    }
    let mut sched = SCHEDULER.lock();
    // Exercise the exact successful-exec withdrawal method without claiming
    // that these boot drivers performed a userspace exec syscall.
    for deferred in [false, true] {
        let mut caps = crate::capability::CAP_BROKER.lock();
        let (endpoint, owner) = caps.create_endpoint(server_pid).unwrap();
        with_shard(endpoint, |bus| {
            assert!(matches!(
                handle_ipc_call(
                    caller_pid,
                    owner,
                    IpcMsg::empty(),
                    &mut caps,
                    &mut sched,
                    bus
                ),
                Err(IpcError::WouldBlock)
            ));
            handle_ipc_recv(server_pid, endpoint, &mut sched, bus).unwrap();
        });
        let ordinary = sched.processes[server].ipc_reply_target;
        let token = if deferred {
            Some(defer_current_reply(server_pid, &mut sched).unwrap())
        } else {
            None
        };
        let counter = sched.processes[server].next_deferred_reply_token;
        drop(caps);
        retire_exec_replies(
            server_pid,
            if deferred { None } else { ordinary },
            &mut sched,
        );
        // Repeating withdrawal is harmless and cannot overwrite the outcome.
        retire_exec_replies(server_pid, ordinary, &mut sched);
        assert_eq!(
            take_terminal_result(&mut sched, caller).map(|r| r.map(|m| m.label)),
            Some(Err(IpcError::PeerClosed))
        );
        assert!(take_terminal_result(&mut sched, caller).is_none());
        assert!(sched.processes[server].deferred_reply_targets.is_empty());
        assert!(sched.processes[server].ipc_reply_target.is_none());
        assert_eq!(sched.processes[server].next_deferred_reply_token, counter);
        if let Some(token) = token {
            assert!(!deferred_reply_is_live(server_pid, token, &sched));
        }
        let mut caps = crate::capability::CAP_BROKER.lock();
        assert_eq!(caps.destroy_endpoint(server_pid, owner), Ok(endpoint));
        assert!(with_shard(endpoint, |bus| bus.remove_endpoint(endpoint)).is_empty());
        sched.remove_from_ready_queues(caller);
        sched.processes[caller].state = ProcessState::BlockedOnIo;
    }
    crate::serial_println!("[IPC-RACE-LIFECYCLE] withdrawal=PASS ordinary=1 deferred=1 duplicate=denied counter=preserved real_exec=NOT_RUN");
    {
        let mut caps = crate::capability::CAP_BROKER.lock();
        let (endpoint, owner) = caps.create_endpoint(server_pid).unwrap();
        with_shard(endpoint, |bus| {
            assert!(matches!(
                handle_ipc_call(
                    caller_pid,
                    owner,
                    IpcMsg::empty(),
                    &mut caps,
                    &mut sched,
                    bus
                ),
                Err(IpcError::WouldBlock)
            ));
            handle_ipc_recv(server_pid, endpoint, &mut sched, bus).unwrap();
        });
        let old = sched.processes[server].ipc_reply_target.unwrap();
        let stale = defer_current_reply(server_pid, &mut sched).unwrap();
        drop(caps);
        // Model only the successful exec withdrawal boundary, not a VM switch.
        sched.processes[caller].pending_call = None;
        retire_exec_call(old, &mut sched);
        retire_exec_call(old, &mut sched);
        assert!(sched.processes[caller].ipc_call_outcome.is_none());
        let mut caps = crate::capability::CAP_BROKER.lock();
        with_shard(endpoint, |bus| {
            assert!(matches!(
                handle_ipc_call(
                    caller_pid,
                    owner,
                    IpcMsg::empty(),
                    &mut caps,
                    &mut sched,
                    bus
                ),
                Err(IpcError::WouldBlock)
            ));
            handle_ipc_recv(server_pid, endpoint, &mut sched, bus).unwrap();
            assert!(complete_deferred_reply(
                server_pid,
                stale,
                IpcMsg::empty(),
                &mut sched,
                &mut caps,
                bus
            )
            .is_err());
            assert!(sched.processes[caller].ipc_call_outcome.is_none());
            handle_ipc_reply(
                server_pid,
                IpcMsg::with_label(100),
                &mut sched,
                &mut caps,
                bus,
            )
            .unwrap();
        });
        assert_eq!(
            take_terminal_result(&mut sched, caller)
                .unwrap()
                .unwrap()
                .label,
            100
        );
        assert!(take_terminal_result(&mut sched, caller).is_none());
        assert_eq!(caps.destroy_endpoint(server_pid, owner), Ok(endpoint));
        assert!(with_shard(endpoint, |bus| bus.remove_endpoint(endpoint)).is_empty());
        sched.remove_from_ready_queues(caller);
        sched.processes[caller].state = ProcessState::BlockedOnIo;
    }
    crate::serial_println!(
        "[IPC-RACE-CALLER] withdrawal=PASS stale=denied next_call=delivered real_exec=NOT_RUN"
    );
    assert!(sched.terminate_process_by_pid(caller_pid, 0, "reply-race-fixture"));
    assert!(sched.terminate_process_by_pid(server_pid, 0, "reply-race-fixture"));
    for index in indices {
        assert_eq!(sched.processes[index].state, ProcessState::Reaped);
        assert!(sched.processes[index].kernel_stack.is_none());
        assert!(sched.processes[index].owned_shared.is_empty());
        assert!(sched.processes[index].mapped_shared.is_empty());
        assert_eq!(sched.diagnostic_ready_occurrences(index), 0);
    }
    assert_eq!(diagnostic_snapshot().current_queue_depth, depth);
    assert_eq!(crate::PMM.lock().free_page_count(), frames);
    // Remove only our two final, fully reaped records before normal boot.
    assert_eq!(sched.processes.len(), server + 1);
    sched.processes.pop();
    sched.processes.pop();
    crate::serial_println!("[IPC-RACE] PASS cases=4 frames_before={} frames_after={} cleanup=reaped scope=kernel-method account_auth=NOT_RUN real_exec=NOT_RUN numeric_pid_reuse=NOT_RUN", frames, frames);
}
