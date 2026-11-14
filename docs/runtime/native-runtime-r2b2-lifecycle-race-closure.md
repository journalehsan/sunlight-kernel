# Native runtime R2B.2: lifecycle and race closure

Date: 2026-11-14. Revision `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`, branch `main`.

**Overall: PARTIAL; R2B.2 is not COMPLETE.** Initial-thread exit is BLOCKED by
native kernel owner/group semantics. No production repair was applied. Independent
userspace probes add actual detached resource accounting and controlled TLS
callback/delete/reuse witnesses. Missing precise registry and futex registration
schedules remain unqualified. No kernel modification, condvar implementation,
commit or push. CPython is not operational.

Evidence root (**E**): `docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b2/`.
Every result below is fresh unless explicitly described as historical.

| Gate | Status | Evidence and limit |
|---|---|---|
| G1 reproducible baseline / ownership | PASS | `source-baseline.json`, `baseline-locked-build.log`, `lifecycle-ownership.md`; retained installed-toolchain locked build, not clean-room bit reproducibility |
| G2 initial pthread_exit preserves workers | BLOCKED | `initial-thread-exit-trace.json`; source-backed failure reproduced on 1/2/4 CPUs; kernel changes prohibited |
| G3 detached bounded steady state | PARTIAL | `detached-reclamation.json`; measured 12-entry cache over four waves and explicit sweep; no autonomous completion, global overflow bound unqualified |
| G4 TLS lifecycle race closure | PARTIAL | `tls-race-witnesses.json`; callback/delete/reuse and stale-cell witnesses qualified; exact create/delete/get/set internal overlap unqualified |
| G5 once/futex race closure | UNQUALIFIED | `once-futex-race-witnesses.json`; functional native pass does not establish registration-point overlaps |
| G6 native regressions | PASS | `regression-results.json`, `regression-suite/results.csv`; individual outcomes preserved |
| G7 integrity / compatibility | PASS | `source-integrity.json`, `cpython-compatibility.json`; every baseline file preserved |

## Baseline and source boundaries

Read R1, R2A and R2B.1 reports, R2B.1 validation summary and its validation commands.
The checkout already contained lockfile, libc header/module, vendor, fixture and
historical evidence changes. `source-baseline.json` records initial revision,
branch, status and hashes of regular tracked/nonignored untracked files.
Directory/dangling symlinks are verified separately against tracked Git blobs;
`integrity-classification-correction.md` preserves the initial classifier error
and its correction. Those changes
were preserved. The exact original `bash tools/build_native_c.sh` passed before
new probes and again after the regression runs. No build dependency or runner
change. Build/kernel/ISO/guest logs and kernel SHA-256 remain in each run folder.
The source fingerprint in JSON identifies the unchanged runtime input snapshot;
it is not a clean-room artifact reproducibility claim.

The source-backed ownership graph and detailed classification were written in
`lifecycle-ownership.md` before any proposed repair. No production edit followed.
The native C driver, static native archive, existing native kernel runner, QEMU
TCG `-cpu max`, 2048 MiB, and disposable guest images were used throughout.

## Thread and process termination

`sunlight-libc/src/std_runtime.rs:35` calls `main`, then `termination::exit`.
`termination.rs:81` runs atexit callbacks and `sl_std_exit` invokes syscall 157.
`kernel/src/arch/x86_64/syscall.rs:3152` implements native exit-group by matching
VM identity, terminating sibling tasks, then exiting the caller.

`sunlight-libc/src/pthread.rs:305` cleans thread-local once/TLS state and invokes
syscall 20. Worker return runs `finish`, then `thread_start` invokes the same
syscall (`thread.rs:65`). This exits a native borrower alone. The initial thread
is a static opaque sentinel without a heap Control and owns the address space.
`kernel/src/sched/mod.rs:4579` explicitly terminates borrowers for native owner
exit. It also closes owner IPC and publishes process completion; removing the
borrower-kill branch alone would leave other ownership obligations unresolved.

The initial-exit fixture proves worker creation, an independently emitted alive
marker, and entry into initial pthread_exit via that thread's TLS destructor.
All three CPU runs show borrower termination with
`reason=address-space-owner-exit`, no worker survival marker, and no worker TLS
cleanup marker. The observation runner exits successfully because exit zero was
expected; **the pthread lifecycle contract FAILS**. The post-cleanup observation
window is not asserted as a deterministic post-syscall barrier. The kernel trace,
not missing output alone, establishes the defect. Identities and traces are in
`initial-thread-exit-trace.json`.

