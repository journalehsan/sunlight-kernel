# SunFS VFS integration and native contract boundaries

2026-11-14. Design only; no adapter, ABI or mount changes in SFS-0.
Source-backed inventory: `docs/audit/sunfs/sfs-0/repository-capability-inventory.md`.

## Reuse and dependency map

| Existing owner | Reusable mechanism | SunFS dependency / required extension |
|---|---|---|
| sunlight-block | 512-byte BlockDevice, flush, cache, partition views, registry identity | 4096-byte adapter, geometry/capability query, uncertainty/quiescence qualification |
| sunlight-fs Vfs | mount table, enum backend dispatch, mount incarnation, local generation handles, open counts, unmount barriers | SunFS enum variant and engine ownership, no backend bypass |
| vfs/lookup.rs | one credential-aware existing-component walker, canonical FAT spelling, metadata provenance | retained object-relative lookup and symlink-aware bounded traversal |
| kernel fd_table | shared open description/refcounts, consuming take and last-owner release | in-flight object/mount pin through blocking I/O; no path relookup for fstat/openat |
| tmpfs | retained unlinked objects and stable internal inode pattern, limits accounting | reuse lifecycle concepts/tests, not volatile allocation as persistence |
| FatFs | explicit stale-handle rejection, fail-stop poisoning, protected mounts | preserve FAT policy; its unlink revocation is weaker than SunFS retained-open contract |
| native libc Rust wrappers | open/read/write/fstat/file_sync/dir_sync, resolve_path/directory queries | versioned metadata and error mapping; do not fabricate POSIX fields |
| Files and SunShell | current shared query consumers and operation helpers | same native resolver/metadata adapter after identity and directory pins mature |
| vfs_server IPC | legacy separate RamFs/boot snapshot namespace | not a second SunFS engine or native descriptor authority |

Native data path is application → libc/native syscall → fd rights + credentials →
KERNEL_VFS → backend adapter → engine → partition/cache/device. Device registry IDs
are runtime transport identities, not filesystem UUID, object inode or authority.
One writer/exclusive volume owner is required; two VFS instances or raw block clients
cannot concurrently write the same volume. Cloned volume UUID detection and exclusive
device grants are integration requirements; UUID equality alone cannot authorize
mounting or merge object identities across two devices.

## Identity, lifetime and metadata adapter

Adapter opens a retained `NodeId` within a mounted volume and wraps it in the existing
mount/local-generation token. NodeId never replaces the token, fd rights or grant.
Generation wrap fails closed. Dup/fork retain the shared open description and only
last close releases the backend open object. Each active I/O pins the object and
mount independently of fd-number reuse. Unmount refuses while opens, operation pins,
commit, GC, dirty working state or uncertain I/O remain. Successful clean unmount
commits and flushes before releasing device authority; a failed flush preserves
ownership/fail-stop state. Forced detach must revoke, quiesce and report failure,
not simply drop pointers.

`metadata_handle` must return pinned identity even after rename/unlink, without
reconstructing a name. SunFS's `(UUID,nid,incarnation)` is wider than existing internal
`Option<u64>` identity. Do not truncate/hash it to claim authoritative identity.
Introduce a versioned full-width internal metadata object before ABI export. POSIX
ino/dev mapping requires a defined collision-free mount/object translation with
exhaustion behavior and explicit scope; it is SFS-6 work. Native metadata provenance
continues to distinguish Stored/Policy/Unsupported/Unavailable. The current 24-byte
native stat subset remains a subset; unavailable time/link fields cannot become
zeros advertised as facts.

SunFS stores ownership/mode. VFS evaluates uid/gid/root and capability restrictions,
ancestor search permission, sticky directories and mount admission. SunFS evaluates
structural namespace validity and provides object-relative operations atomically.
The integration must not infer authority from filesystem ownership alone or bypass
private-secret/KV restrictions. A permission callback must not reacquire scheduler
locks while VFS or storage locks are held.

## Directory capabilities and path operations

Directory open pins a node, not a string index. Relative lookup starts from that
node under the caller's retained capability and mount policy. Resolver validates
each component before `..`, checks ancestor Execute and bounds symlink expansion
(40 followed links,4095 expanded bytes for the later extended API; current native
path payload255 remains unchanged until versioned). No lexical elimination of a
nonexistent component. Root/mount `..` obeys the caller namespace root; no escaping
through a stored directory parent or stale mount incarnation. Symlinks carry no
permission grant. Absolute targets restart at the authorized namespace root; relative
targets restart at the link's parent. Mount crossing applies current grants.

`openat` and create/unlink/rename-at eventually combine retained parent lookup and
mutation under one transaction. A prior canonical string is an observation, not a
TOCTOU cure. Path-based stat can race by definition; fstat remains pinned. Directory
enumeration needs an explicit snapshot/cookie generation, invalidation behavior on
mutation and bounds; path snapshots cannot be sold as retained enumeration. Cross-
mount rename fails EXDEV. SunFS cannot fix remaining lexical application helpers
without VFS/libc convergence and actual UI evidence.

## Locks, blocking I/O and error ownership

Stage A uses one engine writer lock. Stage B must not place disk waits underneath
the current global KERNEL_VFS spin::Mutex. A future adapter must pin authorized
objects and mount, release outer locks, submit bounded I/O, then revalidate tokens
before returning. Establish lock ordering fd description → mount/object pin admission
→ engine transaction → device submission without holding outer locks during wait;
close/revocation use the same ownership protocol. This is a proposed ordering, not a
claim the present kernel already supports it. Never callback into fd/VFS/scheduler
from engine/device locks. Timeout fencing and retained DMA ownership are mandatory
before retry or buffer reuse. Instrument bounded waits, handle pressure and independent
cleaner progress in SFS-5; host tests alone cannot qualify native races.

Errors retain provenance: malformed disk=corruption, known bounds=invalid/range,
unsupported durability=Unsupported, exhausted admission=ENOSPC/resource error,
revoked identity=bad handle/stale, transport/uncertain writes=EIO with latched cause.
Version the native ABI if existing FsError loses necessary detail. A close must consume
its reference even if cleanup reports failure; durable orphan ownership prevents leaks
from becoming unsafe reuse. No automatic filesystem repair in an application path.

## Staged integration

A. Hosted engine, codec, format/check tools and deterministic fault laboratory only.
B. Secondary explicitly selected disposable SunFS volume; preserve root/FAT/boot.
C. SunShell and Files native scenarios through existing syscall paths, with actual
click-driven Files evidence and retained-open/rename/unlink witnesses.
D. Full metadata/time/link and directory-relative POSIX validation through the
native ABI, including native std overlay. C may start earlier with supported subset.
E. RootFS/installer only after recovery/driver qualification and separate authorization.
F. Resume paused CPython work against authoritative native contracts after D. F need
not wait for installation E; interpreter correctness does not depend on becoming root.
FAT remains available throughout. No current bootloader/fstab/root changes are implied.
