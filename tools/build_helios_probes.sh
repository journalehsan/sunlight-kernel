#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/target/helios-probes"
mkdir -p "$out"
as --64 "$root/tools/helios-probes/linux-probe-all.S" -o "$out/linux-probe-all.o"
ld -nostdlib -static -e _start -Ttext=0x400000 \
    "$out/linux-probe-all.o" -o "$out/linux-probe-all"
"$root/tools/stamp_helios_elf.sh" "$out/linux-probe-all"
