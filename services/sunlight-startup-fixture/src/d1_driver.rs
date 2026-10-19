//! Native D1 tests: actual concurrent IPC calls, kernel denial and lifecycle.
#![no_std]
#![no_main]
mod d1_log;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use d1_log::log;
use sunlight_ipc::{self as ipc, demand as d, CapabilityToken, IpcMsg, ServiceCapability as C};
static MANAGER: AtomicU64 = AtomicU64::new(0);
static DONE: AtomicUsize = AtomicUsize::new(0);
static PASSED: AtomicUsize = AtomicUsize::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static PROCESS: AtomicU64 = AtomicU64::new(0);
static RELEASE: AtomicUsize = AtomicUsize::new(0);

fn call(op: u64, name: &str) -> IpcMsg {
    ipc::ipc_call_timeout(
        CapabilityToken(MANAGER.load(Ordering::Acquire)),
        d::request(op, name).unwrap(),
        8_000,
    )
    .unwrap_or_else(|error| {
        log(format_args!(
            "[D1-TEST] RPC op={:#x} unit={} error={:?}",
            op, name, error
        ));
        panic!("RPC");
    })
}
fn sleep(ms: u64) {
    let ep = ipc::endpoint_create();
    let _ = ipc::ipc_recv_timeout(ep, ms);
    assert!(ipc::endpoint_destroy(ep));
}
fn pass(name: &str) {
    log(format_args!("[D1-TEST] {} PASS", name));
}
extern "C" fn worker(arg: *mut u8) -> *mut u8 {
    while RELEASE.load(Ordering::Acquire) == 0 {
        sleep(5);
    }
    let manager = CapabilityToken(MANAGER.load(Ordering::Acquire));
    let deadline = if arg as usize == 1 { 40 } else { 8_000 };
    let r = ipc::ipc_call_timeout(
        manager,
        d::request(d::ACTIVATE, "d1-fixture").unwrap(),
        deadline,
    );
    let ok = match r {
        Err(ipc::IpcCallError::Timeout) if arg as usize == 1 => true,
        Ok(r) if r.label == d::OK && r.words[0] != 0 && r.words[2] != 0 => {
            let old = GENERATION
                .compare_exchange(0, r.words[0], Ordering::AcqRel, Ordering::Acquire)
                .unwrap_or_else(|x| x);
            let old_p = PROCESS
                .compare_exchange(0, r.words[2], Ordering::AcqRel, Ordering::Acquire)
                .unwrap_or_else(|x| x);
            (old == 0 || old == r.words[0]) && (old_p == 0 || old_p == r.words[2])
        }
        _ => false,
    };
    if ok {
        PASSED.fetch_add(1, Ordering::AcqRel);
    }
    DONE.fetch_add(1, Ordering::Release);
    core::ptr::null_mut()
}
fn control(op: u64) -> IpcMsg {
    let mut m = d::request(op, "d1-fixture").unwrap();
    m.label = op;
    ipc::ipc_call_timeout(CapabilityToken(MANAGER.load(Ordering::Acquire)), m, 7_000)
        .expect("control")
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let manager = ipc::nameserver_lookup_timeout("sunlightd", 2_000).expect("manager");
    MANAGER.store(manager.0, Ordering::Release);
    assert_eq!(call(d::DISCOVER, "d1-fixture").words[0], 0);
    assert!(ipc::nameserver_lookup_timeout("d1-fixture", 500).is_none());
    pass("inactive discoverable at boot");
    // A process with no lifecycle authority receives a manager token explicitly.
    let spawn = ipc::nameserver_lookup_timeout("spawn", 1_000).expect("spawn");
    let mut req = IpcMsg::empty();
    ipc::SpawnRequest::new("/sbin/d1-denied", "d1-denied")
        .with_identity(0, 0)
        .with_service_caps(C::Logging.bit())
        .pack_into(&mut req);
    assert_eq!(
        ipc::ipc_call_timeout(spawn, req, 1_000).unwrap().label,
        ipc::SpawnMsg::REPLY
    );
    let mut denied = None;
    for _ in 0..100 {
        denied = ipc::nameserver_lookup_timeout("d1-denied", 100);
        if denied.is_some() {
            break;
        }
        sleep(10);
    }
    assert_eq!(
        ipc::ipc_call_timeout(
            denied.expect("denied"),
            IpcMsg::with_label(1).word(0, manager.0),
            2_000
        )
        .unwrap()
        .words[0],
        1
    );
    assert_eq!(call(d::DISCOVER, "d1-fixture").words[0], 0);
    pass("unauthorized bearer rejected without activation");
    for i in 0..8 {
        unsafe {
            sunlight_libc::thread::spawn(
                worker,
                if i == 0 {
                    1usize as *mut u8
                } else {
                    core::ptr::null_mut()
                },
            )
            .expect("thread");
        }
    }
    let began = ipc::monotonic_millis();
    RELEASE.store(1, Ordering::Release);
    for _ in 0..1_000 {
        if DONE.load(Ordering::Acquire) == 8 {
            break;
        }
        sleep(10);
    }
    assert_eq!(DONE.load(Ordering::Acquire), 8);
    assert_eq!(PASSED.load(Ordering::Acquire), 8);
    let m = call(d::METRICS, "d1-fixture");
    assert_eq!(m.words[1], 1);
    assert!(m.words[2] >= 1);
    pass("concurrent single flight and cancellation");
    let warm_began = ipc::monotonic_millis();
    let first = call(d::ACTIVATE, "d1-fixture");
    log(format_args!(
        "[D1-TEST] warm_activation_ms={}",
        ipc::monotonic_millis().saturating_sub(warm_began)
    ));
    assert_eq!(first.words[0], GENERATION.load(Ordering::Acquire));
    assert_eq!(call(d::METRICS, "d1-fixture").words[1], 1);
    let cap = ipc::nameserver_lookup_timeout("d1-fixture", 500).expect("invocation lookup");
    let pong = ipc::ipc_call_timeout(cap, IpcMsg::with_label(1), 1_000).unwrap();
    assert_eq!(pong.words[0], first.words[0]);
    assert_eq!(pong.words[1], first.words[2]);
    pass("ready reuse and direct IPC identity");
    let mut request = IpcMsg::empty();
    ipc::SpawnRequest::new("/sbin/d1-activate-only", "d1-activate-only")
        .with_identity(0, 0)
        .with_service_caps(C::ServiceLifecycle.bit())
        .pack_into(&mut request);
    assert_eq!(
        ipc::ipc_call_timeout(spawn, request, 1_000).unwrap().label,
        ipc::SpawnMsg::REPLY
    );
    let mut activate_only = None;
    for _ in 0..100 {
        activate_only = ipc::nameserver_lookup_timeout("d1-activate-only", 100);
        if activate_only.is_some() {
            break;
        }
        sleep(10);
    }
    assert_eq!(
        ipc::ipc_call_timeout(activate_only.unwrap(), IpcMsg::with_label(1), 3_000)
            .unwrap()
            .words[0],
        1
    );
    pass("activation does not grant invocation");
    // Failures are explicit, and timeout completion removes pending state.
    assert_eq!(call(d::ACTIVATE, "d1-missing").words[0], d::START_FAILED);
    assert_eq!(call(d::ACTIVATE, "d1-crash").label, d::ERROR);
    let timeout = call(d::ACTIVATE, "d1-timeout");
    assert_eq!(timeout.label, d::ERROR);
    assert_eq!(timeout.words[0], d::TIMED_OUT);
    assert_eq!(call(d::DISCOVER, "d1-timeout").words[0], 3);
    pass("spawn failure crash before ready and timeout");
    assert_eq!(control(2).label, 1);
    sleep(1_050);
    assert_eq!(call(d::ACTIVATE, "d1-dependent").label, d::OK);
    assert_eq!(call(d::METRICS, "d1-fixture").words[1], 2);
    pass("dependency activation");
    let before_restart = call(d::ACTIVATE, "d1-fixture");
    let stale_endpoint = ipc::nameserver_lookup_timeout("d1-fixture", 500).unwrap();
    let _ = ipc::ipc_call_timeout(stale_endpoint, IpcMsg::with_label(3), 500);
    assert_eq!(call(d::DISCOVER, "d1-fixture").words[0], 3);
    pass("destroyed endpoint invalidates live instance");
    assert_eq!(control(3).label, 1);
    let replacement = call(d::ACTIVATE, "d1-fixture");
    assert_eq!(replacement.label, d::OK);
    assert!(replacement.words[0] > before_restart.words[0]);
    assert_ne!(replacement.words[2], before_restart.words[2]);
    assert!(ipc::ipc_call_timeout(cap, IpcMsg::with_label(1), 500).is_err());
    pass("restart fresh identity stale capability rejected");
    let current = ipc::nameserver_lookup_timeout("d1-fixture", 500).unwrap();
    let _ = ipc::ipc_call_timeout(current, IpcMsg::with_label(2), 1_000);
    sleep(100);
    assert_eq!(call(d::DISCOVER, "d1-fixture").words[0], 3);
    pass("ready crash invalidates discovery");
    // Resident timezone behavior still uses the existing contract.
    assert!(ipc::nameserver_lookup_timeout("tz", 500).is_some());
    pass("resident services unaffected");
    let elapsed = ipc::monotonic_millis().saturating_sub(began);
    log(format_args!("[D1-TEST] lifecycle_elapsed_ms={}", elapsed));
    pass("COMPLETE");
    sunlight_libc::exit(0)
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log(format_args!("[D1-TEST] driver FAIL {}", info));
    sunlight_libc::exit(99)
}
