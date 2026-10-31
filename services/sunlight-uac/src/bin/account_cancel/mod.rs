//! Opt-in authenticated cancellation/caller-exit fixture. All coordination is
//! ordinary private IPC; no kernel authority or lifecycle decision uses it.
#![allow(dead_code)] // This module is shared by the daemon and probe executable.
use core::sync::atomic::{AtomicU64, Ordering};
use sunlight_ipc::{accounts as api, CapabilityToken, EndpointId, IpcError, IpcMsg};
const HELLO: u64 = 0xEC01;
const SETUP: u64 = 0xEC02;
const PUBLISHED: u64 = 0xEC03;
const RESULT: u64 = 0xEC04;
const FRESH: u64 = 0xEC05;
const DONE: u64 = 0xEC06;

fn require_race_image(_mode: u64) {
    #[cfg(not(feature = "session-account-completion-exit-test"))]
    assert!(_mode < 11, "exit race requires its separate test image");
    #[cfg(not(feature = "session-account-completion-race-test"))]
    assert!(
        !(8..11).contains(&_mode),
        "concurrent mode requires its separate test image"
    );
}

fn note(mode: u64, event: &str, child: u64) {
    sunlight_ipc::debug_log(&alloc::format!(
        "[CE-USER] mode={} event={} pid={} child={}",
        mode,
        event,
        sunlight_libc::getpid(),
        child
    ));
}
fn observe(event: &str) {
    sunlight_ipc::debug_log(&alloc::format!("[CE-OBSERVE] event={}", event));
}
fn wait_child(pid: u64) -> u64 {
    let start = sunlight_ipc::monotonic_millis();
    for _ in 0..1_000_000 {
        if let Some(code) = sunlight_libc::try_waitpid(pid).expect("spawned child status") {
            return code;
        }
        assert!(
            sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000,
            "child deadline"
        );
        sunlight_ipc::process_yield();
    }
    panic!("child wait exhausted")
}
fn recv(ep: EndpointId, label: u64, pid: u64) -> IpcMsg {
    let m = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("fixture receive");
    assert_eq!(m.label, label);
    assert_eq!(m.badge, pid);
    m
}
fn wait_caller_reaped() {
    // Test-image read-only query of the selected PID + incarnation and parent.
    // The independent serial assessor still requires actual reaper, unmap,
    // shootdown and PMM events; this query supplies no lifecycle authority.
    let query = "[CE-QUERY] caller-reaped";
    let start = sunlight_ipc::monotonic_millis();
    loop {
        let ready =
            unsafe { sunlight_libc::sys::syscall2(99, query.as_ptr() as u64, query.len() as u64) };
        assert!(ready <= 1);
        if ready == 1 {
            return;
        }
        assert!(sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000);
        sunlight_ipc::process_yield();
    }
}
fn ack(label: u64) {
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(label)).expect("fixture ack");
}
fn call(ep: CapabilityToken, label: u64) {
    assert_eq!(
        sunlight_ipc::ipc_call_timeout(ep, IpcMsg::with_label(label), 5000)
            .expect("fixture call")
            .label,
        label
    );
}
fn stale(old: u64) {
    assert!(!sunlight_ipc::ipc_deferred_reply_is_live(old));
    for _ in 0..2 {
        assert_eq!(
            sunlight_ipc::ipc_complete_deferred_reply(old, IpcMsg::with_label(api::REPLY)),
            Err(IpcError::InvalidArgument)
        );
    }
}

