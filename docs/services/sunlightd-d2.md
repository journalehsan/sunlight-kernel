# SunlightD D2: cooperative idle unloading

## D2.0 lifecycle audit

D1 was reviewed before implementation, including `sunlightd-d1.md`,
`target/d1-evidence/validation-summary.json`, the native serial logs and latency
records. The retained baseline is 155 host tests and native 1/2/4 CPU passes.
Generated evidence remains under `target/`, outside version control.

The manager's single event loop owns activation state. Activation generations
are checked, nonwrapping u64 values. Process generation and endpoint identity
are separate kernel-validated identities. D1 Ready resolution is a snapshot;
it neither reserves service admission nor tracks subsequent direct IPC.
Readiness is authenticated to the spawned primary process. Worker processes
cannot report readiness on its behalf. D2 retains that boundary.

The runtime endpoint belongs to the service, not the manager. Nameserver lookup
produces SEND capabilities; removing a nameserver entry alone cannot stop
clients with existing capabilities. D2 therefore requires a service admission
barrier, including calls using previously issued capabilities. The kernel
reaper destroys owned endpoints, closes blocked calls, revokes derived tokens
and clears capability and deferred-reply state. Discovery remains logical and
does not expose owner tokens.

Spawned services are children of init. SunlightD cannot depend on waitpid to
reap them. `SessionGetCredentials` and kill(pid, 0) both exclude Finished as
well as Reaped processes. Consequently D1's loss-of-liveness observation
**does not prove reclamation**. `reap_process_resources` separately waits for
the outgoing stack handoff, run-queue removal, all active CPUs and address-space
borrowers. Only then does it release file handles, shared mappings, private
user pages, page tables, kernel stack and IPC state. Address-space reclamation
invalidates its generation. D2 needs a retained, generation-qualified reaping
receipt; it must not infer successful unload from D1 liveness alone.

Shared mappings are ledger tracked. `cleanup_shared_pages` decrements mapping
references and frees backing frames only at the last mapping. The private
page walker excludes SharedMemory, Framebuffer, Telemetry and BootSharedData
regions. A peer-held shared page is not reclaimed by service exit. No D2 path
may asynchronously unmap a live process's user buffers.

Requires and managed After currently gate startup, but D1 has no residency
holds. D2 must hold a provider while a consumer is Starting or retains a
runtime, including failed-but-not-yet-reaped consumers. Wants remains advisory.
An idle provider must be reconsidered when such a consumer exits.

D1 counts activation requests and waiters, not work, sessions, outstanding
buffers, background tasks or persistent transactions. Those are visible only
to the service. A trusted service's idle report can nominate a candidate, but
cannot replace a fresh closed-admission, drained-work acknowledgement.
Manager-owned residency leases need bounded capacity, exact client/process
and service/activation ownership, and bounded expiry. Expiry relinquishes a
residency promise; it must never authorize freeing accepted work or buffers.

Legacy Stop sends SIGTERM, polls for exit, and escalates to SIGKILL. Native
SIGTERM has a fatal default disposition; the native sigreturn/pause paths do
not supply a general graceful callback mechanism. ExecStop is parsed but is
not executed. This legacy path is unsuitable for cooperative idle unloading,
especially while persistent writes are uncommitted. D2 must use an explicit
service protocol, bounded IPC and confirmed reaping, with no forced timeout
termination. Startup timeout cleanup remains the D1 policy for initialization.

Audit prerequisites, implemented by the pilot below:

* A kernel reaping receipt retaining the exact process generation after the
  address-space identity is destroyed, without weakening credential queries.
* An authenticated service admission/drain/commit handshake and an explicit
  irrevocable boundary; failure after closing admission must quarantine the
  runtime instead of advertising it as Ready.
* Event-scheduled idle deadlines and expiring client residency leases, separate
  from service-local accepted-work/resource accounting.
* Dependency residency checks and safe handling of legacy Stop/Restart for
  opted-in cooperative services.

## Thumbnailer assessment

