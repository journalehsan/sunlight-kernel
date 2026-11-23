#!/usr/bin/env bash
# Requires the retained artifacts and generated recipe from BTop-04.
set -euo pipefail
cd "$(dirname "$0")/../.."
out=target/btop-05
mkdir -p "$out"
sha256sum --check target/btop-04/final.sha256
musl-gcc -O2 -Wall -Wextra -Werror -static -fno-pie -no-pie -pthread \
    tools/btop-05/masked-select.c -o "$out/masked-select.elf"
cp "$out/masked-select.elf" "$out/masked-select-guest.elf"
bash tools/stamp_helios_elf.sh "$out/masked-select-guest.elf"
cp target/btop-03/tty-select.elf "$out/tty-select.elf"
sed 's|out="\$PWD/target/btop-04"|out="$PWD/target/btop-05"|; s|target/btop-04/vfs-config-fixture.elf|target/btop-05/vfs-config-fixture.elf|; /RamEntry::dir("\/tmp\/btopfixture"/a\    RamEntry::file("/bin/masked-select", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-05/masked-select-guest.elf")),\n    RamEntry::file("/bin/tty-select", 0, 0, mode::FILE_755, include_bytes!("../../target/btop-05/tty-select.elf")),' \
    target/btop-04/image.sh > "$out/image.sh"
bash "$out/image.sh" trace
sha256sum target/btop-01/btop "$out/"{masked-select.elf,masked-select-guest.elf,tty-select.elf,kernel-ramfs-trace-config.elf,guest-ramfs-trace-config.iso} > "$out/artifacts.sha256"
