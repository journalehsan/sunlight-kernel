//! R2 authenticated ring request / IPC reply lifecycle fixture. The kernel
//! contender is explicitly a method driver, not a second syscall owner.
use super::*;
pub mod caller_exec;
pub(super) const EXECUTE_R2: u64 = 0x6e05;

pub(super) fn execute(endpoint: &mut Endpoint, m: IpcMsg) {
    let h = endpoint.handles.unwrap();
    assert_eq!(m.word_count, 4);
    assert_eq!(m.cap_count, 0);
    assert_eq!(&m.words[..3], &[h.token, h.generation, 0x10010]);
    assert_eq!(m.words[3] as u32, 1);
    let mode = m.words[3] >> 32;
    assert_eq!(fl::operate(35, h, 0x10010), Ok(2));
    let token = sunlight_ipc::ipc_defer_reply().unwrap();
    let request = receive(endpoint.request.as_mut().unwrap(), h);
    assert_eq!(request.request_id(), 1);
    let mut operation = Operation::committed(Transport::Ring);
    let response = operation.execute(request).unwrap();
    if mode == 15 {
        caller_exec::retain(endpoint, h, token, operation, response);
        return;
    }
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-R2] event=executed mode={} channel={} request_id=1",
        mode,
        h.generation
    ));
    if mode == 7 {
        sunlight_libc::exit(0);
    }
    if mode == 8 {
        let token = alloc::format!("{}", token);
        let handle = alloc::format!("{}", h.token);
        let generation = alloc::format!("{}", h.generation);
        sunlight_libc::exec(
            b"/bin/session-racer",
            &[
                b"session-racer",
                b"FL-R2-replaced",
                handle.as_bytes(),
                generation.as_bytes(),
                token.as_bytes(),
            ],
        )
        .unwrap();
        panic!("exec returned");
    }
    assert_eq!(fl::operate(37, h, mode), Ok(0));
    let result = if mode >= 13 {
        assert_eq!(fl::operate(40, h, token), Ok(0));
        sunlight_ipc::ipc_complete_deferred_reply(token, response.ipc_message())
    } else {
        sunlight_ipc::ipc_complete_deferred_reply(token, response.ipc_message())
    };
    assert_eq!(result.is_ok(), mode < 13 && mode % 2 == 1);
    assert!(!sunlight_ipc::ipc_deferred_reply_is_live(token));
    assert!(sunlight_ipc::ipc_complete_deferred_reply(token, response.ipc_message()).is_err());
    endpoint.last_reply = Some(token);
    endpoint.operation = Some(operation);
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-R2] event=actor-return mode={} channel={} duplicate=rejected accepted={}",
        mode,
        h.generation,
        u8::from(result.is_ok())
    ));
}

pub fn supervisor() -> ! {
    doorbell::pin(0);
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let parent = alloc::format!("{}", public.0);
    for mode in [1, 2, 3, 4, 5, 6, 7, 8, 11, 12, 13, 14] {
        round(mode, ep, public, None);
    }
    for mode in [9, 10] {
        let mode_text = alloc::format!("{}", mode);
        let child = sunlight_libc::spawn(
            b"/tmp/session-race-probe",
            &[
                b"session-race-probe",
                b"FL-R2-caller",
                parent.as_bytes(),
                mode_text.as_bytes(),
            ],
            None,
        )
        .unwrap();
        let intro = sunlight_ipc::ipc_recv_timeout(ep, 10000).unwrap();
        assert_eq!(intro.label, 0x6e10);
        assert_eq!(intro.badge, child);
        assert_eq!(intro.cap_count, 1);
        let h = fl::Handle {
            token: intro.words[0],
            generation: intro.words[1],
        };
        let actor = Actor {
            pid: intro.words[2],
            send: intro.caps[0],
        };
        sunlight_ipc::ipc_reply_result(IpcMsg::with_label(0x6e10)).unwrap();
        let start = sunlight_ipc::monotonic_millis();
        loop {
            if let Some(status) = sunlight_libc::try_waitpid(child).unwrap() {
                assert_eq!(status, 0);
                break;
            }
            assert!(sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000);
            assert!(sunlight_ipc::ipc_recv_timeout(ep, 10).is_none());
        }
        expect(actor, EXIT, fl::EMPTY, 0, 0);
        mapping::census(700 + mode);
        zero();
        await_retirement(h);
        sunlight_ipc::debug_log(&alloc::format!(
            "[FL-R2] event=caller-reaped mode={} channel={} caller={} count=1",
            mode,
            h.generation,
            child
        ));
    }
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-R2] event=complete pid={} modes=14 scope=authenticated-ring3/kernel-method-retirement",
        sunlight_libc::getpid()
    ));
    sunlight_libc::exit(0)
}

