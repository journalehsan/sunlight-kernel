#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/target/helios-probes"
mkdir -p "$out"

build_asm() {
    local name="$1"
    as --64 "$root/tools/helios-probes/${name}.S" -o "$out/${name}.o"
    ld -nostdlib -static -e _start -Ttext=0x400000 \
        "$out/${name}.o" -o "$out/${name}"
    "$root/tools/stamp_helios_elf.sh" "$out/${name}"
}

build_asm linux-probe-all
build_asm linux-probe-runtime

# Unmodified sbase echo(1) hosted by a tiny Linux syscall libc.
clang -nostdlib -static -ffreestanding -fno-pic -fno-pie -fno-stack-protector \
    -nostdinc -O2 \
    -I "$root/third_party/sbase/include" \
    -I "$root/third_party/sbase" \
    -c "$root/third_party/sbase/minilibc.c" -o "$out/sbase-minilibc.o"
clang -nostdlib -static -ffreestanding -fno-pic -fno-pie -fno-stack-protector \
    -nostdinc -O2 \
    -I "$root/third_party/sbase/include" \
    -I "$root/third_party/sbase" \
    -c "$root/third_party/sbase/echo.c" -o "$out/sbase-echo.o"
clang -nostdlib -static -ffreestanding -fno-pic -fno-pie -fno-stack-protector \
    -nostdinc -O2 \
    -I "$root/third_party/sbase/include" \
    -I "$root/third_party/sbase" \
    -c "$root/third_party/sbase/libutil/putword.c" -o "$out/sbase-putword.o"
clang -nostdlib -static -ffreestanding -fno-pic -fno-pie -fno-stack-protector \
    -nostdinc -O2 \
    -I "$root/third_party/sbase/include" \
    -I "$root/third_party/sbase" \
    -c "$root/third_party/sbase/libutil/fshut.c" -o "$out/sbase-fshut.o"
ld -nostdlib -static -e _start -Ttext=0x400000 \
    "$out/sbase-minilibc.o" "$out/sbase-echo.o" \
    "$out/sbase-putword.o" "$out/sbase-fshut.o" \
    -o "$out/sbase-echo"
"$root/tools/stamp_helios_elf.sh" "$out/sbase-echo"
