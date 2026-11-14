# Native runtime R2B.1: pthread keys, once and error strings

Date: 2026-11-14. Branch `main`; baseline and final Git revision
`f6b5ccb24d3ef2d2077eaada148a9208e3e2c738` (uncommitted implementation).

**Overall: PARTIAL, not R2B.1 COMPLETE.** The new pthread key/destructor,
once and error-string paths pass the bounded native tests below. A demonstrated
pre-existing kernel lifecycle limitation prevents POSIX thread-only exit of the
initial address-space owner. The affected initial-thread lifecycle is left
unchanged; no kernel change or new syscall was made. CPython is not built or
operational. No condition-variable implementation, commit or push.

Evidence root (**E**):
`docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b1/`.
The authoritative acceptance run is `E/final2/`; earlier runs remain separate
attempts. Supplemental final-source evidence is at the root of E.

| Gate | Status | Evidence and boundary |
|---|---|---|
| G1 baseline / locked build | PASS | `initial-source.json`, `baseline-locked-build.log`, `final2/locked-final.log`, `locked-final-repeat.log`. Repeated installed-toolchain build, not clean-room bit reproducibility. |
| G2 pthread TLS and lifecycle | PARTIAL | `final2/acceptance-focused-{1,2,4}/results.json`, `exit-race-guest-{2,4}/results.json`; initial-thread `pthread_exit` terminates borrowers (`final2/owner-exit-observation/serial.log`). No claim of full POSIX lifecycle completion. |
| G3 pthread_once | PASS | Native normal/nested initialization, 12 concurrent callers, publication, repeated fast path, explicit initializer thread exit/retry on 1/2/4 CPUs; bounded recursive noncompletion and independent progress in `recursion-guest-2/`. Exact kernel park/wake race interleavings remain UNQUALIFIED. |
| G4 error strings | PASS | C/C++ base/POSIX/GNU ABI checks and guest consumption under `final2/`; `strerror-threads-guest-{2,4}/` exercises all 40 defined numbers, stable pointers, caller buffers and errno isolation. |
| G5 legacy regressions | PASS | `final2/acceptance-suite/`: 73 C + 21 C++; `final2/cxx-abi-final/`, heap/ABI, TCP loopback and pthread ABI. New cases counted separately. |
| G6 actual CPython probe | PASS | `final2-configure-result.json`, `final2-compile-results.json`; three requested units now compile; condition-variable types stop core compilation. Static interpreter linking and execution NOT_RUN. |
| G7 inventory / integrity | PASS | `source-integrity-manifest.json`, `cpython-compatibility-inventory.json`, `validation-summary.json`. Seven scoped existing libc files change; prior work, vendor trees and kernel sources preserved. |

## Baseline and ownership audit

Read the readiness audit, R1 report, R2A report and R2A validation summary.
`initial-source.json` records the revision, branch, initial status and SHA-256
hashes of tracked and nonignored untracked files before production edits. The
checkout already contained R1/R2A changes to locks, headers, libc modules, libm
vendor sources and reports/evidence. Those are baseline content, not this
milestone's changes. The exact original command `bash tools/build_native_c.sh`
passed before implementation. The same installed native compiler, linker,
archive/driver and pinned std overlay are retained.

Reviewed `sunlight-libc/src/{pthread,tls,thread,errno,sys,termination}.rs`,
`include/{pthread,errno,string,limits}.h`, the native std TLS key overlay, native
futex syscall 155, reaped query 156 and scheduler termination/reaping. The
existing C wrapper uses a heap control block, eagerly mapped 2 MiB stack and
variant-II static TLS image followed by a TCB. Join waits for EXITED then native
kernel Reaped before unmapping stack/TLS and freeing the control block. Detached
cleanup uses the existing bounded lazy queue. Native futex compare/registration
and wake are serialized in the kernel; userspace consumes the WAIT receipt and
rechecks state after wake or mismatch. No changes to those kernel interfaces.

The process-wide registry belongs in libc; values belong in the calling thread's
existing TCB. Rust std already owns 256 pointer slots at FS+32. They must not be
reused for pthread keys. New fields follow that table, preserving self-pointer,
errno, pthread-control and std-key offsets. TCB layout grows from 2080 to 4136
bytes; mappings now accommodate it through the existing `area_size` calculation.
The native static TLS image remains at negative FS offsets. Host layout tests
cover offsets, initial zero values, alignment and separation from static TLS;
these are layout evidence, not guest execution.

## pthread key and destructor design

