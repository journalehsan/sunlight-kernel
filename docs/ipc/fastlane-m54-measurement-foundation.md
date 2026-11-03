# SunlightOS FastLane M5.4 — Native Measurement Foundation

**M5.4 PARTIAL. M6 comparative benchmarking: NOT READY. Production FastLane:
DISABLED.** The three measurement-foundation gaps (high-resolution monotonic
clock, CPU-time qualification, allocator/resource accounting) are closed and
demonstrated on the final source. The milestone remains PARTIAL because one
pre-existing regression assertion — the M3 mapping gate's *global* SHM/table
census-constancy check — failed twice at 4 CPUs, and its ownership is
**UNRESOLVED** (see [M5.4R reconciliation](fastlane-m54r-scoped-census-reconciliation.md)).
No comparative throughput, bandwidth or latency-percentile result was produced.
No commit or push was made.

Machine-readable results: `.fastlane-evidence/m54/qualification.json`,
`.fastlane-evidence/m54/suites/20261103T161908Z/matrix.json`, the retained
serials under `.fastlane-evidence/m54/native-20261103T161416.831281Z/`, and the
preservation/ownership evidence under `.fastlane-evidence/m54r/`.

## Source and image identity

| Identity | Value |
|---|---|
| HEAD | `e5ed4849e57cb729252ba7ad0c25efcfe7fe5ae3` |
| Baseline executable-input files | 1,378 |
| Baseline canonical map SHA-256 | `dd0bbe9ed3a97755ff436f8f3499e6e766077acdfc8c489fe814810d0585e61d` |
| Final executable-input files | 1,383 |
| Final canonical map SHA-256 | `18f58216553739767ccdaa95bc22d01e1eb6dcf9b8b50f7b00df0a21d0a3fddf` |
| Instrumented ELF SHA-256 | `8f6ac015115f5b80d52852a3b6f6f96004ccd2c6596a6eb94489d5e2ef1bc821` |
| Instrumented ISO SHA-256 | `fd75351fdca5c1c38ac613ae1ab9f2e151b8bbc5ac1ec167186e1d85bbbd3b0b` |
| Kernel features | `fastlane_measurement_test`, `key_inject` (debug-only) |
| User feature | `fastlane-measurement-test` (implies `fastlane-shared-ring-test`) |

Canonical encoding is `sha256(json.dumps(sources(), sort_keys=True).encode())`
using `fastlane_f11b0e_validate.sources()`. Relative to the clean baseline the
executable map adds 5 files (`ipc/src/measurement_clock.rs`,
`ipc/src/resource_ledger.rs`, `kernel/src/arch/x86_64/measurement_clock.rs`,
`tools/fastlane_m54_receipt.py`, `tools/test_fastlane_m54.py`) and modifies 12
existing executable inputs. The dirty tree also contains this report, the M5.4R
reconciliation, and one unrelated pre-existing untracked document; none are
executable inputs. `git diff --check` is clean.

## What changed

All new code is opt-in. The kernel module is compiled only under
`#[cfg(all(debug_assertions, feature = "fastlane_measurement_test"))]`; the
syscall arm and the fixture probes share that gate. No capability, permission,
mapping, hostile-memory or Doorbell-authentication logic was changed.

- `ipc/src/measurement_clock.rs` — integer-only clock qualification shared by
  kernel and Ring 3: `Calibration`, `Stamp`, `ClockError`, overflow-checked
  `to_ns`, `elapsed_ns` (rejects cross-CPU, backward, stale-epoch,
  uncalibrated), `calibration_ppm`, `fixed_point_mul`.
- `kernel/src/arch/x86_64/measurement_clock.rs` — per-CPU clock. BSP publishes a
  PIT-referenced frequency; each CPU publishes its own counter origin; a bounded
  re-measurement (6 windows, first discarded, median + observed spread) validates
  frequency stability and advances the epoch on scale change.
- `ipc/src/cpu_accounting.rs` — `Charge`, `ActorTime` (migration-safe,
  exactly-once, discontinuity and exhaustion reporting), `Reconcile`,
  `reconcile()`, `scheduling_gap()`.
- `ipc/src/resource_ledger.rs` — six separate dimensions (event counts, live
  objects, frames, mappings, page tables, retained/reclaimed bytes);
  `bookkeeping_balanced()` is explicitly weaker than physical reclamation.
- Fixture (`services/sunlight-uac/.../shared_ring/measurement.rs`) emits
  `[FL-M54-CLOCK]`, `[FL-M54-CLOCKSYNC]`, `[FL-M54-XCPU]`, `[FL-M54-REJECT]`,
  `[FL-M54-KCPU]`, `[FL-M54-RES]`.
