#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT="$ROOT/SunlightMines.sunapp/Program/SUNMINE.EXE"
# Also place a copy in the shell bundle's TESTS for direct launch regression.
TESTS_OUT="$ROOT/ChronosDosShell.sunapp/Program/TESTS/SUNMINE.EXE"
PPC8086=${PPC8086:-ppc8086}
RTL=${FPC_I8086_RTL:-}

mkdir -p "$(dirname "$OUT")" "$(dirname "$TESTS_OUT")"
if command -v "$PPC8086" >/dev/null 2>&1 \
  && [[ -n "$RTL" && -f "$RTL/system.ppu" ]]; then
  "$PPC8086" -Tmsdos -Pi8086 -Mtp -WmLarge -Wh -Xs -O1 \
    -Fu"$RTL" -FD/usr/bin -XP -o"$OUT" "$ROOT/guest/sunmine/sunmine.pas"
  rm -f "$(dirname "$OUT")/sunmine.a" "$(dirname "$OUT")/sunmine.o" || true
  echo "Built SUNMINE.EXE from Free Pascal sources."
elif command -v nasm >/dev/null 2>&1; then
  nasm -f bin "$ROOT/guest/sunmine/sunmine.asm" -o "$OUT"
  echo "Built SUNMINE.EXE from the NASM fallback source."
else
  echo "missing both a configured ppc8086 toolchain and nasm" >&2
  exit 1
fi
cp -f "$OUT" "$TESTS_OUT"
