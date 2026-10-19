//! Native D2 concurrency, ownership, drain, failure and PMM evidence.
#![no_std]
#![no_main]
mod d1_log;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use sunlight_ipc::{self as ipc, demand as d, CapabilityToken, IpcMsg};
static MANAGER: AtomicU64 = AtomicU64::new(0);
static SERVICE: AtomicU64 = AtomicU64::new(0);
static GATE: AtomicUsize = AtomicUsize::new(0);
static DONE: AtomicUsize = AtomicUsize::new(0);
static GOOD: AtomicUsize = AtomicUsize::new(0);
static GEN: AtomicU64 = AtomicU64::new(0);
static LEASE: AtomicU64 = AtomicU64::new(0);
static BUSY: AtomicUsize = AtomicUsize::new(0);
fn manager() -> CapabilityToken {
    CapabilityToken(MANAGER.load(Ordering::Acquire))
}
fn sleep(ms: u64) {
    let ep = ipc::endpoint_create();
    let _ = ipc::ipc_recv_timeout(ep, ms);
    assert!(ipc::endpoint_destroy(ep));
}
fn rpc(msg: IpcMsg) -> IpcMsg {
    ipc::ipc_call_timeout(manager(), msg, 8000).expect("manager RPC")
}
fn call(op: u64) -> IpcMsg {
    rpc(d::request(op, "d2-fixture").unwrap())
}
fn direct(cap: CapabilityToken, msg: IpcMsg) -> IpcMsg {
    ipc::ipc_call_timeout(cap, msg, 4000).expect("service RPC")
}
fn pass(name: &str) {
    d1_log::log(format_args!("[D2-TEST] {} PASS", name));
}
fn wait_phase(phase: u64) {
    let deadline = ipc::monotonic_millis() + 6000;
    loop {
        let r = call(d::DISCOVER);
        if r.words[0] == phase {
            return;
        }
        assert!(
            ipc::monotonic_millis() < deadline,
            "phase {} actual {} failure {}",
            phase,
            r.words[0],
            r.words[2]
        );
        sleep(10);
    }
}
fn activate() -> (IpcMsg, CapabilityToken) {
    let a = call(d::ACTIVATE);
    assert_eq!(a.label, d::OK);
    let cap = ipc::nameserver_lookup_timeout("d2-fixture", 500).expect("fixture discovery");
    let ping = direct(cap, IpcMsg::with_label(1));
    assert_eq!(ping.label, d::OK);
    assert_eq!(ping.words[0], a.words[0]);
    assert_eq!(ping.words[1], a.words[2]);
    (a, cap)
}
fn lease(a: IpcMsg, ms: u64) -> u64 {
    let r = rpc(IpcMsg::with_label(d::LEASE_ACQUIRE)
        .word(0, a.words[0])
        .word(1, a.words[1])
        .word(2, a.words[2])
        .word(3, ms));
    assert_eq!(r.label, d::OK);
    r.words[0]
}
fn release(id: u64, generation: u64) {
    assert_eq!(
        rpc(IpcMsg::with_label(d::LEASE_RELEASE)
            .word(0, id)
            .word(1, generation))
        .label,
        d::OK
    );
}
extern "C" fn worker(arg: *mut u8) -> *mut u8 {
    // The harness barrier is deliberately bounded and sleeps between checks.
    for _ in 0..1000 {
        if GATE.load(Ordering::Acquire) != 0 {
            break;
        }
        sleep(5);
    }
    let mode = arg as usize;
    let ok = if mode == 1 {
        sleep(200);
        let a = call(d::ACTIVATE);
        if a.label == d::ERROR && a.words[0] == d::BUSY {
            BUSY.fetch_add(1, Ordering::AcqRel);
        }
        a.label == d::OK || (a.label == d::ERROR && a.words[0] == d::BUSY)
    } else if mode == 2 {
        let a = call(d::ACTIVATE);
        if a.label == d::OK {
            let previous = GEN
                .compare_exchange(0, a.words[0], Ordering::AcqRel, Ordering::Acquire)
                .unwrap_or_else(|x| x);
            previous == 0 || previous == a.words[0]
        } else {
            false
        }
    } else if mode == 3 {
        // Thread death leaves its lease behind. Expiry, not periodic liveness
        // polling, releases the manager-owned residency promise.
        let a = call(d::ACTIVATE);
        let id = lease(a, 300);
        LEASE.store(id, Ordering::Release);
        true
    } else if mode == 4 {
        rpc(IpcMsg::with_label(d::LEASE_RELEASE)
            .word(0, LEASE.load(Ordering::Acquire))
            .word(1, GEN.load(Ordering::Acquire)))
        .label
            == d::ERROR
    } else {
        let began = ipc::monotonic_millis();
        let result = direct(
            CapabilityToken(SERVICE.load(Ordering::Acquire)),
            IpcMsg::with_label(2).word(0, 300),
        );
        let elapsed = ipc::monotonic_millis().saturating_sub(began);
        d1_log::log(format_args!("[D2-TEST] drain_work_ms={}", elapsed));
        result.label == d::OK && elapsed >= 290
    };
    if ok {
        GOOD.fetch_add(1, Ordering::AcqRel);
    }
    DONE.fetch_add(1, Ordering::Release);
    core::ptr::null_mut()
}
fn workers(modes: &[usize]) {
    GATE.store(0, Ordering::Release);
    DONE.store(0, Ordering::Release);
    GOOD.store(0, Ordering::Release);
    for mode in modes {
        unsafe {
            sunlight_libc::thread::spawn(worker, *mode as *mut u8).unwrap();
        }
    }
    GATE.store(1, Ordering::Release);
    let deadline = ipc::monotonic_millis() + 10000;
    while DONE.load(Ordering::Acquire) < modes.len() {
        assert!(ipc::monotonic_millis() < deadline);
        sleep(10);
    }
    assert_eq!(GOOD.load(Ordering::Acquire), modes.len());
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    MANAGER.store(
        ipc::nameserver_lookup_timeout("sunlightd", 2000).unwrap().0,
        Ordering::Release,
    );
    // D1 runs concurrently on the same boot. Keep memory samples beyond its
    // initialization/crash tests and the resident startup interval.
    sleep(15000);
    assert_eq!(call(d::DISCOVER).words[0], 0);
    pass("inactive default and administrative activation retained");
    let (a, cap) = activate();
    SERVICE.store(cap.0, Ordering::Release);
    assert!(d::instance_status(a.words[1] as u32, a.words[2]).is_none());
    assert_eq!(
        rpc(IpcMsg::with_label(d::IDLE_REPORT)
            .word(0, a.words[0])
            .word(1, 0)
            .word(2, 0))
        .label,
        d::ERROR
    );
    // A caller with a valid endpoint cannot forge the manager's control badge.
    assert_eq!(
        direct(
            cap,
            IpcMsg::with_label(d::QUIESCE)
                .word(0, a.words[0])
                .word(1, 1)
        )
        .label,
        d::ERROR
    );
    let id = lease(a, 5000);
    LEASE.store(id, Ordering::Release);
    GEN.store(a.words[0], Ordering::Release);
    workers(&[4]);
    release(id, a.words[0]);
    pass("authenticated idle quiesce and lease ownership");
    // Declared sessions/background work and persistent state cancel grace.
    direct(cap, IpcMsg::with_label(3).word(0, 1).word(1, 1));
    sleep(800);
    assert_eq!(call(d::DISCOVER).words[0], 2);
    for op in [2, 3] {
        assert_eq!(rpc(d::request(op, "d2-fixture").unwrap()).label, 0xff);
    }
    assert!(ipc::process_is_alive(a.words[1]));
    direct(cap, IpcMsg::with_label(3).word(0, 0).word(1, 0));
    wait_phase(4);
    sleep(200);
    assert_eq!(call(d::ACTIVATE).words[0], a.words[0]);
    sleep(350);
    assert_eq!(call(d::DISCOVER).words[0], 4);
    pass("busy persistent state minimum uptime and grace cancellation");
    // These real calls cross the idle deadline. Work may finish before the
    // barrier, or activation may return Busy after the irrevocable boundary.
    workers(&[0, 1, 1]);
    assert!(
        BUSY.load(Ordering::Acquire) > 0,
        "activation must overlap quiescing/stopping"
    );
    wait_phase(6);
    let stopping = call(d::ACTIVATE);
    assert_eq!(stopping.label, d::ERROR);
    assert_eq!(stopping.words[0], d::BUSY);
    assert_eq!(
        rpc(IpcMsg::with_label(d::LEASE_RELEASE)
            .word(0, id)
            .word(1, a.words[0]))
        .label,
        d::ERROR
    );
    for op in [2, 3] {
        assert_eq!(rpc(d::request(op, "d2-fixture").unwrap()).label, 0xff);
    }
    let closed = ipc::ipc_call_timeout(cap, IpcMsg::with_label(2).word(0, 1), 1000);
    assert!(
        matches!(
            closed,
            Err(ipc::IpcCallError::PeerClosed | ipc::IpcCallError::InvalidCapability)
        ) || closed.is_ok_and(|r| r.label == d::ERROR)
    );
    wait_phase(0);
    assert!(ipc::ipc_call_timeout(cap, IpcMsg::with_label(1), 100).is_err());
    pass("concurrent work activation stopping and restart barrier");
    let starts = call(d::METRICS).words[1];
    GEN.store(0, Ordering::Release);
    workers(&[2, 2, 2, 2, 2, 2, 2, 2]);
    assert_eq!(call(d::METRICS).words[1], starts + 1);
    wait_phase(0);
    pass("concurrent reactivation single flight fresh endpoint");
    let (a, _) = activate();
    workers(&[3]);
    assert_eq!(
        rpc(IpcMsg::with_label(d::LEASE_RELEASE)
            .word(0, LEASE.load(Ordering::Acquire))
            .word(1, a.words[0]))
        .label,
        d::ERROR
    );
    wait_phase(0);
    pass("client death cannot permanently leak residency lease");
    // Failure injections retain the process without forced termination. A
    // cooperative test-only exit allows a later authorized generation.
    for mode in [1, 2, 4] {
        let (a, cap) = activate();
        direct(cap, IpcMsg::with_label(6).word(0, mode));
        wait_phase(3);
        assert!(ipc::process_is_alive(a.words[1]));
        assert_eq!(call(d::ACTIVATE).words[0], d::BUSY);
        if mode == 2 {
            sleep(700);
        }
        direct(cap, IpcMsg::with_label(5));
        sleep(1200);
        assert!(!ipc::process_is_alive(a.words[1]));
    }
    pass("stale acknowledgement quiesce timeout and exit timeout retain runtime");
    // A crash after an idle report is Failed, never a successful unload.
    let before = call(d::IDLE_METRICS).words[0];
    let (a, cap) = activate();
    direct(cap, IpcMsg::with_label(5));
    wait_phase(3);
    assert_eq!(call(d::IDLE_METRICS).words[0], before);
    assert!(!ipc::process_is_alive(a.words[1]));
    sleep(1100);
    pass("crash while idle is not successful unload");
    let consumer = rpc(d::request(d::ACTIVATE, "d2-dependent").unwrap());
    assert_eq!(consumer.label, d::OK);
    let provider = call(d::DISCOVER);
    sleep(800);
    assert_eq!(call(d::DISCOVER).words[0], 2);
    assert_eq!(call(d::DISCOVER).words[1], provider.words[1]);
    let dependent = ipc::nameserver_lookup_timeout("d2-dependent", 500).unwrap();
    let _ = ipc::ipc_call_timeout(dependent, IpcMsg::with_label(2), 1000);
    wait_phase(0);
    pass("dependency holds residency until consumer exit");
    let mut low = u64::MAX;
    let mut high = 0;
    let mut last_gen = 0;
    for cycle in 0..6 {
        let before = ipc::sysinfo();
        let before_free = (before.total_ram_kb - before.used_ram_kb) / 4;
        let began = ipc::monotonic_millis();
        let (a, cap) = activate();
        let ready_ms = ipc::monotonic_millis() - began;
        assert!(a.words[0] > last_gen);
        last_gen = a.words[0];
        let id = lease(a, 5000);
        let mut shared = None;
        if cycle == 0 {
            let r = direct(cap, IpcMsg::with_label(4));
            let token = r.caps[0];
            let p = ipc::shm_map(token).unwrap();
            assert_eq!(unsafe { p.read_volatile() }, 0xa5);
            shared = Some((p, token));
        }
        assert_eq!(direct(cap, IpcMsg::with_label(2).word(0, 20)).label, d::OK);
        let running = call(d::IDLE_ACCOUNT);
        assert_eq!(running.label, d::OK);
        let idle_start = ipc::monotonic_millis();
        sleep(300);
        let idle_end = call(d::IDLE_ACCOUNT);
        let cpu_ns = idle_end.words[0] - running.words[0];
        let idle_ms = ipc::monotonic_millis() - idle_start;
        assert!(cpu_ns < 5_000_000, "service must block while idle");
        assert!(running.words[1] >= 256);
        assert_eq!(running.words[2], 1);
        release(id, a.words[0]);
        wait_phase(6);
        let shutdown_at = ipc::monotonic_millis();
        wait_phase(0);
        let shutdown_ms = ipc::monotonic_millis() - shutdown_at;
        let receipt = call(d::IDLE_METRICS);
        assert!(receipt.words[1] >= 256);
        assert!(receipt.words[2] > 0);
        if let Some((p, token)) = shared {
            assert_eq!(unsafe { p.read_volatile() }, 0xa5);
            ipc::shm_free(token).unwrap();
        }
        let after = ipc::sysinfo();
        let after_free = (after.total_ram_kb - after.used_ram_kb) / 4;
        if cycle > 0 {
            low = low.min(after_free);
            high = high.max(after_free);
        }
        d1_log::log(format_args!("[D2-MEM] cycle={} generation={} before_free={} running_free={} after_free={} private_frames={} page_tables={} shared_pages={} ready_ms={} shutdown_ms={} idle_cpu_ns={} idle_ms={}",
            cycle,a.words[0],before_free,running.words[3],after_free,receipt.words[1],receipt.words[2],running.words[2],ready_ms,shutdown_ms,cpu_ns,idle_ms));
        assert!(after_free > running.words[3] + 200);
        assert!(ipc::ipc_call_timeout(cap, IpcMsg::with_label(1), 100).is_err());
        assert_eq!(
            rpc(IpcMsg::with_label(d::READY).word(0, a.words[0])).label,
            d::ERROR
        );
    }
    d1_log::log(format_args!(
        "[D2-TEST] stability_range_frames={}",
        high - low
    ));
    assert!(
        high - low <= 16,
        "repeated service cycles must not leak private pages"
    );
    pass("repeated cycles private reclamation shared references and idle CPU");
    assert!(ipc::nameserver_lookup_timeout("tz", 500).is_some());
    pass("resident compatibility");
    pass("COMPLETE");
    sunlight_libc::exit(0)
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    d1_log::log(format_args!("[D2-TEST] driver FAIL {}", info));
    sunlight_libc::exit(99)
}
