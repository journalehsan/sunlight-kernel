#!/usr/bin/env bash
# Theme SVG consumers use the same native std platform as Hayro.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
selected=0
for arg in "$@"; do
    if [[ "$arg" == -p || "$arg" == --package || "$arg" == --package=* ]]; then selected=1; fi
done
if [[ "$selected" == 0 ]]; then
    set -- -p sunlight-vortex-shell -p sunlight-files -p sunlight-dialogd -p sunlight-control-panel "$@"
fi
bash tools/build_native_std.sh build "$@" --features native-svg --release
