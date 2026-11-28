# VFS-C2I3A worker admission and shared cwd continuity

**PARTIAL qualification.** The opt-in Downloads candidate now publishes native
shared workers and Linux thread clones only after authenticated admission when
their shared cwd requires the service. The host copied-channel tests verify
that admitted workers keep independently authorized pathname access after cwd
replacement. Both ordinary ownership gates still fail. Native and Linux guest
execution, CPU dispatch, and physical resource accounting are **NOT_RUN**.

## Publication order

C2I3 called `Scheduler::publish_thread` before requesting a VFS retain.
`publish_thread` retains the descriptor snapshot, inserts the process, and
calls `enqueue_ready_on_cpu`. The scheduler can select that Ready entry and
mark it Running as soon as the syscall releases the scheduler lock. The
service retain acknowledgement can arrive later. A host replay of the C2I3
ordering observes a Running worker with neither a receipt for OPEN nor a
namespace for stat. The same test reproduces loss of pathname admission when
the old cwd completion withdraws the worker's profile.

The candidate adapters in `kernel/src/arch/x86_64/syscall.rs` pass the
constructed borrower and its assigned incarnation to the C1A pending-child
transaction. While waiting, only the creating syscall parks. The child PID
is reserved in the pending table and has no process-table or ready-queue
entry. The scheduler and broker locks serialize preparation and commit. The
copied retain and acknowledgement run between those steps.

`kernel/src/ipc/vfs_registration/worker.rs` resumes the original creator and
syscall, then revalidates the service epoch, creator and child incarnations,
shared VM, child credentials, and exact source receipt before commit. The
existing publication function installs the independent binding before the
worker leaves the pending table. Linux's parent TID store follows binding
installation and precedes enqueue, with its user mapping rechecked at
commit. The normal thread publication path still owns descriptor retention
and scheduler placement. Unstaged failures drop the provisional stack,
unretained descriptor snapshot, and borrowed VM handle. Rejected and late
replies, timeouts, and creator or service retirement use the C1A release,
retirement barrier, and private-child cleanup paths. The retain budget is
five seconds. The scheduler rechecks task and run-queue capacity at commit.

## Directory replacement and authorization

Each worker's receipt authenticates its own caller, and each pathname
request carries the shared directory generation independently. The kernel
changes that generation in the existing cwd commit. The VFS service changes
the retained pair for the exact shared group in its exclusive control loop
after accepting the commit, before it handles the next copied pathname
request. Both relative file and stat dispatch check the generation. Old
generation requests are rejected even when their caller receipt still
authenticates. A shared cwd operation cannot start while a worker retain
or a copied pathname operation for that group is outstanding.

The service reuses the new pair's retained handles for existing workers.
The replacements drop each old worker reference, and closing the owner
leaves the current pair pinned by live workers. Rename continues to use the
retained object. The cwd string serves routing and display purposes. It
does not reconstruct the directory reference. A worker created while cwd
was legacy joins through the same copied retention queue on its first
service pathname operation. That syscall parks until admission completes.
The join uses the worker's credentials and the shared owner's current
retained receipt. A successful join consumes eligibility for the initial
join path. Workers with existing admission keep it through successful
service-to-legacy and legacy-to-service crossings. Service loss preserves
service routing for a service-owned cwd and fails closed.

## Qualification and ownership gates

The copied-channel regression covers a delayed private worker, rejected
admission and late acknowledgement after owner retirement, full rollback of
the pending reservation and retained receipt, concurrent cwd change fencing,
multiple admitted workers, relative OPEN, mkdir and stat after cwd rotation
and rename, a legacy-created worker's first service join, credential denial,
and final reference settlement. The host scheduler model exercises the
pending publication boundary. The normal native and Linux syscall adapters
are compiled in the candidate builds. Actual guest dispatch and regression
replays require a guarded boot.

The closure inventory was 71 before and after. Both unchanged gates report
**FAIL** before and after. The guarded ordinary boot is **BLOCKED** before
QEMU. All four historical findings remain:

| Finding | Remaining ownership |
| --- | --- |
| `storage::mount` | Legacy FAT backend construction |
| `linux_procfs::mountinfo_snapshot` | Kernel mount topology |
| `sys_linux_statfs` | Kernel capacity lookup |
| `process::mmap::sys_mmap` | Legacy regular-file descriptions |

Protected SSH producer routing remains deferred to C2I3B. See the
[result matrix](vfs-worker-admission-continuity-results.json).
