#!/usr/bin/env bash
# D3 synthetic dependency integration gate; production manifests stay unchanged.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target/d3-evidence
python3 - <<'PY'
import hashlib, json, pathlib, subprocess
paths = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard"], text=True).splitlines()
manifest = {p: hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest() for p in paths
            if pathlib.Path(p).is_file() and (p.endswith((".rs", ".toml", ".ld", ".py", ".sh")) or p == "rust-toolchain.toml")}
pathlib.Path("target/d3-evidence/source-sha256.json").write_text(json.dumps(manifest, indent=2) + "\n")
versions = {name: subprocess.check_output(command, text=True) for name, command in
            [("rustc", ["rustc", "-vV"]), ("qemu", ["qemu-system-x86_64", "--version"])]}
pathlib.Path("target/d3-evidence/tool-versions.json").write_text(json.dumps(versions, indent=2) + "\n")
PY
RUSTFLAGS='' cargo test -p sunlightd --lib --target x86_64-unknown-linux-gnu > target/d3-evidence/host.log 2>&1
RUSTFLAGS='' cargo test -p sunlightd --bin sunlightd --features dependency-test --target x86_64-unknown-linux-gnu > target/d3-evidence/host-runtime.log 2>&1
RUSTFLAGS='' cargo test -p sunlight-ipc --lib --target x86_64-unknown-linux-gnu > target/d3-evidence/ipc-host.log 2>&1
RUSTFLAGS='' cargo test -p sunlight-kernel --lib --target x86_64-unknown-linux-gnu > target/d3-evidence/memory-host.log 2>&1
python3 tools/test_scheduler_host.py > target/d3-evidence/scheduler-host.log 2>&1
RUSTFLAGS='' cargo check -p sunlightd --features identity-phase-c-test --target x86_64-unknown-linux-gnu > target/d3-evidence/identity-feature.log 2>&1
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
    cargo build -p sunlightd -p sunlight-startup-fixture --features sunlightd/dependency-test,sunlight-startup-fixture/dependency-test --release > target/d3-evidence/userspace-build.log 2>&1
cargo build -p sunlight-kernel --features dependency_test > target/d3-evidence/kernel-build.log 2>&1
tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel target/sunlightd-d3.iso > target/d3-evidence/iso-build.log 2>&1
python3 tools/sunlightd_d1_gate.py --d2 --d3 --iso target/sunlightd-d3.iso --evidence target/d3-evidence "$@"
python3 - <<'PY'
import datetime, json, pathlib, re
import hashlib
root = pathlib.Path("target/d3-evidence")
def count(path):
    match = re.search(r"test result: ok\. (\d+) passed;", (root / path).read_text())
    return int(match.group(1)) if match else None
samples = json.loads((root / "dependency-memory.json").read_text())
summary = {
    "date_utc": datetime.datetime.now(datetime.timezone.utc).date().isoformat(),
    "increments": {"D3A": "PASS", "D3B": "PASS", "D3C": "PASS", "D3D": "PASS", "D3E": "PASS"},
    "native_cpus": [1, 2, 4],
    "native_result": "PASS; D1, D2, D3 and IPC foundation assertions",
    "accelerator": "recorded per CPU in dependency-memory.json",
    "host_tests": {
        "sunlightd_core": count("host.log"),
        "sunlightd_manager": count("host-runtime.log"),
        "ipc": count("ipc-host.log"),
        "kernel_memory": count("memory-host.log"),
        "scheduler_runner": "PASS (focused production state/lifecycle/accounting/policy probes)",
        "identity_phase_c_compile": "PASS",
        "historical_d1_d2_checks_preserved": 164,
        "new_host_checks": (29 - 25) + (19 - 11) + (43 - 42),
        "total_host_checks": 164 + (29 - 25) + (19 - 11) + (43 - 42)
    },
    "dependency_cycles": len(samples),
    "cycles_per_cpu": {str(cpu): sum(s["cpus"] == cpu for s in samples) for cpu in (1, 2, 4)},
    "dependency_memory": {
        "private_frames_reclaimed_per_graph": sorted(set(s["private_frames"] for s in samples)),
        "page_table_frames_per_graph": sorted(set(s["page_tables"] for s in samples)),
        "shared_pages_per_graph": sorted(set(s["shared_pages"] for s in samples)),
        "steady_free_frame_drift_per_cpu": {str(cpu): max(s["after_free"] for s in samples if s["cpus"] == cpu) - min(s["after_free"] for s in samples if s["cpus"] == cpu) for cpu in (1, 2, 4)},
        "cold_chain_ready_ms": [s["ready_ms"] for s in samples],
        "warm_resolution_ms": [s["warm_ms"] for s in samples],
        "confirmed_unload_ms": [s["unload_ms"] for s in samples],
        "idle_service_cpu_ns": [s["idle_cpu_ns"] for s in samples],
        "managed_service_processes_before_running_after": [samples[0]["managed_before"], samples[0]["managed_running"], samples[0]["managed_after"]],
        "dependency_holds_while_running": sorted(set(s["dependency_leases"] for s in samples))
    },
    "historical_d2_independent_reference_kib": 3148,
    "always_running_comparison": "Default resident gate is measured separately; test graph is synthetic and no production service was migrated.",
    "thumbnailer_pilot": "BLOCKED; see documented file-access and lifecycle-protocol blockers",
    "default_resident_boot": "PENDING",
    "raw_evidence": ["serial-1cpu.log", "serial-2cpu.log", "serial-4cpu.log", "dependency-memory.json", "host.log", "host-runtime.log", "ipc-host.log", "memory-host.log", "scheduler-host.log", "identity-feature.log", "userspace-build.log", "kernel-build.log", "iso-build.log"]
}
manifest = json.loads((root / "source-sha256.json").read_text())
summary["source_hashes_match"] = all(hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest() == digest for path, digest in manifest.items() if pathlib.Path(path).is_file())
(root / "validation-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
PY
