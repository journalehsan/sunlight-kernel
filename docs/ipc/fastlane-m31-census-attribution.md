# FastLane M3.1 bounded census attribution — PARTIAL

2026-11-01, Asia/Tehran. Production FastLane remains disabled. This milestone
adds observation and assessment only. No payload transfer, userspace Doorbell,
resource threshold change, cleanup delay, lifecycle repair, commit or push.

## Original observations and Phase A evidence boundary

The requested `target/fastlane-m3-evidence/validation-summary.json` and the
original source copies, image manifests, embedded services and raw serial logs
are unavailable in this checkout. A fresh filename search in accessible
`/home/ehsantor/Projects` and `/tmp` found none. The M3.0 reports remain secondary
evidence. Their original count failures are preserved below; no later receipt
supersedes them. Current HEAD is `4649c3a695d509c0fbf48b279464265c424b9954`.
Pre-existing worktree version/manifest changes are preserved. Neither HEAD nor
the existing unmanifested ISO establishes the original executable identity.

| Topology | Original reported failure | Demonstrated cause |
|---|---|---|
| 2 CPUs | Page tables 391 at 100–103, 392 at 104–105 | UNRESOLVED; the additional allocation identity is unavailable |
| 4 CPUs | SHM 1304 through 104, 1305 at 105, 1304 at 999 | UNRESOLVED; a lower total at 999 does not establish retirement of the same frame |
| 12 CPUs | Page tables 391 at 100, 392 at 101–105 | UNRESOLVED; the additional allocation identity is unavailable |

None of these is classified as harmless, leaked, retained, deferred, cached or
an instrumentation defect. Exact original replay is **UNAVAILABLE**. The new
source-bound diagnostic image is a separate experiment, including when it
reports the same totals as an original attempt.

The checkpoint boundaries are unchanged: each of six cycles creates/maps an
actor, retires/reaps it, creates a replacement with a fresh generation, rejects
the stale handle, stops the replacement, and calls `census(100 + cycle)`.
Checkpoint 999 follows stopping the separate foreign actor and destroying the
endpoint. Inventory still sweeps restricted retirement, takes PMM and broker
locks under the existing scheduler lock, and waits for matching exiting actors
to be reaped before printing. No extra scheduling wait or sleep was introduced.
Checkpoint 999 has a different ownership boundary from checkpoints 100–105.

## Observation implementation

`fastlane_census_trace` is a new opt-in kernel feature depending on the existing
mapping fixture. Every hook and its storage is gated by both that feature and
`debug_assertions`. Default mapping images, ordinary images and release images
exclude it. The recorder has 65,536 fixed slots (5 MiB plus counters), unique
atomic reservations, eight atomic metadata words per slot, and Release/Acquire
publication. It does not allocate, acquire a lock, retry, change an authority
result or read shared payload bytes. NMI handlers can record acknowledgements
without printing or taking locks. Overflow and incomplete publication invalidate
the receipt. Storage is never wrapped or overwritten.

The PMM hooks include single claims, contiguous claims, class transitions and
frees. Recorder revision 3 additionally observes `OtherAccounted` transitions:
the existing boot security test mints a zero-view SHM object from a normally
allocated OtherAccounted frame and retires it without changing its primary
class. Earlier trace assessments lacked that physical claim and correctly
remain FAIL. The final observer captures it explicitly; the compared census
classes remain PageTable and SharedMemory, with unchanged accounting. Logical
broker owner and PMM accounting owner are recorded separately. At every existing successful census call, PMM supplies all currently live
PageTable and SharedMemory frame identities under its existing lock. The same
boundary records lower-half page-table parent links, VM generation/root, owner
PID, virtual range, level and occupied-entry count; broker objects supply stable
diagnostic IDs, owner PID, current view count, retired state, restricted status,
channel association when still present, and individual process view records.
Mapping-created table events include the mapping VA, leaf frame and flags.
The allocation identity is the claim/reclass event sequence plus frame address,
so reuse of an address is distinguishable from retention.

