# T4B — Futex contention, timeout and cancellation hardening

**Status: PARTIAL (Deferred to Signal Subsystem Epoch).** The supported private WAIT/WAKE and timeout lifecycle has
substantial host and native QEMU evidence. Cancellation is only supported as a
kernel-side spurious wake/termination path; user signal-handler interruption,
`EINTR`/restart and pthread cancellation are not implemented. Several requested
controls are production-helper host models rather than independent native race
demonstrations, so they are not promoted to native PASS claims.

## Frozen boundary and next epoch

The November 8, 2026 boundary decision defers signal-dependent futex work;
T4B cannot be promoted to COMPLETE in this development cycle. Reopening that
scope requires these explicit dependencies:

- Kernel userspace signal dispatch, including actual user signal-handler delivery.
- Syscall restart architecture: `restart_syscall`, `ERESTARTSYS` and `SA_RESTART`.
- POSIX pthread cancellation points during wait loops, including `pthread_cancel`.
- Linux timed-wait ABI alignment and timeout mapping.

`EINTR` and `pthread_cancel` remain structurally **UNSUPPORTED**. Linux timed
WAIT remains **UNSUPPORTED**; native timeout receipts do not establish Linux
timed-wait semantics. A generic scheduler wake is a spurious completion, not
userspace signal delivery or a POSIX cancellation point.

Isolation uses a recorded host baseline rather than moving or stashing this
dirty workspace. The fresh boundary receipt is
`.threading-evidence/t4b/20261108T175345.242506Z-deferred-boundary/host-baseline.json`.
`cargo check -p sunlight-kernel --offline`, production-helper T4B host controls,
the four T4B receipt/parser tests and `git diff --check` all passed. The helper
controls retain their VM/placement adapters and UNSUPPORTED cancellation receipt;
these checks do not qualify a native race or cancel the source-integrity failure.
The same directory records the initial status and hashes of all 284 dirty
tracked paths so unrelated changes can be checked for preservation. No branch,
stash, restoration or whole-workspace formatting is required by this isolation.

The implementation focus moves to the bounded interface in
`fastlane-ring/src/data_plane.rs`, with its architecture contract in
`docs/ipc/fastlane-data-plane-draft.md`. That interface has no futex, signal or
pthread cancellation dependency. This transition does not promote existing
FastLane production gates or claim rendering/audio performance.

### Open follow-up and local handoff

The accepted handoff keeps the supported futex behavior and the initial
FastLane data-plane interface. **EINTR and cancellation remain OPEN for a later
Signal Subsystem Epoch**, with no COMPLETE claim or emulated cancellation added:

- [ ] Deliver userspace signal handlers and define interruption of blocked waits.
- [ ] Implement `ERESTARTSYS`, `SA_RESTART` and `restart_syscall` semantics.
- [ ] Implement POSIX pthread cancellation points and `pthread_cancel` in wait loops.
- [ ] Align Linux timed futex WAIT with its timeout ABI.
- [ ] Revisit the remaining native contention controls and obtain source-stable
  evidence before reconsidering T4B completion.

The November 8 local commit handoff includes the complete current workspace,
including existing application changes, metrics tooling, documentation
organization, version metadata and captured T4B evidence. This records those
changes together; the focused validation below does not validate every unrelated
workspace change. The FastLane ring passed 42 host library tests, five doctests,
bare-metal checking, scoped Clippy and formatting. Native application streaming,
aggregate quota enforcement, client-death teardown and audio jitter remain
unvalidated. Existing rejected attempts and repository-wide tooling failures
retain their original status. The handoff is local; no push is requested.

Staging recognizes the 134 previously missing documentation paths as moves into
topic folders. Source/documentation staged whitespace checks pass. The full
staged check reports whitespace in captured `.threading-evidence` logs; those
original logs are retained unchanged. No ISO, ELF or disk images are staged.
No additional runtime tests are performed for this documentation/commit handoff;
the validation results above belong to the preceding implementation step.

## Source identity and preservation

The T4B evidence was created from HEAD
`73cb0b81dff5bd7d6e10d36d82122686cfaf5fa3`. This review is at HEAD
`3002d96dd46eec90f2479062f9af471b17520f11` (November 8, 2026). The checkout
already had 140 status entries before this report was added. The full status and
SHA-256 inventory are in
`.threading-evidence/t4b/20261108T170143Z-current-audit/`; no pre-existing dirty
work was reverted or rewritten.

The accepted November 5 bundles were historical evidence: full-file hashes for
`syscall.rs` and `process/mod.rs` differed from this checkout, so they were not
used as current-source proof. I reran the race pilot and then the complete race
and native std CPU matrices, production build/boot, and Linux smoke from this
HEAD. Each bundle recorded source hashes and passed its source-stability
check. Receipts are under `.threading-evidence/t4b/20261108T170443.484411Z-`
(`races` pilot), `20261108T170730.076339Z-suite-races`,
`20261108T171048.292093Z-suite-std`,
`20261108T171530.807699Z-suite-production` and
`20261108T171717.701054Z-suite-linux`.

