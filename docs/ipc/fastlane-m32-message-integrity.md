# FastLane M3.2 message integrity and hostile-producer contract

2026-11-02, Asia/Tehran. Production FastLane remains disabled. This milestone
adds an explicit message-integrity contract and the smallest safe representation
change that lets a future native transfer be built on top of it. There is no
native ring attachment, native payload transfer, userspace Doorbell, resizing,
reconnect or production enablement. No commit or push.

**Overall status: PARTIAL.** Memory safety and snapshot stability are provided
by the host representation. Coherent publication is explicitly not provided.
Historical failed native attempts remain failed regardless of later replays.

It preserves the completed M3.1B page-table reclamation repair
(`free_shared_page` reclaiming an empty lower table after the synchronous leaf
invalidation) and the M1/M2 authority, mapping-rights and retirement ordering.

## Phase A: three properties, kept separate

The representation must never confuse three different guarantees. They are
reported independently by `sunlight_fastlane_ring::hostile::Integrity`.

| Property | Meaning | Status here | Enforcing boundary and remaining limit |
|---|---|---|---|
| Memory safety | Hostile shared metadata cannot cause an invalid memory access | **PROVIDED** | No pointer, slice or reference is derived from peer data; every index comes from validated local geometry (`fastlane-ring/src/hostile.rs`). No `unsafe` code in the module (`#![forbid(unsafe_code)]`). A sound `Memory` backend is the only assumption |
| Snapshot stability | The bytes returned by a receive cannot change after delivery | **PROVIDED** | Each receive is a bounded copy into a private, fully initialized `[u8; 256]`; the lease returns `&[u8]` over that private array only. The shared slot is never borrowed |
| Publication integrity | Delivered bytes are one coherent committed message | **NOT PROVIDED** | M2 leaves the producer a writable alias to the payload frame. A hostile producer can write after commit or during acquisition, so a valid copy may interleave bytes from more than one write. No mechanism in this milestone enforces coherence |

The private-copy representation therefore gives **transport safety** — bounded
access and a stable private copy — but **not transport coherence**. The two must
not be conflated: stability says "the returned bytes will not change under you",
not "the returned bytes were a single committed transaction".

M1 authenticates channel participants, not a particular payload observation.
Application parsing of the private copy can reject invalid content but cannot
recover a coherent publication guarantee from structurally valid mixed bytes.

## Phase B: path selection

Two candidate paths were evaluated against M2's current rights matrix.

### Small-message path (selected)

Bounded shared transport storage (16 slots x 256 bytes) with validation and
copying into private consumer-owned memory. It never exposes an unchecked
shared reference to application code; it validates length, offset, channel,
sequence and slot generation before any byte is read; it rejects malformed
metadata terminally; and it defines the mutation-during-copy case as a stable
but possibly mixed private copy. Transport integrity is explicitly **not**
application-level message authenticity. This path is implemented and accepts
that coherent publication cannot be enforced against a hostile writer under the
current rights.

### Large-message path (deferred, not implemented)

Page-backed ownership transfer with enforced write revocation before immutable
consumer access. It is **not** implemented, because it is not necessary for a
bounded experimental transfer and would require a new authority/VM contract.
The accounting it must eventually satisfy:

- **Writable aliases.** Every producer mapping of the payload frame must be
  withdrawn (or downgraded) before a consumer may read it immutably. A
  read-only consumer PTE alone does not help while a writable alias exists.
- **TLB shootdowns.** Each ownership transition needs a synchronous
  invalidation of the previous writer's translations and completion
  acknowledgement from every target, reusing the M2 remote-flush path.
- **Mapping transition costs.** Permission flips are per-page/per-message work;
  small messages cannot amortize them, which is why the small-message path stays
  copy-based.
- **Frame ownership.** Frames must move through an explicit owned -> published
  (read-only) -> retired lifecycle with a single owner per state, not a
  shared writable frame.
