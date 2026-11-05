# T4A atomic futex audit and validation

Status: IN PROGRESS. Initial HEAD `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
Initial dirty paths, binary tracked diff, source hashes and relevant pre-change
sources are retained under `.threading-evidence/t4a/initial/`. Existing dirty
work and historical evidence must remain intact. Production FastLane stays off.

## Phase A: state machine recorded before implementation

Both native syscall 155 and Linux syscall 202 (internal 1042) acquire SCHEDULER
before accessing the word. The order is:

1. Check op/alignment; acquire SCHEDULER; select current Process and VM identity.
2. Validate four bytes and resolve their present physical backing via HHDM.
   Anonymous demand backing may acquire PMM while SCHEDULER remains held.
   Swapped pages return an error. No userspace pointer is directly dereferenced.
3. Native performs AtomicU32 acquire load; Linux copies four bytes nonatomically.
4. WAKE scans the bounded task table for BlockedOnIo + matching VM identity +
   address. WAIT compares the sampled value, checks supported timeout rules,
   sets one task-local waiter address/deadline, then sets BlockedOnIo.
5. Release SCHEDULER. Request the existing post-SYSRET scheduling trap, holding
   no lock. The request itself does not write BlockedOnIo or transfer ownership.
6. WAKE under SCHEDULER clears waiter metadata, sets Ready, removes old queue
   entries, then requests the live owner or enqueues an unowned task. The
   scheduler's physical stack pins govern when it may run elsewhere.
7. A wake before physical descheduling leaves Ready; deferred scheduling cannot
   undo it. A wake after descheduling enqueues normally. Resumption returns the
   syscall's preselected zero. Userspace must recheck its condition.

State transitions: Running/unregistered -> (mismatch) Running/unregistered;
Running/unregistered -> BlockedOnIo/registered under one critical section;
BlockedOnIo/registered -> Ready/unregistered under wake or native timeout;
Ready/unregistered -> Running/unregistered through normal dispatch. Blocked can
still be physically executing until the deferred trap. No separate wait-queue
allocation or raw Process pointer exists. Migration preserves the Process slot,
VM identity, and waiter field.

The suspected unlocked compare/enqueue window is ABSENT. The existing global
scheduler lock already serializes both sides, so adding another hot-path lock
is unnecessary. Linux's byte-copy comparison lacks a guaranteed indivisible
32-bit access. Generic I/O/signal wake leaves waiter metadata; a later unrelated
BlockedOnIo operation can be matched by a stale futex wake or expired deadline.
Native-only munmap cancellation misses Linux waiters; MAP_FIXED replacement
cancels neither ABI, so a wake through a new mapping can consume an old waiter.
Termination/reaping also does not explicitly clear futex metadata.

## Intended correction and lock rationale

Use the existing validated AtomicU32 reader for Linux WAIT/WAKE. Retain
SCHEDULER across compare, one waiter publication and BlockedOnIo. Validation
can back one demand page synchronously under SCHEDULER -> PMM; it does not
sleep, fault through the user VA, or drop SCHEDULER. No retry is needed for the
current memory contract. Any future fault path that drops it must restart
validation, load and identity lookup before publication.

Unregister both ABIs on generic wake, signal and termination. Cancel matching
waiters after successful unmap and MAP_FIXED commit while SCHEDULER is held;
failed mutation must preserve waiters. Cancellation allows a spurious successful
return and never accesses the removed word. Clear-before-Ready is the single
terminal transition, serialized against timeout, wake, cancellation and exit.

Spin lock acquire/release orders kernel state. The aligned AtomicU32 acquire
load pairs with userspace release publication; wake itself does not replace
userspace synchronization. The normal owner/pin/queue wake protocol is retained.
No scheduler blocking/context switch occurs with SCHEDULER or PMM held.

## Identity and supported surface

Supported keys are (AddressSpaceIdentity {pml4_phys, generation}, aligned VA).
CLONE_VM borrowers share the identity; new VMs have monotonic nonzero generations,
preventing root-frame reuse from aliasing waiters. CPU identity is not a key.
Anonymous mmap requires MAP_PRIVATE | MAP_ANONYMOUS; file-backed, MAP_SHARED,
and mremap are unsupported. Linux WAIT/WAKE without PRIVATE is accepted only
for the same-VM/same-VA subset (including clear_child_tid). Cross-process shared
backing and different-VA aliases are not Linux-compatible shared futexes.
Exec rejects borrowers/live shared-VM threads and only replaces an owning task;
no Linux multi-threaded exec semantics are claimed. Retiring waiters on committed
unmap/replacement prevents a new mapping at the same VA waking an old wait.

Linux timed WAIT, bitsets, requeue, PI, robust owner-death and T5 cleanup are
unsupported. Native uses absolute monotonic deadlines; expiration and wake are
serialized, with spurious return permitted. Its existing libc wrapper classifies
elapsed finite deadlines after resumption, so exact wake-versus-timeout errno
semantics are not promised. Catchable Linux signal interruption/restart needs
separate verification; no full Linux futex compatibility is claimed.

## Validation

Pending fresh-source focused host checks, controlled native pilot, SMP native std
synchronization regression, debug/release builds, Clippy and production boot.