SHM diagnostic IDs are boot-local numeric counters, independent of bearer
tokens. No token, secret material, payload or memory contents are recorded.
PMM's existing packed PID is only a 24-bit accounting tag, so it is not used as
lifecycle authority; VM generations and process incarnation view records provide
separate correlations. Kernel-shared table ownership outside the lower-half
walk remains explicitly unmatched if it appears as a new allocation.

TLB observations record exact VM/root, VA/range, sequence, target mask, initiating
CPU, handler-produced acknowledgements and synchronous completion. Explicit
SHM unmap request/completion observations bracket every page's invalidation.
The existing leaf-unmap path still completes the shootdown before removing the
view, transferring final backing and returning frames to PMM. Empty-table
reclaim retains its existing covering-shootdown and inactive-VM requirements;
this work does not add or bypass reclamation.

Only a short capacity receipt prints at intermediate checkpoints. All events
are dumped at checkpoint 999, after capturing that boundary, to avoid inserting
large serial dumps between the six comparison checkpoints. Snapshot scanning
and atomic recording still affect timing and image layout. A diagnostic pass
therefore cannot erase an untraced failure.

## Trace format and independent reconciliation

`[FL-OWN] seq=N kind=K x0=... ... x7=...` has the following metadata schema.
Unused words are zero. Full original serials retain all events, including
baseline allocations and frees; ownership JSON contains per-checkpoint additions
and their complete physical/broker history. No extra frame is accepted merely
because the restricted inventory is zero.

| Kind | Eight words x0–x7 |
|---|---|
| 1 claim, 2 free, 3 reclass | frame, old/current class, PMM owner PID, new class, 0, 0, 0, 0 |
| 4 live frame | frame, class, PMM owner PID, checkpoint, 0, 0, 0, 0 |
| 5 table created by map | child frame, VM generation, root frame, parent frame, mapping VA, child level, leaf backing frame, leaf flags |
| 6 root | frame, VM generation, PID (zero at creation), checkpoint (zero at creation), incarnation (zero at creation), 0, 0, 0 |
| 7 SHM create, 8 add, 9 release, 10 retire, 11 live | frame, object ID, owner PID, view count, retired, restricted, checkpoint (zero for transitions), backing page count |
| 12 view | object ID, PID, incarnation, VM generation, VA, bytes, checkpoint, 0; channel association uses object ID, channel generation, region, 0, 0, 0, checkpoint, 1 |
| 13 TLB request | request sequence, VM generation, root, start VA, pages, remote target mask, CPU, active mask |
| 14 ACK produced | request sequence, VM generation, root, start VA, pages, 0, handler CPU, flushed-current-VM |
| 15 TLB complete | request sequence, VM generation, root, start VA, pages, remote target mask, initiator CPU, 0 |
| 16 table at census | frame, VM generation, root, parent, covered VA base, level, occupied entries, checkpoint |
| 17 checkpoint | checkpoint, recorder revision (3 in final image), 0, 0, 0, 0, 0, 0 |
| 18 SHM unmap | object ID, VM generation, root, VA, bytes, complete (0 request / 1 complete), PID, references before release |
| 19 view admitted | object ID, PID, incarnation, VM generation, VA, bytes, 0, 0 |
| 20 table retirement | frame, VM generation, root, covering VA (zero for teardown), condition, active CPU mask, 0, 0 |

Table retirement conditions observe the existing path: 1 empty-table reclaim
after covering shootdown, 2 exact-leaf rollback after shootdown, 3 final inactive
VM teardown, 4 newly created table rollback after shootdown, 5 unpublished root
rollback on ledger reservation failure. They do not authorize retirement.
View-admission records permit independent replay of all active view references;
final backing free must follow a zero-reference release/retirement transition.
The verifier checks each observed table free against its recorded retirement
condition, and covering acknowledgement completion where required.

`tools/fastlane_census_assess.py` checks contiguous unique sequences, storage
completeness, claim/free/reclass replay versus identity snapshots, and exact
identity-count agreement with the unchanged accounting census. It checks TLB
identity/range, complete target acknowledgement sets and ordering, and requires
each explicit SHM unmap's page completions before its completion record. It
compares allocation identities, including same-address reuse, against checkpoint
100. Added tables are correlated with live parent/root/VM/occupancy records;
SHM additions require matching object and individual view references.

