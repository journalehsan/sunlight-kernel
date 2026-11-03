# FastLane M5.4R scoped census reconciliation

Status: **PARTIAL — ownership UNRESOLVED**. Production FastLane remains disabled. No performance optimization or M6 benchmarking is authorized by this result.

Evidence is under `.fastlane-evidence/m54r/`. `initial-state.json` records HEAD, dirty paths and exact dirty-file SHA-256 values; `initial.patch` preserves tracked changes. `m54-preservation.json` hashes all surviving durable M5.4 artifacts, including failed attempts and source snapshots. `probes-preservation.json` hashes volatile probe artifacts; `preserved-probes/` copies their text receipts and source. Binary hashes are preserved without duplicating every probe image. `historical-native-matrix.json` records per-topology actual assessor results and exact canonical source-map digests. Original manifests retain the per-file hashes. No historical attempt was overwritten.

The measurement foundation report was [reconstructed](fastlane-m54-measurement-foundation.md) from these artifacts. Its same-CPU measurement capabilities do not resolve census ownership.

Census contract audit:

| Assertion / observation | Scope and claim | Legitimate interference / quiescence |
|---|---|---|
| `objects=views=frames=0` in FL-CENSUS | Restricted SHM broker inventory; all restricted entries, broader than an exact fixture object set | Other restricted clients could affect it. Census sweeps FastLane lifecycle first. Zero inventory alone is not physical retirement proof. |
| FL-FRAME backing/free by generation, region, physical frame | Fixture retirement inventory; one free per backing | Must correlate with mapping unmap completion and TLB records. Independently allocated foreign frames cannot discharge this obligation. |
| FL-MAP PID/incarnation/VM and FL-TLB request/ACK/completion | Process/VM identity and remote retirement ordering | Remote targets must ACK before unmap acceptance and physical free. A count alone cannot establish this. |
| Repeated-cycle global `shm` and `tables` equality, checkpoints 100–105 | Global system class inventory treated as a quiescent baseline | Background services can allocate. No global service allocation fence exists. This baseline is not proven quiescent. |
| Global `free` and census trace LIVE/TABLE_LIVE/SHM_LIVE/VIEW | Concurrent diagnostic observations | PMM and broker are locked during census under scheduler ownership. Snapshot consistency does not freeze later background allocations. |
| Session-racer pending-exit predicate | Lifecycle-specific bounded waiting | Exiting, unreaped racers delay census success; a live racer is allowed. This does not imply global allocator quiescence. |

The existing bounded census trace is debug-only and enabled through `fastlane_census_trace`. It has 65,536 atomic slots, explicit overflow/incomplete markers, no new hot-path lock, and records SHM object IDs, physical frames, view PID/incarnation/VM identity, PMM transitions and remote TLB events. The final checkpoint dumps its bounded event buffer. Traced replay uses `fastlane_mapping_test,key_inject,fastlane_census_trace`. Build and native manifests under `ownership/` bind this image to source.

The original untraced 4-CPU failures do not identify the extra physical frame. Session-racer timing is insufficient ownership evidence. Neither a census-contract error nor a physical-resource defect has yet been established. The global assertion remains unchanged; no tolerance or exception was added.

Before/after validation:

| Gate | Historical M5.4 | M5.4R |
|---|---|---|
| Measurement instrumentation, 2/4/12 | PASS/PASS/PASS | Not rerun as final matrix |
| Mapping, 2/4/12 | PASS / 2 FAIL + 1 PASS / PASS | Two traced 4-CPU PASS; discrepancy absent |
| Full required final-source regression matrix | Incomplete; see reconstructed report | NOT_RUN |
| Existing mapping parser controls | Historical evidence | 21 tests PASS; M5.4 parser 17 PASS + 1 skipped |
| Seven requested adversarial scoped-census controls | Not established | NOT_RUN; no scoped predicate introduced |
| Miri | Unavailable / NOT_RUN | Unavailable / NOT_RUN |

Historical M1 timeout attribution, M4 failures and SHM census drift remain UNRESOLVED. Current passing replays cannot reclassify them. Historical production exclusion evidence is not a new final-source release audit.

M6 readiness: **NOT READY**. Completion requires an exact frame/object/owner lifecycle witness, a justified minimal repair, deterministic fixture-leak and stale-identity controls, repeated final-source 4-CPU passes and the full 2/4/12 safety matrix. The smallest executable next step is the source-bound ownership replay described in `ownership/`, then a frame-by-frame difference of SHM_LIVE and PMM LIVE records at checkpoints 100–105 with VIEW_ADD/unmap/retirement correlation. Overflow, incomplete trace, missing owner or missing release must keep attribution UNRESOLVED.

