#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SOURCE="$ROOT/guest/palcycle/palcycle.asm"
OUTPUT="$ROOT/ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM"
NASM=${NASM:-nasm}

if ! command -v "$NASM" >/dev/null 2>&1; then
  echo "missing nasm; install NASM to rebuild PALCYCLE.COM" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUTPUT")"
"$NASM" -f bin -o "$OUTPUT" "$SOURCE"
