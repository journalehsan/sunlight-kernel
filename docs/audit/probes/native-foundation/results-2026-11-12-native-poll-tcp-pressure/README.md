# Native POLLOUT send-backpressure evidence — 2026-11-12

Design and semantics:
`docs/networking/2026-11-12_NATIVE_POLL_TCP_WRITE_BACKPRESSURE.md`.

All acceptance QEMU runs used 2048 MiB. Saturation and recovery were driven
over an out-of-band control channel; no fixed sleep was used as readiness
synchronization (the finite poll holds and deadlines are measurements, not
synchronization).

## Revision and worktree context

- Source revision: `5b08147001be963e468711d03284b349fa8b2089` (`main`).
- Uncommitted tree: the only changes are the new fixture/orchestration files
  and documentation listed under "Changes" below. No tracked file was
  modified. `tools/native-c/Cargo.lock` was temporarily synced offline to run
  the ABI regression scripts and restored to its committed (stale) content.
- Production path: unchanged (see "Production changes" below).

## Build note (pre-existing)

`bash tools/build_native_c.sh` fails at `HEAD` with
`cannot update the lock file ... because --locked was passed`:
`tools/native-c/Cargo.lock` pins `sunlight-libc 0.2.7167-alpha.2` while
`sunlight-libc/Cargo.toml` is `0.2.7254-alpha.2`. This predates this
milestone. As in the previous readiness milestone, the pressure runs used the
already-built `target/native-c/sysroot/lib/libsunlight_native_c.a`
(`--skip-build`). The archive was confirmed equivalent to a fresh offline
rebuild (identical defined-symbol set, 3437 symbols; only LLVM hash-mangled
local suffixes differ). The ABI regression scripts were run by syncing the
lock offline for the duration of the run, then restoring it.

## Acceptance matrix

| Directory | Mode | CPUs | Result |
| --- | --- | --- | --- |
| `external-2cpu/` | external | 2 | `poll-pressure: PASS cycles=3 unstable=1 ...` (RST `POLLHUP`) |
| `external-4cpu/` | external | 4 | `poll-pressure: PASS cycles=5 unstable=0 ...` |
| `external-2cpu-repeat/` | external | 2 | `poll-pressure: PASS cycles=5 unstable=0 ...` (race-sensitive repeat) |
| `loopback-2cpu/` | loopback | 2 | `poll-pressure-loopback: PASS cycles=19 unstable=36 ...` |
| `loopback-4cpu/` | loopback | 4 | `poll-pressure-loopback: PASS cycles=16 unstable=32 ...` |

Each run has `pressure-summary.json` (run/build commands, guest pass line,
clean exit, reap counters, peer exit + verdict, and NIC capture). Every
acceptance run reported `socket_live=0 listener_live=0 stream_live=0`.

### Saturation → no POLLOUT → drain → POLLOUT evidence

External runs (16 MiB stream, `payload[i] = (i*31+7) & 0xff`):

- `external-2cpu`: fill reached `EAGAIN`, `poll(POLLOUT, 0) == 0`,
  `poll(POLLOUT, 800 ms) == 0` (timed out), recovery in 10 ms, repeated 3×.
- `external-4cpu` / `external-2cpu-repeat`: same with 5 cycles, each
  `held_ms=800`, `recovery_ms∈{0,10}`.
- Peer verdict every run:
  `HOST-PEER PASS mode=pressure bytes=16777224 payload=16777216 fnv=5105c7d8c3e47d34`
  (byte-exact, independently recomputed FNV-1a 64).

Loopback runs duplicate the same transition patterns and validator
(`PRESSURE-SERVICE PASS`) using guest-internal peers. Loopback progressed
~12 KiB per fill cycle (client 4 KiB TX + peer 8 KiB RX), so the loopback
stream is 256 KiB with a 150 ms hold (the external runs use 16 MiB / 800 ms);
loopback is deliberately cheaper because both endpoints share the guest
smoltcp interface and the guest harness reaps the support service on a bounded
tick budget. Loopback is not a byte-for-byte duplicate of the external byte
budget; the final offset and cycle counts are recorded per run. The loopback
service drains on the explicit `D`/`H` commands during each cycle and re-checks
stream completion independently when it emits the `Q` verdict; an earlier
variant spun a full 30 s `Q` drain on an already-complete stream and was fixed
(recorded as flaky intermediate behaviour, not hidden as a pass).