`services/sunlight-thumbd/src/main.rs` is currently Type=simple, disabled in the
compiled manager table, and registers `thumbd` rather than its executable
basename. It has no D1 readiness or D2 admission/drain contract. PATH_CHUNK
retains a multipart client request between calls. PATH_END performs document
RPCs, temporary cache writes, rename and best-effort metadata writes; these
must be included in active-work and persistent-transaction accounting before
unload is safe. Its cache/prewarming failure history also needs independent
validation. It is not eligible for this pilot. Its policy and the policies of
Audio, Networking, VFS, Compositor and Wise Owl remain unchanged.

## Opt-in policy and supported contract

```ini
[Service]
Type=notify
Activation=on-demand
Restart=no
UnloadWhenIdle=yes
IdleGraceMs=1000
ShutdownTimeoutMs=1000
MinimumUptimeMs=1000
ExecStart=/sbin/cooperative-service
Capability=service-notify
```

UnloadWhenIdle defaults to no. All three durations default to 1000 ms, matching
D1's one-second retry interval and staying below the existing three-second
legacy stop grace. Grace and minimum uptime accept 50..60000 ms; shutdown
accepts 50..1000 ms. Invalid numbers, unknown booleans and resident unloading
are rejected. Existing on-demand notify/no-restart restrictions still apply.
Minimum uptime starts at verified readiness, which is stricter than spawn time.
All scheduling uses the kernel's monotonic millisecond clock (10 ms resolution).

The service must implement the D2 protocol before enabling this policy. It must
account locally for accepted operations, sessions, outstanding buffers/resource
leases, background tasks and uncommitted persistent transactions. Direct IPC
remains direct; the manager does not see or count every operation. An aggregate
primary-process IDLE_REPORT supplies activation generation, busy count and
uncommitted transaction count. Counts must include resources retained between
calls, not just currently executing code. A nonzero declaration cancels grace.
Zero/zero only nominates an idle candidate; it never authorizes exit by itself.
Privately spawned helper processes must also drain or be explicitly retained
as dependencies; parent exit alone does not reclaim their separate address
spaces. Shared-address-space workers remain subject to the kernel reaper's
borrower and active-CPU guards.
Lost reports conservatively retain residency. Repeated idle reports do not
extend a pending grace period.

Only the expected live primary PID and kernel process generation can report
for its activation generation. HELLO now additionally returns the manager PID
in word 1. The service learns it through the nameserver-authenticated manager
endpoint and checks the kernel-stamped incoming badge on control messages.
This experiment keeps D1's primary-process reporting restriction; delegated
worker reporting and manager crash recovery remain outside its contract.

There are 32 manager-owned residency leases across all units. LEASE_ACQUIRE
uses activation generation, service PID, service process generation and a
50..60000 ms lifetime. Acquisition requires the same kernel-checked
ServiceLifecycle administrative authority as ACTIVATE, including when the
caller was handed a manager SEND token. It only pins an already Ready or
IdleCandidate runtime; it cannot activate an inactive or stopping service.
Successful acquisition cancels grace. Each checked, nonwrapping u64 lease ID
records the service's full runtime identity, activation generation, and the
actual caller PID/process generation. LEASE_RELEASE requires the exact ID,
activation generation and owner incarnation. Other clients cannot decrement
the count. Release is allowed during shutdown but cannot reverse a commit.

Lease expiry is an explicit end of a residency promise, not permission to
discard accepted work. A dead client's manager-owned leases disappear by their
deadline (at most 60 seconds plus manager scheduling delay), without a client
death scan. Service death removes its leases on confirmed reaping. Service-local
session/resource cleanup must still be implemented by each real service; a
service that cannot attest that cleanup must refuse to drain and stay resident.
Clients needing longer residency can acquire a replacement lease before expiry
and release the previous one; capacity exhaustion is explicit Busy.

Requires and managed After providers are pinned by Starting consumers and
consumers with retained runtimes, including quarantined D2 consumers. Existing
resident active state is also respected. Wants does not acquire a hold. The
manager reconsiders eligibility on activation, control and dependency-exit
events. No production service receives a new dependency or lifecycle policy.

## Quiescing, commit and confirmed exit

The discovery phase values preserve D1's 0 Inactive, 1 Starting, 2 Ready,
3 Failed, and append 4 IdleCandidate, 5 Quiescing, 6 Stopping:

