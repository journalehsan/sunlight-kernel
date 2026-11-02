//! Actual Account-authorized independent native VMs. No mapped payload access.
use super::*;
const DONE: u64 = 0xf1c430;
pub(super) fn pin(cpu: u64) {
    assert_eq!(unsafe { sunlight_libc::sys::syscall1(0xf1c3, cpu) }, 0);
}
pub(super) fn spawn(ep: EndpointId, public: CapabilityToken, cpu: u64) -> Actor {
    let parent = alloc::format!("{}", public.0);
    let cpu = alloc::format!("{}", cpu);
    let pid = sunlight_libc::spawn(
        b"/bin/session-racer",
        &[
            b"session-racer",
            b"FL-actor",
            parent.as_bytes(),
            cpu.as_bytes(),
        ],
        None,
    )
    .unwrap();
    let hello = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("placed actor hello");
    assert_eq!(hello.label, HELLO);
    assert_eq!(hello.badge, pid);
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(HELLO)).unwrap();
    Actor {
        pid,
        send: hello.caps[0],
    }
}
fn expect(a: Actor, op: u64, h: fl::Handle, arg: u64, code: u64) -> IpcMsg {
    super::expect(
        a,
        if (10..=17).contains(&op) { op + 30 } else { op },
        h,
        arg,
        code,
    )
}
fn mark(case: &str, h: fl::Handle, detail: u64) {
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-M4] case={} generation={} detail={} pid={}",
        case,
        h.generation,
        detail,
        sunlight_libc::getpid()
    ));
}
pub(super) fn actor_command(op: u64, h: fl::Handle, seq: u64, parent: CapabilityToken) {
    if op == 32 {
        let (code, _) = result(fl::doorbell_wait(h, seq).map(|_| 0));
        reply(code, 0, 0);
        return;
    }
    reply(0, 0, 0); // Release controller before registering/blocking.
    let waited = fl::doorbell_wait(h, seq);
    let (code, value) = result(match waited {
        Ok(()) => fl::operate(13, h, 0),
        Err(e) => Err(e),
    });
    sunlight_ipc::ipc_call_timeout(
        parent,
        IpcMsg::with_label(DONE)
            .word(0, code)
            .word(1, value)
            .word(2, h.generation)
            .word(3, u64::from(waited.is_ok())),
        5000,
    )
    .expect("wait completion IPC");
}
pub(super) fn channel(p: Actor, c: Actor) -> fl::Handle {
    let offer = super::expect(p, AUTHORIZE, fl::EMPTY, 60000, 0);
    let h = fl::Handle {
        token: offer.words[1],
        generation: offer.words[2],
    };
    let receipt = super::expect(c, AUTHORIZE, h, 1, 0);
    expect(p, 2, h, receipt.words[1], 0);
    expect(p, 3, h, 0, 0);
    expect(c, 3, h, 1, 0);
    h
}
fn done(ep: EndpointId, c: Actor, h: fl::Handle, code: u64, value: u64, registered: bool) {
    let m = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("consumer made progress");
    assert_eq!(m.label, DONE);
    assert_eq!(m.badge, c.pid);
    assert_eq!(
        m.words[..4],
        [code, value, h.generation, u64::from(registered)]
    );
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(DONE)).unwrap();
}
fn asleep(ep: EndpointId, p: Actor, h: fl::Handle) {
    // An ordinary IPC deadline parks the controller; no notification polling.
    assert!(sunlight_ipc::ipc_recv_timeout(ep, 50).is_none());
    assert_eq!(expect(p, 17, h, 0, 0).words[1], 1 | (1 << 32));
}
fn clean(p: Actor, h: fl::Handle) {
    expect(p, 4, h, 0, 0);
    assert_eq!(expect(p, 17, h, 0, 0).words[1], 0);
    zero();
}
pub fn supervisor() -> ! {
    pin(0);
    mark("start", fl::EMPTY, 0);
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let mut p = spawn(ep, public, 0);
    let mut c = spawn(ep, public, 1);
    let foreign = spawn(ep, public, 1);
    let h = channel(p, c);
    expect(c, 10, h, 0, 1);
    expect(p, 11, h, 0, 1);
    expect(p, 32, h, 0, 1);
    for op in [10, 11, 14, 32] {
        expect(foreign, op, h, 0, 1);
    }
    let stale = fl::Handle {
        generation: h.generation + 1,
        ..h
    };
    expect(p, 10, stale, 0, 2);
    expect(c, 32, stale, 0, 2);
    mark("unauthorized-stolen-stale", h, 0);
    expect(p, 10, h, 0, 0);
    assert_eq!(expect(c, 11, h, 0, 0).words[1], 1);
    expect(c, 30, h, 0, 0);
    done(ep, c, h, 10, 0, false);
    mark("notify-before-wait", h, 1);
    for seq in 2..=9 {
        expect(c, 30, h, seq - 1, 0);
        asleep(ep, p, h);
        assert_eq!(expect(p, 10, h, 0, 0).words[1], seq);
        done(ep, c, h, 0, seq, true);
    }
    mark("sleeping-repeated", h, 9);
    clean(p, h);
    for mode in 1..=2 {
        let h = channel(p, c);
        expect(p, 15, h, mode, 0);
        expect(c, 30, h, 0, 0);
        expect(p, 10, h, 0, 0);
        done(
            ep,
            c,
            h,
            if mode == 1 { 0 } else { 10 },
            if mode == 1 { 1 } else { 0 },
            mode == 1,
        );
        mark("registration-race", h, mode);
        clean(p, h);
    }
    // Cancellation/notification in both arbitration orders. The already
    // committed completion must remain unchanged by the later operation.
    for cancel_first in [true, false] {
        let h = channel(p, c);
        expect(c, 30, h, 0, 0);
        asleep(ep, p, h);
        expect(p, if cancel_first { 14 } else { 10 }, h, 0, 0);
        expect(p, if cancel_first { 10 } else { 14 }, h, 0, 0);
        done(
            ep,
            c,
            h,
            if cancel_first { 9 } else { 0 },
            if cancel_first { 0 } else { 1 },
            true,
        );
        mark("cancel-notify-orders", h, u64::from(cancel_first));
        clean(p, h);
    }
    // Real SIGCONT follows normal foreground signal authorization and cancels
    // the sleeping consumer through the existing scheduler wake path. It races
    // a producer notification on another CPU in both forced lock orders.
    for mode in 5..=6 {
        let h = channel(p, c);
        expect(c, 30, h, 0, 0);
        asleep(ep, p, h);
        expect(p, 15, h, mode, 0);
        expect(foreign, 33, h, c.pid, 0);
        expect(p, 10, h, 0, 0);
        done(
            ep,
            c,
            h,
            if mode == 5 { 9 } else { 0 },
            if mode == 5 { 0 } else { 1 },
            true,
        );
        // Synchronize with the actual signal sender after its syscall returns.
        expect(foreign, 11, h, 0, 1);
        clean(p, h);
        mark("signal-cancel-notify", h, mode);
    }
    let h = channel(p, c);
    expect(c, 30, h, 0, 0);
    asleep(ep, p, h);
    expect(p, 4, h, 0, 0);
    done(ep, c, h, 4, 0, true);
    expect(c, 32, h, 0, 4);
    mark("revocation-wait", h, 0);
    clean(p, h);
    let h = channel(p, c);
    expect(c, 30, h, 0, 0);
    asleep(ep, p, h);
    stop(p); // real producer ProcessExit, controller has no channel authority
    done(ep, c, h, 4, 0, true);
    zero();
    mark("producer-exit-wait", h, 0);
    p = spawn(ep, public, 0);
    for mode in 3..=4 {
        let h = channel(p, c);
        expect(p, 15, h, mode, 0);
        super::expect(c, RACE_EXIT, h, 0, 0);
        expect(p, 10, h, 0, if mode == 3 { 4 } else { 0 });
        assert_eq!(super::super::reap(c.pid), 0);
        mark("consumer-exit-notify", h, mode);
        clean(p, h);
        c = spawn(ep, public, 1);
    }
    let h = channel(p, c);
    expect(p, 16, h, u64::MAX - 1, 0); // bounded, authorized debug seeding only
    assert_eq!(expect(p, 10, h, 0, 0).words[1], u64::MAX);
    expect(c, 30, h, u64::MAX, 0);
    asleep(ep, p, h);
    expect(p, 10, h, 0, 6);
    done(ep, c, h, 6, 0, true);
    expect(c, 11, h, 0, 6);
    mark("sequence-exhaustion", h, 0);
    clean(p, h);
    for cycle in 0..16 {
        let h = channel(p, c);
        expect(c, 30, h, 0, 0);
        asleep(ep, p, h);
        expect(p, 10, h, 0, 0);
        done(ep, c, h, 0, 1, true);
        clean(p, h);
        mark("reclaimed-cycle", h, cycle);
    }
    let h = channel(p, c);
    super::expect(c, FAILED_EXEC, h, 0, 0);
    assert!(sunlight_ipc::ipc_call_timeout(
        c.send,
        IpcMsg::with_label(CMD)
            .word(0, EXEC)
            .word(1, h.token)
            .word(2, h.generation),
        5000
    )
    .is_err());
    assert_eq!(super::super::reap(c.pid), 0);
    expect(p, 10, h, 0, 4);
    clean(p, h);
    mark("exec-authority-retired", h, 0);
    c = spawn(ep, public, 1);
    let h = channel(p, c);
    expect(c, 30, h, 0, 0);
    asleep(ep, p, h);
    sunlight_libc::kill(c.pid, 9).expect("actual blocked consumer exit");
    assert_eq!(super::super::reap(c.pid), 137);
    expect(p, 10, h, 0, 4);
    clean(p, h);
    mark("consumer-killed-wait", h, 0);
    c = spawn(ep, public, 1);
    stop(p);
    stop(c);
    stop(foreign);
    assert!(sunlight_ipc::endpoint_destroy(ep));
    zero();
    mark("complete", fl::EMPTY, 0);
    sunlight_libc::exit(0)
}
