#!/usr/bin/env bash
set -euo pipefail

proof_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
proof_binary="$proof_root/target/alloc-proof"

rustc --test "$proof_root/tools/alloc-proof.rs" \
    --cfg 'feature="global-alloc"' \
    -o "$proof_binary"
"$proof_binary" --test-threads=1
