//! Genuine concurrent graph activation and measured reclamation after all holds.
use super::*;
static RESULTS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
const NAMES: [&str; 3] = ["d3-parent", "d3-leaf", "d2-fixture"];
fn call_name(op: u64, name: &str) -> IpcMsg {
    rpc(d::request(op, name).unwrap())
}
fn cap(name: &str) -> CapabilityToken {
    ipc::nameserver_lookup_timeout(name, 500).unwrap()
}
fn wait(name: &str, phase: u64) {
    let deadline = ipc::monotonic_millis() + 10000;
    loop {
        let r = call_name(d::DISCOVER, name);
        if r.words[0] == phase {
            return;
        }
        assert!(
            ipc::monotonic_millis() < deadline,
            "{} phase {} actual {} failure {}",
            name,
            phase,
            r.words[0],
            r.words[2]
        );
        sleep(10);
    }
}
fn empty() {
    for name in NAMES {
        wait(name, 0);
    }
    assert_eq!(call_name(d::DEPENDENCY_METRICS, "d3-parent").words[0], 0);
}
fn pass3(name: &str) {
    d1_log::log(format_args!("[D3-TEST] {} PASS", name));
}
extern "C" fn graph_worker(arg: *mut u8) -> *mut u8 {
    let id = arg as usize;
    for _ in 0..1000 {
        if GATE.load(Ordering::Acquire) != 0 {
            break;
        }
        sleep(5);
    }
    let r = call_name(
        d::ACTIVATE,
        if id % 2 == 0 { "d3-parent" } else { "d3-leaf" },
    );
    assert_eq!(r.label, d::OK);
    RESULTS[id].store(r.words[0], Ordering::Release);
    DONE.fetch_add(1, Ordering::Release);
    core::ptr::null_mut()
}
fn cleanup_thread(handle: sunlight_libc::thread::JoinHandle) {
    let deadline = ipc::monotonic_millis() + 2000;
    // Existing native std ThreadReaped ABI; DONE alone is not safe stack reuse.
    while unsafe { sunlight_libc::sys::syscall1(156, handle.tid as u64) } != 1 {
        assert!(ipc::monotonic_millis() < deadline);
        sleep(5);
    }
    sunlight_libc::mman::munmap(handle.stack_base, handle.stack_size).unwrap();
    sunlight_libc::mman::munmap(handle.tls_base, 4096).unwrap();
}
static PENDING_LABEL: AtomicU64 = AtomicU64::new(0);
extern "C" fn pending_worker(_: *mut u8) -> *mut u8 {
    let r = call_name(d::ACTIVATE, "d3-parent");
    PENDING_LABEL.store(r.label, Ordering::Release);
    core::ptr::null_mut()
}
fn spawn_client() -> CapabilityToken {
    let spawn = ipc::nameserver_lookup_timeout("spawn", 500).unwrap();
    let mut request = IpcMsg::empty();
    ipc::SpawnRequest::new("/sbin/d3-client", "d3-client")
        .with_identity(1000, 1000)
        .with_service_caps(
            ipc::ServiceCapability::DemandActivateTest.bit()
                | ipc::ServiceCapability::Logging.bit(),
        )
        .pack_into(&mut request);
    assert_eq!(
        ipc::ipc_call_timeout(spawn, request, 1000).unwrap().label,
        ipc::SpawnMsg::REPLY
    );
    for _ in 0..200 {
        if let Some(client) = ipc::nameserver_lookup_timeout("d3-client", 100) {
            return client;
        }
        sleep(10);
    }
    panic!("client did not register")
}
pub(super) fn run() {
    // Original D1 intentionally leaves its consumer alive after destroying and
    // replacing its provider. D3 quarantines that consumer and retains its old
    // ownership correctly. Explicitly stop it before isolated graph metrics.
    let _ = call_name(2, "d1-dependent");
    let deadline = ipc::monotonic_millis() + 6000;
    while call_name(d::DEPENDENCY_METRICS, "d3-parent").words[0] != 0 {
        assert!(ipc::monotonic_millis() < deadline);
        sleep(10);
    }
    empty();
    // Spawn an ordinary application with only its explicitly scoped capability.
    let client = spawn_client();
    let result =
        ipc::ipc_call_timeout(client, IpcMsg::empty().word(0, manager().0), 10000).unwrap();
    assert_eq!(result.label, d::OK);
    pass3("ordinary scoped activation without discovery lookup invocation or management");
    empty();
    let mut low = u64::MAX;
    let mut high = 0;
    for cycle in 0..6 {
        let before = ipc::sysinfo();
        let before_free = (before.total_ram_kb - before.used_ram_kb) / 4;
        let resident_before = call_name(d::DEPENDENCY_METRICS, "d3-parent").words[3];
        let starts: [u64; 3] = core::array::from_fn(|i| call_name(d::METRICS, NAMES[i]).words[1]);
        GATE.store(0, Ordering::Release);
        DONE.store(0, Ordering::Release);
        let began = ipc::monotonic_millis();
        let mut handles: [Option<sunlight_libc::thread::JoinHandle>; 8] =
            core::array::from_fn(|_| None);
        for id in 0..8 {
            RESULTS[id].store(0, Ordering::Release);
            unsafe {
                handles[id] =
                    Some(sunlight_libc::thread::spawn(graph_worker, id as *mut u8).unwrap());
            }
        }
        GATE.store(1, Ordering::Release);
        while DONE.load(Ordering::Acquire) != 8 {
            assert!(ipc::monotonic_millis() - began < 8000);
            sleep(10);
        }
        let ready_ms = ipc::monotonic_millis() - began;
        for handle in handles.into_iter().flatten() {
            cleanup_thread(handle);
        }

        for id in [2, 4, 6] {
            assert_eq!(
                RESULTS[id].load(Ordering::Acquire),
                RESULTS[0].load(Ordering::Acquire)
            );
        }
        for id in [3, 5, 7] {
            assert_eq!(
                RESULTS[id].load(Ordering::Acquire),
                RESULTS[1].load(Ordering::Acquire)
            );
        }
        let parent = cap("d3-parent");
        direct(parent, IpcMsg::with_label(3).word(0, 1).word(1, 0));
        for i in 0..3 {
            assert_eq!(call_name(d::METRICS, NAMES[i]).words[1], starts[i] + 1);
        }
        let holds = call_name(d::DEPENDENCY_METRICS, "d3-parent");
        assert_eq!(holds.words[0], 3);
        assert_eq!(holds.words[1], 2);
        assert_eq!(holds.words[3], resident_before + 3);
        assert_eq!(call_name(d::DEPENDENCY_METRICS, "d2-fixture").words[2], 2);
        let accounts: [IpcMsg; 3] = core::array::from_fn(|i| call_name(d::IDLE_ACCOUNT, NAMES[i]));
        let warm_at = ipc::monotonic_millis();
        assert_eq!(call_name(d::ACTIVATE, "d3-parent").label, d::OK);
        let warm_ms = ipc::monotonic_millis() - warm_at;
        sleep(800); // exceeds every provider's idle grace
        for name in NAMES {
            assert_eq!(call_name(d::DISCOVER, name).words[0], 2);
        }
        let after_idle: [IpcMsg; 3] =
            core::array::from_fn(|i| call_name(d::IDLE_ACCOUNT, NAMES[i]));
        let cpu_ns: u64 = (0..3)
            .map(|i| after_idle[i].words[0] - accounts[i].words[0])
            .sum();
        assert!(cpu_ns < 10_000_000);
        let released = ipc::monotonic_millis();
        direct(parent, IpcMsg::with_label(3).word(0, 0).word(1, 0));
        wait("d3-parent", 6);
        assert_eq!(call_name(d::DEPENDENCY_METRICS, "d3-parent").words[1], 2);
        empty();
        let unload_ms = ipc::monotonic_millis() - released;
        let receipts: [IpcMsg; 3] = core::array::from_fn(|i| call_name(d::IDLE_METRICS, NAMES[i]));
        let private: u64 = receipts.iter().map(|r| r.words[1]).sum();
        let pts: u64 = receipts.iter().map(|r| r.words[2]).sum();
        let after = ipc::sysinfo();
        let after_free = (after.total_ram_kb - after.used_ram_kb) / 4;
        if cycle > 0 {
            low = low.min(after_free);
            high = high.max(after_free);
        }
        assert!(ipc::ipc_call_timeout(parent, IpcMsg::with_label(1), 100).is_err());
        let managed_after = call_name(d::DEPENDENCY_METRICS, "d3-parent").words[3];
        assert_eq!(managed_after, resident_before);
        d1_log::log(format_args!("[D3-MEM] cycle={} before_free={} running_free={} after_free={} private_frames={} page_tables={} shared_pages={} ready_ms={} warm_ms={} unload_ms={} idle_cpu_ns={} dependency_leases=3 managed_before={} managed_running={} managed_after={}", cycle,before_free,accounts[2].words[3],after_free,private,pts,accounts.iter().map(|r|r.words[2]).sum::<u64>(),ready_ms,warm_ms,unload_ms,cpu_ns,resident_before,holds.words[3],managed_after));
    }
    assert!(high - low <= 16, "graph cycles must not leak frames");
    pass3("concurrent shared transitive holds graceful release and stable cycles");
    // Dependency dies while parents are running: no stale Ready or adoption.
    assert_eq!(call_name(d::ACTIVATE, "d3-parent").label, d::OK);
    let parent = cap("d3-parent");
    let leaf = cap("d3-leaf");
    let provider = cap("d2-fixture");
    direct(parent, IpcMsg::with_label(3).word(0, 1).word(1, 0));
    direct(provider, IpcMsg::with_label(5));
    wait("d3-parent", 3);
    wait("d3-leaf", 3);
    assert_eq!(call_name(d::ACTIVATE, "d3-parent").label, d::ERROR);
    assert_eq!(call_name(d::DEPENDENCY_METRICS, "d3-parent").words[0], 3);
    direct(parent, IpcMsg::with_label(5));
    direct(leaf, IpcMsg::with_label(5));
    sleep(1200);
    assert_eq!(call_name(d::DEPENDENCY_METRICS, "d3-parent").words[0], 0);
    pass3("dependency crash quarantines parents until confirmed cleanup");
    // Crashed parent releases its own edges, and remaining transitive work drains.
    assert_eq!(call_name(d::ACTIVATE, "d3-parent").label, d::OK);
    let parent = cap("d3-parent");
    direct(parent, IpcMsg::with_label(5));
    wait("d3-leaf", 0);
    wait("d2-fixture", 0);
    assert_eq!(call_name(d::DEPENDENCY_METRICS, "d3-parent").words[0], 0);
    pass3("crashed dependent cannot permanently pin providers");
    // Partial activation: kill the provider while leaf local init is pending.
    sleep(1100);
    let leaf_starts = call_name(d::METRICS, "d3-leaf").words[1];
    PENDING_LABEL.store(0, Ordering::Release);
    let pending =
        unsafe { sunlight_libc::thread::spawn(pending_worker, core::ptr::null_mut()).unwrap() };
    let deadline = ipc::monotonic_millis() + 5000;
    while call_name(d::METRICS, "d3-leaf").words[1] == leaf_starts {
        assert!(ipc::monotonic_millis() < deadline);
        sleep(5);
    }
    assert_eq!(call_name(d::DISCOVER, "d3-parent").words[0], 1);
    direct(cap("d2-fixture"), IpcMsg::with_label(5));
    while PENDING_LABEL.load(Ordering::Acquire) == 0 {
        assert!(ipc::monotonic_millis() < deadline);
        sleep(10);
    }
    assert_eq!(PENDING_LABEL.load(Ordering::Acquire), d::ERROR);
    cleanup_thread(pending);
    sleep(1400);
    assert_eq!(call_name(d::DEPENDENCY_METRICS, "d3-parent").words[0], 0);
    pass3("dependency death during startup releases partial graph");
    // The activation-only client exits with a live activation waiter. The
    // admitted shared attempt finishes and the unused chain unloads normally.
    let requests = call_name(d::METRICS, "d3-parent").words[0];
    let client = spawn_client();
    assert!(
        ipc::ipc_call_timeout(client, IpcMsg::with_label(2).word(0, manager().0), 5000).is_err()
    );
    assert!(call_name(d::METRICS, "d3-parent").words[0] > requests);
    empty();
    pass3("client death during activation leaves no dependency holds");
    // Two independent consumers share C. A's completed shutdown cannot
    // release B's ownership of C.
    assert_eq!(call_name(d::ACTIVATE, "d3-parent").label, d::OK);
    let parent = cap("d3-parent");
    direct(parent, IpcMsg::with_label(3).word(0, 1).word(1, 0));
    assert_eq!(call_name(d::ACTIVATE, "d2-dependent").label, d::OK);
    direct(parent, IpcMsg::with_label(3).word(0, 0).word(1, 0));
    wait("d3-parent", 0);
    wait("d3-leaf", 0);
    assert_eq!(call_name(d::DISCOVER, "d2-fixture").words[0], 2);
    assert_eq!(call_name(d::DEPENDENCY_METRICS, "d2-fixture").words[2], 1);
    let _ = ipc::ipc_call_timeout(cap("d2-dependent"), IpcMsg::with_label(2), 1000);
    empty();
    pass3("shared provider survives one consumer shutdown");
    pass3("COMPLETE");
}
