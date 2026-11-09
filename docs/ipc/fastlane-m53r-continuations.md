# FastLane M5.3R: private-stack continuation qualification

Status: **BLOCKED for complete milestone acceptance; implementation PARTIAL**.
Production FastLane and ordinary restricted admission remain **DISABLED**.
Evidence: `.fastlane-evidence/m53r-20261108T234821Z/REPORT.md` and `validation-summary.json`.
Recorded dates are UTC in directory names; the work is November 9, 2026 in
Asia/Tehran. No commit or push was made.

## Established cause and baseline

The original M1 failure is a **Fixture or assertion defect**, present before
M5.3. Two fresh unmodified current-tree attempts and two fresh pristine
`7a2b4f1d7c3c43f149489866a3f8abbff29eddff` baseline attempts reproduce the same
`kernel/src/capability/channel_race.rs:165` assertion on two CPUs. The isolated
baseline matches the recorded clean pre-M5.3 hashes. Selected service binaries,
features, CPU count, fixture selection and QEMU arguments match. Source changes
between these two builds are exactly the M5.3 inputs recorded in evidence.
The requested service-side `channel_race.rs` path does not exist; the helper is
kernel-side and is driven by the service's independent native actors.

Ordinary syscall entry already selects each task's private stack. The old debug
yield helper assumed that stack was unused and tried to pivot back to its top.
Diagnostic-only observations place each caller 1,192 bytes below its own stack
top, Running on its matching owner CPU, unqueued and with IF clear. Emitted
instructions prove that the proposed pivot's first push would alias the saved
user RSP slot of the live syscall dispatcher. The guard stops before that write:
these traces do not demonstrate actual stack corruption or duplicated scheduler
ownership. Failure is inside M1 race coordination, after earlier authority
operations but before the raced operation completes or any M2 mapping workload.

## Repair and ownership argument

The debug helper now retains the live stack and ordinary return chain. The
assertion remains fatal and checks the owned private stack and reserved top
area; there is no fallback pivot or silent recovery. Before yielding, the
existing scheduler lock validates allocation identity, process incarnation,
VM generation, Running state, CPU ownership and absence from a ready queue.
The scheduler still performs its existing Ready publication and reschedule.
RBX carries only the trusted stack top across the interrupt. Return checks its
range and IF without acquiring the lifecycle lock before a real contention
probe. The caller checks unchanged RSP/top; exact actor identity is checked at
the original winning/contending lifecycle-lock acquisition.

No scheduler policy, stack allocation/reaping algorithm, authority path,
production hot path, retry bound, deadline or timing delay was changed.
Scheduler publication retains a physical outgoing-stack pin until the naked
interrupt epilogue's final pivot and release. Remote dispatch/reaping observe
that pin; current/queued/pinned tasks cannot be reclaimed. Exec retains the
kernel allocation while replacing the VM and does not return into the old
image. These source arguments and focused host tests complement the native
M1 interleavings; they do not certify every forced-cancellation or migration
interleaving.

An initial diagnostic implementation acquired SCHEDULER before a contender's
probe and made all 16 mode-1 contention attempts miss. Both failed attempts and
interrupted runs remain preserved. Final observations use immutable scalars and
the original lock points. The contention requirement and bounded attempt count
were retained. Passing final replays apply to the final source only.

## Qualified evidence and remaining limits

The emitted ABI regression rejects the affected image and accepts the repaired
image, including negative instruction controls. Native M1/M2 runs pass on
2/4/12 CPUs. They exercise both real winner orders for attachment/revoke and
attachment/exit, independent authenticated authorization, foreign/duplicate/
stale/terminal refusal, permission faults, exec retirement, row reuse, mapping
cleanup, physical release and remote invalidation/reuse ordering. Those M1
concurrency races do not carry restricted backing. Native one-CPU M1 is
NOT_RUN because that fixture explicitly requires different CPUs.

Debug census images additionally assert per-backing process/session refunds
only after PMM release and expose final Account quota/receipt inventories.
The receipts join to the exercised restricted backing. Global allocator
changes caused by unrelated persistent services remain diagnostic.
Kernel quota prerequisites separately pass on 1/2/4/12 CPUs, with zero frame
and quota drift and 0/1/3/11 remote acknowledgements. Their assigned PTY tuples
are fixture inputs, not proof of authenticated login membership.

Complete M5.3R acceptance remains blocked. The scheduled legacy quota-specific
normal exit/kill/crash/exec and surviving-owner/peer matrix, mapped-backing
close/revoke/exit races, native PID-incarnation reuse and independent live
session ceilings remain unvalidated. Restricted fault/exec/reclamation results
must not be relabeled as that complete legacy lifecycle matrix.

## Session enforcement boundary

The 512 MiB process ceiling and 1 GiB session ceiling remain unchanged. Trusted
PTY binding derives broker incarnation in kernel and validates the exact target
incarnation. Later children/workers inherit the bound tuple; exec and backing
receipts retain their existing identity. Workers created before binding are not
updated. General Account/desktop and service-launched processes lack a complete
authenticated quota-membership record and retain the process fallback. Restricted
backing uses the authenticated channel row's immutable Account SessionRef.

No session assignment based on UID, user-supplied identifiers, bearer possession
or unauthenticated ancestry was added. Completing membership requires a trusted
admission/binding contract, handling outstanding backing at membership changes,
and native independent-session and termination validation. Current restricted
session refund receipts do not establish complete login-wide enforcement.

Next milestone: **M5.3R2 — authenticated membership contract and scheduled
quota lifecycle closure**, with the unexecuted cases kept explicitly blocked
and production disabled. See the machine summary for per-run results, preserved
failures, exact images/source inputs and wider security findings.
