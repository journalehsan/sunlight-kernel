#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
out="$PWD/target/btop-04"
mkdir -p "$out"
printf '%s  %s\n' bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702 target/btop-01/btop | sha256sum -c -
if [[ ${1:-} != --reuse-probes ]]; then
docker run --rm --network host -v "$PWD:/work" -w /work \
    docker.arvancloud.ir/alpine@sha256:5b10f432ef3da1b8d4c7eb6c487f2f5a8f096bc91145e68878dd4a5019afde11 sh -ec '
apk add --no-cache gcc=15.2.0-r2 musl-dev=1.2.5-r24
apk info -v | sort
for probe in ifaddrs route-scope; do
    gcc -O2 -Wall -Wextra -Werror -static -fno-pie -no-pie "tools/btop-04/$probe.c" -o "target/btop-04/$probe.elf"
    chmod a+w "target/btop-04/$probe.elf"
done
' > "$out/toolchain.log" 2>&1
for probe in ifaddrs route-scope; do
    bash tools/stamp_helios_elf.sh "$out/$probe.elf"
done
python3 tools/native-c/cc.py -O2 -Wall -Wextra -Werror tools/btop-04/publish-denied.c -o "$out/publish-denied.elf"
fi
RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static" \
    cargo build --target-dir target/embedded-build --release --package sunlight-net-server --bin net_server
cp target/embedded-build/x86_64-unknown-none/release/net_server "$out/net_server"
# Reuse the earlier fixture recipe in a new output directory. Trap restores the
# temporary source packaging and tracing patches on both success and failure.
python3 - "$out/image.sh" <<'PY'
from pathlib import Path
import sys
s = Path("target/btop-03/image.sh").read_text().replace("target/btop-03", "target/btop-04")
s = s.replace('b.starts_with(b"/proc")', 'b.starts_with(b"/proc") || b.starts_with(b"/sys/class/net")')
s = s.replace('static EVENTS: AtomicUsize = AtomicUsize::new(0);',
              'static EVENTS: AtomicUsize = AtomicUsize::new(0);\n        static SELECT_EVENTS: AtomicUsize = AtomicUsize::new(0);')
s = s.replace('            if !path.as_deref().is_some_and',
              '            if original_linux_num == 270 && SELECT_EVENTS.fetch_add(1, Ordering::Relaxed) >= 16 {\n'
              '                // Keep repeated masked waits out of the discovery trace budget.\n'
              '            } else if !path.as_deref().is_some_and')
s = s.replace('} else if EVENTS.fetch_add(1, Ordering::Relaxed) < 1024 {\n            let seq = EVENTS.load(Ordering::Relaxed);',
              '} else {\n            let seq = EVENTS.fetch_add(1, Ordering::Relaxed) + 1;\n'
              '            if seq <= 1024 {')
s = s.replace('            }\n        }\n    }\n\n', '            }\n            }\n        }\n    }\n\n', 1)

s = s.replace('"/bin/tty-select"', '"/bin/route-scope"').replace('/tty-select.elf', '/route-scope.elf')
s = s.replace('    RamEntry::file("/bin/proc-native"', '''    RamEntry::file("/bin/net_server", 0, 0, mode::FILE_755,
        include_bytes!("../../target/btop-04/publish-denied.elf")),
    RamEntry::file("/bin/proc-native"''')
s = s.replace('Path(sys.argv[1]).write_text(\'\'.join(result))', '''edit('kernel/src/main.rs',
    '../../target/x86_64-unknown-none/release/net_server',
    '../../target/btop-04/net_server')
Path(sys.argv[1]).write_text(''.join(result))''')
Path(sys.argv[1]).write_text(s)
PY
bash "$out/image.sh" trace
sha256sum target/btop-01/btop "$out/"{ifaddrs.elf,route-scope.elf,publish-denied.elf,net_server,kernel-ramfs-trace-config.elf,guest-ramfs-trace-config.iso} > "$out/final.sha256"
