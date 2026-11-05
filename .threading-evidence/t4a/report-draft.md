# T4A — Atomic futex wait/wake

**Status: COMPLETE for the supported private, same-VM WAIT/WAKE contract.**
This is not full Linux futex compatibility. Shared backing, PI, robust lists,
Linux timed waits and signal-handler interruption/restart are outside the
supported surface described below.

Initial HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
The starting tree was already extensively dirty. Initial status, tracked binary
diff, 557 source/config hashes and original edited sources are preserved in
`.threading-evidence/t4a/initial/`. No commit, push, clean, privileged disk
operation or historical-evidence deletion was performed. Production FastLane
remains disabled.

## Root cause and before state machine

The suspected unlocked compare/enqueue window was **absent**. Both native syscall
155 and Linux syscall 202 (internal dispatch 1042) already held `SCHEDULER`
through validation, comparison, registration and `BlockedOnIo` publication.
Adding another global or bucket lock would not repair a missing critical section.

Before the change:

1. Reject unsupported op/alignment; acquire `SCHEDULER` and select the current
   thread and its address-space identity.
2. Validate four bytes, backing an anonymous demand page synchronously if needed,
   then resolve the physical mapping through HHDM. Native uses an `AtomicU32`
   acquire load; Linux uses a byte copy with no atomic-access guarantee.
3. On mismatch return native 1 / Linux `EAGAIN`, without registration. Otherwise
   publish one task-local address, an optional native deadline, and `BlockedOnIo`.
4. Release `SCHEDULER`, then request the existing post-SYSRET scheduling trap.
   This request does not write the blocked state again or transfer stack ownership.
5. WAKE, under the same lock, selects matching blocked tasks, clears metadata,
   sets `Ready`, removes obsolete queue entries and either notifies the live owner
   or enqueues an unowned task. Normal dispatch later restores its context.

A wake after step 4 but before physical descheduling already leaves the task
`Ready`; the deferred trap cannot undo that completion. A wake after physical
descheduling enqueues normally. No external waiter allocation or raw `Process`
pointer is involved.

The audit confirmed adjacent defects:

- Linux's value read lacked a guaranteed indivisible, aligned atomic load.
- Generic signal/I/O wake left futex metadata. A subsequent unrelated I/O block
  could be selected by an old futex wake or native deadline.
- Native-only syscall-level munmap cancellation missed Linux waiters and internal
  munmap callers such as brk shrink. Successful `MAP_FIXED` replacement cancelled
  neither ABI, permitting an old waiter to be selected through a new mapping.
- Termination/reaping did not explicitly detach all futex metadata.
- Native libc classified timeout by reading the clock after resumption. A wake
  that completed before the deadline could be reported as a timeout after delayed
  execution. Initial timeout comparison also used the per-core scheduler clock
  rather than the public cross-core monotonic clock.

## Corrected state machine and completion ownership

Both entry points now use `read_process_futex_u32`, which restricts storage to
approved private mapping classes and performs the validated aligned atomic load.
The original `SCHEDULER` critical section and scheduling ownership are retained.

| State | Event under `SCHEDULER` | Result |
| --- | --- | --- |
| Running, unregistered | invalid storage / mismatch / expired native deadline | return error / mismatch / timeout; never enqueue |
| Running, unregistered | matching supported WAIT | one address publication, then `BlockedOnIo` before unlock |
| Blocked, registered, still owned or quiescent | matching WAKE | detach once, record native result 0 if applicable, make Ready |
| Blocked, registered | native deadline expiry | detach once, record result 2, make Ready |
| Blocked, registered | generic cancellation or committed unmap/replacement | detach once, native spurious-success result 0, make Ready |
| Any task | terminal exit/reap | detach; discard completion receipt; never make Finished/Reaped runnable |
| Ready, completed | dispatch / physical handoff | normal queue, owner and pin rules; no second blocking transition |
| Native completed wait | RESULT operation 2 | consume this thread's matching receipt once, without reading the old word |

`complete_futex_wait` consumes the registration with `Option::take`. Repeated
wake bookkeeping cannot overwrite its completed result. `clear_futex_wait`
additionally discards the receipt on exit/reap and successful exec. Constructors
initialize both registration and receipt empty; clone does not inherit receipts.
A new native WAIT invalidates the previous receipt, including a new WAIT that
returns without blocking. Receipts are bounded task-local scalar data, not waiter
references or retained mappings.

