#!/usr/bin/env bash
set -euo pipefail

proof_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
proof_binary="$proof_root/target/mem-string-proof"

mkdir -p "$proof_root/target"
rustc --test "$proof_root/tools/mem-string-proof.rs" -o "$proof_binary"
"$proof_binary" --test-threads=1
