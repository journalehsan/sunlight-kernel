# VFS-N1A: authoritative ProcFS mount snapshots

2026-11-27. **PARTIAL. The historical ProcFS finding remains open.**
The kernel can take a coherent sample of its own global mounts. The registered
VFS service cannot take that sample for it: no transaction binds the live
mount owners to the caller's retained namespace. N1A adds host regressions
and a protocol design; it does not publish a new ABI or change routing.

## Who owns the rows today?

| Attachment | Authority and limits |
| --- | --- |
| Global `/` | [Kernel boot initialization](../../kernel/src/main.rs#L2563) publishes a seeded tmpfs; ordinary root-backed directories are not separate mounts. The service builds a different tmpfs from the seed. Its retained objects identify its own tree. |
| Protected FAT | The same initializer may select **one** of `/boot`, `/state`, or `/mnt` and attach its registry extent. The [service fstab handler](../../services/vfs_server/src/main.rs#L423) may expose a read-only BootFs share through legacy handlers; this neither publishes nor owns the disk attachment. |
| Runtime FAT | [Kernel storage](../../kernel/src/storage.rs#L102) and [Vfs](../../sunlight-fs/src/vfs.rs#L1053) validate the target/source, publish the backend, and remove it after busy and sync checks. The device registry establishes source identity, not attachment or visibility. |
| Virtual paths | ProcFS and virtual device/descriptor handlers are not mounts in this table. There is no scratch mount. |

## Consumer and compatibility trace

The sole live ProcFS topology read is [mountinfo_snapshot](../../kernel/src/linux_procfs.rs#L172).
The [open path](../../kernel/src/arch/x86_64/syscall_procfs.rs#L5) looks up
`/proc/self/mountinfo` under scheduler serialization. The Linux caller must
see its own leader under the ordinary UID/group visibility rules; neither
task nor shared process may use file routing and the shared root must not
be rebased. Metadata-only routing does not disqualify the global view.
Numeric PID mountinfo, `/proc/mounts` and `/proc/self/mounts` have no virtual
node. Preserve that surface; do not substitute mountinfo text for a mounts
table or introduce aliases as part of a source change.

Open drops scheduler serialization, locks VFS then devices, formats one
complete sample, then rechecks node liveness (leader PID/incarnation/VM and
current caller eligibility) before installing the description. Lookup, stat,
access and directory enumeration do not enumerate topology. Read/pread and
seek use the stored bytes and check liveness; fstat checks liveness too.
Short reads and dup share ordinary description semantics; pread does not
advance the offset. Drop releases the ProcFS open credit (128 descriptions).
The sample holds no mount open. Missing kernel VFS gives EIO, unrepresentable
metadata EOPNOTSUPP, oversize output EOVERFLOW. A hidden node gives ENOENT.
The current recheck has no topology revision or historical view-change token.

The [formatter](../../sunlight-fs/src/vfs/mountinfo.rs) emits at most eight
mounts and 8192 bytes for ProcFS. Longest path first, tag breaks ties;
nearest strict component ancestor is parent, and root parents itself.
Backend root is `/`; mountpoint escapes space, tab, newline and backslash.
Each line has mount and parent IDs, major:minor, root, mountpoint, `rw`,
separator, type, source and super options `rw`. No invented optional fields.
Root tmpfs is 0:0; other tmpfs uses 0:mount-ID. FAT requires a registered
FAT32 source in [1, 2^20), uses 240:source and `helios-block:<source>`.
Protection from unmount is not a read-only flag.

## Why registration is insufficient

The [registry](../../ipc/src/vfs_rpc/registration.rs) binds endpoint, owner
PID/incarnation/VM and a non-recycled registration epoch (called
`mount_epoch` on the wire). [Admission](../../kernel/src/ipc/vfs_resources.rs)
pins exact caller PID/incarnation/VM, registration, namespace ID/generation
and root/cwd receipt objects. Credentials, supplementary groups and grants
are authenticated request inputs. A directory receipt is not a mount list;
the [context codec](../../ipc/src/vfs_rpc/context.rs) admits Stat with a
metadata grant. Neither retained-directory transitions nor copied file
operations serialize topology.

In the kernel VFS the exported ID is tag+1. The anchored identity is
(slot+1, tag/8+1); slot reuse advances the tag by eight, without wrapping
an exhausted slot. These identify individual attachments, not a revision of
the whole table. A source may be assigned without changing the tag.
StorageCtl LIST_MOUNTS (admin) and INVENTORY_MOUNTS (public) read one global
indexed row per call, with no root pin or common revision across calls.
Polling them, querying stat per path, or replaying fstab would miss or mix
changes. A forwarding wrapper around the kernel read would retain the finding.

## Proposed request (not implemented)

1. **Admission.** Use a distinct read-only snapshot operation and grant.
   Capture non-reused request ID, exact task and shared leader/image, active
   registration, retained namespace/root/cwd, credential and grant snapshot,
   and a non-reused view revision. Authorize under existing kernel policy;
   service verifies kernel origin, active binding and pins. Never accept a
   caller-selected namespace, root path, or mount ID as authority.
2. **Publication.** The single namespace attachment owner must publish
   mount/unmount, source and visibility changes with one non-wrapping topology
   revision, including metadata used to serialize each row. Failed staging
   leaves the old revision intact. Read the entire visible set under one
   revision, resolving from the pinned root. Never return a hidden ancestor,
   host mount path or sibling. Initially decline unsupported rebased views;
   supporting them requires root-relative paths, backend roots and visible
   parent semantics. Do not join two independently sampled trees.
3. **Bounded reply.** One immutable reply, no cursor. Proposed v1 uses
   little-endian fixed-width integers: version, row count (u32 each), request
   ID, registration epoch, namespace ID/generation, view and topology revisions
   (u64 each). Request ID correlates all other retained request inputs. Each
   row carries slot ID, generation, Linux mount ID, parent Linux ID and source
   registry ID (u64), type and flags (u32), byte lengths (u32) for backend root
   and visible mountpoint followed by their exact UTF-8 bytes. Define v1 type
   1=tmpfs, 2=vfat; flags bit 0=rw, bit 1=protected. Source 0 for tmpfs.
   Derive major/minor, labels and options by the existing formatting rules.
   Limit to 8 rows, 16 KiB entire wire reply (within MAX_TRANSFER), 8192 bytes
   rendered. Checked lengths; reject trailing bytes, unknown types/bits,
   duplicate IDs/paths, invalid identity, missing root, noncanonical paths,
   inconsistent parents/source, or overflow. No partial output or truncation.
4. **Commit.** Reserve bounded request/output and cleanup credit before
   dispatch; park without scheduler, VFS or device locks. Match exactly one
   authenticated completion. Revalidate task/leader, policy, profile, root
   and live binding before install. A publication token must order the
   revision check and FD install against both view change and withdrawal
   (e.g. a bounded read lease fenced by the writer). A second RPC asking
   whether the revision is current would still race. Reject ABA even if
   credentials, paths or routing flags return to their former values.
5. **Transitions and loss.** Hidden view: ENOENT; admitted but changing
   topology/view: EAGAIN and discard; unavailable or retired provider: EIO.
   No stale cache, synthetic rows, automatic retry across epochs or kernel
   fallback. Fence admission on retirement, discard late replies, settle
   request/pins exactly once after cancellation, timeout, exec, exit or FD
   exhaustion. An installed description retains only its immutable sample and
   authorization identity: later mount churn does not splice in new rows.
   Recheck view authorization on use; refuse after view replacement or
   retirement. Last close frees the sample, not a mount. Service loss has
   no new effect on today's independent kernel-view description.

**Dependency for cutover:** publish authoritative namespace attachment
transactions at the same boundary used by path crossing and live FAT
publication/withdrawal, and fence caller-view changes through install. No
such service owner, revision or commit fence exists. Qualify that boundary,
then the actual wire decoder (including malformed frames), concurrent
transitions, lost/late completions, cleanup and ordinary guest reads before
removing the kernel read. No second writable topology inventory is needed.

## Qualification

See [machine-readable results](vfs-authoritative-mount-snapshots-results.json).
New tests exercise the existing FAT formatter across detach, slot reuse,
unavailable source and exact output limits; a separate host test exercises
two real caller bindings, withdrawal, pin release and registration loss.
These are component results, not a service snapshot transaction. The unchanged
guard fails on ProcFS, statfs, private mmap and runtime FAT. Ordinary boot
stops before QEMU. L1C-3 remains blocked, D1 deferred, scratch unpublished.