- Validators: `tools/fastlane_m54_receipt.py` (classification) and
  `tools/test_fastlane_m54.py` (18 structural + retained-guest controls).

## Phase B — High-resolution monotonic clock

Source identity: `pit-calibrated-tsc`. QEMU/TCG does **not** advertise an
invariant TSC (CPUID.8000_0007H:EDX[8] = 0), so the frequency is not certified by
CPUID; instead each CPU re-measures its counter against the PIT and the record is
only usable when the observed spread plus two boot-calibration bounds stays under
the 20,000 ppm ceiling. Conversion is fixed-point `(dt * mul) >> 32`, overflow
rejected beyond the calibrated horizon, and the unit is nanoseconds.

Contract: **same-CPU subtraction only**. Each stamp carries the CPU that read it;
`elapsed_ns` returns `CrossCpu` for mismatched CPUs and the fixture never
subtracts across CPUs. Per-CPU origins differ by ~180–350 ms under TCG, so a
cross-CPU duration is meaningless without a validated offset, which this kernel
does not establish.

| Topology | Boot TSC Hz | Boot ppm | Stability spread ppm | Effective bound ppm | Probes resident / 16 | Probe regressions | Max probe uncertainty ppm | Cross-CPU pairs / rejected |
|---|---|---|---|---|---|---|---|---|
| 2 CPU | 2,486,467,793 | 251 | 34, 103 | 537, 606 | 10 | 0 | 606 | 8 / 8 |
| 4 CPU | 2,492,040,132 | 251 | 144, 85 | 647, 588 | 9 | 0 | 647 | 6 / 6 |
| 12 CPU | 2,479,965,894 | 250 | 234, 172 | 735, 673 | 8 | 0 | 735 | 3 / 3 |

`not-resident` probes are reported as unqualified, not coerced: the actor was on
another CPU, so no duration is claimed. The deterministic rejection self-check
(`[FL-M54-REJECT]`) exercises backward, uncalibrated, overflow, cross-CPU,
stale-epoch and time-backward cases on the exact shared code and requires all to
be refused.

## Phase C — CPU-time qualification

`[FL-M54-KCPU]` reports, per measured actor window: scheduler-charged runtime
(`charged_ns`, the existing per-process accumulator delta), the qualified
same-CPU wall window (`wall_ns`), migration (`cpu_start`/`cpu_stop`), switch-in
count, and a reconciliation verdict. The accounting boundary is the scheduler
charge interval; migration is close+open with no separate cross-CPU accumulator,
so the straddling interval is charged exactly once. `ActorTime` in
`sunlight-ipc` is the host-checked model of that contract (migration,
duplicate-open, backward clock, exhaustion, reconciliation).

| Topology | Records | charged valid | wall valid | migrated | discontinuities | reconcile=consistent | max charged ns | max wall ns |
|---|---|---|---|---|---|---|---|---|
| 2 CPU | 8 | 8 | 8 | 0 | 0 | 8 | 193,018,623 | 597,777,100 |
| 4 CPU | 8 | 8 | 8 | 0 | 0 | 8 | 244,129,960 | 745,079,504 |
| 12 CPU | 8 | 8 | 8 | 0 | 0 | 8 | 211,852,339 | 635,305,939 |

Unsupported distinctions are reported, not inferred: blocked/waiting time,
user-vs-kernel split and CPU utilization are NOT_QUALIFIED. Windowed migration
of a *measured* actor was not forced natively (all windows `migrated=0`); the
migration-safety of the accounting is established by the host `ActorTime` model
and the scheduler's migration-safe accumulator, not by a native forced migration.

## Phase D — Allocator and resource accounting

`[FL-M54-RES]` reports a bounded, *scoped global delta* over the whole system
(`sysinfo` total/used/swap), with `attribution=scoped-global-delta` and
`physical_reclaim=not-proven`. Interference is stated explicitly
(`concurrent-services,ramfs,desktop,dma`). Across all topologies `used_kb`
stayed at 518,800 and `swap_used_kb` at 0; `total_kb` varied only with RAM
topology (1,544,392 / 1,542,344 / 1,534,152).

Exact per-actor attribution is infeasible: the allocator exposes only aggregate
figures. The `resource_ledger` type keeps the dimensions separate so a balanced
ledger is never read as physical reclamation. Frame/mapping/retirement evidence
is taken from the same guest's M2/M5.2 records: exactly-once physical release,
remote TLB ACK set and ordering, and zero scoped restricted inventories
(`objects/views/frames = 0`). A zero bookkeeping delta does **not** prove
physical memory was reclaimed.

