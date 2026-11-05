#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 ELF" >&2
    exit 2
fi

elf=$1
[[ -f "$elf" ]] || { echo "missing ELF: $elf" >&2; exit 1; }

# Keep the legacy Linux OSABI stamp and add the explicit marker used when a
# Linux toolchain emits ELFOSABI_NONE. EI_PAD (bytes 9..14) is reserved for
# OS-specific metadata and is outside the native Sunlight contract.
printf '\x03' | dd of="$elf" bs=1 seek=7 conv=notrunc status=none
printf 'HLNX01' | dd of="$elf" bs=1 seek=9 conv=notrunc status=none