- **Cancellation and peer exit.** A revocation in flight must converge with
  cancellation, producer exit, exec and session withdrawal without leaking or
  double-freeing frames.
- **Reuse only after verified retirement.** A frame may only be reissued after
  the last consumer view is released and the retirement is observed, exactly as
  M2 established for its backing objects.

Until that contract exists, coherent publication is impossible under the
current shared-memory rights. This document records that as a limitation rather
than weakening the definition of integrity.

## Phase C: implemented contract

The smallest safe change is a machine-checkable integrity classification so the
API can **never** present a mixed or mutable observation as coherent. Nothing
in the transfer path, the authority model or the mapping rights changes.

- `Integrity` (in `fastlane-ring/src/hostile.rs`) exposes four properties with
  private fields and read-only accessors. There is no constructor or setter that
  can set `coherent_publication` or `application_authenticated` true.
- `Integrity::PRIVATE_COPY` is the only value the representation produces, and
  `ReadLease::integrity()` returns it for exactly the bytes `payload()` returns.
- The type is `Copy` + `const`-constructible so callers can branch on it without
  allocation or locks.

Deliberately **not** used as a coherence proof: sequence counters, checksums,
Acquire loads or a read-only consumer mapping. A producer that retains a
writable alias can replay the same sequence or produce a second valid checksum;
none of these removes the alias. The contract states this explicitly so no
future code mistakes such a check for immutable publication.

Preserved invariants:

| Requirement | How it is met |
|---|---|
| Explicit ownership transitions | Commit via the `WriteLease`; reclamation via the `ReadLease`; Drop publishes/reclaims nothing |
| Bounded payload sizes | Fixed 16 x 256 page; length checked against `PAYLOAD_BYTES`; `offset + length` checked in `u64` before any conversion |
| Fallible allocation | No heap allocation; a lease carries a fixed private array. Future native backing allocation retains M2's fallible reservations |
| Safe slot reuse | Whole reused slot is rewritten from private staging, so no stale or uninitialized byte is exposed beyond `len` |
| No exposure of uninitialized bytes | Private staging starts zeroed; copies fill complete words; `bytes[len..]` is cleared |
| No unbounded waiting on hostile state | No spin, lock or retry on peer bytes; one bounded descriptor/cursor observation per operation |
| Generation-qualified stale rejection | Channel, sequence and slot generation are checked exactly; lease serials reject stale local leases |

## Phase D: adversarial validation

Host tests in `fastlane-ring/src/hostile/tests.rs` cover every listed behavior
and assert the **claimed** guarantees, not stronger ones:

| Behavior | Test |
|---|---|
| Concurrent post-commit mutation | `producer_mutation_after_commit_is_visible_before_private_copy`, `concurrent_hostile_payload_writes_never_mutate_returned_copy`, `post_commit_mutation_keeps_stability_but_never_claims_coherence` |
| Mutation during snapshot acquisition | `mutation_during_copy_can_mix_bytes_but_cannot_change_bounds`, `mixed_snapshot_is_stable_bounded_and_not_reported_as_coherent` |
| Invalid lengths and offsets | `malformed_lengths_offsets_and_overflow_are_terminal`, `all_lengths_and_private_identity` |
| Sequence and generation exhaustion | `sequence_and_local_lease_serial_exhaustion_never_wrap` |
| Stale lease reuse | `stale_local_leases_cannot_publish_or_reclaim`, `abandoned_reservations_and_private_read_leases` |
| Cancellation during publication | `abandoned_reservations_and_private_read_leases`, `producer_exit_and_cancellation_never_upgrade_the_integrity_claim` |
| Producer exit during publication | `explicit_retirement_fences_operations_and_preserves_first_error`, `retirement_at_each_access_fails_closed_without_publication_or_output` |
| Consumer exit during acquisition | `explicit_retirement_fences_operations_and_preserves_first_error` |
| Malformed shared metadata | `wrong_channel_sequence_and_slot_generation`, `corrupted_producer_cursor_never_chooses_an_address`, `corrupted_consumer_cursor_is_checked_against_private_publication`, `adversarial_descriptor_corpus_has_bounded_work_and_access` |
| Repeated slot reuse and byte clearing | `partial_initialization_and_whole_slot_reuse_clearing` |
| Full-ring backpressure | `capacity_exhaustion_and_abandoned_reader_apply_bounded_backpressure`, `false_valid_ack_can_trigger_reuse_but_private_read_is_stable` |
| Honest, minimal integrity classification | `every_receive_reports_the_honest_private_copy_integrity` |

