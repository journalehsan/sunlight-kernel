//! Ordinary client launched by an authenticated shell. No trusted roles or
//! direct broker access. The controller uses existing session/SunlightD IPC.
#![no_std]
#![no_main]
extern crate alloc;
#[cfg(feature = "session-account-cancel-test")]
mod account_cancel;
#[cfg(feature = "session-reply-test")]
mod authenticated_reply;
#[cfg(any(feature = "session-account-exec-test", feature = "session-account-cancel-test"))]
mod account_exec;
use sunlight_ipc::{accounts as api, CapabilityToken, IpcMsg};

#[cfg(feature = "session-h4-test")]
static FIXTURE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

#[global_allocator]
static ALLOCATOR: linked_list_allocator::LockedHeap = linked_list_allocator::LockedHeap::empty();
static mut HEAP: [u8; 256 * 1024] = [0; 256 * 1024];

fn endpoint(name: &str) -> CapabilityToken {
    // Preserve the original lookup, timeout and failure behavior, but retain
    // the transport/non-GRANT distinction previously collapsed into None.
    let start = sunlight_ipc::monotonic_millis();
    let reply = sunlight_ipc::ipc_call_timeout(
        sunlight_ipc::get_init_cap(),
        IpcMsg::with_label(sunlight_ipc::InitMsg::LOOKUP).word(0, sunlight_ipc::name_to_u64(name)),
        2000,
    );
    let elapsed = sunlight_ipc::monotonic_millis().saturating_sub(start);
    match reply {
        Ok(reply) if reply.label == sunlight_ipc::InitMsg::GRANT => {
            sunlight_ipc::debug_log(&alloc::format!(
                "[SESSION-E3-LOOKUP] name={} outcome=grant elapsed_ms={}",
                name,
                elapsed
            ));
            CapabilityToken(reply.words[0])
        }
        Ok(reply) => {
            sunlight_ipc::debug_log(&alloc::format!(
                "[SESSION-E3-LOOKUP] name={} outcome=non-grant label={} status={} elapsed_ms={}",
                name,
                reply.label,
                reply.words[0],
                elapsed
            ));
            panic!("endpoint unavailable")
        }
        Err(error) => {
            sunlight_ipc::debug_log(&alloc::format!(
                "[SESSION-E3-LOOKUP] name={} outcome=transport error={:?} elapsed_ms={}",
                name,
                error,
                elapsed
            ));
            panic!("endpoint unavailable")
        }
    }
}

