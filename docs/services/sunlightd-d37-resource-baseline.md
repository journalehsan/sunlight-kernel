# SunlightD D3.7: Post-Boot Resource Baseline and Residency Audit

**Status:** source/runtime audit and bounded resident-boot measurements completed; no production D1/D2 unload pilot. D3.6R RecoveryBlocked behavior is unchanged.

## Scope and evidence rules

This report describes the repository and runtime as observed on 2026-10-19. The machine-readable process map, service overlay, measurements, and raw serial logs are under `target/d37-evidence/`. The source ownership audit was run with `tools/audit_d35_ownership.py --check-owners`; it reconciled 55 services, 159 embedded artifacts, and 150 Cargo targets to the D3.5 snapshot. Counts are source inventory counts, not live process counts.

The boot measurements use QEMU TCG, a nominal 1024 MiB guest, no disk, default resident profile, no authenticated graphical session, and 1/2/4 vCPUs. Telemetry samples begin around the SunlightD-ready marker; CPU windows are nominally five seconds, with a 10 ms counter clock resolution. Each configuration is one baseline and one final observation, so host scheduling variability is material. No desktop-idle or application workload was measured. Historical D2 synthetic reclamation numbers are not used as production savings.

## A. Boot ownership and actual process map

The source inventory reports 3 direct kernel defaults, 16 direct Init defaults, and 20 SunlightD unit definitions (16 enabled by default). Runtime inspection observed 31 live tasks at the two-vCPU checkpoint. The inventory overlay lists each observed process, its observed launcher/lifecycle owner, state, and mapped user pages in `target/d37-evidence/service-inventory.json`.

* Kernel creates `init`, `vfs_server`, and `tty_server`.
* Init directly starts its configured services. `sunlight-swapd` is a bounded one-shot and exits/reaps; other configured services were observed live at the checkpoint.
* SunlightD started 13 units: `timezone_service`, `timed`, `niced`, `gcd`, `uac_service`, `sunlight-sessiond`, `sunlight-sm`, `sunlight-kv`, `sunlight-clipd`, `sunlight-dialogd`, `rand_service`, `sunlight-tls`, and `wiseowl-memorydb`.
* Wise Owl memory DB is live but suspended because persistent `/state` is unavailable. The dependent memory, index, and brain units are configured but not spawned while their dependency endpoint is unavailable.
* `solar`, `vortex-lock-presenter`, `secret_store_test`, and `sunlight-thumbd` are disabled by default. Document service/docview definitions are conditional on build configuration.
* No authenticated graphical session, Vortex shell, or application child was present. No duplicate service registrations were observed in the captured profile.

The D3.5 snapshot remains the source audit baseline; D3.7 runtime results are an overlay rather than silently rewriting historical ownership evidence. A task/process row is not automatically a SunlightD-managed service. PID is observational only and is not used as a lifecycle identity.

## B. Memory baseline

At the final two-vCPU sample (about 15.0 seconds after SunlightD-ready), kernel accounting reported:

| Accounting class | Bytes | Interpretation |
|---|---:|---|
| Managed physical domain | 545,968,128 | PMM-managed bytes; the useful denominator for this report |
| Free frames | 104,656,896 | Free within that managed domain |
| Allocated managed frames | 441,311,232 | Managed minus free |
| Userspace private unique | 171,892,736 | Kernel's unique private classification |
| Shared unique | 16,384 | Unique shared backing, not a sum of per-process mappings |
| Kernel core | 119,009,280 | Kernel-owned core frames |
| Kernel heap | 16,777,216 | Kernel heap class |
| Page tables | 1,306,624 | Kernel-accounted page-table frames |
| RAMFS file data | 132,255,744 | Embedded file payload; remains resident in this profile |
| Other | 53,248 | Other classified managed use |
| FS cache / graphics / DMA | 0 / 0 / 0 | As reported by this instrumentation/profile |

The kernel's `installed` value is 13,958,571,008 bytes despite QEMU being invoked with 1024 MiB. This domain mismatch is unresolved; installed bytes are therefore not treated as guest RAM capacity, and comparisons use the PMM-managed domain. The RAMFS metadata field uses `u64::MAX` as unavailable sentinel. Kernel stack, per-process heap/stack split, and unique per-process physical attribution are not exposed. Per-process mapped bytes/pages in the JSON include potentially shared and file-backed mappings and must not be summed as physical consumption. Shared pages are counted once only in global unique accounting. RAMFS payload is reported independently; it is not added again to per-process mappings.

