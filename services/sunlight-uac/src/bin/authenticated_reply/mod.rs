//! Opt-in userspace protocol. Only ordinary IPC/exec/endpoint syscalls are used.
//! Closure is owner-ordered: this does not manufacture an independent closer.
#![allow(dead_code)] // Shared by the Account daemon and the ordinary probe binary.
use sunlight_ipc::{CapabilityToken, EndpointId, IpcCallError, IpcError, IpcMsg};

const ROUNDS: u64 = 15;
const SETUP: u64 = 0xE311;
const CALL: u64 = 0xE313;
const ACK: u64 = 0xE314;

fn note(round: u64, event: &str) {
    sunlight_ipc::debug_log(&alloc::format!(
        "[AUTH-REPLY] round={} event={} pid={}",
        round,
        event,
        sunlight_libc::getpid()
    ));
}
fn observe(round: u64, event: &str) {
    sunlight_ipc::debug_log(&alloc::format!(
        "[AUTH-OBSERVE] round={} event={}",
        round,
        event
    ));
}
fn denied(token: u64) {
    assert!(!sunlight_ipc::ipc_deferred_reply_is_live(token));
    assert_eq!(
        sunlight_ipc::ipc_complete_deferred_reply(token, IpcMsg::with_label(CALL)),
        Err(IpcError::InvalidArgument)
    );
}
fn retired_endpoint(cap: CapabilityToken) {
    assert!(matches!(
        sunlight_ipc::ipc_call_timeout(cap, IpcMsg::with_label(CALL), 100),
        Err(IpcCallError::InvalidCapability | IpcCallError::EndpointNotFound)
    ));
}
fn receive(ep: EndpointId, label: u64, round: u64, account: u64) -> IpcMsg {
    let msg = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("reply peer receive");
    assert_eq!(msg.label, label);
    assert_eq!(msg.words[0], round);
    assert_eq!(msg.badge, account);
    msg
}

pub fn account(control: CapabilityToken) {
    for round in 1..=ROUNDS {
        let setup =
            sunlight_ipc::ipc_call_timeout(control, IpcMsg::with_label(SETUP).word(0, round), 5000)
                .expect("reply peer setup");
        assert_eq!(setup.label, SETUP);
        assert_eq!(setup.cap_count, 2);
        // Even possession of the owner's token cannot impersonate its PID.
        assert!(!sunlight_ipc::endpoint_destroy(EndpointId(setup.caps[1].0)));
        assert!(!sunlight_ipc::endpoint_destroy(EndpointId(setup.caps[0].0)));
        assert!(sunlight_ipc::ipc_recv_timeout(EndpointId(setup.caps[1].0), 1).is_none());
        note(round, "foreign-owner-and-send-only-denied");
        let outcome = sunlight_ipc::ipc_call_timeout(
            setup.caps[0],
            IpcMsg::with_label(CALL).word(0, round),
            5000,
        );
        if round % 5 == 3 {
            assert_eq!(outcome.expect("completion wins").label, CALL);
            note(round, "ReplyDelivered");
        } else {
            assert!(matches!(outcome, Err(IpcCallError::PeerClosed)));
            note(round, "PeerClosed");
        }
        observe(round, "consumed");
        if round == ROUNDS {
            retired_endpoint(control);
            retired_endpoint(setup.caps[0]);
            note(round, "retired-endpoint-denied");
            continue;
        }
        // The peer tries the old completion against this fresh pending call.
        let ack =
            sunlight_ipc::ipc_call_timeout(control, IpcMsg::with_label(ACK).word(0, round), 5000)
                .expect("fresh call survives stale completion");
        assert_eq!(ack.label, ACK);
        // Reusing a retired endpoint capability must fail before publication.
        retired_endpoint(setup.caps[0]);
        note(round, "retired-endpoint-denied");
    }
    note(0, "account-rounds-complete");
}

pub fn controller(ep: EndpointId, account: u64) {
    let pending = sunlight_ipc::ipc_defer_reply().expect("defer authenticated Account rendezvous");
    let handle = alloc::format!("{}", sunlight_ipc::endpoint_bind(ep.0).0);
    let foreign = alloc::format!("{}", pending);
    let child = sunlight_libc::spawn(
        b"/bin/session-racer",
        &[
            b"reply-peer",
            b"reply-peer",
            handle.as_bytes(),
            foreign.as_bytes(),
        ],
        None,
    )
    .expect("ordinary reply peer spawn");
    let ready = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("reply peer bootstrap");
    assert_eq!(ready.badge, child);
    assert_eq!(ready.label, SETUP);
    assert_eq!(ready.cap_count, 1);
    assert!(sunlight_ipc::ipc_deferred_reply_is_live(pending));
    note(0, "foreign-target-still-live");
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(SETUP)).expect("reply peer ready");
    sunlight_ipc::ipc_complete_deferred_reply(
        pending,
        IpcMsg::with_label(0xE301).with_cap(0, ready.caps[0]),
    )
    .expect("release Account for real reply operations");
    denied(pending);
    let start = sunlight_ipc::monotonic_millis();
    let mut status = None;
    for _ in 0..1_000_000 {
        status = sunlight_libc::try_waitpid(child).expect("reply peer child status");
        if status.is_some() {
            break;
        }
        assert!(sunlight_ipc::monotonic_millis().saturating_sub(start) < 15000);
        sunlight_ipc::process_yield();
    }
    assert_eq!(status, Some(0));
    let done = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("Account rounds completed");
    assert_eq!(done.label, 0xE312);
    assert_eq!(done.badge, account);
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(0xE301))
        .expect("release Account to userspace");
    note(0, "peer-reaped");
}

