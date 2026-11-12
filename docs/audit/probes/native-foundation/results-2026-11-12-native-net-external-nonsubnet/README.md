# Native C guest-to-host TCP under an explicit non-default QEMU subnet

**Date:** 2026-11-12
**Scope:** repeat of the external native C TCP acceptance with
`-netdev user,id=net0,net=10.0.3.0/24`.

This directory is fresh evidence for the non-default-subnet repeat. The
default-subnet evidence remains in
`../results-2026-11-12-native-net-external/` and is not reclassified here.
No socket API was broadened for this run.

## Runner change

`tools/native-c/guest.py` gained one option, `--net-subnet A.B.C.D/PREFIX`.
When present it appends `,net=<subnet>` to the existing `-netdev user,id=net0`
string. Without the option the emitted QEMU argument is byte-identical to
before (`-netdev user,id=net0`). The value is validated as a dotted IPv4
network with zero host bits and a prefix in 8..30; there is no arbitrary QEMU
argument passthrough.

`tools/native-c/run_external_tcp.py` gained `--net-subnet` and `--host-alias`.
`--net-subnet` requires an explicit `--host-alias`; the default remains
`10.0.2.2` and the default invocation is unchanged. The runner now also
records the DHCP DNS address, asserts the DHCP-advertised gateway equals the
supplied host alias, asserts ARP for that alias, asserts bidirectional TCP on
the echo runs, and (only when the alias is not `10.0.2.2`) asserts that no
IPv4 frame in the capture mentions `10.0.2.2`.

`tools/native-c/pcap_tcp.py` additionally returns the observed IPv4
`(src, dst)` pairs so the legacy-alias assertion reads the capture instead of
the DHCP log.

## Commands

All commands were run from the repository root.

```sh
E=docs/audit/probes/native-foundation/results-2026-11-12-native-net-external-nonsubnet
python3 tools/native-c/run_external_tcp.py --mode echo --cpus 2 \
  --net-subnet 10.0.3.0/24 --host-alias 10.0.3.2 \
  --out $E/echo-2cpu \
  --timeout 300 --run-id nonsubnet-echo-2cpu
python3 tools/native-c/run_external_tcp.py --mode echo --cpus 4 \
  --net-subnet 10.0.3.0/24 --host-alias 10.0.3.2 \
  --out $E/echo-4cpu --timeout 300 --run-id nonsubnet-echo-4cpu
python3 tools/native-c/run_external_tcp.py --mode echo --cpus 2 \
  --net-subnet 10.0.3.0/24 --host-alias 10.0.3.2 \
  --out $E/echo-repeat-2cpu --timeout 300 --run-id nonsubnet-echo-repeat-2cpu
python3 tools/native-c/run_external_tcp.py --mode stall --cpus 2 \
  --net-subnet 10.0.3.0/24 --host-alias 10.0.3.2 \
  --out $E/stall-2cpu --timeout 300 --run-id nonsubnet-stall-2cpu
python3 tools/native-c/run_external_tcp.py --mode stall --cpus 4 \
  --net-subnet 10.0.3.0/24 --host-alias 10.0.3.2 \
  --out $E/stall-4cpu --timeout 300 --run-id nonsubnet-stall-4cpu
```

Regressions after the runner changes:

```sh
E=docs/audit/probes/native-foundation/results-2026-11-12-native-net-external-nonsubnet
python3 tools/native-c/run_external_tcp.py --mode echo --cpus 2 \
  --out $E/regressions/default-echo-2cpu --timeout 300 --run-id default-echo-2cpu
python3 tools/native-c/run_external_tcp.py --mode stall --cpus 2 \
  --out $E/regressions/default-stall-2cpu --timeout 300 --run-id default-stall-2cpu
python3 tools/native-c/cc.py -std=c11 -Wall -Wextra -Werror \
  tools/native-c/tests/tcp-echo-client.c -o $E/regressions/tcp-echo-client.elf
python3 tools/native-c/cc.py -std=c11 -Wall -Wextra -Werror \
  tools/native-c/tests/tcp-echo-service.c -o $E/regressions/tcp-echo-service.elf
python3 tools/native-c/guest.py $E/regressions/tcp-echo-client.elf --net \
  --service $E/regressions/tcp-echo-service.elf --cpus 2 \
  --out $E/regressions/loopback-2cpu --timeout 300 --arg 4040
python3 tools/native-c/guest.py $E/regressions/tcp-echo-client.elf --net \
  --service $E/regressions/tcp-echo-service.elf --cpus 4 \
  --out $E/regressions/loopback-4cpu --timeout 300 --arg 4040
```