`TABLE_LINKED_AT_CHECKPOINT` and `SHM_PINNED_AT_CHECKPOINT` describe observations.
They do not establish that retention is required by the workload or explain an
original failure. Empty/unmatched additions remain UNEXPLAINED. Trace assessment
is independent of M1/M2 acceptance; the original global-drift assessor and its
thresholds are unchanged.

## New experiments and validation

New attempts are under `target/fastlane-m31-evidence/`; original evidence paths
are not recreated or overwritten. Each runner retains source maps and copies,
commands, service/image hashes, raw serials, assessments and artifact hashes.
An initial sandbox attempt failed before boot because QMP socket binding was
not permitted; its FAIL manifest is retained. The permitted retry is separate.

The first diagnostic build is
`diagnostic/build-20261101T122617.145580Z/validation-summary.json`.
The first 2-CPU runtime is
`diagnostic/native-20261101T122743.499471Z/validation-summary.json`.
It passes the existing M1/M2 assessor, including real permission faults, frame
reuse and remote invalidation witnesses. Its 60,817 events have zero overflow
and incomplete publications; independent replay reconciles all PMM inventories,
254 shootdowns and 19 remote-target requests. Checkpoints 100–105 each contain
391 page tables and 1,304 SHM frames, with identical allocation identities.
Checkpoint 999 contains 383 tables and 1,304 SHM frames; the eight removed
allocation identities are retained in `2cpu.ownership.json`.

### Captured new 2-CPU census failure

The strengthened recorder's guest
`final-diagnostic/native-20261101T123846.885522Z/2cpu.serial.log` captures page
tables 391 at 100–102 and 392 at 103–105 (384 at 999); SHM stays 1,304.
The unchanged native assessor records FAIL. This is a new failure, not an
identity-verified replay of the original 104/105 increase.

`retained-table-2cpu.json` retains the exact event slice and serial hash:

| Identity | Value |
|---|---|
| Table frame / claim event | 620990464 / 47312 |
| Owning VM / root / parent | 79 / 473878528 / 513253376 |
| Mapping VA / table level | 12889096192 (`0x300400000`) / P1 |
| VM participants | Vortex PID 34 incarnation 46; borrower PID 35 incarnation 47 |
| Temporary backing / object | 620986368 / diagnostic object 65 |
| Additional peer view | PID 31 incarnation 43, VM 73 (sunlight-kv) |
| Last-view shootdown | Request 243, target mask 0; complete event 47326 |
| Final view withdrawn / object release / backing PMM free | 47327 / 47328 / 47329 |
| Table's final observed state | Still parent-linked, zero occupied entries, at 103–105 and 999; no table release/reuse in capture |

This demonstrates retained **address-space ownership in the new guest**. The
observed owner/peer pair is consistent with Vortex's `kv_get_bytes` path: that
source allocates a request page, sends a SHM-backed KV request, then calls
`shm_free`. This is source-level correlation; the trace does not record a call
stack or IPC opcode and does not prove the exact higher-level caller. The peer
identity is observed in the raw spawn log. No key or data bytes are logged. `free_shared_page` clears
the leaf through `unmap_page`, completes its synchronous invalidation, removes
the view/ledger record and releases backing; it does not invoke empty-table
reclamation. `reclaim_user_space` removes and frees the lower tables at final
inactive-VM teardown. The table remains reachable through the live Vortex root.
The ordinary SHM VA bump is monotonic, so a new range can require a new P1 table
after earlier SHM has been released.

This is neither evidence of deferred asynchronous reclamation nor an unowned
PMM frame. No arbitrary delay can be assumed to reclaim it: the demonstrated
retirement condition is final VM teardown or a separate safe empty-table reclaim
operation. **Whether this empty-table retention is an acceptable bounded cache,
or resource accumulation requiring a VM-policy repair, remains unproved.**
The persistent Vortex VM was not stopped to manufacture a census pass. The
strict verifier therefore retains this empty addition as unresolved, and the
native FAIL is not waived. A global-flat-count expectation does not isolate
FastLane ownership from this ordinary VM allocation, but is unchanged here.

