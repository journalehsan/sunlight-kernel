#!/usr/bin/env bash
set -euo pipefail

proof_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$proof_root/target"

rustc --edition 2021 --test "$proof_root/tools/time-proof.rs" -o "$proof_root/target/time-proof"
"$proof_root/target/time-proof" --test-threads=1

rustc --edition 2021 --test "$proof_root/tools/vortex-shell-calendar-proof.rs" \
    -o "$proof_root/target/vortex-shell-calendar-proof"
"$proof_root/target/vortex-shell-calendar-proof" --test-threads=1
