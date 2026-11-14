# Architecture decision summary

Selected: ADR0001 A, compact segment log with immutable checkpointed bounded tables.
Rejected initially: B general COW tree (split/ancestor machinery), C independently
versioned segment metadata (selector/generation complexity). All costs are qualitative
estimates until later measurements. Correctness and inspectability take precedence
over full-table write amplification and bounded maximum volume/object count.

Normative documents:
- docs/filesystems/sunfs/sunfs-architecture-v0.2.md
- docs/filesystems/sunfs/adr/0001-checkpointed-segment-log.md
- docs/filesystems/sunfs/sunfs-on-disk-format-v0.2.md
- docs/filesystems/sunfs/sunfs-crash-recovery-contract.md
- docs/filesystems/sunfs/sunfs-vfs-integration.md
- docs/filesystems/sunfs/sunfs-implementation-roadmap.md

Stable UUID/nid/incarnation object identities and separate runtime capability tokens;
no nid recycling. Three checkpoint slots, normally two retained closures. Full-volume
serialized fsync with dependency/publication/retirement barriers. Foreground cleaner
with reserve admission; no safe capacity means ENOSPC, not early reclamation. Damaged
publication slots may require read-only fallback. No automatic repair or roll-forward.

SFS-1 recommendation: implement the isolated device/fault laboratory and byte codecs,
format empty disposable images and build independent read-only checking; validate
geometry/reservation and golden records before starting engine mutations in SFS-2.
SFS-0 does not implement/activate SunFS, migrate root/boot or resume CPython.
