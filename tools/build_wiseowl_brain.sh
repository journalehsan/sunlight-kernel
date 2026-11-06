#!/usr/bin/env bash
# Native std owns startup, TLS, allocation, and panic/exit for the brain service.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
bash tools/build_native_std.sh build -p wiseowl-brain \
    --bin wiseowl-braind --bin wiseowl-brainctl \
    --no-default-features --features sunlightos --release "$@"
