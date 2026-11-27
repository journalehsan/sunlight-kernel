# VFS-D2B parked private mmap lifecycle

**PARTIAL overall.** The isolated native lifecycle cases passed on one, two,
and four vCPUs. They cover parked completion, caller exit, rejection of a
computed late PREAD reply, and VFS endpoint withdrawal while the service image
is still alive. Exec during a parked mmap is **BLOCKED** by the isolated
single-executor contract. Ordinary descriptor exec passed in the same guests.
The historical kernel-backed mmap ownership finding remains open.

## Lifecycle contract

A parked transaction belongs to the exact caller PID, process incarnation,
and address-space generation. It retains a reference to the same service
description row as the source descriptor and uses the row's original token
and registration epoch for each PREAD. The descriptor row owns a backend
handle in the service. At most one copied read is pending for the continuation.
The transport correlates a copied reply by registration, caller, request ID
and operation. Replay also checks the original syscall arguments and epoch.

D2A cancels the preliminary region reservation before IPC. While parked,
the transaction holds the candidate address, syscall arguments, a bounded
byte buffer and the description pin. There are no provisional user leaves,
mapping frames or page tables from the transaction at this point. A successful
replay validates placement again, acquires the installation reservation and
installs frames and page tables within the existing VM transaction. Allocation
or installation failure returns resources through the existing rollback.
Commit transfers private frames to the address space, independently of the
source description. The scheduler and capability broker serialize admission
and replay. Scheduler, broker, PMM and VFS locks are released or absent while
waiting for service progress.

| Transition before installation | Terminal disposition and cleanup |
| --- | --- |
| Last read completes under the original image and live epoch | Replay consumes the completion and returns the mapped address once. The buffer and transient pin are released. An ordinary last-reference release uses exact Close. |
| Exact service registration withdraws | File retirement terminalizes the publication and makes the blocked caller ready through the existing wake path. Replay sees lost authority, removes the matching PREAD, drops staging and releases its pin. Native mmap returns the all-ones failure value for internal `FileIo`. An open FD can still hold a local row until close or image cleanup. Withdrawal fences use of its old epoch. The service's epoch disposal owns the remote close. |
| Caller exits while parked | Image retirement removes the continuation, drops staging and releases the pin. The copied publication is removed, and the ordinary exact file-owner barrier closes the remote description. Reap reclaims the remaining FD and VM resources. The retired image receives no mmap return. |
| Caller execs normally | Exec is current-image work. A caller with the sole executor parked in mmap cannot also enter exec. Successful ordinary exec retires the exact old generation before activating the new VM. CLOEXEC rows follow the old-image barrier, and transfer obligations fence that barrier until the surviving descriptions have been handled. Failed exec preserves the old image. |
| Reply follows transaction retirement | A computed reply for the removed publication fails correlation. A reply after endpoint destruction loses transport authentication. Neither completion can recreate a transaction or install pages. |

## Native evidence

The new `vfs_d2b_test` fixture extends D2A and runs after the original
three D1B callers and the existing D2A caller. Three extra retained children
exercise exit, normal completion, and withdrawal in that order. On each
child, the service computes the second PREAD reply and the kernel confirms
that the exact caller is blocked and has no owning CPU before the service
publishes the reply or changes lifecycle state. Each wait has a three-second
bound. The service uses the ordinary copied reply path and the ordinary
endpoint-destroy syscall. Init uses ordinary SIGKILL and waits for status
137. Endpoint withdrawal leaves the service alive until the caller has
consumed its terminal mapping result. The service explicitly disposes of its
retired epoch after destroying the endpoint and records the remaining table
and backend counts before exiting.

| Case | UP, 2 vCPU, 4 vCPU | Evidence |
| --- | --- | --- |
| Caller exit and late PREAD | PASS | The exact image retired with 4096 bytes staged. The pin count fell from two references to one. The old request's computed nine-byte success reply was rejected by the still-live service transport. Exact file-owner retirement reduced service handles from one to zero. Caller VM reap freed 538 owned frames and eight page tables. |
| Normal completion and subsequent mapping | PASS | The second PREAD was acknowledged, the EOF read returned zero, and 4105 bytes committed to two pages. Ring 3 read both pages, unmapped them, and performed another file map/unmap at the same address. Counts returned to 538 frames and four regions. |
| Endpoint withdrawal and subsequent mapping | PASS | Endpoint destruction rejected the held reply, and the service file table dropped from one handle to zero before service exit. Mmap returned `FileIo` internally and left both target leaves absent. Owned frames, region count and cursor matched the parked baseline. The caller mapped, wrote and unmapped anonymous pages at the same address. |
| Caller settlement | PASS | Each lifecycle child reached zero service rows, references and owned frames. Reap found a reclaimed address space, an empty FD table and a released kernel stack. The service backend returned to two open handles in every case, including the withdrawal sample before service exit. |
| Exec with an outstanding mmap | BLOCKED | No supported second executor or external exec operation can replace the parked image. The D1B exec cycles separately revalidated ordinary generation change, CLOEXEC and survivor transfer. |

