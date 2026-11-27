# VFS-D1A: Service-owned descriptor lifetime

**November 27, 2026 — host component qualification passed.** Descriptor
epoch enforcement and failed-install Close are implemented on the existing
copied file channel. Native application descriptor execution was not run and
production VFS-D1 remains blocked.

## Authority and ownership

| Boundary | Owner and lifetime |
| --- | --- |
| Open | The kernel captures PID, process incarnation, address-space generation, credentials and the authenticated service registration, and sends a retained namespace receipt. The service validates this scope and publishes a non-reused token in that registration. The service description owns the cursor and a backend open reference. |
| Installation | The correlated OPEN reply and its publication stay under scheduler, broker and registration serialization through installation. The fd table acquires one row with the original token and epoch, and installs only the requested access rights. Local admission failure rolls back the kernel row. |
| Lookup and use | Read, write, seek and fstat carry the descriptor epoch and exact current caller to admission. The kernel checks the epoch against the live authenticated registration and matches caller, operation, token and epoch when resuming. Read, write and fstat check their descriptor rights; the service independently checks token ownership. An unavailable or replaced service fails closed. |
| Duplication and close | Kernel duplicates share one description row and the service cursor. Only the final row release claims Close. Explicit close and displacement by dup2/dup3 now also require the original epoch. The fd number is consumed before remote cleanup. |
| Exit and exec | Exit discards local fd references and publishes exact-image file owner retirement. Exec drops CLOEXEC rows, publishes transfer of surviving tokens to the new image before allowing the old owner barrier, and filters transfers by epoch. Namespace withdrawal alone does not retire established file ownership. |
| Registration withdrawal | The kernel terminalizes matching parked operations, refuses fresh use of the old epoch and drops old-epoch compensations in the same registration retirement transition. The retained registration protocol owns epoch retirement; the service drops the epoch's file table when it observes retirement, or exact service image teardown destroys it. Local withdrawal is not an acknowledgement of remote cleanup. |

## Compensation

If installation fails after a successful OPEN reply, the kernel overwrites
the existing publication buffer with Close. It keeps the original full
registration, caller, token and correlation ID, and changes the opcode.
Neither a free publication slot nor a new allocation or request ID is needed.
The user receives the installation error and does not park on cleanup. The
same publication remains charged while completion is uncertain, including
after the ordinary file timeout or requester retirement. An exact success
reply or an exact BadHandle reply discharges it. The service removes an
entry only once: Close matches the original caller and token, and a repeated
Close cannot close a later open. Other replies leave the obligation pending.
A successful OPEN reply explicitly rejected by the kernel is different:
the service still owns that token and closes it directly. That also covers
an OPEN that finishes after its kernel publication has been canceled.

These steps use the existing file publication, fd table and retirement
protocol. They do not wait for IPC while holding scheduler, PMM, or VFS
locks. Cursor reads and retained PREAD keep their existing semantics.
The best-effort policy for ordinary final Close publication and the bounded
exec transfer protocol are unchanged; their capacity and timeout behavior
are separate from failed-install compensation.

## Evidence and next gate

The registration harness exercised the real copied publication and retirement
code with host scheduler records and an injected installer outcome. The
service host tests exercised repeated rejected OPEN replies, repeated Close
against a live successor token, and backend handle counts over 100 cycles.
Existing tests cover shared cursor and PREAD behavior, exact caller and epoch
retirement, exec transfer ordering, and N1N split and unsettled withdrawal.
The harness did not execute the native syscall, deliberately exhaust a live
native fd table, or prove an end-to-end native exec or exit cleanup.

All 481 baseline, 59 N1H, seven N1D, three N1L and 14 FAT tests were matched
by name. The production kernel and release service compiled. The ownership
guard still fails with 71 references and the same four unmatched sites:
statfs, ProcFS mountinfo, private mmap and runtime FAT mount. N1P's isolated
native FAT32 evidence stands as recorded; no descriptor test or build here
extends it to ordinary application descriptors.

The next descriptor gate is an isolated native caller with an authenticated
namespace already bound before dispatch: prove actual fd installation,
full-table compensation, duplication and exact-image exit/exec with service
handle counts. Production also needs pre-dispatch namespace publication on
both ordinary child launch paths, and a defined ownership handoff for shared
tables across threads or fork; a routed bit or a queued transfer is not that
proof. Broad L1C-3, mmap integration and /vfs-scratch remain outside this
stage. Runtime FAT ownership, authenticated block I/O, device leases, FSInfo
ordering and the opt-in stack default were preserved.

See [machine-readable results](vfs-service-descriptor-lifetime-results.json).