The mixed-snapshot tests assert `snapshot_stable() == true` **and**
`coherent_publication() == false`. A stable but mixed snapshot is therefore
never reported as a coherent committed message. A `compile_fail` doctest proves
the stronger claim cannot be forged from outside the module.

`concurrent_copy_can_return_bytes_that_never_existed_together` additionally
forces the actual host writer thread between the consumer's first and second
payload loads. Shared words change from `(old, old)` through `(new, old)` to
`(new, new)`, while the receive returns `(old, new)`. Those returned bytes never
existed together in transport storage. The writer also corrupts length and
offset after validation; copying still uses the validated local bounds. Further
writes cannot change the private result, whose coherence claim remains false.
Test rendezvous use bounded host waits; the representation itself never waits.

`slot_generation_is_bounded_by_nonwrapping_sequence` checks the final possible
slot generation and rejects neighboring and maximum forged generations. There
is no independent generation counter to exhaust: generation is
`sequence / 16 + 1`, so sequence exhaustion occurs before generation overflow.

The cancellation/exit rows above are **host representation controls**: dropped
or forgotten leases, explicit local retirement, and an injected `Closed` result
at each backend access. They do not execute process exit or mapping withdrawal
concurrently with a native payload operation. Native producer exit during
publication and consumer exit during acquisition remain **NOT_RUN**. The
existing M1/M2 exit fixtures prove authority/mapping retirement only.

Both receive APIs have the same private-copy contract, including `try_recv`,
which does not return an `Integrity` value. If that convenience API copies
successfully but release fails, it returns an error with the output already
changed; no successful consumption or immutable publication is implied.

## Phase E: native compatibility and regression

The change is confined to `fastlane-ring` (a host-only crate not linked by the
kernel) and documentation. Kernel authority, restricted mappings, the M3.1B
repair and SHM/TLB ordering are untouched.

`cargo tree -p sunlight-kernel` shows no path from the kernel to
`sunlight-fastlane-ring`, so this representation cannot alter any native result.

### Host receipt (PASS)

`target/fastlane-m32-evidence/host-20261101T172849.004232Z/validation-summary.json`:
PASS for 37 library tests (including the 4 new integrity tests) and 5
documentation tests (including the new `compile_fail` non-forgeability control),
Clippy (`-D warnings`), the `x86_64-unknown-none` bare-metal check,
`rustfmt --check`, 218 Python parser controls, whitespace, and the ASan and TSan
runs of the `hostile::` tests. **Miri is NOT_RUN** because `cargo-miri` is not
installed; the failed availability command is retained, and no dynamic
memory-model proof is claimed.

### Native regressions

`target/fastlane-m3-evidence/regressions-20261101T173329.450531Z/orchestration.json`
(source-bound, identical 1,350-input source hash across the passing and failing
guests):

| Suite | Result |
|---|---|
| M1 authority (S1/S2) | FORCED_AND_PASSED, 2/4/12 |
| IPC/VM/SHM with reply race | PASS, 2/4/12 |
| Authenticated Account exec, exit modes 11/12/13 | FORCED_AND_PASSED, 2/4/12 |
| Scheduler, R2D pin handoff, kernel Doorbell | PASS, 2/4/12 |
| Production exclusion (marker absent, empty feature list) | PASS |
| M1/M2/M3 mapping compatibility, 2 and 4 CPUs | PASS |
| M1/M2/M3 mapping compatibility, 12 CPUs | **FAIL** (retained) |

