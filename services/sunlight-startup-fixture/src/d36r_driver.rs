//! Native recovery-blocked slice after the unchanged D1/D2/D3 matrix.
use super::*;
use ipc::supervision as own;
fn log36(args: core::fmt::Arguments<'_>) {
    d1_log::log(args);
}
pub(super) fn request(cap: CapabilityToken, op: u64, selector: u64) -> IpcMsg {
    ipc::ipc_call_timeout(cap, IpcMsg::with_label(op).word(0, selector), 2000).unwrap()
}
pub(super) fn inventory(cap: CapabilityToken, pid: u64, generation: u64) -> [u64; 4] {
    let mut found = None;
    for index in 0..own::CAPACITY {
        let r = request(cap, own::RPC_RECORD, index as u64);
        if r.label == d::OK && r.words[1] == pid && r.words[2] == generation {
            assert!(found.is_none(), "duplicate ownership record");
            found = Some([r.words[0], r.words[1], r.words[2], r.words[3]]);
        }
    }
    found.expect("retained authoritative service record")
}
extern "C" fn register_competitor(_: *mut u8) -> *mut u8 {
    let ep = ipc::endpoint_create();
    assert!(!ipc::nameserver_register("sunlightd", ep));
    assert!(ipc::endpoint_destroy(ep));
    DONE.fetch_add(1, Ordering::Release);
    core::ptr::null_mut()
}
pub fn run() {
    let scenario = option_env!("D36R_SCENARIO").unwrap_or("after-ready");
    if matches!(scenario, "pre-ready" | "quiescing" | "receipt-release") {
        super::d36r_fault_driver::run(scenario);
        return;
    }

    assert!(
        own::query(own::STATUS, 0).is_none(),
        "client cannot read privileged ledger"
    );
    // Two simultaneous unclaimed registration attempts cannot mint a manager grant.
    DONE.store(0, Ordering::Release);
    let a = unsafe {
        sunlight_libc::thread::spawn(register_competitor, core::ptr::null_mut()).unwrap()
    };
    let b = unsafe {
        sunlight_libc::thread::spawn(register_competitor, core::ptr::null_mut()).unwrap()
    };
    let deadline = ipc::monotonic_millis() + 4000;
    while DONE.load(Ordering::Acquire) != 2 {
        assert!(ipc::monotonic_millis() < deadline);
        sleep(10);
    }
    for tid in [a.tid, b.tid] {
        while unsafe { sunlight_libc::sys::syscall1(156, tid as u64) } != 1 {
            assert!(ipc::monotonic_millis() < deadline);
            sleep(10);
        }
    }
    log36(format_args!(
        "[D36R-TEST] concurrent unauthorized registration rejected PASS"
    ));
    let resolution = rpc(d::request(d::ACTIVATE, "d3-parent").unwrap());
    assert_eq!(resolution.label, d::OK);
    let parent = ipc::nameserver_lookup_timeout("d3-parent", 500).unwrap();
    let leaf = ipc::nameserver_lookup_timeout("d3-leaf", 500).unwrap();
    let provider = ipc::nameserver_lookup_timeout("d2-fixture", 500).unwrap();
    direct(parent, IpcMsg::with_label(3).word(0, 1).word(1, 0));
    let holds = rpc(d::request(d::DEPENDENCY_METRICS, "d3-parent").unwrap());
    assert_eq!(holds.words[0], 3);
    let lease = rpc(IpcMsg::with_label(d::LEASE_ACQUIRE)
        .word(0, resolution.words[0])
        .word(1, resolution.words[1])
        .word(2, resolution.words[2])
        .word(3, 60000));
    assert_eq!(lease.label, d::OK);
    let account = rpc(d::request(d::IDLE_ACCOUNT, "d3-parent").unwrap());
    assert_eq!(account.label, d::OK);
    let original = inventory(manager(), resolution.words[1], resolution.words[2]);
    assert_eq!(original[3], 1);
    let mut graph = [original; 3];
    for (i, name) in ["d3-leaf", "d2-fixture"].iter().enumerate() {
        let r = rpc(d::request(d::ACTIVATE, name).unwrap());
        assert_eq!(r.label, d::OK);
        graph[i + 1] = inventory(manager(), r.words[1], r.words[2]);
    }
    let initial = request(manager(), own::RPC_ACCOUNT, 0);
    log36(format_args!("[D36R-MEM] before_free={} owned_before={} service_pid={} service_generation={} record={} leases_before=1 holds_before=3",initial.words[0],initial.words[1],original[1],original[2],original[0]));
    let mut current = manager();
    for epoch in 2..=if scenario == "registration-timeout" {
        2
    } else {
        4
    } {
        let began = ipc::monotonic_millis();
        assert_eq!(request(current, own::TEST_CRASH, 0).label, d::OK);
        let deadline = began + 10000;
        let next = loop {
            assert!(ipc::monotonic_millis() < deadline, "recovery timeout");
            if let Some(candidate) = ipc::nameserver_lookup_timeout("sunlightd", 200) {
                if candidate != current {
                    let s = request(candidate, own::RPC_STATUS, 0);
                    if s.label == d::OK && s.words[1] == epoch {
                        break candidate;
                    }
                }
            }
            sleep(20);
        };
        assert!(ipc::ipc_call_timeout(current, IpcMsg::with_label(d::HELLO), 100).is_err());
        let state = request(next, own::RPC_STATUS, 0);
        assert_eq!(state.words[0], own::BLOCKED);
        assert_eq!(state.words[3], epoch);
        assert_eq!(inventory(next, original[1], original[2]), original);
        for op in [d::ACTIVATE, d::READY, d::LEASE_RELEASE, 1, 2, 3] {
            let r = ipc::ipc_call_timeout(next, d::request(op, "d3-parent").unwrap(), 500).unwrap();
            assert_eq!(r.label, d::ERROR);
            assert_eq!(r.words[0], own::ERROR_BLOCKED);
        }
        let probe = ipc::ipc_call_timeout(
            next,
            IpcMsg::with_label(own::TEST_PROBE)
                .word(0, parent.0)
                .word(1, original[1])
                .word(2, original[2]),
            1000,
        )
        .unwrap();
        assert_eq!(probe.label, d::OK);
        // The surviving endpoint still names exactly the original service incarnation.
        assert_eq!(direct(parent, IpcMsg::with_label(1)).words[1], original[2]);
        let sample = request(next, own::RPC_ACCOUNT, 0);
        log36(format_args!("[D36R-CYCLE] epoch={} recovery_ms={} owned={} reaped={} free_frames={} old_service_instances=1 replacements=0 leases=unknown holds=unknown",epoch,ipc::monotonic_millis()-began,sample.words[1],sample.words[2],sample.words[0]));
        current = next;
    }
    // Explicit disposable service operation, not manager signaling or takeover.
    // No persistent writes; accepted work has finished before these direct calls.
    for endpoint in [parent, leaf, provider] {
        let _ = ipc::ipc_call_timeout(endpoint, IpcMsg::with_label(5), 1000);
    }
    let deadline = ipc::monotonic_millis() + 5000;
    let mut private_frames = 0;
    let mut page_tables = 0;
    let mut free_at_reap = 0;
    for record in graph {
        let receipt = loop {
            let r = request(current, own::RPC_DETAIL, record[0]);
            assert_eq!(r.label, d::OK);
            if r.words[0] == 1 {
                break r;
            }
            assert!(ipc::monotonic_millis() < deadline, "reap timeout");
            sleep(20);
        };
        assert!(receipt.words[1] > 0 && receipt.words[2] > 0);
        private_frames += receipt.words[1];
        page_tables += receipt.words[2];
        free_at_reap = receipt.words[3];
    }
    assert!(ipc::ipc_call_timeout(parent, IpcMsg::with_label(1), 100).is_err());
    assert_eq!(inventory(current, original[1], original[2]), original);
    let stale = request(current, own::RPC_DETAIL, u64::MAX);
    assert_eq!(stale.label, d::ERROR);
    let sample = request(current, own::RPC_ACCOUNT, 0);
    log36(format_args!("[D36R-REAP] record={} pid={} generation={} private_frames={} page_tables={} free_at_reap={} owned_unreaped={} retained_reaped={} free_now={} external_leases=unknown",original[0],original[1],original[2],private_frames,page_tables,free_at_reap,sample.words[1],sample.words[2],sample.words[0]));
    assert_eq!(request(current, own::TEST_CRASH, 0).label, d::OK);
    sleep(5200);
    assert!(ipc::nameserver_lookup_timeout("sunlightd", 500).is_none());
    log36(format_args!("[D36R-TEST] retained identity stale endpoint blocked activation exact reap and bounded restart PASS"));
    log36(format_args!("[D36R-TEST] COMPLETE PASS"));
}
