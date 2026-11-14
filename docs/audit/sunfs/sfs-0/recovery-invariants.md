# Recovery invariant review

Normative protocol: docs/filesystems/sunfs/sunfs-crash-recovery-contract.md.
This is a design review matrix, not an executed crash-test result.

| Hazard | Invariant / proposed prevention | Required later evidence |
|---|---|---|
| CP names incomplete data | I1/I2, dependency flush D before CP publication P | Every write/flush/tear cut |
| Older CP points into reused blocks | I3/I4/I5, union ownership + zero-slot flush R | Retained-old reads after repeated reuse |
| Three slots assumed sufficient | Retain closures, not merely record copies; R0 if third survives | Three-valid-slot restart/retirement traces |
| Torn publication accidentally accepted | Full CRC plus all dependency graph validation | Torn and checksum-recomputed malformed fixtures |
| Valid CRC but bad graph | Full independent ownership/namespace checks | Missing nodes/cycles/duplicate blocks |
| Failed flush treated as no write | I6, uncertain outcome/error latch/quarantine | Partial/all-persisted flush errors |
| Interrupted GC | I7 ordinary commit and fallback pins | Every relocation/retirement cut |
| ID reuse after rollback | I8 nonrecycling + read-only corruption fallback | Stale-generation/name-replacement/exhaustion |
| Orphans lost or leaked unsafely | Persistent nlink0 ownership; reap via later commit | Crash unlink/open-fsync/last-close/reap |
| Automatic repair hides corruption | I9 read-only inspection; before/after image hash | Corruption fixtures and no-write assertion |
| Concurrent fsync acknowledges wrong cut | I10 one writer epoch/serialized sync | Mutation/fsync schedule oracle |
| Full disk forces unsafe free | Preflight bound + cleaner-only reserve | ENOSPC at each admission stage |

Any violation blocks SFS-2/SFS-4 as applicable. Unknown-corrupt-slot handling prefers
read-only fallback to ambiguous writable recovery; this explicit availability limit
must remain in test expectations. CRC collision resistance and honest device barriers
are assumptions, not proven invariants. No crash-safety PASS is issued in SFS-0.
