#!/usr/bin/env bash
# Opt-in cooperative idle experiment; production manifests retain their policy.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target/d2-evidence
python3 - <<'PY'
import hashlib, json, pathlib, subprocess
paths = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard"], text=True).splitlines()
manifest = {p: hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest() for p in paths
            if pathlib.Path(p).is_file() and (p.endswith((".rs", ".toml", ".ld", ".py", ".sh")) or p == "rust-toolchain.toml")}
pathlib.Path("target/d2-evidence/source-sha256.json").write_text(json.dumps(manifest, indent=2) + "\n")
versions = {name: subprocess.check_output(command, text=True) for name, command in
            [("rustc", ["rustc", "-vV"]), ("qemu", ["qemu-system-x86_64", "--version"])]}
pathlib.Path("target/d2-evidence/tool-versions.json").write_text(json.dumps(versions, indent=2) + "\n")
PY
RUSTFLAGS='' cargo test -p sunlightd --lib --target x86_64-unknown-linux-gnu > target/d2-evidence/host.log 2>&1
RUSTFLAGS='' cargo test -p sunlightd --bin sunlightd --features idle-test --target x86_64-unknown-linux-gnu > target/d2-evidence/host-runtime.log 2>&1
RUSTFLAGS='' cargo test -p sunlight-ipc --lib --target x86_64-unknown-linux-gnu > target/d2-evidence/ipc-host.log 2>&1
RUSTFLAGS='' cargo test -p sunlight-kernel --lib --target x86_64-unknown-linux-gnu > target/d2-evidence/memory-host.log 2>&1
python3 tools/test_scheduler_host.py > target/d2-evidence/scheduler-host.log 2>&1
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
    cargo build -p sunlightd -p sunlight-startup-fixture --features sunlightd/idle-test,sunlight-startup-fixture/idle-test --release > target/d2-evidence/userspace-build.log 2>&1
cargo build -p sunlight-kernel --features idle_test > target/d2-evidence/kernel-build.log 2>&1
tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel target/sunlightd-d2.iso > target/d2-evidence/iso-build.log 2>&1
python3 tools/sunlightd_d1_gate.py --d2 --iso target/sunlightd-d2.iso --evidence target/d2-evidence "$@"