pub fn account(
    ep: EndpointId,
    grant: CapabilityToken,
    scope: api::GrantScope,
    msg: &IpcMsg,
    pointer: *mut u8,
) {
    let control = msg.caps[1];
    let caller = msg.badge;
    let request = msg.caps[0];
    // The Account handler already mapped and validated this complete page.
    let requested_mode = unsafe { core::ptr::read_unaligned(pointer.add(264).cast::<u64>()) };
    sunlight_ipc::debug_log("[AE-OBSERVE] event=committed");
    observe("committed");
    let old = if requested_mode == 1 {
        0
    } else {
        sunlight_ipc::ipc_defer_reply().expect("original authenticated deferred completion")
    };
    observe("deferred");
    let phase = unsafe { &*pointer.add(272).cast::<AtomicU64>() };
    phase.store(1, Ordering::Release);
    let setup = sunlight_ipc::ipc_call_timeout(
        control,
        IpcMsg::with_label(SETUP)
            .word(0, caller)
            .word(1, scope.session)
            .word(2, scope.session_generation)
            .word(3, old)
            .with_cap(0, grant),
        5000,
    )
    .expect("postcommit supervisor");
    assert_eq!(setup.label, SETUP);
    let mode = setup.words[0];
    assert_eq!(mode, requested_mode);
    if mode >= 8 {
        require_race_image(mode);
        let query = alloc::format!(
            "[{}-ARM] mode={}",
            if mode >= 11 { "EX" } else { "RC" },
            mode
        );
        assert_eq!(
            unsafe { sunlight_libc::sys::syscall2(99, query.as_ptr() as u64, query.len() as u64) },
            0
        );
        phase.store(4, Ordering::Release);
        let result = sunlight_ipc::ipc_complete_deferred_reply(
            old,
            IpcMsg::with_label(api::REPLY).with_cap(0, grant),
        );
        match mode {
            8 | 11 => assert_eq!(result, Ok(())),
            9 | 12 => assert_eq!(result, Err(IpcError::InvalidArgument)),
            10 | 13 => assert!(matches!(result, Ok(()) | Err(IpcError::InvalidArgument))),
            _ => unreachable!(),
        }
        phase.store(if result.is_ok() { 2 } else { 3 }, Ordering::Release);
        note(
            mode,
            if result.is_ok() {
                "completion-won"
            } else {
                if mode >= 11 {
                    "exit-won"
                } else {
                    "cancellation-won"
                }
            },
            caller,
        );
        call(control, PUBLISHED);
    } else if matches!(mode, 3 | 5) {
        sunlight_ipc::ipc_complete_deferred_reply(
            old,
            IpcMsg::with_label(api::REPLY).with_cap(0, grant),
        )
        .expect("commit original reply");
        observe("published");
        phase.store(2, Ordering::Release);
        call(control, PUBLISHED); // Hold request view until the original caller exits.
    } else if matches!(mode, 6 | 7) {
        assert!(sunlight_ipc::endpoint_destroy(ep));
        observe("endpoint-closed");
    }
    if mode == 1 {
        sunlight_ipc::ipc_reply_result(IpcMsg::with_label(api::REPLY).with_cap(0, grant))
            .expect("late ordinary reply safely dropped");
    }
    stale(old);
    if mode >= 8 {
        note(mode, "late-completions-rejected", caller);
    }
    assert_eq!(
        api::try_broker_grant(caller, scope, Some(grant)),
        Err(api::Error::Denied)
    );
    assert_eq!(
        api::try_broker_grant(caller, scope, None),
        Err(api::Error::Denied)
    );
    observe("late-denied");
    call(control, DONE);
    if !matches!(mode, 6 | 7) {
        // Exact original completion must remain invalid while a different
        // process has a fresh live call through the same Account endpoint.
        let fresh = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("fresh foreign caller");
        assert_eq!(fresh.label, FRESH);
        observe("fresh");
        stale(old);
        assert_eq!(
            api::try_broker_grant(fresh.badge, scope, Some(grant)),
            Err(api::Error::CapabilityRejected)
        );
        note(mode, "foreign-adoption-denied", caller);
        sunlight_ipc::shm_free(request).expect("Account last request view release");
        observe("account-unmapped");
        assert!(sunlight_ipc::shm_map(request).is_err());
        assert!(sunlight_ipc::shm_free(request).is_err());
        ack(FRESH);
    } else {
        sunlight_ipc::shm_free(request).expect("closed endpoint request release");
        observe("account-unmapped");
    }
    note(mode, "account-complete", caller);
    // Endpoint-closure probes use the ordinary service restart to restore it.
    if matches!(mode, 6 | 7) {
        sunlight_libc::exit(0);
    }
}