The initial boot free-frame line was approximately 282/521 MiB, 281/520 MiB, and 279/518 MiB on 1, 2, and 4 CPUs respectively. Final free memory was approximately 104.6–105.7 MiB. Between baseline and final runs, managed allocation differed by only about 8 KiB and free frames by about 16 KiB across the paired checkpoints. This is measurement noise / run variation, not evidence of reclaimed service memory. No production service was unloaded, so production frames reclaimed per unload are **not measured**.

## C. CPU, wakeups, and timing

| vCPU | Baseline wakeups/s | Final wakeups/s | Delta | Baseline context switches/s | Final context switches/s | Delta | Busy basis points, baseline → final |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 710.0 | 663.5 | -46.5 | 1077.0 | 1013.5 | -63.5 | 1175 → 1062.5 |
| 2 | 709.5 | 660.5 | -49.0 | 1216.0 | 1159.0 | -57.0 | 846.5 → 973 |
| 4 | 706.5 | 660.0 | -46.5 | 1343.0 | 1268.5 | -74.5 | 840 → 802 |

The wakeup and context-switch reductions are consistent with the DialogD idle receive change described below, but the paired TCG samples are single runs with serial diagnostic overhead and host variability. Aggregate busy time moves in different directions by CPU count; no general CPU percentage or performance improvement is claimed. Raw windows, per-task top CPU samples, and sample timestamps are retained in each `baseline-*cpu.serial.log` and `after-*cpu.serial.log`.

DialogD previously used a 20 ms timed receive/yield loop while idle despite doing no periodic work. It now blocks on the existing IPC receive primitive. No IPC or notification semantics changed. This limits idle polling in that task; it does not prove all measured wakeup reduction came from DialogD. Other observed periodic work includes thermal sampling at roughly one second and audio pump waits around 8 ms. USB mouse polling is about 10 ms (100 Hz) because the current xHCI path has no validated IRQ wait endpoint. These paths were not changed. SunlightD's bounded wait services exit/D2 deadlines. No timer/IPC retry optimization beyond DialogD was undertaken.

Boot-to-SunlightD, login, and desktop milestone timestamps were not recorded at sufficient resolution to report latency. Cold activation, warm resolution, idle grace, reactivation, and exit-to-reap values from D2/D3 fixtures are synthetic and are not presented as production timings. No application open/close comparison was performed.

## C. Residency classifications and candidate ranking

Classification is based on launch path and actual lifecycle contracts, not process name or low CPU use. The detailed default unit policy and runtime state are in the JSON overlay.

| Class | Current observed examples / policy |
|---|---|
| BootstrapRequired | Init, VFS, TTY, and SunlightD. Keep under current bootstrap owners. |
| Resident | Init services needed for baseline availability; current SunlightD default units without complete D1/D2 shutdown contracts; ClipD due to retained clipboard state. |
| OnDemand | D1/D2/D3 synthetic fixtures only. No production unit opts in. |
| SessionBound | Authenticated shell/session and its application-owned children; absent in this boot profile. |
| OneShot | `sunlight-swapd`, which terminates after bounded boot work. |
| BlockedCandidate | DialogD, ClipD, thumbnail processing, and deferred Wise Owl chain pending their specific missing contracts. |

| Candidate | Observed benefit | Risk | Missing prerequisites / disposition |
|---|---|---|---|
| DialogD | Idle 20 ms polling removed; paired runs show ~46–49 fewer total wakeups/s across 1/2/4 CPUs, with attribution limited. Its mapped count was about 974 pages in the final sample. | Medium for unloading: user-visible dialogs and client disappearance make lifecycle races consequential. | No D1 readiness registration, D2 lease/admission/quiesce/drain protocol, or end-to-end graphical dialog test. Keep resident. The blocking receive edit is validated as a narrow idle-wait change only. |
| ClipD | About 572 mapped pages in the final sample; none of that is a reclaimable-byte estimate. | High: clipboard contents and transfer state are user-visible persistent state. | No D2 lifecycle contract; KV persistence can be unavailable/volatile in this profile. Keep resident until state persistence/ownership transfer and drain are proven. |
| Thumbnail worker | Disabled by default, so enabling on-demand could avoid a resident task if a real workload needs it. | Medium/high: file access and malformed input behavior; no proven activation/drain lifecycle. | No D1/D2 protocol or validated file/resource ownership. Remain disabled; candidate for isolated future work. |
| Wise Owl dependent workers | Not spawned in this boot while MemoryDB persistence endpoint is unavailable. | High: identity/persistence and dependent service chain. | No lifecycle optimization proposed. Preserve dependency and availability policy. |
| Thermal sampling | Roughly one-second periodic sample, low observed wake rate. | Low to medium. | Current measured cost is small; no event source or demonstrated unnecessary work. No change. |
| USB mouse polling | About 100 polls/s. | High if replaced without a validated interrupt/wait path; input responsiveness risk. | IRQ-backed wait endpoint unavailable; no change. |