At all three rendezvous points one page of data was staged and the caller had
538 owned frames, four committed regions, an unchanged mmap cursor, two absent
target leaves, and one service description row with two references. Terminal
handling released the extra reference. On normal and withdrawal return, the
explicit local FD close reduced the remaining row and reference to zero. The
normal case's remote release can arrive through the exact owner barrier when
exit overtakes the asynchronous Close. The preceding D1B and D2A cases each
provide direct final Close acknowledgements. The withdrawn service exits
after its handle sample, so the global generic FD count can fall below the
earlier service-live baseline. The caller-specific counts are unaffected.
Scheduler reap reclaimed the withdrawn service's 22,262 owned frames and
53 page tables before the completion marker on all three runs.

The implementation adds fixture coordination and accounting to the existing
continuation, publication and service dispatch paths. The lifecycle cases
demonstrated no defect requiring a change to mapping, descriptor, IPC,
retirement or memory policy.

## Regressions

- **PASS** for D2A on 1, 2 and 4 vCPUs. Each run checked the seven original
  commits, including nonzero offsets, EOF padding, page protections, private
  writes, cursor preservation and source Close. All six rollback injections,
  repeated unmap, exit with a live page, frame accounting and backend handle
  accounting passed.
- **PASS** for D1B on 1, 2 and 4 vCPUs. Each run repeated saturation,
  compensating Close and retry, explicit final Close, and two descriptor exec
  cycles. **PASS** for the D1A host components.
- **PASS** for fresh N1P populated and unknown FSInfo images. Each completed
  four isolated single-vCPU Ring-3 FAT32 cycles with settlement and lease
  release. The existing checker passed read-only fsck, exact payload checks,
  closed-image comparison and capacity accounting. The previous closed
  images also passed re-verification.
- **PASS** for production kernel and release VFS service compilation.
  **FAIL** for the unchanged ownership guard. It counts 71 references and
  reports the same four unmatched sites in the no-growth check. Ordinary
  production boot is **BLOCKED** by the guard and was **NOT_RUN**. Native
  D3 reclaim and fault restoration of a service-populated page was **NOT_RUN**.

## Coordinated cutover readiness

The components support a staged candidate once the remaining integration
prerequisites pass. Ordinary cutover remains **BLOCKED**. All four historical
dependencies are live.

| Finding | Qualified component | Remaining integration |
| --- | --- | --- |
| Private mmap | D2A retained positional population, D2B lifecycle, D1B descriptor settlement | Authenticate both launch adapters, attach the eager mapper to ordinary native and Linux service descriptors, and qualify the commit and retirement protocol for every admitted sharing case. Remove the historical kernel file population path in the coordinated candidate. |
| ProcFS mountinfo | Service-side caller view and bounded attachment snapshot components | Source topology exclusively from the authorized service view, then fence FD installation against caller, view and topology changes. Preserve the Linux rendering, copyout and retained snapshot lifetime. |
| Linux statfs | Copied metadata admission and validated service attachment components | Implement anchored capacity lookup with a pin to the precise mount incarnation. Preserve the Linux 120-byte ABI and stale, unsupported, lookup and copyout errors. |
| Runtime FAT mount | N1P native FAT32 attachment, bounded block access, lease drain, flush and release on disposable media | Bind existing StorageCtl authorization and policy to the service attachment, qualify production extent and media fencing and retained FAT objects, and prove successor handoff with a single durable owner. |

Implementation order and acceptance gates follow.

1. Qualify both ordinary pre-dispatch namespace adapters and ordinary
   descriptor ownership. Require rights and epoch enforcement, FD install
   compensation, exact final Close, CLOEXEC/survivor ordering, shared-table
   lifecycle accounting and rejected stale replies.
2. Attach runtime FAT to the authoritative service namespace under StorageCtl.
   Require authorized mount, alias exclusion, busy handling for retained
   objects and in-flight population, sync failure preserving the old
   attachment, confirmed drain and flush before detach, media retirement,
   fresh successor identity and N1P closed-image checks. Publish crossing,
   topology and capacity for one mount incarnation.
3. Wire mountinfo and statfs to that authority. Require caller-visible
   topology agreement with StorageCtl, bounded mountinfo snapshot and
   FD-install races, exact Linux ABI and copyout behavior, capacity under
   mutation, and rejection across view, mount or service replacement.
4. Wire native and Linux eager private mmap to service descriptors. Require
   D2A and D2B on UP/SMP plus every newly admitted shared-address-space or
   descriptor ownership case. Gate reclaim compatibility separately on
   D3's native modified-page reclaim and real fault restoration evidence.
5. Remove the historical mmap, mountinfo, statfs and runtime FAT dependencies
   together, alongside the obsolete runtime owners caught by the full
   inventory. Run the unchanged guard before ordinary boot. Require release
   builds, ordinary boot, matched native and Helios syscall/IPC behavior,
   lifecycle settlement and durable closed-image checks before acceptance.

This milestone leaves service-backed mmap opt-in. Broad routing and
`/vfs-scratch` remain blocked. ZRAM policy and the opt-in default for
demand-backed main stacks remain in force.

## Reproduction

Build with
`CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo build -p sunlight-kernel --features vfs_d2b_test`.
Use `tools/make_hybrid_iso.sh` to package the debug ELF. Start with the
diskless D2A TCG arguments at `-smp 1`, then repeat at `-smp 2` and
`-smp 4`. The terminal `[VFS-D1B] complete` follows all seven settlements.
Correlate `[VFS-D2B]` park/terminal records, exact `[VFS-D1B]` request and
retirement records and scheduler reap receipts before recording a pass.

The [machine-readable results](vfs-private-mmap-lifecycle-closure-results.json)
record serial lines, commands, artifact hashes, test counts, and gates.
