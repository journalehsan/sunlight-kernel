# SFS-0 design review checklist

Review date2026-11-14. PASS below means the requested design is present and was
reviewed against current sources, not implemented or independently proven safe.

| Gate | Design review result and evidence |
|---|---|
| G1 PASS | Capability inventory and v0.1 comparison trace actual VFS/service/FAT/tmpfs/block/driver/tool/libc/apps and PY5–7 owners, distinguish history/current code |
| G2 PASS | ADR0001 compares A/B/C against all requested criteria and selects bounded compact checkpointed log with costs/stop criteria |
| G3 PASS | Architecture/integration define UUID+nid+incarnation, directory references, no recycle, link vs open/pin ownership, rename/unlink/orphan and future POSIX contracts |
| G4 PASS | Format and field inventory propose all fourteen structures/classes, explicit encoding/checksums/reserves/validation/version/features/recovery; offsets remain experimental |
| G5 PASS | Block assessment/recovery contract distinguish accepted/completed/durable, unsupported/failed flush, partial/uncertain writes, cache reorder, required missing geometry/error/fencing API |
| G6 PASS | Recovery contract defines complete closure, D/P/R/R0 barriers, three slots/two retained graphs, all-CP union, classification/fallback, serialized fsync and close semantics |
| G7 PASS | Architecture defines segment transitions, summary authority, self-accounting planning, no early reuse, ordinary-commit relocation, cleaner reserve and bounded ENOSPC |
| G8 PASS | Roadmap specifies hosted volatile/durable image model, deterministic schedules/seeds/tears/reorder/errors, first real vertical test, oracles, traces and independent read-only checker |
| G9 PASS | Integration stages A–F and SFS1–8 roadmap specify scope/dependencies/acceptance/crashes/security/evidence/stops; installation deferred and CPython paused |
| G10 PASS | validate.py verifies source bytes, document links/required outputs, field arithmetic and CRC convention; source-integrity manifest and machine-readable summary bind evidence |

Cross-review corrections made before finalization: explicit payload totals136/160/200;
CRC-free metadata summary hints avoid self-checksum cycles; full metadata relocation
requires reference-ancestor copies; ID rollback cannot permit writable corruption
fallback; metadata self-accounting reserves are preplanned;64-byte native enumeration
cannot silently truncate255-byte SunFS names. These are design corrections, not
runtime bug fixes or proof by serialization. Codec/image/recovery/device execution
remain NOT_RUN; inability to resolve any of these in SFS-1/2 is a stop condition.
