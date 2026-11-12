# Native C TCP bridge (Phase 23)

**Date:** 2026-11-12
**Scope:** bounded native C `AF_INET`/`SOCK_STREAM` transport for SunlightOS
**Status:** loopback TCP and guest-to-host TCP over QEMU user-mode networking
both work end to end through the production `net_server` stack. This is **not**
complete BSD sockets and not complete TCP/UDP support.

## What changed

Native `socket()` already created a kernel `inet_socket` descriptor, but that
descriptor had no userspace transport attachment: `read`/`write`/`send`/`recv`
returned `ENOTCONN` and there was no `bind`/`connect`/`listen`/`accept`.

The TCP/IP stack already lives in `net_server` (`sunlight-net` smoltcp manager
over the kernel frame proxy) with `SOCKET`/`CONNECT`/`BIND`/`LISTEN`/`ACCEPT`/
`SEND_SHM`/`RECV_SHM`/`WAIT`/`CLOSE`/`GET_SOCKNAME`/`GET_PEERNAME`/
`GET_DIAGNOSTIC` operations. Phase 23 **bridges** those operations into native
libc instead of adding a second stack:

- `ipc/src/netop.rs` is the single canonical opcode/status/readiness table;
  `sunlight-net/src/netop.rs` re-exports it.
- `sunlight-libc/src/net_bridge.rs` owns a fixed 256-entry fd -> net_server
  socket-id table. The table lock is a short spinlock and is never held across
  an IPC call.
- `socket`/`bind`/`connect`/`listen`/`accept`/`send`/`recv`/`read`/`write`/
  `sendto`/`recvfrom`/`getsockname`/`getpeername`/`close` in native libc
  dispatch to the bridge when the descriptor has an attachment.
- `close()` releases the net_server socket before the kernel descriptor is
  consumed; `socket()` clears any stale mapping for a reused fd number.

No file, pipe, shared-buffer or host-side response substitution is used. The
bytes cross smoltcp and `ProxyNetDevice`.

## Capability matrix

`topology` is the only topology actually exercised by the Phase 23 fixtures.
Everything else is untested, even where the code path exists.

| Operation | Transport | Blocking / nonblocking | Readiness | Tested topology |
| --- | --- | --- | --- | --- |
| `socket(AF_INET, SOCK_STREAM, 0)` | IPv4 TCP | creation only | n/a | loopback (client + service) |
| `bind` | IPv4 TCP | blocking, 5 s IPC deadline | n/a | loopback `127.0.0.1:4040` |
| `listen` | IPv4 TCP | blocking, 5 s IPC deadline | backlog via `WAIT` ACCEPT | loopback, backlog 1 |
| `accept` | IPv4 TCP | blocking, 10 s `WAIT` deadline; `O_NONBLOCK` -> `EOPNOTSUPP` | `NetOp::WAIT` ACCEPT | loopback |
| `connect` | IPv4 TCP | blocking, 10 s `WAIT` WRITE deadline; `O_NONBLOCK` -> `EOPNOTSUPP` | `NetOp::WAIT` WRITE | loopback `127.0.0.1:4040`; external `10.0.2.15 -> 10.0.2.2` |
| `send` / `write` | IPv4 TCP | blocking with 10 s readiness deadline; `MSG_DONTWAIT`/`O_NONBLOCK` -> `EAGAIN` | `WAIT` WRITE | loopback; external 1..1371 bytes |
| `recv` / `read` | IPv4 TCP | blocking with 10 s readiness deadline; `MSG_DONTWAIT`/`O_NONBLOCK` -> `EAGAIN` | `WAIT` READ/EOF | loopback; external 1..1371 bytes; stalled peer -> `ETIMEDOUT` |
| `sendto(dst = NULL)` | IPv4 TCP | same as `send` | same as `send` | loopback; external |
| `sendto(dst != NULL)` | IPv4 TCP | immediate `EISCONN` | n/a | loopback (rejected) |
| `recvfrom` | IPv4 TCP | same as `recv`, then fills peer | same as `recv` | loopback peer `127.0.0.1:4040`; external peer `10.0.2.2:<port>` |
| `getsockname` | IPv4 TCP | 5 s IPC deadline | n/a | loopback; external (ephemeral local port) |
| `getpeername` | IPv4 TCP | 5 s IPC deadline | n/a | loopback; external `10.0.2.2:<port>` validated |
| `shutdown` | IPv4 TCP | immediate `EOPNOTSUPP` | n/a | loopback (rejected, stream stays usable) |
| `close` | IPv4 TCP | 5 s IPC deadline for `NetOp::CLOSE` | n/a | loopback and external; live counters return to 0 |
| `poll` | IPv4 TCP | timeout 0 / finite / negative | `WAIT` ACCEPT/READ/EOF/WRITE/terminal bits | loopback 2/4 CPUs; external default and non-default 2/4 CPUs |
| `select` | IPv4 TCP | shares `poll` | as `poll` | loopback 2/4 CPUs |
| `epoll` | IPv4 TCP | not bridged | kernel reports `POLLERR` for `inet_socket`; never false readiness | not tested |
| `dup`/`fork` | IPv4 TCP | not bridged; attachment is not copied/transferred | n/a | not tested |
| UDP (`SOCK_DGRAM`) | — | not implemented in native libc | — | not tested |

