# FastLane zero-copy slot leases

This increment implements direct payload access in `sunlight-fastlane-ring`.
It is a cooperative SPSC building block. Production FastLane remains **BLOCKED**
by the security gates recorded in `fastlane-security-closure-progress.md`.
No native channel establishment, mapping, authority, scheduler, syscall or
service transport is changed. Existing worktree changes are preserved.

## API and publication

`Producer::try_reserve(len)` returns a `WriteLease` borrowing the unique producer.
It bounds-checks len, cursor distance and capacity before touching the slot.
`payload_mut()` exposes exactly len bytes, initialized to zero on every
reservation. Encode directly into that slice; consuming `commit()` writes the
length and Release-publishes head. Dropping or forgetting a lease publishes
nothing. Cancellation and unwinding permit retry of the same unpublished slot.

`Consumer::try_read()` Acquire-observes head, validates cursor distance and
length, and returns a `ReadLease` borrowing the unique consumer. `payload()`
borrows the actual slot bytes. Consuming `release()` Release-publishes tail,
allowing producer reuse after its Acquire tail. Dropping or forgetting a read
lease leaves the message queued. No automatic consume during unwinding occurs.
An outstanding reader can intentionally apply bounded backpressure indefinitely;
the ring cannot guarantee scheduling or application progress.

Empty messages are valid leases. Full, Empty, invalid channel, invalid length
and too-large payload remain distinct errors. Existing copying APIs share the
same reservation/reclamation logic. `try_send` overwrites all requested bytes
without a redundant clear; `try_recv` retains short-output non-consumption and
untouched output suffix semantics. The v1 layout, ABI and cursor rollover rules
are unchanged. There are no new dependencies, heap allocations, locks or syscalls
in these operations. Zero-copy refers to payload handoff: reservation still
clears len bytes, encoding writes them, and application processing reads them.

## Unsafe audit and isolation boundary

The new unsafe operations form slices into a single slot's `UnsafeCell` and
read/write its length. All lengths are private and checked against const P;
indices mask a validated cursor with const N - 1. Shared values never supply a
pointer, offset, capacity or unchecked slice bound. Reservation only follows
Acquire reclamation; a reader only follows Acquire publication. A slot is never
simultaneously granted to writer and reader under the cooperative SPSC contract.

Lease lifetimes are tied to `&mut` role handles, and slice lifetimes are tied to
the lease borrow. Neither lease is Clone or Sync. Consuming commit/release
cannot coexist with a live slice borrow. Scoped cross-thread slice reads retain
the lease; they cannot permit producer reuse. Forgetting a lease loses no heap
owner and cannot release a slot while borrowed. No `&mut Ring`, payload array or
whole Slot reference is created during live operations. The original unique
role creation and quiescent retirement requirements remain in force.

Clearing exposed write bytes prevents old payload or cancelled scratch contents
from escaping through a later partially filled lease. Read slices expose only
the committed length. This is an API-level integrity/confidentiality property
for cooperative code, **not** hostile-peer or cross-channel memory isolation.
Raw shared mappings can expose the backing beyond the slice. The implementation
does not sanitize Rust padding, authenticate a generation, prevent adversarial
payload writes, establish page permissions or revoke existing mappings.
An allocator must sanitize all backing pages before exposure; native setup must
enforce role-bound page rights and protect the backing through acknowledged
unmapping. No raw attach API is added before those requirements are met.

## Signaling and endpoint closure contract

Commit must precede notification. The existing Doorbell protocol still requires
snapshot sequence -> check/drain ring -> wait-if-unchanged under the same
serialization as notification and blocked-state registration. An integration
test now runs both copying and lease paths through the existing host model;
its first publication follows an independently observed waiter registration.
That is a host integration check, not native scheduler or capability evidence.

Adaptive polling can repeat bounded ring checks before conditional wait. It
cannot replace the sequence/registration protocol or suppress notification using
an unproven armed flag. Blocking a full producer requires the corresponding
authenticated capacity notification after read release. Peer death after commit
but before notify must cause a terminal wake through the real lifecycle path.
Holding a lease never authorizes teardown: backing must remain mapped until
operations finish or the owning process is stopped under the platform's
revocation protocol. This increment adds no substitute endpoint-close protocol.

The outstanding production proof is an actual pending reply racing endpoint
closure with exact caller incarnation, VM identity, endpoint and call generation,
one terminal result, late publication rejection, and stale adoption denial.
The existing peer-close-first result is scoped evidence and does not close that
race. Grant lifecycle/global/session authority and Doorbell B remain separate
gates. Native transport and measurements must wait for those prerequisites.

## Reproduce and interpret evidence

Run `python3 tools/fastlane_leases_validate.py`. Each attempt gets a new directory
under `target/fastlane-leases-evidence/`, retaining commands, exit statuses,
compiler/host information, source hashes, raw measurements and a summary.
Source hashes must remain unchanged during validation. Failed commands or
missing/invalid benchmark rows make host acceptance fail. No result upgrades a
native security gate.

Ring tests cover direct slot address identity, every length from zero to 64,
cancel/reuse clearing, partial writes, panic/forget behavior, interoperation with
copying APIs, invalid lengths/cursors, held-reader producer progress, and
1,130,000 concurrent lease transfers including rollover. The preexisting copying
stress also transfers 1,130,000 messages. Compile-fail documentation tests verify
exclusive producer borrowing and rejection of a read slice surviving release.
These stress checks are not an exhaustive memory-model proof. Miri is not
installed; no Loom dependency or dynamic unsafe-analysis claim is introduced.

The release `lease_bench` example compares copying versus leases at capacities
2/64 and payloads 64/256/1024/4096. Both modes generate and validate every payload
byte. Each case has 10,000 single-thread warmups, 100,000 timestamped operation
pairs, 10,000 concurrent warmups and 100,000 timed concurrent transfers. Thread
creation/join is outside the throughput interval; barriers, retries, scheduling,
encoding and verification are included. Pair p50/p95/p99 includes clocks and
refers to a single-thread pair, not one-way or RTT IPC latency. Tests apply
backpressure and cancellation independently from performance measurement.

No affinity or host isolation is requested. Cases run copying before leases;
cache/frequency/order effects are uncontrolled, so results are diagnostics,
not a speedup acceptance gate. Process user/system CPU time includes the entire
benchmark (warmups, clocks, integrity checks and thread creation); it is not
steady-state CPU overhead or a native kernel-IPC comparison. Host threads do not
stand in for guest workloads or for 2/4/12-CPU topologies. Native latency,
throughput, CPU overhead and the standard kernel IPC baseline are explicitly
null/BLOCKED in the summary. QEMU was not run for this ring-only increment.