pub fn supervisor(mode: &str, fixture: &str) -> ! {
    assert_eq!(sunlight_libc::getuid(), 0);
    let mode = mode.parse::<u64>().expect("mode");
    assert!((1..=13).contains(&mode));
    require_race_image(mode);
    let ep = sunlight_ipc::endpoint_create();
    let send = sunlight_ipc::endpoint_bind(ep.0);
    let handle = alloc::format!("{}", send.0);
    let ordinal = alloc::format!("{}", mode);
    let child = sunlight_libc::spawn(
        b"/bin/session-racer",
        &[
            b"session-racer",
            b"cancel-caller",
            handle.as_bytes(),
            ordinal.as_bytes(),
        ],
        None,
    )
    .expect("spawn original authenticated requester");
    let hello = recv(ep, HELLO, child);
    assert_eq!(hello.cap_count, 2);
    let caller_endpoint = hello.caps[1];
    let account = hello.caps[0];
    ack(HELLO);
    sunlight_ipc::debug_log(&alloc::format!(
        "[SESSION-E3-CLIENT] case=N6 checkpoint=request-start pid={} uid=0 result=0 fixture={}",
        sunlight_libc::getpid(),
        fixture
    ));
    let setup = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("real committed Account operation");
    assert_eq!(setup.label, SETUP);
    assert_eq!(setup.words[0], child);
    assert_eq!(setup.cap_count, 1);
    let issuer = setup.badge;
    // A real completion handle in another process is never completion authority.
    stale(setup.words[3]);
    let deferred = sunlight_ipc::ipc_defer_reply().expect("hold Account postcommit rendezvous");
    assert!(!sunlight_ipc::endpoint_destroy(EndpointId(account.0)));
    assert!(sunlight_ipc::ipc_recv_timeout(EndpointId(account.0), 1).is_none());
    let scope = api::GrantScope {
        session: setup.words[1],
        session_generation: setup.words[2],
        revision: 0,
        operation: api::Operation::GroupCreate,
        target: 61006,
        member: 0,
    };
    assert_eq!(
        api::try_broker_grant(child, scope, Some(setup.caps[0])),
        Err(api::Error::Denied)
    );
    note(mode, "foreign-live-authority-denied", child);
    if mode == 4 {
        // This PID is the child returned by spawn and matched to the kernel
        // badge at HELLO and Account's exact original request. No name scan.
        sunlight_libc::kill(child, 15).expect("authorized original-child SIGTERM");
        assert_eq!(wait_child(child), 143);
        wait_caller_reaped();
        observe("caller-dead");
        note(mode, "caller-reaped-before-completion", child);
    } else if matches!(mode, 1 | 2 | 6) {
        recv(ep, RESULT, child);
        observe("cancel-observed");
        ack(RESULT);
        assert_eq!(wait_child(child), 0);
        wait_caller_reaped();
        note(mode, "caller-reaped-before-completion", child);
    }
    sunlight_ipc::ipc_complete_deferred_reply(deferred, IpcMsg::with_label(SETUP).word(0, mode))
        .expect("release Account postcommit");
    if matches!(mode, 3 | 5 | 8..=13) {
        recv(ep, PUBLISHED, issuer);
        let published = sunlight_ipc::ipc_defer_reply().expect("hold committed Account view");
        if mode == 3 || (8..=10).contains(&mode) {
            let result = recv(ep, RESULT, child);
            if mode >= 8 {
                assert!(matches!(result.words[0], 0 | 9));
                if mode == 8 {
                    assert_eq!(result.words[0], 0);
                }
                if mode == 9 {
                    assert_eq!(result.words[0], 9);
                }
                if result.words[0] == 0 {
                    assert_eq!(result.cap_count, 1);
                    assert_eq!(result.caps[0], setup.caps[0]);
                } else {
                    assert_eq!(result.cap_count, 0);
                }
                note(mode, "caller-result-matched", child);
            }
            ack(RESULT);
        }
        assert_eq!(wait_child(child), 0);
        wait_caller_reaped();
        observe("caller-dead");
        note(mode, "caller-reaped-after-completion", child);
        sunlight_ipc::ipc_complete_deferred_reply(published, IpcMsg::with_label(PUBLISHED))
            .expect("release completed Account");
    }
    recv(ep, DONE, issuer);
    ack(DONE);
    if mode == 7 {
        recv(ep, RESULT, child);
        ack(RESULT);
        assert_eq!(wait_child(child), 0);
    } else if mode != 6 {
        call(account, FRESH);
    }
    assert!(matches!(
        sunlight_ipc::ipc_call_timeout(caller_endpoint, IpcMsg::with_label(FRESH), 100),
        Err(sunlight_ipc::IpcCallError::InvalidCapability
            | sunlight_ipc::IpcCallError::EndpointNotFound)
    ));
    note(mode, "caller-endpoint-retired", child);
    let manager = sunlight_libc::spawn(
        b"/bin/sunlightctl",
        &[b"sunlightctl", b"restart", b"uac_service"],
        None,
    )
    .expect("ordinary authorized Account restart");
    assert_eq!(wait_child(manager), 0);
    let session =
        sunlight_ipc::nameserver_lookup(sunlight_ipc::SESSION_ENDPOINT).expect("session endpoint");
    let logout = sunlight_ipc::ipc_call_timeout(
        session,
        IpcMsg::with_label(sunlight_ipc::SessionMsg::SESSION_ACTION)
            .word(0, setup.words[1])
            .word(1, setup.words[2])
            .word(2, sunlight_ipc::SessionAction::Logout as u64),
        5000,
    )
    .expect("ordinary session logout");
    assert_eq!(logout.label, sunlight_ipc::SessionMsg::REPLY);
    assert!(sunlight_ipc::endpoint_destroy(ep));
    observe("supervisor-clean");
    note(mode, "fixture-complete", child);
    sunlight_ipc::debug_log(&alloc::format!(
        "[SESSION-E3-CLIENT] case=N6 checkpoint=fixture-complete pid={} uid=0 result=0 fixture={}",
        sunlight_libc::getpid(),
        fixture
    ));
    sunlight_libc::exit(0)
}

