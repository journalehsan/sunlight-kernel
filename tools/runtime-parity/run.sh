#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 1 ]]; then
    echo "usage: $0 NEW-EVIDENCE-DIR" >&2
    exit 2
fi
mkdir -- "$1"
out=$(realpath -- "$1")
cd "$(dirname "$0")/../.."
exec > >(tee "$out/run.log") 2>&1
set -x
src=tools/runtime-parity
bash -x tools/build_native_cxx.sh > "$out/runtime-build.log" 2>&1
"${MUSL_CC:-musl-gcc}" -v -std=c11 -O2 -Wall -Wextra -Werror -static -fno-pie -no-pie -pthread \
    "-Wl,-Map,$out/musl.map" "$src/musl.c" -o "$out/musl.elf" > "$out/musl-link.log" 2>&1
bash tools/stamp_helios_elf.sh "$out/musl.elf"
python3 tools/native-c/cxx.py -v -O2 -Wall -Wextra -Werror \
    "-Wl,-Map,$out/native.map,--why-extract=$out/native-extract.tsv" \
    "$src/native.cpp" -o "$out/native.elf" > "$out/native-link.log" 2>&1
for fixture in musl native; do
    readelf -W -h -l -S -d -s "$out/$fixture.elf" > "$out/$fixture.readelf"
    objdump -d "$out/$fixture.elf" > "$out/$fixture.disassembly"
done
sha256sum "$out/"{musl,native}.elf target/native-c/sysroot/lib/libsunlight_{cxx,eh,native_c}.a > "$out/artifacts.sha256"
python3 "$src/verify.py" prepare "$out"
python3 tools/native-c/guest.py --cases "$out/cases.json" --mixed-abi --memory-mb 4096 \
    --cpus 4 --timeout 120 --out "$out/guest"
python3 "$src/verify.py" trace "$out"