After those runs, the focused formatter check exposed three formatting-only
differences in T4B syscall code; those were fixed and focused rustfmt passed.
A subsequent fresh race run reached `[T4B] PASS`, but the runner rejected it
because its source-integrity check detected workspace changes during the run.
The rejected attempt and its guest log are
`.threading-evidence/t4b/20261108T172558.810957Z-suite-races/` and
`.threading-evidence/t4b/20261108T172638.884620Z-races-2cpu/serial.log`.
At review time, 134 tracked documentation files listed in the passing race
bundle's full source manifest appear as deletions; 5,402 existing file hashes
also differ from that snapshot. Their original hashes and the final status are recorded under
`.threading-evidence/t4b/20261108T170143Z-current-audit/`. I did not restore or
rewrite those files. Therefore, the full fresh matrices above apply to the
pre-format source snapshot; the post-format pilot is NOT accepted as
source-stable evidence. The historical T4A report and evidence remain
untouched.

Existing T4A evidence was reviewed in `docs/kernel/threading-t4a-atomic-futex.md`.
It establishes atomic aligned private-word reads and registration under the
existing `SCHEDULER` lock, correct retirement, task-local native timeout
receipts, wake cardinality, and 1/2/4/12 CPU native std synchronization. T4A
does not establish full Linux futex compatibility.

## Supported operations and state machine

The kernel exposes native private WAIT, WAKE and per-thread RESULT operations.
Linux compatibility accepts same-VM/same-VA WAIT and WAKE (private flag
optional), but Linux timed WAIT is unsupported. Shared backing, PI, REQUEUE,
WAIT_BITSET, robust owner death and cross-ABI futexes are outside the contract.
The T4B host-control receipt explicitly records `EINTR`/`pthread_cancel` as
UNSUPPORTED.

The registration is stored in the existing process/thread slot, keyed by VM
identity, address and ABI namespace. The existing scheduler lock serializes
comparison, registration, timeout expiry, wake, generic cancellation, mapping
retirement and termination. There is no separately allocated waiter object or
second lifecycle lock.

| State | Event winning under scheduler serialization | Effect |
| --- | --- | --- |
| Running, unregistered | mismatch, invalid storage or already-expired native deadline | Return immediately; no waiter is published. |
| Running, unregistered | matching WAIT | Publish one address/deadline and `BlockedOnIo` before releasing the scheduler lock. |
| Registered and blocked | matching WAKE | Detach once, preserve native result 0, transition to Ready and apply existing owner/queue placement rules. |
| Registered and blocked | native timeout | Detach once, preserve native result 2, transition to Ready. A later wake selects zero. |
| Registered and blocked | supported generic signal/cancel wake or committed mapping retirement | Detach once and resume as a spurious completion (native result 0). This does not implement user signal delivery or EINTR. |
| Registered and blocked | thread/process termination | Clear registration and receipt; do not make the finished thread runnable. Reaping follows normal lifecycle rules. |
| Completed native WAIT | RESULT for matching address | Consume the task-local terminal receipt once; wrong or repeated RESULT is rejected. |

`Option::take` on the registration owns completion. This prevents duplicate
terminal results and prevents later generic wake bookkeeping from replacing a
timeout or successful wake result. Generic wake paths remove futex metadata so
an old registration cannot complete a later unrelated I/O wait. VM unmap and
fixed replacement cancel registrations only after the mapping transaction
commits. Thread identity checks include PID, incarnation and VM generation.

## Race controls

The host lifecycle controls compile the production helper logic with placement
and VM/scheduler adapters. Their own log labels each control “production
helpers; placement stub”; those are useful logic checks, not proof of kernel
interleavings. `C11` and delayed-IPI admission checks use syscall helpers with
stub adapters. The native T4B fixture exercises actual ring-3 waiters, scheduler
transitions and wake/timeout/termination paths under QEMU; the refreshed
pre-format 2-CPU log is
`.threading-evidence/t4b/20261108T170826.906625Z-races-2cpu/serial.log`.

| Control | Evidence and result | Limit |
| --- | --- | --- |
| C1 wake before timeout | Host helper PASS; native wake case PASS. | Native fixture shows the terminal result; it is not a user-level timed-wait API test. |
| C2 timeout before wake | Host helper PASS; native timeout then later wake selects zero. | Native result receipt is native ABI behavior. |
| C3 wake/timeout contention | Host mutex/barrier model PASS; native blocked-waiter expiry/wake paths run on 2/4/12 CPUs. | No separate deterministic native instruction-level tie at the terminal lock was recorded. |
| C4 cancellation before wake | Host helper PASS; native scheduler signal-wake then later WAKE selects zero. | User signal handler and EINTR/restart remain unsupported. |
| C5 wake before cancellation | Host helper PASS; native wake receipt survives later cancellation. | No userspace signal delivery claim. |
| C6 cancellation/timeout contention | Host helper PASS. | Native simultaneous cancellation/timeout winner not independently demonstrated. |
| C7 exit with registered waiter | Native termination case exits with code 37, does not resume, and is reaped; inventories return to zero. | This is terminal exit, not recoverable cancellation. |
| C8 wake selection during migration | Native fixture migrates a quiescent waiter and checks same TID, incarnation, TLS, stack/TSS and CPU dispatch. | The code intentionally migrates only after the waiter is quiescent; selection concurrent with migration is not separately forced. |
| C9 concurrent wake(1) | Native 32-waiter waves and contending wakers remain duplicate-free; bounded selection confirmed. | No FIFO guarantee. |
| C10 repeated timeout/retry | Native 16 repeated timeout receipts on the same thread; registration and terminal counts reconcile. | Bounded to the fixture rounds. |
| C11 word mutation before registration | T4A native mismatch-before-registration scenario and T4B production-syscall host adapter both show no registration on mismatch. | The T4B adapter uses VM/trap stubs; refer to T4A native evidence for the real guest path. |
| C12 spurious scheduler wake | Host helper PASS; native generic wake/termination checks preserve the specified spurious result or discard on exit. | No claim that a user cancellation API exists. |

