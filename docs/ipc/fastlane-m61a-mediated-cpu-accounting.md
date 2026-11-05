# FastLane M6.1A mediated actor CPU accounting

**M6.1A COMPLETE.** Mediated same-CPU scheduler-charged execution and matched actor windows: **QUALIFIED_WITH_BOUNDS**. Migration-safe execution remains **NOT_QUALIFIED**.

This is an accounting qualification for one debug-only, instrumented request/acknowledgement workload. It is not a new transport comparison. Production FastLane remains **DISABLED**. No adaptive routing, performance optimization, scheduler policy/tick change, new hot-path lock, commit or push is included.

## Established failure mechanism

The M6.0 fixture started/stopped actor accounting only for `path == 2`. Its mediated (`path == 1`) actor windows therefore never existed. M6.0R explicitly attempted operations 26 and 27 for both mediated actors. Both returned `Incomplete=7`: `Scheduler::fastlane_measurement` first authenticates the current process against the channel's creator/consumer identity, then requires `row.native_ring[role]`. A mediated channel uses the existing authenticated repeated-word backend and has no activated native-ring role. The selector rejects it before setting `measurement.enabled` or recording a runtime baseline.

Classification: **fixture instrumentation plus accounting API selector coverage**. Missing windows are the consequence. Incorrect actor identity, corrupt scheduler charging and migration are not demonstrated causes of the original omission. Missing execution time and switch counts were unavailable, never zero. Historical receipts cannot reconstruct these windows.

The original M5.4 selector (26/27) remains byte-identical. M6.1A adds separate debug-only 28/29 operations under `fastlane_mediated_accounting_test`, using the existing `ActorCounters`, scheduler runtime accumulator, qualified clock and `cpu_accounting::reconcile`. Operation 30 runs bounded local rejection controls on temporary state. These operations provide no scheduling, mapping, transfer or capability authority.

## Source identity and preservation

