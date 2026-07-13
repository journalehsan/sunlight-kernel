#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT="$ROOT/ChronosDosShell.sunapp/Program/SUNSH.EXE"
PPC8086=${PPC8086:-ppc8086}
RTL=${FPC_I8086_RTL:-}

if ! command -v "$PPC8086" >/dev/null 2>&1; then
  echo "missing ppc8086; install or build Free Pascal 3.2.2 with the i8086-msdos target" >&2
  exit 1
fi
if [[ -z "$RTL" || ! -f "$RTL/system.ppu" ]]; then
  echo "set FPC_I8086_RTL to the Free Pascal i8086-msdos RTL unit directory" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUT")"
"$PPC8086" -Tmsdos -Pi8086 -Mtp -WmLarge -Wh -Xs -O1 \
  -Fu"$RTL" -FD/usr/bin -XP -o"$OUT" "$ROOT/guest/sunshell/sunshell.pas"
rm -f "$(dirname "$OUT")/sunshell.a"
