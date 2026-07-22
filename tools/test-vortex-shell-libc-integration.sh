#!/usr/bin/env bash
set -euo pipefail

# Focused dependency/configuration proof for the native Vortex Shell.  Keep
# this separate from host test binaries: the full Sunlight libc exports native
# ABI symbols such as clock_gettime and must not be linked into host runtimes.

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

shell_manifest="services/sunlight-vortex-shell/Cargo.toml"
libc_manifest="sunlight-libc/Cargo.toml"

grep -Fq 'sunlight-libc = { path = "../../sunlight-libc", features = ["global-alloc"] }' "$shell_manifest"
grep -Fq 'default = ["dynamic-heap"]' "$shell_manifest"
grep -Fq 'dynamic-heap = ["sunlight-libc/dynamic-heap-8m"]' "$shell_manifest"

# A path package appears once in Cargo.lock.  This catches a stale registry
# package without rewriting the lockfile or broadly updating dependencies.
if [[ "$(awk '/^name = "sunlight-libc"$/ { count += 1 } END { print count + 0 }' Cargo.lock)" != "1" ]]; then
    echo "expected exactly one sunlight-libc package in Cargo.lock" >&2
    exit 1
fi

dynamic_tree="$(cargo tree -p sunlight-vortex-shell -e features -i sunlight-libc)"
printf '%s\n' "$dynamic_tree" | grep -Fq "sunlight-libc v$(awk -F'"' '/^version =/ { print $2; exit }' "$libc_manifest") ($root/sunlight-libc)"
printf '%s\n' "$dynamic_tree" | grep -Fq 'sunlight-libc feature "global-alloc"'
printf '%s\n' "$dynamic_tree" | grep -Fq 'sunlight-libc feature "dynamic-heap-8m"'

# The normal build keeps the mmap-backed 8 MiB shell heap.  The same shell can
# also be checked against the static heap for allocator-path coverage; stress
# remains opt-in and does not change production builds.
cargo check -p sunlight-vortex-shell
cargo check -p sunlight-vortex-shell --no-default-features --features stress

static_tree="$(cargo tree -p sunlight-vortex-shell --no-default-features --features stress -e features -i sunlight-libc)"
printf '%s\n' "$static_tree" | grep -Fq 'sunlight-libc feature "global-alloc"'
if printf '%s\n' "$static_tree" | grep -Fq 'sunlight-libc feature "dynamic-heap"'; then
    echo "static Vortex Shell proof unexpectedly enables the dynamic libc heap" >&2
    exit 1
fi

bash "$root/tools/test-alloc-proof.sh"
bash "$root/tools/test-time-proof.sh"

echo "[vortex-libc-proof] native dependency and static/dynamic allocator checks passed"