// Exact existing native register IPC ABI, kept local to this test fixture so
// the first syscall return can be observed before its terminal is collected.
fn submit(cap: CapabilityToken, msg: IpcMsg) -> (u64, IpcMsg) {
    let (ret, label, badge, counts, w0, w1, w2, w3, c0, c1): (
        u64,
        u64,
        u64,
        u64,
        u64,
        u64,
        u64,
        u64,
        u64,
        u64,
    );
    unsafe {
        core::arch::asm!("syscall", inlateout("rax") 1u64 => ret,
            inlateout("rdi") msg.label => label, inlateout("rsi") cap.0 => badge,
            inlateout("rdx") u64::from(msg.word_count) | (u64::from(msg.cap_count)<<32) => counts,
            inlateout("r8") msg.words[0] => w0, inlateout("r9") msg.words[1] => w1,
            inlateout("r10") msg.words[2] => w2, inlateout("r12") msg.words[3] => w3,
            inlateout("r13") msg.caps[0].0 => c0, inlateout("r14") msg.caps[1].0 => c1,
            lateout("rcx") _, lateout("r11") _);
    }
    let mut r = IpcMsg::with_label(label)
        .word(0, w0)
        .word(1, w1)
        .word(2, w2)
        .word(3, w3);
    r.badge = badge;
    r.word_count = counts as u32;
    r.cap_count = (counts >> 32) as u32;
    r.caps = [CapabilityToken(c0), CapabilityToken(c1)];
    (ret, r)
}