## Phase E — Instrumentation validation

The opt-in fixture demonstrates: known-duration local probes with bounded
expectations; cross-CPU qualification (subtraction rejected); block/wake and
cancellation via the existing M5.2/M5.3 cases; controlled allocation/free and
repeated channel creation/retirement with zero final scoped inventories;
deterministic rejection of invalid/incomplete measurements; and an
instrumentation-on vs instrumentation-off count comparison (below). Every
result is classified; a passing structural test never promotes an unqualified
metric.

## Phase F — Native regression matrix

Each row is a distinct build + native run on 2, 4 and 12 CPUs.

| Gate | 2 CPU | 4 CPU | 12 CPU |
|---|---|---|---|
| M5.4 measurement (M5.2/M5.3 + clock/CPU-time/resource) | PASS | PASS | PASS |
| M5.2 non-instrumented control | PASS | PASS | PASS |
| M5.1 repeated mediated transfer | PASS | PASS | PASS |
| M5 mediated transfer | PASS | PASS | PASS |
| M4 authenticated Doorbell | PASS | PASS | PASS |
| M1 authority and bounded coordination | PASS | PASS | PASS |
| M3.1B page-table retention/reclamation | PASS | PASS | PASS |
| M1/M2/M3 mapping + hostile-word (`assess_words`) | PASS | PASS (hostile-word) | PASS |
| M3 mapping global census-constancy | PASS | **2 FAIL / 1 PASS** | PASS |

Host gates all pass: kernel/ring/Doorbell/IPC/UAC/scheduler tests,
dispatch-boundary, M5.2 parser controls, mapping parser controls, M3 ring
clippy/rustfmt/ASan/TSan, `git diff --check`. Miri is **UNAVAILABLE / NOT_RUN**
(`cargo-miri` not installed).

**Preserved failure — 4-CPU mapping census.**
`suites/20261103T161908Z/mapping/native-20261103T163851.028220Z` and
`mapping-rerun/native-20261103T165420.723275Z` FAIL; a later replay
(`mapping-rerun2/native-20261103T171135.821855Z`) PASSes. The failing assertion is
only the *global* `shm`/`tables` constancy across checkpoints 100–105. In the
original receipts checkpoint 100 showed `shm=1305 free=256755` while all others
showed `shm=1304 free=256756`; a transient global SHM increase and concurrent
`session-racer` teardown are **observations**, not ownership proof. Scoped
`objects/views/frames` were 0 at every checkpoint and `assess_words` passed on the
same serial. The later passing replay does not explain either failure and cannot
reclassify it.

M5.4R preserved the artifacts and ran two traced 4-CPU replays
(`ownership/`, `ownership-repeat/`) that PASS with `shm=1304` stable and no
SHM frame/object/owner set difference across checkpoints 100–105 — but they did
not reproduce the original interleaving, so **frame ownership remains
UNRESOLVED**. No tolerance or exception was added to the assertion. See
[M5.4R reconciliation](fastlane-m54r-scoped-census-reconciliation.md).

## Observer effect (counts only)

Final whole-system `[FL-M52-KDIAG]` on 2 CPUs:

| Guest | syscalls | context switches |
|---|---|---|
| M5.2 control (no measurement) | 234,381 | 40,636 |
| M5.3 measurement (no M5.4 probes) | 216,553 | 37,109 |
| M5.4 measurement | 239,910 | 39,729 |

The M5.4 probes add ~23.4k syscall-167 reads and ~2.6k context switches over the
M5.3 instrumented guest (16 local probes × 257 reads, plus bounded residency
yields and cross-CPU pinning). These are instruction/syscall counts, not a
timing comparison; no zero-overhead claim is made.

## Failed, aborted and accepted attempts

1. **Truncated probe line (found and fixed).** The first probe format exceeded
   the kernel `DebugLog` 256-byte ABI and the kernel silently truncated the
   `contract=` suffix. Fixed by compacting the probe line (203-byte worst case);
   a structural control now rejects a probe without `contract=`.
2. **4-CPU mapping census (preserved, ownership UNRESOLVED).** Two FAIL, one
   PASS; original receipts retained; two traced M5.4R replays PASS but do not
   reproduce the interleaving.
3. **M3-native orchestration error (preserved).** The first matrix run invoked
   `fastlane_m3_validate.py --section native` without `--image-manifest`; this is
   an orchestration error, not a source failure. The mapping native gates were
   then run directly.

## Production exclusion and security audit