This is a native ownership/process-group contract defect exposed by libc, not a
runner limitation. A userspace parked-owner shim would retain kernel task/session,
IPC and descriptor ownership and would not cover arbitrary Rust native borrowers.
There is no reviewed native thread-only owner-exit interface establishing the
required contract. The affected repair is therefore BLOCKED under the no-kernel-
changes rule. A final surviving-worker-only process cannot currently be qualified.

`_Exit` is actually absent from stdlib.h and native exports, an additional libc
gap rather than a demonstrated working API. `exit-api-probe.log` preserves the
fresh undeclared-function compile failure; `exit-link-probe.log` separately proves the undefined export after an explicit declaration. The callback-free `sl_std_exit` is
process-wide, but it is not an exported `_Exit`. No new API was silently substituted.
Returning from main and explicit exit retain group termination; worker pthread
exit/return retain per-borrower termination and supported cleanup.

## Blocked kernel patch proposal

A separately authorized milestone should introduce native thread-group lifetime
independent of the initial schedulable task. Preserve explicit syscall 157 group
termination. Thread-only exit must retain group-owned VM/resources until every
borrower is Reaped and outgoing execution pins are released; publish process
completion only after final-thread retirement. Reuse identity/generation and
existing deferred-retirement rules. Audit parent waiting/reaping, session/PTY and
foreground group ownership, IPC endpoint close, quota membership and capability
revocation. Native survivor progress, final-thread teardown and session/PTY/IPC
regressions must qualify that change. No such patch was applied here.

## Detached accounting and negative attempts

C Control owns native TID, return/state metadata, the 2 MiB stack extent and
static TLS/TCB extent. TLS values remain application-owned; callbacks run before
EXITED. The kernel owns task/futex state and outgoing-stack pins; query 156 is
the userspace exit acknowledgment. Joining waits for Reaped before release.
Detaching queues the Control after cleanup. Only subsequent pthread_create
sweeps the C queue; pthread_join does not, contrary to the old module comment.
The separate Rust std 256-slot deferred queue cannot reclaim C Control objects
without a resource-specific adapter and also needs later API activity.

The first attempt launched 72 gated detached workers to avoid intervening sweeps.
All CPU configurations admitted 32 before creation failed and main returned 2.
The gate was never released and no intended overflow schedule occurred. Exact
admission cause was not established. These failed runs and original source are
preserved as `detached-churn-cpu-*`; no quota, capacity or timeout was increased.
They do not validate overflow behavior or TLS completion.

The accounting fixture uses 12 detached workers in each of four waves. It binds
a test linker alias to the unchanged runtime's private DETACHED symbol; the final
ELF symbol address is verified by `build-accounting.py` and `accounting-build.json`.
The C mirror of repr(C) Control is 72 bytes. An initial incorrect 80-byte assertion
failed compilation; `accounting-layout-failed-attempt.log` preserves that negative
attempt. No library binary or source was patched. The ordinary static native
archive is linked. This is a deliberately white-box test, not a public libc API.

No queue writer runs during snapshots: workers are either held at the entry gate
or every TID has returned Reaped from syscall 156; only the initial thread can
create/sweep. Snapshots print actual queue cardinality and allocation extents.
Each wave records callbacks=12 and kernel_reaped=12, then waits another bounded
100 one-millisecond sleeps. Clock granularity means this is a requested quiescence
window, not a precisely measured 100 ms wall duration.