Native operation 2 returns 0 for wake/spurious cancellation, 2 for timeout, and
EINVAL for an absent, wrong-address or already-consumed receipt. It belongs to
the current thread, not the shared process owner. It needs no memory revalidation:
unmap can be the event that completed the wait. Existing raw WAIT still returns
zero after a registered wait; native std libc immediately consumes the receipt.
Immediate mismatch and expired-deadline returns remain 1 and 2. This preserves
the raw WAIT/WAKE interface while removing libc's post-resumption clock heuristic.

Deadlines now use `timekeeping::monotonic_ns()` for admission and expiry, matching
native std's public monotonic clock across migration. Resolution is 10 ms. A
terminal wake that wins the lock remains a wake even if receipt consumption is
later than the deadline; an expiry that wins remains a timeout. Expiration need
not have priority over a wake merely because wall time has advanced past a
deadline before the timer processes it.

## Locks, memory ordering and bounded work

The existing order is `SCHEDULER -> PMM` when a demand page needs backing. The
address-space ledger's terminal lock is released before PMM or scheduler work.
The test gate is debug-only and ordered after `SCHEDULER`; it is absent from
ordinary images. No new production hot-path lock or scheduling policy was added.

Validation retains `SCHEDULER`; it may back one anonymous page synchronously but
never drops the guard, sleeps, or faults through a raw user VA. Swapped/unsupported
storage fails rather than entering a fault-and-unlock path. A future fault path
that drops the guard must restart mapping validation, key lookup and atomic load
before enqueue. The present implementation needs no unlock/revalidation retry.

Spin-lock acquire/release serializes registration and all terminal transitions.
The futex word uses an aligned `AtomicU32::load(Acquire)` through its checked HHDM
alias; user synchronization still requires its own release/acquire protocol.
Alignment keeps four bytes on one page. No user pointer is retained or
later dereferenced by wake, timeout, cancellation or completion-result retrieval.
No lock is held when the deferred trap/context-switch operation is requested.

Waiters occupy existing admitted task slots. Wake/cancel/expiry scans are bounded
by the existing 512-slot task-table limit. There is no per-WAIT allocation,
unbounded waiter list, duplicate registration or separate lifetime owner.
Finished-task guards prevent a concurrently killed caller registering again.

## Identity, mapping retirement and unsupported semantics

Supported key: `(AddressSpaceIdentity { pml4_phys, generation }, aligned VA)`.
Native and Linux wait namespaces remain separate. CLONE_VM/native borrowers use
the same VM identity; CPU number and APIC ID are not part of a key. New VMs use
monotonic nonzero generations, so reuse of a PML4 frame does not reuse identity.
The corresponding generation/borrower host tests pass in the fresh kernel gate.

Approved storage classes are anonymous private mappings, brk, user stacks and ELF
segments. These either remain stable for the VM lifetime or are retired by the
mmap/brk transactions. Both ABIs are cancelled after successful unmap or fixed
replacement commit, before releasing `SCHEDULER`. Failed transactions preserve
the original registration. This also covers internal brk-shrink munmap calls.
A per-mapping generation is unnecessary for this contract because no old
registration survives successful replacement.

Capability SHM, framebuffer, telemetry, boot-shared and internal/borrowed mapping
classes are rejected for futex storage. The fixture creates two actual SHM views
of the same backing at different VAs, verifies rejection before word access, then
releases both. This is an unsupported-semantics control, not shared-futex support.
Linux mmap still rejects MAP_SHARED/file backing; native SHM has separate alias
and teardown contracts. Cross-VM, different-VA shared-backing futexes are not
implemented. Raw user VA is never claimed to identify a shared futex object.

Linux accepts WAIT/WAKE, with or without PRIVATE, only for the same-VM/same-VA
subset. Positive wake counts use the signed 32-bit Linux `val`; negative counts
are rejected with EINVAL. Timed Linux WAIT, WAIT_BITSET, REQUEUE, PI, robust
owner-death, T5 cleanup and cross-ABI synchronization are unsupported. Catchable
user signal frames/restart/EINTR semantics are not implemented; cancellation here
is an allowed spurious completion. Fatal exit discards a waiter instead of
resuming it. The native cancellation fixture injects the real scheduler signal-
wake path; it does not claim a userspace signal-handler execution test.

Successful exec clears receipts only after image preparation succeeds. Failed
exec preserves the old VM and receipt. Exec rejects borrowers/live shared-VM
threads; multithreaded Linux exec replacement is not claimed. Owner VM lifetime
continues to be governed by existing borrower retirement and physical stack pins.

## Wake cardinality

