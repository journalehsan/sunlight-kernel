#!/usr/bin/env bash
set -euo pipefail
out=target/btop-14/final
printf '%s  %s\n' bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702 target/btop-01/btop | sha256sum -c
mkdir -p "$out"
cp target/btop-10/final/probe.elf "$out/"
sed 's@target/btop-08/final@target/btop-14/final@g' tools/btop-08/image.sh > "$out/base.sh"
SUNLIGHT_BTOP14_TRACE=1 SUNLIGHT_BTOP06_WAIT_TRACE=1 SUNLIGHT_HELIOS_SIG01_TRACE=1 bash "$out/base.sh"