| CPUs | Wave | Snapshot | Queued controls | Queued control bytes | Queued stack bytes | Queued TLS bytes |
|---|---|---|---|---|---|---|
| 1 | 0 | baseline | 0 | 0 | 0 | 0 |
| 1 | 1 | live-workers-gated | 0 | 0 | 0 | 0 |
| 1 | 1 | all-kernel-reaped | 12 | 864 | 25165824 | 98304 |
| 1 | 1 | bounded-quiescence | 12 | 864 | 25165824 | 98304 |
| 1 | 4 | live-workers-gated | 0 | 0 | 0 | 0 |
| 1 | 4 | all-kernel-reaped | 12 | 864 | 25165824 | 98304 |
| 1 | 4 | bounded-quiescence | 12 | 864 | 25165824 | 98304 |
| 1 | 5 | explicit-create-sweep-and-join | 0 | 0 | 0 | 0 |
| 2 | 0 | baseline | 0 | 0 | 0 | 0 |
| 2 | 1 | live-workers-gated | 0 | 0 | 0 | 0 |
| 2 | 1 | all-kernel-reaped | 12 | 864 | 25165824 | 98304 |
| 2 | 1 | bounded-quiescence | 12 | 864 | 25165824 | 98304 |
| 2 | 4 | live-workers-gated | 0 | 0 | 0 | 0 |
| 2 | 4 | all-kernel-reaped | 12 | 864 | 25165824 | 98304 |
| 2 | 4 | bounded-quiescence | 12 | 864 | 25165824 | 98304 |
| 2 | 5 | explicit-create-sweep-and-join | 0 | 0 | 0 | 0 |
| 4 | 0 | baseline | 0 | 0 | 0 | 0 |
| 4 | 1 | live-workers-gated | 0 | 0 | 0 | 0 |
| 4 | 1 | all-kernel-reaped | 12 | 864 | 25165824 | 98304 |
| 4 | 1 | bounded-quiescence | 12 | 864 | 25165824 | 98304 |
| 4 | 4 | live-workers-gated | 0 | 0 | 0 | 0 |
| 4 | 4 | all-kernel-reaped | 12 | 864 | 25165824 | 98304 |
| 4 | 4 | bounded-quiescence | 12 | 864 | 25165824 | 98304 |
| 4 | 5 | explicit-create-sweep-and-join | 0 | 0 | 0 | 0 |

During each gated snapshot, 12 successful creates and ready==12 establish 12 live
Control/stack/TCB allocations. Queue zero at that point describes only the deferred
queue, not all live resources. Full wave-by-wave counts and native logs are in
`detached-reclamation.json`.
The same 12-entry retention repeats; a final explicit create/sweep and joined
sentinel reaches queue zero. This proves bounded retention for the tested waves
and reclaimability when another create occurs. It does not prove autonomous
reclamation or a global architectural cache bound. Stack/TLS bytes are retained
allocation extents read from Control, not independently enumerated PTEs. The
sweep calls munmap, but no per-mapping unmap acknowledgment was instrumented.
Final process teardown is separately checked by the runner's owned-frame/FD/stack
resource checks.

`pthread.rs:92` silently drops the cleanup reference when 64 queue entries are
occupied; those resources survive until process exit. The 64 slots therefore do
not establish a bound on retained allocations. Runtime overflow remains source-
confirmed but its native schedule was not reached. A follow-up needs detach
admission before publication and an explicit cleanup owner with exit acknowledgment.
There is also a source-only observer risk: mark_detached changes 0->1 and then
reads Control without holding the queue lock. A worker could enqueue/exit while
that caller is descheduled, allowing a third creator to sweep before the caller
resumes. No native witness forced that interval; it remains UNQUALIFIED, not an
observed use-after-free. It requires a controlled test and protected observer
ownership before concurrent detach/reclamation can be qualified.
Do not enlarge the queue, free a running stack, or add unbounded polling.

## TLS witnesses

`tls-controlled.c` runs on 1/2/4 CPUs with bounded handshakes. T1 prints holder
pthread identity, key index/generation and populated stale cell, then blocks.
T2 deletes/reuses the same slot with a new generation. T3 proves old get is NULL,
old set returns EINVAL, and the reused key does not expose the retained cell.
This is a qualified stale-cell/reuse ordering witness; it is not an API-internal
get/delete overlap witness.

A second worker explicitly exits with a non-NULL TLS value. Its callback proves
the cell was cleared and reenters create/set/get/delete before printing T4 and
blocking. The initial thread deletes/reuses its key at T5 while the callback is
held; release permits T6 completion. T7 records two joins, one callback and zero
live test keys. This qualifies callback/delete/reuse overlap, reentrancy without
registry lock retention, and callback completion before worker Reaped/join.
The original in-flight destructor fixture is also rerun on every CPU count.
The two worker identities reuse the same public Control pointer in these logs
after the first join, while their kernel TIDs are distinct. Public pthread_t is
untagged; stale-handle alias rejection is not supplied. No stale-handle operation
was attempted. TLS key generations and kernel TID monotonicity must not be
misrepresented as generation protection for public pthread handles.