```text
Inactive -> Starting -> Ready -> IdleCandidate -> Quiescing -> Stopping -> Inactive
                            ^       |                |
                            +-------+                +-> Failed (runtime retained)
```

1. A zero-work report with no residency/dependency hold schedules one unit
   deadline at max(report time + grace, readiness time + minimum uptime).
2. Authorized activation during grace cancels and rearms the grace deadline,
   returning the same validated runtime. Work or leases can cancel candidacy.
   Reuse remains a point-in-time resolution; subsequent direct admission can
   explicitly fail if shutdown wins later.
3. The scheduled deadline rechecks phase, work declaration, holds and endpoint
   ownership, then enters Quiescing and sends QUIESCE(generation, nonce) using
   bounded IPC. A checked nonce distinguishes attempts within one activation.
4. The service closes admission across every endpoint and direct operation,
   finishes/replies to previously accepted work, and releases local resource
   references. It refuses with ERROR(BUSY) if persistent transactions, sessions,
   background work or resources cannot safely drain within the budget. A refusal
   promises admission was never closed and returns the manager to Ready; it
   requires a fresh idle report before another attempt.
5. DRAINED echoes the generation and nonce and explicitly reports zero work and
   zero persistent transactions. The manager verifies the full reply shape,
   exact live runtime, endpoint identity, phase and holds again. This verified
   acknowledgement is the **irrevocable commit boundary**. No new Ready reply
   can be issued for this generation after that boundary.
6. The manager enters Stopping and sends COMMIT_EXIT(generation, nonce). The
   service keeps admission closed, replies OK echoing both values, and exits
   itself. New direct work receives explicit STOPPED or transport closure.
   Requests queued in the manager during its bounded handshake are handled in
   the resulting state: ACTIVATE gets Busy during Stopping, or can start a new
   generation after confirmed exit. No service operation is silently queued
   for replay. The administrative caller decides whether to retry Busy.
7. The existing process supervisor checks a manager-only, generation-qualified
   kernel receipt. Successful unload requires the verified commit reply, zero
   exit status and fully Reaped state. Sending either request, receiving an ACK,
   kill(pid,0)==false, or observing Finished is insufficient. The receipt is
   recorded, leases/timers are removed, and only then is the unit Inactive.

QUIESCE and COMMIT share one absolute shutdown budget. The single manager loop
cannot process an activation in the middle of that synchronous exchange;
concurrent callers remain in bounded kernel IPC and are checked afterwards.
Immediate replies are sent before maintenance; a prepared Ready resolution
cannot sit in the daemon while an idle deadline commits the same generation.
This keeps its existing ownership model. A cooperative exchange can delay
other manager work by at most the one-second budget plus bounded lookup and
scheduling overhead. Services should bound idle-report RPCs well below that
budget; the pilot makes at most three 100 ms attempts for an idle declaration,
allowing crossed reports to cancel before the primary loop receives QUIESCE.
These are bounded retries of an explicit report, not an idle polling task.
Accepted delayed calls relinquish their current reply target with the existing
deferred-reply API before sleeping; expired completion tokens are tolerated.
No service callback waits indefinitely for a
manager that is simultaneously waiting for it.

Unknown/malformed/stale drain replies, quiesce transport failure, missing commit
confirmation and exit timeout quarantine the live runtime as Failed. The
manager retains its identity, never spawns a replacement, and never forces
unmapping or sends a timeout SIGKILL. A later confirmed exit releases the
runtime; a later authorized activation can retry after D1's cooldown. A service
that closed admission must keep it closed on an ambiguous failure. Services
that never closed admission may continue serving their existing clients.
Neither case is falsely advertised Ready. Cooperative units also retain their
runtime on initialization timeout; the D1 forced startup cleanup remains only
on the original non-unloading path.

Legacy Stop/Restart on a live cooperative unit returns a structured
DETAIL_TRANSITION_BUSY result instead of entering legacy signal escalation.
The caller must wait for cooperative unloading or use a service-specific safe
shutdown mechanism. Once inactive, existing explicit start/restart semantics
remain available. This conservative pilot does not promise an administrative
forced-stop override for uncommitted persistent state.

