# FastLane F1.2: SPSC ring foundation

F1.2 implements an independent, dependency-free `no_std` library in
`fastlane-ring/` (`sunlight-fastlane-ring`). Host validation and bare-metal target
compilation pass. No native channel, mapping syscall, service integration or
kernel security behavior changes. This is a data-plane component, not a native
IPC transport. [R4.1](fastlane-f11pr41-mint-delivery.md) and F1.1B-0 remain
blocked at their documented service-policy and native lifecycle assertions.

## Audit and API

The kernel `ipc/mod.rs::EndpointQueue` uses `VecDeque` under the bus's existing
synchronization. audiod's `PcmQueue` and `capture::CaptureQueue` use exclusive
mutable byte/period queues, and `ipc/src/pty.rs::PtyBuffer` is a separate TTY
buffer. None provides this SPSC publication/ownership contract. Existing SHM
allocation/retirement in `kernel/src/memory/shared.rs` is not a secure per-peer
mapping mechanism. See [F1](fastlane-f1.md) and [F1.1](fastlane-f11-grants.md).
The workspace defaults to `x86_64-unknown-none`; host commands explicitly select
`x86_64-unknown-linux-gnu`. No runtime or development dependency was added.

`Ring::<CAPACITY, PAYLOAD>::new(nonzero_generation)` initializes typed owned
storage. `split(&mut self, expected_generation)` yields one Producer and one
Consumer borrowing that storage. `try_send(&mut self, &[u8])` and
`try_recv(&mut self, &mut [u8])` perform one bounded attempt. There are no loops,
allocations, waits, notifications or scheduler interactions in these operations.
A caller chooses its retry/wait policy. Handles are Send, not Sync, not Clone;
operations require exclusive handle access. Dropping a handle and splitting
again after both borrows end resumes existing cursors and queued messages.

Errors distinguish Full, Empty, PayloadTooLarge, OutputBufferTooSmall,
InvalidChannel, InvalidFrame and layout/generation/readiness failures.
Empty messages are supported. Too-small destinations leave the message queued.
Malformed lengths leave the slot queued and return InvalidFrame repeatedly;
the owner must quiesce and retire that generation. Invalid cursor distances
also fail closed. No partial receive or silent discard API is provided.

## ABI version 1

Supported ABI is little-endian x86_64 with native lock-free 64-bit atomics.
Other architectures are compile-time rejected. All shared structures use
`repr(C)`; page regions additionally use `align(4096)`. No pointer, reference,
heap owner, process address, or pointer-sized integer appears in shared storage.
References exist only in process-local handles. AtomicU64 is eight bytes aligned
to eight on this target. Cursor pages start 4096 bytes apart, exceeding the
64-byte cache-line separation relevant to this initial target.

| Region | Offset | Contents | Future producer / consumer rights |
| --- | ---: | --- | --- |
| Metadata | 0 | Header at 0, ready AtomicU64 at 64 | RO / RO after owner setup |
| Producer cursor | 4096 | head AtomicU64 at region offset 0 | RW / RO |
| Consumer cursor | 8192 | tail AtomicU64 at region offset 0 | RO / RW |
| Payload | 12288 | fixed array of slots | RW / RO |

Each cursor region occupies one page. The payload region and whole Ring round
up to 4096 bytes. Fields within the 64-byte Header are:

| Field | Byte offset | Type |
| --- | ---: | --- |
| magic | 0 | u64 = 0x53554e464c523031 |
| version | 8 | u32 = 1 |
| capacity | 12 | u32 |
| payload_size | 16 | u32 |
| slot_stride | 20 | u32 |
| generation | 24 | u64, nonzero |
| producer_offset | 32 | u64 = 4096 |
| consumer_offset | 40 | u64 = 8192 |
| payload_offset | 48 | u64 = 12288 |
| total_size | 56 | u64 |

Slot: length u32 at offset zero, payload `[u8; P]` at offset four,
4-byte alignment, stride `align_up(4 + P, 4)`. Slots live inside UnsafeCell.
Capacity N must be a power of two in 1..=65536; P must be 1..=4096.
Total size is `align_up(12288 + N * stride, 4096)`. Header calculations use
checked u64 additions/multiplication and reject unsupported inputs before
construction. Maximum total size is 268709888 bytes (N=65536, P=4096).
Construction additionally checks actual Rust size/alignment; tests verify
region offsets and a non-multiple-of-four payload's slot stride.

Padding is not serialized data and is never read by the library. Initializing
fields does not promise initialized Rust padding. A future backing allocator
must sanitize every backing page before exposure; do not serialize this object
by reading its representation as a byte slice.

These contiguous, page-aligned logical ranges can be independently mapped from
one backing object using disjoint page ranges. Alternatively a future adapter
may use separate backing objects and process-local region pointers; the current
handles intentionally have no such constructor. Mapping only subsets with the
current `&Ring` handles is unsupported. Host page alignment establishes no RO/RW
protection. Kernel policy must prevent writable aliases, including owner and
legacy SHM mappings, before any protection claim is valid.

