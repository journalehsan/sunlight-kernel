#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
bash tools/build_native_std.sh build -p sunlight-js-cli --release "$@"