Public declarations live in `sunlight-libc/include/pthread.h`; limits are in
`include/limits.h`. `pthread_key_t` is native LP64 unsigned long (8 bytes).
`PTHREAD_KEYS_MAX=128`; `PTHREAD_DESTRUCTOR_ITERATIONS=4`.
Implementation is `sunlight-libc/src/pthread.rs`, with private storage/layout
types in `src/tls.rs`. No allocation occurs in create/delete/get/set.

A process-global static 128-slot registry stores liveness, generation and an
optional destructor. Its futex mutex serializes all metadata selection. A key
encodes a 7-bit index and a nonzero generation in the remaining bits. Generations
never wrap: a slot at the representable maximum is retired. Exhaustion returns
EAGAIN. Delete invalidates only the registry entry and never calls a destructor.
Invalid/deleted keys return EINVAL from delete/set and NULL from get, a defensive
extension for invalid-key uses. Pthread routines return errors directly; tests
verify errno preservation. A NULL destructor/value is valid.

Each TCB contains 128 `(key, value)` cells. Get validates both registry generation
and cell identity; set changes only the current TCB. Key reuse therefore cannot
expose an old cell in a surviving thread. The registry and destructor descriptors
are never freed, and deletion never touches another thread's TCB. This removes
registry reclamation use-after-free hazards. The API's thread isolation is not a
memory-security boundary against arbitrary C code in the same address space.

Both normal pthread start-routine return and explicit `pthread_exit` run cleanup
before publishing EXITED. For up to four passes, cleanup clears each selected
cell before invoking its valid non-NULL destructor. The registry lock is released
before every callback. Callbacks may create/delete keys and set values again;
new values get subsequent processing within the iteration bound. All cells are
discarded at the end. Values themselves remain application-owned. Process
`exit`, main return, abort and external process destruction do not run pthread
TSD destructors; `process-exit-guest-2/` demonstrates normal process exit separately.
Rust std's own destructor protocol is unchanged; this is the pthread lifecycle.

Destructor selection and deletion linearize under the registry mutex. Deletion
before selection suppresses that callback. A callback already selected or
running can finish after delete returns; delete does not cancel user code. Its
code must remain live during invocation. `exit-race-guest-{2,4}` holds a callback
open while the parent deletes/reuses its key, then releases it and joins. Another
handshake deletes/reuses a key before worker exit and verifies no callback and no
stale get/set. Callback reentrancy tests would block if an internal registry lock
were held. Concurrent registry churn is additional bounded stress, not proof of
every possible interleaving.

## Lifecycle limitation and resource boundaries

The old initial-thread exit comment was incorrect. Native syscall 20 calls
`finish_current_process`; `kernel/src/sched/mod.rs` terminates address-space
borrowers when the owner exits. The existing initial `pthread_exit` path uses
that syscall. `final2/owner-exit-observation/` prints the initial exit marker,
never prints the worker survival marker, and records borrower termination with
`reason=address-space-owner-exit`. The runner's exit-zero PASS validates the
observation only; **POSIX initial-thread survival is FAIL on the existing native exit path**, which makes G2 PARTIAL. The new cleanup
hook runs the initial thread's own destructors but cannot preserve its peers.
No kernel workaround or new syscall was introduced. A userspace owner-lifetime
registry/wait protocol is a possible follow-up design, not proven impossible;
interoperation with native Rust borrowers and last-thread process teardown must
be reviewed first. This affected existing exit path is left unchanged at the
milestone boundary, rather than claiming its comment establishes correctness.

Spawned pthreads clean their TSD before termination even when detached, without
requiring join or a later create to run callbacks. Their stack, TCB mapping and
heap control block still use the existing deferred reaper; at most 64 detached
entries can be queued, and overflow can retain metadata until process exit.
This pre-existing limitation remains PARTIAL. The tests observe detached
callbacks, repeated creates/joins, and final process physical-frame/resource
reaping. They do not directly measure zero per-thread allocator/mapping retention
before process exit: that stronger resource gate is UNQUALIFIED. No blanket
leak-free or exhaustive concurrent-memory-safety claim is made.

## pthread_once design and qualification

The public object is one naturally aligned unsigned 32-bit word (size/alignment
4); `PTHREAD_ONCE_INIT={0}`. States are 0 uninitialized, 1 initializing and 2
complete. A successful acquire CAS elects the initializer. Completion uses a
release store followed by wake-all. The fast path is one acquire load. Waiters
use existing process-private futex WAIT(expected=1), then retry the state;
compare-before-registration prevents a completion between failed CAS and WAIT
from losing the wake. Spurious wakes are tolerated. No lock is held over the
initializer and no spinning wait substitutes for the futex.

