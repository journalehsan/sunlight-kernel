#!/usr/bin/env bash
# Rabbit is a native std application; the kernel remains freestanding.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
bash tools/build_native_std.sh build -p rappid-rabbit --release "$@"