## Initialization and retirement

A trusted owner selects N, P and a nonzero generation before sharing. `new`
validates the configuration, initializes header, atomics, every slot length and
every payload byte to zero, checks representation size, then Release-stores the
generation to ready. Returning the owned value and establishing thread borrows
is the host setup protocol. `split` Acquire-loads ready and compares every header
field against the exact expected configuration and generation. A zero ready
value rejects cancelled or retired setup. Wrong versions, offsets, strides,
alignments encoded by offsets, lengths and sizes reject validation.

`Header::validate` operates on an already valid typed Header value. It is not a
parser that turns arbitrary bytes into a Ring. There is no raw attach API and
no conversion of untrusted memory into atomic references. Rust enforces actual
object alignment. Tests simulate incomplete initialization by clearing ready
under exclusive fixture ownership; construction itself is synchronous.

`retire(&mut self)` clears ready only after all handles and operations end.
It is quiescent retirement, not active revocation. Existing handles cannot
coexist with safe retirement, destruction or generation replacement. A stale
expected generation fails readiness/header checks. The owner must not reuse a
generation for a replacement channel: the library does not allocate globally
unique generations. Generations are informational, never authorization.

A future unsafe attachment API would require initialized correctly aligned
atomics and typed storage, exact size/layout, appropriate rights, persistent
mapping lifetime, a unique producer/consumer, no concurrent teardown, immutable
header, no adversarial concurrent slot/cursor writes and authenticated expected
generation. None of these kernel obligations is enforced by a host fixture.
A hostile producer racing payload reads is outside Rust's cooperative SPSC
contract; length checks alone do not make arbitrary shared-memory attacks safe.

## Cursor arithmetic and memory ordering

Head counts published messages; tail counts consumed messages, modulo 2^64.
Initially both are zero. Producer owns head, consumer owns tail. Each loads its
own cursor Relaxed, and the peer cursor Acquire. Distance is
`head.wrapping_sub(tail)`. Zero is empty, N is full, greater than N is invalid.
Index is `cursor & (N - 1)`; increment uses wrapping_add(1).

Inductively the logical outstanding distance starts at zero, increases only
when less than N and decreases only when nonzero. Thus it stays in [0,N]. Since
N <= 65536 < 2^63, this interval has a unique representation in modulo-2^64
subtraction, including rollover. N divides 2^64, so masked indices also remain
correct through rollover. A peer cannot lap a stopped endpoint: bounded
capacity stops it first. Atomic read-read coherence and endpoint program order
prevent observations from regressing behind previously observed peer progress;
a stale observation may conservatively return Full/Empty, not authorize reuse
or consumption beyond the invariant. Corrupted cursors are outside the proof;
distance checks detect some corruptions, not every forged plausible state.

Producer writes payload and length using raw pointers into UnsafeCell, then
Release-stores head. Consumer's Acquire head observes all writes preceding
that publication before reading the published slot. Consumer validates length,
copies only that length, then Release-stores tail. Producer's Acquire tail
orders all completed consumer reads before reuse. The consumer never releases
a slot on a short-output or malformed-frame error. Trailing payload bytes may
retain older data internally but are never returned. The payload copy is not
atomic; exclusive slot ownership makes that unnecessary.

Rust Acquire/Release constrains compiler motion and establishes happens-before
across threads. x86 cache coherence supplies coherent atomic cursor visibility;
x86 ordering does not replace compiler synchronization or provide message-wide
atomic copies. No SeqCst or independent ad hoc CPU fence is needed. This contract
provides publication and reclamation only, with no lost-wakeup or scheduling
guarantee. This is bounded lock-free SPSC under its cooperative ownership model,
not a promise that a process will be scheduled or that memory copies have a
fixed cycle count.

## Unsafe and ownership audit

Only the Ring Sync implementation and slot pointer reads/copies/writes are
unsafe. Fields are private; no unrestricted storage accessor exists. `split`
requires an exclusive borrow for the lifetime of both unique handles. Handles
cannot be cloned, shared for concurrent operations, or outlive the Ring.
Separate slot UnsafeCells allow disjoint slot access without manufacturing an
exclusive reference to the entire payload array. No overlapping mutable Rust
references are created, including during concurrent operations. Input/output
slices cannot safely alias private ring storage. The initialized slot lengths
are read only after publication, and the writer cannot modify that slot until
reclamation. Acquire/Release ordering therefore removes concurrent read/write
races for each slot. The Ring's Sync implementation does not authorize an
additional role handle. Read-only header access never touches mutable payload.

## Validation and measurements

Raw evidence: `target/fastlane-f12-evidence/`; machine-readable status in
`validation-summary.json`. Five grouped tests cover configuration/header
mutations, cancelled setup, retirement, repeated teardown, zero/one/small/max
messages, FIFO, untouched destination suffixes, full/empty, insufficient output,
partial full-ring recovery, index reuse, invalid frames/cursors and u64 rollover.
Capacities 1, 2, 8 and 256 exercise sequential operation; maximum allowed
configuration is size-validated without allocating a 256 MiB test object.