Address handling: `sockaddr_in` is the standard 16-byte layout, `sin_family`
host order, `sin_port` and `sin_addr` network order. `INADDR_LOOPBACK` is the
BSD host-order constant `0x7f000001`; callers pass it through `htonl`.
`bind`/`connect` require `addrlen >= 16`; `getsockname`/`getpeername` require
capacity `>= 16` and always report `socklen_t` 16.

## Deterministic topology

`tools/native-c/guest.py --net --service ...` boots one disposable QEMU TCG
guest (2048 MiB, 2 or 4 CPUs) with `virtio-net-pci` behind QEMU user-mode
networking and a `filter-dump` pcap. `sunlight-init`, `rand_service` and
`sunlight-net-server` are support processes; the echo service is support PID 4.
The client is the fixture process.

The service retries `bind`/`listen` every 20 ms for up to 15 s while
`net_server` publishes its handlers and interface. The client retries
`connect` every 20 ms for up to 15 s. Both are bounded readiness handshakes,
not fixed sleeps.

Loopback runs keep the exchange guest-internal. Their pcap contains only the
four DHCP UDP frames; zero TCP frames cross the virtual NIC.

## Guest-to-host topology (QEMU user-mode networking)

The external milestone uses the same runner with `--net` but no guest support
service. Only `sunlight-init`, `rand_service` and `sunlight-net-server` are
started. The QEMU backend is `-netdev user,id=net0` with no explicit subnet,
`hostfwd` or guest network override. Under this configuration QEMU's user-mode
backend uses `10.0.2.0/24`, gateway/host alias `10.0.2.2` and DHCP server
`10.0.2.2`. The guest's own DHCP log in every run records:

```
[DHCP] lease acquired address=10.0.2.15/24 gateway=10.0.2.2 dns=10.0.2.3
```

Neighbor resolution is real: each pcap has an ARP request from `10.0.2.15` for
`10.0.2.2` and an ARP reply from `10.0.2.2`. The host peer binds
`127.0.0.1:0` only and announces the selected port. QEMU user-mode networking
forwards the guest's connection to `10.0.2.2:<port>` to that host-loopback
listener; the host-observed peer is a slirp-side `127.0.0.1:<ephemeral>`
socket, not the guest address. No public internet, DNS, root privilege or
`hostfwd` is required.

The host peer (`tools/native-c/tcp_host_peer.py`) selects the port with
`bind(..., 0)` and keeps the listener open until the guest connects, avoiding a
check-then-bind race. It prints an explicit
`HOST-PEER READY mode=<mode> addr=127.0.0.1 port=<port>` line before the guest
starts. It closes the listener immediately after `accept` so the client's
follow-up connect is deterministically refused. The peer validates every
length-prefixed frame against the expected generated payload, echoes the
header and payload byte-for-byte, handles partial reads/writes, recognizes the
zero-length terminator, and uses bounded socket deadlines with `finally`
cleanup.

