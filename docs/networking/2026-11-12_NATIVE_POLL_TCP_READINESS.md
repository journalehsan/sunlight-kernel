# Native poll/TCP readiness slice

**Date:** 2026-11-12
**Scope:** `poll(2)` readiness for bridged `AF_INET`/`SOCK_STREAM` descriptors
**Status:** implemented and guest-validated for the matrix below. Evidence is
under
`docs/audit/probes/native-foundation/results-2026-11-12-native-poll-tcp/final/`.

## Inspected paths

- Native C ABI: `sunlight-libc/src/multiplex.rs` (`poll`, `select`) calls
  syscall 159. `kernel/src/arch/x86_64/syscall.rs::sys_native_pipe_poll`
  validates/copies the 8-byte LP64 records and performs the descriptor scan.
- Descriptor classification: `kernel/src/process/fd_table.rs::FileHandle` and
  `kernel/src/process/epoll.rs::descriptor_events`.
- Kernel wait registration/wakeup: `kernel/src/sched/mod.rs`
  (`block_current_native_poll`, `wake_linux_pipe`, `wake_linux_unix_socket`,
  `expire_linux_poll_timeouts`).
- TCP transport bridge: `sunlight-libc/src/net_bridge.rs` maps native
  descriptors to `net_server` socket identities in a 256-entry table. Its
  lock protects only the mapping and is never held across IPC.
- Production TCP stack: `services/net_server/src/main.rs` (`WAIT`,
  `WAITERS`, `complete_waiters`) and `sunlight-net/src/tcp.rs`
  (`TcpManager::ready`, `recv`, `send`, `accept`, reaping).
- Opcode/readiness table: `ipc/src/netop.rs` (`NetOp::WAIT`,
  `NetStatus`, `NetReady`).

## Source of truth

| Condition | Authoritative source |
| --- | --- |
| Listener accept readiness | `TcpManager::ready`: `SocketKind::Listener` has nonempty `pending`; `listen` clamps backlog to `MAX_BACKLOG` (32) |
| Stream readable bytes | `TcpManager::ready`: Established/CloseWait and `socket.can_recv()` |
| Peer receive EOF | `ready`: `slot.peer_closed`, or CloseWait with no buffered bytes; `recv` returns an empty vector only after buffered bytes are drained |
| Write capacity | `ready`: Established/CloseWait and `socket.can_send()` (smoltcp TX space), not merely "connected" |
| Reset/error | `ready`: `slot.reset` -> RESET; transport failures reported as ERROR |
| Fully closed | `ready`: TCP `Closed`/`TimeWait` -> CLOSED (also EOF); local close is removed from the bridge before polling |
| Invalid/reaped peers | `WAIT` returns INVALID_SOCKET/NOT_CONNECTED when identities no longer resolve |

Important distinctions:

- `READ` and `EOF` are separate. After a FIN with unread data the stack
  reports `READ` (and may report `EOF`); `recv` still returns buffered bytes
  before the zero-length EOF result.
- `WRITE` means `can_send()`. A "connected" socket whose TX window/buffer is
  full is not writable.
- `EOF` is a peer receive half-close, not the same condition as `CLOSED`.
  Only `CLOSED` maps to `POLLHUP`; an EOF-only stream maps to `POLLIN`.
- `POLLPRI`/urgent TCP data is not implemented by this bridge and is not
  manufactured.

## Event mapping

The libc `poll` wrapper asks `net_server` for `NetReady` bits and maps:

| NetReady | poll revents |
| --- | --- |
| `ACCEPT`, `READ`, `EOF` | `POLLIN` |
| `WRITE` | `POLLOUT` |
| `RESET`, `ERROR` | `POLLERR` |
| `CLOSED` | `POLLHUP` (the stack also reports `EOF`, so this usually appears with `POLLIN`) |
| bridged identity no longer resolves | `POLLERR \| POLLHUP` (reaped/closed, not invented readability) |

Filtering follows POSIX ordering rules:

