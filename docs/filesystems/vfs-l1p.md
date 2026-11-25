# VFS-L1P private process reservation and reclamation

| Status | Scope |
| --- | --- |
| IMPLEMENTED | Bounded local pending-child ownership, reservation-aware PID allocation, and unpublished-child destruction. |
| HOST_VALIDATED | Production transaction state, deterministic arbitration, and subsystem regressions. |
| NATIVE_CANDIDATE_VALIDATED | Passed for the local-only fixture at 2 GiB on 1, 2, and 4 vCPUs. |
| PRODUCTION_QUALIFIED | Blocked by the four existing primary ownership findings. Both ordinary spawn paths still use their existing publication sequence. |

VFS-L1P is a provisional local-resource foundation. VFS-L1 publication, VFS-L1C integration, and VFS-D1 remain open. [Machine-readable results](vfs-l1p-results.json)

## Ownership and visibility

The scheduler owns eight pending creation slots alongside its ordinary task storage. Each slot holds a transaction and a private PID claim. Once construction succeeds, the slot owns a boxed `Process`; the creator holds a non-cloneable RAII owner that cancels on drop. Only the creation interface can obtain the transaction's exact identity. The payload has no ordinary task index. Process lookup, process enumeration, procfs, ordinary process-control operations, and dispatch continue to use the ordinary task table. Cancellation leaves no Finished or Reaped row in that table.

Constructing reserves a slot before any child resources are acquired. The creator owns partial resources during this state. The production constructor uses the existing fallible process constructor, ELF loader, user and kernel stacks, and initial context preparation without activating the VM. It records the exact parent PID, incarnation, and image generation. It preserves the supplied child UID/GID, copies the parent's existing capability values, and can retain canonical initial stream descriptions from the parent. Parent identity and liveness are rechecked when the private child is installed and when an owner requests its identity. A disappeared or replaced parent at the install check sends the child to local rollback. After installation the creator remains accountable for cancellation or drop; there is no asynchronous parent-exit sweep in this phase.

The descriptor interface permits freshly constructed standard streams and inherited standard streams with the same canonical handle. Inheritance of general backend descriptors, shared mappings, session permits, broker grants, and remote references requires a wider destruction contract. The constructor creates none of those resources. Ordinary spawn and fork/thread paths use the reservation-aware PID helper, and ordinary insertion checks for private claims. They do not call the private constructor.

## Source changes

- `process/pending.rs` and `process/pending_state.rs` implement the private constructor, bounded claims, parent and child identity checks, and sole-owner cancellation.
- `sched/mod.rs` owns the slots, includes their PIDs in allocation, and checks for collisions at ordinary insertion. The embedded/session spawn path, syscall spawn, fork/clone and thread creation, and IPC race fixture use the same allocator.
- `process/mod.rs` makes the initial kernel stack, cwd, and boxed descriptor setup fallible before root allocation. Root-allocation failure releases the initial descriptions. `process/fd_table.rs` releases partial descriptor setup and retains canonical inherited descriptions without acquiring a backend lease.
- `process/spawn.rs` reuses nonactivating image loading for the private constructor. Stack and proc-argument heap allocations are fallible and expose source-bound fault checkpoints. The ordinary initial-image rollback still checks the same three canonical handles.
- `mm2a_host.rs` runs the actual transaction slot tests. The debug-only `vfs_l1p_test` feature adds the native fixture before first dispatch in `main.rs`.

## Identity and ownership decisions

The existing allocator uses checked max-PID-plus-one across the retained ordinary rows, including reaped rows. Its input now includes pending claims under the same scheduler lock. Reusing an ordinary task slot still follows the existing reap, current-core, stack pin, child wait, and watcher conditions. The pending constructor limits PID allocation to the physical-page owner field's range. Numeric PIDs can be reused when absent from both populations. Pending identity reuse is independent of VFS registration-slot recovery.

A checked monotonic transaction serial distinguishes reservations even when the numeric PID is immediately reused. The constructed identity also includes the nonzero process incarnation and final VM generation. Construction may replace the initial empty address space, so that empty root's generation cannot represent the finished image. A copied identity is a correlation value and retains no local resources. An in-progress constructor or detached cancellation winner is an actual resource owner and keeps the slot charged until teardown finishes.

