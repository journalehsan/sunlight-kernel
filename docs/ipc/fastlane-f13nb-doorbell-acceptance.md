# FastLane F1.3N-B: D3 replay and native Doorbell acceptance

## Decision and scope

**Gate A: VERIFIED NATIVE. Gate B: PARTIAL. Gate C: BLOCKED.**

The original D3 failure is an actual guest assertion failure, with an unresolved
lifecycle/IPC cause. It is not a boot timeout, an unexecuted trigger, or merely a
parser failure. Repeated passing runs do not resolve it. No scheduler, IPC,
activation policy, recovery policy, or production transport correction is
justified by the evidence gathered in this phase.

The changes add failure-only diagnostics to the synthetic dependency-test
build, retain the client reply payload in the graph assertion, and provide a
bounded replay/evidence validator. A separate, reproduced build-input isolation
defect is corrected: resident images must rebuild normal Init/SunlightD rather
than embed the last lifecycle-test binaries. This does not establish a cause
for the original activation error. Production Native FastLane remains unavailable.

The machine-readable index is
`target/fastlane-f13nb-evidence/validation-summary.json`. All commands, source
manifests, image hashes, raw serial and unsuccessful checks remain under that
directory. Dates in the JSON are UTC; the local investigation date is October
22, 2026 (Asia/Tehran). Historical F1.3N-A files were copied and hashed before
editing source, and the originals were preserved.

## Original failed replay

The authoritative failed log is
`target/fastlane-f13na-evidence/d1-d2-d3-failed-final-harness/serial-2cpu.log`.
Its controller detected native failure, terminated its own QEMU child, and
returned 1. The current investigation preserves the log, controller, build
logs, source manifest and classification under `original/`, indexed by
`original-manifest.json` and `original-audit.json`.

The failed invocation was the established `tools/test_sunlightd_d3.sh --cpus 1
2 4`, following the lifecycle compatibility suite. It built the manager and
synthetic services with `dependency-test` and the kernel with `dependency_test`.
Those features exclude the native Doorbell adapter. D1 runs concurrently with
D2 startup; D2 finishes before its driver enters D3. Security/IPC foundation
checks also precede D3. Earlier Doorbell/pin fixtures ran in separate VMs.

The reconstructed failing QEMU command is:

```sh
qemu-system-x86_64 -cdrom target/sunlightd-d3.iso \
  -serial file:target/d3-evidence/serial-2cpu.log -display none \
  -m 1024M -smp 2 -device virtio-rng-pci,disable-modern=on \
  -no-reboot -no-shutdown
```

TCG was recorded; no explicit `-cpu max` or `tcg,thread=multi` was supplied by
the D3 harness. The original argv and host load were not independently saved.
QEMU was 11.1.1, and rustc was nightly 1.98.0, commit `8954863c8`.
The earlier orchestration also records an interrupted duplicate lifecycle
controller. That is host context, not evidence that it caused these errors.

The 1-CPU guest passed. The 2-CPU guest passed D1, D2, ordinary scoped D3
activation, and graph cycles 0 through 3. Cycle 4 then reached:

```text
[D1] ready unit=d2-fixture generation=20 pid=136 process_generation=161 endpoint=2306 cold_ms=40 ready_ms=150
[D1] ready unit=d3-leaf generation=6 pid=137 process_generation=163 endpoint=2313 cold_ms=150 ready_ms=480
[SUNLIGHTD] spawned d3-parent pid=138
[SYSCALL] mmap(0x0, 0x100000) -> 0x1000000000
[CAP] Created endpoint 2319 for pid=138
```

Six graph worker threads subsequently failed `d3_driver.rs:50`, receiving
`0xD1FF` (ERROR) instead of `0xD180` (OK). Two workers had already exited with
code 0. The main driver did not reach its eight-success barrier or cycle-4
memory receipt. Parent creation and its mmap prove execution, beyond runnable
publication. The client failures likewise prove execution after reply delivery.
They do not prove that all other waits or scheduling obligations were healthy.

The **earliest missing expected logged checkpoint** is parent READY for
activation generation 6. The first observed incorrect checkpoint is the worker
ERROR reply. The causal divergence lies between recorded parent startup and
those replies; no earlier internal divergent transition can be recovered from
this log. In particular, the original assertion omitted `reply.words[0]`, so
Dependency, Stale, Busy, and other error paths cannot be distinguished.

