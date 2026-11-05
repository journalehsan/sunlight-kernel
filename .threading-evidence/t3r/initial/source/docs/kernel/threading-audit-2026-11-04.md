# SunlightOS threading audit

Date: 2026-11-04. Base commit: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.

## Assessment

SunlightOS implements real SMP threads, shared address spaces, per-thread FS/FP
state, native futex synchronization, and Linux-compatible thread groups. The
scheduler has substantial protection against premature context reuse and stack
reclamation. Threading is nevertheless **not ready to be treated as a hardened
boundary for untrusted applications**: ordinary syscalls execute on user-writable
stacks, and thread creation can exhaust the kernel heap through infallible
allocations. Resolve those issues before prioritizing scheduler policy changes.

This audit reviews the **current working tree**, including pre-existing
uncommitted scheduler, syscall, interrupt, process, and IPC changes. The commit
above alone does not reproduce it. Source fingerprints and validation artifacts
are retained locally under `target/threading-audit-2026-11-04/`; generated
artifacts are not part of this report's source-control deliverable.

No implementation changes were made for this audit. Severity describes impact;
source-confirmed findings below are not claims that an exploit was executed.

## Scope and method

Reviewed creation, scheduling, blocking/waking, thread-local state, exit/join,
reaping, and associated test coverage across:

- `kernel/src/sched/`, especially the RR dispatcher, handoff pins, reschedule
  requests, quantum accounting, and resource reclamation.
- `kernel/src/process/mod.rs`, user-memory copy helpers, heap allocation, and
  x86-64 syscall/interrupt/SMP context handling.
- `sunlight-libc/src/thread.rs`, `std_runtime.rs`, and the native std thread,
  futex, and key-based TLS overlays.
- Linux clone flags, clone/futex/clear-child-TID handling, and robust-list
  registration.
- Existing scheduler host/lifecycle tests, native FP gate, and Linux thread
  probe. Historical reports were checked against current code, not assumed
  current.

This is a focused engineering audit, not a formal memory-model proof or a
complete review of every driver lock, syscall, memory-management race, or std
dependency. Guest validation covers an isolated lifecycle workload, not a full
desktop or adversarial pthread stress suite.

## Verified architecture and protections

| Area | Current implementation |
| --- | --- |
| Task representation | Threads use `Process` records. `sched/thread.rs` is a compatibility re-export. Each thread has its own 32 KiB kernel stack, register frame, FS base, FP state, scheduling state, and PID/TID. |
| Native creation | Syscall 22 creates a borrower of the caller's address space with a copied descriptor table. Syscall 158 additionally opts into shared descriptors and cwd through `native_shared_owner`. These are distinct resource-sharing contracts. |
| Linux creation | The supported `clone` subset requires shared VM/files/FS/signal dispositions and a thread group. It preserves the child's register state, optionally installs TLS and parent TID, inherits affinity, and records child-TID clearing. Unsupported process-copy flags are rejected. |
| Scheduler | Compile-time default is RR, with per-CPU FIFOs and work stealing, but a single global `SCHEDULER` lock still serializes process-state and dispatch operations. Physical per-CPU queues do not imply independently concurrent schedulers. |
| Quantum | Adaptive base quanta are 5–50 ticks, nominally 50–500 ms at 100 Hz. Frozen slice entitlement survives a higher-priority wake interruption; equal/lower-priority wakes need not rotate an unexpired turn. These are policy budgets, not latency guarantees. |
| Context state | Dispatch saves/restores x87/XMM state and FS base, changes address space, and updates the current CPU's TSS stack. AVX/YMM state is deliberately unsupported. |
| Physical handoff | Stable per-CPU pins protect outgoing stacks until assembly pivots away. Selection, reaping, and slot reuse check pins; release notifications address CPUs that deferred selection. |
| Address-space lifetime | Borrowers do not reclaim owner user pages. Owner reclamation waits for borrowers, physical ownership, and active-address-space checks. Native owner exit terminates borrowers; a Linux leader can retain group resources while workers finish. Exec rejects borrowers and owners with outstanding borrowers. |
| Native futex | Comparison uses a validated atomic `u32` load. Comparison/registration/wake are serialized under the scheduler lock; keys include VM identity and address. Unmap cancels matching native waits. |
| Native std join | Join polls syscall 156 until the target is `Reaped`, then releases stack/TLS mappings. It does not free a merely `Finished` thread's live stack. |