| State | Decision and owner |
| --- | --- |
| Constructing | The slot reserves the PID. The constructor owns partial resources. |
| Privately reserved | The slot owns the complete child. The creator's RAII handle controls local cancellation. |
| Cancelled | A serialized state change fences completion. If the constructor still owns partial resources it remains responsible for teardown. Otherwise the cancellation winner takes the only child payload. |
| Reclaimed | After rollback and Rust resource drops finish, the exact slot is cleared under the scheduler lock. A stale identity cannot find it again. |
| Commit eligible / ownership transferred | The host-only transition exercises the same payload and serialization without inserting into a scheduler table. Test transfer consumes the sole payload and fences later cancellation. Production eligibility and transfer await authenticated publication. |

The cancellation winner releases the scheduler lock before entering PMM, releases the initial open-description references, and calls the existing inactive-address-space reclaim primitive. That primitive enforces retirement-journal and active-CPU checks, releases user frames and page tables, and tears down the region ledger. The existing initial descriptor rollback retains its canonical-handle assertions. Kernel stack, task, capability vector, and other Rust-owned storage drop once before the PID claim is released. The scope is one initial task and a private image with no out-of-band owner or consumer. Shared-memory cleanup, deferred IPC, live TLB shootdown, and remote compensation have no pending-child producer in this interface.

## ZRAM and VM lifetime

Current pressure reclaim resolves a candidate's numeric PID through the ordinary scheduler table and checks its anonymous region, PMM owner, and present PTE. The private loader installs image and user-stack mappings and does not enroll anonymous frames. Diagnostic direct fill targets the current task, and fault restoration also requires an executing image. Pending images are therefore excluded from production pressure selection and do not acquire swap slots in this phase. The fixture checks that the candidate count stays at its starting value. The existing VM teardown still owns untracking and swap metadata release where applicable. This path adds no userspace IPC or VFS dependency to memory reclaim.

Independent ZRAM work remains actual fault-driven swap-in, exit and exec with swapped pages, re-enrollment after restoration, tmpfs backing reclaim, and service-backed private mmap integration. The default 1 GiB boot failure during Service Manager allocation also remains open. The earlier 12-page direct-call restoration result remains KLC-Q1 evidence.

## Deterministic tests and gates

The host library compiles the production slot/transaction code directly. Three tests cover 100 iterations of same-PID reclamation and reuse, capacity and collisions, stale incarnation/image/transaction keys, parent disappearance during construction, construction cancellation, repeated cancellation, and both deterministic commit/cancel orders. Threads synchronize through a host mutex and channel, and the payload is a counted drop sentinel. The host tests do not exercise the actual scheduler queues, page tables, PMM, or guest descriptor backends.

| Gate | Result |
| --- | --- |
| Registration | 94 passed, including 100 exit/replacement and 100 mixed lifecycle cycles. These existing fixtures model scheduler/VM transitions. |
| IPC | 168 unit and 9 integration tests passed. |
| VFS service | 47 passed. |
| Descriptor harness | 18 passed with production description logic and mocked backend boundaries. |
| Focused scheduler and process lifecycle | 76 passed. |
| Kernel host | 216 passed, including the three new transaction tests. |
| Resource retirement | 34 passed. |
| ZRAM and swap-slot filters | 4 and 1 passed, respectively. |
| Compile checks | Primary kernel binary passed. Transaction/session and Helios mmap feature checks passed in the reduced candidate. |
| Unchanged ownership guard | Primary 71 references and four findings before and after. Candidate 68 references and zero findings. |

## Isolated native matrix and equivalence

