# M5.3 native measurement contracts

2026-11-03. Measurement readiness and correctness only. This document defines
future measurements; it contains no comparative benchmark results. Durable
receipts and the final acceptance status are in `.fastlane-evidence/m53/REPORT.md`.
Production FastLane remains disabled. No optimization, batching change, ring
capacity change, capability relaxation, commit or push is part of this work.

## Scope and source

The pre-change HEAD is `04ac8dfac5b675d64c9066be39dffe8b1f6b7e2b`.
The full worktree identity was recorded before editing under
`.fastlane-evidence/m53/baseline-20261103T132244Z/`. Its 1,375 canonical executable
inputs match the M5.2 current-reconciliation source map exactly. The dirty
worktree is intentional; HEAD alone does not identify either image.
The canonical map encoding for M5.3 is UTF-8 Python `json.dumps(map,
sort_keys=True)` followed by SHA-256. Its digest is
`d851531c352735e60dcb853274ef178c485c9983bf437044ec8ea8afe1b56a31`.
M5.2 used a different canonical encoding; map equality is the reconciliation
criterion. The broader pre-change 10,981-file manifest has its own digest and
includes non-build files. Each build and native summary archives the exact
source hashes before/after the attempt, service bytes, ELF and ISO identities.

## Future M6 metric definitions

Use the same workload, peer identity, payload sequence, CPU placement, timing
source and observation configuration for conventional SunlightOS IPC,
FastLane kernel-mediated transfer, and FastLane shared-ring transfer. Transport
implementation alone may differ. These fixtures currently validate the M5.2
shared-ring path; existing M5.1, IPC and lifecycle fixtures are correctness
regressions. They are not interchangeable measurement implementations yet.

The initial future comparison profile is explicitly limited to inline payloads
of 0/1/8/32/48 bytes, one independently identified message outstanding at a time,
with identical payload bytes and acknowledgement policy. Conventional `IpcMsg`
contains eight u64 words: reserve word 0 for sequence and word 1 for length, with
up to 48 payload bytes in words 2–7. Set unused bytes to zero and `cap_count=0`.
Do not silently compare this profile with the current 0–256-byte ring correctness
sequence or compare one fragmented logical IPC message with one ring message.
Payloads beyond the inline common profile require a separately versioned bulk
protocol contract before admission. This is a future contract, not an executed
M5.3 workload or a change to current batching.

| Future path | Establishment | One successful logical transfer | Retirement |
|---|---|---|---|
| Conventional IPC | Create/bind endpoint, authorize producer SEND and consumer receive/reply, exchange ready identities | Producer call; kernel copies/queues the inline request; consumer receives a private `IpcMsg`, validates sequence/length/bytes, replies with sequence acknowledgement; producer receives that acknowledgement. Count one message, actual application bytes only. Retain IPC syscall and queue/scheduler costs | Release endpoint/capabilities and prove no outstanding request/reply ownership |
| Kernel-mediated FastLane | Original M5.1 channel authorization, attach, persistent mappings and repeated-transfer activation | Existing M5.1 hostile adapter over `transfer_word` syscalls and its kernel-controlled publication/reclamation protocol; validate one private copy, complete the same logical acknowledgement. All word-access and control syscalls count; no debug formatting belongs in the future common window | Original close/revocation plus complete view/TLB/backing retirement |
| Shared-ring FastLane | Original M5.2 authorization, attach, persistent mappings and native role activation | Existing direct mapped-word adapter, publication, private-copy validation and release; Doorbell remains a notification hint. Complete the same logical acknowledgement. Direct memory accesses, required notifications/waits and control syscalls count | The same original close/revocation and physical-retirement contract |