Each initializing call links a stack frame into the current TCB. Normal return
unlinks it before publication. Explicit pthread_exit resets every active nested
control to 0 with release ordering and wakes waiters before running TSD callbacks;
a surviving caller can retry initialization. The frame and control stay live
until that thread's exit cleanup finishes. Callers must keep control storage
aligned, mapped and alive for all concurrent uses. Cancellation is not supplied;
no cancellation handlers, C++ exception unwinding through this Rust C boundary,
longjmp out of an initializer, or fork-repair semantics are advertised. Raw
syscalls/forced process termination bypass userspace cleanup.

Different controls may nest. Recursing on the same control waits on its own
in-progress initialization and cannot complete. `recursion-guest-2` observes no
completion in a bounded interval while an independent thread completes; process
exit ends the deliberately blocked worker and the outer runner has a 90-second
host timeout. This finite observation is not proof of eternal blocking or an
exact kernel parked-state witness. An earlier implementation returned EDEADLK;
that nonstandard extension was removed before the authoritative final runs.

The main fixture launches 12 callers, holds the initializer, verifies a single
entry and no completed callers, releases it, checks published non-atomic data,
then performs 10,000 completed calls per worker. It also parks a live initializer
until a retrying thread has started, exits it via pthread_exit and verifies the
retry's publication and both joins. Started flags and elapsed sleeps do not prove
every waiter reached kernel registration. Exact wake-before-registration,
registered-waiter abandonment, spurious-wake and SMP cross-dispatch schedules are
**UNQUALIFIED**, not asserted PASS by repetition. The functional G3 PASS is
bounded to the demonstrated supported C lifecycle, not a complete concurrency
proof or unsupported cancellation/exception behavior.

## Native execution and regressions

All authoritative guest runs use the actual static native SunlightOS driver,
no Linux-host libc, QEMU TCG with `-cpu max`, 2048 MiB and disposable images.
The runner verifies exit/output contracts, kernel faults and process reaping.

| Test group | 1 CPU | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|---|
| TLS/once lifecycle fixture, 12 callers | PASS | PASS | PASS | NOT_RUN |
| In-flight destructor/delete/reuse | NOT_RUN | PASS | PASS | NOT_RUN |
| All error strings with four threads | NOT_RUN | PASS | PASS | NOT_RUN |
| Recursive once bounded noncompletion | NOT_RUN | PASS (bounded observation) | NOT_RUN | NOT_RUN |
| C/C++ header guests | NOT_RUN | PASS | NOT_RUN | NOT_RUN |
| Existing 73 C + 21 C++ suite | NOT_RUN | PASS | NOT_RUN | NOT_RUN |

`tools/native-c/guest.py` accepts only 1, 2 and 4 CPUs; no unsafe runner expansion
or 12-CPU inference was made. Twelve *caller threads* do not mean twelve CPUs.

The legacy count remains **73 C and 21 C++**, all final-source build/link/guest
PASS in `final2/acceptance-suite/results.csv`. Supplemental `final2/heap-abi-guest`
prints the existing R1 heap/SysV ABI marker; `final2/tcp-loopback/poll-summary.json`
checks loopback transport and zero live socket/listener/stream counters.
`final2/cxx-abi-final/` verifies target exception/unwind/RTTI/static guard exports,
archive ownership, ELF frames and upstream vendor integrity. The original
archive-free C++ LINK_FAIL remains an intentional negative control.
`final2/pthread-abi/` retains the 12 existing C/C++ ABI compile checks and historical
pthread fixture integrity.

New tests are separate: three main-fixture CPU executions, six header compile
profiles, two header guests, two in-flight destructor race executions, two
error-string thread executions, one recursive fixture, one process-exit fixture,
and one owner-exit limitation observation. Host TCB layout tests are three host
checks, not native guest cases. Supplemental output assertions and race details
are also machine-readable in `race-test-results.json`.

## Error-string compatibility

`sunlight-libc/src/string.rs` supplies strong native strerror/strerror_r exports;
`include/string.h` declares them in C linkage for both C and C++.
`include/errno.h` additionally exposes existing pthread EDEADLK=35. No other new
platform error numbers or Linux-host linking are introduced.

`strerror` returns stable immutable C-locale strings for zero and all 39 distinct
advertised nonzero numbers (aliases share their number). Callers must not modify
or free them. Unknown positive/negative numbers return the same immutable
`"Unknown error"`; errno is preserved. Static storage supports concurrent calls
without a shared mutable formatting buffer.

