#!/usr/bin/env bash
# Check the full published Hayro graph against the pinned native std overlay.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
bash tools/build_native_std.sh check \
    --manifest-path tools/hayro-native-probe/Cargo.toml \
    --features full-render --release "$@"