## Kernel receipt and resource accounting

The explicit SPAWN_TRACKED mode is accepted only from the trust-chain-marked
manager. Registration of its receipt watcher happens under the spawn lock,
before the new process can execute. At most 32 outstanding watches belong to
one manager incarnation. All ordinary spawns remain unchanged.

The reaper saves process generation, actual private user frames and page tables
released, and the PMM free-frame sample after cleanup. It also retains exit
status in the process record. Watched reaped records cannot be reused until the
same manager incarnation releases the receipt. A manager's own reaping drops
its watches, so its death cannot permanently retain those records. No new
scheduler lock is introduced. Existing handoff, active-address-space and shared
mapping reference checks remain responsible for safe physical reclamation.

IDLE_METRICS returns successful unload count and the last receipt's private
frames, page-table frames and free-frame sample. IDLE_ACCOUNT makes an explicit
diagnostic sample of scheduler CPU runtime, mapped user pages, shared mapping
pages and global PMM free frames, requiring kernel-checked administrative
authority even with a passed manager token. The mapped-page sample provides context;
reclamation evidence uses the actual reaper counts and physical free-frame
deltas. Shared memory still mapped by a peer is intentionally excluded from
private reclaim counts. Kernel heap retention, page-table metadata and unrelated
resident allocations can affect global before/after deltas.

D2 stores at most one timer per unit and one per lease (64 entries total).
Insertion/cancellation is bounded and physically removes superseded entries;
idle expiry only examines the earliest entry. Lease expiry, eligibility changes
and shutdown completion schedule work as events. Existing D1 50 ms process
supervision remains; D2 adds no periodic global scan, permanent runnable thread,
new per-message allocation or CPU-usage heuristic. At most one unit deadline
and therefore one cooperative handshake runs per event-loop turn; simultaneous
idle units cannot multiply the handshake budget before manager IPC is serviced.

## Validation and measurements

Run `bash tools/test_sunlightd_d2.sh` for the focused host checks and combined
D1/D2 native 1/2/4 CPU matrix. Raw serial logs, tool versions, source SHA-256
manifest, build logs, activation timings and memory samples are retained in
`target/d2-evidence/`. The original D1 evidence was preserved separately in
`target/d2-evidence/d1-baseline/`. Earlier development failures remain in
`probe-1/` through `probe-6/`; they are not counted as passing matrix runs.

Validated 2026-10-19 with QEMU 11.1.1 TCG, 1024 MiB, one final guest boot per
CPU count. All three final boots pass the unchanged D1 assertions and IPC
foundation expectations, plus D2 assertions for authenticated reporting and
leases, busy/persistent refusal, grace cancellation, actual work overlapping
timeout, activation during quiescing/stopping, Stop/Restart refusal during
shutdown, single-flight concurrent reactivation, client death/lease expiry,
stale drain acknowledgement, quiesce and exit timeouts, crash after idle report,
dependency residency, stale readiness/capabilities and six measured cycles.
Eight actual native client threads join reactivation. A separate native worker
completes a measured 300 ms accepted operation across the idle deadline while
two clients race activation; at least one activation receives Busy. This is
in addition to the host threads racing timeout and activation under the
manager's serialized state ownership.

Host results: 25 core tests (18 D1 + 7 D2), 11 manager tests (9 D1 + 2 D2),
42 IPC tests, 35 kernel memory tests, and the unchanged scheduler host runner
(51 Rust checks and production probes) pass: all original 155 plus 9 new
tests, 164 total. The unchanged Wise Owl identity Phase C configuration also
passes compilation.

Steady-cycle physical free memory, in KiB, from the final sample on each boot:

| CPUs | Before activation | Running | After confirmed unload | Physical delta | Private frames freed | Page-table frames freed |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 74864 | 71716 | 74864 | 3148 | 774 | 12 |
| 2 | 73832 | 70684 | 73832 | 3148 | 774 | 12 |
| 4 | 71784 | 68636 | 71784 | 3148 | 774 | 12 |

