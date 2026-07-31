#!/usr/bin/env bash
set -euo pipefail

proof_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$proof_root/target"

rustc --edition 2021 --test "$proof_root/tools/time-proof.rs" -o "$proof_root/target/time-proof"
"$proof_root/target/time-proof" --test-threads=1

rustc --edition 2021 --test "$proof_root/tools/rtc-proof.rs" \
    -o "$proof_root/target/rtc-proof"
"$proof_root/target/rtc-proof" --test-threads=1

rustc --edition 2021 --test "$proof_root/tools/timekeeping-proof.rs" \
    -o "$proof_root/target/timekeeping-proof"
"$proof_root/target/timekeeping-proof" --test-threads=1

rustc --edition 2021 --test "$proof_root/tools/pit-proof.rs" \
    -o "$proof_root/target/pit-proof"
"$proof_root/target/pit-proof" --test-threads=1

rustc --edition 2021 --test "$proof_root/tools/vortex-shell-calendar-proof.rs" \
    -o "$proof_root/target/vortex-shell-calendar-proof"
"$proof_root/target/vortex-shell-calendar-proof" --test-threads=1

cargo test --manifest-path "$proof_root/Cargo.toml" \
    --target x86_64-unknown-linux-gnu \
    --package sunlight-tz

cargo test --manifest-path "$proof_root/Cargo.toml" \
    --target x86_64-unknown-linux-gnu \
    --package sunlight-vortex-shell \
    panel_formats_july_31_and_august_1_from_service_weekday
