# Native POLLOUT send-backpressure, recovery and peer termination

**Date:** 2026-11-12
**Scope:** `poll(POLLOUT)` readiness for bridged `AF_INET`/`SOCK_STREAM`
descriptors under sustained **send** backpressure, its recovery, and peer
termination while a writer waits for capacity.
**Status:** implemented in fixtures/orchestration only; the production path
needed no change. Guest-validated on 2 and 4 CPUs plus a race-sensitive repeat.
Evidence is under
`docs/audit/probes/native-foundation/results-2026-11-12-native-poll-tcp-pressure/`.

This milestone continues
`docs/networking/2026-11-12_NATIVE_POLL_TCP_READINESS.md`. It does **not**
extend epoll, UDP, async connect, `shutdown`, or general nonblocking sockets.

## The actual WRITE readiness predicate

`NetReady::WRITE` is produced only by
`sunlight-net/src/tcp.rs::TcpManager::ready`:

```rust
match socket.state() {
    tcp::State::Established | tcp::State::CloseWait => {
        ...
        if socket.can_send() {
            ready.insert(SocketReady::WRITE);
        }
        ...
    }
    _ => {}
}
```

`smoltcp`'s `tcp::Socket::can_send` is `may_send() && !tx_buffer.is_full()`.
So the predicate is:

1. the transmit half is open (`Established` or `CloseWait`), **and**
2. the local smoltcp transmit buffer is not full.

### What capacity it represents