For the future end-to-end sample, record before the successful message's
publication operation and after the consumer has validated its private copy and
completed the transport acknowledgement handoff. For IPC that includes the
reply handoff; for a ring it includes lease release. Throughput completion also
requires the producer to observe the acknowledgement. A future common
supervisor gate encloses those transfers only. Setup/grant creation, identity
handshakes, warm-up, post-window receipt formatting and retirement belong to
separate phases. Current M5.3 samples and serial controller windows remain the
documented ring diagnostic profile; they are not substituted for that future
common profile.

| Metric | Exact future contract | Current observability |
|---|---|---|
| Successful messages/s | Consumer-validated and transport-acknowledged **measured** messages divided by a single positive common transfer window in seconds; require exact producer/consumer count match and successful terminal acknowledgement | Counts available; no rate calculated in M5.3 |
| Payload bytes/s | Sum of actual acknowledged payload lengths over that same window; excludes headers, padding, failed publications and notifications | Actor byte counters available; no rate calculated |
| End-to-end latency | Publication attempt entry for a message that succeeds, through validated private receipt and release of that same sequence; includes queue residence and the transport's required copy; excludes channel setup | First 64 measured sequences sampled with public monotonic ms, source timestamps retained; coarse and observer-affected |
| p50/p95/p99 | Sort matched latency observations; nearest rank `ceil(p*N)` (one-based). Report N, sample selection and quantization. Never treat absent or dropped samples as zero | No percentiles reported. Deterministic prefix is not representative of an entire long run; 10 ms resolution is inadequate for qualified IPC percentiles |
| Producer/consumer CPU time | Per-incarnation scheduler effective-runtime delta, including open execution intervals and syscall tails; excludes blocked time. Separately charge control and measurement work | Native scheduler-runtime deltas retained as diagnostics; calibrated TSC cross-core stability is not certified here, so these are not qualified benchmark CPU-time results |
| Kernel/user execution time | CPU runtime split by user/kernel execution transitions, excluding off-CPU blocked intervals | UNOBSERVABLE in existing accounting; never infer from wall time or syscall count |
| Syscalls | Each dispatch entry by the selected actor during its enabled kernel window, including control and observation calls; separately annotate observer calls | Checked per-process counters under the existing scheduler lock |
| Context switches | Per-actor task-change **switch-ins** at publication of a different current task on a core; include idle-to-task and migrated dispatch, exclude same-task resume | Checked counter; distinct from system-wide context-switch diagnostics |
| Doorbell notifications | Actor attempts to invoke opcode 10, including duplicate hints; successful publication count remains separate | Checked per-actor counter; exact expected cooperative notification count checked |
| Wakeups | Scheduler wake publications for the selected actor, via existing `record_wakeup`; separately retain identity-bound Doorbell registration/completion/take receipts | Checked per-actor wake-publication counter; not a count of runnable instructions or proof every publication slept |
| Backpressure | Producer attempts returning `Full` in the measured window | Checked local counter; warm-up attempts excluded |
| Allocations | Count allocator calls separately for establishment, transfer, reporting and retirement; distinguish allocation operations, bytes and retained capacity | Transfer adapter and local counter/sample storage are source-audited allocation-free. Existing backing-allocation receipts are exact object inventories, not allocator-hook measurements. General measured allocation count remains unavailable |
| Persistent mappings | Count live role-qualified mapped views at each phase boundary, separately from backing objects; do not equate physical objects and views | Three pages/role, six views/channel and three backing objects/channel, validated by M2 receipts; final restricted inventory must be zero |
| Setup latency | Supervisor before authorization/create through both role mappings and adapter activation | Independent setup timestamps for four cooperative channels |
| Retirement latency | Before close through zero local/channel resources, required TLB ACKs and diagnostic cleanup checkpoint | Existing bounded cleanup timestamps plus per-view physical retirement proof; includes census/diagnostic observer work |

A timed-out or incomplete run has **no throughput denominator or numerator
eligible for acceptance**, even if it published messages. A complete run with a
zero coarse interval is invalid for rates. Partial counts remain diagnostic
only. Expected terminal faults are adversarial correctness cases and never
contribute successful benchmark traffic. Source-audited zero allocation is not
a measured zero-allocation claim.