`tools/native-c/run_external_tcp.py` starts the peer, waits for its readiness
line, invokes `guest.py` with the peer port, `10.0.2.2` and a run identifier,
then requires both endpoints to pass and reconstructs the NIC TCP streams for
byte comparison. `tools/native-c/pcap_tcp.py` is the minimal stdlib
reader/reconstructor used because `tshark`, `tcpdump` and `scapy` were not
available.

## Non-default user-mode subnet

`guest.py --net --net-subnet A.B.C.D/PREFIX` appends `,net=<subnet>` to the
same `-netdev user,id=net0` string; without the option the emitted QEMU
argument is unchanged. `run_external_tcp.py --net-subnet` requires an
explicit `--host-alias`, because the destination is client argv and is never
read from the DHCP lease at runtime. The default invocation still uses
`10.0.2.2`.

Under `net=10.0.3.0/24` the guest DHCP log records
`address=10.0.3.15/24 gateway=10.0.3.2 dns=10.0.3.3`, and the pcap shows ARP
for `10.0.3.2` plus TCP between `10.0.3.15` and `10.0.3.2` with the same
reconstructed byte streams. This proves non-default-subnet host-alias
connectivity. It does not prove off-subnet default routing (the alias is
on-subnet), DNS, public connectivity, or guest-to-guest networking, and it is
not evidence that the destination was selected from the DHCP gateway.
Evidence: `docs/audit/probes/native-foundation/results-2026-11-12-native-net-external-nonsubnet/`.

Canonical reproduction:

```sh
python3 tools/native-c/run_external_tcp.py --mode echo --cpus 2 \
  --out target/netexternal-echo-2cpu --timeout 300 \
  --run-id external-echo-2cpu
python3 tools/native-c/run_external_tcp.py --mode stall --cpus 2 \
  --out target/netexternal-stall-2cpu --timeout 300 \
  --run-id external-stall-2cpu
```

Fresh external results:

| Run | Guest endpoint | Guest result | Host result | NIC capture |
| --- | --- | --- | --- | --- |
| 2 CPUs | `10.0.2.15 -> 10.0.2.2:46163` | PASS, 5 exchanges / 1371 bytes | PASS, 5 framed exchanges | 2 ARP, 4 DHCP UDP, 38 TCP |
| 4 CPUs | `10.0.2.15 -> 10.0.2.2:44185` | PASS, identical | PASS, identical | 2 ARP, 4 DHCP UDP, 35 TCP |
| repeat 2 CPUs | `10.0.2.15 -> 10.0.2.2:36329` | PASS, identical | PASS, identical | 2 ARP, 4 DHCP UDP, 35 TCP |
| stalled peer, 2 CPUs | `10.0.2.15 -> 10.0.2.2:34775` | PASS, `recv` `ETIMEDOUT` after 10000 ms | PASS, one frame read, no response | 2 ARP, 4 DHCP UDP, 9 TCP |
| stalled peer, 4 CPUs | `10.0.2.15 -> 10.0.2.2:36315` | PASS, same 10000 ms deadline | PASS, same | 2 ARP, 4 DHCP UDP, 9 TCP |

The outbound reconstructed stream is 1395 bytes and the inbound stream is 1391
bytes on every echo run; their SHA-256 values are
`47a9ea4aa44f1a4c6fab1982557cfea60efce87f55c49d9c41888a47e33e11bf` and
`acd17ec6a05c03d57191eb4a8e03b92b1aa18109134a30fbce50ad0accf5fd89`
respectively. The stall run's outbound stream is the 12-byte frame
(`3e3c22af446e48578259d3857b8b22018326dc5dc92abb77f0823ae81e1a7822`) and its
inbound payload stream is empty. Packet counts are not the acceptance
criterion; the reconstructed endpoint bytes are.