The production loops already stopped after `limit` eligible selections, including
an immediate stop for zero, and returned that selection count. They did not
unconditionally wake all mutex waiters. This behavior is retained. Selection
requires the exact VM, address, ABI namespace and `BlockedOnIo` registration;
completed, unrelated and terminal tasks are excluded. Normal wake placement
preserves live-owner and stack-pin rules and removes stale queue occurrences.

Selection follows task-slot scanning; no FIFO guarantee is introduced. Selection
is distinct from a later thread dispatch or userspace resumption. Native std
mutex unlock and Condvar notify_one use count 1. Explicit notify_all uses the
existing `futex_wake_all` with `u32::MAX`; Linux callers can broadcast with
`INT_MAX`.

The 32-waiter controls run for both ABIs on 2/4/12 CPUs. All 32 are confirmed
registered and quiescent before selection. Resumed threads remain alive behind
a separate release gate, so selection and execution counts can be checked
independently:

| Request | Selected by this call | Total selected | Remaining |
| --- | ---: | ---: | ---: |
| wake zero | 0 | 0 | 32 |
| wake one | 1 | 1 | 31 |
| wake four | 4 | 5 | 27 |
| explicit broadcast | 27 | 32 | 0 |
| repeat wake one | 0 | 32 | 0 |

A wake at an unrelated address selects zero. Another 32-thread wave splits 16/16
across two addresses: broadcasting the first resumes exactly 16 and leaves the
other 16 registered; a repeat on the first selects zero; broadcasting the second
selects the remaining 16. Every registration completes once and all workers exit.
For example, final 2-CPU Linux evidence records total selected 5 while resumed is
still less than 5; neither counter is substituted for the other.

## Native race controls and migration

Evidence is real ring-3 execution under QEMU multi-thread TCG, not a host model
or a bare-metal timing/performance claim. The runner freezes kernel ELF/ISO
hashes, records complete source hashes and commands, retains failures/timeouts,
and checks source stability. Parser unit transcripts are explicitly synthetic
negative controls and are not counted as native evidence.

The final two-CPU pilot passes before expansion. Roles use distinct CPUs for the
critical locked rendezvous. Actual syscall entry on CPU 1 is observed while the
waiter on CPU 0 is paused. The lock remains held for compare/register/block
boundaries; the waker performs the normal production WAKE after it acquires that
lock. The post-unlock control additionally requires wake completion while the
waiter still physically owns its CPU/stack. Rendezvous loops are bounded and
require a running waker before entering the locked interval. Their gate word is
a validated, permanently owned static ELF word in the isolated debug fixture.

Each ABI runs scenarios 0–16 (Linux scenarios are numbered +17):

| Scenario | Native observation / constraint |
| --- | --- |
| 0 | mismatch without registration; null, kernel, noncanonical and misaligned addresses rejected |
| 1 | store and WAKE complete before validation/registration; mismatch, zero selected |
| 2 | competing waker enters after value load while compare/enqueue lock is held; one completion |
| 3 | competing wake after registration, before BlockedOnIo publication; one completion |
| 4 | competing wake during blocked-state publication; one completion |
| 5 | wake completes after unlock but before physical descheduling; no subsequent sleep |
| 6 | wake of an already quiescent blocked waiter |
| 7 | two waiters; zero/one/one/repeat wake counts and remaining waiter checks |
| 8 | native timeout wins; later wake selects zero; result 2 consumed once; Linux timed wait returns ENOSYS without registration |
| 9 | native wake wins; consume after deadline, still result 0; later cancellation cannot duplicate it; Linux uses indefinite wait |
| 10 | scheduler cancellation wins over later WAKE; zero selected by that wake |
| 11 | successful unmap, remap at the same VA, then wake selects zero old waiters |
| 12 | fixed replacement while blocked, then wake selects zero old waiters |
| 13 | external termination while waiting; no resumption; one detach and normal deferred reaping |
| 14 | 128 actual blocked handoffs with alternating affinity; same thread dispatches on CPUs 0 and 1 |
| 15 | 32-waiter cardinality and repeat-wake control |
| 16 | 32 waiters split across two independent addresses |

All 34 scenarios pass on 2/4/12 CPUs. One CPU cannot run the locked two-CPU
rendezvous; its mutex/condvar/handoff execution is covered by native std below.
Every final raw run creates, finishes and reaps 190 borrowers. The intentionally
terminated waiter exits 37; all other workers exit zero. TLS sentinels, kernel
stack identity, loaded TSS RSP0, loaded FS and absence of remote stack restoration
while pinned are asserted. Migration uses only the existing affinity field on a
quiescent waiter; normal wake/dispatch performs queueing and notification.

