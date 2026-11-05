#!/usr/bin/env bash
# Focused opt-in D1 experiment. Does not build or activate production services differently.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target/d1-evidence
RUSTFLAGS='' cargo test -p sunlightd --lib --target x86_64-unknown-linux-gnu > target/d1-evidence/host.log 2>&1
RUSTFLAGS='' cargo test -p sunlightd --bin sunlightd --features demand-test --target x86_64-unknown-linux-gnu > target/d1-evidence/host-runtime.log 2>&1
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
    cargo build -p sunlightd -p sunlight-startup-fixture --features sunlightd/demand-test,sunlight-startup-fixture/demand-test --release
cargo build -p sunlight-kernel --features demand_test
tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel target/sunlightd-d1.iso
python3 tools/sunlightd_d1_gate.py "$@"
