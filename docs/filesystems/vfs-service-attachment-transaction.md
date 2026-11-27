# VFS-N1D: Service-owned mount attachment transaction

N1D qualifies volatile tmpfs attachments under the existing service
`DirectoryNamespace` and its shared `Arc<Mutex<Vfs>>`. The mount slots in that
VFS still own path crossing, metadata and the backend. There is no shadow
registry. The API is for trusted host callers and is excluded from the
freestanding build. No registration, grant, credential, syscall, ProcFS or
StorageCtl routing changes. Production root and FAT authority remain where
[N1C](vfs-mount-authority-storage-handoff.md) recorded them.

## Transaction and lifetime

| State | Contract |
| --- | --- |
| Staged | Own a private tmpfs and copied target, with a weak owner identity and base revision. Require a directory backend root with no outstanding backend handles, a free incarnation slot, and a secure, empty, enumerated target under `/mnt` without mount overlap. Stages neither reserve a visible slot nor exclude competing stages. |
| Published | Under the existing VFS lock, verify the owner and base revision, repeat validation and compare the target object identity. Insert the backend, crossing, ID, source (0 for tmpfs) and protection flag together. Readers of that lock also see the new revision. |
| Cancelled or failed | Drop the stage and its backend. Invalid target, duplicate, changed target, wrong owner, stale revision, or unavailable slot/revision leave the live graph, incarnation credits and revision unchanged. There is no compensating withdrawal because failure never publishes a row. |
| Withdrawal | Match the exact exported ID; reject protected mounts and return busy while the mount has open handles. Volatile tmpfs needs no durability barrier. Existing FAT withdrawal still requires its barrier before deleting the row. Successful withdrawal reveals the underlying directory, destroys the backend and advances the revision. |
| Remount | Stage and publish again. Reuse of a slot gets a fresh incarnation; the former ID cannot address the new backend. A slot retires rather than wrapping its tag. Remount here is two transactions with an observable unmounted interval. |

Revision zero denotes an empty host VFS. Successful insertion, withdrawal
or a changed mount source advances a checked 64-bit revision; an unchanged
source is a no-op. At exhaustion, reads at the final revision remain valid
and mutations that would advance it fail before publication. A slot ID stays
stable while its occupied incarnation is live. The exported ID is tag + 1;
anchored metadata reports slot + 1 and tag / 8 + 1. IDs and revisions are
local to the owner instance.

Host attachment ancestors cannot be renamed or unlinked through the VFS.
Retained directory pairs own two handles and file descriptions own one backend
handle irrespective of duplication. Those handles survive revision changes
until ordinary retirement. A pinned mount cannot be withdrawn; a shadowed
anchor cannot silently turn into an anchor on a replacement backend. A stale
view grants no new access through this API. Existing descriptions keep their
identity, access rights and cursor; PREAD still uses an explicit offset.

## Caller visibility and installation

A trusted host caller binding owns a non-clonable view with retained root and
cwd, captured credentials and the topology revision. Capture brackets the pin
with revision checks. Anchored stat and snapshot capture validate that
revision while holding the owner lock. Snapshot rows come from the retained
root; they are rebased to caller paths and omit siblings and inaccessible
mounts. They carry the exact mount incarnation and visible parent ID. The
maximum is the eight existing slots. The rows are host data; they do not
encode N1A's ProcFS wire schema or backend-root field.

The snapshot borrows its view. The binding owner must keep that borrow valid
through installation and drop/replace the view on retirement or policy
replacement. Installation rechecks the revision and visible rows and calls
the consumer's synchronous publication function while holding the topology
lock. This orders the actual installation against writers, including
visibility changes through directory permissions. The callback must be
bounded and may neither enter the VFS nor park or issue IPC. Copying rows
out for rendering gives no installation authority. Once installed, the
sample is immutable and owns no backend handles.

Production still needs an authenticated registration and caller-bound view
generation, a bounded writer fence spanning the actual kernel FD install,
request/reply framing and quotas, cancellation and retirement cleanup, and
rechecks of eligibility on installed snapshot use. The host callback is
evidence of the ordering contract, not a cross-process lease. Existing
production callers continue using their current views and adapters.

## Future exclusive device lease boundary

The attachment owner consumes a ready backend with an exclusive lease; the
kernel owns admission and transport. A future readiness receipt must name the
transaction, owner registration, normalized physical range (including parent
aliases), media generation, lease generation and I/O rights. It must remain
valid through publication; a stale receipt cannot authorize a backend.

1. **Acquire.** Authorize and reserve an extent, transport credit and buffer
   limits. Fence staged I/O while the predecessor can access any overlapping
   extent. Device reservations confer no namespace visibility.
2. **Drain.** Freeze new predecessor admissions. Drain retained roots,
   descriptions, mapping work and queued requests. Flush the old filesystem
   and require confirmed durable completion, then settle transport and DMA.
   Never park under scheduler, PMM or VFS locks.
3. **Transfer.** Revoke predecessor access before enabling the new lease.
   Probe read-only with a fresh cache, verify media and capacity, and associate
   the ready receipt with the staged backend. Publish only while the receipt
   and namespace reservation remain valid. Keep the target fenced during
   transfer so loss cannot expose a misleading parent directory.
4. **Recover.** Cancel untouched reservations. Regrant the old owner only
   when successor access is revoked and drained, media is unchanged and no
   staged writes occurred. Reprobe with a fresh incarnation if the old cache
   was destroyed. On uncertain writes, reset, or failed drain, keep the
   extent fenced until explicit recovery validates the media.
5. **Release.** Fence admissions and settle all reference, durability and
   transport obligations before reclaiming the lease and backend. Reject
   delayed completions by lease generation. Withdrawal from the namespace
   alone does not prove device release.

Device leasing, fault recovery and fake-transport qualification are an
independent milestone. No real FAT device or cache was transferred.

## Qualification

[Machine-readable results](vfs-service-attachment-transaction-results.json)
record seven focused tests and a full host total of 455, preserving the 448
baseline. Tests verify pre/post publication snapshots and matching backend
lookup, the installation fence, stale and restricted views, rollback without
revision or tag consumption, busy withdrawal, remount identity, revision
exhaustion, and mounted tmpfs frame reclamation after rollback and final close.
Kernel and service builds pass. The unchanged ownership guard still fails
with 71 references and four unmatched findings: statfs, ProcFS mountinfo,
private mmap and runtime FAT mount. QEMU and production integration were
not run. L1C-3 stays blocked, VFS-D1 stays deferred, and `/vfs-scratch`
and demand-backed default main stacks remain disabled.
