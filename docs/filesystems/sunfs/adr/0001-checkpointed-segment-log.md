# ADR 0001: Compact segment log with immutable checkpointed tables

Date: 2026-11-14. Status: accepted for SFS-0 design; format remains experimental.
Decision scope: single volume, no boot/root migration, no production implementation.

## Decision

Choose A: out-of-place data and node writes, stable generation-qualified node IDs,
and immutable, sorted, paged node-map/segment/orphan tables published together by
one checkpoint. Initially rewrite these tables completely on every commit. Use
segment allocation and a synchronous foreground cleaner. No roll-forward, journal,
in-place metadata updates, snapshots, shared file data, or asynchronous GC.
The engine serializes namespace mutations and commits. Read operations pin object
and mount incarnations. Checkpoint publication uses three fixed slots, normally
retaining two fully recoverable generations and their complete block closures.

A table root points to a bounded directory of immutable pages. It is not a general
B-tree. Capacity and memory limits are explicit formatter parameters within reader
limits. Reject growth before mutation when those limits would be exceeded. A future
incremental map needs a new ADR and feature negotiation. O(number of entries)
metadata rewriting is an intentional initial limitation, not a performance claim.

## Alternatives (qualitative engineering estimates, not benchmarks)

| Criterion | A: compact checkpointed log | B: simplified COW tree | C: hybrid segment/versioned metadata |
|---|---|---|---|
| Crash consistency | One root publication; no metadata overwrite | One root publication; COW path ordering | Root plus version-set consistency |
| Write amplification | High full map/state rewrite; data out of place | Lower sparse updates, tree path copies | Potentially lower with bitmap-selected table pages |
| Metadata update cost | O(N + segments) per commit | O(log N) paths, splitting/rebalancing | Dirty table pages plus selectors |
| Recovery | Bounded full graph validation | Validate tree levels/order/reachability | Validate selectors, epochs and log ownership |
| GC | Relocate by map; retain fallback union | Relocate and repair tree ancestors | Relocate plus reconcile table generations |
| Memory | Bounded map/mark sets; predictable O(volume blocks + nodes) | Page cache can be bounded; more algorithms | Dirty/version maps and checkpoint bookkeeping |
| Small files | No packing; 4 KiB data/node costs, costly fsync | Similar data overhead, cheaper sparse fsync | Could amortize small writes; more states |
| Implementation | Lowest: page arrays, one writer, checker | Tree validation/splits and relocation coupling | Highest initial version/selector state space |
| Extensibility | Tables can later become indexed with feature flag | Natural scalable index | Flexible but more permanent mechanisms |

B is rejected initially because sparse-update efficiency does not justify split,
ancestor relocation, and independent tree checking machinery before persistence
proof. C is rejected initially because NAT/SIT-style version selectors and multiple
metadata update mechanisms expand the crash state space. A still uses versioned
metadata in the ordinary immutable sense; it does not use C's independently switched
metadata table copies. Segment geometry remains a modeled experiment.

## Why this meets the mission

A directly supplies stable identity, atomic namespace snapshots, deterministic
recovery and explicit fallback ownership with few mutation paths. Costs are bounded
volume size, high checkpoint write amplification and potentially slow mount. None
may be hidden by weakening barriers, discarding fallback retention, or claiming
performance before measurements. A full disk returns ENOSPC before consuming the
cleaner/commit reserve. Metadata with no safe relocation destination remains pinned.

## Historical and external context

The local `docs/sunfs/README.md` v0.1 already requires out-of-place metadata,
checkpoint dependency durability, real fsync and retained fallback blocks. This ADR
makes those requirements concrete, rather than treating the draft as implemented.
The Linux F2FS documentation describes separate NAT, SIT and segment-summary areas:
https://docs.kernel.org/filesystems/f2fs.html (consulted 2026-11-14). SunFS borrows the
identity indirection idea; it does not copy F2FS layout or recovery algorithms.
All comparative costs above are design estimates. No external performance result is
used to select SunFS geometry.

## Revisit criteria

SFS-1 records table bytes, maximum memory, tail waste and commit reserve over 64,
256 and 512 blocks/segment. SFS-2 must execute exhaustive bounded crash schedules.
Stop before large-volume support if full rewrites violate the agreed laboratory
budget; do not silently substitute a tree or freeze v0.2 offsets.