Sources: [process representation](../../kernel/src/process/mod.rs#L114),
[creation](../../kernel/src/arch/x86_64/syscall.rs#L2616),
[Linux clone](../../kernel/src/arch/x86_64/syscall.rs#L2877),
[dispatch](../../kernel/src/sched/mod.rs#L1776),
[handoff protocol](../../kernel/src/sched/handoff.rs#L1),
[reaping](../../kernel/src/sched/mod.rs#L2725),
[exec restriction](../../kernel/src/arch/x86_64/syscall.rs#L3285),
[native join](../../sunlight-libc/src/std_runtime.rs#L244).

## Ranked findings

| ID | Severity | Finding | Evidence status |
| --- | --- | --- | --- |
| T1 | Critical | Kernel syscall execution uses a user-writable stack | Confirmed source; corruption/exploit not executed |
| T2 | High | Thread creation can turn resource exhaustion into a kernel halt | Confirmed allocation/error paths; exhaustion not executed |
| T3 | High, conditional | Logical CPU indices assume dense APIC IDs and BSP ID zero | Confirmed source; unsupported topology not boot-tested |
| T4 | Medium | Linux futex comparison and child-TID clearing use non-atomic byte copies | Confirmed source; no observed torn access claimed |
| T5 | Medium | Linux robust-list registration succeeds without owner-death cleanup | Confirmed registration and absence of list processing |
| T6 | Medium | Reusable task slots can discard unconsumed child exit status | Confirmed source; no fresh guest reproduction |
| T7 | Medium | Native worker stacks have no guard mapping | Confirmed mapping path; overflow not executed |

### T1 — Kernel execution on user-writable syscall stacks

**Evidence.** `syscall_entry` pushes registers and calls `syscall_dispatch`
without changing away from the caller's RSP. Rust call frames and return
addresses therefore occupy user-accessible memory. Interrupt disabling affects
the current CPU; it does not stop a sibling thread sharing the VM on another
CPU. The normal syscall path is affected, independently of the scheduler's
kernel-stack pinning repairs.

**Impact.** A sibling can modify memory containing live privileged call frames
without acquiring a kernel lock. This exposes kernel control-flow/data integrity
to user memory and creates a potential privilege-escalation path. An invalid
caller RSP can also fault during entry before normal argument validation.

There is a concrete exit-lifetime concern: `finish_current_process` clears a
Linux child's TID and wakes its joiner **before** `process_exit` pivots to the
per-CPU idle stack. A joiner can observe completion and begin reusing stack/TLS
storage while kernel code still uses the exiting thread's user stack. The
outgoing kernel-stack pin does not prevent userspace writes to that storage.
The current native std join avoids early release by waiting for `Reaped`;
Linux child-TID completion has a different boundary.

**Recommendation.** Establish a trusted per-CPU/per-thread kernel stack at
syscall entry, preserving user RSP in trusted storage before any stack writes.
Keep the entire ordinary dispatch and exit tail on supervisor-only storage.
Publish Linux join completion only once no privileged path depends on user
stack/TLS storage. Preserve handoff pins and active-VM lifetime checks.

**Required verification.** Force a sibling stack/TLS-reuse operation at the
child-TID-clear boundary, and exercise invalid/guard-page syscall RSPs inside
an isolated guest. Assert kernel-frame storage is supervisor-only and that
completion cannot precede the last access to released user storage.

References: [entry](../../kernel/src/arch/x86_64/syscall.rs#L223),
[self-exit](../../kernel/src/arch/x86_64/syscall.rs#L2483),
[finish/clear ordering](../../kernel/src/sched/mod.rs#L4328),
[child TID clear](../../kernel/src/sched/mod.rs#L3084).

### T2 — Infallible, unbounded kernel allocations during thread creation

**Evidence.** Both native spawn and Linux clone call `Process::new_thread`.
It obtains its kernel stack through `Box::new_uninit`; construction also uses
infallible descriptor/environment/capability allocations. Task-table and queue
growth add allocation sites. The kernel heap is bounded at 16 MiB, and the
allocation-error handler halts the CPU. No per-VM thread quota or fallible
publication reservation was found in these creation paths.

**Impact.** An application can repeatedly invoke raw thread creation without
allocating a unique 2 MiB user stack each time. Accepted kernel stacks alone
consume 32 KiB each. Heap exhaustion can occur while the global scheduler lock
is held; halting its owner leaves other CPUs unable to schedule. The exact
thread count depends on existing heap usage and was not measured.

**Recommendation.** Add per-VM and global admission limits, reserve task/queue
capacity fallibly, and make kernel-stack and inherited-state creation return
recoverable errors. Publish the child only after all allocations and user
stores succeed; unwind reservations on every failure.

**Required verification.** Deterministically fail each allocation and test quota
boundaries. Require an error return, unchanged parent resources, no published
child, and continued scheduling of an unrelated process.

References: [native creation](../../kernel/src/arch/x86_64/syscall.rs#L2616),
[clone](../../kernel/src/arch/x86_64/syscall.rs#L2877),
[thread construction](../../kernel/src/process/mod.rs#L592),
[kernel-stack allocation](../../kernel/src/process/mod.rs#L30),
[heap bound](../../kernel/src/memory/heap.rs#L13),
[allocation failure](../../kernel/src/panic.rs#L78).

### T3 — CPU identity is not mapped independently of hardware APIC identity

**Evidence.** `current_cpu_id` uses CPUID's initial APIC ID, clamped to 63, as
the scheduler index. AP TSS/GDT allocation instead follows sequential startup
order. TSS updates use `cpu_id - 1`, and IPI delivery encodes the scheduler
index directly as an APIC destination. Initialization scans `0..online_cores`.

**Impact.** Sparse APIC IDs, a nonzero BSP ID, or high IDs can select the wrong
TSS/state, omit online CPUs from scans, misdirect wakeups, or alias multiple
physical CPUs onto slot 63. This is a conditional correctness/safety defect;
the contiguous QEMU topologies tested here do not establish that it occurs on
any particular physical machine.

**Recommendation.** Build an explicit dense logical-ID to hardware-APIC-ID
mapping from Limine data, record each CPU's logical index, and use that index
consistently for TSS, queues, accounting, pins, and masks. Translate at IPI
delivery. Reject unsupported topology rather than clamping identities.

**Required verification.** Test a nonzero BSP, sparse IDs, and IDs above 63;
verify distinct TSS stacks/state and delivery to every mapped CPU.

References: [CPU indexing](../../kernel/src/sched/mod.rs#L4594),
[AP startup](../../kernel/src/arch/x86_64/smp.rs#L125),
[AP-local initialization](../../kernel/src/arch/x86_64/smp.rs#L311),
[TSS updates](../../kernel/src/arch/x86_64/smp.rs#L179),
[IPI destination](../../kernel/src/arch/x86_64/lapic.rs#L168).

### T4 — Linux futex words do not use the native atomic-access discipline

**Evidence.** Linux `FUTEX_WAIT` reads four bytes through
`copy_from_process_bytes`; child-TID clearing writes zero through
`copy_to_process_bytes`. Those helpers use `copy_nonoverlapping`, rather than
an atomic word load/store. Sibling userspace threads can access those words
while the kernel holds its scheduler lock. The native futex path already has
a dedicated validated atomic-load helper.

**Impact.** The implementation lacks an explicit atomic and ordering contract
for these concurrently accessed words. A particular x86 build may lower a
four-byte copy to one instruction, but that is not the source-level guarantee
needed by the synchronization primitive. This audit did not demonstrate a
torn read or lost wake in the emitted guest binary.

**Recommendation.** Use the validated aligned atomic `u32` helper for Linux
futex comparison and add an atomic store helper for child-TID completion.
Define the ordering and retain compare/register/wake serialization. Validate
alignment appropriately before atomic child-TID access.

**Required verification.** Stress compare/register/wake and clear-child-TID
across CPUs while repeatedly changing the word; inspect atomic access in the
actual build and cover invalid/unaligned addresses.

References: [Linux futex](../../kernel/src/arch/x86_64/syscall.rs#L3011),
[clear child TID](../../kernel/src/sched/mod.rs#L3084),
[byte-copy implementation](../../kernel/src/memory/user.rs#L242),
[existing atomic read](../../kernel/src/memory/user.rs#L361).

### T5 — Robust-list registration advertises an unfinished lifecycle feature

**Evidence.** `sys_linux_set_robust_list` validates basic arguments, stores the
head/length, and returns success. References to those fields are limited to
initialization, registration, and clone reset; no exit-time robust-list walk
or owner-death update was found. Futex commands also support only wait/wake.

**Impact.** A runtime can register successfully while robust-mutex owner death
does not release/mark affected futexes. Programs relying on that behavior may
hang after a thread dies. This is separate from ordinary non-robust mutex use.

**Recommendation.** Implement bounded, validated exit-time robust cleanup,
including the pending operation and wakeup, or return an explicit unsupported
result and document the boundary. Review runtime fallback behavior before
changing the return value.

**Required verification.** Exit and externally kill a thread holding a robust
mutex. Require a bounded owner-death result for its waiter; cover malformed
and cyclic user lists without kernel faults or unbounded traversal.

References: [registration](../../kernel/src/arch/x86_64/syscall.rs#L6304),
[per-thread Linux state](../../kernel/src/process/mod.rs#L62),
[cleanup path](../../kernel/src/sched/mod.rs#L2725).

### T6 — Slot reuse can erase a child's unconsumed exit status

**Evidence.** `sys_waitpid` looks up completion only in retained `Process`
records. `add_process_after_reaping` can overwrite a `Reaped` slot without
checking whether its parent's exit status was consumed. Native thread spawn
also calls `add_process`, so unrelated thread churn can trigger the overwrite.
The supervisor-specific `service_reap_watcher` protects its own receipts, not
ordinary waitpid status.

**Impact.** A parent that waits after cleanup and slot reuse can receive
"no such child" instead of the child's exit result. Native std's separate
quiescence probe treats an absent old TID as complete, so this finding does not
by itself demonstrate premature native join reclamation.

**Recommendation.** Separate bounded child-completion records from reusable
execution slots, or retain a zombie receipt until consumed or reparented.
Define one-time consumption and dead-parent cleanup.

**Required verification.** Delay parent waitpid; exit/reap its child; force
reuse via thread/process creation; then require the original exact exit code.

References: [waitpid](../../kernel/src/arch/x86_64/syscall.rs#L3394),
[reuse](../../kernel/src/sched/mod.rs#L567),
[quiescence probe](../../kernel/src/arch/x86_64/syscall.rs#L2825).

### T7 — Native thread stacks lack guard pages

**Evidence.** `spawn_with_stack` maps exactly the rounded requested stack size
read/write, with no additional inaccessible boundary. The std overlay chooses
a minimum size but does not add a guard. Kernel thread stacks are also plain
heap allocations, without a separately protected boundary.

**Impact.** An overflow can corrupt an adjacent mapped allocation before
faulting. Native worker stacks are also used by ordinary syscalls (T1), so
their remaining headroom affects privileged execution. A fault at an unmapped
neighbor is not a deliberately maintained stack guard.

**Recommendation.** Reserve an inaccessible guard below each downward-growing
user stack, release the entire reservation during join/detach cleanup, and
consider guarded virtual kernel stacks separately. Preserve ABI alignment and
minimum usable space.

**Required verification.** Overflow a deliberately small stack in a guest and
require a contained fault with adjacent memory intact; test reclamation and
large stack-size arithmetic.

References: [stack allocation](../../sunlight-libc/src/thread.rs#L80),
[std thread minimum](../../tools/native-std/overlay/thread/sunlightos.rs#L7),
[kernel stacks](../../kernel/src/process/mod.rs#L32).

## Performance and API limits

These are verified constraints and improvement opportunities, not measured
performance regressions:

- **Global serialization and cleanup cost.** Timer/dispatch, futex scans,
  creation, and reaping share `SCHEDULER`. Normal dispatch calls a global reap
  scan; completed-task cleanup can perform substantial work and synchronous
  serial logging. Syscalls run with interrupts disabled. Measure lock wait/
  hold time and IRQ-off tails before splitting locks or changing policy.
- **Join polls.** Native std join repeatedly probes syscall 156 and yields.
  It is safe against early stack freeing but consumes syscalls and scheduling
  work. An exact-TID completion wait could block until physical quiescence.
- **Timeout granularity.** Native futex deadlines are checked by CPU 0's
  periodic tick. A nanosecond deadline is not nanosecond wake precision; long
  IRQ-off sections and scheduler delays add latency.
- **Conservative stealing.** `steal_work_round_robin` skips victim FIFOs with
  one queued task. A busy CPU can retain one eligible waiter while another CPU
  idles. Stealing also bypasses local nice-admission decisions. Benchmark
  skewed load before modifying either behavior.
- **Detached retention and capacity.** Finished detached std handles are swept
  on later spawn/join/detach activity, not immediately. The table has 256 slots;
  overflow calls group exit with code 101. More than 256 still-live detached
  workers can therefore terminate the application. This is documented code
  behavior, not a new leak proof. Prefer a fallible admission contract or a
  dynamically managed completion mechanism.
- **Different thread contracts.** The alloc-only API has no join and does not
  reclaim stack/TLS on handle drop. Native syscall 22 copies descriptor tables;
  syscall 158 shares them. Native creation copies UID/GID/nice but starts with
  the constructor's unrestricted CPU mask; Linux clone explicitly inherits
  affinity. Credential changes are per task in `sys_setuid`/`sys_setgid`;
  process-wide privilege dropping would need an explicit group contract.
- **Deliberate runtime limits.** Key-based TLS has 255 usable keys; native std
  thread names/OS IDs and `available_parallelism` are unimplemented. AVX,
  general compiler TLS, and unwind-based worker panic recovery are outside the
  native runtime's current contract. Linux timed futex waits and commands
  beyond basic wait/wake return unsupported.

References: [dispatch/reap](../../kernel/src/sched/mod.rs#L1979),
[stealing](../../kernel/src/sched/mod.rs#L1392),
[timeout scan](../../kernel/src/sched/mod.rs#L2137),
[join/detach](../../sunlight-libc/src/std_runtime.rs#L244),
[TLS keys](../../tools/native-std/overlay/thread_local/key/sunlightos.rs#L1),
[std thread APIs](../../tools/native-std/overlay/thread/sunlightos.rs#L41),
[credentials](../../kernel/src/arch/x86_64/syscall.rs#L4458).

## Validation performed

| Check | Result | What it establishes |
| --- | --- | --- |
| `python3 tools/test_scheduler_host.py --timeout 120` | PASS | 75 Rust test executions across the focused harnesses, plus policy/pin-eligibility and queue-publication probes. Some module tests run again inside the service harness; 75 is not a distinct-test count. |
| `cargo check --package sunlight-kernel --bin sunlight-kernel --offline` | PASS | Current normal kernel type-checks. Existing startup-fixture manifest and sunlight-net naming warnings remain. |
| `cargo test --package sunlight-compat-linux --lib --target x86_64-unknown-linux-gnu --offline` | PASS, 29 tests | Includes clone flag validation and ABI/affinity tests. Does not execute kernel clone/futex bodies. |
| Fresh `scheduler_b_test` QEMU gate, 1/2/4/12 CPUs | PASS on all four configurations | Isolated ring-3 lifecycle, register/SSE cookies, terminal transitions, deferred reaping, borrower cleanup, and slot reuse. |

The fresh guest command is:

```sh
python3 tools/test_scheduler_smp.py --cpus 1 2 4 12 --timeout 120 \
  --evidence-dir target/threading-audit-2026-11-04/scheduler-smp
```

The driver builds a dedicated feature-enabled ELF/ISO, records their hashes,
and runs QEMU TCG with contiguous APIC IDs. Its successful lifecycle assertions
do not prove T1–T7 absent. It preconstructs borrower objects and does not test
all raw native spawn/Linux clone admission paths. Host service tests stub
architecture/reclamation operations; their success is not a physical handoff
or memory-model proof.

Retained serial verdicts (one fresh boot per configuration):

```text
[SCHED-B] PASS cpus=1 deferred_reaps=5 reused=1 exit_races=1
[SCHED-B] PASS cpus=2 deferred_reaps=7 reused=2 exit_races=1
[SCHED-B] PASS cpus=4 deferred_reaps=7 reused=4 exit_races=1
[SCHED-B] PASS cpus=12 deferred_reaps=12 reused=12 exit_races=1
```

The exact ELF/ISO hashes, QEMU commands and exit status are in
`target/threading-audit-2026-11-04/scheduler-smp/results.json`; the focused host
log, source hashes, and validation summary are retained alongside that folder.

Not rerun in this audit: the complete workspace suite, ordinary desktop boot
gate, native std/Hayro FP and synchronization suite, Linux raw thread probe,
pin-release ordering/remote-wake matrices, malicious stack/heap exhaustion,
sparse-APIC boots, or performance benchmarks. Earlier reports contain useful
historical evidence, but their results are not relabeled as fresh validation.

## Relationship to earlier audits

The historical [scheduler Phase A audit](scheduler-audit.md) reported outgoing
stack lifetime, publication-before-handoff, positive-nice starvation,
self-rearming dispatch, and quantum-accounting defects. Current code contains
explicit repairs: physical pins, release notification, internal enqueue without
notification, aging admission, and frozen slice accounting. Those old findings
must not be copied into a new report as if the repairs were absent. T1 concerns
ordinary syscall storage and Linux join completion, a separate boundary.

The earlier waitpid status-retention concern remains visible (T6). Dense APIC
assumptions and global critical-section costs also remain. Consult
[scheduler hardening](scheduler-hardening.md),
[pin-release follow-up](scheduler-remote-wake-f13nr2d.md), and the
[native std audit](../native-std-post-implementation-audit.md) for historical
scope and validation limits; experimental FastLane adapters have their own
contracts and are not evidence that every production wait path is hardened.

## Recommended order of work

1. Move ordinary syscall execution to trusted kernel stacks and validate the
   Linux completion/reuse boundary (T1), preserving current handoff invariants.
2. Make thread admission/allocation recoverable and bounded (T2).
3. Add explicit CPU-ID mapping before expanding hardware support (T3).
4. Give Linux futex/TID accesses a defined atomic contract and complete or
   explicitly reject robust owner-death handling (T4/T5).
5. Retain child completion independently of execution slots and add guarded
   stacks (T6/T7).
6. Re-run lifecycle, remote-wake/pin-release, native std, and Linux thread gates
   after each relevant change. Add the targeted counterexamples listed above.
7. Measure saturated CPU, contended locks, repeated join/detach, IPC, and mixed
   interactive workloads on matched 1/2/4/12-CPU configurations. Optimize global
   critical sections and wake/completion waits before selecting a new policy.