Parent HELLO receipt/completion, its exact pending IPC, manager phase/failure,
waiter tokens, scheduler queues, pending IPIs and post-failure cleanup were not
captured. There was no activation-timeout marker in the failing interval. The
harness stopped on assertion before its 180-second watchdog. A later main
driver timeout is neither observed nor needed to classify this failure.

The copied legacy `validation-summary.json`, measurement JSON and 4-CPU serial
log are stale from an earlier run. `classification.json` correctly records
1=passed, 2=failed, 4=NOT_RUN. Neither this phase nor its parser treats that
stale 4-CPU log as a new pass.

## Successful isolated retry and image identity

The historical isolated retry invoked `sunlightd_d1_gate.py --d2 --d3` directly,
with the unchanged retained ISO, and passed 1/2/4 CPUs with six graph receipts
each and zero steady frame range. “Isolated” means the established D1/D2/D3
guest replay invoked independently of the wider host validation sequence. It
does not mean a guest containing only D3.

Retained retry ISO SHA-256:
`37dae18f3d7ef41bff7325713104b052a37bb4481df4011630626c28ddc4bd62`.
The ELF extracted from `/boot/sunlight-kernel.elf` has SHA-256:
`c64df48eca0b8a41c45fcb295cb7adc0d7bf0841c5da14b2de5696ef742249ae`.
The original failed attempt did not independently retain an image digest or
ELF. Its unchanged-image relationship is the previous investigation's recorded
provenance, not a newly recovered measurement from the failing process.
The historical source manifest had zero mismatches at this phase's start.

## Controlled reproduction matrix

The bounded plan retains three replays of the original ISO on the failing
2-CPU configuration, three independent replays of the initial diagnostic ISO on
2 CPUs, both Doorbell/D3 host orders, and the original compatibility-suite-before-
D3 host order. Every D3 attempt starts a fresh guest with the unchanged
180-second assertion watchdog and records the QEMU command. No attempt is
discarded when another passes. Individual repetitions and resource samples are
in `original-replays.json`, `isolated-final/results.json`, `matrix-c-final/`,
`matrix-d-final/`, and `combined-final/`.

After reproducing embedded lifecycle-test input contamination, the plan adds a
fixed corrected-input set: isolated D3 at 1/2/4 CPUs, Doorbell-before-D3 at two
CPUs, and the compatibility-suite-before-D3 order at 1/2/4 CPUs. A two-CPU
Doorbell replay follows corrected isolated D3. Normal resident startup is
checked separately at 1/2/4/12 CPUs. These are separate, fixed-image guests.

| Image / ordering | CPUs | Attempts | Observed result |
|---|---|---:|---|
| Historical combined failure | 1 / 2 / 4 | 1 passed / 1 failed / NOT RUN | Six worker ERROR assertions in 2-CPU graph cycle 4 |
| Historical isolated retry | 1 / 2 / 4 | 3 | PASS; six graph receipts each |
| Retained original ISO, isolated control | 2 | 3 | PASS; original-image controls, not a fix |
| Diagnostic ISO, isolated control | 2 | 3 | PASS; six graph receipts each |
| Diagnostic ISO after Doorbell 1/2/4/12 | 2 | 1 | PASS; six graph receipts each |
| Diagnostic ISO before Doorbell 2 | 1/4 | 2 | PASS; six graph receipts each |
| Diagnostic ISO after full compatibility suite | 1/2/4 | 3 | PASS; six graph receipts each |
| Corrected inputs, isolated | 1/2/4 | 3 | PASS; six graph receipts each |
| Corrected inputs after Doorbell 2 | 2 | 1 | PASS; six graph receipts each |
| Corrected inputs after combined suite | 1/2/4 | 3 | PASS; six graph receipts each |

The corrected set has **7/7 D3 passes**, 42 graph cleanup receipts, zero
assertion failures and zero watchdogs. All seven attempts have zero steady
frame range; every graph has `managed_after == managed_before`. Both fixed
2-CPU Doorbell-after-D3 replays also passed. Earlier successful controls retain
their original input qualifications; none cancels the historical failure.

Final native Doorbell results (`native-final/results.json`):

| CPUs | Transition records | Created / retired | Frames before / after | Terminal accounting |
|---:|---:|---:|---:|---|
| 1 | 42 | 43 / 43 | 68151 / 68151 | all zero |
| 2 | 42 | 43 / 43 | 67893 / 67893 | all zero |
| 4 | 42 | 43 / 43 | 67381 / 67381 | all zero |
| 12 | 42 | 43 / 43 | 65333 / 65333 | all zero |