`strerror_r` always uses the POSIX-style `int(int,char*,size_t)` ABI, including
under `_GNU_SOURCE`; there is no GNU char-pointer-returning variant. Success
copies the complete NUL-terminated message and returns zero. Unknown errors copy
the documented message and return EINVAL. Insufficient capacity returns ERANGE;
a nonzero-length valid buffer receives a truncated NUL-terminated string. Zero
capacity returns ERANGE without accessing storage. Null with nonzero capacity
returns EINVAL. All paths preserve errno. C/C++ typed function-pointer checks
and native buffer/error tests establish this selected ABI; no locale-dependent
error text or full POSIX locale API conformance is claimed.

## CPython 3.13.0 continuation

Reused `/tmp/sunlight-r2a-cpython/Python-3.13.0`, R2A's minimized configure/config.sub
recognition patch and `/tmp/sunlight-r1-cpython/build-host/python` (3.13.0).
`cpython-source-provenance.json`, `cpython-source-hashes.json` and
`cpython-platform.patch` preserve identity and modifications. No new CPython
source patch. Archive SHA-256 remains
`086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d`.

Fresh final build directory: `/tmp/sunlight-r2b1-final2-cpython-cross`.
`continue_cpython_final2.py` retains commands/environment and diagnostics. Cross-configure
again succeeds using the real native driver, static linking, matching build
interpreter, R2A triplets/options and the same three justified negative cache
answers (no POSIX semaphores, /dev/ptmx or /dev/ptc). No unsupported threading
feature was forced to yes. Configure success does not validate upstream cross-run
assumptions or optional modules.

Actual `make -j1` results:

| Target | Result |
|---|---|
| Programs/python.o | PASS, previously blocked on pthread_key_t |
| Python/pyctype.o | PASS |
| Python/pymath.o | PASS |
| Python/thread.o | FAIL, first diagnostic unknown pthread_condattr_t; pthread_cond_t and condvar functions also absent |
| Objects/longobject.o | FAIL, first diagnostic unknown pthread_condattr_t |
| python | FAIL at Parser/pegen.c after Parser/token.o compiles; unknown pthread_condattr_t first, plus pthread_cond_t and undeclared strncpy |

`final2-compile-*.log` retains complete compiler commands, errors and warnings.
Time-header visibility/incomplete timespec and struct stat/lconv/tm warnings
remain separate platform/header findings. Prior keyword-initializer diagnostics
were not reproduced in these final attempts; their historical cause is not
asserted resolved. No interpreter linker command was reached, so unresolved
interpreter symbols are UNKNOWN and static linking/execution are NOT_RUN.
`final2-cpython-object-hashes.json` and ELF header logs identify actual emitted
native target objects. No fake symbols, condition-variable stubs or host objects
were substituted.

## Integrity, negative evidence and next milestone

`source-integrity-manifest.json` compares every initial file against its initial
hash; only the authorized seven libc files changed. New production-test files
are under `tools/native-c/tests/`; other additions are this report and evidence.
All pre-existing R1/R2A evidence, vendor numerical/unwind sources, dependency
locks, native driver/target/overlay and kernel/scheduler files are preserved.
The revision remains unchanged because this work is deliberately uncommitted.
Final status, exact source diff and final input fingerprint are retained. Binary
ELFs, generated images and build products are ignored, not proposed source.

Preserved negative attempts include the first fixture's undeclared usleep and
wrong callback type, feature-gating failure from placing shared-TCB types inside
the gated pthread module, an intermediate generated Rust syntax/import failure,
the superseded EDEADLK recursion implementation's test logs, and a C++
negative-control header failure introduced by including limits.h from pthread.h.
The latter was corrected by retaining self-contained pthread declarations and
also providing the limit macros in limits.h. These attempts
are not final-source acceptance. The main suite, supplements and CPython probe
were rerun after final production edits. Initial-owner exit failure is retained
as an unresolved architectural result, not overwritten by ordinary worker exit
successes. Stress does not qualify unobserved race schedules.

Recommended **R2B.2**: obtain separate authorization for an initial-owner
thread-only lifecycle design and measure/rework detached runtime reclamation;
close exact registered-waiter/exit race witnesses. Then implement actual
process-private condition variables with a defined timed-wait/clock/lifetime
contract, resolve demonstrated time-header integration and strncpy, and repeat
this exact CPython core probe. Kernel changes require their own authorization.
Interpreter static linking follows only if real compilation reaches it; native
execution and optional modules remain subsequent gates.
