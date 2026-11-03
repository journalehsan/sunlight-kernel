//! Authenticated userspace actors using ordinary IPC for coordination.
#[cfg(feature = "fastlane-doorbell-test")]
pub(super) mod doorbell;
#[cfg(feature = "fastlane-mapping-test")]
pub(super) mod mapping;
#[cfg(feature = "fastlane-repeated-test")]
pub(super) mod repeated;
#[cfg(feature = "fastlane-transfer-test")]
pub(super) mod transfer;
#[cfg(feature = "fastlane-shared-ring-test")]
pub(super) mod shared_ring;
use sunlight_ipc::{fastlane_authority as fl, CapabilityToken, EndpointId, IpcMsg};
const HELLO: u64 = 0xF1C110;
const CMD: u64 = 0xF1C111;
const EXIT: u64 = 10;
const EXEC: u64 = 11;
const FAILED_EXEC: u64 = 12;
const AUTHORIZE: u64 = 13;
const RACE_ATTACH: u64 = 14;
const RACE_EXIT: u64 = 15;
const RACE_DONE: u64 = 16;
const BORROWER: u64 = 17;
const COORD_RECEIPT: u64 = 18;
fn note(case: &str, result: u64) {
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-USER] case={} pid={} result={}",
        case,
        sunlight_libc::getpid(),
        result
    ));
}
fn lookup() -> CapabilityToken {
    super::endpoint("uac")
}
fn authorize(op: u64, h: fl::Handle, ttl: u64) -> Result<fl::Handle, u64> {
    let m = sunlight_ipc::ipc_call_timeout(
        lookup(),
        IpcMsg::with_label(fl::REQUEST)
            .word(0, op)
            .word(1, h.token)
            .word(2, h.generation)
            .word(3, ttl),
        5000,
    )
    .expect("Account authorization IPC");
    assert_eq!(m.label, fl::REPLY);
    if m.words[0] != 0 {
        return Err(m.words[0]);
    }
    Ok(fl::Handle {
        token: m.words[1],
        generation: m.words[2],
    })
}
fn reply(code: u64, value: u64, generation: u64) {
    sunlight_ipc::ipc_reply_result(
        IpcMsg::with_label(CMD)
            .word(0, code)
            .word(1, value)
            .word(2, generation),
    )
    .expect("actor reply");
}
fn result(r: Result<u64, fl::Error>) -> (u64, u64) {
    match r {
        Ok(v) => (0, v),
        Err(e) => (e as u64, 0),
    }
}
pub fn actor(args: &[&str]) -> ! {
    #[cfg(feature = "fastlane-doorbell-test")]
    if args.len() > 3 {
        doorbell::pin(args[3].parse().unwrap());
    }
    let parent = CapabilityToken(args[2].parse().unwrap());
    let ep = sunlight_ipc::endpoint_create();
    let send = sunlight_ipc::endpoint_bind(ep.0);
    let hello =
        sunlight_ipc::ipc_call_timeout(parent, IpcMsg::with_label(HELLO).with_cap(0, send), 5000)
            .unwrap();
    assert_eq!(hello.label, HELLO);
    #[cfg(feature = "fastlane-transfer-test")]
    let mut transfer = transfer::Endpoint::new();
    #[cfg(feature = "fastlane-repeated-test")]
    let mut repeated = repeated::Endpoint::new();
    #[cfg(feature = "fastlane-shared-ring-test")]
    let mut shared_ring = shared_ring::Endpoint::new();
    #[cfg(feature = "fastlane-benchmark-test")]
    let mut benchmark = shared_ring::benchmark::Consumer::new();
    #[cfg(feature = "fastlane-benchmark-test")]
    let mut fair_benchmark = shared_ring::fair_benchmark::Consumer::new();
    loop {
        let m = sunlight_ipc::ipc_recv_timeout(ep, 10000).expect("actor command");
        #[cfg(feature = "fastlane-benchmark-test")]
        if shared_ring::benchmark::handles(m.label) {
            benchmark.request(m);
            continue;
        }
        #[cfg(feature = "fastlane-benchmark-test")]
        if shared_ring::fair_benchmark::handles(m.label) {
            fair_benchmark.request(m);
            continue;
        }
        assert_eq!(m.label, CMD);
        let h = fl::Handle {
            token: m.words[1],
            generation: m.words[2],
        };
        match m.words[0] {
            #[cfg(feature = "fastlane-benchmark-test")]
            100..=103 => benchmark.command(m, parent),
            #[cfg(feature = "fastlane-benchmark-test")]
            110..=113 => fair_benchmark.command(m, parent),
            // Userspace command numbers are separate from kernel opcodes.
            // Kernel opcode 23 overlaps the mapping fixture's COLLIDE command.
            COORD_RECEIPT => {
                let (code, value) = result(fl::operate(23, h, 0));
                reply(code, value, 0);
            }
            #[cfg(feature = "fastlane-shared-ring-test")]
            op @ 80..=90 | op @ 92..=96 => shared_ring.command(op - 10, h, m.words[3], parent),
            #[cfg(feature = "fastlane-shared-ring-test")]
            91 => { let (code, value) = result(fl::operate(24, h, m.words[3])); reply(code, value, 0); }
            #[cfg(feature = "fastlane-repeated-test")]
            op @ 70..=79 => repeated.command(op, h, m.words[3], parent),
            #[cfg(feature = "fastlane-transfer-test")]
            op @ 60..=69 => transfer.command(op, h, m.words[3], parent),
            #[cfg(feature = "fastlane-doorbell-test")]
            op @ 40..=47 => {
                let (code, value) = result(fl::operate(op - 30, h, m.words[3]));
                reply(code, value, 0);
            }
            #[cfg(feature = "fastlane-doorbell-test")]
            33 => {
                reply(0, 0, 0);
                sunlight_libc::kill(m.words[3], 18).expect("OS-authorized cancellation signal");
            }
            #[cfg(feature = "fastlane-doorbell-test")]
            30 | 32 => doorbell::actor_command(m.words[0], h, m.words[3], parent),
            #[cfg(feature = "fastlane-mapping-test")]
            op @ 20..=27 => mapping::actor_command(op, h, m.words[3]),
            EXIT => {
                reply(0, 0, 0);
                sunlight_libc::exit(0)
            }
            EXEC => {
                let token = alloc::format!("{}", h.token);
                let generation = alloc::format!("{}", h.generation);
                sunlight_libc::exec(
                    b"/bin/session-racer",
                    &[
                        b"session-racer",
                        b"FL-replaced",
                        token.as_bytes(),
                        generation.as_bytes(),
                    ],
                )
                .expect("channel participant exec");
                panic!("exec returned")
            }
            FAILED_EXEC => {
                assert!(sunlight_libc::exec(b"/missing-channel-exec", &[b"missing"]).is_err());
                let (code, value) = result(fl::operate(5, h, 0));
                reply(code, value, 0);
            }
            AUTHORIZE => match authorize(
                m.words[3] & 1,
                h,
                if m.words[3] > 1 {
                    m.words[3] >> 1
                } else {
                    30000
                },
            ) {
                Ok(h) => reply(0, h.token, h.generation),
                Err(e) => reply(e, 0, 0),
            },
            BORROWER => {
                // Valid native thread bootstrap, then the real sharing entry.
                // The membership check must reject before publishing a thread.
                extern "C" fn unused(_: *mut u8) -> *mut u8 {
                    core::ptr::null_mut()
                }
                let mut stack = alloc::vec![0u64;4096];
                let end = stack.len();
                stack[end - 2] = unused as *const () as u64;
                stack[end - 1] = 0;
                let value = unsafe {
                    sunlight_libc::sys::syscall3(
                        22,
                        unused as *const () as u64,
                        stack.as_ptr().add(end - 2) as u64,
                        stack.as_ptr() as u64,
                    )
                };
                assert_eq!(value, u64::MAX);
                reply(0, 0, 0);
            }
            RACE_EXIT => {
                reply(0, 0, 0);
                sunlight_libc::exit(0)
            }
            RACE_ATTACH => {
                // Reply before the operation so the producer can independently
                // invoke revoke. Actual overlap is assessed separately.
                reply(0, 0, 0);
                let (code, _) = result(fl::operate(3, h, 1));
                let start = sunlight_ipc::monotonic_millis();
                let done = sunlight_ipc::ipc_call_timeout(
                    parent,
                    IpcMsg::with_label(RACE_DONE)
                        .word(0, code)
                        .word(1, h.generation),
                    5000,
                )
                .unwrap();
                assert_eq!(done.label, RACE_DONE);
                assert!(sunlight_ipc::monotonic_millis() - start < 5000);
            }
            op => {
                let (code, value) = result(fl::operate(op, h, m.words[3]));
                reply(code, value, 0);
            }
        }
    }
}
pub fn replaced(args: &[&str]) -> ! {
    let h = fl::Handle {
        token: args[2].parse().unwrap(),
        generation: args[3].parse().unwrap(),
    };
    assert_eq!(fl::operate(3, h, 1), Err(fl::Error::Terminal));
    assert_eq!(fl::operate(2, h, 0), Err(fl::Error::Denied));
    #[cfg(feature = "fastlane-doorbell-test")]
    {
        assert_eq!(fl::operate(11, h, 0), Err(fl::Error::Denied));
        assert_eq!(fl::doorbell_wait(h, 0), Err(fl::Error::Denied));
    }
    note("replacement-denied", 0);
    sunlight_libc::exit(0)
}
#[derive(Clone, Copy)]
struct Actor {
    pid: u64,
    send: CapabilityToken,
}
fn spawn(ep: EndpointId, public: CapabilityToken) -> Actor {
    let parent = alloc::format!("{}", public.0);
    let pid = sunlight_libc::spawn(
        b"/bin/session-racer",
        &[b"session-racer", b"FL-actor", parent.as_bytes()],
        None,
    )
    .expect("independent VM actor");
    let hello = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("actor hello");
    assert_eq!(hello.label, HELLO);
    assert_eq!(hello.badge, pid);
    let send = hello.caps[0];
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(HELLO)).unwrap();
    Actor { pid, send }
}
fn command(a: Actor, op: u64, h: fl::Handle, arg: u64) -> IpcMsg {
    sunlight_ipc::ipc_call_timeout(
        a.send,
        IpcMsg::with_label(CMD)
            .word(0, op)
            .word(1, h.token)
            .word(2, h.generation)
            .word(3, arg),
        5000,
    )
    .expect("actor operation")
}
fn expect(a: Actor, op: u64, h: fl::Handle, arg: u64, code: u64) -> IpcMsg {
    let m = command(a, op, h, arg);
    assert_eq!(m.label, CMD);
    assert_eq!(m.words[0], code);
    m
}
fn stop(a: Actor) {
    expect(a, EXIT, fl::EMPTY, 0, 0);
    assert_eq!(super::reap(a.pid), 0);
}
fn offer() -> fl::Handle {
    authorize(0, fl::EMPTY, 30000).expect("producer Account permit")
}
fn pair(a: Actor) -> fl::Handle {
    let h = offer();
    let consent = expect(a, AUTHORIZE, h, 1, 0);
    assert_eq!(fl::operate(2, h, consent.words[1]), Ok(0));
    h
}
fn zero() {
    assert_eq!(fl::operate(6, fl::EMPTY, 0), Ok(0));
}
pub fn supervisor() -> ! {
    // The independent session-racer is placed on CPU 1 by the debug image.
    // Reserve CPU 0 eligibility for this controller before releasing it.
    assert_eq!(unsafe { sunlight_libc::sys::syscall1(0xf1c3, 0) }, 0);
    assert_eq!(sunlight_libc::getuid(), 0);
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let c = spawn(ep, public);
    let foreign = spawn(ep, public);
    assert_eq!(
        fl::call(0, fl::EMPTY, 30000, fl::NO_SESSION),
        Err(fl::Error::Denied)
    );
    let h = offer();
    assert_eq!(fl::operate(2, h, 0), Err(fl::Error::Incomplete));
    assert_eq!(authorize(0, fl::EMPTY, 30000), Err(3));
    let consent = expect(c, AUTHORIZE, h, 1, 0);
    expect(foreign, AUTHORIZE, h, 1, 3); // immutable consumer, no reassignment
    expect(c, 2, h, consent.words[1], 1);
    expect(foreign, 2, h, consent.words[1], 1);
    assert_eq!(fl::operate(2, h, 0), Err(fl::Error::Denied));
    assert_eq!(fl::operate(2, h, h.token), Err(fl::Error::Denied));
    assert_eq!(fl::operate(2, h, consent.words[1]), Ok(0));
    assert_eq!(fl::operate(2, h, 0), Err(fl::Error::Duplicate));
    note("authorized-create", 0);
    for role in 0..2 {
        expect(foreign, 3, h, role, 1);
    }
    expect(c, 3, h, 0, 1);
    assert_eq!(fl::operate(3, h, 1), Err(fl::Error::Denied));
    let stale = fl::Handle {
        generation: h.generation + 1,
        ..h
    };
    expect(c, 3, stale, 1, 2);
    assert_eq!(fl::operate(3, h, 0), Ok(0));
    expect(c, 3, h, 1, 0);
    assert_eq!(fl::operate(3, h, 0), Err(fl::Error::Duplicate));
    expect(c, 3, h, 1, 3);
    expect(foreign, 4, h, 0, 1);
    expect(c, BORROWER, h, 0, 0);
    note("recipient-role-duplicate-stale", 0);
    assert_eq!(fl::operate(4, h, 0), Ok(0));
    expect(c, 3, h, 1, 4);
    let terminal = fl::operate(5, h, 0).unwrap();
    assert_eq!(terminal & 255, 4);
    assert_eq!(fl::operate(4, h, 0), Ok(0));
    assert_eq!(fl::operate(5, h, 0), Ok(terminal));
    zero();
    note("revoked-terminal", 0);
    let reused = pair(c);
    assert_ne!(reused.generation, h.generation);
    assert_eq!(fl::operate(3, h, 0), Err(fl::Error::Stale));
    expect(c, 3, reused, 1, 0);
    let preserved = expect(c, FAILED_EXEC, reused, 0, 0);
    assert_ne!(preserved.words[1] & (1 << 17), 0);
    expect(c, 3, reused, 1, 3);
    note("failed-exec-preserved", 0);
    // Successful real exec withdraws the old VM's one-use role. The old
    // outstanding IPC is retired by the established exec contract.
    assert!(sunlight_ipc::ipc_call_timeout(
        c.send,
        IpcMsg::with_label(CMD)
            .word(0, EXEC)
            .word(1, reused.token)
            .word(2, reused.generation),
        5000
    )
    .is_err());
    assert_eq!(super::reap(c.pid), 0);
    assert_eq!(fl::operate(3, reused, 0), Err(fl::Error::Terminal));
    zero();
    note("successful-exec-retired", 0);
    let c = spawn(ep, public);
    let incomplete = pair(c);
    stop(c);
    assert_eq!(fl::operate(3, incomplete, 0), Err(fl::Error::Terminal));
    zero();
    note("peer-exit-incomplete", 0);
    // A new PID/incarnation cannot adopt the exited peer's role.
    let replacement = spawn(ep, public);
    expect(replacement, 3, incomplete, 1, 4);
    let own = expect(replacement, AUTHORIZE, fl::EMPTY, 60000, 0);
    let orphan = fl::Handle {
        token: own.words[1],
        generation: own.words[2],
    };
    stop(replacement);
    assert_eq!(fl::operate(3, orphan, 0), Err(fl::Error::Terminal));
    zero();
    note("creator-exit-incomplete", 0);
    // Each actual syscall pair overlaps on different CPUs and proves a failed
    // try_lock while the selected winner owns the lifecycle lock. Hooks only
    // coordinate arrival/order; the ordinary authority methods decide results.
    for mode in 1..=4 {
        let mut covered = false;
        // Natural lock contention is observed, never prolonged by a peer wait
        // under SCHEDULER. Fresh attempts are bounded; misses are not coverage.
        for _attempt in 0..16 {
            let actor = spawn(ep, public);
            let raced = pair(actor);
            assert_eq!(fl::operate(7, raced, mode), Ok(0));
            if mode <= 2 {
                expect(actor, RACE_ATTACH, raced, 0, 0);
                assert_eq!(fl::operate(4, raced, 0), Ok(0));
                let done = sunlight_ipc::ipc_recv_timeout(ep, 5000).unwrap();
                assert_eq!(done.label, RACE_DONE);
                assert_eq!(done.badge, actor.pid);
                assert_eq!(done.words[0], if mode == 1 { 0 } else { 4 });
                assert_eq!(done.words[1], raced.generation);
                sunlight_ipc::ipc_reply_result(IpcMsg::with_label(RACE_DONE)).unwrap();
                stop(actor);
            } else {
                expect(actor, RACE_EXIT, raced, 0, 0);
                assert_eq!(
                    fl::operate(3, raced, 0),
                    if mode == 3 {
                        Ok(0)
                    } else {
                        Err(fl::Error::Terminal)
                    }
                );
                assert_eq!(super::reap(actor.pid), 0);
            }
            assert_eq!(fl::operate(3, raced, 0), Err(fl::Error::Terminal));
            zero();
            if fl::operate(23, raced, 0).expect("exact creator race receipt") == 1 {
                expect(foreign, COORD_RECEIPT, raced, 0, 1);
                assert_eq!(
                    fl::operate(
                        23,
                        fl::Handle {
                            token: raced.token ^ 1,
                            ..raced
                        },
                        0
                    ),
                    Err(fl::Error::Denied)
                );
                covered = true;
                break;
            }
            note("contention-missed", mode);
        }
        assert!(covered, "M1 contention coverage exhausted");
        note("contended-race", mode);
    }
    // Eight real Account-authorized independent producers exhaust metadata.
    let mut actors = alloc::vec::Vec::new();
    for _ in 0..8 {
        let a = spawn(ep, public);
        expect(a, AUTHORIZE, fl::EMPTY, 60000, 0);
        actors.push(a);
    }
    assert_eq!(authorize(0, fl::EMPTY, 30000), Err(5));
    note("metadata-exhaustion", 0);
    for a in actors {
        stop(a);
    }
    zero();
    let short = authorize(0, fl::EMPTY, 5).unwrap();
    let start = sunlight_ipc::monotonic_millis();
    loop {
        if fl::operate(5, short, 0).unwrap() & 255 == 4 {
            break;
        }
        assert!(sunlight_ipc::monotonic_millis() - start < 5000);
        sunlight_ipc::process_yield();
    }
    assert_eq!(fl::operate(2, short, 0), Err(fl::Error::Terminal));
    zero();
    note("abandoned-expiry", 0);
    stop(foreign);
    assert!(sunlight_ipc::endpoint_destroy(ep));
    zero();
    note("complete", 0);
    sunlight_libc::exit(0)
}