That observer revision also lacks physical coverage for the boot test's
OtherAccounted SHM backing; its full-trace FAIL and exact error remain preserved.
Revision 3 fixes observation coverage only. It does not change accounting,
unmap, SHM bump allocation or table reclamation. Final revision 3 results are
recorded below. The original three causes remain unresolved even if they pass.

## Final-source validation matrix

The final diagnostic build is `complete-trace/build-20261101T124208.138572Z`;
native evidence is `complete-trace/native-20261101T124235.748917Z`.
The untraced build is `final-control/build-20261101T124236.929656Z`;
its native evidence is `final-control/native-20261101T124810.388490Z`.
All six guests use final-source manifests with verified ELF/ISO/embedded-service
hashes. Each guest runs the original six complete actor lifecycles.

| Image / CPUs | Tables 100–105 → 999 | SHM 100–105 → 999 | M1/M2 / M3 words | Ownership / overflow |
|---|---|---|---|---|
| Traced / 2 | 391/391/391/391/391/391 → 383 | 1304 → 1304 | PASS / PASS | Reconciled / 0 |
| Traced / 4 | 392/392/392/392/392/392 → 384 | 1304 → 1304 | PASS / PASS | Reconciled / 0 |
| Traced / 12 | 391/391/391/391/391/391 → 383 | 1304 → 1304 | PASS / PASS | Reconciled / 0 |
| Untraced / 2 | 391/391/391/391/391/391 → 383 | 1304 → 1304 | PASS / PASS | Not instrumented |
| Untraced / 4 | 391/391/391/391/391/391 → 383 | 1304 → 1304 | PASS / PASS | Not instrumented |
| Untraced / 12 | 391/391/391/391/391/391 → 383 | 1304 → 1304 | PASS / PASS | Not instrumented |

| Traced CPUs | Records | TLB requests / remote-target requests | Incomplete publications |
|---|---|---|---|
| 2 | 61571 | 260 / 20 | 0 |
| 4 | 61697 | 272 / 21 | 0 |
| 12 | 61583 | 262 / 15 | 0 |

The final 4-CPU guest allocates table **606158848** between checkpoints 25 and
100 (claim/link 36566/36567). It belongs to VM 79, root 473903104, parent
513277952, at the same SHM VA `0x300400000`; snapshots at 100 and 999 show it
linked and empty. `4cpu.pre-comparison-additions.json` preserves that comparison.
This explains its different starting table count, without certifying that the
retention policy is acceptable. Its six-cycle count comparison passes because
the allocation precedes the comparison window. The raw table remains visible.

| Other validation | Result and scope |
|---|---|
| Kernel / IPC / UAC host tests | PASS: 153 / 62 / 18 tests on Linux target |
| Focused scheduler host checks | PASS: production state, lifecycle, accounting and policy |
| Ring representation | PASS: 33 tests (21 hostile, 12 existing), four documentation tests, Clippy with warnings denied, no_std check |
| Parser/control suite | PASS: 218 tests, including 25 new sequence/overflow/ownership/reuse/TLB/retirement controls |
| Ordinary production image | PASS: source-bound ELF, ISO and embedded services; no authority/mapping/M3-word/ownership trace markers |
| Ordinary 2-CPU production boot | PASS: complete existing phase3_0 expected-message list plus security hardening, no panic or experimental markers; not a desktop/capture/performance claim |
| Ordinary release check | PASS |
| Release with debug test feature explicitly selected | REJECTED as intended by existing `fastlane_channel_test requires a debug experimental image` guard; both mapping-only and census-trace invocations return 101 |
| Raw extra-check driver | FAIL preserved: it expected zero exit for the deliberately forbidden release/test-feature combination; guard rejection is assessed separately |
| Whitespace / new Rust module formatting | PASS |
| Broader historical native IPC/grant contention, R2D, kernel Doorbell lineages | NOT RERUN in M3.1; prior reports retain their scope and limits |
| Miri / fresh ASan / fresh TSan | NOT RUN in M3.1; no new claim from M3.0 historical receipts |

