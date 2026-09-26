#![cfg(feature = "host")]

use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use wiseowl_memorydb::activation::LocalActivationRecord;

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn launch(data: &std::path::Path, socket: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_wiseowl-memorydb"))
        .env("WISEOWL_MEMORYDB_DIR", data)
        .env("WISEOWL_MEMORYDB_SOCKET", socket)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_for_socket(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < deadline, "MemoryDB never became ready");
        thread::sleep(Duration::from_millis(10));
    }
}

fn local(data: &std::path::Path) -> LocalActivationRecord {
    let bytes = std::fs::read(data.join("IDENTITY/LOCAL")).unwrap();
    LocalActivationRecord::decode(&bytes).unwrap()
}

#[test]
fn separate_memorydb_processes_contend_and_dead_owner_can_be_replaced() {
    for _ in 0..8 {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("db");
        let socket_a = root.path().join("a.sock");
        let socket_b = root.path().join("b.sock");
        let socket_c = root.path().join("c.sock");
        let mut first = Running(launch(&data, &socket_a));
        wait_for_socket(&socket_a);
        let before = local(&data);

        let duplicate = launch(&data, &socket_b).wait_with_output().unwrap();
        assert!(duplicate.status.success());
        assert!(String::from_utf8_lossy(&duplicate.stderr).contains("writer authority unavailable"));
        assert!(!socket_b.exists(), "rejected process exposed writable readiness");
        assert_eq!(local(&data), before, "rejected writer changed LOCAL");

        first.0.kill().unwrap();
        first.0.wait().unwrap();
        let replacement = Running(launch(&data, &socket_c));
        wait_for_socket(&socket_c);
        let after = local(&data);
        assert_eq!(after.identity_id, before.identity_id);
        assert_eq!(after.installation_id, before.installation_id);
        assert_eq!(after.activation_id, before.activation_id);
        assert_eq!(after.activation_sequence, before.activation_sequence);
        drop(replacement);
    }
}
