# AF_UNIX connect state validation and alias coherence

Scope: pathname `AF_UNIX` + `SOCK_STREAM` as implemented by
`kernel/src/process/unix_socket.rs` and dispatched by
`kernel/src/arch/x86_64/syscall.rs` (`sys_linux_connect`, native dispatcher
1051, Linux syscall 42). This document records the intended contract, the
observed behavior, and the one demonstrated defect that was fixed.

## State model

An AF_UNIX endpoint entry has `path: Option<Vec<u8>>`, `listening: bool`,
`pending: VecDeque<pair-index>` and `refs`. The reachable states are:

| State | `path` | `listening` | `pending` | Represented by |
| --- | --- | --- | --- | --- |
| Newly created / unbound | none | false | empty | `FileHandle::unix_socket` |
| Bound, not listening | set | false | empty | `FileHandle::unix_socket` |
| Listening, empty queue | set | true | empty | `FileHandle::unix_socket` |
| Listening, nonempty queue | set | true | 1..128 | `FileHandle::unix_socket` |
| Connected | n/a | n/a | n/a | `FileHandle::socket(pair, endpoint)` |
| Peer-closed / locally shut down | n/a | n/a | n/a | pipe-ring EOF/HUP; `shutdown` closes ring ends |
| Closed / reused descriptor | n/a | n/a | n/a | slot freed; fresh registration required |

`bind` attaches a path; `listen` sets `listening` and publishes the path;
`connect` allocates a pipe-backed pair, queues endpoint 1 on the destination
listener and swaps the source descriptor's backing handle to pair endpoint 0;
`accept` dequeues endpoint 1 into a new descriptor; `close` releases one
endpoint reference and, at zero, drains the queue and unregisters the path.

## Entry-point contract

| Source state | `connect(2)` result | Notes |
| --- | --- | --- |
| Newly created / unbound | proceeds to destination lookup | Linux allows connect on an unbound socket |
| Bound, not listening | proceeds to destination lookup | local bound address is discarded on success |
| **Listening** | **`EINVAL`, no mutation** | Linux `unix(7)`: "The socket must not be in LISTEN state, otherwise an error (EINVAL) is returned." |
| Connected (`is_socket`) | `EISCONN` | checked before any allocation |
| Non-socket | `ENOTSOCK` | checked before any allocation |

Destination-side outcomes (unchanged):

| Destination | Result |
| --- | --- |
| No live listener at path | `ECONNREFUSED` |
| Bound but not listening | `ECONNREFUSED` |
| Live listener, queue below cap (128) | success |
| Live listener, queue full | `EAGAIN` (bounded nonblocking contract) |
| Self path while source is listening | `EINVAL` (source check precedes lookup) |

The source-state check runs before endpoint allocation, destination enqueue
and `FdTable::replace_handle`, so a rejected attempt cannot consume queue
capacity, allocate a pair, unregister the source pathname, drain pending
connections, or disturb registrations.

## Alias / shared-description ownership

`dup`, `dup2`, `dup3`, thread `clone_boxed` and spawn `install_at`/`install_from`
all retain the endpoint (`unix_socket::retain`) and share the description index.
Aliases therefore share the Unix endpoint entry and the status flags.

| Path | Shared state | `replace_handle` effect |
| --- | --- | --- |
| `dup` / `dup2` / `dup3` of an unconnected endpoint | endpoint refs + description | swaps only the connecting descriptor's handle; releases exactly one endpoint ref |
| alias of a listening source | listener path/queue + description | with the fix, connect is rejected for every alias; nothing is released |
| close aliases in either order | refs decremented per descriptor | last close drains queue/frees entry exactly once |
| close + reuse one descriptor | fd-number-keyed | freed number can be rebound; the surviving alias keeps the endpoint |

Registrations are fd-keyed and resolve readiness from the live handle on each
collection. A connect that changes the backing handle does **not** invalidate
the descriptor's registration; a `dup2`/`dup3` descriptor replacement does
(`invalidate_replaced_fd`), because the fd number is rebound to a different
logical object.

Observed alias divergence: connecting through one `dup` alias leaves the other
alias as a valid unconnected endpoint (it can independently `bind`/`listen`/
`connect`). This differs from Linux open-file-description semantics, where
dup'd descriptors are the same socket. The local contract is fd-keyed and
explicitly excludes OFD/alias attachment transfer, so this is recorded as
reachable-but-unspecified, not changed. No ownership, double-release or
stale-readiness defect was reproduced: refcounts, close order and per-fd
registration continuity all held.

## Demonstrated defect and fix

Before this change `sys_linux_connect` accepted any `is_unix_socket` source,
including a listening socket. A listening source then connected successfully
and `replace_handle` released its Unix entry, which unregistered the pathname,
drained the pending queue (queued peers observed EOF) and installed a
connection at the destination. This violated the advertised Linux AF_UNIX
compatibility (`unix(7)` EINVAL) and destroyed listener state.

Fix: reject a listening source with `EINVAL` immediately after resolving the
source index and before `find_listener`, `alloc_socketpair`, `enqueue` or
`replace_handle`. The check reuses `unix_socket::is_listening` under the
existing `SCHEDULER -> Unix pool` lock order. No registration is cleared, no
lock order changes, no public export is added.

Residual limitation: `listen` mutates the Unix entry without holding
`SCHEDULER`, so a concurrent `listen` on a cross-thread alias could race the
source-state check. That is a pre-existing general bind/listen atomicity gap,
not introduced or fixed here; serial state transitions are fully validated.

Evidence: `docs/audit/probes/native-foundation/results-2026-11-14-af-unix-state-validation/REPORT.md`.