It is **local transmit-buffer occupancy**: the 4 KiB smoltcp `TX_BUF`
allocated per socket in `TcpManager::alloc_socket`. It is *not* the remote
receive window, the intervening QEMU slirp/socket buffers, or NIC completion.
The host/slirp path can keep absorbing bytes after the local buffer is empty
and the remote application has stopped reading, so `can_send()` becoming false
requires the *whole path* to be full. Conversely, a remote RCVBUF/slirp
recovery can free local TX space without any application read (see "Scheduling
races" below).

### WRITE readiness vs. native send/write

`sunlight-libc/src/net_bridge.rs::send` drives `NetOp::SEND_SHM`. On
`NetStatus::WOULD_BLOCK` it returns `EAGAIN` for a nonblocking call
(`MSG_DONTWAIT` or `O_NONBLOCK`), otherwise it blocks in `wait_ready(...,
NetReady::WRITE, IO_TIMEOUT_MS)` and retries the transfer. `poll(POLLOUT)`
reports exactly the same predicate that makes a subsequent blocking `send`
return promptly.

### Partial writes

`send` is a loop over 4 KiB SHM chunks. `TcpManager::send` calls
`socket.send_slice`, which enqueues as many bytes as fit and returns that
count; `Ok(0)` maps to `WouldBlock`. So a single `send` may transfer fewer
bytes than requested, and the native `send` returns that short count. A
caller must treat a short return as progress, not as an error.

### How a full transmit buffer changes readiness

When the 4 KiB `TX_BUF` fills, `can_send()` is false and `ready` omits
`WRITE`. `poll` therefore reports no `POLLOUT` (for a `POLLOUT`-only request,
`revents == 0`), and a blocking `send` blocks in `WAIT`.

### What triggers waiter reevaluation

`net_server`'s main loop runs, every `NETWORK_TICK_MS` (10 ms) and after each
IPC message:

1. `progress_network` → `TcpManager::progress` → `iface.poll(...)`, which
   pushes queued TX to the device, processes ACKs/window updates, and thereby
   frees `TX_BUF` space; then
2. `complete_waiters`, which re-evaluates every registered waiter's interest
   and completes those whose readiness changed or whose deadline expired.

So recovery is driven by ordinary stack progress, not by the waiter's own
polling: a `WAIT` registered while `can_send()` is false is completed by the
tick that observes the space freed (directly, or via
`first_ready_per_identity`). There is no aggressive polling inside `poll`
itself for a pure network set; it parks in one `WAIT`.

### Peer FIN, reset and terminal errors

- **Orderly FIN (receive half-close):** smoltcp moves to `CloseWait`, where
  `may_send()` is still true. `ready` keeps reporting `WRITE` (and `READ`
  while bytes remain). A receive-side FIN must **not** make the local send
  direction unusable; the fixture verifies a `send` still succeeds after FIN.
  `ready` only emits `EOF` once the receive buffer is drained.
- **RST / transport error:** smoltcp moves to `Closed` and clears the tuple.
  `ready` maps `Closed`/`TimeWait` to `CLOSED | EOF` and the bridge maps
  `CLOSED` to `POLLHUP`, plus `NetReady::EOF` → `POLLIN`. The tcp manager
  never sets `slot.reset` today, so a peer RST surfaces as the connection
  becoming `Closed` (`POLLHUP`), not as an invented `POLLERR`. A subsequent
  write fails with the real send error. The fixture records the exact bits
  and errno rather than asserting a specific one.
- **Local close** removes the mapping before the kernel descriptor is
  released, so the fd reports `POLLNVAL` after reuse (unchanged from the
  readiness milestone).

### The operation that safely creates sustained backpressure

A connected, *already-supported* peer that withholds application reads. The
fixture uses a second TCP connection as an out-of-band control channel and
fills the data path with `send(..., MSG_DONTWAIT)` (an already-supported
mechanism, no new nonblocking support) until `can_send()` is false. `MSG_DONTWAIT`
was already implemented for bridged TCP send/recv; `O_NONBLOCK` on an
already-connected stream via `fcntl(fd, F_SETFL, ...)` is also honoured
(`fd_nonblocking`). The fixture uses only `MSG_DONTWAIT`.

## Readiness transitions proved

| Step | Observation |
| --- | --- |
| A. Initial capacity | `poll(POLLOUT)` reports `POLLOUT`; an 8-byte `send` succeeds |
| B. Sustained backpressure | nonblocking fill reaches `EAGAIN`; `poll(POLLOUT, 0)` is 0; `poll(POLLOUT, 800 ms)` times out (no capacity) |
| C. Recovery | after a control-channel drain release, a waiting `poll(POLLOUT)` returns before its deadline, and the fill continues |
| D. Repetition | the fill/drain transition is repeated ≥ 3 times (typically 5 external, ~70 loopback), exercising waiter re-registration |

The peer validates the complete stream: 8-byte big-endian length plus
`payload[i] = (i*31 + 7) & 0xff`, checked byte-exact and by an independently
recomputed FNV-1a 64 hash.

## Scheduling races

The intervening QEMU slirp + socket buffers can free local TX space without any
application read. The fixture therefore treats a `POLLOUT` observed *between*
the `EAGAIN` and the poll snapshot, or a finite poll that returns before its
deadline, as `unstable` (capacity legitimately changed) and re-establishes
saturation, rather than failing the readiness contract. `unstable` is
reported but not fatal; the transition must still be observed
`REQUIRED_CYCLES` times.

## Peer termination

- **Reset while waiting for capacity** (external only): the peer closes data2
  with `SO_LINGER(0)`, sending a real RST. The blocked `poll(POLLOUT)` ends
  within its 3 s deadline and returns `POLLHUP` (`0x10`); the following
  nonblocking send fails with `ECONNREFUSED` (`111`). The fixture asserts
  *some* `POLLERR`/`POLLHUP` bit and a negative send, and records the exact
  values for the report.
- **Orderly FIN** (external): the peer `shutdown(SHUT_WR)`s the data socket.
  `poll(POLLIN)` reports readability, `recv` returns 0 (EOF), and a subsequent
  `send` still succeeds and reports `POLLOUT`. FIN is not labelled RST and no
  immediate write failure is required from a receive-side FIN.

### Loopback RST limitation

Loopback covers saturation/recovery/repetition only. An in-guest RST cannot be
manufactured with the supported native surface: native libc has no
`setsockopt`/`SO_LINGER`, and `shutdown` returns `EOPNOTSUPP`. Peer reset is
therefore exercised **external only**; no fake reset path was added.

## Explicitly not claimed

- Full POSIX socket semantics.
- Arbitrary writable capacity after `poll`; the readiness is a snapshot and a
  short write is still possible.
- Saturation support from a single successful write (the fixture proves a
  sustained false predicate, a timed-out wait, and recovery).
- General nonblocking sockets; only `MSG_DONTWAIT` and the already-supported
  `O_NONBLOCK` send/recv path are used.
- TCP epoll integration; still unbridged.
- That local transmit-buffer occupancy equals the remote receive window or NIC
  completion.
- Signal interruption of `poll`.
