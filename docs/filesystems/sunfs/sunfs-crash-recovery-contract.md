# SunFS checkpoint, fsync and crash-recovery contract

2026-11-14. Normative proposed behavior; persistence/crash execution is NOT_RUN.
This is a protocol to implement and test, not a claim of proven crash safety.

## Failure model and block-layer requirements

Writes can reorder, be torn at any byte/sector boundary, fail after a prefix, or
complete ambiguously. No filesystem-block write atomicity is assumed. Successful
flush is the only initial persistence primitive: it drains every accepted earlier
write through software caches, transport and device volatile caches to stable media.
It establishes a persistence cut; later writes cannot retroactively alter it.
Previously durable blocks are not spontaneously damaged by an unrelated power loss.
Misbehaving hardware, deliberate checksum forgery and checksum collisions are outside
this model and must not be called repaired by recovery. Flush honesty is a driver/
platform qualification obligation, not established by an API declaration.

Proposed reusable interface (conceptual, no implementation in SFS-0):

- `geometry()`: capacity bytes/u64, logical_sector_bytes/u32, physical_sector_bytes/
  optional u32, transfer_alignment/u32, max_transfer_bytes/u32, read_only/bool,
  flush_supported/bool, FUA_supported/bool and media_incarnation/u64.
- `read_exact(offset:u64, output)`: completes the full range or returns an error;
  no unvalidated partial bytes become filesystem-visible.
- `write_exact(offset:u64, input)`: complete success or `IoFailure` with
  known_completed_bytes/optional u64 and completion certainty. A success means the
  bytes completed at the device, not durable media. Adapter caches may instead
  acknowledge acceptance, but must expose and drain that distinction at flush.
- `flush()`: full persistence barrier or explicit Unsupported/error. No default
  success. `quiesce()`/device reset fencing must ensure no timed-out request can
  later DMA into reused buffers or modify subsequently reused blocks.

Terms: accepted=ownership transferred to layer; completed=device reports successful
transfer; durable=survives power loss under the model; ordered=required dependency
persistence precedes publication; flush-completed=all prior accepted writes durable;
failed/uncertain=some, all or none may have reached media. CPU fences do not substitute
for flush. Single-writer request submission and completion simplify host ordering,
but completion order alone never establishes persistence order.

FUA is optional and unused in v1. FUA on the CP alone cannot replace flushing its
dependencies. Sector size must divide4096, capacity arithmetic must not overflow,
partition offset must align, and unsupported geometry rejects writable admission.
The current 512-byte BlockDevice can be wrapped with eight checked sector transfers;
any sector error makes the whole 4 KiB write uncertain. See the evidence assessment
for actual trait/driver capabilities and missing timeout fencing/error detail.

Unsupported flush: read-only access only; fsync must fail Unsupported, never pretend
success. Flush failure, partial write, media replacement or uncertain completion:
latch a volume-wide I/O error, stop mutations, retain all ownership and buffers until
quiescence, report EIO/uncertain to waiters, then require recovery. A retry does not
turn uncertainty into confirmed failure or authorize block reuse. A valid newly
published state may appear after an operation returned EIO; applications must be told
that outcome is indeterminate. No automatic rollback overwrites media.

## Ownership invariants

I1. Every accepted CP references a complete graph of checksummed data and metadata.
I2. A dependency becomes durable before any CP naming it can become durable.
I3. Protected blocks = union of closures of ALL still-valid CPs, plus runtime pins,
working reservations and uncertain-I/O quarantine. Allocation excludes that union.
I4. A CP is retired only after its slot is durably zero and no outstanding I/O can
republish it. Root age or a newer generation alone does not retire it.
I5. A normal writable volume retains two fully validated generations. Three slots
permit publication while retaining two; copies alone do not preserve their blocks.
I6. Failed commits never acknowledge durability and never release uncertain ownership.
I7. Relocation, orphan reap, rename and truncate use the same publication protocol.
I8. Published generations and node IDs never wrap or silently regress in writable mode.
I9. Recovery/checking never writes or silently repairs corruption.
I10. Acknowledged fsync includes a durable complete cut containing all mutations in
its serialized input epoch; later operations belong to a later cut.

## Concrete commit sequence

Bootstrap: formatter writes both SBs, empty root and all tables/summaries, flushes
D0; writes CP1/CP2 in separate slots, flushes P0. Writable mount requires both fully
recoverable. They can reference the same immutable closure. Generation3 is next.

Under exclusive writer ownership, capture a complete mutation epoch. Other writers
wait; reads use pinned immutable/working objects with version checks. Preflight the
full bounded allocation plan, cleaner reserve and dirty-memory budget before any
persistent mutation. Reserved blocks exclude ALL retained CP/pin ownership.

1. If three CP slots are already valid after an interrupted previous retirement,
   validate all three, durably zero the oldest slot with flush R0, then discard its
   closure from the protected union only after that flush. Keep the newest two.
   Otherwise choose an already all-zero empty slot. Never overwrite either retained
   checkpoint to make room for new data.
2. Write new raw data blocks (zero-tail semantics), then extent/directory pages,
   new node records, new node-map pages/root, orphan pages/root, summaries and segment
   state pages/root. Their exact locations are reserved before serialization; their
   physical write order can vary because nothing publishes them yet. Include all
   pages and current orphan objects in authoritative accounting. Preserve unmodified
   old nodes/data by reference, and retain fallback ownership independently.
3. Wait for completed transfers, then flush D. All dependencies must be durable.
   On any error, freeze; do not write a checkpoint.
4. Encode CP generation g, predecessor newest previous generation, complete root set
   and counts. Write the entire CP into the durably empty slot. The slot is not
   assumed atomic. Nothing points to unflushed metadata.