## Phases and bounded windows

The native fixture performs these stages for each cooperative channel:

1. Establish the authorized channel, attach both actors, map and activate roles.
2. Exercise identity/role denials and cancelled reservations, then probe clocks.
3. Enable a warm-up state independently of measurement. Counts 1/16 use zero
   warm-up; counts 1,000/10,000 use 16 fully acknowledged messages of warm-up.
4. Send explicit start commands to both actors after the warm-up acknowledgement.
   Reset counters and samples; record each actor's public-clock start. Kernel
   counting starts just before the user counter window.
5. Transfer a fixed count of 1/16/984/9,984 measured messages with unchanged
   batch sizes and original ring geometry. Producer publication and consumer
   private-copy/release are counted separately. Full attempts do not count as
   messages. Every batch retains the existing transport acknowledgement.
6. Complete each actor window, disable its kernel counters, then emit compact
   receipts. Payload/order assertions occur online; aggregate validation and
   final supervisor acknowledgement follow measurement completion. Reporting
   allocations and sample printing occur after counters are disabled.
7. Retire the channel through the original path. Validate resource inventories
   and physical/TLB retirement separately from the transfer interval.

These are correctness windows, not a production throughput harness. They
include the original supervisor's control IPC, notifications, forced Full
attempts, online byte checks and batch acknowledgements. Their actor windows
are nested within individual kernel windows; start/stop commands are serial,
not a common simultaneously released gate. A future M6 common window must
start after both actors are ready and end on successful final receipt before
reporting. Do not turn the current diagnostic windows into comparative rates.

## Storage, overflow and observer effects

`fastlane-measurement-test` opts in for the user fixture;
`fastlane_measurement_test` opts in for the debug kernel and inherits the M5.2
feature chain. Ordinary builds have neither. Actor state is private to its
independent address space: integer counters plus a fixed 64-timestamp array.
There is no heap allocation, logging, shared measurement atomic or new lock in
successful measured-message accounting. Timestamp reads issue a clock syscall
for each sampled message; failed sampled publication attempts can issue a
clock read whose timestamp is discarded. Counters use checked addition and
latch overflow. Sample capacity is explicit: excess **latency observations**
are counted as dropped; message and byte counters continue. No sample is
silently overwritten. Any counter overflow rejects acceptance.

The bounded prefix selection does not support whole-run distribution claims.
Samples are emitted after completion, four per line, because DebugLog accepts
only 256 bytes. Schema and semantic checks reject missing/truncated chunks,
duplicates, wrong identities, out-of-order chunks, negative implied latency,
clock regression, count/byte mismatch, overflow and invalid integrity claims.
There are eight actor receipts, eight kernel actor receipts, sixteen clock
probes and at most 128 sample chunks. Each machine receipt is capped at 65,536
encoded bytes. Correctness serial output remains bounded by the fixed fixture
case and transfer limits; adversarial M5.2 logs remain for their existing proof.

Kernel counters reside in each experimental process record. Dispatch-entry
updates reuse the lock already held for native/Linux ABI selection. Task-change
counts reuse existing current-task publication and its existing per-core lock;
wakeup counts reuse the existing scheduler path. There is no new global
measurement lock or per-message kernel allocation. Four counters and metadata
increase experimental process size. User state increases by the sample array
and scalar fields; native ring pages, offsets, cache-line boundaries, capacity
and hostile adapter layout do not change. Actors retain CPU 0/1 placement after
affinity migration probes. Existing M5.2 global diagnostic atomics remain an
observer effect; they are not presented as actor counts. Their original
unchecked atomics are inherited diagnostics, not qualified measurement
counters; only the new checked per-actor counters supply this contract.