- A release kernel built without the measurement feature contains **0**
  `FL-M54` markers, **0** `FL-M53` markers and **no** `sys_measurement_stamp`
  symbol. Production FastLane remains **DISABLED**.
- Syscall 167 is debug-only and feature-gated; no capability check, mapping
  permission, hostile-memory path or Doorbell authentication was weakened.
- M3.2 remains: memory safety PROVIDED; stable private snapshots PROVIDED;
  coherent publication **NOT PROVIDED**; application authenticity **NOT
  PROVIDED**.
- Historical M1 timeout attribution, M4 failures and SHM census drift remain
  **UNRESOLVED**.

## Qualification inventory

| Metric | Classification | Limit |
|---|---|---|
| Same-CPU monotonic duration (PIT-calibrated TSC) | QUALIFIED_WITH_BOUNDS | same-CPU only; frequency qualified by re-measured PIT stability |
| Cross-CPU timestamp subtraction / cross-CPU latency | NOT_QUALIFIED | no validated cross-CPU offset; refused by shared code |
| Scheduler-charged execution time reconciled to same-CPU wall | QUALIFIED_WITH_BOUNDS | windowed migration not forced natively |
| Wall-clock elapsed (same CPU) | QUALIFIED_WITH_BOUNDS | same-CPU only |
| Blocked/waiting time; user-vs-kernel split; utilization | NOT_QUALIFIED | no trustworthy transition attribution |
| Scheduler dispatch / switch-in count | QUALIFIED_WITH_BOUNDS | count only, not a duration |
| Scoped global memory delta | QUALIFIED_WITH_BOUNDS | concurrent services interfere; not per-actor |
| Physical retirement / exactly-once / TLB ACK ordering | QUALIFIED_WITH_BOUNDS | fixture-scoped; zero bookkeeping ≠ physical reclaim |
| Per-channel heap events, peaks, retained/reclaimed bytes | NOT_QUALIFIED | allocator exposes only aggregate figures |
| Global SHM/table census constancy under concurrency | NOT_QUALIFIED | ownership UNRESOLVED; no quiescence fence |

## Readiness decision

**Safe to use in M6 (with the stated bounds):** same-CPU monotonic durations;
scheduler-charged execution time reconciled to same-CPU wall; dispatch/switch-in
counts; scoped global memory delta; fixture-scoped physical retirement and
exactly-once/TLB-ACK ordering.

**Disqualified for M6:** any cross-CPU latency or timestamp subtraction; blocked
or waiting time; user/kernel split and CPU utilization; per-channel heap
events/peaks/retained/reclaimed; global SHM/table census under concurrent
services; any percentile or rate spanning CPUs or derived from wall time alone.

M6 comparative benchmarking therefore remains **NOT READY** as a milestone: it
still needs a common simultaneously-released transfer gate and a representative
sampling policy (per M5.3), and cross-CPU latency and per-channel allocator
accounting are not qualified.

**Smallest executable next milestone:** a source-bound ownership replay that
carries an exact SHM frame/object/owner witness (object ID, owner PID/incarnation/
VM), a frame-by-frame difference of SHM_LIVE and PMM LIVE at checkpoints 100–105,
and correlated VIEW_ADD/unmap/retirement — as specified in the M5.4R
reconciliation. Overflow, incomplete trace, missing owner or missing release must
keep attribution UNRESOLVED. No measurement-foundation work remains.

## Reproduction

```
python3 tools/fastlane_m52_validate.py --section build --measurement --evidence-root .fastlane-evidence/m54
python3 tools/fastlane_m52_validate.py --section native --measurement --cpus 2 4 12 \
    --image-manifest <build>/validation-summary.json --evidence-root .fastlane-evidence/m54
M54_SERIAL=<native>/<N>cpu.serial.log M54_CPUS=<N> python3 -m unittest discover -s tools -p test_fastlane_m54.py
python3 .fastlane-evidence/m54/orchestration/m54-suites.py
python3 .fastlane-evidence/m54/orchestration/m54-summary.py
```


## M5.4R2 handoff update

The scoped census contract and source-bound evidence are in
[the M5.4R2 report](fastlane-m54r2-scoped-census-contract.md). Measurement
foundation is qualified with explicit same-CPU limits; the primary final-source
2/4/12 matrix, adversarial controls, ASan/TSan, host checks and production
exclusion passed. Overall R2 remains PARTIAL pending independent safety closure.
Original M5.4 FAIL classifications and historical M1/M4/SHM causal uncertainties
remain unchanged. No M6 benchmark was run; production FastLane remains disabled.
