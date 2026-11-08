# FastLane bulk data plane: initial interface

Status: **DRAFT INTERFACE**. November 8, 2026. Production transport remains
disabled; application wiring and native stream/termination validation are
NOT_RUN for this draft. T4B remains **PARTIAL (Deferred to Signal Subsystem
Epoch)** and is not a prerequisite for this interface.

## Control and data contract

Rappid Rabbit rendering and Melody Mina audio are the first intended consumers
(the task's “Melody Mine” refers to `melody-mina/`). Play, pause, seek,
navigation, stream setup, acknowledgements and close remain ordinary synchronous
kernel call/reply messages. Commands keep the existing authenticated endpoint,
capability and reply-generation checks. A ring slot carries bulk bytes, never
authority to execute a control command. Control callers can block under the
existing synchronous contract; this draft needs no asynchronous call-start ABI.

Each unidirectional stream uses one bounded SPSC ring. Rendering sends bounded
tiles/chunks; PCM sends bounded audio blocks. A reverse stream requires a
separately authorized ring. Queue full returns bounded backpressure; queue empty
returns immediately. The application must negotiate its overflow/underrun
policy and PCM format or tile geometry through the control plane. Seek/format
replacement closes the old stream and negotiates a fresh channel generation;
it must not reinterpret queued bytes using new geometry. Zero jitter is a goal
requiring measured scheduling, buffering and device output, not a property this
layout or its host tests establish.

## Interface and SHM layout

`sunlight-fastlane-ring::data_plane::Plan` extends the existing `no_std` ring
crate. It calculates geometry using `Header::for_config`, rejects invalid
configuration/generation and enforces a caller-supplied page budget bounded by
`MAX_STREAM_PAGES = 64` (256 KiB including metadata/cursors). It contains no
allocator, syscall, futex or signal dependency. `Producer::try_reserve` and
`Consumer::try_read` already provide cooperative
zero-copy leases with explicit publication/release and bounded Full/Empty.

```rust
use sunlight_fastlane_ring::data_plane::{Plan, Role};

let plan = Plan::for_ring::<16, 256>(1, 5).unwrap();
assert_eq!(plan.bytes(), 5 * 4096);
let producer_requirements = plan.regions(Role::Producer);
let consumer_requirements = plan.regions(Role::Consumer);
// A future authenticated SHM adapter must enforce these requirements.
// No raw-pointer attachment is provided by Plan.
```

The layout reuses the cooperative ABI, not the experimental hostile ring's
different representation. All offsets and lengths are page-aligned and cover
the full object. Every page is non-executable.

| Region | Offset | Producer | Consumer |
| --- | --- | --- | --- |
| Immutable header/ready metadata | 0 | Read-only after initialization | Read-only |
| Producer cursor | `SHM_PAGE` | Read/write | Read-only |
| Consumer cursor | `2 * SHM_PAGE` | Read-only | Read/write |
| Slots and lengths, rounded to pages | `3 * SHM_PAGE` | Read/write | Read-only |

`ipc/src/lib.rs` defines `SHM_PAGE = 4096`. `shm_alloc()` allocates one such page;
the complete multi-page plan needs `shm_create(plan.bytes(), 0)`, or an adapter
over separately allocated pages. Received kernel SHM capabilities use `shm_map`
or `shm_map_sized`; `Plan::check_mapping_size` rejects a kernel-reported length
that differs from trusted local geometry. Shared header values are not trusted
to set bounds or permissions. Page size parity with the kernel is part of the
adapter acceptance gate.

Generic `shm_map` does not express the per-region permissions above. Setup must
use/extend the existing restricted kernel mapping path to enforce them, remove
initial writable metadata aliases and bind the exact authorized roles. Passing
a generic SHM token and casting its pointer to `&Ring` is not an implementation
of this contract. The draft deliberately provides no such cast or allocator.

Cooperative leases require peers that obey slot ownership. Producer-writable
payload pages still allow mutation after publication; cursor permissions do
not enforce coherent publication for hostile peers. Existing `hostile` APIs
provide bounded private copies with explicit integrity limits. Untrusted native
zero-copy consumption remains gated on enforced immutable publication/slot
ownership, lifecycle safety and an approved security contract.

## Bounded grants and retirement contract

The existing capability broker and authenticated channel identities remain the
authority source (`kernel/src/capability/channel.rs`, `channel_native.rs`,
`channel_mapping.rs`, and `delivery::RecipientIdentity`). A local `Plan`, PID,
generation number or shared readiness flag grants no mapping authority.

The adapter must reserve a stream's entire page budget before allocation and
deny admission on exhaustion. Draft aggregate limits are eight live streams per
client (at most 512 pages) and 32 per service (at most 2048 pages), counting
retiring streams until physical cleanup completes. These aggregate quotas are
requirements for the future adapter, not counters implemented by `Plan`.
Grants bind channel generation, operation, producer/consumer PID plus
incarnation and VM generation, and exact byte/page range. There is no implicit
cross-client delegation or raw SHM fallback on failed authorization. Failed
setup unwinds every partial mapping/allocation and returns its reservation.

Close, client termination, peer death, successful exec, session withdrawal and
revocation must converge on this bounded retirement sequence without requiring
client cooperation:

1. Retire admission/publication for the exact channel generation and reject
   stale capabilities. Resolve pending control calls once through the existing
   terminal outcome path, and release any registered wake resources.
2. Stop local access and remove all authorized views, including aliases in
   surviving peers. Respect scheduler/address-space ownership and existing lock
   order; never wait for a dead client's lease destructor.
3. Complete local/remote TLB invalidations before dropping the final backing
   reference; return frames through the ordinary SHM last-view release path.
4. Release quota/channel slots only after mappings, references and backing are
   reconciled. Deferred reap retains its reservation; it must not reuse an
   uncertain generation or free still-accessible frames.

Existing anchors are `fastlane_mapping_sweep`/`fastlane_drain_mappings`, the
scheduler reap call to `cleanup_shared_pages`, and `free_shared_page`'s
synchronous unmap/last-reference release. Restricted FastLane mapping and
channel adapters are debug feature-gated today. Ordinary SHM exit cleanup alone
does not establish surviving-peer revocation for a production stream. Those
paths need native client-kill/peer-survival, stale-token, exec and remote-TLB
evidence before application enablement. Metadata zero alone is not proof of
physical reclamation.

## Verification boundary

Host tests check storage geometry against the existing `Ring`, permission
requirements, invalid configurations, page-budget exhaustion and mapping-size
rejection; existing ring tests cover cooperative lease behavior. Bare-metal
checking establishes that this interface builds without futex cancellation
stubs. Scoped Clippy and formatting cover the ring crate only. The current
T4B host baseline and exact check commands/results are retained in
`.threading-evidence/t4b/20261108T175345.242506Z-deferred-boundary/`.

No application stream, native SHM adapter, aggregate quota enforcement,
client-kill cleanup, production enablement, rendering throughput, audio capture
or jitter result is claimed by this deliverable. These are the next integration
gates; prior FastLane milestone failures and PARTIAL decisions remain intact.