Host checks are not native evidence. Model/parser controls do not substitute
for the real guest frame/ACK records. Passing untraced or revision-3 guests do
not discharge the earlier new failed guest, the observer coverage failures,
or the three original unknown allocations. The top-level summary remains
**PARTIAL** and names each unresolved allocation or missing identity.

Evidence entry points are `target/fastlane-m31-evidence/validation-summary.json`,
the immutable per-attempt manifests and serials, `retained-table-2cpu.json`,
each final `*cpu.ownership.json` and `*cpu.reassessment-v3.json`, and the final
root artifact index. No existing count gate was changed.

## Repeat native runs on the final traced image

These are separate experiments on `complete-trace/build-20261101T124208.138572Z`
(its `source_sha256` still equals the tree), not an identity-verified replay of
M3.0. Evidence is under `repeat-attempts*/` and `repeat-analysis-b/`
(`summary.json`, per-run assessor JSON, `correlate_all.py`).

| Attempt | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| `repeat-attempts` | ABORTED, no guest result (`ABORTED.txt`) | not run | not run |
| 1 | **FAIL**, kernel panic before any checkpoint | PASS | PASS |
| 2 | PASS | PASS | PASS |
| 3 | PASS | PASS | PASS |

The 2-CPU FAIL (`repeat-attempts-b/native-20261101T154928.904344Z`) is a panic
at `kernel/src/capability/channel_race.rs:43` ("channel race deadline", the
existing 5 s spin wait) in the M1 contended race. No census checkpoint was
reached. Its cause is undiagnosed and it is preserved unchanged. It is unrelated
to the census differences but is a real M1 liveness observation on 2 CPUs.

All eight guests that reached the census show tables 391 at 100–105 and 383 at
999, with zero unresolved additions. Seven show SHM 1304 throughout. One 2-CPU
guest (`native-20261101T155222.526086Z`) shows SHM 1304 at 100–105 and 1305 at
999. Its extra frame 633307136 is accounted:

| Field | Value |
|---|---|
| Claim / SHM create | seq 58107 / 58108, after checkpoint 105 (seq 58106) |
| Object, owner | 72, Vortex PID 34 incarnation 46, VM 79 |
| View | one page at `0x3003fe000`, 1 reference, not retired |
| Peer view | added at 58127, released at 58133 |
| State at 999 | SHM_PINNED_AT_CHECKPOINT; no later events exist because the dump is taken at 999 |

This is a live page owned by the persistent Vortex VM at the census instant,
consistent with the request page of a `kv_get_bytes` call in flight. It is an
accountable owner, not an unowned frame. It does not prove final release,
because the guest ended before the next event. It also is not the original 4-CPU
difference (that was +1 at checkpoint 105 on 4 CPUs); it only shows the same
class of transient service-owned page can appear in a window the strict global
count does not isolate.

### Page-table cause (VM 79 SHM cursor)

Across every traced serial, VM 79's SHM view end (a proxy for the monotonic bump)
sits just below `0x300400000`. When that cursor crosses the 2 MiB boundary, a new
empty P1 table is linked and stays linked, because `free_shared_page` never calls
`reclaim_empty_tables_for_page` (only `process/mmap.rs` does). The
table count goes 391 to 392 exactly in the interval where the cursor crosses
(`final-diagnostic/native-20261101T123846.885522Z`, checkpoint 103) and 4-CPU
`complete-trace` starts at 392 because the crossing happened before checkpoint
100. Runs whose cursor stays below the boundary stay flat at 391. This links the
mechanism to every new +1 table observation and is consistent with the original
2-CPU and 12-CPU differences, but the original archive is unavailable, so those
two causes remain **UNRESOLVED** by the strict rule above. Per-cycle accounting
is identical in all runs: 16 links and 20 retirements per cycle, zero links from
105 to 999.

Retention is bounded by VM lifetime (final teardown retires the table with
condition 3) and by the 4 GiB SHM window (at most about 2048 P1 tables per VM,
derived from the constants and not run to exhaustion). Whether this is an
acceptable cache or needs repair is still unproved.

## Next smallest executable milestone

