#!/bin/bash
set -o pipefail
d=${DN22_OUT:-docs/audit/chronos-dn/dn-22}
p=target/x86_64-unknown-linux-gnu/debug/examples/dn22_probe
name=$1 input=$2 keys=$3 budget=${4:-7000000}
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=5000000 DN21_KEYS="$keys" DN21_BUDGET="$budget" CHRONOS_TEXT_CAPTURE=/tmp/dn22/"$name".ppm "$p" target/dos-navigator/dn151/"$input" | gzip -n >"$d/$name.log.gz"
r=$?
printf 'probe=%s\n' "$r" >"$d/$name-status.log"
if [[ -f /tmp/dn22/"$name".ppm ]]; then magick /tmp/dn22/"$name".ppm "$d/$name.png"; fi
zcat "$d/$name.log.gz" | tail -28