fn n4_query() -> u64 {
    let query = "[SESSION-E3-N4] checkpoint-query";
    unsafe {
        sunlight_libc::sys::syscall2(
            sunlight_libc::sys::SYS_DEBUG_LOG,
            query.as_ptr() as u64,
            query.len() as u64,
        )
    }
}
fn note(case: &str, checkpoint: &str, result: u64) {
    use core::fmt::Write;
    let mut text = heapless::String::<200>::new();
    let _ = write!(
        text,
        "[SESSION-E3-CLIENT] case={} checkpoint={} pid={} uid={} result={}",
        case,
        checkpoint,
        sunlight_libc::getpid(),
        sunlight_libc::getuid(),
        result
    );
    #[cfg(feature = "session-h4-test")]
    let _ = write!(
        text,
        " fixture={}",
        FIXTURE.load(core::sync::atomic::Ordering::Relaxed)
    );
    sunlight_ipc::debug_log(&text);
}
fn logout(session: u64, epoch: u64) {
    logout_on(endpoint(sunlight_ipc::SESSION_ENDPOINT), session, epoch);
}
fn logout_on(endpoint: CapabilityToken, session: u64, epoch: u64) {
    let reply = sunlight_ipc::ipc_call_timeout(
        endpoint,
        IpcMsg::with_label(sunlight_ipc::SessionMsg::SESSION_ACTION)
            .word(0, session)
            .word(1, epoch)
            .word(2, sunlight_ipc::SessionAction::Logout as u64),
        5000,
    )
    .expect("logout transport");
    assert_eq!(
        reply.label,
        sunlight_ipc::SessionMsg::REPLY,
        "logout rejected"
    );
}
fn reap(pid: u64) -> u64 {
    let start = sunlight_ipc::monotonic_millis();
    for _ in 0..1_000_000 {
        if let Some(status) = sunlight_libc::try_waitpid(pid).expect("child status") {
            return status;
        }
        assert!(
            sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000,
            "child timeout"
        );
        sunlight_ipc::process_yield();
    }
    panic!("child wait exhausted")
}
fn controller(case: &str, bootstrap: CapabilityToken) -> ! {
    #[cfg(feature = "session-account-exec-test")]
    if case == "N6" {
        account_exec::controller(bootstrap);
    }
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    #[cfg(feature = "session-h4-test")]
    let prepared = if matches!(case, "N4" | "N5" | "N6" | "H4R" | "S1" | "S2" | "S3") {
        let unit: &[u8] = if case.starts_with('S') {
            b"sunlight-sessiond"
        } else {
            b"uac_service"
        };
        let control = sunlight_ipc::endpoint_create();
        let send = sunlight_ipc::endpoint_bind(control.0);
        let handle = alloc::format!("{}", send.0);
        let fixture = alloc::format!("{}", FIXTURE.load(core::sync::atomic::Ordering::Relaxed));
        let child = sunlight_libc::spawn(
            b"/bin/sunlightctl",
            &[
                b"sunlightctl",
                b"restart",
                unit,
                b"--h4-prepare",
                handle.as_bytes(),
                fixture.as_bytes(),
            ],
            None,
        )
        .expect("prestart authorized sunlightctl");
        let ready =
            sunlight_ipc::ipc_recv_timeout(control, 5000).expect("controller prepare timeout");
        assert_eq!(ready.label, 0xE304);
        assert_eq!(
            ready.words[0],
            FIXTURE.load(core::sync::atomic::Ordering::Relaxed)
        );
        assert_eq!(ready.badge, child);
        assert_eq!(ready.cap_count, 1);
        sunlight_ipc::ipc_reply_result(IpcMsg::with_label(0xE304)).expect("ready ack");
        Some((child, ready.caps[0]))
    } else {
        None
    };
    sunlight_ipc::ipc_call_timeout(
        bootstrap,
        IpcMsg::with_label(0xE302).with_cap(0, public),
        5000,
    )
    .expect("controller bootstrap");
    note(case, "controller-ready", 0);
    let message = sunlight_ipc::ipc_recv_timeout(ep, 15000).expect("rendezvous not reached");
    assert_eq!(message.label, 0xE301);
    note(case, "policy-accepted-rendezvous", message.words[1]);
    #[cfg(feature = "session-reap-test")]
    let userspace = if case == "N6" {
        use core::sync::atomic::{AtomicU64, Ordering};
        assert_eq!(message.words[1], 1, "committed-grant rendezvous required");
        assert_eq!(message.cap_count, 1);
        let cap = message.caps[0];
        let (ptr, bytes) = sunlight_ipc::shm_map_sized(cap).expect("userspace readiness mapping");
        assert!(bytes >= core::mem::size_of::<AtomicU64>());
        let state = unsafe { &*ptr.cast::<AtomicU64>() };
        #[cfg(feature = "session-reply-test")]
        authenticated_reply::controller(ep, message.badge);
        #[cfg(not(feature = "session-reply-test"))]
        sunlight_ipc::ipc_reply_result(IpcMsg::with_label(0xE301))
            .expect("release committed issuer to userspace");
        let start = sunlight_ipc::monotonic_millis();
        let mut ready = false;
        for _ in 0..1_000_000 {
            if state.load(Ordering::Acquire) == 1 {
                ready = true;
                break;
            }
            if sunlight_ipc::monotonic_millis().saturating_sub(start) >= 5000 {
                break;
            }
            sunlight_ipc::process_yield();
        }
        if !ready {
            state.store(2, Ordering::Release);
            let _ = sunlight_ipc::shm_free(cap);
            panic!("issuer did not return to userspace");
        }
        sunlight_ipc::debug_log(&alloc::format!(
            "[REAP-TEST] fixture={} checkpoint=issuer-userspace-ready issuer={} observer={}",
            FIXTURE.load(Ordering::Relaxed),
            message.badge,
            sunlight_libc::getpid()
        ));
        Some(cap)
    } else {
        None
    };
    if matches!(message.words[0], 4..=6 | 12..=15) {
        if matches!(message.words[0], 4 | 12..=15) && n4_query() != 1 {
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(0xE301)).expect("release N4 issuer");
            let start = sunlight_ipc::monotonic_millis();
            let mut ready = false;
            for _ in 0..1_000_000 {
                if n4_query() == 2 {
                    ready = true;
                    break;
                }
                assert!(
                    sunlight_ipc::monotonic_millis().saturating_sub(start) < 5000,
                    "N4 entry timeout"
                );
                sunlight_ipc::process_yield();
            }
            assert!(ready, "N4 entry exhausted");
            note(case, "broker-pending-observed", 0);
        }
        // The manager authenticates the root caller and performs its existing
        // stop/start lifecycle (eager UAC uses its legacy kill branch).
        #[cfg(feature = "session-h4-test")]
        let pid = {
            let (pid, release) = prepared.expect("prepared participant missing");
            let reply = sunlight_ipc::ipc_call_timeout(
                release,
                IpcMsg::with_label(0xE305)
                    .word(0, FIXTURE.load(core::sync::atomic::Ordering::Relaxed)),
                2000,
            )
            .expect("prepared controller release");
            assert_eq!(reply.label, 0xE305);
            pid
        };
        #[cfg(not(feature = "session-h4-test"))]
        let pid = sunlight_libc::spawn(
            b"/bin/sunlightctl",
            &[b"sunlightctl", b"restart", b"uac_service"],
            None,
        )
        .expect("authorized lifecycle client spawn");
        let status = reap(pid);
        note(case, "manager-restart-response", status as u64);
        assert_eq!(status, 0);
        #[cfg(feature = "session-reap-test")]
        if let Some(cap) = userspace {
            sunlight_ipc::shm_free(cap).expect("release readiness mapping after issuer death");
            note(case, "userspace-readiness-cleaned", 0);
        }
        // No attempt to execute or inject an old server reply after retirement.
    } else {
        let session_endpoint = endpoint(sunlight_ipc::SESSION_ENDPOINT);
        if matches!(message.words[0], 3 | 9 | 10 | 11) {
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(0xE301)).expect("release contender");
        }
        logout_on(session_endpoint, message.words[2], message.words[3]);
        note(case, "logout-response", 0);
        if !matches!(message.words[0], 3 | 9 | 10 | 11) {
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(0xE301)).expect("release UAC");
        }
    }
    sunlight_libc::exit(0)
}
fn request(
    scope: api::GrantScope,
    label: u64,
    grant: CapabilityToken,
    password: bool,
    control: CapabilityToken,
) -> Result<IpcMsg, sunlight_ipc::IpcCallError> {
    let (ptr, shm) = sunlight_ipc::shm_create(api::SHM_BYTES, 0).expect("request memory");
    unsafe {
        core::ptr::write_unaligned(ptr.add(256).cast::<u64>(), scope.revision);
        if password {
            core::ptr::copy_nonoverlapping(b"root".as_ptr(), ptr, 4);
        }
    }
    let mut msg = IpcMsg::with_label(label)
        .word(0, scope.session)
        .word(1, scope.session_generation)
        .word(2, scope.operation as u64)
        .word(3, scope.target as u64 | ((scope.member as u64) << 32));
    msg = if password {
        if (61001..=61015).contains(&scope.target) {
            msg.with_cap(0, shm).with_cap(1, control)
        } else {
            msg.with_cap(0, shm)
        }
    } else {
        msg.with_cap(0, grant).with_cap(1, shm)
    };
    #[cfg(feature = "session-account-exec-test")]
    let worker = (scope.target == 61006).then(|| account_exec::start_worker(ptr));
    let result = sunlight_ipc::ipc_call_timeout(endpoint("uac"), msg, 12000);
    unsafe {
        api::clear_secret(core::slice::from_raw_parts_mut(ptr, api::SECRET_LIMIT));
    }
    #[cfg(feature = "session-account-exec-test")]
    {
        if worker.is_some() {
            account_exec::prepare_release();
        }
        sunlight_ipc::shm_free(shm).expect("release request view");
        if let Some(worker) = worker {
            account_exec::finish_worker(worker);
        }
        if scope.target == 61006 {
            assert!(sunlight_ipc::shm_map(shm).is_err(), "retired request SHM accepted");
            assert!(sunlight_ipc::shm_free(shm).is_err(), "duplicate request free accepted");
            sunlight_ipc::debug_log(&alloc::format!(
                "[AE-USER] event=request-release-and-stale-denied pid={}",
                sunlight_libc::getpid()
            ));
            sunlight_ipc::debug_log("[AE-OBSERVE] event=caller-consumed");
        }
    }
    #[cfg(not(feature = "session-account-exec-test"))]
    let _ = sunlight_ipc::shm_free(shm);
    result
}
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    unsafe {
        ALLOCATOR
            .lock()
            .init(core::ptr::addr_of_mut!(HEAP).cast::<u8>(), 256 * 1024);
    }
    let mut args = [""; 10];
    let count = unsafe { sunlight_libc::crt0::collect_utf8_args(argc, argv, &mut args, 64) };
    assert!(count >= 2, "case required");
    let case = args[1];
    #[cfg(feature = "session-account-cancel-test")]
    {
        if case == "cancel-caller" {
            account_cancel::caller(&args[..count]);
        }
        if case == "N6" {
            account_cancel::supervisor(args[3], args[2]);
        }
    }
    #[cfg(feature = "session-account-exec-test")]
    if case == "account-exec" {
        account_exec::replacement(&args[..count]);
    }
    #[cfg(feature = "session-reply-test")]
    if matches!(case, "reply-peer" | "reply-exec") {
        authenticated_reply::peer(&args[..count]);
    }
    #[cfg(feature = "session-h4-test")]
    {
        let index = if args.get(2) == Some(&"controller") {
            4
        } else {
            2
        };
        let fixture = args[index].parse::<u64>().expect("fixture ID required");
        assert_ne!(fixture, 0);
        FIXTURE.store(fixture, core::sync::atomic::Ordering::Relaxed);
    }
    assert_eq!(
        sunlight_libc::getuid(),
        0,
        "requires genuinely authenticated root shell"
    );
    if args.get(2) == Some(&"controller") {
        controller(
            case,
            CapabilityToken(args[3].parse().expect("bootstrap handle")),
        );
    }
    let ordinal = match case {
        "N1" => 1,
        "N2" => 2,
        "N3M" => 3,
        "N3R" => 9,
        "N4" => 4,
        #[cfg(feature = "session-h4-test")]
        "H4R" => 12,
        #[cfg(feature = "session-h4-test")]
        "S1" => 13,
        #[cfg(feature = "session-h4-test")]
        "S2" => 14,
        #[cfg(feature = "session-h4-test")]
        "S3" => 15,
        "N5" => 5,
        "N6" => 6,
        "N7R" => 7,
        "N7C" => 8,
        "N7OC" => 10,
        "N7OR" => 11,
        _ => panic!("unsupported case"),
    };
    let snapshot = api::snapshot().expect("active session snapshot");
    let scope = api::GrantScope {
        session: snapshot.session_id,
        session_generation: snapshot.generation,
        revision: snapshot.revision,
        operation: api::Operation::GroupCreate,
        target: 61000 + ordinal,
        member: 0,
    };
    // Delegate a SEND_ONLY endpoint through the ordinary spawn argument path.
    // The handle is never logged. No arbitrary service lookup is added.
    let bootstrap = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(bootstrap.0);
    let handle = alloc::format!("{}", public.0);
    #[cfg(feature = "session-h4-test")]
    let fixture_arg = alloc::format!("{}", FIXTURE.load(core::sync::atomic::Ordering::Relaxed));
    #[cfg(not(feature = "session-h4-test"))]
    let fixture_arg = alloc::string::String::new();
    let child = sunlight_libc::spawn(
        b"/bin/session-racer",
        &[
            b"session-race-probe",
            case.as_bytes(),
            b"controller",
            handle.as_bytes(),
            fixture_arg.as_bytes(),
        ],
        None,
    )
    .expect("ordinary controller spawn");
    let hello = sunlight_ipc::ipc_recv_timeout(bootstrap, 5000).expect("controller handshake");
    assert_eq!(hello.label, 0xE302);
    assert_eq!(hello.badge, child);
    assert_eq!(hello.cap_count, 1);
    let control = hello.caps[0];
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(0xE302)).expect("bootstrap reply");
    note(case, "request-start", 0);
    let result = request(
        scope,
        api::AUTHORIZE,
        CapabilityToken::INVALID,
        true,
        control,
    );
    #[cfg(feature = "session-h4-test")]
    if matches!(ordinal, 13..=15) {
        note(
            case,
            "session-owner-result",
            result.as_ref().map_or(u64::MAX, |r| r.label),
        );
        assert_eq!(reap(child), 0);
        let owner_reply = result.expect("session owner call transport");
        assert!(matches!(owner_reply.label, api::ERROR | api::REPLY));
        if ordinal == 14 {
            assert_eq!(owner_reply.label, api::ERROR);
            assert_eq!(
                owner_reply.words[0],
                api::Error::SessionAuthorityUnavailable as u64
            );
            note(case, "broker-session-denied", owner_reply.words[0]);
        }
        let mut stale = scope;
        stale.target = 61000;
        let denied = request(
            stale,
            api::AUTHORIZE,
            CapabilityToken::INVALID,
            true,
            control,
        )
        .expect("post-retirement transport");
        assert_eq!(denied.label, api::ERROR);
        assert!(matches!(
            api::Error::from_raw(denied.words[0]),
            api::Error::Unavailable | api::Error::SessionChanged
        ));
        note(case, "post-session-owner-denied", denied.words[0]);
        note(case, "fixture-complete", 0);
        sunlight_libc::exit(0);
    }
    if matches!(ordinal, 4..=6 | 12) {
        assert!(matches!(
            result,
            Err(sunlight_ipc::IpcCallError::PeerClosed)
        ));
        note(case, "old-call-peer-closed", 0);
        #[cfg(not(feature = "session-account-exec-test"))]
        assert_eq!(reap(child), 0);
        let mut stale = scope;
        stale.target = 61000;
        let denied = request(
            stale,
            api::AUTHORIZE,
            CapabilityToken::INVALID,
            true,
            control,
        )
        .expect("replacement transport");
        assert_eq!(denied.label, api::ERROR);
        note(case, "replacement-old-session-denied", denied.words[0]);
        #[cfg(feature = "session-account-exec-test")]
        assert_eq!(reap(child), 0);
        logout(scope.session, scope.session_generation);
        note(case, "fixture-complete", 0);
        sunlight_libc::exit(0);
    }
    let reply = result.expect("mint transport");
    note(case, "mint-response", reply.label);
    match ordinal {
        1 | 9 => assert_eq!(reply.label, api::ERROR),
        2 | 3 => assert_eq!(reply.label, api::REPLY),
        7 | 8 | 10 | 11 => {
            assert_eq!(reply.label, api::REPLY);
            let consumed =
                request(scope, 0xE300, reply.caps[0], false, control).expect("consume transport");
            note(case, "consume-response", consumed.label);
            assert_eq!(
                consumed.label,
                if matches!(ordinal, 7 | 11) {
                    api::ERROR
                } else {
                    api::REPLY
                }
            );
            assert_eq!(
                request(scope, 0xE300, reply.caps[0], false, control)
                    .expect("second consume transport")
                    .label,
                api::ERROR
            );
            note(case, "second-consume-denied", 0);
        }
        _ => unreachable!(),
    }
    assert_eq!(reap(child), 0);
    if matches!(ordinal, 2 | 3) {
        let mut stale = scope;
        stale.target = 61000;
        assert_eq!(
            request(
                stale,
                api::AUTHORIZE,
                CapabilityToken::INVALID,
                true,
                control
            )
            .expect("stale mint transport")
            .label,
            api::ERROR
        );
        assert_eq!(
            request(scope, 0xE300, reply.caps[0], false, control)
                .expect("retired grant transport")
                .label,
            api::ERROR
        );
        note(case, "post-logout-authority-denied", 0);
    }
    note(case, "fixture-complete", 0);
    sunlight_libc::exit(0)
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // Source location only: formatted assertion operands could contain tokens.
    use core::fmt::Write;
    let mut text = heapless::String::<200>::new();
    let line = info.location().map_or(0, |location| location.line());
    let _ = write!(text, "[SESSION-E3-CLIENT] checkpoint=FAIL line={}", line);
    sunlight_ipc::debug_log(&text);
    sunlight_libc::exit(1)
}