Recover the original M3.0 archive and verify its source/image/service and raw-log
hashes before replaying it unchanged. The prior M3.0 conversation confirms that
commit `4649c3a69` was made after M3.0, but its available final messages do not
recover the original failing images, manifests or physical allocation identities.
Preserve every failed attempt. If recovery is impossible, those historical
identities remain unavailable.

Immediate executable follow-up, using the existing final diagnostic image:

```sh
python3 tools/fastlane_mapping_validate.py --section native --census-trace --cpus 2 \
  --image-manifest target/fastlane-m31-evidence/complete-trace/build-20261101T124208.138572Z/validation-summary.json \
  --evidence-root target/fastlane-m31-evidence/follow-up
```

The next smallest diagnostic implementation is a separately gated **generic
SHM VA-boundary and final-VM teardown case** beside the existing mapping fixture.
Use a bounded count of normal SHM allocations to cross the observed 2 MiB P1
coverage boundary; free each view through the existing verified path, identify
the selected retained table, then stop/reap that independent owner VM and require
its exact root/parent/table release after actual execution quiescence. Repeat
complete cycles and establish a bound on retained empty tables and virtual-range
consumption. Do not stop an unrelated persistent desktop VM or silently change
legacy SHM reclaim policy to get a passing count. This case has not been
implemented or executed here. It is needed to distinguish an acceptable VM
lifetime allocation policy from resource accumulation requiring repair.

For the original 4-CPU SHM observation, capture the additional frame at the
original checkpoint, its object/view owners, any pending retire/ACK obligations
and exact final release; lower totals at 999 alone remain insufficient. If
asynchronous work is implicated, use its real completion condition and a
separately labelled boundary. No cleanup delay or count threshold change is
justified.

Only after every original difference has a demonstrated cause, every additional
frame is owned or retired, all repeated-cycle 2/4/12 native checks and relevant
security/production regressions pass can M3.1 be COMPLETE. Native payload work
remains a separate future milestone.

## M3.1B — bounded page-table retention and VM teardown (2026-11-01)

Production FastLane remains disabled; this section adds one opt-in fixture and,
after the mechanism was demonstrated, one minimal reclaim in the SHM free path.
No payload transfer, userspace Doorbell, census-threshold change or cleanup
delay is introduced.

### Mechanism, now directly demonstrated

New opt-in kernel fixture `kernel/src/memory/m31_retention_gate.rs`
(feature `fastlane_m31_retention_test`, debug-only) drives the ordinary
`alloc_shared_region` / `free_shared_page` entry points against dedicated owner
VMs and walks each owner's own lower-half table tree, so every observation is a
per-frame identity, not an aggregate. It uses no sleeps or wall-clock waits:
quiescence is the existing synchronous leaf shootdown plus the table walk.

Phase A (controlled 2 MiB crossings) and Phase B (long-lived retention) show the
pre-repair behaviour on the same tree: each 2 MiB SHM span the monotonic cursor
crosses links exactly one fresh P1 table (`table-link`), and the ordinary
`free_shared_page` retires the leaves and backing but left that now-empty P1
table parent-linked. Nine crossings retained nine empty P1 tables plus the upper
tables; two same-span allocations shared exactly one P1 table, so the retained
table is reachable and reusable while the VM lives. This is **unbounded
address-cursor-driven retention along the 4 GiB SHM window (at most 2048 empty
P1 tables = 8 MiB of frames per live VM), not deferred reclamation and not an
unowned leak**: the tables stay reachable through the live root and are only
released by final VM teardown.

The pre-repair mechanism is reproducible on demand from the same tree with the
debug-only, default-off evidence feature `fastlane_m31_prerepair` (requires
`fastlane_m31_retention_test`; never present in production or ordinary images).
Its serial is retained under `target/fastlane-m31b-evidence/final/prerepair-native-*/`;
the `phase=setup ... pre_repair=1` record and the per-crossing retained-id lists
are the file-backed evidence for the accumulation.