The 12-CPU compatibility guest fails the unchanged global M2 census check:
total SHM frames are 1304 at checkpoints 100-101, 1305 at 102, and 1304 from
103 onward. Restricted-channel objects/views/frames are zero at every census,
and all six M3 word witnesses pass. This is the **same unresolved
global-census-drift class** recorded in M3.0 and M3.1 and is not attributable to
this host-only change. An identical-image 12-CPU replay
(`target/fastlane-m32-evidence/replay12/native-20261101T180021.631255Z`) PASSES
with SHM flat at 1304 across all checkpoints. That replay does **not** discharge
the original failed attempt; both are retained and the aggregate stays PARTIAL.
Kernel inputs are byte-for-byte unchanged by this milestone, so no kernel
assessment is upgraded.

### M3.1B repair preserved

`target/fastlane-m32-evidence/retention/native-20261101T180945.553403Z/validation-summary.json`:
PASS on 2/4/12 CPUs with **zero retained empty P1 tables at every one of the
eight crossings** and zero drift after VM teardown. The minimal
`free_shared_page` reclamation repair is intact.

## Independent diagnostic debt (retained)

The M1 2-CPU `channel race deadline` is a debug-only fixture progress stall: one
actor reaches the rendezvous and the other never enters, so `wait()` spins to
its 5 s deadline. It is **not** a production channel deadlock. Its timeout is
unchanged and it is not classified as a production deadlock. It is tracked
separately and no fixture repair is folded into this milestone.

## November 2 review and fresh validation

The starting checkout already contains `Integrity`, `ReadLease::integrity()`
and the M3.2 report. This review preserves that implementation and adds two
adversarial tests plus receive API documentation. It changes no kernel code,
mapping permission, authority check, cooperative lease or timeout.

Fresh host receipt:
`target/fastlane-m32-evidence/refresh-20261102/host-20261101T210442.818841Z/validation-summary.json`.
Its UTC timestamp falls on November 2 in Asia/Tehran. It records **PASS** for
39 library tests, 5 doctests, strict crate Clippy, bare-metal crate checking,
formatting, 218 parser controls, whitespace and both ASan and TSan (27 hostile
tests each). Source hashes match before and after. **Miri remains NOT_RUN**:
the component is unavailable, with its failed availability log retained.

Fresh M1/M2 compatibility receipts are under
`target/fastlane-m3-evidence/regressions-20261101T210443.991300Z/compatibility-{2,4,12}/`.
All three pass the unchanged authority/mapping assessor and all six real mapped
word witnesses. At checkpoints 100-105, global SHM is flat at 1304 and page
tables at 381; restricted objects/views/frames are zero. These are new attempts,
not explanations or replacements for the earlier failed 12-CPU receipt.

The full fresh regression receipt is
`target/fastlane-m3-evidence/regressions-20261101T210443.991300Z/orchestration.json`:
**PASS**, all 22 commands, no failed suite, executable source hashes equal
before/after and equal to the current tree. M1/M2 compatibility, IPC/VM/SHM reply
races, authenticated authority S1/S2, Account exec and exit modes 11/12/13,
scheduler, R2D pin handoff and kernel-only Doorbell all pass on 2/4/12 CPUs.
The ordinary image has empty features and no M3 fixture marker; mapping host
checks and release kernel checking pass. Kernel Doorbell regression is not a
userspace Doorbell integration or a native FastLane payload result. The original
failed global-census attempt and the independent M1 rendezvous debt remain open.

