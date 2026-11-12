# Phase 23 native C TCP bridge acceptance (2026-11-12)

Fresh evidence for the bounded native C networking slice. Commands were run
from the repository root. `target/` build products are not copied here; only
the serial transcripts, QEMU captures and machine-readable results are kept.

## Fixtures

- Service: `tools/native-c/tests/tcp-echo-service.c`
- Client: `tools/native-c/tests/tcp-echo-client.c`
- Historical source (unchanged): `docs/audit/probes/socket.c`

Wire format: 4-byte big-endian length + payload; a zero-length frame ends the
session. Payload sizes 1, 7, 64, 255 and 1024 bytes, each with embedded NUL
bytes, for 5 exchanges / 1371 payload bytes.

## Commands

```sh
bash tools/build_native_c.sh
python3 tools/native-c/cc.py -std=c11 -Wall -Wextra -Werror \
  tools/native-c/tests/tcp-echo-service.c -o target/netphase-build/tcp-echo-service.elf
python3 tools/native-c/cc.py -std=c11 -Wall -Wextra -Werror \
  tools/native-c/tests/tcp-echo-client.c -o target/netphase-build/tcp-echo-client.elf

python3 tools/native-c/guest.py target/netphase-build/tcp-echo-client.elf --net \
  --service target/netphase-build/tcp-echo-service.elf --cpus 2 \
  --out target/netphase-final-2cpu --timeout 300 --arg 4040
python3 tools/native-c/guest.py target/netphase-build/tcp-echo-client.elf --net \
  --service target/netphase-build/tcp-echo-service.elf --cpus 4 \
  --out target/netphase-final-4cpu --timeout 300 --arg 4040
```

`guest.py --net` builds `sunlight-net-server` and `sunlight-init`, boots one
disposable TCG guest with `-m 2048M`, `-smp {2,4}`, `-netdev user,id=net0`,
`virtio-net-pci,disable-modern=on`, and a QEMU `filter-dump` pcap. The service
runs as support PID 4 and net_server as support PID 3; the client is the
fixture process (PID 100).

## Topology

- Guest NIC: virtio-net over QEMU user-mode networking, DHCP lease
  `10.0.2.15/24`, gateway `10.0.2.2`, DNS `10.0.2.3`.
- Transport under test: guest-internal **loopback** `127.0.0.1:4040`. smoltcp
  and `ProxyNetDevice` keep loopback frames inside the guest; the pcap shows
  only DHCP/UDP frames and zero TCP frames.
- This is loopback coverage only. It is not guest-to-host or guest-to-guest
  evidence and says nothing about NIC routing or neighbor discovery.

## Results

| Configuration | Client result | Cleanup |
| --- | --- | --- |
| 2 CPUs, 2048 MiB | `PASS exchanges=5 bytes=1371 socket_live=0 listener_live=0 stream_live=0` | `clean=1`, service exit 0 |
| 4 CPUs, 2048 MiB | same | `clean=1`, service exit 0 |
| 2 CPUs repeat | same | `clean=1`, service exit 0 |

`network-summary.log` in each directory records `frames=4`,
`ipv4-proto-17=4` (DHCP), and no TCP frames.

## Historical probe

`historical/original-command.log` is the recorded bare invocation re-executed
verbatim:

```sh
clang --target=x86_64-unknown-none-elf -isystem /usr/lib/clang/23/include \
  -nostdinc -I"$PWD/sunlight-libc/include" docs/audit/probes/socket.c \
  -o OUTPUT -nostdlib -fuse-ld=lld
```

It remains **LINK_FAIL** (exit 1). The original 2026-11-11 run failed earlier
with `'sys/socket.h' file not found`; with the current include tree the same
command now compiles and fails at link with `undefined symbol: close`. Both
outcomes are the same intrinsic limitation: the bare command supplies no
native archive.

`historical/driver-build.log` links the unchanged source through
`tools/native-c/cc.py` with no undefined symbols, and
`historical/driver-guest-2cpu.serial.log` records guest exit 0.

`socket-abi/` records the 12 C/C++ ABI checks, unique exports, and the
byte-for-byte source hash (`socket.c.sha256`,
`historical-sha256.log`).

## Regressions

`suite-2cpu/` and `suite-4cpu/` contain the full native suite results:
93/93 buildable cases PASS on each configuration, with the preserved
historical `cpp-runtime` LINK_FAIL row and no fault markers.