Terminal accounting includes active objects, waiters, task receipts, wake
obligations, deferred dispatch, pending operations, queues and pins. Each CPU
configuration records 37 completions, zero metadata heap allocations and zero
polling wakes. Raw N1..N17 rows identify notify-before/during/after-block,
coalescing, spurious wake, close/cancel, exit, reuse, exhaustion and cycle
interleavings actually reached. On UP, N4 remote exclusion, N7 cross-CPU
notification and N13 remote-exit exclusion remain NOT_FORCED; their SMP rows
provide the remote proof. A boot alone is never the success criterion.

The final acceptance ISO identities are:

- D3, normal Init plus dependency-test manager: `c6fab2100f2e44f4d531e0cf8256e8ab6effddd1d3a0e7540a47644797b07467`.
- Native Doorbell exclusive fixture: `347e0e23a10d9246d9910a1a872a820fcf97ac044a0afbfb75833de6a7293f93`.
- Normal resident services, fastlane_f1_audit kernel: `61be8c6bbc84689488ecd21d80d13d279e5c5651bf4057a36ec46efff33294e1`.
- Pin-release exclusive fixture: `58c954f7302b2bc7bfb11a20db247bd353d421a1252f71b87cc6b0de60a122b4`.

Each corresponding ELF digest and feature-specific build command is in the
image entry of `validation-summary.json`; D3 and resident entries also record
embedded userspace digests. The corrected
D3 ELF, corrected resident ELF and normal default ELF contain zero Doorbell
adapter symbols (`final-symbol-audit.json`); the dedicated native ELF contains
76 (`doorbell-symbol-audit.json`).

D3's established harness supports 1/2/4 CPUs. Twelve CPUs are used for native
Doorbell, pin-release and relevant memory/network gates, not relabeled as an
unrun D3 configuration. A true D3-only guest and same-guest Doorbell/D3 reversal
are **NOT SUPPORTED** by the existing fixtures. The Doorbell boot enters
`idle_loop`, masks LAPIC timers, and ends with debug-exit; it never returns to
resident startup. Adding a production continuation/reinitialization protocol
would broaden this verification task. Host ordering tests explicitly record
fresh guests and cannot prove restoration for an unsupported continuation.

## D3 lifecycle trace from source

The existing lifecycle is `activation::Phase::{Inactive, Starting, Ready,
Failed, IdleCandidate, Quiescing, Stopping}`. `ServiceState` separately tracks
the supervised process. There is no new D3 state machine.

1. Kernel IPC mediation checks the real caller profile before ACTIVATE reaches
   `DemandManager::handle`. DISCOVER is metadata; activation grants no operation
   capability. Deferred replies retain opaque, generation-qualified kernel
   completion targets, with 16 manager waiter slots.
2. `plan` checks the bounded Requires/After closure, enabled state, cycles,
   stopping state, exhaustion, retained failed runtimes and cooldown. The
   manager reserves all dependency edges before requesting generations. A
   Starting unit joins one flight; Ready/IdleCandidate may reuse a validated
   runtime. The absolute activation deadline remains five seconds.
3. `tick` advances at most one dependency/spawn operation per turn.
   `spawn_service_at` requires the provider closure, binds exact provider
   activation/runtime identities, and reserves tracked process ownership
   through the established spawn service. A created process is Starting,
   not Ready. Scheduler Ready publication is not reported as execution.
4. The fixture allocates its private/shared resources and sleep endpoint,
   resolves the manager, sends HELLO, creates/registers its service endpoint,
   and sends READY after local initialization. The manager checks live sender,
   activation generation, process generation, endpoint ownership and transitive
   dependency bindings before `Activation::ready` publishes Ready.
5. Deferred activation completion revalidates dependencies, selects resolution
   or the existing error, and calls `ipc_complete_deferred_reply`. The kernel
   checks caller incarnation and pending-call generation, deposits the reply,
   and uses the existing scheduler wake path. Actual client execution is a
   separate observation, provided by the worker result or assertion.
6. Idle reports, dependency holds and administrative leases drive the existing
   deadline-based nomination, authenticated QUIESCE/DRAINED and COMMIT_EXIT.
   Stopping is not cleanup. The exact reaping receipt ends runtime ownership;
   only then may consumer edges release. Quarantined live processes keep holds.
   Provider failure invalidates the dependent closure without adoption,
   automatic replacement or an extended timeout.

