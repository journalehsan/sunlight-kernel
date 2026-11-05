//! Sequential caller-owned exec pilot. No scheduler or reply ownership override.
use super::*;
pub const RELEASE: u64 = 0x6e21;
const STARTED: u64 = 0x6e22;
const BLOCKED: u64 = 0x6e23;

pub(super) fn retain(
    endpoint: &mut Endpoint,
    h: fl::Handle,
    token: u64,
    operation: Operation,
    response: Frame,
) {
    assert!(sunlight_ipc::ipc_deferred_reply_is_live(token));
    endpoint.last_reply = Some(token);
    endpoint.operation = Some(operation);
    endpoint.held_response = Some(response);
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-P1] event=held channel={} request_id=1 operation=65552 reply=deferred",
        h.generation
    ));

}

pub(in super::super::super) fn release(endpoint: &mut Endpoint, m: IpcMsg) {
    let h = endpoint.handles.unwrap();
    assert_eq!(m.words[0], h.generation);
    let token = endpoint.last_reply.unwrap();
    let response = endpoint.held_response.take().unwrap();
    assert!(!sunlight_ipc::ipc_deferred_reply_is_live(token));
    assert!(sunlight_ipc::ipc_complete_deferred_reply(token, response.ipc_message()).is_err());
    assert!(sunlight_ipc::ipc_complete_deferred_reply(token, response.ipc_message()).is_err());
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-P1] event=late-reply channel={} accepted=0 duplicate=rejected",
        h.generation
    ));
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(RELEASE)).unwrap();
}

pub(super) fn enter(
    _actor: Actor,
    h: fl::Handle,
    _control: IpcMsg,
    supervisor: CapabilityToken,
    _page: u64,
) -> ! {
    // IpcCall unconditionally parks a pending caller. Neither a raw syscall
    // continuation nor a mapped readiness flag is a legal asynchronous call.
    // Report the missing primitive through authenticated IPC before submitting
    // that call. Never dereference the peer mapping after a lifecycle change.
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-P1] event=blocked caller={} channel={} reason=nonblocking-call-start-unavailable request_submitted=0",
        sunlight_libc::getpid(), h.generation
    ));
    sunlight_ipc::ipc_call_timeout(
        supervisor,
        IpcMsg::with_label(BLOCKED).word(0, h.generation),
        5000,
    ).unwrap();
    sunlight_libc::exit(0)
}

pub fn replaced(args: &[&str]) -> ! {
    let h = fl::Handle {
        token: args[2].parse().unwrap(),
        generation: args[3].parse().unwrap(),
    };
    let parent = CapabilityToken(args[4].parse().unwrap());
    let pid = sunlight_libc::getpid();
    assert_eq!(pid, args[5].parse::<u64>().unwrap());
    assert!(fl::operate(3, h, 0).is_err());
    assert!(fl::operate(35, h, 0x10010).is_err());
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-P1] event=replacement-entry caller={} channel={} old_vm_adoption=rejected",
        pid,
        h.generation
    ));
    sunlight_ipc::ipc_call_timeout(
        parent,
        IpcMsg::with_label(STARTED).word(0, h.generation),
        5000,
    )
    .unwrap();
    sunlight_libc::exit(0)
}

pub fn supervisor() -> ! {
    doorbell::pin(0);
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let parent = alloc::format!("{}", public.0);
    let child = sunlight_libc::spawn(
        b"/tmp/session-race-probe",
        &[
            b"session-race-probe",
            b"FL-R2-caller",
            parent.as_bytes(),
            b"15",
        ],
        None,
    )
    .unwrap();
    let intro = sunlight_ipc::ipc_recv_timeout(ep, 10000).expect("P1 caller introduction");
    assert_eq!(intro.label, 0x6e10);
    assert_eq!(intro.badge, child);
    let h = fl::Handle {
        token: intro.words[0],
        generation: intro.words[1],
    };
    let actor = Actor {
        pid: intro.words[2],
        send: intro.caps[0],
    };
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(0x6e10)).unwrap();
    let started = sunlight_ipc::ipc_recv_timeout(ep, 10000)
        .expect("P1 original caller did not enter replacement image");
    if started.label == BLOCKED {
        assert_eq!(started.badge, child);
        assert_eq!(started.words[0], h.generation);
        sunlight_ipc::ipc_reply_result(IpcMsg::with_label(BLOCKED)).unwrap();
        assert_eq!(crate::reap(child), 0);
        expect(actor, EXIT, fl::EMPTY, 0, 0);
        // The reaped caller cannot authorize the scoped inventory observer.
        // No hybrid endpoint was installed, so its inventory is unavailable.
        sunlight_ipc::debug_log("[FL-P1] event=blocked-cleanup checkpoint=FAIL reason=nonblocking-call-start-unavailable scoped_inventory=UNVERIFIED");
        sunlight_libc::exit(1)
    }
    assert_eq!(started.label, STARTED);
    assert_eq!(started.badge, child);
    assert_eq!(started.words[0], h.generation);
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(STARTED)).unwrap();
    let result = sunlight_ipc::ipc_call_timeout(
        actor.send,
        IpcMsg::with_label(RELEASE).word(0, h.generation),
        5000,
    )
    .unwrap();
    assert_eq!(result.label, RELEASE);
    assert_eq!(crate::reap(child), 0);
    expect(actor, EXIT, fl::EMPTY, 0, 0);
    await_retirement(h);
    mapping::census(715);
    zero();
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-P1] event=complete pid={} caller={} channel={} scope=sequential",
        sunlight_libc::getpid(),
        child,
        h.generation
    ));
    sunlight_libc::exit(0)
}