Completed ownership attempts:

- `ownership/native-20261103T173400.709705Z`: 4 CPUs PASS; 61,528/61,528 events emitted, incomplete=0, overflow=0.
- `ownership-repeat/native-20261103T173527.434169Z`: 4 CPUs PASS; 61,548/61,548 events emitted, incomplete=0, overflow=0.

Both retain global SHM=1304, tables=381 through checkpoints 100–105. No SHM frame/object/owner set difference occurs across those checkpoints. Final restricted inventories are zero. These runs exercise the existing session-race-probe FL then FL-M2 arrangement but do not reproduce the original failing interleaving. Full original frame lifetime attribution remains UNRESOLVED, and these passes do not close the gate. `ownership-analysis.json` contains bounded extraction results and the raw serial hashes; `analyze_ownership.py` reproduces the extraction.

No kernel, service, assertion, timing or security code was changed in this investigation. Only reports and ignored evidence artifacts were added. The original dirty implementation remains intact. Consequently no new scoped predicate or native leak-injection control is claimed. The seven requested adversarial controls, sanitizer/release checks and required final-source 2/4/12 regression matrix remain NOT_RUN for M5.4R. Miri was rechecked and is unavailable. Check commands, results and logs are recorded in `checks.json`.

Executable next replay (no rebuild needed while source hashes match):

```sh
python3 tools/fastlane_mapping_validate.py --section native --census-trace --cpus 4 --timeout 240 --image-manifest .fastlane-evidence/m54r/ownership/build-20261103T173228.485778Z/validation-summary.json --evidence-root .fastlane-evidence/m54r/ownership-next
```

If bounded natural replays continue to miss the discrepancy, the next implementation step is an opt-in coordinated session-racer allocation hold across checkpoint 100, carrying an explicit SHM object ID and owner PID/incarnation/VM witness, followed by release and acknowledged retirement. Such a forced control proves the concurrent-resource distinction only for its own trace; it does not retrospectively identify the untraced original frame. Do not relax the historical failure on that basis.

Additional replay: `ownership-next/native-20261103T173917.315682Z` passed on
4 CPUs. Its trace emitted all 61,531 events with zero incomplete or overflow
records. `ownership-next-analysis.json` records no SHM frame/object/owner set
changes across checkpoints 100–105. This brings traced replays to three PASS,
but ownership remains UNRESOLVED and the historical failures remain FAIL.
No census predicate or kernel code was changed by this replay. The forced,
identity-carrying allocation control described above remains the next step;
negative controls and the final-source regression matrix are still NOT_RUN.

## Opt-in allocation hold control (2026-11-03)

The `--census-hold --census-trace` validator options build the service with
`fastlane-census-hold-test` and bind that selection into the image manifest.
The existing foreign session-racer actor holds one ordinary SHM page across
checkpoint 100, then explicitly frees it. The strict assessor is unchanged.
Build `hold/build-20261103T174553.038671Z` PASS; native
`hold/native-20261103T174613.597485Z` **FAIL**, preserved as such, with
`repeated cycles changed shared frame total`. This is a controlled demonstration,
not a passing regression gate or retrospective attribution of historical failures.

The complete trace has 61,553 events, no overflow or incomplete records.
`hold/ownership-analysis.json` identifies physical frame 615317504, unique trace
object 56, owner PID 58, incarnation 70, VM generation 126, VA 12884901888.
Creation/publication occur at sequences 36682–36684; checkpoint 100 records its
one live view. SHM is 1305 at checkpoint 100 and 1304 at 101–105; restricted
objects/views/frames remain zero. Unmap begins at 40150, TLB request 220 completes
at 40152, unmap completion is 40157, view release is 40158, and PMM free is 40159.
The frame is subsequently reused as a page table at 40160: frame identity alone
must therefore never replace object/lifetime identity. This request has zero
remote targets; it does not demonstrate a pending remote ACK control.

The controlled independently owned allocation establishes that global constancy
can reject a valid concurrent lifecycle. Original failures remain UNRESOLVED.
No scoped predicate or leak-injection acceptance has been implemented. Remaining
work includes exact lifecycle validation, negative controls (including remote ACK),
production exclusion, repeated final-source runs and the full regression matrix.
The existing 21 mapping parser tests and `git diff --check` passed during this step.
