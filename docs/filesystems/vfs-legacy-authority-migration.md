# VFS-C2A: Legacy filesystem authority migration

**PARTIAL.** The StorageCtl client used by `storagectl mount` and the fstab
apply path now completes a service preparation before reporting success.
The isolated C1E12 runtime selects the reserved whole-device cohort before
dispatch. Ordinary production volumes and caller namespaces have not moved.
See the [results](vfs-legacy-authority-migration-results.json) for fresh
evidence and the [C1E12 results](vfs-committed-fat-private-mmap-results.json)
for the prior acceptance, failed and blocked runs.

The cohort is the registered disposable VirtIO FAT32 partition at
`/mnt/volume`, its scoped StorageAdmin manager and the authenticated native
and Linux callers in the C1E12 fixture. `sys_storage_ctl` dispatches all
registered aliases of the reserved parent to the service path, where the
lease authority admits only the exact registered partition. It continues
to dispatch other devices to the legacy owner. A service error or loss on
the reserved parent cannot trigger a kernel mount. The existing kernel-side
whole-parent fence remains in place.

`storage::mount_fat32` in IPC sequences the existing StorageCtl operations:
zero indicates a completed legacy mount; a positive preparation token must
be published by the same caller. A failed publish attempts exact-token
abort and returns the publish error. The ordinary utility and the positive
path in the isolated fixture share this helper. The fixture retains the
raw preparation, replay, abort and withdrawal checks. The StorageAdmin
gate, namespace publication, file installation and close, mapping
transactions, snapshots, leases and DMA settlement use their existing paths.

| Guard finding | Original live dependency and population | C2A change and remaining blocker |
| --- | --- | --- |
| `storage::mount` | Native StorageCtl MOUNT_FAT32, used by manual mounts and boot's fstab manager. Instantiates FAT in `KERNEL_VFS` over registered USB or VirtIO. | The transactional client and candidate route use service publication for the reserved parent. The guard site remains live for non-reserved transports. Production admission must fence the real parent before any kernel probing or attachment can leave a live alias; USB needs its own settled transport adapter. |
| `linux_procfs::mountinfo_snapshot` | Linux leader opening `/proc/self/mountinfo` in the legacy view; snapshots the kernel root, protected boot FAT and runtime FAT. | No adapter change. The qualified candidate observes the committed service graph; nonrouted Linux leaders still need genuine legacy topology. Move their filesystem namespace and all visible mounts before deleting the snapshot. |
| `sys_linux_statfs` | Linux pathname resolution followed by a kernel FAT capacity lookup by resolved mount ID. | No adapter change. Candidate callers use the service pin and capacity. Legacy callers still need capacity for kernel VirtIO FAT where sampled; USB and oversize scanning limits are not new capabilities. |
| `process::mmap::sys_mmap` | Linux private mapping of a readable legacy VFS description; kernel FSTAT and positional copying with the description live. | No adapter change. Candidate native and Linux service descriptions use the existing mapping transaction. Legacy generic open can produce descriptions for root tmpfs and FAT, not just FAT. The kernel SSH secret creation path and native C boot fixture also produce kernel VFS handles with their own scopes. Preserve their object and close semantics before removing the legacy handle path. |

The seeded root is a mutable kernel tmpfs with immutable boot entries.
Embedded executable lookup and early FAT probe/share in `main.rs` are boot
bootstrap. The protected FAT mount established by `init_kernel_vfs` may be
`/boot`, `/state` or `/mnt`; protected against withdrawal does not make
`/state` or the boot-selected mutable partition immutable. The early
`/state/users` marker belongs to the persistent state volume. Kernel
virtual ProcFS, devices, and descriptor types such as pipes do not become
filesystem service tokens. The scoped SSH secret operations also retain
their checks and cannot be replaced by a generic open.

Both unchanged guard modes still fail. The no-growth inventory remains
71 references with the same four unmatched sites. No finding was removed.
Guarded ordinary boot remains blocked and was not attempted.

Both fresh one-vCPU, 2 GiB disposable guests passed: namespace readiness,
committed publication, FAT writes and reattachment, native and Linux
private mmap, StorageCtl/mountinfo/statfs agreement and withdrawal, and
fail-closed Linux mutation and metadata after service loss. Each run
settled seven leases with zero pending commands or mapping users and the
same 20 KiB of retained DMA. Each caller returned to 22,320 owned frames,
four VM regions and 50 page tables after unmap, and final service
descriptor rows and references returned to zero. Both callers reaped with
zero swap blocks and cleared IPC entries. Independent closed-image checks
passed for populated and unknown FSInfo. The normal fstab invocation
completed during fixture boot; the shared transaction was exercised on
the disposable mount by the Ring 3 fixture, not by that fstab invocation.
Exec with a committed FAT descriptor was not exercised.
Fresh diskless D1B/D2A/D2B controls passed at one, two, and four vCPUs.
Each reported seven settlements with zero remaining service rows,
references, and caller-owned frames. These controls are separate from
the one-vCPU block acceptance.

The smallest next coordinated change is to take one non-boot VirtIO fstab
partition through production-shaped admission: reserve its whole parent
before `init_kernel_vfs` can attach it, bind the real transport to the
authenticated VFS registration, and start the existing `storagectl fstab`
child and the selected ordinary client family after their retained namespace
publication. Exercise the executable itself, the reader and writer paths,
and fail-closed service loss. A path prefix alone cannot handle relative
lookups, directory descriptors, or cross-mount rename.
