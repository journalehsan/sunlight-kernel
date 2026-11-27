# VFS-C1 coordinated ownership cutover candidate

**PARTIAL integration. BLOCKED for ordinary acceptance.** C1 connects capacity
lookup to the caller-scoped service attachment owner in the existing isolated
path. The four live kernel dependencies are still present. The unchanged
no-growth guard failed, so the ordinary one-vCPU, 2 GiB boot and subsequent
CPU configurations were **NOT_RUN**.

## Disposition by ownership finding

| Finding and current live dependency | Existing replacement and C1 disposition | Mandatory integration and acceptance |
| --- | --- | --- |
| **BLOCKED — Private mmap.** `process::mmap::sys_mmap` still obtains a kernel VFS handle, stats it and populates the Linux private mapping from the kernel backend. The copied route is selected for the D2 fixture by the native syscall adapter. | D1B provides exact descriptor installation and retirement. D2A/D2B pin the exact service description and stage bytes before VM commit. C1 leaves production mmap routing at its existing setting. | Admit ordinary native and Linux service descriptors before dispatch. Preserve Linux flags and result handling when selecting the same retained PREAD/VM continuation from both syscall paths. Revalidate the exact caller image, shared owner, epoch, rights and placement through commit or rollback. Requalify D1A/B, D2A/B and newly admitted shared-FD and shared-VM interactions, including late completions, source Close, exit, ordinary exec, placement and frame accounting. |
| **BLOCKED — ProcFS mount topology.** `linux_procfs::mountinfo_snapshot` holds the kernel VFS and registry locks; open installs a text snapshot after a caller liveness recheck. | N1D's service `CallerView` and bounded snapshot/install fence read the existing attachment graph. C1 uses the same caller view for capacity. | Carry the authenticated task/leader, view, epoch and topology revision through one bounded copied snapshot and an FD-install commit fence. The synchronous service callback cannot span IPC. Bind rows to the published backend type and registered source. Preserve the supported `/proc/self/mountinfo` surface, visibility, escaping, byte bound, retained bytes and FD/copyout behavior. Test races with policy or topology changes, overflow, hidden mounts, dup/read/seek and agreement with StorageCtl. |
| **PARTIAL — Linux statfs.** `sys_linux_statfs` performs kernel pathname resolution and reads kernel FAT capacity by mount ID. | C1 now resolves capacity against retained root/cwd under the service owner's topology mutex. It returns the full mount-incarnation ID with the count. Host and isolated native coverage exercise this service-side lookup. | Reuse the authenticated caller and policy admission at the copied syscall boundary. Carry the result and exact mount identity through the reply fence before serializing the existing 120-byte Linux record and copying it out. Preserve lookup, unsupported medium, stale request and bad pointer behavior. Verify path/permission parity, mutation, churn and registration replacement, and match identity with the other two mount consumers. |
| **BLOCKED — Runtime FAT construction and publication.** `sys_storage_ctl` still calls `storage::mount` under the kernel VFS mutex. Mount inventory and unmount use that table. The kernel boot path also constructs a live FAT mount for an eligible `/boot`, `/state` or `/mnt` volume. | N1D–N1P provide reservation, lease and bounded sector primitives, Ring-3 FAT32, split withdrawal and durable settlement. The native path still acquires a fixture extent through the test-gated block control and runs before the ordinary service mount loop. | Admit a real registry device/partition and an existing authorized StorageCtl request. Pass one exclusive, alias-fenced extent lease to the registered service and publish retained objects, source, protection and capacity in one incarnation. Move mount, unmount, inventory and every crossing or operation on the migrated mount together, including boot-selected writable FAT. Test target policy, protected mounts, USB/VirtIO support within the current contract, overlap exclusion, busy pins, in-flight I/O, failed settlement, confirmed flush, removal and reacquisition after release. Recheck both N1P closed images and ordinary persistence. |

## Integration boundary

