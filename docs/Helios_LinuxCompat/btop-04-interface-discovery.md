# Helios BTop-04: read-only interface discovery

This phase continues [BTop-03](btop-03-readiness-network.md). It leaves the
earlier evidence, the pinned btop and masked `pselect6` unchanged.

## What the pinned caller needs

The stripped ELF was verified against SHA-256
`bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702`.
The retained link map locates the linked musl callback at `0x60d762`,
`getifaddrs` at `0x60dc7b`, and its netlink helpers at `0x60dedf` and
`0x60dfc5`. Disassembly of those addresses in **the pinned binary** shows:

- `socket(AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, NETLINK_ROUTE)`, without
  bind, connect, a destination address or multicast subscription.
- A 20-byte `RTM_GETLINK` request (sequence 1), then a 20-byte
  `RTM_GETADDR` request (sequence 2). Both have flags `0x301`, family
  `AF_UNSPEC`, and zero sender and padding.
- `recv` with an 8192-byte buffer and `MSG_DONTWAIT`. The caller walks
  four-byte-aligned messages, invokes the callback on records, stops on
  `NLMSG_DONE`, and treats `NLMSG_ERROR` as failure. It closes the fd.
  Link names, indices and flags associate address records with links;
  the IPv4 prefix yields the mask. `IFLA_ADDRESS` gives musl its AF_PACKET
  record with the link index, even for an interface with no IPv4 lease.

## Data ownership and scope

Native `NetInfo` reads the selected driver backend and is restricted to
net_server, networkd and networkctl. `GET_BACKEND` and `GETIP_LIVE` are
existing network IPC reads. Native networkd uses a bounded `GETIP_LIVE`
request for the executing lease; `GETIP` can instead return policy.
`GETIP_LIVE` reports the same zero words before backend admission and after
a completed attempt with no lease. Separate queries would also not pin the
same link and address state for both dumps.

The existing network stack publishes the results after installing its
addresses, using a narrow `NetInfo` operation restricted to the
kernel-identified, root-launched embedded net_server image. The ELF
loader assigns a marker used only by this publication; it does not alter
the service's general credential classification. It accepts
only an optional Ethernet IPv4 address/prefix, an optional installed
loopback IPv4 address/prefix, and the matching backend kind and MAC.
Existing frame I/O, nameserver and IPC permissions did not change. A
publication binds to the process and address-space generation, and a new
dump checks that the publisher is still live and the backend still
matches. A missing publication returns ENETDOWN. A completed acquisition
attempt without a lease retains the Ethernet link and omits its address.
The Ethernet address comes from `LIVE_CONFIG`; there is no fixture IP in
the compatibility implementation. The guest currently configures DHCP
once at network service startup; there is no in-place DHCP renew/removal
path in this phase.

The first request samples carrier and freezes the pair of dumps for the
same socket. There are at most 64 open descriptions and one pending
datagram per description. Replies contain at most 128 bytes, echo the
request sequence and use multipart kernel-sender framing:
`RTM_NEWLINK` with `IFLA_IFNAME` / `IFLA_ADDRESS`, `RTM_NEWADDR` with
`IFA_LOCAL`, and `NLMSG_DONE`. The guest's two modeled link identities are
`lo` (1) and `eth0` (2). This is a Helios query interface for the selected
backend and its local IPv4 stack; it does not implement general netlink,
IPv6, routes, subscriptions, arbitrary interface lookup, or any mutation.
It does not expose the native network capabilities to Linux callers.

The first replay's worker (PID 39, trace 69–77) completed both dumps and
then attempted to stat `/sys/class/net/{lo,eth0}/statistics/{rx,tx}_bytes`.
All four files were then absent. The final tree exposes only the two
Ethernet files, as read-only Helios virtual files. They sample the same
driver RX-delivered/TX-submitted byte counters as `/proc/net/dev`.
There is no equivalent loopback counter producer and no address file was
requested on that replay, so those files remain absent. The original ISO,
screen captures and the missing-file trace are in
`target/btop-04/discovery-only/`.

