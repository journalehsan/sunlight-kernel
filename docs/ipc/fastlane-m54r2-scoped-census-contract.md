# FastLane M5.4R2 — scoped census contract and adversarial closure

Status: **PASS within the current-source contract — primary and safety matrices complete**.
Exact historical incident attribution remains **UNRESOLVED** for the evidence limits below.
Production FastLane remains **DISABLED**.
No M6 benchmark or performance optimization is part of this milestone.

## Source and preservation

HEAD is `e5ed4849e57cb729252ba7ad0c25efcfe7fe5ae3`; the tree was already dirty.
Before edits, `.fastlane-evidence/m54r2/preservation-20261103T175058.045587Z/`
saved `initial.patch`, all dirty files including untracked files, exact dirty
SHA-256 values, the canonical source map, and hashes of 590 surviving historical
text/manifests/log artifacts. The initial canonical source identity is
`b46034b109aae5742845a47ef1310088d1eb6a8a718c072eed64a0483c79df8c`.
Historical binary hashes and original manifests remain in their original paths.

The final validation source has 1,386 canonical files and identity
`5b20cba2627952f162263b72be359a8c314ebf963874b0095793d10a36991f74`.
Canonical means SHA-256 of the sorted source map encoded as JSON with
`separators=(',', ':')`; `sources()` in `fastlane_f11b0e_validate.py` defines
the executable/build input inventory. Reports and ignored evidence scripts
are outside that map and are separately hashed in the final evidence index.
Exact per-file source and dirty-tree hashes are in each build/native manifest
and the final matrix manifest identified by `final-matrix-path.txt`. Original dirty changes are
preserved; no commit or push is made.

## Contract repair

Before, `assess_mapping()` required both SHM and page-table global counts to
have one value across checkpoints 100–105:

```python
len(cycles) == 6 and all(
    len({c.get(k) for c in cycles}) == 1 for k in ('shm', 'tables'))
```

This treated concurrent, internally consistent snapshots as globally quiescent
baselines. The scheduler/PMM/broker locks make an individual census consistent;
they do not freeze independent services between censuses. The session-racer
pending-exit predicate waits for exiting actors, while allowing live actors.
There is no global allocation fence.

After, the acceptance predicate is a conjunction of three distinct contracts:

| Contract | Acceptance obligation | Interference and limit |
|---|---|---|
| Exact fixture lifecycle | Unique `(channel generation, region)` backing lifetime; authenticated role PID/incarnation/VM; one or two distinct role views; matching backing and permissions; exactly one unmap request/completion and TLB request/completion per view; exact remote ACK set; one final physical free after all completions; no live fixture lifetime at any accepted census or end | Another allocation/free has a different object lifetime and cannot discharge an obligation, even on a reused physical frame |
| Restricted broker inventory | `objects=views=frames=0` at every accepted checkpoint, separately from exact physical retirement | Counts include every restricted client. This isolated fixture assumes no independently active restricted client. Concurrent restricted clients could legitimately make this inventory nonzero and would require a separately scoped inventory fixture; ordinary foreign SHM is unrestricted |
| Global census | Preserve every SHM/table/PMM-free observation as `CONCURRENT_DIAGNOSTIC`; require valid nonnegative fields and complete checkpoint coverage | Quiescent baseline equality is `NOT_REQUIRED` because global quiescence is `NOT_ESTABLISHED`; no tolerance or compensating delta is used |

The generation allocator uses checked increment and refuses exhaustion. A region
is backed once within its channel generation, so `(generation, region)` is a
generation-qualified object lifetime within one source-bound boot. A PMM address
is backing evidence, never an object identity. Authenticated `FL-BIND op=2`
records bind each mapping's PID, incarnation and VM generation to its role.
Duplicate bindings/backing lifetimes, orphan map/TLB/free records, missing frees,
duplicate frees and stale recipient identities fail. Every free is matched to
the same generation, region and backing frame, with completed retirement before
fixture quiescence. Broker metadata balance alone proves no physical release.

The existing production VM unmap path is unchanged: preflight verifies every
PTE/backing and exact VM-region ledger, synchronous TLB invalidation completes,
empty tables may then be reclaimed, the VM region/view is removed, the broker
releases its final reference, and PMM receives the physical frame. The repair
changes the host acceptance predicate and opt-in fixtures only.