The native `sys_spawn` adapter and the `process::spawn` path used by
in-kernel callers still enqueue before namespace inheritance has produced
an authenticated acknowledgement. The retained-child path already
owns unpublished PID/image reservation, reply correlation and rollback. The
smallest common prerequisite is to put both ordinary adapters' final enqueue
and success return behind that existing commit. Preserve per-adapter image,
stdio, session and credential setup. Provision the caller's admitted file
rights explicitly. Validate delayed acknowledgement, refusal, requester
exit, timeout, service withdrawal and epoch replacement before first
instruction. The direct init/VFS bootstrap needs its existing readiness
handshake. [Namespace and descriptor gates](userspace-vfs-migration.md#coordinated-candidate-after-d2b)

The normal VFS service uses `DirectoryNamespace<NullDevice>`, and its
policy bindings and file table also use the default backend type. The native
fixture uses a separate `DirectoryNamespace<Device>` with hard-coded extent
bounds. The next storage integration must give the ordinary runtime, retained
policies and description table one block-capable namespace. The authenticated
device interface must bind the exact registered image and lease generation
to registry geometry, extent and rights, while covering whole-device,
partition and internal-writer aliases. A production publication needs the
real source and protection at insertion. The fixture currently publishes
source zero. FAT has path-based open state, and anchored metadata supports
the attached FAT root only. Ordinary retained FAT root/cwd and file identity
through close, rename, unlink and withdrawal remain prerequisites. Capacity
lookup can use path existence and the mount incarnation without relaxing
those identity requirements. [Storage handoff](vfs-mount-authority-storage-handoff.md)

A mount transfer includes path resolution, metadata, directories, open and
create, reads, writes, seek and positional access, mutation, sync, private
mmap and executable loading wherever these operations are supported on that
mount. Move the StorageCtl writer, service path crossing, ProcFS reader and
statfs reader with the same publication and retirement ordering. Broadly
enabling the current routed open would drop supported operations, including
creation and directory descriptors. Boot share reads and protected boot FAT
publication need an explicit compatibility boundary. A read-only boot extract
can remain a bootstrap input, but a migrated mount can have only one mutable
owner.

Cross-caller descriptor accounting is mandatory for supported shared tables.
Preserve the existing rejection of process fork and unsupported clone modes.
The parked D2B caller has one executor. Concurrent exec of that parked
image remains **BLOCKED** within that contract. Ordinary exec and CLOEXEC
settlement remain required. Live successor-owner handoff and physical DMA
reclamation are deferred for the qualified bounded retained-buffer lifecycle.
The mandatory storage boundary is old-lease fencing, confirmed settlement
and release before any subsequent admission. D3's service-populated page
reclaim and real fault restoration are a separate evidence gate. No new
filesystem format or cache/transport architecture is needed.

## Changes and evidence

- The attachment owner now performs anchored path traversal, permission and
  existence checks, and capacity reads under one lock. The existing anchored
  object operations keep their stable-identity lookup. Capacity returns the
  same full ID as snapshot rows. A topology change invalidates old caller
  views, and a fresh view fails with busy while the attachment drains.
- The isolated host and Ring-3 FAT attachment paths sample the FAT directly
  with a 1024-sector bound. The host regression checks the snapshot/lookup
  ID, a case-insensitive file lookup, confinement, missing components,
  unsupported capacity, capacity after allocation and drain fencing. The
  Ring-3 fixture verifies the mount ID and expected cluster delta through the
  same service lookup during each existing N1P cycle.
- **PASS** for the filesystem, FAT, IPC, VFS service, VirtIO and registration
  host suites, the release kernel and VFS service builds, and both release
  builds with compressed boot resources enabled.
  Fresh name matching covers all 481 N1G baseline, 59 N1H, seven N1D,
  three N1L, 14 N1P FAT, and 522 D1A recorded host cases. Counts overlap.
- **PASS** for fresh isolated N1P native runs on one vCPU and 2 GiB, four
  cycles each for populated and unknown FSInfo. Both images passed the
  read-only consistency check, exact payload and full-byte comparison.
  The independently scanned free count fell from 127004 to 127003.
  Each cycle settled with zero outstanding commands and mapping users and
  retained the existing 20 KiB DMA allocation. Earlier standalone native
  storage probes retain their original evidence.
- **PARTIAL** for the fresh isolated D1B/D2A/D2B matrix. On one and two vCPUs
  the original serial assertions passed. The three four-vCPU guests reached
  completion and passed the D1B assertions. The full verifier rejected the
  first four-vCPU log for a missing D2A backend-baseline transition marker.
  Two retries recorded the normal D2B close with zero service handles and
  an unacknowledged reply as caller exit retired the request. The verifier
  requires an acknowledgement whenever it sees a Close record. Those
  attempts remain failed verification runs. The prior qualified D2B
  four-vCPU result retains its historical scope.
- **FAIL** for `python3 tools/audit_vfs_cutover.py --no-growth`, exit 1.
  It reports 71 ring-0 references and the same four unmatched lines. The
  inventory-only invocation exits 0 and lists the same 71 references. The
  strict `--check` invocation exits 1 with the full inventory still present.
  The guard and its reviewed inventory were untouched. Two guard unit tests
  passed. All ordinary native acceptance checks remain **NOT_RUN**.

The evidence index records fresh runs separately from preserved component
qualification. This patch keeps the production routing, compressed boot
resources, stack defaults and ZRAM policy at their existing settings.
[Machine-readable results](vfs-coordinated-cutover-candidate-results.json)
