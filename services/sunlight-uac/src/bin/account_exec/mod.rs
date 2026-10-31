//! Test-only real Account exec protocol. Capabilities cross private IPC/argv;
//! debug output contains no credentials, bearer tokens or completion handles.
#![allow(dead_code)] // Shared by the daemon and ordinary replacement binary.
use sunlight_ipc::{accounts as api, CapabilityToken, EndpointId, IpcError, IpcMsg};

const SETUP: u64 = 0xEA01;
const READY: u64 = 0xEA02;
const FRESH: u64 = 0xEA03;
const DONE: u64 = 0xEA04;

fn user(event: &str) {
    sunlight_ipc::debug_log(&alloc::format!(
        "[AE-USER] event={} pid={}",
        event,
        sunlight_libc::getpid()
    ));
}
fn observe(event: &str) {
    sunlight_ipc::debug_log(&alloc::format!("[AE-OBSERVE] event={}", event));
}
fn denied(handle: u64) {
    assert!(!sunlight_ipc::ipc_deferred_reply_is_live(handle));
    for _ in 0..2 {
        assert_eq!(
            sunlight_ipc::ipc_complete_deferred_reply(handle, IpcMsg::with_label(FRESH)),
            Err(IpcError::InvalidArgument)
        );
    }
}

pub fn account(
    ep: EndpointId,
    control: CapabilityToken,
    grant: CapabilityToken,
    scope: api::GrantScope,
    caller: u64,
) -> ! {
    observe("committed");
    let old = sunlight_ipc::ipc_defer_reply().expect("defer actual committed AUTHORIZE");
    assert!(sunlight_ipc::ipc_deferred_reply_is_live(old));
    let setup = sunlight_ipc::ipc_call_timeout(
        control,
        IpcMsg::with_label(SETUP)
            .word(0, scope.session)
            .word(1, scope.session_generation),
        5000,
    )
    .expect("exec controller");
    assert_eq!(setup.label, SETUP);
    assert!(sunlight_libc::exec(b"/missing-account-exec", &[b"account-exec"]).is_err());
    assert!(sunlight_ipc::ipc_deferred_reply_is_live(old));
    observe("failed-exec-preserved");
    let values = [
        ep.0,
        control.0,
        old,
        grant.0,
        scope.session,
        scope.session_generation,
        caller,
        scope.revision,
    ];
    let args: alloc::vec::Vec<alloc::string::String> =
        values.iter().map(|v| alloc::format!("{}", v)).collect();
    observe("exec-enter");
    sunlight_libc::exec(
        b"/bin/session-racer",
        &[
            b"session-racer",
            b"account-exec",
            args[0].as_bytes(),
            args[1].as_bytes(),
            args[2].as_bytes(),
            args[3].as_bytes(),
            args[4].as_bytes(),
            args[5].as_bytes(),
            args[6].as_bytes(),
            args[7].as_bytes(),
        ],
    )
    .expect("privileged Account exec");
    panic!("exec returned to old Account image")
}