Optional ownership traces have 65,536 slots. Requested traces require exactly
one final dump, complete consecutive sequences, matching reserved/emitted counts,
zero incomplete/overflow, fixed event schema and valid bounds. A trace cannot
quietly become optional when requested evidence disappears. Diagnostic records
are not authority. Mapping receipt input is bounded to 32 MiB. No new kernel lock,
allocator path, IPC protocol or waiting loop is introduced.

When present, the diagnostic SHM IDs must also agree with the independent
mapping ledger: one exact VIEW_ADD per authenticated PID/incarnation/VM/VA,
one SHM object per channel/region lifetime, the recorded allocation owner,
the exact backing and view-release inventory, per-view unmap and remote ACK
records, and the next physical transition to PMM Free. A free while the object
is live or a second free before a new claim/reclassification fails. A subsequent
allocation lifetime may reuse the address. Contradictory fixture and independent
owners are rejected even when the trace has no missing sequence numbers.

## Preserved controlled failure witness

The original M5.4R hold run remains **FAIL** at
`.fastlane-evidence/m54r/hold/native-20261103T174613.597485Z/` with
`repeated cycles changed shared frame total`. It has all 61,553 trace events,
zero incomplete and overflow. At checkpoint 100 unrestricted SHM object 56,
frame 615317504, owner PID 58/incarnation 70/VM generation 126, view
VA 12884901888 is live. SHM changes 1305 → 1304; restricted inventories remain
zero. Creation/publication are sequences 36682–36684; unmap request 40150,
TLB request/completion 40151/40152, unmap completion 40157, final view release
40158 and PMM free 40159. Sequence 40160 reuses the frame as a page table.
That zero-remote-target request is not the remote-ACK proof.

This establishes the flaw in global constancy for its own controlled run.
Reassessment with the new predicate is a development control, not a change to
the original classification or retrospective identification of an untraced
historical frame.

## Final-source positive concurrency and remote ACK witnesses

The final-source `native-20261103T183748.985985Z` receipt under
`matrix-20261103T183322.818190Z/hold-4-repeat1/` is PASS.
`matrix-20261103T183322.818190Z/scoped-witnesses.json` binds its serial hash,
source map, independent lifecycle and still-running remote view to that manifest.
The independent page is SHM object 57, frame 617775104, owner
PID 59/incarnation 71/VM 128, VA 12884901888.
It remains live at checkpoint 100 with no restricted-channel attribution record.
The same guest records the controlled 1305 → 1304 global SHM variation and zero
restricted inventory. Creation is OWN sequence 36737, final view release
40213, and the next physical transition is PMM Free at 40214.
Object/owner/view and physical-retirement correlation all pass; subsequent frame
reuse is a different lifetime. The global variation remains in the receipt.

The guest separately demonstrates real remote invalidation for fixture lifetime
`(generation=27, region=0)`,
SHM object 53, backing frame 612794368,
consumer PID 67/incarnation 79/VM 145, VA 12884901888:

| Per-object event | Mapping-trace sequence | Actual identity |
|---|---|---|
| Published consumer view | 140 | Role 1, read-only, NX, published on CPU 1 |
| Unmap requested | 146 | Exact PID/incarnation/VM/VA |
| Remote TLB request | 147 | Request 214, initiator CPU 0, target mask 2 |
| Target ACK | 148 | CPU 1, matching observed request, flushed=1 |
| Synchronous TLB completion | 149 | Same request, target mask, VM and VA |
| Unmap completion | 150 | Same complete process/view identity |
| Physical frame release | 151 | Same generation, region and backing frame |

A nonzero target actually flushes before unmap completion and physical release.
Each native mapping gate requires remote and zero-target cases, excludes the
initiator from remote target masks, and requires a still-running remote VM flush.
Full records and SHM object correlations remain in the physical-object receipt.

## Adversarial controls and proof boundaries