The fixture drives the production constructor and cancellation entry points before ordinary first dispatch. It uses the real scheduler, PMM, address-space ledger, ELF loader, kernel stack allocator, and descriptor pool. It checks private lookup exclusion, run-queue counts, no current task with the private PID, reuse of a cancelled PID, rejection of its old transaction, and complete restoration of frame, heap, descriptor, and reclaim-candidate counts. It rejects a stale parent, an invalid executable, and an unsupported inherited descriptor. Fault injection covers the child box, capability copy, kernel stack, cwd, descriptor table and its three records, argv/envp address vectors, startup stack table and proc args. The native fixture also fills the real VM ledger to reach both root-allocation ledger failure paths and walks the PMM allocation ordinal through image construction. Ordinary init and service progress after the fixture establishes post-cancellation forward progress. Coverage is BSP fixture execution on guests with 1, 2, and 4 vCPUs; the concurrent terminal ownership interleavings are host observations.

| vCPUs | Guest | Cycles | Heap faults | Ledger faults | PMM faults | Free frames | Heap bytes | Descriptions / refs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | PASS | 100 | 12 | 2 | 523 | 266448 | 176832 | 72 / 72 |
| 2 | PASS | 100 | 12 | 2 | 523 | 266190 | 177856 | 72 / 72 |
| 4 | PASS | 100 | 12 | 2 | 523 | 265678 | 179904 | 72 / 72 |

The unchanged ordinary boot gate passed at 2 GiB and 2 vCPUs. The fixture table shows the baseline values restored after cancellation and fault rollback. The matrix covers the fixture constructor with real resources and leaves both existing production spawn adapters outside the native qualification.

The candidate retains the six previously reviewed KLC-Q1 source reductions covering private file mmap, statfs, procfs mountinfo, and FAT capacity scans. The phase files match byte-for-byte except for those retained reductions in the syscall and boot source. Verification covered 12,346 baseline paths and found zero unexpected candidate drift. Exact hashes and the residual overlays are recorded in the equivalence evidence. The guard, ordinary boot script, ISO helper, IPC admission/registration source, and VFS service entry point match the primary. The primary keeps its Btop and Qalculate compatibility code and existing trusted VFS provenance.

## VFS-L1C interface and remote ownership

A future transaction must pin the parent namespace and bind a separate bounded registration request to the exact service registration, epoch, endpoint, request ID, creation ticket, child PID/incarnation/final VM generation, and trusted credential snapshot. Admission and dispatch must authenticate the launching context and trusted service, without treating arbitrary pending PIDs as normal VFS callers. Provenance remains boot-only. Reserve capacity for local cleanup, pending operations, scheduler insertion, and any session or broker handoff before publishing work that can create a remote obligation.

Validate and correlate the acknowledgement before eligibility. Revalidate parent identity and image, inheritance and session permissions, service epoch, and transaction ownership at the serialized publication point. The winning publication path must move ownership and the PID claim to the ordinary scheduler slot atomically and enqueue once. Refusal, deadline or caller cancellation, parent exit or exec, and epoch retirement must fence publication under the same arbiter. A late success after cancellation can only be handed to exact-identity compensation.

Future local reclaim may finish before remote settlement. A bounded owner must retain the service obligation and its correlation data until authenticated refusal, acknowledged compensation, or a qualifying retirement proof with its drain conditions. KLC-1 and KLC-1B cover exact last-owner exit and qualifying successful exec of the embedded service's private backing. Timeout, endpoint withdrawal alone, and replacement registration cannot certify cleanup. Until every numeric-PID dependency is discharged, the PID claim must stay charged. Any later proposal to release it with surviving exact-identity compensation must prove that all retained references are generation-fenced and have no numeric-only effects or publication path. This phase implements local-only ownership and carries no remote settlement records.

## Evidence paths

- Phase logs and exit codes are under `target/vfs-l1p/`. Native image hashes, candidate/ordinary boot logs, and per-CPU launches are under `target/vfs-l1p/native/`.
- Full prior KLC-Q1 evidence is preserved under `target/vfs-l1p/klc-q1/`. The original remains at `/tmp/klc-q1/evidence/`.
- Source verification is in `target/vfs-l1p/source-equivalence-final.json`. Exact gate and scenario paths are indexed in the results JSON.

[VFS-L1](vfs-l1.md) · [VFS-L1A](vfs-l1a.md) · [VFS-G0](vfs-g0.md) · [Migration history](userspace-vfs-migration.md) · [KLC-1](../kernel/klc-1.md) · [KLC-1B](../kernel/klc-1b.md)