## Observed non-default network configuration

The actual QEMU argument recorded in each `guest/qemu.log`:

```
-netdev user,id=net0,net=10.0.3.0/24
```

The guest's own DHCP log in every non-default run:

```
[DHCP] lease acquired address=10.0.3.15/24 gateway=10.0.3.2 dns=10.0.3.3
```

| Field | Observed value |
| --- | --- |
| Guest address | `10.0.3.15` |
| Mask / prefix | `255.255.255.0` (`/24`) |
| Gateway | `10.0.3.2` |
| DNS | `10.0.3.3` |
| Host alias used by the client | `10.0.3.2` |

These values were read from the serial DHCP line and the pcap, not assumed.

## Claim A vs Claim B

**Claim A (proved):** the client reaches the explicitly supplied non-default
host alias `10.0.3.2`. The alias is a runner argument
(`--host-alias 10.0.3.2`), passed to the client as `argv[2]`, and the pcap
shows `10.0.3.15 -> 10.0.3.2` and `10.0.3.2 -> 10.0.3.15` with byte-exact
reconstructed streams. ARP for `10.0.3.2` is present in both directions.

**Claim B (not proved):** the destination is *not* selected from the
DHCP-advertised gateway at runtime. No native API exposes the guest's lease to
the client, and the client destination is fixed in its argv before boot. The
runner only performs a post-hoc check that the DHCP-advertised gateway equals
the alias it was told to use. This is host-side orchestration around a
verified DHCP result, not a native route/lease query, and it is not evidence
of automatic gateway selection.

Because `10.0.3.2` is on the guest's own `/24`, this run also does not
exercise off-subnet default routing.

## Successful non-default exchanges

Wire payloads are unchanged: sizes 1, 7, 64, 255 and 1024 bytes with embedded
NULs, a 4-byte big-endian length prefix, and a zero-length terminator. The
same host peer, framing, endpoint validation, deadlines and cleanup checks
are reused.

| Run | Guest endpoint | Guest result | Host result | NIC capture |
| --- | --- | --- | --- | --- |
| echo, 2 CPUs | `10.0.3.15 -> 10.0.3.2:40753` | `PASS exchanges=5 bytes=1371 socket_live=0 listener_live=0 stream_live=0` | `PASS exchanges=5 bytes=1371` | 2 ARP, 4 DHCP UDP, 35 TCP |
| echo, 4 CPUs | `10.0.3.15 -> 10.0.3.2:46513` | same PASS | same PASS | 2 ARP, 4 DHCP UDP, 35 TCP |
| echo repeat, 2 CPUs | `10.0.3.15 -> 10.0.3.2:39839` | same PASS | same PASS | 2 ARP, 4 DHCP UDP, 35 TCP |
| stall, 2 CPUs | `10.0.3.15 -> 10.0.3.2:42499` | `PASS errno=110 elapsed_ms=10000 socket_live=0 listener_live=0 stream_live=0` | `PASS mode=stall received_frames=1 bytes=8` | 2 ARP, 4 DHCP UDP, 9 TCP |
| stall, 4 CPUs | `10.0.3.15 -> 10.0.3.2:33151` | same PASS | same PASS | 2 ARP, 4 DHCP UDP, 9 TCP |

Byte integrity is identical to the default-subnet run:

