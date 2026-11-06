# Before-change ownership audit
HEAD 73cb0b81dff5bd7d6e10d36d82122686cfaf5fa3; clean tracked/untracked source tree.

Registration and all completion/VM transitions hold SCHEDULER. There is no separate
allocated waiter queue: the bounded process table contains native address/deadline,
Linux address and native one-shot (address,result) receipt. Identity is task PID +
foreground incarnation + VM (PML4,generation); CPU is placement, not identity.

Running/unregistered -> validated aligned atomic word read -> mismatch/immediate
expiry return without registration, or address/deadline publication -> BlockedOnIo.
WAKE filters exact VM, ABI, VA, blocked state, stops at limit. Completion takes the
address and stores receipt 0; native expiry first takes it with receipt 2, then uses
normal wake placement. record_wakeup's second take cannot overwrite receipt 2.
Generic signal/I/O cancellation takes registration with receipt 0 (allowed spurious
return), never EINTR. Committed unmap/fixed replacement cancels both namespaces.
Exit/exec/reap clear registration, deadline AND receipt. Finished/reaped tasks are
not eligible; physical ownership/pins defer resource teardown. Borrower retirement
precedes VM destruction. New threads start empty; monotonically assigned IDs and
VM generations prevent slot/PML4 reuse from matching old incarnations.

Wake placement removes old queue occurrences, makes Ready, then notifies a live
owner or enqueues an unowned task. Deferred post-SYSRET blocking request never
writes BlockedOnIo again. Affinity changes on quiescent blocked tasks leave waiter
identity intact. Dispatch checks owned stack and FS/TSS identities.

Supported: native private WAIT=0 (absolute monotonic deadline or indefinite),
WAKE=1, RESULT=2 (0 spurious/wake,2 timeout); Linux WAIT/WAKE with/without PRIVATE
for same VM/VA private supported mappings, no timed Linux WAIT. No shared backing,
PI, requeue, robust death, signal-frame/restart/EINTR or pthread cancellation.

Terminal contract: first serialized detach wins, exactly one receipt if native;
termination discards receipt and never resumes the dead incarnation. Scheduler
spurious completion is legal result 0 but is not a FUTEX_WAKE selection. Expiry
winning is determined by processing under the lock, not elapsed wall time alone.
Existing T4A final manifest and all historical evidence remain authoritative for
their original source identities, not automatically for new sources.