1. `POLLIN`/`POLLOUT`/`POLLPRI` are returned only when requested in
   `events`.
2. `POLLERR` and `POLLHUP` are unconditional, including `events == 0`.
3. `POLLNVAL` remains the kernel descriptor-table result and is never
   overwritten by a stale bridge result.
4. Negative `fd` values produce zero `revents` but still occupy a record.
5. The return value counts descriptors with a nonzero final `revents`.

An unbound or bound-but-not-listening bridged socket has no `NetReady` bits;
it is not given synthetic writability. An unbridged `inet_socket` retains the
existing kernel `POLLERR` result.

## Wait and wakeup mechanism

- Every poll first takes a kernel snapshot (`timeout=0`, syscall 159) so
  `POLLNVAL`, negative fds, TTY/pipe/eventfd, and non-network behavior remain
  exactly the kernel's.
- For bridged fds, libc takes a zero-timeout `NetOp::WAIT` snapshot per fd to
  collect all ready descriptors. It does not infer all-ready state from the
  single identity returned by a blocking WAIT.
- If nothing is ready and `timeout != 0`, pure-network sets block in one
  `NetOp::WAIT` for the remaining deadline. `net_server` checks readiness and
  registers the deferred reply in one server thread, and completes waiters
  after each stack progress tick, so inspection/registration cannot lose a
  wakeup.
- WAIT has a per-identity interest extension (`words[3]` bit 0 and a
  parallel interest array in the existing 4 KiB SHM page). In this mode the
  server wakes only on the requested bits; terminal bits are explicitly
  requested by poll. The legacy single-interest WAIT behavior is unchanged.
- Mixed local+network sets cannot be blocked on by one existing kernel
  primitive. They block on the kernel only for the local descriptors in a
  bounded slice (20 ms), then re-snapshot the network set. More than
  `MAX_WAIT_SET` (32) bridged fds use the same bounded re-snapshot. This is a
  bounded readiness recheck, not a busy spin.
- An infinite pure-network poll uses repeated one-second bounded WAIT calls so
  descriptor close/reuse is revalidated without an unbounded kernel deadline.

## Timeout rules

- `timeout == 0`: one snapshot, never park.
- `timeout > 0`: one absolute monotonic deadline for the whole call; IPC
  round trips and bounded slices consume it rather than extending it.
- `timeout < 0`: no deadline.
- Existing blocking I/O deadlines (`recv`/`send`/`connect`/`accept` use their
  own bounded WAIT deadlines) are independent of poll's timeout.

## Descriptor lifetime

- `socket()` clears any stale mapping for a reused fd; `close()` forgets the
  mapping before releasing the kernel descriptor.
- Poll captures `(fd, net socket identity)` for each bridged record and
  verifies the mapping before applying a result. A close/reuse observed
  during the wait restarts the snapshot instead of applying the old socket's
  events to the new descriptor. The mapping is revalidated once more before
  copy-out; more than eight consecutive stale observations fail with `EAGAIN`
  rather than looping forever.
- Kernel `POLLNVAL` wins over any stale bridge event.
- Dup/fork attachment transfer remains unsupported.

## Supported readiness matrix

| Descriptor / condition | Requested | Reported |
| --- | --- | --- |
| Listener with pending connection | `POLLIN` | `POLLIN` |
| Stream with buffered bytes | `POLLIN` | `POLLIN` |
| Peer FIN after buffered bytes are drained | `POLLIN` | `POLLIN`; `read` returns 0 |
| Stream with TX space (`can_send`) | `POLLOUT` | `POLLOUT` |
| `RESET` / transport `ERROR` | any, including 0 | `POLLERR` |
| TCP `Closed` / `TimeWait`, or local close | any, including 0 | `POLLHUP` (with `POLLIN` when EOF is also set) |
| Identity no longer resolves | any | `POLLERR \| POLLHUP` |
| Invalid kernel descriptor | any | `POLLNVAL` |
| Negative `fd` | any | zero `revents`, record still counted in `nfds` |