The control test parser also has synthetic positive/negative transcripts. These
validate receipt rejection and parsing only; they are not counted as race or
native evidence. The accepted 2-CPU fixture reports `PASS cpus=2 cases=11` and
checks exact actor TID/incarnation, terminal-result counts, queue occurrences,
deadlines, receipts, pins, references, stack handoffs and independent progress.

## Wake cardinality and CPU matrix

The refreshed native race bundle
`.threading-evidence/t4b/20261108T170730.076339Z-suite-races/` passed on 1, 2,
4 and 12 CPUs. T4A's 32-waiter controls remain source-compatible and show the
selection-count behavior for both ABI namespaces on 2/4/12 CPUs. T4B's native
2-CPU 32-waiter test additionally runs concurrent wake calls. It observes
`wake(0)=0`, `wake(1)` selecting one eligible waiter, bounded `wake(4)`, a
broadcast selecting the remaining eligible waiters, and repeated wakes
selecting zero after retirement. It also partitions waiters across independent
addresses. Selected counts are recorded separately from user-space execution
after scheduling; the fixture holds resumed workers behind a release gate.

Native std synchronization plus Hayro threading compatibility passed on 1, 2,
4 and 12 CPUs in
`.threading-evidence/t4b/20261108T171048.292093Z-suite-std/`. It includes
contended mutexes, condvar notify/notify_all, timed condvar, repeated
producer/consumer handoffs, thread join/detach/capacity checks and independent
worker progress. The real ring-3 Linux-thread smoke passed on 2 CPUs; Linux
compatibility host tests passed. These runs do not add shared-futex support.

## Resource reconciliation

At each raw fixture case the native gate verifies no active futex address,
deadline, result receipt, ready-queue occurrence, stack pin or handoff waiter;
thread references return to the owner baseline, and every actor is reaped with
its recorded incarnation. It checks free frame count against the post-mapping
baseline. Heap accounting distinguishes reusable per-slot FD-table caches from
active references: the expected cached table capacity is accounted for and
remains stable. Native std handoff rounds return to their warmed live-allocation
and backing-byte baseline. These are fixture-owned scopes, not a global kernel
allocator census.

## Compatibility, production and tooling

The refreshed Linux and production bundles passed. Debug and release
kernel builds, ABI checks, ordinary 2-CPU production boot, Linux thread smoke,
and the test-only instrumentation exclusion were recorded.
Production exclusion confirms T4B fixture strings/modules are absent from the
ordinary image. Production FastLane status was not changed.

Host kernel tests, Linux compatibility tests, scheduler host tests, T4B helper
controls and historical threading receipt tests passed in the refreshed race
bundle. Focused rustfmt checks passed for the T4B fixture and the T4B syscall
and process spans. Whole-file rustfmt checks for `syscall.rs` report unrelated
pre-existing formatting differences elsewhere in that large file. Direct
kernel Clippy exits 1 with 161 diagnostics; none points into `t4b_fixture.rs`.
The ordinary `cargo clippy` route also exits 101 while the native std/SVG build
script compiles the custom std, before a useful kernel Clippy result. Clippy is
therefore **NOT PASS** for the repository. Direct output is in
`.threading-evidence/t4b/20261108T172200Z-clippy-current-formatted/`; an earlier
current-checkout attempt is in `20261108T171900Z-clippy-current/`. Earlier
baseline comparisons are historical and are not applied to this HEAD. No
unrelated lint cleanup was folded into this task.

## Remaining scope

T4B remains PARTIAL until cancellation/interruption semantics are deliberately
implemented and validated, the remaining contention controls are either
demonstrated as native interleavings or accepted as host/helper-only coverage,
and a final source-stable pilot follows the formatting-only source update.
In particular, `EINTR`/restart, pthread cancellation, user signal handler
execution, a native cancellation-timeout tie, and migration concurrent with
wake selection are not claimed. T5 robust-futex owner death, T6 waitpid
retention and T7 stack guard mappings were not implemented. No test failure,
source-integrity rejection or NOT_RUN result is reclassified as a pass.