The functional TLS fixture revalidates 128 generation-tagged keys, exhaustion,
isolation, four destructor passes, stale-generation rejection, nested callback
operations, explicit exit, detach and churn. Static registry slots are never freed;
TCB values belong to their own thread. Generations never wrap. Exact concurrent
create/delete, delete/get, delete/set and detached-registry internal schedules
remain UNQUALIFIED. Initial/final-thread schedules are BLOCKED by G2. G4 remains
PARTIAL; starting competitors is not accepted as proof of overlap.

## Once and futex qualification

`pthread.rs:550` uses states 0/1/2, Acquire election CAS, Release completed store
and wake-all, and Acquire reads on return. It holds no lock during user init.
Stack-linked TCB frames reset abandoned nested initializers to 0 with Release
ordering and wake contenders during supported pthread_exit. Kernel futex compare,
registration and wake are serialized (`syscall.rs:3040-3135`); userspace consumes
WAIT results and retries. Recursion on the same control cannot complete itself;
this source conclusion is distinct from a precise kernel wait-state witness.

Fresh focused runs on 1/2/4 CPUs pass concurrent caller publication, initialization
count checks, nested controls, initializer pthread_exit/retry and repeated completed
fast paths. Their start flags and test sleeps do not show where each contender
was parked. Completion immediately before/during registration, registered multiple
waiters, abandonment with registered waiters, retry across registration and rapid
transition overlap remain UNQUALIFIED. Concurrent thread termination under once
contention is NOT_RUN. No exhaustive lost-wakeup or memory-safety claim follows.
Existing debug-only T4A kernel hooks control a separate registered raw-futex actor
fixture; they were inspected but not substituted for once-integrated guest evidence.
No new kernel futex behavior or production sleeps were introduced. G5 is UNQUALIFIED.

## CPU and regression results

1/2/4 CPUs: initial-exit observation reproduced the failure; focused TLS/once,
original callback/delete race, controlled TLS and detached accounting results are
recorded individually in `cpu-matrix.json`. Each guest folder preserves ELF/kernel
identity, serial log, exit/output checks and physical process-reaping observations.
12 CPUs: NOT_RUN because the unchanged native C runner accepts only 1/2/4.

Fresh legacy suite: C 73/73,
C++ 21/21. `regression-results.json`
retains every case, including expected nonzero exits. Static initialization and
exception guest tests remain individual suite cells. Supplemental heap/ABI,
TCP loopback, pthread ABI and target C++ ABI/vendor checks: respectively
PASS, PASS, PASS, PASS.
Libm files were freshly compared with the immutable R2A manifest. Historical
execution results were not reused; only existing fixture language labels were
used to classify the fresh 94 CSV rows. No failure is hidden behind aggregate
counts. New probes are outside the 73 C/21 C++ legacy counts.

## Compatibility, integrity and next milestone

CPython 3.13.0 configure/compile was not rerun: no production input changed and
this milestone does not advance the port. R2B.1 configure PASS and core compile
FAIL are historical evidence explicitly labeled in `cpython-compatibility.json`.
Remaining condition-variable types/functions, strncpy and time/header integration
remain blockers; `_Exit` absence is freshly demonstrated here. Generation-tagged
TLS and supported once behavior remain available with the lifecycle/race limits
above. No fake symbols, artificial stubs or host substitution.

`source-integrity.json` compares every hashed baseline file, verifies tracked
symlink targets against Git blobs, and records final Git state.
Production files modified: **none**. All previous report/evidence and source files
remain unchanged. Added files are this report and E's probes, runners, logs and
JSON evidence. No commit or push occurred.

Recommended next step: separately authorize a native group-lifetime/thread-only
exit milestone, detached admission/reclamation ownership and deterministic
once-integrated registration witnesses. Only after G2-G5 close should the Condvar
milestone design wait-registration/unlock atomicity, mutex reacquisition,
signal/broadcast selection, deadline/clock semantics, destruction with waiters,
and cancellation boundaries. Reuse existing futex semantics and require controlled
native races on 1/2/4 CPUs before any CPython integration.
