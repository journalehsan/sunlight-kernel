#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
out="$PWD/target/btop-03"
mkdir -p "$out"
printf '%s  %s\n' bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702 target/btop-01/btop | sha256sum -c -
docker run --rm --network host -v "$PWD:/work" -w /work \
    docker.arvancloud.ir/alpine@sha256:5b10f432ef3da1b8d4c7eb6c487f2f5a8f096bc91145e68878dd4a5019afde11 sh -ec '
apk add --no-cache gcc=15.2.0-r2 musl-dev=1.2.5-r24
apk info -v | sort
for probe in ifaddrs tty-select; do
    gcc -O2 -Wall -Wextra -Werror -static -fno-pie -no-pie "tools/btop-03/$probe.c" -o "target/btop-03/$probe.elf"
    chmod a+w "target/btop-03/$probe.elf"
done
' > "$out/toolchain.log" 2>&1
for probe in ifaddrs tty-select; do
    bash tools/stamp_helios_elf.sh "$out/$probe.elf"
done
# Use the retained packaging recipe without changing the BTop-02 artifacts.
sed -e 's|out="\$PWD/target/btop-02"|out="$PWD/target/btop-03"|' \
    -e 's|target/btop-02/vfs-config-fixture.elf|target/btop-03/vfs-config-fixture.elf|g' \
    -e '/RamEntry::file("\/bin\/proc-native"/i\    RamEntry::file("/bin/ifaddrs", 0, 0, mode::FILE_755,\n        include_bytes!("../../target/btop-03/ifaddrs.elf")),\n    RamEntry::file("/bin/tty-select", 0, 0, mode::FILE_755,\n        include_bytes!("../../target/btop-03/tty-select.elf")),' \
    -e 's~41 | 99 | 217)~41 | 44 | 45 | 46 | 47 | 99 | 217 | 270)~' \
    target/btop-02/image.sh > "$out/image.sh"
bash "$out/image.sh" trace
sha256sum target/btop-01/btop "$out/"{ifaddrs.elf,tty-select.elf,kernel-ramfs-trace-config.elf,guest-ramfs-trace-config.iso} > "$out/final.sha256"
