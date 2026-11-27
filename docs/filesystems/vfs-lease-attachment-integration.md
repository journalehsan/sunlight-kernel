# VFS-N1K: Lease-to-attachment readiness

**BLOCKED** — the authenticated lease and N1D attachment transaction have no
shared device backend or readiness contract. This stage stops at that boundary.
It adds no runtime connection, mount authority, device authority or route.

| Question | Finding |
| --- | --- |
| Can the authenticated lease prove backend readiness? | It can prove admission to the registered physical extent for the current owner, rights and generations. [N1J](../../kernel/src/vfs_n1j_test.rs) exercises sector I/O through a kernel fixture; the ring-3 service does not receive a block backend. A working sector read does not verify filesystem geometry, root identity or a fresh backend/cache. |
| Can N1D consume that evidence today? | No. The [host transaction](../../services/vfs_server/src/directory/attachment.rs) stages a concrete Tmpfs and captures owner, target identity and topology revision. Its [publication path](../../sunlight-fs/src/vfs/attachment.rs) inserts a RAM backend. Neither path validates a lease or carries one into the installed mount. The host snapshot callback validates revision and rows under the owner lock; it does not fence a native kernel FD install. |
| Does withdrawal drain retained descriptions after hiding the mount? | The [existing unmount](../../sunlight-fs/src/vfs.rs) returns Busy while handles remain; it leaves the mount installed on sync failure. Successful unmount removes the mount and advances the host topology revision. Hiding a mount before closing retained descriptions would require a separate retirement state on the existing owner, and is not the N1D contract. |
| Does mount withdrawal need physical DMA reclamation? | No, while the same live transport retains the queue and bounce allocation and no request or mapping user survives release. Physical reclamation is needed to return pages to the allocator on transport destruction; it requires an independently verified hardware DMA fence and allocator handback. VerifiedRetained reset supplies neither handback nor a reusable live queue. |
| Must live cross-service transfer precede isolated attachment? | No. A single authenticated owner can acquire a vacant disposable device. Successor handoff needs separate qualification before a coordinated cutover that transfers a live predecessor's device. |
| Is more settlement required? | The happy path can use the existing retained-request drain and confirmed virtual barrier. A bounded attach must also keep the owner accounted for and prevent reuse when a probe, flush, transport, or service teardown leaves uncertainty. Fault recovery and allocator return may be deferred if uncertainty stays fenced; successful recovery cannot be inferred from N1J. |

## Smallest implementation boundary

Extend the existing N1D reservation and publication path to accept one real
backend over authenticated I/O for the isolated whole device. Start with one
owner and a read-only backend probe with fresh state; verify media, bounds,
geometry and filesystem root. Bind the resulting readiness to the exact
N1D owner, reservation, target identity and base topology revision, and to
the authenticated registration, normalized extent, rights, media and lease
generations. Use a bounded pin recognized by the existing authorities so
revocation or media change cannot slip between validation and publication.
A copied token or an independent check before taking the mount lock is
insufficient. Do not perform I/O or wait while holding the namespace lock.
Carry lease ownership into the installed backend on success; keep it with
the existing lease authority during failure cleanup. A fixture that
publishes tmpfs next to a leased device would not exercise this connection.

Acquire authenticated admission first, probe privately, then stage and
revalidate for atomic publication. Caller views captured before a topology
change must reject stale use and stale snapshot installation. Reuse N1D's
busy-and-retry semantics for retained descriptions: close new mount opens at
the existing owner, keep the mount accounted for while retained descriptions
close, and continue to allow the I/O needed for closure and writeback.
Coordinate backend sync and the existing lease release transition so settled
data and a confirmed barrier precede successful withdrawal and final lease
release. Keep the extent unavailable to another owner until withdrawal
commits. If staging or publication fails, leave topology unchanged and
drain and release the unpublished lease; on uncertainty preserve the fence.
In particular, a known failed release flush can return the lease to Active
([settlement code](../../services/vfs_server/src/device_lease.rs)); cleanup
must retain responsibility and retry or fence rather than drop the lease
handle and report success. Endpoint exit must invalidate publication and
account for retained namespace and transport work before admitting reuse.

## Evidence and limits

**PASS:** Fresh host runs matched every N1J reference test name: baseline
373 filesystem/IPC/service tests plus 108 registration tests, 33 VirtIO
and 26 lease tests, and all seven N1D tests (reported separately; groups
overlap). Production kernel and release service builds passed. Host N1D
evidence covers tmpfs publication, revision and identity, stale views and
retained handles only.

**PASS (preserved N1J evidence):** One authenticated ring-3 registration
and exact image retirement on the isolated one-vCPU VirtIO fixture.
Across 64 release/reacquire cycles the kernel reused the one queue and
bounce allocation allocated before the loop: four queue pages plus one
bounce page (20,480 bytes), with no allocation per lease. The recorded
run reports 64 reads, 65 writes, 129 confirmed virtual barriers and a
full backing image match. Withdrawal of the registration left zero caller
credits and one retained command, settled after exact image retirement;
final retained commands and mapping users were both zero. This is bounded
reuse across **lease cycles** only. Terminal reset leaves the pages pinned.

**NOT_RUN:** Integrated host and QEMU mount lifecycle, mount-cycle DMA
stability, native FD installation, cross-service native handoff, injected
fault settlement and allocator handback. N1J evidence was inspected, not
rerun. **FAIL:** The unchanged ownership guard exits 1 with 71 references
and four unmatched sites: statfs, ProcFS mountinfo, private mmap and
runtime FAT mount. **BLOCKED:** Coordinated cutover and physical frame
reclamation. No ordinary storage or scratch route is enabled. L1C-3 remains
blocked, VFS-D1 deferred, and demand-backed main stacks opt-in.

See [machine-readable results](vfs-lease-attachment-integration-results.json).