| Run | outbound bytes | outbound SHA-256 | inbound bytes | inbound SHA-256 |
| --- | --- | --- | --- | --- |
| echo, 2/4 CPUs and repeat | 1395 | `47a9ea4aa44f1a4c6fab1982557cfea60efce87f55c49d9c41888a47e33e11bf` | 1391 | `acd17ec6a05c03d57191eb4a8e03b92b1aa18109134a30fbce50ad0accf5fd89` |
| stall, 2/4 CPUs | 12 | `3e3c22af446e48578259d3857b8b22018326dc5dc92abb77f0823ae81e1a7822` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

The reconstructed outbound and inbound streams equal the independently
generated expectations in every echo run.

ARP for the actual non-default alias, from `external-summary.json`:

```
{"opcode": 1, "sender": "10.0.3.15", "target": "10.0.3.2"}
{"opcode": 2, "sender": "10.0.3.2", "target": "10.0.3.15"}
```

Observed IPv4 endpoints in every non-default capture:
`0.0.0.0`, `10.0.3.15`, `10.0.3.2`, `255.255.255.255` (DHCP). The
`no_legacy_alias_10_0_2_2` check passed: no IPv4 frame mentions the old
`10.0.2.2` alias.

## Refusal and stalled-peer semantics

Unchanged from the default-subnet milestone:

- Refused connect after the host listener closes: `errno=111`
  (`ECONNREFUSED`).
- Nonblocking connect: `errno=95` (`EOPNOTSUPP`).
- Stalled peer: `recv` returns `-1` with `errno=110` (`ETIMEDOUT`) after the
  bridge's 10000 ms bounded deadline.

## Cleanup

- Guest: `[NATIVE-C-INET] before=0 after=0`,
  `[NATIVE-C-REAP-END] ... owned_frames=0 fds=0 stack=0`,
  `[NATIVE-C-RESULT] ... clean=1`, `[NATIVE-C-DONE] cases=1`.
- net_server diagnostics: `socket_live=0 listener_live=0 stream_live=0`.
- Host peer: `HOST-PEER PASS`, exit 0.
- QEMU: exit 33, no `qemu-system-x86_64` or `tcp_host_peer` process remained.
- `[NATIVE-C-SUPPORT-REAP]` is absent because the external runs start no guest
  support service.

## Regressions

| Gate | Result |
| --- | --- |
| Default-subnet external echo, 2 CPUs | PASS, `10.0.2.15 -> 10.0.2.2`, DHCP `10.0.2.15/24 gw 10.0.2.2 dns 10.0.2.3` |
| Default-subnet external stall, 2 CPUs | PASS, `errno=110` after 10000 ms |
| Default QEMU argument preserved | `-netdev user,id=net0` (no `net=`) |
| Loopback echo, 2 CPUs | PASS; pcap has 4 DHCP UDP frames, 0 TCP |
| Loopback echo, 4 CPUs | PASS; pcap has 4 DHCP UDP frames, 0 TCP |
| Python syntax of the three changed tools | `py_compile` OK |

## Limitations

- This proves non-default-subnet host-alias connectivity only.
- It does not prove off-subnet default routing (the alias is on-subnet), DNS
  resolution, public/internet connectivity, or guest-to-guest networking.
- The destination is not derived from the guest's DHCP lease at runtime; see
  Claim B above.
- No UDP, asynchronous connect, nonblocking accept, shutdown, readiness
  integration, dup/fork transfer, bind-then-connect, socket options, SIGPIPE,
  IPv6 or `hostfwd` behavior is exercised.

## Files

Each run directory contains `external-summary.json`, `host-peer.log`,
`guest-runner.log`, `client-build.log`, `native-archive.log`, and
`guest/{serial.log,results.json,results.csv,network.pcap,network-summary.log,
qemu.log,invocation.log,kernel-sha256.log,revision.log,faults.log,worktree.log,
elf.stdout.txt,elf.stderr.txt,qemu-version.log}`.
`regressions/` holds the default-subnet and loopback reruns plus the freshly
compiled loopback client/service ELFs.
