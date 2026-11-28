# VFS-C2D: Legacy tmpfs authority and ordinary callers

**PARTIAL.** The opt-in `vfs_c2d_candidate` routes ordinary native launches of
the embedded `mkdir`, `stat`, `cp` and `cat` commands through the existing
service namespace. This moves a group of callers, including both ends of
`cp`, without moving or copying a live kernel tmpfs object. It does not
retire the kernel mount or qualify a system-wide `/tmp` cutover.

The launch selector in [spawn.rs](../../kernel/src/ipc/vfs_registration/spawn.rs)
matches the embedded pathname and the applet selected by argv[0]. Its
scope is the native Spawn adapter; it covers the `/bin` and `/usr/bin`
aliases and the dedicated `/sunlight-utils/cat` alias. Boot init and services
are not selected by name. The child stays private until C1A authenticates
and publishes the retained root and cwd. There is no per-path owner switch.
Native stat and chdir use that root and cwd; the copied file channel handles
mkdir and the reader and writer opens. The candidate does not accept a
legacy VFS description as an inherited stream for a file-routed child.
Pipes and TTY streams retain their existing installation path.

The existing service descriptor install, duplicate, release and D2A/D2B
private mapping mechanisms remain in place. Routed Linux mount observation
uses the authenticated receipt and the service graph; capacity remains
limited to the existing FAT capacity contract. No new Linux executable is
selected. Errors in routed operations, including a missing or retired
service receipt, return through the existing error paths. C2D did not
change exit or exec lifecycle code. Broader executable loading and native
and Linux operations outside the supported cohort still need migration.

The [kernel boot path](../../kernel/src/main.rs) loads static initramfs
entries (or the compressed boot resource module when configured), starts
embedded boot executables, and constructs the legacy root before userspace
registration exists. Static file contents and embedded images are boot
resources; writable directories, newly created files, and live open
descriptions in that root are mutable kernel objects. Early disk selection,
the protected boot attachment and any early persistent state operations
also precede ordinary service admission. Protection against unmount does
not make a persistent state partition immutable. The service uses its
existing boot resource view and mounted namespace. C2D adds no filesystem,
copies no boot data, and changes no PMM, ZRAM, heap or stack policy.

| Unchanged guard finding | Remaining reachable consumer |
| --- | --- |
| `storage::mount` | Legacy StorageCtl MOUNT_FAT32 for manual and fstab mounts on non-reserved USB or VirtIO storage. C2B's early reservation check also invokes this constructor to prove it rejects the fenced parent. |
| `linux_procfs::mountinfo_snapshot` | The legacy Linux procfs open of `/proc/self/mountinfo` still snapshots kernel root and FAT topology. |
| `sys_linux_statfs` | Unrouted Linux pathname queries still resolve against the kernel view and look up FAT capacity by kernel mount ID. |
| `process::mmap::sys_mmap` | Private mappings of readable legacy VFS descriptions still fstat and copy through the kernel owner. Generic native and Linux open of root tmpfs and FAT, the restricted SSH secret creation syscall, and the native C guest fixture still produce legacy handles. Native spawn may also inherit an already-open legacy description outside the candidate. |

The candidate commands no longer use legacy regular open, mkdir, or
descriptor-backed read/write/FSTAT for the operations listed above. None of
the four historical source sites was removed. The inventory still reports
71 references and four unmatched sites before and after. Both the no-growth
guard and the final removal check fail. The prepared C2C image was left
untouched; guarded boot and GPT acceptance were not run.

The remaining tmpfs prerequisite is to identify all writers and readers of
one existing subtree, including non-spawn and shell operations, and either
prevent legacy creation there from boot or settle every open description,
directory pin and executable source before withdrawal. In particular,
`vfs_read_file` still backs non-embedded spawn and exec; scoped SSH secret
operations still require their own authorized service operation. Moving
`/tmp` by redirecting a pathname prefix would leave relative paths,
existing descriptors and those producers on the legacy owner.

Host registration (including selector coverage), service tests, the
production release build and candidate debug build passed. Native command
execution, Linux enrollment, mount observation, cleanup, service loss,
boot resource integrity and complete frame/reference accounting are
**NOT_RUN** for C2D. The earlier isolated FAT, reservation, stack and
retirement results retain their original scope; they are not new C2D
execution evidence. See the [results](vfs-legacy-tmpfs-authority-migration-results.json).