| Control | Mechanism and required result | Proof class |
|---|---|---|
| E1 fixture leak | `--census-leak --census-hold --census-trace` retains a normally authorized fixture mapping at checkpoint 999 while freeing an independent ordinary SHM allocation; exact gate must remain FAIL | Native safe leak fixture; no retirement bypass |
| E2 stale identity | Native repeated lifecycle asserts old-handle `Stale` after row reuse; present stale generation, PID, incarnation and VM in individually saved mapping receipt models; REJECT | Native authority refusal plus adversarial acceptance-boundary models |
| E3 duplicate/wrong lifetime | Present a second tagged retirement, a second PMM free before reuse, or a different generation; REJECT; host `SharedLifetime` tests reject release underflow | Safe receipt models and host lifetime tests; no native double free |
| E4 remote ACK | An actual nonzero remote target must produce exact matching request and target ACK, then TLB completion, unmap completion and physical free, all under the same object/view identity | Native per-object chain; zero targets do not qualify |
| E5 premature free | Move a real remote object's modeled physical retirement before its actual ACK; REJECT with the explicit order error | Safe acceptance-boundary model over a native chain; unsafe native early-free injection is NOT_RUN |
| E6 concurrency | Normal foreign hold must PASS; its independent identity/view and release are recorded; deleting fixture retirement while retaining all foreign frees must FAIL | Native concurrency plus compensation attack model |
| E7 incomplete trace | Remove closure or the whole trace, force overflow or a sequence hole, or contradict the fixture/independent owner; NOT_QUALIFIED/FAIL | Bounded evidence fault injection; actual negative verdict preserved |

`fastlane_m54r2_controls.py` requires a passing native manifest with the exact
current source and serial hash before generating models. Each model is retained
as `*.serial.model.log` with a machine-readable negative verdict. Expected
rejection is recorded separately from `gate_status=FAIL`; it is never relabeled
as a passing native run. The native leak summaries likewise remain FAIL.

The safe premature-retirement model tests the gate against an impossible order.
It does not make a kernel free before an ACK and does not certify native fault
recovery after such a violation. The unchanged production path's synchronous
ACK wait and the native positive ordering chain are separate evidence.

## Validation results

The final-source primary and independent IPC/authority/exec/exit/scheduler safety
matrices are PASS. The completed safety receipts were inspected in the focused
follow-up; no full matrix was rerun. The final readiness assessment has zero unmet
criteria and matches the same canonical executable source. Miri is
`UNAVAILABLE / NOT_RUN` (`cargo-miri` is absent).

`measurement-summary.json` under the primary evidence directory records the
current measurement qualification:

| CPUs | Qualified resident probes | Nonresident / NOT_QUALIFIED | Max qualified uncertainty (ppm) | Consistent CPU-time records | Discontinuities / migrated windows |
|---|---|---|---|---|---|
| 2 | 10 | 6 | 1015 | 8 / 8 | 0 / 0 |
| 4 | 10 | 6 | 837 | 8 / 8 | 0 / 0 |
| 12 | 8 | 8 | 724 | 8 / 8 | 0 / 0 |

Qualified probes have zero regressions. These are bounded same-CPU observations;
nonresident probes do not become durations. Native measured-window migration was
not forced. Cross-CPU timing, blocked time, user/kernel split and per-actor
allocation peaks remain unqualified. Resource snapshots are concurrent global
deltas; physical reclamation is established separately by object retirement.

Host kernel/ring/Doorbell/IPC/UAC/scheduler units, dispatch-boundary checks,
format/clippy/bare-metal checks and parser controls PASS. Parser discovery ran
305 tests with 18 conditional native-attachment skips and no failures; actual
native gates are assessed separately. Hostile-memory ASan and TSan tests PASS.
The symbol-bearing production debug ELF has no measurement syscall symbol;
debug and ordinary release ELFs contain zero M5.3/M5.4, mapping/TLB/census,
foreign-hold or fixture-leak markers. The ordinary release build PASSes.
`production-debug-audit.json` and `release-audit.json` retain binary hashes and
the individual zero counts.

<!-- M54R2_MATRIX_BEGIN -->
Final-source primary evidence: `.fastlane-evidence/m54r2/matrix-20261103T183322.818190Z/`.
Primary orchestration status: **PASS**. Every row below comes from retained native manifests.

