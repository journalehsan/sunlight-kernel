# VFS-L1 — blocked at namespace retirement

[VFS-L1P](vfs-l1p.md) implements bounded private construction and local
cancellation. [VFS-L1C](vfs-l1c.md) records the request-correlation change and
the remaining production publication and settlement work.

[VFS-L1A retirement audit](vfs-l1a.md): descriptor host integration is repaired. KLC-1 connects private-image exit to settlement with passing host recovery. Native qualification, private child publication and VFS-D1 remain blocked.

Neither ordinary spawn adapter satisfies the publication invariant. The change in this phase repairs early construction rollback. It does not enable descriptor routing or qualify first dispatch.

## Outstanding retirement qualification

A service can acquire a namespace lease and exit before its reply is consumed. Abandonment prevents installation but leaves the request owned. The unchanged host regression reproduced four retired epochs with unanswered leases and failed admission for the next registration. An exact last-owner exit proof now lets the VFS runtime collect the retired epoch when its local namespace bindings drain. Host validation recovered at full capacity and completed 100 replacement cycles. See [KLC-1](../kernel/klc-1.md).

The exit proof covers the embedded service's private lease backing. [KLC-1B](../kernel/klc-1b.md) adds a separate successful-exec proof for that same backing after VFS withdrawal and old VM reclaim. Failed exec preserves authority. Endpoint-only withdrawal remains a cancellation fence, and a live, unresponsive owner can retain an unresolved obligation indefinitely. The ordinary ownership guard prevents native qualification. Host-only provisional integration requires a project-owner decision and the private child reservation, publication and rollback protocol below. Reusable registration slots establish no PID reservation or release guarantee. The [readiness matrix](../kernel/klc-1b.md#validation-and-integration-decision) specifies which decisions and resources VFS-L1 must own independently.

## Required contract and present implementation

| Transition | Owner and condition |
|---|---|
| Constructing → pending | Creator reserves identity, scheduler insertion capacity and rollback capacity, then moves the private Process into one bounded transaction. Only the creator and authenticated retention code may address it. No PID lookup, procfs, signals, enumeration, run queue or dispatch. |
| Pending → binding committed | Authenticate exact registration, request, target PID/incarnation/VM generation and pinned parent binding. Derivation uses trusted credentials, cwd and grants. The VFS must never depend on the child running. |
| Committed → published | Under scheduler, broker and registration serialization revalidate parent identity/VM, current binding, session permit, service epoch and pending ownership. Move into an ordinary scheduler slot and enqueue once before unlocking. The slot insertion is the intended visibility point; dispatch follows. All fallible capacity admission must precede it. |
| Constructing/pending/committed → cancelled | Under the same serialization withdraw the private transaction from commit eligibility and transfer its sole cleanup ownership. This take is the intended cancellation decision point. Refusal, invalid authority, transport failure, deadline, parent exit/exec, changed inheritance or service loss cannot authorize publication. Invalid replies must never consume another request. |
| Cancelled → reclaimed | Dispose of the private local image outside scheduler serialization; separately settle delivered requests, release receipts and retire continuations. Release the PID reservation last. Late success owns compensation only. Published children instead enter ordinary exit/reaping. |

These are the required integrated transitions. VFS-L1P supplies a private registry and reservation across unlock. Production publication/cancellation arbitration and the spawn continuation remain open. Today both adapters allocate max-retained-PID plus one under the scheduler lock. Process construction allocates the incarnation, kernel stack, descriptor table and VM; image loading replaces the initial root. The authenticated target must name the **final** VM generation. The current adapters still insert at `Scheduler::add_process_after_reaping`, provision inheritance by table index and enqueue. Ordinary authority is not established at insertion. Session bootstrap eligibility is consumed by broker commit: deferral must retain the opaque permit and commit it only after the final checks.

## Local rollback change

Both `process::spawn::spawn_with_session_attachment` and `sys_spawn` retain the initial image as a private Rust value. Its default descriptors own three records in the global open-description pool; dropping the boxed FD table did not release them. The common early rollback now takes those descriptors before reclaiming the inactive image. The syscall checks session validity before acquiring inherited stdio references. The stack and ordinary Rust fields drop as the constructor returns. No fake process or reaper insertion is needed for this early failure.

This helper applies only to the initial standard descriptors. It does not destroy an arbitrary pending child with inherited handles, shared resources, timers or delivered IPC. It uses the existing construction locks and VM reclamation assertions; moving general cancellation cleanup outside the scheduler lock remains unimplemented. There is no exactly-once or forward-progress qualification for that later stage.

Embedded init, VFS and device-only tty bootstrap retain their existing launch policy (see the [namespace inventory](../Helios_LinuxCompat/filesystem-namespace.md)). No new exception was introduced. The ordinary gate fails before QEMU. UP/SMP delayed acknowledgement, both cancellation winners, first dispatch, namespace-dependent first operation, native exit and repeated resource accounting are NOT_RUN.

See [machine-readable results](vfs-l1-results.json) for gate outcomes and logs. VFS-D1 remains blocked. Its two separate repairs remain deferred: validate the stored descriptor epoch on lookup, and send authenticated compensating Close if descriptor installation fails after service Open.

## VFS-G0 follow-up

The ordinary boot gate still fails at the unchanged ownership guard. Prior native qualification of Btop storage telemetry blocks the proposed deferral. Pending-child integration remains BLOCKED. The owner excluded host-only integration in this phase. [VFS-G0 report](vfs-g0.md)