For the original failure, activation started, providers became Ready, and the
parent executed. Whether parent HELLO/READY was rejected, a provider observation
failed, a pending activation was rejected, or an IPC/liveness defect intervened
remains unknown. No source-level candidate is promoted to a finding merely
because it could produce ERROR.

## Doorbell isolation and scheduler interaction

All adapter modules, process fields, lifecycle hooks, private dispatch `0xf1a0`
and boot entry require **both** `debug_assertions` and `native_doorbell_test`.
The number is absent from the public userspace syscall enum. Existing
NotifySend/NotifyWait stubs retain their contracts; this phase changes neither.
Ordinary services do not create these objects. Release builds exclude the
adapter even when the feature is requested.

The fixture prepares its tasks and object state before setting ARMED and
publishing scheduler readiness. Ordinary wake hooks without an exact Doorbell
registration return immediately; callbacks record only armed fixture state.
Other exclusive scheduler fixtures are rejected by its compile-time feature
guard. Combining dependency_test with native_doorbell_test does not run both
tests: the exclusive Doorbell entry prevents resident startup. It must not be
used as combined compatibility evidence.

The adapter serializes condition check, registration, exact token publication
and BlockedOnIo under SCHEDULER. Notify/close/cancel commit the receipt and Ready
transition under that same guard. The synchronous private kernel-stack
continuation enters the existing reschedule vector, with no unconditional
Blocked write afterward. `wake_io_pid` is reached only after exact
PID/incarnation/token checks; the existing owner, runqueue and physical pin
rules govern subsequent selection. Deferred dispatch stores CPU scheduling
work, never a retired Doorbell pointer or an old task-specific completion.

Exit withdraws registrations before task retirement; retirement requires a
terminal object and no waiter/process receipt. Checked object generations and
registration epochs do not reset on slot reuse. Native assertions independently
observe physical blocking, scheduler selection, client execution or deliberate
exit, single queue membership, exact stale-identity rejection and Reaped state.
The final CPU matrix records empty objects, waiters, receipts, obligations,
dispatch, queues and pins, with equal physical-frame counts.

This establishes isolation for the exercised exclusive fixture. It does not
establish that the earlier D3 error was harmless. Shared scheduler code exists
in both builds, but the failed D3 image has no Doorbell registrations or test
hooks. The errored callers executed after replies, so runnable publication or
an IPI attempt is not substituted for their execution evidence. No pin-release
algorithm or scheduler policy was changed.

## Changes and regression coverage

### Reproduced build-input isolation defect

The legacy compatibility builder rebuilds only the kernel. However,
`kernel/src/main.rs` embeds the canonical release `sunlight-init` and
`sunlightd` through `include_bytes!`. Those files can still contain the features
from a preceding recovery/dependency test. Kernel source identity alone does
not establish embedded userspace feature identity.

The fresh legacy `resident-1cpu.serial.log` contained:

```text
[D36R-TEST] live manager bootstrap competitor rejected PASS
[SUNLIGHTD] capability profile d2-driver uid=0 gid=0 caps=spawn-user,service-lifecycle,logging,time-sync,demand-fixture
[SPAWN] Request from pid=11 for path=/sbin/d2-driver uid=0 gid=0 caps=0x80100c8
[SUNLIGHTD] failed to spawn d2-driver detail=9
```

It also tried `d1-driver`. The old resident parser passed despite these
unintended fixtures. The new parser rejects this exact retained log with no
missing positive boot markers. `resident-contamination.json` preserves the
classification and raw-directory location. Diagnostic D3 images likewise
contained the recovery-test Init competitor assertion; their successful
repetitions remain controlled observations of that particular image.

`tools/fastlane_f13nb_resident.py` explicitly rebuilds normal Init and, for
resident images, normal SunlightD without test/default features. For D3 images
it then builds the intended dependency-test manager and fixtures, preserving
normal Init. It records both embedded binary digests, checks for unwanted
fixture literals, and verifies that the actual kernel ELF contains those exact
binary bytes. Its resident gate requires actual service startup and rejects
D1/D2/D3/recovery fixture markers, in addition to the established security/SHM
assertions. All corrected images use fresh directories; earlier passes are not
rewritten as normal-resident evidence.
The legacy F1.1 validator is retained unchanged for comparison; its resident
PASS alone remains insufficient. This phase uses the new builder and validator
for normal-resident acceptance.