The existing native-std/Hayro fixture now adds three rounds of four independent
producer/consumer Mutex+Condvar pairs, 128 transfers per pair, totalling 1,536
handoffs. A ninth thread independently progresses; workers verify TLS and stack
canaries. The existing contended mutex/RwLock, notify_all, timed condvar, thread
join/detach/capacity, OOM/group-exit and Hayro compatibility checks remain active.
This full synchronization fixture passes on 1/2/4/12 CPUs. Hayro remains a
compatibility regression; no rendering behavior was changed.

## Resource reconciliation

Raw workers use owner-static user stacks/TLS; no per-worker user mapping is
allocated or falsely counted as reclaimed. After each scenario all worker
kernel stacks, TLS/context fields, waiter metadata and receipts are cleared;
no ready-queue occurrence, live owner, physical pin or handoff waiter remains.
FD description references return to the owner's three. Free-frame counts return
to the post-mapping baseline after every scenario.

The existing scheduler retains one FD-table cache per reused task slot. Relative
to the warmed two-worker baseline, the three-worker scenario retains one extra
`size_of::<FdTable>()`; the 32-worker waves retain 30 extra tables. Heap checks
account for exactly those retained caches and remain stable thereafter. A
one-entry anonymous replacement/reclaim-candidate buffer is warmed before the
baseline. Registrations/results themselves never allocate. Native std's three
handoff rounds return to identical live allocation count and backing bytes after
warmup; exact values are in each serial receipt.

## Validation, failures and reproduction

Final evidence bundles (all under `.threading-evidence/t4a/`):

- `20261105T181008.050467Z-suite-races/`: final controlled races, 2-CPU pilot then
  4/12 CPUs; fresh 171-test kernel host gate, 29-test Linux compatibility gate,
  scheduler ownership/wait/wake host controls, receipt negative controls, ABI gate.
- `20261105T181152.684093Z-suite-std/`: final native std synchronization and Hayro
  regression on 1/2/4/12 CPUs, rebuilt with the completion-result libc wrapper.
- Final Linux and production bundles are listed in `final/manifest.json`.
- `clippy/comparison-completion.json`: actual kernel Clippy still fails with the
  exact initial 155 diagnostic records, including summary; no added/removed
  diagnostics and no primary span intersects T4A changes.
- `clippy/libc/comparison.json`: actual native-runtime Clippy has the same 50
  records as original source (47 warnings, two existing errors and summary).
  Existing errors are `alloc.rs:454` mutable-from-immutable input and
  `launch_trace.rs:12` public raw-pointer dereference. Neither is introduced here.
- Native docserv Clippy exits zero. It has the existing warnings plus one
  `missing_const_for_thread_local` warning from the pinned native std macro at
  the new T4A TLS key, whose initializer is already explicitly `const`.
- Changed Rust spans and new fixtures pass rustfmt. Ordinary debug/release,
  release-with-fixture-feature exclusion, production ELF ABI and production boot
  are checked on final kernel/runtime source. Fixture commands, strings, module
  symbols and embedded raw ELF are excluded from ordinary production images.

Production release reaches Foundation Complete, sunlightd OK, timezone OK and
login input ready. No T4A fixture executes in production; the FastLane channel
selector remains guarded by its pre-existing debug-only feature. No unrelated
T1/T2/T3 full matrix was repeated.

Failed attempts are preserved: initial fixture compilation missed an existing
feature-gated handoff diagnostic; first pilot stalled because it did not ensure
the waker was running before the waiter locked; second pilot's 5 ms deadline
could expire on a 10 ms clock; third pilot exposed the 24-byte first-replacement
candidate-cache growth. These fixture assumptions were corrected before accepted
expansion. A release exclusion command failed before Cargo due to argument order;
its old-image marker receipt is explicitly invalid. Native userspace Clippy
capture first mis-scoped wrapper Cargo arguments, and its first isolated baseline
omitted an asset symlink; corrected attempts and original failures remain.
Earlier passing images remain bound to their earlier sources and are not proof
for the later completion-result change.

Reproduce with serialized builds sharing `target/`:

```sh
python3 tools/threading_t4a_format.py
python3 tools/threading_t4a_run.py races
python3 tools/threading_t4a_run.py std
python3 tools/threading_t4a_run.py linux
python3 tools/threading_t4a_run.py production
```

`final/manifest.json` ties accepted receipts, artifacts, final edited-source
hashes and preservation checks together. No required supported WAIT/WAKE behavior
is left unverified by the state-machine proof and focused native controls above.
Unsupported semantics are explicit limitations, not claims of Linux equivalence.
