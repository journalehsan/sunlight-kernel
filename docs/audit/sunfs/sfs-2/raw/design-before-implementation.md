# SunFS SFS-2 persistent transactions and recovery

2026-11-14. Implementation and qualification in progress; no COMPLETE verdict yet.

## Pre-implementation format decision

The v0.2 codecs already encode every required object: raw checksummed file blocks,
one-block extents, extent roots, regular inodes, root bindings, node map, orphan
root, segment bitmaps and self-accounting summaries, and three checkpoints. No
byte offsets, version numbers, feature bits, or golden fixtures need to change.
Existing valid v0.2 bootstrap images remain compatible. The engine implements a
bounded subset and returns Unsupported for valid images outside that subset.
The independent checker continues to support its existing wider inspection scope.

SFS-1's geometry metadata estimate covers empty content roots, not populated file
extent/directory pages. SFS-2 therefore computes every actual dependency and the
self-referential summary fixed point before I/O, and enforces the existing SB
transaction bound as an additional ceiling rather than relying on that estimate.

Initial implementation policy: one exclusive owner, root-only regular files,
16 files, 64-byte names, 16 blocks (65536 bytes) per file, dense zero-filled holes,
one dirty volume epoch. Full-volume immutable snapshots rewrite every object on
commit. Each snapshot occupies otherwise unprotected segments, simplifying segment
epoch ownership. Unused blocks in a protected segment are unavailable, explicitly
accounted as placement overhead. Cleaner reserve is never consumed. No background
cleaner, unlink, rename, hierarchy, or orphan mutation is implemented.

Admission builds and encodes the entire snapshot before persistent writes. It
reserves exact dependency blocks, segment placement, fixed checkpoint slots, and
retained closures. Mutations are staged in bounded runtime memory; unsuccessful
admission leaves that working state and the last committed state untouched.

Protocol follows D/P/R: dependencies, flush D; checkpoint into an empty slot,
flush P; zero oldest slot, flush R; only then acknowledge fsync. Three retained
slots after interrupted retirement require R0 before publication. I/O errors freeze
the owner until drop and remount. Corrupt slots or invalid advertised closures
permit read-only fallback only. Recovery never writes or repairs.
