#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
out=target/helios-sig01
mkdir -p "$out"
sha256sum --check target/btop-04/final.sha256
musl-gcc -O2 -Wall -Wextra -Werror -static -fno-pie -no-pie -pthread \
    "-Wl,-Map,$out/sig01.map" tools/helios-sig01/{roundtrip.c,registers.S} -o "$out/sig01.elf"
objdump -d "$out/sig01.elf" > "$out/sig01.disassembly"
cp "$out/sig01.elf" "$out/sig01-guest.elf"
bash tools/stamp_helios_elf.sh "$out/sig01-guest.elf"
timeout 8 "$out/sig01.elf" | tee "$out/linux.log"
sed 's|out="\$PWD/target/btop-04"|out="$PWD/target/helios-sig01"|; s|target/btop-04/vfs-config-fixture.elf|target/helios-sig01/vfs-config-fixture.elf|; /RamEntry::dir("\/tmp\/btopfixture"/a\    RamEntry::file("/bin/sig01", 0, 0, mode::FILE_755, include_bytes!("../../target/helios-sig01/sig01-guest.elf")),\n    RamEntry::file("/bin/tty-select", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-03/tty-select.elf")),' \
    target/btop-04/image.sh > "$out/image.sh"
SUNLIGHT_HELIOS_SIG01_TRACE=1 bash "$out/image.sh" trace
sha256sum target/btop-01/btop "$out/"{sig01.elf,sig01-guest.elf,kernel-ramfs-trace-config.elf,guest-ramfs-trace-config.iso} > "$out/artifacts.sha256"
