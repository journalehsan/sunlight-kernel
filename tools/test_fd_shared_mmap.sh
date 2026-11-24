#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mode=${1:-mmap}
extra_flags=()
case "$mode" in
    mmap) out="$PWD/target/fd-shared-mmap" ;;
    scm-rights)
        out="$PWD/target/native-scm-rights"
        extra_flags=(-DSCM_FIXTURE)
        ;;
    pool)
        out="$PWD/target/fd-shm-pool"
        extra_flags=(-DPOOL_FIXTURE)
        ;;
    *) echo "Unknown fixture: $mode" >&2; exit 2 ;;
esac
out=${SUNLIGHT_TEST_OUTPUT:-"$out"}
mkdir -p "$out"
source_file=tools/tests/fd-shared-mmap.c
flags=(-Wall -Wextra -Werror -Os -static -nostdlib -fno-pie -no-pie -ffreestanding -fno-stack-protector -Wl,--build-id=none)
"${CC:-cc}" "${flags[@]}" "${extra_flags[@]}" -DNATIVE "$source_file" -o "$out/server.elf"
"${CC:-cc}" "${flags[@]}" "${extra_flags[@]}" "$source_file" -o "$out/client.elf"
bash tools/stamp_helios_elf.sh "$out/client.elf"

SUNLIGHT_FD_SHARED_SERVER="$out/server.elf" SUNLIGHT_FD_SHARED_CLIENT="$out/client.elf" \
    cargo build --package sunlight-kernel --features fd_shared_mmap_test 2>&1 | tee "$out/build.log"
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
    "$out/guest.iso" 2>&1 | tee "$out/pack.log"

serial="$out/serial.log"
if [[ "$mode" == pool ]]; then
    serial="$out/serial-${SUNLIGHT_TEST_ACCEL:-tcg}.log"
fi
: > "$serial"
qemu_log="$out/qemu.log"
if [[ "$mode" == pool ]]; then qemu_log="$out/qemu-${SUNLIGHT_TEST_ACCEL:-tcg}.log"; fi
qemu-system-x86_64 -accel "${SUNLIGHT_TEST_ACCEL:-tcg}" -m 2048 -smp 2 \
    -device virtio-rng-pci,disable-modern=on \
    -cdrom "$out/guest.iso" -display none -monitor none -serial "file:$serial" \
    -no-reboot > "$qemu_log" 2>&1 &
guest=$!
trap 'kill "$guest" 2>/dev/null || true; wait "$guest" 2>/dev/null || true' EXIT
for ((attempt = 0; attempt < 120; ++attempt)); do
    if rg -qF '[FD-SHARED] FAIL' "$serial" || ! kill -0 "$guest" 2>/dev/null; then break; fi
    if [[ "$mode" == pool ]] && rg -qF '[MEMFD-TEST] PASS' "$serial"; then
        awk '
            /\[MEMFD\] freed/ && !released {
                if ($4 == "pages=2" && $6 == "live_pages=1") normal_large++
                if ($4 == "pages=1" && $6 == "live_pages=0") normal_small++
            }
            /independent mappings released/ { released=1 }
            released && /dropped queued reference on receiver close/ { closed=1 }
            closed && /\[MEMFD\] freed/ && /pages=2 live_objects=0 live_pages=0/ { discard++ }
            END { exit !(normal_large == 1 && normal_small == 1 && discard == 1) }
        ' "$serial" || break
        rg 'MEMFD|SCM-RIGHTS' "$serial"
        exit 0
    fi
    if [[ "$mode" == scm-rights ]] && rg -qF '[SCM-RIGHTS] PASS' "$serial" &&
        rg -qF '[SCM-RIGHTS] dropped queued reference on receiver close' "$serial"; then
        rg 'FD-SHARED|SCM-RIGHTS' "$serial"
        exit 0
    fi
    if rg -qF '[FD-SHARED] PASS native read after Helios exit' "$serial"; then
        native=$(sed -n 's/.*\[FD-SHARED\] pid=711 fd=[0-9]* object=shared-page frame=\(0x[0-9a-f]*\) va=.*/\1/p' "$serial")
        helios=$(sed -n 's/.*\[FD-SHARED\] pid=712 fd=[0-9]* object=shared-page frame=\(0x[0-9a-f]*\) va=.*/\1/p' "$serial")
        if [[ -n "$native" && "$native" == "$helios" ]]; then
            rg -F '[FD-SHARED]' "$serial"
            exit 0
        fi
        break
    fi
    sleep 1
done
rg 'FD-SHARED|SCM-RIGHTS|MEMFD' "$serial" || true
echo "Guest fixture failed; see $serial and $qemu_log" >&2
exit 1
