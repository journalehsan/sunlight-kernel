#!/usr/bin/env bash
set -euo pipefail
out=target/btop-09/final
mkdir -p "$out"
printf '%s  %s\n' bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702 target/btop-01/btop | sha256sum -c
sed -e 's@target/btop-08/final@target/btop-09/final@g' \
    -e 's@tools/btop-07/image.sh@target/btop-09/final/base-source.sh@g' \
    tools/btop-08/image.sh > "$out/image.sh"
sed -e 's@b == b"/mnt"@b == b"/mnt" || b == b"/mnt/BTOP09.BIN"@' \
    tools/btop-07/image.sh > "$out/base-source.sh"
bash "$out/image.sh"