pub fn controller(bootstrap: CapabilityToken) -> ! {
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let hello = sunlight_ipc::ipc_call_timeout(
        bootstrap,
        IpcMsg::with_label(0xE302).with_cap(0, public),
        5000,
    )
    .expect("controller bootstrap");
    assert_eq!(hello.label, 0xE302);
    let setup = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("Account committed setup");
    assert_eq!(setup.label, SETUP);
    let account = setup.badge;
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(SETUP)).expect("permit exec");
    let ready = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("replacement entry");
    assert_eq!(ready.label, READY);
    assert_eq!(ready.badge, account);
    assert_eq!(ready.cap_count, 1);
    let inherited = ready.caps[0];
    // A SEND capability and a different PID cannot manage the retained endpoint.
    assert!(!sunlight_ipc::endpoint_destroy(EndpointId(inherited.0)));
    assert!(sunlight_ipc::ipc_recv_timeout(EndpointId(inherited.0), 1).is_none());
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(READY)).expect("entry ack");
    let fresh = sunlight_ipc::ipc_call_timeout(inherited, IpcMsg::with_label(FRESH), 5000)
        .expect("process endpoint survives exec");
    assert_eq!(fresh.label, FRESH);
    let done = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("replacement finished denials");
    assert_eq!(done.label, DONE);
    assert_eq!(done.badge, account);
    sunlight_ipc::ipc_reply_result(IpcMsg::with_label(DONE)).expect("replacement exit ack");
    let manager = sunlight_libc::spawn(
        b"/bin/sunlightctl",
        &[b"sunlightctl", b"restart", b"uac_service"],
        None,
    )
    .expect("ordinary authorized Account restart");
    let start = sunlight_ipc::monotonic_millis();
    let mut restored = false;
    for _ in 0..1_000_000 {
        if let Some(code) = sunlight_libc::try_waitpid(manager).expect("manager wait") {
            assert_eq!(code, 0);
            restored = true;
            break;
        }
        assert!(
            sunlight_ipc::monotonic_millis().saturating_sub(start) < 10000,
            "manager restart deadline"
        );
        sunlight_ipc::process_yield();
    }
    assert!(restored);
    user("manager-restored");
    assert!(sunlight_ipc::endpoint_destroy(ep));
    observe("controller-complete");
    user("controller-complete");
    sunlight_libc::exit(0)
}

pub fn replacement(args: &[&str]) -> ! {
    assert_eq!(args.len(), 10);
    let number = |i: usize| args[i].parse::<u64>().expect("private exec argument");
    let ep = EndpointId(number(2));
    let control = CapabilityToken(number(3));
    let old = number(4);
    let grant = CapabilityToken(number(5));
    let scope = api::GrantScope {
        session: number(6),
        session_generation: number(7),
        revision: number(9),
        operation: api::Operation::GroupCreate,
        target: 61006,
        member: 0,
    };
    let caller = number(8);
    observe("replacement");
    denied(old);
    let public = sunlight_ipc::endpoint_bind(ep.0);
    assert_ne!(public, CapabilityToken::INVALID);
    sunlight_ipc::ipc_call_timeout(control, IpcMsg::with_label(READY).with_cap(0, public), 5000)
        .expect("replacement handshake");
    // The original caller and the controller can arrive in either order.
    let mut controller_seen = false;
    let mut caller_seen = false;
    while !(controller_seen && caller_seen) {
        let msg = sunlight_ipc::ipc_recv_timeout(ep, 5000).expect("replacement fresh request");
        observe("fresh-request");
        denied(old); // Stale completion cannot steal a newer caller's request.
        assert_eq!(
            api::try_broker_grant(msg.badge, scope, Some(grant)),
            Err(api::Error::Denied)
        );
        assert_eq!(
            api::try_broker_grant(msg.badge, scope, None),
            Err(api::Error::Denied)
        );
        if msg.label == FRESH {
            assert!(!controller_seen);
            controller_seen = true;
            sunlight_ipc::ipc_reply_result(IpcMsg::with_label(FRESH))
                .expect("fresh endpoint reply");
        } else {
            assert_eq!(msg.label, api::AUTHORIZE);
            assert_eq!(msg.badge, caller);
            assert!(!caller_seen);
            caller_seen = true;
            sunlight_ipc::ipc_reply_result(
                IpcMsg::with_label(api::ERROR).word(0, api::Error::SessionChanged as u64),
            )
            .expect("old session denied");
        }
    }
    observe("replacement-complete");
    user("replacement-stale-and-authority-denied");
    // Keep the process-owned endpoint until ordinary exit cleanup. No role is
    // assigned to the replacement, even though its PID is unchanged.
    let done = sunlight_ipc::ipc_call_timeout(control, IpcMsg::with_label(DONE), 5000)
        .expect("replacement done handshake");
    assert_eq!(done.label, DONE);
    sunlight_libc::exit(0)
}