The refused probe reports `errno=111` (`ECONNREFUSED`) after the host listener
closes, and asynchronous connect is rejected with `errno=95` (`EOPNOTSUPP`).
The stalled peer exercises the bridge's documented bounded I/O wait: `recv`
returns `-1` with `errno=110` (`ETIMEDOUT`) after 10000 ms, not an invented
POSIX result and not an unbounded sleep.

## Failure paths exercised

- `recv(..., MSG_DONTWAIT)` with no data -> `EAGAIN`.
- `connect` after the listener closes -> `ECONNREFUSED` (observed `errno=111`);
  a transport reset remains mapped to `ECONNRESET`.
- `connect` on a `SOCK_NONBLOCK` descriptor -> `EOPNOTSUPP` (observed
  `errno=95`, explicit).
- Connected `recv` with a peer that accepts and withholds data ->
  `ETIMEDOUT` (observed `errno=110`) after the bridge's 10 s I/O deadline.
- `shutdown` -> `EOPNOTSUPP` (explicit), stream remains usable.
- Peer EOF after the zero-length terminator frame.
- `sendto` with a destination on a connected socket -> `EISCONN`.

## Evidence

`docs/audit/probes/native-foundation/results-2026-11-12-native-net/` contains
the 2-CPU, 4-CPU and repeat-2-CPU serial logs, results, pcaps and packet
summaries, the historical-command logs, the ABI/export logs and the full suite
results.

`docs/audit/probes/native-foundation/results-2026-11-12-native-net-external/`
contains the separate external evidence: per-run `external-summary.json`,
host-peer logs, guest serial logs/results, QEMU command lines, pcaps and
reconstructed-stream hashes for echo 2/4 CPUs and the repeat, plus the 2/4-CPU
stalled-peer runs. Its `regressions/` subtree holds the fresh loopback, full
suite, socket ABI, host libc, kernel-check and native-std overlay logs.

Cleanup evidence in the external runs: each client reports
`socket_live=0 listener_live=0 stream_live=0`; the kernel runner reports
`[NATIVE-C-INET] before=0 after=0` and
`[NATIVE-C-REAP-END] ... owned_frames=0 fds=0 stack=0`; the host peer exits 0;
no QEMU or host-peer process remains. `[NATIVE-C-SUPPORT-REAP]` is absent
because these runs intentionally start no guest support service.

## Known limitations / remaining work

- No UDP, no `SOCK_DGRAM`, no multicast/broadcast.
- No asynchronous connect (`EINPROGRESS`/`SO_ERROR`), no nonblocking
  `accept`; both are rejected explicitly.
- No `shutdown` half-close; peer closure is observed as `recv() == 0`.
- `poll`/`select` readiness is bounded to the `NetReady` matrix. `epoll` for
  TCP is still not bridged.
- Mixed local+network `poll` sets recheck the network set at a 20 ms bounded
  slice; more than 32 bridged descriptors use the same slice. This is not a
  busy spin, but it is not single-wakeup multiplexing.
- FIFO/EOF-only streams report `POLLIN`; `POLLHUP` requires the stack's
  `CLOSED` state, not merely a peer FIN. `POLLERR` reports RESET/ERROR.
- No `dup`/`fork` attachment transfer.
- `bind`-then-`connect` (client-side source address/port selection) is not
  supported; `connect` on an already-bound descriptor returns `EISCONN`.
- Guest-to-host is exercised through the default QEMU user-mode topology
  (`10.0.2.0/24`, host alias `10.0.2.2`) and one explicit non-default subnet
  (`10.0.3.0/24`, host alias `10.0.3.2`), each with one host peer. The
  non-default destination is an explicit runner argument, not a value read
  from the guest's DHCP lease. Off-subnet default routing and guest-to-guest
  topologies are untested.
- No `SO_*` socket options, no `TCP_NODELAY`, no `MSG_PEEK`/`MSG_WAITALL`.
- No `SIGPIPE` generation; writing after peer closure surfaces as `EPIPE`.

The non-default-subnet repeat is recorded above. A remaining step is an
off-subnet destination (or a guest-side lease/route query) that would prove
the path follows the DHCP-advertised gateway rather than an explicit alias.
Host-to-guest inbound with an explicit `hostfwd` remains a separate milestone.