Phase C (retention-arm teardown) builds four deliberately retained empty P1
tables through the ordinary VM map/unmap path, records their exact frame
identities, then tears the owner down through `reclaim_user_space(free_root)`.
Every recorded identity and the VM root is `Free` afterwards and
`reclaim_page_tables` equals the observed reachable tables plus the root exactly
once; the shared-memory and page-table classes and the free-frame count return
to their pre-fixture baselines (zero drift).

### Decision and minimal repair

The retention is bounded, but it provides no reuse benefit for the observed
SHM workload (each crossing needs a fresh P1 anyway) and accumulates across an
unbounded number of crossings up to the window bound. `mmap`/`munmap` already
reclaim empty lower tables after the covering shootdown; `free_shared_page` was
the only missing call. The minimal repair adds exactly that call per retired
leaf page, **after** `unmap_page` has cleared the leaf and completed its
synchronous remote invalidation, and only when the table is actually empty.
Shared tables still used by a surviving mapping are retained by the existing
`reclaim_empty_tables_for_page` check; ordering is unchanged; no lock is added
to the hot path beyond the one already held.

After the repair the M3.1B fixture shows zero retained empty P1 tables across
all eight crossings, the same shared-table reuse while two views are live, and
zero drift after both VM teardowns. Re-running the M3.1 census fixture on the
final traced image shows the page-table counter now **flat at 381** across
checkpoints 100-105 on 2/4/12 CPUs (previously 391->392), and
`tools/fastlane_census_assess.py` returns `RECONCILED_OBSERVATIONS` with zero
unresolved additions on all three topologies.

Final source-bound evidence (every `validation-summary.json` records
`source_sha256` equal to the frozen tree): `target/fastlane-m31b-evidence/sweep/`
(M3.1B build + native 2/4/12 and the pre-repair `prerepair-build`/
`prerepair-native` pair), `sweep-m12/` (M1/M2 + census regression 2/4/12),
`sweep-checks/` (host), `sweep-prod/` (production exclusion). Earlier
pre-repair mechanism serials and the M1 deadline witnesses are preserved under
`target/fastlane-m31b-evidence/native-*`, `m1-loop/`, and `m1-diag/`.

### Independent M1 regression (`channel_race.rs`)

The 2-CPU `channel race deadline` panic reproduced on the final source-bound
image (1 of 4 mapping runs). The observation-only
`[FL-RACE-DEADLINE]` witness records the live placement state: in every captured
failure `entered=1 cpus=(1,u64::MAX) mode=2 held=0 contended=0 finished=0`, i.e.
only one of the two actors reached the rendezvous and the other never entered,
so `wait()` spins to its 5 s deadline. Because `syscall_entry` runs with
interrupts disabled (`cli`) and the actor under the scheduler path holds the
run slot, the waiting actor has no yield point and cannot be replaced. This is a
**test-coordination / insufficient-progress failure in a debug-only fixture**,
not a production channel lifecycle race or deadlock: the authority operations
themselves complete normally in every run including the passing ones, and the
reference `[FL-AUTH]`/`[FL-OUTCOME]` records are well-formed. It is preserved
unchanged as an independent regression: the panic text (`channel race deadline`)
and the 5 s bound are untouched. The original `assert!` at
`channel_race.rs:43` was expanded in place into a bounded `if` that prints the
observation witness and then panics; the deadline line is now at
`channel_race.rs:58`. No bound was raised and no timeout was lengthened to make
the run pass.

### Status

| Deliverable | Result |
|---|---|
| Phase A controlled crossings | COMPLETE |
| Phase B long-lived retention measurement | COMPLETE |
| Phase C exact teardown | COMPLETE |
| Phase D cache-vs-leak decision + minimal repair | COMPLETE |
| Phase E M1 timeout cause | COMPLETE (coordination stall; not a lifecycle race) |
| Phase F 2/4/12 validation + production exclusion + host checks | PASS |
| Original M3.0 archive failures | UNRESOLVED (archive unavailable) |

The original M3.0 image/service/raw-log identities remain unavailable; the
mechanism now demonstrated is consistent with the original 2-CPU and 12-CPU
page-table differences, but those historical identities remain **UNRESOLVED**
under the strict rule. Native payload work remains a separate future milestone.
