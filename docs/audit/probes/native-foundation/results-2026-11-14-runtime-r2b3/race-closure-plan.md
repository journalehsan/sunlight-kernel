# Exact witness boundaries and safe follow-up coordination

The final-source native TLS fixture proves a callback is executing and held open
while another identified thread deletes and reuses its key. Callback reentrancy
completes before the hold; completion precedes Reaped/join. A separate holder
retains an old-generation cell while delete/reuse occurs and then rejects the old
key. Those are real ordered/overlapping states. Functional once contention and
abandonment are not evidence of a particular kernel registration point.

## TLS and registry

Create/delete/get/set serialize under KEY_LOCK; their critical sections cannot
legally overlap. Blocking an owner inside that critical section until a competing
API returns would deadlock. Do not use such a gate. A safe test-only facility must
publish a request marker before acquiring the lock and record acquisition and
linearization after acquisition. A controller outside the lock can release the
first actor once the contender's pre-acquisition request has been observed; then
check generations/results and the actual serialized order. Keep markers separate
from pthread TSD and the futex mutex they instrument. Instrumentation must be off
by default, bounded, keyed to VM/thread identities and never change production
lock ordering. Current tests do not contain these internal request markers.

The added handle-claim probe locks the actual registry, observes a reserved join
while its target remains gated, then performs detach and a second join before
releasing the target. Both competing operations must return EINVAL; the selected
join completes and the registry becomes empty. This is a native ownership-interval
witness, not a claim that two critical sections overlap.

Detach/public lookup/sweep now use the same registry lock. Force the historical
observer schedule by holding a detacher at a test-only pre-acquisition point,
completing the worker, and holding the sweeper until the detacher's request is
observed. Record which actor acquired the lock first, unique reclaim ownership,
Reaped acknowledgment and exact unmap receipts. A gate waiting for a competing
registry operation to finish while holding DETACHED_LOCK is invalid. Add a
pre-publication create gate to force worker self-detach before the creator has
published TID/mapping metadata; observe published=false, then release the creator
and require exactly one queue entry. The implementation protects this ordering,
but no current native fixture forces that interval.

## Once / kernel registration

Source: pthread_once uses 0/1/2, acquire election/read and release publication;
cleanup_current resets abandoned nested controls to zero and wakes contenders.
Kernel sys_native_futex serializes compare/registration and wake under SCHEDULER,
and validates the word against the current VM. It installs the registration
before setting BlockedOnIo and releases the scheduler lock before replay wait.

Existing debug-only t4a_fixture::point has useful stages 0 (pre-lock), 1
(compared), 2 (registration installed), 3 (blocked), 4 (post-lock). It belongs to
an isolated actor fixture and is not armed by the ordinary native C runner. Do
not count those source hooks or prior raw-futex runs as once-integrated proof.

A follow-up should add explicit test-only arming for the actual pthread_once
word and generation-qualified actors, then record election, publication, wait
comparison, installed registration and consumed completion receipt. Before-lock
and post-lock gates may wait for completed wake. Inside SCHEDULER they may wait
only for a pre-lock syscall-entry announcement from another CPU, never for wake
completion: the wake needs the same lock. Reuse the existing lock ordering and
bounded rendezvous pattern rather than inventing a blocking cross-lock callback.

On 1 CPU, another user actor cannot publish completion while the waiter remains
inside a nonpreemptible registration critical section. Prove completion-before
and registration-before on 1 CPU; qualify actual publication during registration
on 2/4 CPUs. State this structural distinction in the matrix. Test multiple
installed waiters, initializer abandonment/retry, receipt lifetime under supported
forced contender termination, and rapid independent controls before any stress.
No pthread cancellation API, production sleep, weakened compare or new syscall
is proposed. These missing witnesses keep G5/G6 open.