| Gate | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| M2 mappings / M3.2 hostile words | PASS | 3 × PASS | PASS |
| Controlled foreign hold | PASS | 3 × PASS | PASS |
| Fixture-owned leak rejection | FAIL (expected) | 2 × FAIL (expected) | FAIL (expected) |
| M1 authority / coordination | PASS | PASS | PASS |
| M3.1B page-table reclamation | PASS | PASS | PASS |
| M4 authenticated Doorbell | PASS | PASS | PASS |
| M5 mediated transfer | PASS | PASS | PASS |
| M5.1 repeated transfer | PASS | PASS | PASS |
| M5.2 shared-ring transfer | PASS | PASS | PASS |
| M5.3 bounded instrumentation | PASS | PASS | PASS |
| M5.4 clock / CPU accounting / resources | PASS | PASS | PASS |

Full IPC/authority/exec/exit/scheduler safety matrix, host tests, sanitizers and production exclusion PASS; completed receipts are assessed in the decision below.
<!-- M54R2_MATRIX_END -->



An initial orchestration predicate was too strict for the native leak control:
it expected a missing physical free at the end of the log in addition to a leak
at checkpoint 999. The expected guest assertion starts ordinary process-exit
cleanup, which can free the frame after that failed checkpoint. The native
assessment still correctly reports `restricted resource drift` and
`fixture lifetime outstanding at census`. The original driver failure is
preserved. `orchestration/verify_native_leaks.py` checks the same immutable
receipt against the actual checkpoint contract: one exact backing outstanding
at checkpoint 999, a prior independent physical free, complete diagnostic
evidence, and an actual gate FAIL. Any later ordinary exit cleanup is retained
in the witness. No successful replay is used to explain this harness error,
and no guest FAIL is changed to PASS.

**Preserved build-provenance failure.** The final-source attempt under
`matrix-20261103T182653.020598Z/hold-build/build-20261103T182938.448397Z`
failed before boot with `service not embedded: uac_service`. Kernel build took
0.26 seconds and reused an ELF containing the leak variant's UAC bytes,
SHA-256 `7c3b6ba4adf2e7e9ff4d9d5a232bd1d3224f8f4bd82f8294a994d745fcfa488d`,
at offset 109754648. `stale-embedding-analysis.json` identifies the matching
archived leak services. The expected hold service was absent. This is an
explained build-cache/provenance failure; no guest result from that image was
accepted. Final orchestration cleans only generated kernel package artifacts
before each image build, then retains exact embedded-service, ELF and ISO checks.
Source identity is unchanged. Archived images and original FAIL classifications
remain outside Cargo targets.

`interrupted-attempts.json` records developmental interruptions and the startup
wait that encountered an incomplete receipt from an interrupted controller.
An incomplete RUNNING receipt is classified ABORTED/NOT_QUALIFIED externally;
it is not rewritten as a passing test or retrospectively called a guest failure.

## Historical limits

The original two untraced M5.4 4-CPU mapping attempts remain **FAIL**; the later
successful replay and three successful traced M5.4R replays do not explain them.
Historical M1 timeout attribution, historical M4 failures and SHM census drift
remain **UNRESOLVED**. The controlled session-racer witness identifies only its
own allocation lifetime.

### Focused historical follow-up (2026-11-03)

Evidence: `.fastlane-evidence/m54r2/historical-followup/reassessment.json`;
reproduce with `python3 .fastlane-evidence/m54r2/historical-followup/reassess.py`.
The reassessment hashes each original serial against its native manifest and
retains a separate current-predicate result. Original manifests, serials and
FAIL verdicts are unchanged. Reassessment is not a new native execution.

| Historical question | Demonstrated mechanism / repair | Remaining incident evidence |
|---|---|---|
| M1 entry timeout | IF-clear entry spin can occupy a peer's run slot; bounded scheduler yields and impossible-placement rejection are already implemented. Retained failing spin control and nine passing corrected unpinned controls have verified manifest/serial hashes. | Original failed image/serial and producer scheduling state are unavailable. The prior fixed known-location/archive recovery search was exhausted. Exact original attribution cannot be reconstructed. |
| M4 continuation failure | Software reschedule inserted IF into a CPL0 syscall continuation; preserving saved RFLAGS until SYSRET is already implemented. The retained pre-repair control still hashes correctly and records `Doorbell kernel continuation resumed with IF set`. | The two original failed image/serial pairs were deleted by the earlier disk-exhaustion `cargo clean`. The later controlled defect does not prove either deleted guest's exact causal chain. |
| M5.4 census failures | Both original 4-CPU serials fail only the old global-constancy predicate. Direct reassessment of those same serials passes the repaired exact lifecycle contract, including every fixture physical retirement. Global equality has no quiescence guarantee. | Neither original run requested ownership tracing. The extra global frame's object/owner remains unidentified; a new successful traced guest cannot identify it retrospectively. |