use core::sync::atomic::{AtomicU64, Ordering};
static WORKER_PHASE: AtomicU64 = AtomicU64::new(0);

extern "C" fn hold_mapping(address: *mut u8) -> *mut u8 {
    // Safety: the requester retains this view through both handshakes. Only
    // the final unused byte is read, never the secret area. No pointer access
    // occurs after worker-held is published with Release.
    unsafe {
        core::ptr::read_volatile(address.add(api::SHM_BYTES - 1));
    }
    observe("worker-ready");
    WORKER_PHASE.store(1, Ordering::Release);
    let start = sunlight_ipc::monotonic_millis();
    for i in 0..1_000_000_000u64 {
        match WORKER_PHASE.load(Ordering::Acquire) {
            2 => return core::ptr::null_mut(),
            3 => {
                // The owner waits for phase 4 before unmapping this view.
                unsafe {
                    core::ptr::read_volatile(address.add(api::SHM_BYTES - 1));
                }
                observe("worker-held");
                WORKER_PHASE.store(4, Ordering::Release);
            }
            _ => {}
        }
        if i % 100_000 == 0 {
            assert!(
                sunlight_ipc::monotonic_millis().saturating_sub(start) < 5000,
                "mapping worker deadline"
            );
        }
        core::hint::spin_loop();
    }
    panic!("mapping worker exhausted")
}

pub fn start_worker(address: *mut u8) -> sunlight_libc::thread::JoinHandle {
    WORKER_PHASE.store(0, Ordering::Relaxed);
    // Safety: address points to the requester's live 4 KiB SHM view. The
    // handshake above finishes its only read before start_worker returns.
    let worker =
        unsafe { sunlight_libc::thread::spawn_with_stack(hold_mapping, address, 64 * 1024, false) }
            .expect("ordinary caller thread");
    let start = sunlight_ipc::monotonic_millis();
    for _ in 0..1_000_000 {
        if WORKER_PHASE.load(Ordering::Acquire) == 1 {
            return worker;
        }
        assert!(
            sunlight_ipc::monotonic_millis().saturating_sub(start) < 5000,
            "worker ready deadline"
        );
        sunlight_ipc::process_yield();
    }
    panic!("worker readiness exhausted")
}

pub fn prepare_release() {
    WORKER_PHASE.store(3, Ordering::Release);
    let start = sunlight_ipc::monotonic_millis();
    for _ in 0..1_000_000 {
        if WORKER_PHASE.load(Ordering::Acquire) == 4 {
            return;
        }
        assert!(
            sunlight_ipc::monotonic_millis().saturating_sub(start) < 5000,
            "worker hold deadline"
        );
        sunlight_ipc::process_yield();
    }
    panic!("worker hold exhausted")
}

pub fn finish_worker(worker: sunlight_libc::thread::JoinHandle) {
    WORKER_PHASE.store(2, Ordering::Release);
    let start = sunlight_ipc::monotonic_millis();
    let mut reaped = false;
    for _ in 0..1_000_000 {
        // NativeThreadReaped is the existing VM-bound quiescence query.
        // Finished alone does not authorize unmapping a thread stack.
        if unsafe { sunlight_libc::sys::syscall1(156, u64::from(worker.tid)) } == 1 {
            let code = sunlight_libc::try_waitpid(u64::from(worker.tid))
                .expect("worker wait")
                .expect("reaped worker status");
            assert_eq!(code, 0);
            reaped = true;
            break;
        }
        assert!(
            sunlight_ipc::monotonic_millis().saturating_sub(start) < 5000,
            "worker reap deadline"
        );
        sunlight_ipc::process_yield();
    }
    assert!(reaped, "worker not quiescent");
    sunlight_libc::mman::munmap(worker.stack_base, worker.stack_size)
        .expect("quiescent worker stack release");
    sunlight_libc::mman::munmap(worker.tls_base, 4096).expect("quiescent worker TLS release");
    user("worker-cleaned");
}