Instrumentation changes code generation, cache working sets, scheduler-lock
hold time and syscall traffic. Prefix sampling adds clock syscalls, and removing
cooperative per-message logs removes DebugLog calls and formatting allocations.
Consequently instrumented and control runs can be compared for correctness,
security outcomes, identities, ordering and final inventories, not as an
instrumentation performance comparison. No zero-overhead claim is made. The
native clock-probe spans bound the coarse aggregate observation cost of 257
clock reads; they cannot resolve individual read cost or overhead subtraction.

## Clock contract and limits

`ipc::monotonic_millis()` invokes syscall 86 and
`timekeeping::monotonic_ms()`. The latter reads one atomic global BSP-owned
100 Hz counter. APs cannot advance it. Resolution is 10 ms, even though the API
returns milliseconds. Monotonic nanoseconds uses the same coarse counter;
changing units does not improve resolution. Delayed calibrated BSP callbacks
catch up from elapsed time and never decrement the global ticks. Fallback
callbacks advance only on the BSP. Calibration derives BSP elapsed time from
TSC; this does not require comparing an AP TSC to the BSP for public reads.

The monotonic contract supplies one scalar across scheduling and migration.
Native probes pin each actor to 0/1 and back, read the clock 257 times per probe,
and reject within-probe and across-migration regressions. This is empirical
coverage across requested affinity changes, not hardware certification for every AP,
long-duration drift, suspend/resume or arbitrary bare-metal topology. Probe CPU
labels record requested affinity; there is no independent per-read CPU sample.
Kernel actor receipts verify the restored producer CPU 0 / consumer CPU 1 at
window completion. Cross-core comparability fundamentally follows the single
global clock source, not a claim that every probe read was independently
attested to a particular CPU. Tests
using the exact extracted timekeeping source cover non-owner rejection,
calibration regression, saturation, duplicate ticks and catch-up. Ordinary
kernel tests exercise the same implementation. At saturation, conversion
plateaus instead of wrapping; receipts reject the saturated clock horizon.

For ideal uninterrupted ticks a timestamp has up to one tick of quantization;
two endpoint errors can differ by up to two ticks conservatively. IRQ delivery,
BSP stalls and virtualization add unbounded scheduling uncertainty beyond that
simple quantization bound. TCG guest time is not host physical elapsed time.
Zero span for 257 reads means below the observed coarse boundary; it does not
mean zero clock-read overhead. Scheduler runtime uses calibrated raw TSC in
`interrupts::now_ns()` and saturating deltas; its successful arithmetic alone
does not prove cross-CPU calibration. Receipts therefore retain it as an
unqualified observation. Kernel/user splitting is absent.

## Security and next step

M1 authority, VM exclusivity, mapping permissions, hostile adapter, notification
ordering and retirement methods remain the enforcement boundaries. Diagnostic
start/stop validates the caller's full generation-qualified channel handle,
live state, authenticated identity and previously activated native role.
Counters do not grant access or drive lifecycle decisions. A killed process
may lose its private measurements; successful compact cooperative receipts
must not be required for expected adversarial terminal faults.

The M3.2 contract remains: memory safety **PROVIDED**, stable private snapshots
**PROVIDED**, coherent publication **NOT PROVIDED**, application authenticity
**NOT PROVIDED**. Expected cooperative payload equality does not authenticate
a hostile producer and does not certify coherent publication.

Historical M1 timeout attribution, historical M4 failures and historical SHM
census drift remain **UNRESOLVED**. Miri remains **NOT_RUN** unless explicitly
run. These classifications are independent of current correctness passes.

M6 comparative benchmarking remains **NOT READY** until a sufficiently precise
native clock is validated, common workload gates exist for all three transports,
CPU-runtime clock qualification and requested allocator accounting are supplied,
and an appropriate latency sampling/distribution policy is selected. Any
unobservable kernel/user split must stay explicit. The bounded fixture and
correctness receipts are preparation for those steps, not comparative results.
