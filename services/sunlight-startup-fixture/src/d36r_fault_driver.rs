//! Phase-specific supervisor loss. A new manager remains diagnostic in every case.
use super::d36r_driver::{inventory, request};
use super::*;
use ipc::supervision as own;
fn by_path(manager: CapabilityToken, path: &str) -> [u64; 4] {
    let expected = d::request(0, path).unwrap();
    let mut found = None;
    for i in 0..own::CAPACITY {
        let r = request(manager, own::RPC_RECORD, i as u64);
        if r.label != d::OK {
            continue;
        }
        let p = request(manager, own::RPC_PATH, r.words[0]);
        assert_eq!(p.label, d::OK);
        if p.words[..4] == expected.words[..4] {
            assert!(found.is_none(), "duplicate managed path");
            found = Some([r.words[0], r.words[1], r.words[2], r.words[3]]);
        }
    }
    found.expect("spawn-owned path record")
}
fn wait_next(old: CapabilityToken, epoch: u64) -> CapabilityToken {
    let deadline = ipc::monotonic_millis() + 10000;
    loop {
        assert!(
            ipc::monotonic_millis() < deadline,
            "diagnostic manager timeout"
        );
        if let Some(cap) = ipc::nameserver_lookup_timeout("sunlightd", 200) {
            if cap != old {
                let r = request(cap, own::RPC_STATUS, 0);
                if r.label == d::OK && r.words[1] == epoch {
                    assert_eq!(r.words[0], own::BLOCKED);
                    return cap;
                }
            }
        }
        sleep(20);
    }
}
pub fn run(scenario: &str) {
    let stage = match scenario {
        "pre-ready" => 1,
        "quiescing" => 2,
        "receipt-release" => 3,
        _ => unreachable!(),
    };
    let old = manager();
    if stage == 1 {
        assert_eq!(request(old, own::TEST_ARM, stage).label, d::OK);
        assert!(
            ipc::ipc_call_timeout(old, d::request(d::ACTIVATE, "d3-parent").unwrap(), 8000)
                .is_err()
        );
    } else {
        let r = rpc(d::request(d::ACTIVATE, "d3-parent").unwrap());
        assert_eq!(r.label, d::OK);
        let parent = ipc::nameserver_lookup_timeout("d3-parent", 500).unwrap();
        direct(parent, IpcMsg::with_label(3).word(0, 1));
        let lease = rpc(IpcMsg::with_label(d::LEASE_ACQUIRE)
            .word(0, r.words[0])
            .word(1, r.words[1])
            .word(2, r.words[2])
            .word(3, 60000));
        assert_eq!(lease.label, d::OK);
        assert_eq!(
            rpc(d::request(d::DEPENDENCY_METRICS, "d3-parent").unwrap()).words[0],
            3
        );
        assert_eq!(request(old, own::TEST_ARM, stage).label, d::OK);
        assert_eq!(
            rpc(IpcMsg::with_label(d::LEASE_RELEASE)
                .word(0, lease.words[0])
                .word(1, r.words[0]))
            .label,
            d::OK
        );
        direct(parent, IpcMsg::with_label(3).word(0, 0));
    }
    let mut current = wait_next(old, 2);
    assert!(ipc::ipc_call_timeout(old, IpcMsg::with_label(d::HELLO), 100).is_err());
    let records = [
        by_path(current, "/sbin/d3-parent"),
        by_path(current, "/sbin/d3-leaf"),
        by_path(current, "/sbin/d2-fixture"),
    ];
    assert!(records.iter().all(|r| r[3] == 1));
    for epoch in 2..=4 {
        if epoch > 2 {
            let previous = current;
            assert_eq!(request(previous, own::TEST_CRASH, 0).label, d::OK);
            current = wait_next(previous, epoch);
            assert!(ipc::ipc_call_timeout(previous, IpcMsg::with_label(d::READY), 100).is_err());
        }
        for r in records {
            assert_eq!(inventory(current, r[1], r[2]), r);
        }
        for op in [d::ACTIVATE, d::READY, d::LEASE_RELEASE, 1, 2, 3] {
            let reply =
                ipc::ipc_call_timeout(current, d::request(op, "d3-parent").unwrap(), 500).unwrap();
            assert_eq!(reply.label, d::ERROR);
            assert_eq!(reply.words[0], own::ERROR_BLOCKED);
        }
        let detail = request(current, own::RPC_DETAIL, records[0][0]);
        if stage == 2 {
            assert_eq!(detail.words[0], 0);
        } else {
            assert_eq!(detail.words[0], 1);
        }
        let sample = request(current, own::RPC_ACCOUNT, 0);
        d1_log::log(format_args!("[D36R-CYCLE] epoch={} observed_ms={} stage={} owned={} reaped={} free_frames={} old_service_instances={} snapshot_rows=3 replacements=0 leases=unknown holds=unknown",epoch,ipc::monotonic_millis(),stage,sample.words[1],sample.words[2],sample.words[0],u64::from(detail.words[0]==0)));
    }
    // Only disposable fixture operations; the successor issues no control IPC.
    for name in ["d3-parent", "d3-leaf", "d2-fixture"] {
        if let Some(cap) = ipc::nameserver_lookup_timeout(name, 100) {
            let _ = ipc::ipc_call_timeout(cap, IpcMsg::with_label(5), 1000);
        }
    }
    let deadline = ipc::monotonic_millis() + 5000;
    let mut private = 0;
    let mut pts = 0;
    let mut free = 0;
    for r in records {
        let detail = loop {
            let d = request(current, own::RPC_DETAIL, r[0]);
            assert_eq!(d.label, d::OK);
            if d.words[0] == 1 {
                break d;
            }
            assert!(ipc::monotonic_millis() < deadline, "graph reap timeout");
            sleep(20);
        };
        private += detail.words[1];
        pts += detail.words[2];
        free = detail.words[3];
        assert_eq!(inventory(current, r[1], r[2]), r);
    }
    assert!(private > 0 && pts > 0);
    let sample = request(current, own::RPC_ACCOUNT, 0);
    d1_log::log(format_args!("[D36R-REAP] stage={} service_count=3 private_frames={} page_tables={} free_at_reap={} owned_unreaped={} retained_reaped={} free_now={} external_leases=unknown",stage,private,pts,free,sample.words[1],sample.words[2],sample.words[0]));
    assert_eq!(request(current, own::TEST_CRASH, 0).label, d::OK);
    sleep(5200);
    assert!(ipc::nameserver_lookup_timeout("sunlightd", 500).is_none());
    d1_log::log(format_args!(
        "[D36R-TEST] phase {} blocked without replacement PASS",
        scenario
    ));
    d1_log::log(format_args!("[D36R-TEST] COMPLETE PASS"));
}
