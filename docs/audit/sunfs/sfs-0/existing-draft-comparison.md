# SunFS v0.1 comparison

Source: `docs/sunfs/README.md`, preserved and hashed in source-baseline.json.
No v0.1 statement is treated as implemented behavior.

| v0.1 proposal | v0.2 decision / clarification |
|---|---|
| Correctness → recoverability → simplicity | Retained without reordering |
| F2FS inspiration, no compatibility | A/B/C comparison; compact checkpointed paged arrays selected, F2FS layout not copied |
| 4 KiB candidate, geometry later | Explicit4096 codec candidate,64/256/512 segment model and pending runtime comparison |
| Stable node IDs | UUID+nid+incarnation; no ID recycling, descriptor tokens separate |
| Mapping and allocation must be crash-safe | Immutable bounded full rewrites, typed roots, no in-place tables |
| Out-of-place data/nodes | Parent-verified CRC/epoch and zero-tail extents; all metadata owned too |
| Checkpoint generations | Three slots, normally two retained closures, explicit D/P/R/R0 flushes |
| Two copies not enough | Union liveness plus durable slot invalidation before reclaim; damaged slot blocks writable fallback |
| Full commit fsync | Serialized full-volume cut; namespace and orphan ownership included; close not durability |
| Cleaner reserve unspecified | Admission-derived worst-case reservation, exact accounting and ENOSPC stop |
| Supporting GC/prefetch/backup services | Foreground engine cleaner only; services deferred |
| Checksums for critical metadata | Full metadata CRC64, parent expected CRC, raw data CRC domain; explicit nonadversarial model |
| Hosted crash laboratory first | Volatile/durable split, ordering/fragments, deterministic traces, independent checker |
| Future LZ4 fields | encoding0 only; no compression flag enabled or storage optimization committed |
| Symbolic links in v1 | Representation reserved; activation gated on SFS-6 VFS capability/metadata support |
| No installation requirement | Secondary-volume staging and separate future root/installer authorization |

Unresolved by design: executed serialization/recovery evidence, final geometry,
real-device barrier honesty, native timeout fencing/latency, retained directory ABI,
and POSIX time authority. These are named milestone gates, not hidden SFS-0 failures:
SFS-0 requires their contracts and test plans, not production implementation.