The original census details are now checked directly: in
`mapping/native-20261103T163851.028220Z`, SHM is 1305 at checkpoint **101**,
and 1304 at 100 and 102–105. In
`mapping-rerun/native-20261103T165420.723275Z`, SHM is 1305 at checkpoint
**100**, and 1304 at 101–105. Both have tables=381 and zero restricted
objects/views/frames throughout. This corrects the M5.4 foundation report's
description that placed both transients at checkpoint 100; that historical
report and its preserved hash are retained unchanged.

Only one fresh native guest was run:
`historical-followup/census-only/native-20261103T202244.896667Z`, **4 CPUs PASS**,
using the source-matched archived hold image, with required complete ownership
trace and independent SHM lifecycle. Its serial SHA-256 is
`089cffc9b301ebe5b84f23d84294e19a73951d4c3f69ef080ede6c1a06f7340e`.
Three selected host controls PASS: global variation remains diagnostic, a
foreign free cannot replace a fixture free, and quiescence rejects an outstanding
fixture lifetime. No other native suites, builds or benchmarks were started.

The repair mechanisms and current regression closure are established. Strict
historical causal closure stays UNRESOLVED because the necessary observations
are absent. Closing it requires recovery of the original M1/M4 artifacts and
previously unrecorded census ownership evidence; changing a status or rerunning
the full suite cannot supply those facts.

Measurement qualification remains independent: same-CPU PIT-qualified durations
and scheduler-charged time have explicit limits. Cross-CPU subtraction/latency,
blocked-time attribution, user/kernel time split and per-actor allocation peaks
remain unqualified. Only passing measurement and final-source safety gates could
support a recommendation for a gated same-CPU M6.0 experiment. Production
FastLane remains disabled regardless of that recommendation.

## Security review

`security-final-source-audit.json` and `m54r2-only.patch` compare this milestone with
the preserved dirty source, rather than incorrectly attributing prior M5.4
changes to this repair. Seven executable inputs changed: the UAC feature list,
mapping fixture, mapping assessor/validator, scoped diagnostic assessor,
adversarial runner and scoped parser tests. Kernel and IPC inputs are byte-for-byte
unchanged from preservation. No new unsafe block, global lock, allocator policy,
scheduler behavior, capability authority, PTE permission or hostile-memory access
was introduced. The leak control uses ordinary authenticated allocation/mapping
and simply reaches the diagnostic checkpoint while the mapping remains live.
Its independent free uses the ordinary SHM API. Features are off by default;
the final production binary audit is an additional requirement.

The existing M3.2 contract remains memory safety and private scalar snapshots;
coherent peer publication and application authenticity are not supplied.
Trace diagnostics never authorize an operation or retire an object. Native TCG
runs provide functional evidence, not performance measurements.

## Final decision after focused follow-up

Machine-readable decision: `.fastlane-evidence/m54r2/final-readiness.json`.
Measurement foundation: **QUALIFIED_WITH_BOUNDS**. Primary final-source FastLane
2/4/12 matrix, all seven adversarial categories, host/sanitizer checks and
production debug/release exclusion: **PASS**. Full current-source regression
readiness: **PASS**. Historical causal closure: **UNRESOLVED**.

The completed independent safety run at `safety-20261103T192044.990473Z` passes
mapping compatibility, IPC, authority, Account exec, exit modes 11/12/13,
scheduler, remote wake and generic Doorbell on 2/4/12 CPUs. Inspection through
`python3 .fastlane-evidence/m54r2/orchestration/final_readiness.py` returned
**COMPLETE**, zero unmet criteria, unchanged executable source and preserved
historical evidence. Its dated decision is
`readiness-20261103T202300.315824Z.json`; the earlier
`readiness-20261103T192830.260328Z.json` remains the original PARTIAL handoff.
This completes current-source validation without claiming lost historical
observations. Production FastLane remains disabled. No M6 benchmark, commit or
push was performed.
