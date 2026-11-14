# Native TCP level-triggered epoll

This implements native C `epoll_ctl`/`epoll_wait` for production AF_INET TCP
listeners and connected streams. It supersedes the epoll exclusions in the
November 12 TCP bridge and poll-readiness documents. The TCP stack remains in
net_server. The implementation/acceptance record is
`docs/audit/probes/native-foundation/results-2026-11-12-native-tcp-epoll-lt-implementation/REPORT.md`.

## Ownership decision

Option A was selected: retain kernel registrations and add a narrow native
observation protocol. Option B (libc aggregation owning registrations) would
require a second lifetime/registration authority, synchronization with raw
kernel close and local ET/oneshot state, and independent event.data storage.
It is unnecessary here. Libc holds only a temporary stack snapshot; the
existing fd-to-transport attachment map is not an epoll registration table.

Syscall 177's wire format and argument contract are in
`ipc/src/native_epoll.rs`. Native ctl still parses the guest's aligned record
in the kernel. The kernel alone stores masks, eight data bytes, registration
tokens, transport identities, and the fairness cursor. Snapshots carry no user
event data. Collect returns the kernel's current data, never data supplied in
an observation. Native event size/alignment/data offset are 16/8/8. Linux's
separate 12-byte packed event path remains separate.

Every successful ADD or MOD assigns a globally distinct registration token.
Close and DEL remove that token. An external observation must match fd, token,
and the registration's transport identity. TCP identities themselves include
the net_server generation, not just its reusable slot number. Native ctl
captures a kernel inet-endpoint generation before reading the attachment map;
the kernel rechecks that generation with registration mutation under
SCHEDULER. A close/reuse during ctl cannot install the old transport on the new
endpoint. A registration can precede bind/connect: an unbound placeholder is
inert, and its first matching observation binds its transport identity in the
kernel. Merely being an inet placeholder never produces an epoll POLLERR.
Native poll's existing unconnected-socket behavior is unchanged.

The collector holds SCHEDULER and the epoll pool lock while validating and
selecting events. MOD/DEL that complete before collection invalidate old
observations. Concurrent operations linearize at this selection; an event
already selected before a concurrent DEL may finish copying out. No promise
of retracting already selected events is made. No spinlock spans NetOp IPC or
sleep. User copy helpers run after dropping SCHEDULER.

## Waiting, capacity, fairness

Each iteration snapshots every inet registration (maximum 64), takes a
zero-timeout production NetOp WAIT observation for each attached identity,
rechecks the libc attachment, and submits observations to the kernel. The
kernel combines them with current local readiness and filters ordinary IN/OUT
against its current mask. ERR/HUP are unconditional. Scanning one identity per
IPC avoids silently truncating registration sets at NetWait's 32-identity
limit and collects every ready identity, not just WAIT's first response.

A positive timeout has one monotonic deadline covering all IPC, scans and
sleep slices. Timeout zero takes one scan. Negative timeouts loop without a
deadline; an empty slice never becomes an application timeout. Empty scans
sleep at most 20 ms (or the remaining deadline), then revisit both local and
network domains. This is bounded rechecking, not a unified wake primitive or
a busy spin. Scan/IPC overhead adds to revisit latency: the 20 ms bound is the
sleep bound, not a hard end-to-end latency guarantee. An individual zero-time
NetOp IPC has a 100 ms transport watchdog. Errors propagate instead of being
reported as success or readiness.

One rotating fd cursor covers both domains. A call starts strictly after the
last delivered fd and wraps once. When maxevents is smaller than the ready
set, subsequent calls eventually return later ready registrations. Local ET
and oneshot state are still maintained by the same kernel collector.

Limits remain 64 registrations per instance, 32 instances, 256 process fds,
128 net sockets, and net_server's existing 1 MiB heap. A socket consumes RX/TX
buffers; a listening socket plus a client and pending accepted endpoint uses
three network slots. Capacity acceptance uses 40 listeners and eight pending
connections (plus a local ready socket), rather than exceeding that heap.

## Public contract and boundaries

- Listener pending connections, buffered receive data, partial/full drain,
  writable capacity, TX saturation and recovery use production NetReady.
- READ/ACCEPT/EOF map to requested IN; WRITE maps to requested OUT. EOF alone
  does not create HUP. RESET/ERROR map to ERR; CLOSED maps to HUP. A reaped
  attached identity uses the existing poll bridge's ERR|HUP mapping.
- No RST mapping was added. The external reset fixture records the actual
  existing result, including HUP and subsequent write errno 111 in the runs.
- TCP EPOLLET and EPOLLONESHOT are rejected with EINVAL on ADD/MOD, including
  before attachment. Existing local-fd modes remain available.
- ADD/MOD/DEL, full-width data, multiple independent epoll instances observing
  one socket, zero/positive/infinite waits, and shared-native-thread MOD/DEL
  are supported. Capacity exhaustion currently reports EMFILE.
- This is the native libc API. Raw syscall 173 alone cannot obtain userspace
  TCP observations. Linux-compat TCP epoll and nested TCP epoll visibility via
  poll/select on an epoll fd are outside this contract.
- Existing exclusions remain: UDP, TCP urgent data/PRI, RDHUP, nonblocking
  connect/accept, shutdown, and signal interruption of epoll_wait.
- This does not add TCP attachment transfer through dup/fork. FdTable has
  shared open descriptions for offset/status, but the libc transport map is
  per fd and net_server ownership is per PID. Close removes fd-keyed interests
  from epoll queues held by that descriptor table. It is not Linux OFD epoll
  lifetime behavior. Duplicated/fork-inherited epoll handles also lack a
  reference-counted instance lifetime (`close_file_handle` frees the pool
  instance); those handles are not claimed as supported queue aliases.
  Native std shared-descriptor workers use current_shared_process and are
  covered by the concurrent-ctl fixtures. Concurrent connect/bind/close on
  the same fd is not newly specified by this readiness work.
- Descriptor replacement (`dup2`/`dup3`) is specified separately in
  `2026-11-13_NATIVE_DUP2_REPLACEMENT_IDENTITY.md`: it invalidates the
  replaced fd's registration and its transport attachment, and the native C
  surface still exposes no duplication entry point.

Historical fixtures and evidence remain unchanged. New pressure adapters
include the existing pressure clients and replace only their readiness call
with a persistent native epoll registration; the original pressure assertions,
byte-integrity protocol, bounded peers and cleanup checks remain in force.