## Separate upstream error path

The retained btop source's `IfAddrsPtr` in `src/btop_shared.hpp` leaves its
output pointer uninitialized, passes it to `getifaddrs`, and unconditionally
frees it in the destructor. The linked musl stores the output pointer only
on success. BTop-03 recorded the resulting fault in `freeifaddrs` after
EAFNOSUPPORT; a genuine failure can still expose the same upstream defect.
Initialize the pointer and free it only on success in upstream btop.
This phase neither changes the binary nor reports a discovery failure as
successful enumeration.

## Reproduction

Run from the repository root. Use fresh evidence directories. The boot
fixture uses BIOS/TCG, four vCPUs, 4096 MiB, virtio VGA and an authenticated
framebuffer TTY. Each run saves the effective QEMU command in
`command.json`. The DHCP replay uses restricted user networking with a
non-default DHCP start address. The unconfigured run retains a VirtIO NIC
but uses a QEMU packet buffer to withhold DHCP traffic through the
service's acquisition window. The unavailable case has no NIC.

```sh
objdump -d --start-address=0x60d6e5 --stop-address=0x60e04a target/btop-01/btop > target/btop-04/pinned-musl.dis
sha256sum -c target/btop-03/final.sha256
# The matched Alpine ifaddrs.elf was retained from the earlier build.
# A later Alpine package fetch stalled; only the separate scope probe was
# rebuilt with the local musl compiler.
musl-gcc -v -O2 -Wall -Wextra -Werror -static -fno-pie -no-pie tools/btop-04/route-scope.c -o target/btop-04/route-scope.elf
bash tools/stamp_helios_elf.sh target/btop-04/route-scope.elf
bash tools/btop-04/build-replay-image.sh --reuse-probes
sha256sum -c target/btop-04/final.sha256
iso=target/btop-04/guest-ramfs-trace-config.iso
python3 tools/btop-04/replay.py "$iso" target/btop-04/final-lease
python3 tools/btop-04/replay.py "$iso" target/btop-04/final-unconfigured --mode unconfigured
python3 tools/btop-04/replay.py "$iso" target/btop-04/final-unavailable --mode unavailable
python3 tools/btop-04/replay.py "$iso" target/btop-04/final-btop --mode btop
python3 tools/btop-02/replay.py "$iso" target/btop-04/final-net --network
python3 tools/btop-02/verify-network.py target/btop-04/final-net
bash tools/runtime-parity/run.sh target/btop-04/final-parity
cargo check -p sunlight-kernel --features native_document_viewer
cargo test -p sunlight-compat-linux -p sunlight-net -p sunlight-telemetry --target x86_64-unknown-linux-gnu
git diff --check
```

The matched Alpine build uses GCC 15.2.0-r2, musl 1.2.5-r24, and the
container digest recorded in the build script. The native denial probe
uses the repository's native C toolchain. The separate scope probe uses
host GCC 16.2.1 with musl; see `scope-toolchain.log`.

| Final fixture artifact | SHA-256 |
|---|---|
| Pinned btop | `bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702` |
| Static musl getifaddrs probe | `2ed04b3454070f79479f2b5ea28bfd2ad249acc7246a1ebea98acbea1d86468e` |
| Route scope and counter probe | `62212bed13cb7417990ec2eb9628effcd04d058e407ff4a5f23ac6f7e40b1f47` |
| Native publication denial probe | `69911738530d4eaf16a1d02fae8773d06a1db22626407104c00b1c21fa1d89fc` |
| Embedded net_server | `9d46a25c86d844ae725dbfaa4bcb27b396e4cda1739cb8c1e5740ab7745a33c5` |
| Kernel ELF | `37bdfe4dd04f943c3c051c8676ed158bab6ea6711ad9b29c26e8230fd8d4d8bb` |
| Replay ISO | `65bb9c4770d5efc2d0a0c3aab7e96738d712d092b131a91cf4b7c2e4ab531f51` |