Current HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6` (unchanged). Final canonical source: `b7d57b0045444e77a492fcf3a80be7207a1d418aeb6eb1bbbdf571547a660882` (1400 executable inputs). Experimental ELF: `927227ca2272f3ea018390a743489137545377ecebe5d16c244e3bf69a45f3c5`. Experimental ISO: `6e2e730ab72da1cf8b420417e28434324a5001965a1841a718b22a2118c622c0`.

`initial-source.json`, `initial.patch` and `initial-source/` preserve the inherited tree, including pre-existing version/manifest changes. `inherited-source-delta.json` distinguishes that tree from historical M6.0R. `final-source.json`, the build/run manifests and `m61a-only.patch` identify this experiment. Canonical identity is SHA-256 of `sources()` encoded as sorted JSON with `separators=(',', ':')`. Each native manifest binds sources before/after, archived inputs, embedded service hashes, ELF/ISO hashes, raw serial hash and the exact command.

`m60r-preservation-before.json` records every pre-existing M6.0R artifact. Final preservation verification checks every recorded hash. The ten accepted historical M6.0R assessments were also replayed through the source-compatible retirement-parser extension and are identical. The accepted 2/4/12-CPU throughput medians remain exactly **11,777.0 / 11,184.1 / 9,789.1 inline**, **1,015.8 / 910.1 / 869.8 mediated**, and **7,608.9 / 6,917.0 / 6,848.7 shared ring** acknowledged requests/s. They are neither rerun nor pooled with these accounting observations.

## Before/after accounting contract

| Property | M6.0/M6.0R mediated | M6.1A focused fixture |
|---|---|---|
| Admission | Old selector requires native-ring activation | Exact live attached channel, mapped role, VM exclusivity, mediated repeated mode; no native-ring activation |
| Identity | Channel identity already kernel-authenticated | Current process PID, incarnation and VM generation, matched against immutable channel role and retained in the window |
| Window | Omitted, then explicit refusal | One bounded window per process; exact token, channel generation, authenticated actor and nonzero monotonically increasing window ID |
| Matching | No mediated counters existed | Duplicate start/stop, stale ID/generation and foreign identity rejected without consuming a valid open window |
| Charge | Unavailable | Delta of the existing scheduler accumulator; no second execution accumulator |
| Validity | Not qualified | Exhaustion, backward charge, missing clock, changed epoch/scale, invalid timestamp and migration cannot qualify |
| Exit | No window to retire | Open observation explicitly aborted once; no success/zero-time substitute |
| Migration | No mediated accounting evidence | Every switch-in records placement; away-and-back is sticky; actual CPU 0→1 control is rejected |
| Production | Disabled | New state, hooks and selectors require debug assertions and the opt-in feature |

Payload PID/UID bytes are never identity authority. The kernel derives `RecipientIdentity::of(current_process())`; the handle is a selector, not a bearer grant. `Window` holds bounded matching metadata only. `ActorCounters` continues to hold the original counters and scheduler-runtime baseline. Stop records include PID/incarnation/VM, channel generation/window ID, both CPU placements, raw timestamp/calibration records, raw scheduler accumulators, charged delta, switch-ins, overflow/discontinuity flags and reconciliation.

Observation-only hooks check the existing charge arithmetic for regression/exhaustion, record placement at dispatch, and abort on normal exit/termination. Removing those gated hooks yields the inherited scheduler source modulo whitespace; charge arithmetic, scheduling decisions, tick frequency and lock structure are unchanged. In particular, no scheduler defect was repaired or inferred.

## Focused native fixture and controls

One producer and consumer in distinct native VMs use the existing Account-authorized channel and authenticated IPC coordination. Both positive windows run on CPU 0. After exactly 16 warm-up request/acknowledgement operations, the consumer starts window 1, then the producer starts window 1. Exactly 128 measured operations follow, one outstanding request at a time. All 32 application bytes follow the retained deterministic sequence formula; the consumer privately compares every byte, validates channel/sequence/slot generation, releases once, and acknowledges the exact digest and count. The producer checks every acknowledgement.

The producer stops first, then the consumer. Each window is reconciled independently, and the host independently requires the sum of both charges to fit within their common CPU-0 covering interval. Windows include bounded control calls, rejection probes and existing serial instrumentation. They do not isolate pure application or transport CPU cost.

| Control | Execution and expected outcome |
|---|---|
| Missing start | Actual selector stop before start: `Incomplete=7` |
| Duplicate start / stop | Actual selector calls: `Duplicate=3`; original window preserved |
| Wrong window / reused ID | Actual calls: `Stale=2`; original active observation preserved |
| Wrong actor | Third native process has the exact handle but no role: both start and stop `Denied=1` |
| Stale channel generation | Actual start with changed generation: `Stale=2` |
| Stale incarnation / VM | Kernel-local controls and host tests call the exact matching code with altered identity: rejected; no native PID-reuse injection claimed |
| Counter exhaustion / backward charge | Kernel-local temporary counters and host accounting/window tests; overflow/discontinuity invalidates qualification |
| Invalid same-CPU timestamps | Exact shared clock helper rejects backward pair; independent parser rejects backward/zero, stale epoch, changed calibration and excess uncertainty |
| Migration | Actual window 2 moves consumer from CPU 0 to CPU 1: no wall duration, `valid=0`, `reconcile=not-qualified` |
| Incomplete actor exit | Consumer opens window 3, exits without stop; one authenticated abort, no completed window |
| Away-and-back placement | Host test of the actual sticky placement state; native return-within-one-window qualification is not claimed |

Current-source host checks pass: kernel **159**, IPC **81**, ring **39 + 5 doc tests**, Doorbell **9**, UAC **18**, dispatch-boundary **29**, focused scheduler accounting/state/lifecycle/policy tests, plus retained M5.2/mapping parser controls. New parser controls: **33/33 PASS** (3 basic clock/empty controls and **30/30 rejected receipt mutation models**). New Rust files pass rustfmt; `git diff --check` passes. Mutation logs are explicitly host receipt models, not injected native faults.

Native kernel-local controls are explicitly distinguished from actual actor/syscall controls and host receipt mutation models. No unsafe early-free fault or forced native accumulator saturation is introduced. The independent parser checks exact record populations, authenticated binding/incarnation/VM, matched boundaries, scheduler deltas, clocks, positive and negative verdicts, acknowledgements, cleanup and retirement; a guest `consistent` string alone is insufficient.

## Native matrix and accounting results

| CPUs | Rep | Charged P / C ms | Wall P / C ms | Switch-ins P / C | Migration control | Reconciliation / retirement |
|---|---|---|---|---|---|---|
| 2 | 1 | 421.828 / 426.990 | 1003.691 / 1012.672 | 192 / 175 | 0→1 rejected | consistent / PASS |
| 2 | 2 | 58.208 / 56.711 | 119.523 / 121.390 | 134 / 136 | 0→1 rejected | consistent / PASS |
| 2 | 3 | 61.643 / 60.605 | 127.221 / 129.066 | 137 / 136 | 0→1 rejected | consistent / PASS |
| 4 | 1 | 61.883 / 59.665 | 123.091 / 124.708 | 128 / 129 | 0→1 rejected | consistent / PASS |
| 4 | 2 | 65.130 / 63.161 | 128.911 / 130.805 | 130 / 133 | 0→1 rejected | consistent / PASS |
| 4 | 3 | 75.703 / 70.501 | 147.897 / 149.732 | 130 / 129 | 0→1 rejected | consistent / PASS |
| 12 | 1 | 74.427 / 74.709 | 150.671 / 152.994 | 128 / 129 | 0→1 rejected | consistent / PASS |
| 12 | 2 | 69.228 / 65.149 | 136.068 / 138.287 | 128 / 129 | 0→1 rejected | consistent / PASS |
| 12 | 3 | 78.526 / 74.249 | 152.019 / 154.053 | 128 / 129 | 0→1 rejected | consistent / PASS |

Accepted repetitions: **{'2': 3, '4': 3, '12': 3}**. Final matrix: **1152 measured requests**, **1296 including warm-up**, **18 qualified CPU-0 actor windows**, **9 migrated windows rejected**, **9 incomplete exit windows aborted**, and **0 rejected positive windows**. Each accepted guest also requires the deterministic selector rejection controls. Final scoped inventories are zero; physical frees total **27**, with **18** current-source remote retirement requests. The accepted pilot is additional and is not pooled.

All final accepted guests use one exact source/image group. Preliminary acceptances, failed attempts and the pilot are separate populations. No throughput or CPU-utilization figure is derived from this table. Both positive actors stay on CPU 0; the topology matrix does not measure parallel application scaling.

## Clock qualification, uncertainty and observer cost

Clock stamps preserve the existing seven fields: CPU, calibration epoch, counter-derived nanoseconds, TSC Hz, uncertainty ppm, flags and observed stability spread. Kernel records additionally preserve the unmodified scheduler accumulator at each endpoint. The PIT reference, six calibration windows (first discarded), five-window retained median/spread and same-CPU clock rules are inherited from M5.4. Host monotonic time and guest coarse milliseconds only enforce execution deadlines; neither substitutes for a rejected duration.

The wall parser independently requires matching nonzero epochs, CPU 0 endpoints, unchanged frequency/flags/spread/ppm, PIT/stability flags, positive forward duration and at most 20,000 ppm. Cross-CPU timestamp subtraction is refused. This is observed QEMU/TCG clock qualification, not an invariant-TSC or bare-metal guarantee. Original raw TSC ticks are not separately exported by the inherited seven-word API.

Wall scale uncertainty is `ceil(wall_ns * ppm / 1e6) + 1 ns`. Scheduler execution counters retain their original **boot PIT scale**, whereas wall stamps use the revalidated scale. `results-summary.json` therefore retains both frequencies and reports their observed scale difference, together with a separate charge-scale allowance `ceil(charged * (abs(window_hz - boot_hz) / window_hz + (window_ppm + boot_ppm) / 1e6))`. This is a conditional scale allowance, not a total accuracy guarantee: it does not bound PIT systematic error, per-checkpoint rounding, stamp/syscall boundary skew, TCG/host scheduling noise, pauses or unobserved within-window frequency variation. The raw scheduler charge is not silently rescaled or replaced by wall time. Reconciliation compares the recorded values conservatively and does not infer blocked time from slack.

Across accepted matrix actor windows: maximum wall uncertainty **777 ppm**; largest per-window wall scale allowance **778747 ns**; observed boot/window scale difference **2662–17712 ppm**. Per-actor charge scale allowances and all original frequencies are retained in `results-summary.json`. No positive-window counter discontinuity or overflow was observed.

Instrumentation overhead in nanoseconds is **NOT_QUALIFIED**: there is no independently matched instrumentation-off control. Boundary syscalls, identity checks, duplicate/stale probes and existing mediated trace output contribute to the observed actor windows. These execution times must not be presented as intrinsic transport cost or substituted into the M6.0R comparison.

## Retirement, safety reuse and production exclusion

Every accepted guest requires three unique backing lifetimes, two role views for each, one physical free per backing, and complete request → exact remote-ACK set → TLB completion → view-unmap completion → physical-free ordering. Final restricted objects/views/frames are all zero. Each of the three fixture processes has exactly one authenticated IPC cleanup receipt: owned endpoints 0, no pending call/reply target/deferred target/commit, and exactly one reap with `ipc_cleared=0`. The deliberately incomplete accounting observation is aborted separately from physical channel retirement. Remote targets depend on actual active address spaces: six final guests have three remote requests each; three guests have empty remote target sets. The host requires exact ACK sets for every guest and a current-source positive remote witness at each topology. The first finalization postcheck accidentally required positive remote targets in every guest; `postcheck-remote-scope.json` and the original PARTIAL audit retain that extra-check failure and its contract correction. No native or parser result was changed.

`source-safety-audit.json` verifies unchanged IPC, ring, Doorbell, mapping and word-transfer implementations against M6.0R, byte-identical old selectors, and observation-only scheduler changes. The retained M5.4R2 primary/safety predicates, serial identities, stale/foreign/duplicate/adversarial controls and positive remote retirement witness are replayed read-only into a new receipt. They cover unchanged implementations; they do not supply the new mediated accounting proof. Current host tests and the focused native matrix cover the new binding/window path. Historical unresolved M1/M4 attribution and original untraced SHM-census ownership stay **UNRESOLVED**. No full historical native matrix is rerun.

Current default-feature production build passes exclusion: **zero** M6.1A/M6.0/M5.4/M5.3/mapping/transfer measurement markers, and no `sys_measurement_stamp`, mediated selector, `Window` or `ActorCounters` symbol. Production ELF: `c4c88e28234a06c9d90622aca78cdbf4c7afdd33a433335808bfdd0f2977d633`; ISO: `f71a57baffd71c10ed7c5fcb311484b453b9548f12fc75ed32cec57476c1247a`. The production manifest binds the same final source map. `23989` historical M6.0R artifact hashes and its report hash remain unchanged.

Default features do not enable the fixture. The kernel module, syscall branches, process measurement layout and observation hooks require `all(debug_assertions, feature = ...)`; the user fixture has a separate default-off feature. No new unsafe block, authority relaxation, global lock or privileged general-purpose measurement API is added. Production FastLane remains disabled.

## Failed, superseded and aborted attempts

| Source group | CPUs | Status | Attempt manifest |
|---|---|---|---|
| matrix | 2 | PASS | `.fastlane-evidence/m61a/matrix/attempt-20261103T222431.054407Z/native-20261103T222431.096023Z/validation-summary.json` |
| matrix | 4 | PASS | `.fastlane-evidence/m61a/matrix/attempt-20261103T222506.177630Z/native-20261103T222506.215762Z/validation-summary.json` |
| matrix | 12 | FAIL | `.fastlane-evidence/m61a/matrix/attempt-20261103T222544.694646Z/native-20261103T222544.732956Z/validation-summary.json` |
| matrix | 12 | FAIL | `.fastlane-evidence/m61a/matrix/attempt-20261103T222705.847303Z/native-20261103T222705.887137Z/validation-summary.json` |
| matrix-final | 2 | PASS | `.fastlane-evidence/m61a/matrix-final/attempt-20261103T223052.692170Z/native-20261103T223052.734920Z/validation-summary.json` |
| matrix-final | 4 | PASS | `.fastlane-evidence/m61a/matrix-final/attempt-20261103T223129.154827Z/native-20261103T223129.190938Z/validation-summary.json` |
| matrix-final | 12 | FAIL | `.fastlane-evidence/m61a/matrix-final/attempt-20261103T223204.325401Z/native-20261103T223204.363847Z/validation-summary.json` |
| matrix-qualified | 2 | PASS | `.fastlane-evidence/m61a/matrix-qualified/attempt-20261103T223844.759836Z/native-20261103T223844.799800Z/validation-summary.json` |
| matrix-qualified | 4 | PASS | `.fastlane-evidence/m61a/matrix-qualified/attempt-20261103T223958.852329Z/native-20261103T223958.972655Z/validation-summary.json` |
| matrix-qualified | 2 | PASS | `.fastlane-evidence/m61a/matrix-qualified/attempt-20261103T224114.932035Z/native-20261103T224114.969552Z/validation-summary.json` |
| matrix-qualified | 4 | PASS | `.fastlane-evidence/m61a/matrix-qualified/attempt-20261103T224156.829971Z/native-20261103T224156.866402Z/validation-summary.json` |
| matrix-qualified | 12 | PASS | `.fastlane-evidence/m61a/matrix-qualified/attempt-20261103T224238.693426Z/native-20261103T224238.730486Z/validation-summary.json` |
| matrix-qualified | 2 | PASS | `.fastlane-evidence/m61a/matrix-qualified/attempt-20261103T224315.146136Z/native-20261103T224315.190029Z/validation-summary.json` |
| matrix-qualified | 4 | PASS | `.fastlane-evidence/m61a/matrix-qualified/attempt-20261103T224350.183290Z/native-20261103T224350.223130Z/validation-summary.json` |
| matrix-qualified | 12 | PASS | `.fastlane-evidence/m61a/matrix-qualified/attempt-20261103T224426.560371Z/native-20261103T224426.601087Z/validation-summary.json` |
| matrix-qualified | 12 | PASS | `.fastlane-evidence/m61a/matrix-qualified/attempt-first12/native-20261103T223757.953114Z/validation-summary.json` |
| pilot | 2 | FAIL | `.fastlane-evidence/m61a/pilot/native-20261103T222228.657111Z/validation-summary.json` |
| pilot-authorized | 2 | PASS | `.fastlane-evidence/m61a/pilot-authorized/native-20261103T222318.595466Z/validation-summary.json` |
| pilot-qualified | 2 | PASS | `.fastlane-evidence/m61a/pilot-qualified/native-20261103T223701.858681Z/validation-summary.json` |
| pilot-residency | 2 | PASS | `.fastlane-evidence/m61a/pilot-residency/native-20261103T222919.749926Z/validation-summary.json` |

The first sandbox pilot could not bind its local QMP socket and never booted. It remains FAIL. Build-only superseded images (including the pre-pilot predicate/revoke review and the unused return-migration handshake variant) retain original PASS build statuses and are not native acceptances. No interrupted attempt was deleted. All native attempts, raw failure receipts and source groups are listed above and in `attempt-inventory.json`.

Three earlier 12-CPU guests remain **FAIL**, even though both positive windows reconciled in each. Two exhausted the original 4,096-yield placement observation; the third exhausted the extended one-second/131,072-yield bound while its last stamp still showed CPU 0. All failed the required migration control and cannot count as accepted guests. These receipts establish failed placement observation, not its underlying scheduler cause and not a scheduler accounting defect.

The final fixture uses an authenticated IPC block/wake boundary after requesting CPU-1 affinity. The supervisor, itself on CPU 0, receives the actor's request and verifies its kernel-authenticated badge and channel generation before replying. This provides an explicit deschedule/wake sequence rather than relying on a tight yield loop as proof of completed migration. A CPU-1 stamp is still mandatory before stop; there is no synthetic migration flag or fallback timestamp. The consumer stays on CPU 1 for the incomplete-window exit, adding a current-source remote-TLB retirement witness. Residency remains capped by both 131,072 attempts and 1,000 coarse monotonic milliseconds. The deadline is solely an execution bound. A fresh pilot and full focused source group gate this correction; earlier acceptances are not pooled. The successful handshake does not retroactively explain or reclassify the failed yield-loop attempts.

## Remaining limits and M6.1B recommendation

Same-CPU instrumented scheduler-charged execution and dispatch counts are qualified only for this bounded fixture and the stated clock/observation limits. **Migration-safe execution duration remains NOT_QUALIFIED**, even though native migration rejection now works. Cross-CPU elapsed time/latency, CPU utilization, blocked time, user/kernel split, uninstrumented transport CPU cost, allocator peaks, physical copy counts and general workload prediction remain unqualified. Host-only stale-incarnation and away-and-back controls do not become native lifecycle proofs.

M6.1B may proceed as a separate debug-only adaptive-routing **prototype**, gated by explicit actor/channel binding and valid same-CPU observations. First implement deterministic replayable policy decisions with fixed rules and fallback when accounting is invalid; require a separate routing-correctness/safety experiment before any optimization claim. Do not use utilization, blocked-time estimates or migrated-window duration as inputs. Keep production disabled. This milestone implements no routing policy.

## Reproduction and evidence entry points

Durable root: `.fastlane-evidence/m61a/`. Start with `final-verification.json`, `results-summary.json`, `final-source.json`, `source-safety-audit.json`, `production-audit.json`, `pilot-qualified-decision.json`, the `matrix-qualified-driver-*.json` receipts and `attempt-inventory.json`. Each attempt retains source snapshots, bounded raw serial, assessments, command and artifact index. Receipt mutation models are under `receipt-controls-qualified/`; their execution is `checks/parser-native-qualified.log`.

```sh
python3 tools/fastlane_m61a_validate.py --section build --measurement --evidence-root <new-evidence-root>
python3 tools/fastlane_m61a_validate.py --section native --measurement --cpus 2 \
  --image-manifest <build>/validation-summary.json --evidence-root <new-pilot-root>
M61A_SERIAL=<pilot>/2cpu.serial.log M61A_CPUS=2 \
  python3 -m unittest discover -s tools -p test_fastlane_m61a.py
# Only after pilot, parser, authentication and retirement pass:
python3 .fastlane-evidence/m61a/run_matrix_final.py
python3 tools/fastlane_m61a_validate.py --section checks --measurement --evidence-root <new-check-root>
```