`unstable` counts samples where capacity legitimately changed between the
`EAGAIN` and a poll snapshot (or a finite poll returned early), i.e. slirp /
socket buffers drained without an application read. It is reported, never
hidden, and is not a failure. See the design doc, "Scheduling races".

### Peer termination (external only)

- **Reset:** `poll-pressure: reset revents=0x10 elapsed_ms=0 write_errno=111`.
  The blocked `poll(POLLOUT)` ended immediately with `POLLHUP` (0x10) and the
  follow-up nonblocking send failed with `ECONNREFUSED` (111). `slot.reset` is
  never set by the current tcp manager, so RST surfaces through `Closed`
  (`POLLHUP`), not an invented `POLLERR`; the fixture asserts *some*
  `POLLERR`/`POLLHUP` bit and a negative send and records the exact values.
- **Orderly FIN:** the peer `shutdown(SHUT_WR)`s; `poll(POLLIN)` reports
  readability, `recv` returns 0, and a subsequent write still succeeds and
  reports `POLLOUT`. FIN is not reported as reset.

## Regression gates (`regressions/`)

- `poll-abi/`: 12 C/C++ native poll ABI checks, unique export, unchanged
  historical link — PASS.
- `socket-abi/`: 12 socket C/C++ ABI checks, exports, unchanged historical
  link — PASS.
- `poll-loopback-2cpu/`: existing native TCP poll loopback acceptance — PASS.
- `poll-external-2cpu/`: existing native TCP poll external acceptance — PASS.
- `loopback-tcp-2cpu/`: existing loopback TCP echo acceptance — PASS.
- `external-echo-2cpu/`: existing external TCP echo acceptance — PASS
  (byte-exact both directions, matching SHA-256).
- `external-stall-2cpu/`: existing external TCP stall acceptance — PASS
  (`tcp-stall: PASS errno=110 elapsed_ms=10000`).
- `kernel-check.log`: `cargo check -p sunlight-kernel` — PASS.
- `net-ipc-check.log`: `cargo check -p sunlight-net -p sunlight-ipc -p sunlight-net-server` — PASS.
- `libc-native-c-check.log`: `cargo check -p sunlight-libc --features native-c` — PASS (existing warnings only).

### Known failures retained (not rerun as unbounded)

- Historical `cpp-runtime` `LINK_FAIL` (no executable). Not part of this
  milestone's targeted gates; the full native suite would require the same
  stale-lock build path.
- The documented host `secret_store` hang and the seven `alloc`/`dirent`
  failures reproduce at `HEAD` (see the previous milestone's
  `REGRESSION-NOTES.md`); host `cargo test` was not rerun because no
  production code changed.

## Production changes

None. The production paths (`sunlight-net/src/tcp.rs`,
`services/net_server/src/main.rs`, `sunlight-libc/src/net_bridge.rs`,
`sunlight-libc/src/multiplex.rs`, `ipc/src/netop.rs`) were inspected but not
modified; the readiness predicate, waiter registration/wakeup, descriptor
lifetime and timeout accounting are unchanged.

## Cleanup counters

Every acceptance run ends with `socket_live=0 listener_live=0 stream_live=0`.
The external peer exits 0 with `HOST-PEER PASS`. Failed saturation attempts, if
any, are preserved in the run's `guest_stdout`/`guest_fail_line` and are not
promoted to PASS.

## Changes

- `tools/native-c/tcp_pressure_peer.py` (new external peer + validator)
- `tools/native-c/tests/tcp-poll-pressure-client.c` (new external client)
- `tools/native-c/tests/tcp-poll-pressure-service.c` (new loopback service)
- `tools/native-c/tests/tcp-poll-pressure-loopback-client.c` (new loopback client)
- `tools/native-c/run_poll_pressure.py` (new runner, external + loopback)
- `docs/networking/2026-11-12_NATIVE_POLL_TCP_WRITE_BACKPRESSURE.md` (design)
- this evidence directory

Historical probes, fixtures and command records outside this evidence
directory were not modified. Worktree changes are left uncommitted.