Concurrent deterministic byte/sequence checks exchanged 1,130,000 messages:
10k at (1,64), 100k at (2,256), 1M at (64,64), 10k at (8,1024), and 10k at
(2,4096). Producer/consumer yields alternate which endpoint is slowed. The
validation command uses a 60-second external timeout. These tests check FIFO,
loss, duplication, corruption and reuse; stress is not an exhaustive ordering
proof. The unsafe and ordering argument above remains necessary.

Miri is not installed; its availability error is retained. Loom is not an
existing dependency/tool in this workspace and was not added or run. No model
checker or dynamic memory-safety pass is claimed. Workspace formatting still
fails on existing files (raw log retained); changed Rust files pass their own
rustfmt check. Prior R4.1's missing media WAV fixture remains unresolved; media
and native regressions were not rerun because no production route changed.

Host benchmark: `cargo run -p sunlight-fastlane-ring --example bench --release
--target x86_64-unknown-linux-gnu`. Release uses opt-level 3, LTO and one codegen
unit. Exact rustc, host OS/CPU are recorded in compiler/cpu/environment logs.
Each configuration uses 10k single-thread warmups, 100k uninstrumented pairs,
100k timestamped pairs, 100k empty/full attempts and a 100k-message two-thread
warmup before 100k measured transfers. Thread creation is outside the timed
transfer interval. Caller busy retry is benchmark-only. No logging occurs in
timed operations. No CPU affinity or scheduler isolation was applied.

| Capacity | Payload bytes | Single-thread Mmsg/s | Two-thread Mmsg/s | Pair p50/p95/p99 ns |
| ---: | ---: | ---: | ---: | --- |
| 2 | 64 | 123.04 | 9.97 | 22 / 30 / 31 |
| 64 | 64 | 254.85 | 14.72 | 15 / 18 / 26 |
| 2 | 256 | 167.41 | 6.87 | 19 / 25 / 27 |
| 64 | 256 | 190.85 | 13.12 | 16 / 21 / 28 |
| 2 | 1024 | 57.76 | 5.53 | 28 / 32 / 36 |
| 64 | 1024 | 51.32 | 10.35 | 34 / 44 / 52 |
| 2 | 4096 | 14.85 | 2.36 | 78 / 91 / 93 |
| 64 | 4096 | 10.73 | 6.51 | 93 / 112 / 117 |

Raw output also records payload bytes/s: message rate times P, counting each
message once. Empty receive averaged 0.71–0.75 ns; full send 0.50 ns in this
optimized host loop. Sub-nanosecond amortized values are throughput estimates,
not isolated-call latency. Pair percentiles include clock overhead, refer to
single-thread enqueue+dequeue and are not two-thread round trips. The counting
global allocator measured/asserted zero allocations in each single-thread
steady-state batch; two-thread allocator counts are not separately claimed.
Values vary with host load and compiler optimization. These are host Ring
measurements, never FastLane IPC, native speedup, kernel-crossing, CPU-saving
or battery measurements.

## Future integration contract

The ring independently guarantees bounded framing, FIFO, non-overwrite,
non-consuming short-output errors and ordered cooperative slot ownership.
It guarantees no authentication, authority, wait/wakeup, closure, crash recovery,
page permissions or physical lifetime.

Before native use, the kernel/channel layer must provide:

- Restricted backing provenance; recipient/incarnation-specific grants;
  producer RW/consumer RO payload, each role's cursor rights and immutable
  metadata; NX mappings and prevention of legacy SHM/owner writable aliases.
- Generation-safe lifetime, active revocation, process-exit cleanup, exact
  mapping teardown and acknowledged TLB invalidation before frame reuse.
- Authenticated peer setup, trusted owner initialization, layout and expected
  generation verification, committed readiness publication and protected
  mapping lifetime throughout every operation.
- F1.3 kernel-authorized Doorbell, notification sequence, conditional wait and
  lost-wakeup prevention; peer-death/closure wakeups, plus capacity/backpressure
  notification where needed. Ring cursor observations alone cannot implement
  a race-free sleep transition.

Failure policy must define producer/consumer exit, closure with queued data,
stale mappings and generation replacement. A producer dying before head
publication leaves its partial slot unpublished; after head publication it is
complete under this contract. A consumer dying before tail publication leaves
the message unreclaimed. Neither endpoint may be replaced by blindly resetting
cursors. Quiesce/revoke old mappings before generation replacement; define
whether queued data drains or is discarded on close and wake every waiter.
The host retirement API cannot implement these failure policies.

Native validation: none. Deferred: raw mapping attachment, restricted grants,
Doorbell, cross-process transport, Audio migration and end-to-end IPC comparison.
Blocked prerequisites remain exactly those in R4.1/F1.1B-0; F1.2 does not attempt
to repair them or expose a restricted native channel.