pub fn caller(args: &[&str]) -> ! {
    assert_eq!(args.len(), 4);
    let control = CapabilityToken(args[2].parse().expect("private supervisor endpoint"));
    let mode = args[3].parse::<u64>().expect("caller mode");
    let snapshot = api::snapshot().expect("authenticated caller session");
    let account = sunlight_ipc::nameserver_lookup("uac").expect("Account endpoint");
    let owned = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(owned.0);
    sunlight_ipc::ipc_call_timeout(
        control,
        IpcMsg::with_label(HELLO)
            .with_cap(0, account)
            .with_cap(1, public),
        5000,
    )
    .expect("caller bootstrap");
    let (ptr, shm) = sunlight_ipc::shm_create(api::SHM_BYTES, 0).expect("original request SHM");
    unsafe {
        core::ptr::copy_nonoverlapping(b"root".as_ptr(), ptr, 4);
        core::ptr::write_unaligned(ptr.add(256).cast::<u64>(), snapshot.revision);
    }
    unsafe {
        core::ptr::write_unaligned(ptr.add(264).cast::<u64>(), mode);
    }
    let worker = super::account_exec::start_worker(ptr);
    let msg = IpcMsg::with_label(api::AUTHORIZE)
        .word(0, snapshot.session_id)
        .word(1, snapshot.generation)
        .word(2, api::Operation::GroupCreate as u64)
        .word(3, 61006)
        .with_cap(0, shm)
        .with_cap(1, control);
    let deadline = sunlight_ipc::monotonic_millis()
        + if matches!(mode, 1 | 2 | 6) {
            1500
        } else {
            10000
        };
    assert_eq!(unsafe { sunlight_libc::sys::syscall1(8, deadline) }, 0);
    let (mut code, mut reply) = submit(account, msg);
    assert_eq!(code, IpcError::WouldBlock as u64);
    observe("first-return");
    if mode >= 8 {
        require_race_image(mode);
        let phase = unsafe { &*ptr.add(272).cast::<AtomicU64>() };
        let start = sunlight_ipc::monotonic_millis();
        while phase.load(Ordering::Acquire) != 4 {
            assert!(sunlight_ipc::monotonic_millis().saturating_sub(start) < 5000);
            sunlight_ipc::process_yield();
        }
        if mode >= 11 {
            note(mode, "calling-real-exit", 0);
            // No cancel, signal, explicit unmap or borrower stop: the original
            // caller executes its ordinary exit syscall with its view intact.
            sunlight_libc::exit(0);
        }
        // The debug-image arm only wakes this exact pending caller. The
        // cancellation syscall retains its production self-only authority.
        assert_eq!(unsafe { sunlight_libc::sys::syscall0(7) }, 0);
        let start = sunlight_ipc::monotonic_millis();
        while !matches!(phase.load(Ordering::Acquire), 2 | 3) {
            assert!(sunlight_ipc::monotonic_millis().saturating_sub(start) < 5000);
            sunlight_ipc::process_yield();
        }
        observe("race-late-cancel-before");
        assert_eq!(unsafe { sunlight_libc::sys::syscall0(7) }, 0);
        observe("race-late-cancel-after");
    }
    if matches!(mode, 3 | 5) {
        let phase = unsafe { &*ptr.add(272).cast::<AtomicU64>() };
        let expected = if mode == 2 { 1 } else { 2 };
        let start = sunlight_ipc::monotonic_millis();
        while phase.load(Ordering::Acquire) < expected {
            assert!(sunlight_ipc::monotonic_millis().saturating_sub(start) < 5000);
            sunlight_ipc::process_yield();
        }
    }
    if mode == 5 {
        // Exit after commitment without collecting its delivered bearer grant.
        observe("exit-with-reply");
        sunlight_libc::exit(0);
    }
    if mode == 3 {
        observe("cancel-after-commit-before-collect");
        assert_eq!(unsafe { sunlight_libc::sys::syscall0(7) }, 0);
        observe("cancel-preserved-reply");
    }
    let start = sunlight_ipc::monotonic_millis();
    while code == IpcError::WouldBlock as u64 {
        (code, reply) = submit(account, msg);
        assert!(sunlight_ipc::monotonic_millis().saturating_sub(start) < 12000);
        if code == IpcError::WouldBlock as u64 {
            sunlight_ipc::process_yield();
        }
    }
    assert_eq!(
        code,
        if mode == 3
            || mode >= 8
                && unsafe { &*ptr.add(272).cast::<AtomicU64>() }.load(Ordering::Acquire) == 2
        {
            0
        } else if mode >= 8 {
            IpcError::Cancelled as u64
        } else if mode == 7 {
            IpcError::PeerClosed as u64
        } else {
            IpcError::DeadlineExpired as u64
        }
    );
    if mode == 3 || mode >= 8 && code == 0 {
        assert_eq!(reply.label, api::REPLY);
        assert_eq!(reply.cap_count, 1);
    }
    observe("caller-consumed");
    super::account_exec::prepare_release();
    sunlight_ipc::shm_free(shm).expect("caller explicit view release");
    super::account_exec::finish_worker(worker);
    observe("caller-unmapped");
    note(mode, "result-consumed", 0);
    if mode >= 8 {
        let mut result = IpcMsg::with_label(RESULT).word(0, code);
        if code == 0 {
            result = result.with_cap(0, reply.caps[0]);
        }
        assert_eq!(
            sunlight_ipc::ipc_call_timeout(control, result, 5000)
                .expect("independent caller result acknowledgement")
                .label,
            RESULT
        );
    } else {
        call(control, RESULT);
    }
    sunlight_libc::exit(0)
}