5. Flush P. Only after success is g known durable. It is now a new recovery state;
   dependency data cannot be reclaimed even if the caller has not received success.
6. If three valid CPs exist, zero the oldest CP slot and flush R. Until successful,
   preserve its complete closure. Retain g and its immediate predecessor. Release
   blocks only after R AND last runtime pin AND request quiescence. If retirement
   fails, fsync returns EIO and freezes writes even though g may be durable.
7. Publish the durable epoch to fsync waiters and return success. Recompute union
   free space. Future commits may reuse retired-only blocks; current metadata can
   conservatively undercount those newly freed blocks until its next snapshot.

No commit acknowledgment depends on background work. A zeroing write interrupted
before R can leave a valid old CP, all-zero slot or corrupt bytes. All are handled
conservatively on mount. All-zero is recognized as unpublished/retired; no allocations
need an invalid slot's contents to reconstruct a working transaction.

## Crash classification and mount algorithm

| Crash point | Permitted state | Recovery rule |
|---|---|---|
| Before/during new dependency writes | Old two CPs; arbitrary unreachable garbage | Select newest old complete state; garbage not live |
| During D | Dependencies may be partial, no new CP | Same old state |
| After D before CP | New graph durable but unpublished | Ignore new graph, no roll-forward |
| During CP write / before P completes | New CP absent, torn, or complete | Accept new only if full closure validates; otherwise old complete state |
| After P before retirement | Three valid CPs possible | Validate/protect all, select newest; retire oldest via R0 before writes |
| During retirement R | Old slot valid, zero or corrupt | Never reclaim its uncertain ownership in writable mode |
| After R / before reply | New two CPs retained; fsync outcome unknown to caller | New state may appear; caller did not get a durability success |
| After success | New complete checkpoint durable | Acknowledged epoch survives model power loss |

Mount first reads without modifying any media. Validate both SB candidates including
CRC, geometry, feature/version rules and agreement. Inspect every CP slot; distinguish
all-zero (empty), bad header/checksum (corrupt or interrupted, origin uncertain),
structurally complete, and structurally valid but unrecoverable dependency graph.
An intact state0 record is INCOMPLETE and never accepted; the normal publisher
emits only state1. Bad-CRC bytes are CORRUPT_OR_INTERRUPTED because their history
cannot be recovered reliably. A state1 record with a bad closure is UNRECOVERABLE;
a state1 record with a complete verified closure is VALID. These distinct diagnostic
classes do not imply that all torn writes can be distinguished from later damage.
Do not equate a valid CP checksum with a valid filesystem.

For each structurally valid candidate traverse its FULL metadata and data closure:
validate types, UUID, address/epoch, generation ordering, map uniqueness, root,
directories/parent graph, nlinks, orphan separation, extents and CRCs, exact segment
bits/summary ownership and counts. Checks are bounded and iterative; failure/budget
exhaustion is explicit. Choose the largest fully recoverable generation. If none
exists, fail mount. Conflicting generation identities fail writable mounting.

A normal writable mount requires two recoverable CPs, no nonzero malformed slot,
no unrecoverable advertised CP, compatible SBs and enforceable persistence/quiescence.
Three valid CPs are allowed and all remain protected until durable oldest retirement.
If a slot is corrupt, or newest CP dependencies fail but an older graph is complete,
offer explicit read-only degraded inspection and report fallback/data-loss risk.
Do not silently mount writable or automatically clear the bad slot. This conservative
rule can require operator action even after a torn unpublished CP; that availability
cost is accepted in v1. Determining interrupted-publication versus lost-durable-CP
intent would require stronger publication/retirement records and a new ADR.

Old checkpoint data has the same checksum verification as new data. A missing old
fallback closure prevents writable mount, even if the newest looks sound. Corruption
fallback does not promise the latest acknowledged data survives arbitrary media
failure. After read-only inspection, explicit recovery tooling is future scope;
SFS-0 provides no repair path. Failed reads return corruption/EIO, never guessed bytes.

## Application durability semantics

| Operation | Before synchronization | After successful relevant fsync |
|---|---|---|
| write | Accepted bytes visible in working epoch; may be lost on crash | Data and size/extent/node metadata durable |
| fsync(file) | Captures serialized mutation cut | Full consistent volume checkpoint, including namespace in that cut |
| fsync(directory) | Captures serialized mutation cut | All bindings, link/orphan ownership and atomic rename transactions in cut durable |
| close | Releases runtime reference; last unlinked close queues reap | No implied fsync, no new durability promise |
| rename | Atomic visibility under namespace lock; same-volume only | Both parents, replacement and orphan changes durable together |
| unlink | Name gone in working namespace; open object survives | Name removal and orphan backing ownership durable |
| truncate | New size and zero-tail mapping visible atomically | Shrink/extension, holes and reclaim ownership durable |

Initial full-volume fsync is deliberately stronger than minimum per-file POSIX
persistence: successful file fsync also persists creation/name updates in the captured
cut. Applications should still use directory fsync after creation/rename/unlink for
an explicit portable namespace guarantee. A rename occurring after the file fsync's
cut is not covered. Successful directory fsync also includes dirty file data in the
cut. Cross-volume atomic namespace transactions are unsupported.

Concurrent fsync calls are serialized, initially without group commit. A waiter whose
target epoch is already included may reuse its success; later dirty work cannot be
smuggled into an earlier acknowledgment. Bounded queues and cancellation retain the
commit/pin until actual completion. ENOSPC is detected before planning writes where
possible; I/O/corruption errors latch and propagate to every affected waiter and
subsequent write/sync. Close still consumes its reference exactly once and may report
a latched error; it does not retry publication or promise cleanup durability. The
future libc adapter must preserve errors, including delayed fsync errors; mapping
unknown completion to apparent success is forbidden.
