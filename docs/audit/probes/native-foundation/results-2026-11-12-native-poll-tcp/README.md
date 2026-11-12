# Native poll/TCP readiness evidence — 2026-11-12

Design and semantics:
`docs/networking/2026-11-12_NATIVE_POLL_TCP_READINESS.md`.

All acceptance QEMU runs used 2048 MiB. Peer synchronization was explicit;
no fixed sleeps were used as readiness synchronization.

## Final acceptance (`final/`)

| Directory | CPUs | Result |
| --- | --- | --- |
| `loopback-2cpu/` | 2 | `tcp-poll: PASS socket_live=0 listener_live=0 stream_live=0` |
| `loopback-4cpu/` | 4 | same PASS |
| `loopback-2cpu-repeat/` | 2 | same PASS (race-sensitive repeat) |
| `external-2cpu/` | 2 | `poll-external: PASS exchanges=5 bytes=1371 ...` (default subnet) |
| `external-4cpu/` | 4 | same PASS |
| `external-nonsubnet-2cpu/` | 2 | same PASS (`10.0.3.0/24`) |
| `external-nonsubnet-4cpu/` | 4 | same PASS |

Each run records `poll-summary.json` with the build/run commands, guest pass
line, clean exit, reap counters, and (external runs) host-peer exit plus NIC
capture. External capture establishes traffic; it is not used as proof of
poll event ordering.

## Regression gates (`final/`)

- `regress-poll-abi/`, `regress-socket-abi/`: C/C++ base, POSIX, GNU,
  reverse-include and historical ABI/symbol/object comparisons passed.
- `regress-loopback-tcp-2cpu/`, `regress-loopback-tcp-4cpu/`: existing
  loopback TCP acceptance passed.
- `regress-external-echo-2cpu/`, `regress-external-nonsubnet-2cpu/`: existing
  external echo acceptance passed with byte-exact streams and matching
  bidirectional SHA-256.
- `suite-2cpu/`, `suite-4cpu/`: 94 cases each, 93 PASS; only the pre-existing
  `cpp-runtime` build `LINK_FAIL` (no executable) remains.
- `regress-kernel-check.log`, `regress-net-ipc-check.log`: kernel and
  net/ipc `cargo check` passed; `sunlight-libc --features native-c` check also
  passed (existing warnings only).

## Host-test classification

`REGRESSION-NOTES.md` records that the `secret_store` hang and the seven
`alloc`/`dirent` failures reproduce on a clean `HEAD` worktree. Baseline logs
are `baseline-head-secret-store-timeout.log` and
`baseline-head-alloc-dirent-tests.log`.

## Diagnostic history

`preflight-2cpu-multi-fail/`, `loopback-2cpu-diag*/`, `loopback-*-final/` and
`pre-final-identity-check/` are preserved as intermediate records. Their
`FAIL` results document defects already fixed (multi-descriptor errno and
close/reuse identity handling); their `PASS` results are superseded by
`final/`.

## Preservation

Historical probes, fixtures and command records outside this evidence
directory were not modified. Worktree changes were left uncommitted.