pub fn peer(args: &[&str]) -> ! {
    assert_eq!(sunlight_libc::getuid(), 0);
    let (stable, first, mut stale, mut account) = if args[1] == "reply-exec" {
        assert_eq!(args.len(), 7);
        let stable = EndpointId(args[2].parse().expect("inherited endpoint"));
        let ep = EndpointId(args[3].parse().expect("old endpoint"));
        let token = args[4].parse().expect("retired completion");
        let round = args[5].parse().expect("exec round");
        let account = args[6].parse().expect("Account peer");
        observe(round, "exec-return");
        denied(token);
        assert!(sunlight_ipc::endpoint_destroy(ep));
        receive(stable, ACK, round, account);
        denied(token);
        sunlight_ipc::ipc_reply_result(IpcMsg::with_label(ACK)).expect("inherited endpoint reply");
        note(round, "exec-stale-denied-fresh-delivered");
        (stable, round + 1, Some(token), account)
    } else {
        assert_eq!(args.len(), 4);
        denied(args[3].parse().expect("other server live completion"));
        note(0, "foreign-live-completion-denied");
        let bootstrap = CapabilityToken(args[2].parse().expect("reply peer bootstrap"));
        let stable = sunlight_ipc::endpoint_create();
        sunlight_ipc::ipc_call_timeout(
            bootstrap,
            IpcMsg::with_label(SETUP).with_cap(0, sunlight_ipc::endpoint_bind(stable.0)),
            5000,
        )
        .expect("reply peer bootstrap accepted");
        (stable, 1, None, 0)
    };
    for round in first..=ROUNDS {
        let setup = sunlight_ipc::ipc_recv_timeout(stable, 5000).expect("Account setup");
        assert_eq!(setup.label, SETUP);
        assert_eq!(setup.words[0], round);
        if account == 0 {
            account = setup.badge;
        }
        assert_eq!(setup.badge, account);
        let ep = sunlight_ipc::endpoint_create();
        sunlight_ipc::ipc_reply_result(
            IpcMsg::with_label(SETUP)
                .with_cap(0, sunlight_ipc::endpoint_bind(ep.0))
                .with_cap(1, CapabilityToken(ep.0)),
        )
        .expect("new endpoint delegation");
        receive(ep, CALL, round, account);
        observe(round, "received");
        if let Some(token) = stale {
            denied(token);
            note(round, "prior-token-denied-with-new-pending");
        }
        if round % 5 == 2 {
            assert!(sunlight_ipc::endpoint_destroy(ep));
            assert_eq!(
                sunlight_ipc::ipc_defer_reply(),
                Err(IpcError::InvalidArgument)
            );
            // The ordinary stale reply is discarded using its existing contract.
            let _ = sunlight_ipc::ipc_reply_result(IpcMsg::with_label(CALL));
            note(round, "close-before-defer-denied");
        } else {
            let token = sunlight_ipc::ipc_defer_reply().expect("live authenticated call deferral");
            assert!(sunlight_ipc::ipc_deferred_reply_is_live(token));
            if round == ROUNDS {
                observe(round, "shutdown-enter");
                note(round, "shutdown-outstanding");
                note(0, "peer-exit");
                // Real exit owns endpoint/target cleanup; no explicit close or
                // completion is issued for this still-pending Account call.
                sunlight_libc::exit(0);
            }
            if round % 5 == 0 {
                assert!(sunlight_libc::exec(b"/missing-reply-exec", &[b"reply-peer"]).is_err());
                assert!(sunlight_ipc::ipc_deferred_reply_is_live(token));
                observe(round, "exec-enter");
                let values =
                    [stable.0, ep.0, token, round, account].map(|v| alloc::format!("{}", v));
                sunlight_libc::exec(
                    b"/bin/session-racer",
                    &[
                        b"reply-peer",
                        b"reply-exec",
                        values[0].as_bytes(),
                        values[1].as_bytes(),
                        values[2].as_bytes(),
                        values[3].as_bytes(),
                        values[4].as_bytes(),
                    ],
                )
                .expect("real exec with outstanding Account reply");
                panic!("successful exec returned to retired image");
            }
            if round % 5 == 3 {
                sunlight_ipc::ipc_complete_deferred_reply(token, IpcMsg::with_label(CALL))
                    .expect("completion wins");
            }
            assert!(sunlight_ipc::endpoint_destroy(ep));
            denied(token);
            stale = Some(token);
            note(
                round,
                match round % 5 {
                    1 => "defer-before-close",
                    3 => "complete-before-close",
                    _ => "close-before-complete-denied",
                },
            );
        }
        assert!(!sunlight_ipc::endpoint_destroy(ep));
        receive(stable, ACK, round, account);
        if let Some(token) = stale {
            denied(token);
        }
        sunlight_ipc::ipc_reply_result(IpcMsg::with_label(ACK)).expect("fresh call reply");
        note(round, "fresh-delivered-after-late-denial");
    }
    assert!(sunlight_ipc::endpoint_destroy(stable));
    note(0, "peer-exit");
    sunlight_libc::exit(0)
}