Additional fixture hashes are in `target/btop-04/reused-fixtures.sha256`.
Runtime parity writes separate ELF/library hashes in `final-parity/artifacts.sha256`
and its separate guest kernel and ISO hashes in `final-parity/guest/*-sha256.log`.

## Final results

| Gate | Result | Evidence on the final source |
|---|---|---|
| Static musl probe, DHCP lease | **PASS** | QEMU provided a real DHCP ACK and net_server logged the installed `10.0.2.42/24` lease. Musl returned `lo#1 127.0.0.1/255.0.0.0` and `eth0#2 10.0.2.42/255.255.255.0`. The separate native `networkctl status` queries agreed on both names, IDs and CIDRs. Seventy enumerations and frees completed; the process exited 0 and was reaped. |
| Static musl probe, unconfigured Ethernet | **PASS** | QEMU withheld DHCP traffic until the service timed out. Musl retained `eth0#2` with no IPv4 address, agreeing with native networkctl. It still returned the loopback address and completed 70 cleanups, exit 0 and reap. |
| Static musl probe, unavailable backend | **PASS** | No NIC. The link request returned ENETDOWN; musl left the initialized result pointer unchanged. The test exited 0 and was reaped. |
| Protocol and access controls | **PASS** | The Linux scope probe rejected NETLINK_USERSOCK, RTM_NEWADDR, RTM_GETROUTE and a false message length. It verified that a short buffer did not consume the reply, and that a drained socket returned EAGAIN. It also checked EROFS for an Ethernet counter write and ENOENT for the unbacked loopback counter. An ordinary native program named `net_server` received EPERM for the publication operation. |
| Pinned btop: interface discovery | **PASS** | Worker PID 39: trace events 42–46 opened the socket, sent 20-byte requests and received 128-byte link and 84-byte address replies. Trace events 49–52 show both Ethernet attributes statted and opened successfully. The same sequence recurs at events 74–84 and in subsequent refreshes. |
| Pinned btop: completed collector refreshes | **PASS** | Multiple worker collection cycles and populated scanouts, including the process row, CPU, memory and network measurements, in `final-btop/tty/03-dashboard-*.png`. |
| Pinned btop: panel data | **PASS** for observed CPU, memory, process and Ethernet data | The first dashboard capture shows four CPU rows, 3.39 GiB total memory, the btop process and `10.0.2.42` with 1.21 KiB download and 958 B upload totals. Controlled changes to the process list and a panel-specific throughput workload were **NOT RUN**. |
| Ethernet counters | **PASS** | The scope probe matched sysfs against bracketed procfs samples: RX/TX 1244/810 B with a lease and 0/912 B in the no-lease run. The separate gateway test matched both procfs rows to prefixes of the captured Ethernet traffic: RX 1244 B / 3 frames → 1700 B / 8, TX 810 B / 5 frames → 1392 B / 12, four local echo pairs. |
| Masked `pselect6` and normal keyboard exit | **BLOCKED** | Main PID 38 still receives EOPNOTSUPP (trace events 19–34). The unmodified replay timed out waiting for a normal exit after the input sequence. No signal or input bypass was used. |
| Runtime parity | **PASS** | Both static musl and native workloads passed file I/O, TLS and thread join. The trace confirmed each ABI's syscall and worker routing. |
| Kernel and host regressions | **PASS** | Kernel feature check, 29 Helios, 31 network and 12 telemetry unit tests; one telemetry doc test ignored. |
| Full boot/storage gate, VMXNET3 runtime, service restart and in-place lease removal | **NOT RUN** | The focused 4 GiB guest used VirtIO; the service performs one DHCP acquisition at startup. |

Serial traces, captures and verifiers are under `target/btop-04/final-*`.
The btop QEMU command and packet capture are in `final-btop/`, with its
serial trace and scanouts in `final-btop/tty/`. Development attempts
remain separate: the hub-only boot never reached the authenticated shell,
and an earlier counter replay got “command not found” before launching
the probe. Those attempts do not establish the final result. The final
unconfigured and counter runs use the procedures above.

No commit or push was made.