Fresh M3.1B receipts under
`target/fastlane-m32-evidence/refresh-20261102/retention/` are **PASS**:
`build-20261101T213344.563094Z`, `native-20261101T213413.789255Z` (2 CPUs), and
`native-20261101T213440.207523Z` (4/12 CPUs). All eight crossings on each topology
retain zero empty P1 tables, surviving mappings retain their shared tables, and
the normal and deliberately retained-table teardown arms return all frames and
tables with zero final free/SHM/page-table drift. Sources and archived image
hashes match the current executable inputs. The reclamation repair is preserved.

This review's aggregate receipt is
`target/fastlane-m32-evidence/refresh-20261102/review-summary.json`.
The host contract and fresh regressions pass within their scopes; the aggregate
milestone remains **PARTIAL** with the historical failure retained. Coherent
publication is **NOT PROVIDED**, native payload lifecycle tests are **NOT_RUN**,
and no production readiness or native transfer completion is claimed.

## Deliverables

| Deliverable | Result |
|---|---|
| Explicit hostile-peer integrity contract | PASS, Phase A table and `Integrity` |
| Selected representation and rationale | PASS, Phase B small-message path |
| Minimal implementation changes | PASS, `Integrity` + `ReadLease::integrity` + tests |
| Adversarial validation evidence | PASS host scope, see receipt and Phase D |
| M1/M2/M3.1B regression results | See receipt; unchanged by this milestone |
| Updated security and unsafe-code audit | PASS, Phase E and below |
| Exact remaining native-transfer requirements | See below |

## Unsafe-code and security audit

- `fastlane-ring/src/hostile.rs` still has `#![forbid(unsafe_code)]`; the change
  adds no `unsafe`, no pointer, no `AtomicU64` and no shared reference.
- The cooperative `Ring`/`leases.rs` `unsafe` blocks are unchanged and remain a
  separate, documented representation that must never be attached to hostile
  native mappings.
- The bounded concurrent-copy control uses only host atomic words and private
  arrays. Its thread rendezvous is test-only and introduces no transport lock,
  native access, or lifecycle authority.
- The new type cannot weaken any guarantee: it is monotone toward "less
  claimed", never toward "more claimed".
- No token, secret, seed or grant material is logged or stored.
- Production exclusion is preserved: the marker/feature changes are host-only
  and debug-only tooling; the production image and release kernel are unaffected.

## Exact remaining requirements for a native transfer

1. An opt-in, authenticated **copy-only** native backend using the existing M1
   attachments and M2 views, with audited single-instruction raw-word access
   (no Rust references into peer memory) and enforced mapping lifetime. Actual
   operation admission and commit/release must serialize against channel
   retirement; withdrawal must wait for admitted accesses or stop their process
   before unmapping. A shared terminal flag or this host `Memory` trait cannot
   provide that lifecycle proof. Bind exactly one local endpoint per attached
   role to the fresh kernel channel generation; constructors alone authorize
   nothing, and cursor reset/reconnect is unsupported.
2. A bounded polling transfer fixture (no Doorbell) exercising the M3.2
   contract on real mappings: hostile mutation, malformed words, peer
   exit/revoke/exec during operations, cancellation and capacity.
3. Application-level validation on the private copy, with the `Integrity`
   classification surfaced to callers so mixed bytes are never treated as one
   coherent message.
4. Only if large messages are needed: the separate page-ownership/authority
   milestone in Phase B (writable-alias revocation, TLB shootdown, ownership
   lifecycle, verified retirement) before any immutable zero-copy receive.
5. Userspace Doorbell and production FastLane remain disabled until their own
   authority and lifecycle milestones close.

## M5 integration follow-up

The [M5 native transfer report](fastlane-m5-authenticated-transfer.md) connects
this unchanged representation to the exact backing of real M2 mappings through
an opt-in kernel-mediated word backend. Metadata validation and private copying
execute in the independent authenticated consumer VM. The backend serializes
each admitted access against physical retirement and admits only one message
per channel. Native mutation between loads and after private copying preserves
this report's distinction: memory safety and private snapshot stability are
provided; coherent publication and application authenticity are not. This
follow-up does not rewrite any historical host/native result or enable production.