Audio, VFS, Networking, identity, security, and Wise Owl were inspected for resource accounting only. Their default lifecycle policies were not changed. No security, storage, or network prerequisite was converted to on-demand.

## D. DialogD and ClipD lifecycle audit

D3.5 centralized their startup under SunlightD; the measured boot confirms both are spawned by the current manager. Exclusive startup ownership is not proof of unload safety. DialogD accepts SHOW requests and runs synchronous dialog work, but lacks D1 readiness and D2 authenticated lease/drain integration. This phase ran the 13 `sunlight-dialogs` host protocol/color tests, not graphical dialog creation, cancellation, dismissal, or repeated-use smoke tests. Pending-dialog count is not currently an authoritative D2 drain condition. Keep DialogD resident.

ClipD obtains/persists current clipboard and history through KV. The boot profile reports persistence unavailable for `/state`, so volatile state is possible. ClipD has no authenticated lease/drain protocol, and no safe data transfer or persistence contract was demonstrated. Do not unload it or discard clipboard contents. Client-side startup ownership remains unchanged.

No service met every pilot criterion: D1 activation/readiness, identified clients, D2 authenticated leases, quiesce/drain, generation-safe endpoints, and known shared/persistent resource ownership. Therefore no production unload/reactivation pilot was attempted. D3.6R RecoveryBlocked survivors are excluded from ordinary D3.7 lifecycle candidates; no ownership or dependency record was cleared.

## E. D4 memory-pressure prerequisites

D4 can eventually request existing D2 idle-unload operations only after these inputs are authoritative and bounded:

1. Kernel PMM pressure observations with a defined managed-memory denominator, timestamp, and consistency flags; resolve the installed-vs-managed discrepancy first.
2. Per-service unique reclaimable-frame estimates that separate private, shared, file-backed, RAMFS, and page-table lifetime; mapped virtual pages alone are insufficient.
3. Eligibility registry, active generation, minimum uptime, active dependency holds, authenticated lease count, and last activity from the valid current manager generation.
4. Cold-start/readiness costs and session/user availability requirements to define hysteresis and minimum idle grace.
5. Critical-service exemptions for bootstrap, storage, network, identity/security, audio, graphics/session, and persistence components.
6. RecoveryBlocked records explicitly excluded from candidate enumeration.

Pressure must only influence which already eligible D2 service to ask to unload. D2 admission, lease, quiesce, drain, process exit, and reap checks remain mandatory. No global pressure threshold, automatic eviction, or arbitrary termination is implemented here.

## F. Validation, evidence, and remaining work

* **Audited:** source ownership snapshot and default boot configuration; measured process map and service runtime overlay.
* **Measured:** resident boot accounting and CPU counters on 1/2/4 vCPUs, with limitations above.
* **Implemented:** DialogD idle wait uses existing blocking receive; optional `cpu_accounting_diag` memory telemetry; resident test harness can observe after the ready marker; evidence summarizer writes machine-readable output.
* **Validated:** fresh default resident gates passed on 1/2/4 CPUs; Dialog protocol host suite passed 13 checks; native DialogD build passed; D3 lifecycle evidence reports 177 host checks and native 1/2/4 D1/D2/D3 gates passed. The D3 evidence is synthetic lifecycle validation and is not a production unload result.
* **Blocked:** safe DialogD/ClipD unloading; production activation/unload/re-activation cycles; authenticated graphical smoke; resource reclamation attribution per service; desktop/application workload measurement.
* **Not attempted:** general adoption/recovery, migration of Init services, production service unloading, D4 eviction, IPC/FastLane changes, or changes to device/input wait primitives.

The default resident validation uses `tools/test.sh sunlightd` with the optional diagnostic feature and 16-second post-ready observation. Raw serial logs are `target/d37-evidence/{baseline,after}-{1,2,4}cpu.serial.log`; machine-readable outputs are `service-inventory.json` and `validation-summary.json`. The summarized D3 synthetic regression evidence remains at `target/d3-evidence/validation-summary.json`. These target artifacts are local evidence and are not source-controlled.

Next safe work is to add generation-qualified DialogD readiness, leases, and a verified pending-dialog drain contract, then run real graphical interaction and client-disappearance tests. ClipD requires durable or transferable clipboard ownership first. Until either contract is validated, both stay resident and D3.6R RecoveryBlocked remains the fallback for all unresolved supervisor ownership.