Each frame is 4 KiB. The reaper confirms 3096 KiB of private pages and 48 KiB
of page tables freed per service instance. Ordinary measured cycles additionally
release the service's one 4 KiB shared page at its last mapping, explaining the
3148 KiB physical delta. In the explicit peer-sharing cycle the client reads
the shared page successfully after service exit and only then releases its own
mapping. That still-mapped page is not included in private reclaim counts.
The pilot intentionally allocates/touches a 1 MiB private mapping; receipts also
include its actual private heap, stack and executable mappings. These are
measurements of this synthetic process, not projected savings for other services.

| CPUs | First request to spawn / Ready | Steady reactivation to manager Ready | Quiesce to confirmed reap, all successful attempts | Idle service CPU per 300 ms sample | Steady-cycle free-frame range |
| --- | --- | --- | --- | --- | ---: |
| 1 | 10 / 10 ms | 0 ms | 230–690 ms | 0 ns | 0 |
| 2 | 0 / 10 ms | 0–10 ms | 230–680 ms | 0 ns | 0 |
| 4 | 10 / 10 ms | 0–10 ms | 230–690 ms | 0 ns | 0 |

`lifecycle-measurements.json` includes all activation and shutdown observations;
`idle-memory.json` includes all 18 memory/CPU samples. Client-observed Ready
samples in D2-MEM also include subsequent discovery and a generation ping;
the table uses manager request-to-Ready measurements. Zero millisecond samples
fall within one 10 ms clock tick. The pilot deliberately delays exit by 200 ms
to expose Stopping races. The longest shutdown includes accepted work and
bounded crossed-report retries. D1 fixtures on the same boot retain their
separate deliberate 250 ms readiness delay; that field in the inherited
`benchmark-context.json` describes D1, not D2.

Limitations: each CPU configuration has one final boot and six measured cycles,
not a production latency distribution or a long-duration leak proof. The first
shared-view cycle leaves global free memory two frames below its pre-activation
sample; later cycles return to exactly the same free-frame count. Global PMM
deltas include harness mapping metadata and unrelated resident activity, and
cannot assign those first two frames solely to the service. Five subsequent
steady cycles show no physical-frame drift. PMM stability cannot exclude tiny
metadata leaks hidden inside already-reserved kernel heap arenas. Service CPU
samples exclude manager CPU and the unchanged D1 50 ms supervision overhead.
Executable fixtures embedded in the kernel image remain pinned there; unloading
reclaims the process's private copies, not those embedded bytes.
The implementation's fixed deadlines and bounded events establish that D2
introduces no continuous scan; the zero service runtime samples independently
verify that the pilot blocks while idle. Raw reaper counts, generation checks,
stale-capability failures and peer-held mappings provide evidence beyond RSS.

Development gates exposed and corrected stale supervisor failure state during
retry, invalid fixture sleeps before deferring a reply, late completion-token
errors, overly short idle-report deadlines, and test-driver lookup authority.
The protocol review also corrected immediate-reply ordering before maintenance
and bounded unit handshakes to one per event-loop turn. No unresolved critical
lifecycle invariant was bypassed to enable the synthetic pilot.

The ordinary two-CPU resident gate also passes with 20 definitions and no D1/D2
fixtures enabled: `SUNLIGHT_TEST_CPUS=2 bash tools/test.sh sunlightd`. Its raw
serial, complete build output and gate report are retained as
`resident-serial.log`, `resident-build.log` and `resident-gate.log` in the same
evidence directory. The default userspace/kernel artifacts were rebuilt for
this check after the experimental matrix. Final source hashes match the
manifest captured before the native build.

## D3 prerequisites

Before migrating a real service: implement and review its complete admission
barrier, persistent transaction protocol, multipart/session ownership, resource
lease cleanup and all background tasks; validate client retry behavior and
generation handshakes on every endpoint. Thumbnailer's alias, readiness,
multipart state and cache transactions require this work first.

Further prerequisites are crash recovery for the manager, delegated worker
reporting, richer invocation-scoped residency permissions, asynchronous control
delivery for services unable to bound crossed RPCs, cooperative explicit Stop
and Restart, production workload/latency budgets, and longer memory stability
runs. General memory-pressure eviction and migration of critical services are
not enabled by D2.