This is a confirmed harness defect and a targeted correction. It is **not** a
confirmed explanation of the original graph-worker ERROR: the historical
isolated retry used the same retained image, the original payload is absent,
and no causal connection from the Init competitor to cycle-4 rejection was
captured. Gate B therefore remains PARTIAL even if corrected-input replays pass.

`services/sunlightd/src/demand_runtime.rs` adds failure-only diagnostics behind
`dependency-test`: rejection checkpoint, actual Phase, activation generation,
retained runtime, provider identity/binding, dependency count and deferred
completion code. It performs no extra IPC or clock reads and does not alter
the lifecycle decision. Expected crash/quarantine tests can emit these records;
their presence alone is not a failed test.

`services/sunlight-startup-fixture/src/d3_driver.rs` retains worker identity and
the actual reply in the existing OK assertion. The assertion is not weakened.
`tools/fastlane_f13nb_validate.py` bounds repetitions, refuses reused output
directories, records image/source identity, order, command, resource samples
and failure checkpoints after every attempt. It invokes all established D1/D2/
D3 assertions rather than accepting only a boot/completion marker.

Ten focused parser tests distinguish guest assertion, missing D3 trigger,
watchdog, truncated capture, duplicate graph receipts, parser failure after
guest completion, resource samples, contaminated resident startup, incomplete
service startup and normal resident acceptance. The original failed log remains a
GUEST_ASSERTION_FAILED input. Existing manager tests exercise transitive
failure, stopping-provider rejection, generation mismatch, retained shutdown
holds and bounded cleanup.

There is **no deterministic regression for the original cause**, because that
cause has not been established. No race is claimed fixed. The next useful
failure must retain its first rejection checkpoint and ERROR payload, and,
if those identify a blocked/nonexecuting task, a bounded generation-qualified
scheduler/IPC snapshot at that divergence. Repeating successful boots is not
a substitute for that evidence.

## Compatibility, accounting and performance

Exact commands and exit codes are indexed in `checks.json`. Compatibility
uses final source with separate immutable images for exclusive feature sets;
one ISO cannot simultaneously execute all these boot fixtures. The source
manifests and ELF/ISO digests qualify each result. Existing global recovery
logs were preserved before invoking its legacy shell driver. No evidence from
an interrupted controller is promoted to a pass.

Runtime Rust inputs were unchanged throughout the final native and compatibility
replay. The new resident validator and parser regressions were added after the
input contamination was observed; the whole-source manifest consequently
changes for tooling. Corrected-input sequences record their own stable complete
source manifests. Documentation changes do not change a kernel image. Original
ISO repetitions are explicitly historical-image controls, not executions of the
new diagnostic fixture.

| Gate | Configuration / evidence | Result |
|---|---|---|
| Kernel host / adapter host | 91 / 97 tests | PASS |
| IPC, audiod, SunlightD host | 61 / 25 / 29 tests | PASS |
| Dependency-test manager | 19 tests | PASS |
| F1.2 Ring / F1.3 Doorbell host | 5 / 8 tests; strict Clippy | PASS |
| Scheduler host / generation boundary | Established scheduler gate / 29 boundary tests | PASS |
| Parser regressions | 10 focused; 37 in the complete F13N parser suite | PASS |
| D1/D2/D3 and RecoveryBlocked | Established recovery-native 1/2/4; six D3 graph cycles per guest | PASS |
| Corrected isolated / combined D3 | 1/2/4 each, plus two-CPU after-Doorbell | PASS; original cause unresolved |
| Pin-release native | 1/2/4/12 | PASS; independent native chain retained |
| Native Doorbell | 1/2/4/12, plus order-control two-CPU replays | PASS for applicable forced cases |
| Scheduler native | Established 1/2/4/12 gate | PASS |
| C1.2F trace / mixed | Corrected normal lifecycle inputs, 1 CPU; 22 / 8 real guest trials | PASS; 30,200 / 13,184 completed samples, no censored samples |
| Corrected resident boot | 1/2/4/12; exact normal Init/manager bytes and no test markers | PASS |
| Legacy resident boot | Old parser passed 1/2/4/12 despite test binaries | EXCLUDED from normal-resident acceptance; retained as harness counterexample |
| VM/TLB | Shootdown 12 CPUs, remote unmap/protect 4 CPUs | PASS |
| SHM retirement, authority and exec lifecycle | Established authority 1/2/4/12; nine exec commits/reclaims, eight replacements, live-borrower rejection | PASS for established fixture assertions |
| Native TCP/TLS | Corrected resident ISO, 12 CPUs; verified HTTPS, untrusted CA and wrong-host rejection, SOCKS hostname forwarding | PASS after authorized local-socket rerun |
| Audio | audiod host tests, mixed native test-tone with silent backend | PASS for exercised behavior; no human-audible/capture claim |
| Bare-metal compilation | Default, native feature and release-feature checks; final default target build | PASS |
| Strict kernel Clippy | 24 matched error/file pairs in both normal and native checks | Existing failure; zero current-only pairs |
| Primary kernel Clippy audit | 132 current / 132 baseline keyed diagnostics | Existing failure; zero additions |
| Changed-service strict Clippy | Dependency-inclusive 5 / 5 signatures; no-deps 18 / 18 | Existing failures; zero current-only signatures in either comparison |
| Workspace formatting | 53 raw hunks byte-identical to baseline | Existing failure; zero new hunks |
| Media/WAV fixture | 36 passed, one missing `docs/songs/onaldin_music-catch-the-sunlight-333176.wav` | MATCHED BASELINE FAILURE |
| Miri | Toolchain component unavailable | UNAVAILABLE |
| Changed Rust formatting / diff whitespace | Two changed Rust files and repository diff | PASS |