`POLLIN` requires the event to be requested; `POLLERR`/`POLLHUP` are
unconditional. `events == 0` therefore reports only terminal error/hangup
conditions. Return value is the number of records with nonzero `revents`.

## Explicitly unsupported

- `epoll` for bridged TCP: kernel epoll still sees `inet_socket` and reports
  `POLLERR`; this slice does not claim epoll integration. This is explicitly
  unvalidated, not merely untested.
- `select` is implemented in terms of libc `poll`. It is exercised by the
  loopback fixture's `select-integration` sub-step (a TCP stream becomes
  readable after the peer is triggered) and by the existing `poll_select`
  probe, which passed on 2 and 4 CPUs. No broader TCP `select` matrix is
  claimed beyond those checks.
- `POLLPRI`/urgent TCP data, `POLLRDHUP`, nonblocking connect/accept,
  `shutdown`, and `dup`/`fork` attachment transfer.
- Proof of `POLLOUT` under TX saturation.
- Signal interruption of poll.
- Mixed local+network sets and more than 32 bridged descriptors use bounded
  rechecks (20 ms and 1 s slices); they are not a single unbounded wait.

## Evidence summary

All paths below are under
`docs/audit/probes/native-foundation/results-2026-11-12-native-poll-tcp/final/`.
Every QEMU run used 2048 MiB; peer synchronization was explicit, not timed
sleeps.

| Run | CPUs | Result |
| --- | --- | --- |
| `loopback-2cpu` | 2 | `tcp-poll: PASS socket_live=0 listener_live=0 stream_live=0` |
| `loopback-4cpu` | 4 | same PASS |
| `loopback-2cpu-repeat` | 2 | same PASS (race-sensitive repeat) |
| `external-2cpu` | 2 | `poll-external: PASS exchanges=5 bytes=1371 ...` (default `10.0.2.0/24`) |
| `external-4cpu` | 4 | same PASS |
| `external-nonsubnet-2cpu` | 2 | same PASS (explicit `10.0.3.0/24`) |
| `external-nonsubnet-4cpu` | 4 | same PASS |

- External runs also required host-peer exit 0 and TCP frames in the NIC
  capture (`external-*/poll-summary.json`). Capture established external
  traffic; it was not used as proof of poll event ordering.
- `regress-poll-abi/` and `regress-socket-abi/`: C/C++ base, POSIX, GNU and
  reverse-include ABI gates passed, including historical source/object/ELF
  comparisons.
- `suite-2cpu/` and `suite-4cpu/`: 94 cases each, 93 PASS. The only failure
  is the pre-existing `cpp-runtime` `LINK_FAIL` (no executable to run), also
  present before this slice. The suite's `poll_select`, `poll-contract` and
  `poll-socket-contract` cases passed; the loopback fixture also passed its
  TCP `select-integration` sub-step.
- `regress-loopback-tcp-2cpu/` and `regress-loopback-tcp-4cpu/`: pre-existing
  loopback TCP acceptance PASS.
- `regress-external-echo-2cpu/` and `regress-external-nonsubnet-2cpu/`:
  pre-existing external echo acceptance PASS with byte-exact reconstructed
  streams and matching SHA-256 in both directions.
- `regress-kernel-check.log` and `regress-net-ipc-check.log`: kernel and
  net/ipc `cargo check` PASS; `sunlight-libc` `cargo check --features native-c`
  PASS (existing warnings only).
- Host `cargo test` results are classified in `../REGRESSION-NOTES.md`: the
  `secret_store` hang and seven `alloc`/`dirent` failures reproduce on a
  clean `HEAD` worktree and are unrelated to this change.

Intermediate evidence (`preflight-2cpu-multi-fail/`, `loopback-2cpu-diag*`,
`loopback-*-final/`, `pre-final-identity-check/`) is preserved unchanged.
The `FAIL` records document defects already fixed (multi-descriptor errno and
close/reuse identity handling); intermediate `PASS` records are superseded by
the `final/` runs above and are not used as the acceptance evidence.