pub fn replaced(args: &[&str]) -> ! {
    let h = fl::Handle {
        token: args[2].parse().unwrap(),
        generation: args[3].parse().unwrap(),
    };
    let token = args[4].parse().unwrap();
    assert!(sunlight_ipc::ipc_complete_deferred_reply(token, IpcMsg::empty()).is_err());
    assert!(!sunlight_ipc::ipc_deferred_reply_is_live(token));
    assert!(fl::operate(3, h, 1).is_err());
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-R2] event=exec-replacement channel={} late=rejected reattach=rejected",
        h.generation
    ));
    sunlight_libc::exit(0)
}

fn round(
    mode: u64,
    ep: sunlight_ipc::EndpointId,
    public: CapabilityToken,
    supervisor: Option<CapabilityToken>,
) {
    let parent = alloc::format!("{}", public.0);
    let pid = sunlight_libc::spawn(
        b"/test/contract-required",
        &[b"session-racer", b"FL-actor", parent.as_bytes(), b"1"],
        None,
    )
    .unwrap();
    let hello = sunlight_ipc::ipc_recv_timeout(ep, 5000).unwrap();
    assert_eq!(hello.label, HELLO);
    assert_eq!(hello.badge, pid);
    let actor = Actor {
        pid,
        send: hello.caps[0],
    };
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(HELLO)).unwrap();
    // Preserve the original invalid-direction control; cancellation before
    // establishment is additional coverage rather than its replacement.
    if mode == 1 {
        for invalid in [true, false] {
            let h = offer();
            let consent = expect(actor, AUTHORIZE, h, 1, 0);
            if invalid {
                assert_eq!(fl::operate(31, h, 0x20202), Err(fl::Error::Denied));
            } else {
                assert_eq!(fl::operate(31, h, 0x10202), Ok(2));
                assert_eq!(fl::operate(33, h, 0), Ok(0));
            }
            assert!(fl::operate(2, h, consent.words[1]).is_err());
            sunlight_ipc::debug_log(&alloc::format!(
                "[FL-R2] event=negative control={} channel={} terminal=rejected count=1",
                if invalid {
                    "invalid-directional-contract"
                } else {
                    "cancellation-before-establishment-commit"
                },
                h.generation
            ));
        }
    }
    let h = offer();
    let consent = expect(actor, AUTHORIZE, h, 1, 0);
    assert_eq!(fl::operate(31, h, 0x10202), Ok(2));
    fl::operate(2, h, consent.words[1]).unwrap();
    fl::operate(3, h, 0).unwrap();
    expect(actor, 3, h, 1, 0);
    let native = memory(h, 0);
    let consumer_page = native.pages[1];
    let mut producer = ring::Producer::new(native, h.generation).unwrap();
    let setup = IpcMsg::with_label(SETUP)
        .word(0, h.token)
        .word(1, h.generation);
    assert_eq!(
        sunlight_ipc::ipc_call_timeout(actor.send, setup, 5000)
            .unwrap()
            .label,
        SETUP
    );
    assert_eq!(fl::operate(32, h, 0), Ok(2));
    assert_eq!(fl::operate(39, h, 0), Err(fl::Error::Stale));
    sunlight_ipc::debug_log(&alloc::format!("[FL-R2] event=negative control=stale-policy-generation channel={} terminal=rejected count=1",h.generation));
    if let Some(supervisor) = supervisor {
        let intro = IpcMsg::with_label(0x6e10)
            .word(0, h.token)
            .word(1, h.generation)
            .word(2, actor.pid)
            .with_cap(0, actor.send);
        sunlight_ipc::ipc_call_timeout(supervisor, intro, 10000).unwrap();
    }
    if mode != 1 && mode != 15 {
        assert_eq!(fl::operate(39, h, 1), Err(fl::Error::Stale));
        sunlight_ipc::debug_log(&alloc::format!("[FL-R2] event=negative control=reused-policy-slot-stale-reference channel={} terminal=rejected count=1",h.generation));
    }
    assert_eq!(
        fl::operate(
            5,
            fl::Handle {
                generation: h.generation - 1,
                ..h
            },
            0
        ),
        Err(fl::Error::Stale)
    );
    sunlight_ipc::debug_log(&alloc::format!("[FL-R2] event=negative control=stale-channel-generation channel={} terminal=rejected count=1",h.generation));
    if mode == 1 {
        let foreign = spawn(ep, public);
        let forwarded = IpcMsg::with_label(FORWARD)
            .word(0, h.token)
            .word(1, h.generation)
            .with_cap(0, actor.send);
        let denied = sunlight_ipc::ipc_call_timeout(foreign.send, forwarded, 5000).unwrap();
        assert_eq!(denied.words[0], 1);
        sunlight_ipc::debug_log(&alloc::format!(
            "[FL-R2] event=negative control=foreign-caller channel={} foreign={} original={} terminal=rejected count=1",
            h.generation, foreign.pid, sunlight_libc::getpid()
        ));
        stop(foreign);
    }
    let request = Frame::request(1, b"0123456789abcdef").unwrap();
    producer.try_send(&request.ring_bytes()).unwrap();
    fl::operate(10, h, 0).unwrap();
    let control = IpcMsg::with_label(EXECUTE_R2)
        .word(0, h.token)
        .word(1, h.generation)
        .word(2, 0x10010)
        .word(3, 1 | (mode << 32));
    if mode == 15 {
        caller_exec::enter(actor, h, control, supervisor.unwrap(), consumer_page);
    }
    let response = sunlight_ipc::ipc_call_timeout(actor.send, control, 10000);
    assert!(
        !matches!(mode, 9 | 10),
        "exiting caller unexpectedly resumed"
    );
    if (mode <= 6 || mode == 11) && mode % 2 == 1 {
        assert_eq!(
            Frame::from_ipc_message(&response.unwrap()),
            Ok(request.transform().unwrap())
        );
    } else {
        assert_eq!(
            response.unwrap_err(),
            if mode == 2 {
                sunlight_ipc::IpcCallError::Cancelled
            } else {
                sunlight_ipc::IpcCallError::PeerClosed
            }
        );
    }
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-R2] event=caller-terminal mode={} channel={} terminal={} count=1",
        mode,
        h.generation,
        if (mode <= 6 || mode == 11) && mode % 2 == 1 {
            "ReplyDelivered"
        } else if mode == 2 {
            "ExplicitlyCancelled"
        } else {
            "PeerClosed"
        }
    ));
    if mode == 1 {
        let probe =
            sunlight_ipc::ipc_call_timeout(actor.send, IpcMsg::with_label(REPLY_PROBE), 5000)
                .unwrap();
        assert_eq!(probe.label, REPLY_PROBE);
        assert!(
            sunlight_ipc::ipc_complete_deferred_reply(probe.words[0], IpcMsg::empty()).is_err()
        );
        sunlight_ipc::debug_log(&alloc::format!(
            "[FL-R2] event=negative control=foreign-reply-adoption channel={} terminal=rejected count=1",
            h.generation
        ));
    }
    fl::operate(4, h, 0).unwrap();
    if mode <= 6 || mode >= 13 {
        stop(actor);
    } else {
        assert_eq!(crate::reap(actor.pid), 0);
    }
    mapping::census(700 + mode);
    zero();
    await_retirement(h);
    assert!(fl::operate(3, h, 0).is_err());
}

fn await_retirement(handle: fl::Handle) {
    let start = sunlight_ipc::monotonic_millis();
    for _ in 0..100_000 {
        if fl::operate(38, handle, 0).unwrap() == 0 {
            return;
        }
        assert!(sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000);
        sunlight_ipc::process_yield();
    }
    panic!("hybrid resource retirement exhausted");
}

pub fn caller(args: &[&str]) -> ! {
    doorbell::pin(0);
    let supervisor = CapabilityToken(args[2].parse().unwrap());
    let mode = args[3].parse().unwrap();
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    round(mode, ep, public, Some(supervisor));
    panic!("exiting caller returned");
}