The original and corrected sandbox TCP/TLS attempts both stopped at host
`socket()` with PermissionError before QEMU. Both remain in the check list.
The authorized rerun uses the corrected immutable ISO, preserves actual QEMU
argv, and passed all expected command outcomes; certificate verification was
retained. `tcp-tls-authorized-command.json` records its 12-CPU/4096-MiB command.

The compatibility wrapper exits 1 for its existing format check; its known
media failure and contaminated resident observations remain visible. Its
nested pass flags are legacy harness results, qualified by the explicit
interpretations above, not an overall acceptance decision. Additional strict
service lint comparisons use an isolated read-only HEAD archive and separate
artifact directories; the changed D3 fixture is included in that comparison.

Final corrected D3 graph readiness samples are descriptive measurements:
42 samples, p50 310 ms, p95 330 ms, maximum 340 ms. These include graph/worker
startup and do not establish a matched performance improvement. Native
notify/Ready/execution quantiles, phase IPI/context-switch counts and the
roughly 100-ms idle intervals are retained in the JSON. The idle intervals
record zero handlers. Only 1 same-CPU execution sample exists per SMP run;
those quantiles cannot establish a latency tail distribution.

Doorbell accounting covers every object/task lifetime, not just boot PMM
samples. D3 reports every graph's private frames, page tables, shared pages,
holds, managed process counts, cold/warm activation and confirmed unload.
Successful graph receipts establish their own cleanup; the original failed
guest's cleanup remains unknown. Whole-system heap drift and an exact
post-crash dependency inventory are not inferred from these samples.

Doorbell timing/counter samples are diagnostic TCG observations and include
instrumentation. Only same-CPU notify/commit/execution timestamps may be
subtracted. Cross-vCPU clock comparability is unestablished. D3's reported
ready_ms includes worker creation and graph startup; wall runtime of a whole
VM is not activation latency. Context-switch/IPI counters are fixture-phase
counters, not uniquely attributed per message. No hot-path serial logging,
periodic retry poller or benchmark optimization was added.

Matched performance, reliable tail distributions, cross-CPU notify-to-execution
latency, D3 dispatch latency and whole-service metadata/heap drift remain
**UNAVAILABLE**. No speedup, CPU saving, idle-power saving or production
throughput claim is made. Missing matched performance alone is not classified
as a correctness regression.

## Final prerequisites

Gate B cannot become VERIFIED NATIVE while the original assertion lacks a
causal explanation. The narrowed unresolved failure and all passing replays
remain visible together. Gate A's independent pin-release evidence is retained
with its fresh native regression results. Gate C stays BLOCKED regardless of
Doorbell fixture success.

Before production Native FastLane: close the D3 causal gap; implement and verify
Restricted SHM authorization, recipient access rights, active revocation,
delegation wiring and cross-process exit/exec/generation lifecycle; then validate
production channel creation/teardown and real service integration. No legacy
unrestricted SHM substitution, public wake API, capability issuance expansion,
recovery adoption or production transport is introduced here.
