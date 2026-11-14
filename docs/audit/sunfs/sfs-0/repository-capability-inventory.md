# Repository capability inventory and dependency map

2026-11-14. Static source review against incoming HEAD
`1affc823311cb6014e2aecf63b848b2bd4f2af6d` and actual working-tree bytes in
`source-baseline.json`. No incoming modifications were reported by git status.
Source line references refer to those hashed bytes; historical audit numbers are
not a substitute for current source. No production code was modified during discovery.

## Implemented owners and reusable contracts

| Capability / source | Implemented behavior | Limits and dependency |
|---|---|---|
| `docs/sunfs/README.md:1` | v0.1 architecture draft, not engine | No existing SunFS codec/formatter/recovery implementation found in filename/content discovery |
| `sunlight-fs/src/vfs.rs:124` | FileSystem operations incl sync_file/sync_dir/fstat_handle, create-exclusive, rename/unlink | Native FileType only file/directory at53; FileStat subset at83, not POSIX stat |
| `sunlight-fs/src/vfs.rs:683` | FsNode enum dispatch Ram(Tmpfs) / Fat | New backend requires enum delegation, not a trait-object plugin |
| `sunlight-fs/src/vfs.rs:891` | Eight mounts, runtime String paths, mount tags and counts | Longest component-boundary prefix routes mounts; bounded incarnations retire rather than wrap |
| `sunlight-fs/src/vfs.rs:990` | Runtime target validation /mnt, source ID uniqueness, directory ownership/emptiness | Not arbitrary root/boot mounting; keep device and protected mount policy |
| `sunlight-fs/src/vfs.rs:1040` | Runtime FAT admission, unmount sync barrier, open-count refusal | Mount remains installed on barrier failure; only FAT runtime adapter exists |
| `sunlight-fs/src/vfs.rs:20` | 22 local handle bits + 8 mount bits; local 6 slot bits +16 generation bits | Capacity exhaustion fails closed; never reuse stale token |
| `sunlight-fs/src/vfs/lookup.rs:9` | Metadata provenance/capabilities and existing-object credential walker | Path result is observation, not object pin; 255-byte input bound |
| `sunlight-fs/src/tmpfs.rs:32` | Unlinked inode backing remains charged while open; stable internal IDs at218 | Volatile memory only; directories indexed by full paths at42, no complete retained openat |
| `sunlight-fs/src/ramfs.rs:1`, `sunlight-fs/src/lib.rs:16` | Static RamEntry seed and Tmpfs exports; RamFs compatibility name | Do not confuse old RamFs design in November7 audit with today's dynamic Tmpfs owner |
| `sunlight-fs/src/vfs.rs:220` | FatFs owns16 slots, entry-location authentication, revocation/poisoning | Retained-open-unlink is absent for FAT; entry location is not persistent inode identity |
| `sunlight-fat/src/fat.rs`, `sunlight-fat/src/fat/write.rs:390` | FAT32 parsing/writes, component lookup, internal entry locator, syncs | Nontransactional multi-write mutation, possible partial failure; no SunFS-level recovery |
| `kernel/src/process/fd_table.rs:259` | fd rights/flags + shared description; consuming take at474; final release | Offset/status/refcount sharing, not a proof of every concurrent close/I/O schedule |
| `kernel/src/main.rs:2024` | Global authoritative KERNEL_VFS spin mutex | Native file syscalls use it; blocking/latency adaptation is required before SunFS integration |
| `kernel/src/main.rs:2351` | Boot root Tmpfs; selected FAT partition or legacy /boot,/state | Existing paths retained; no install or root migration |
| `services/vfs_server/src/main.rs:223` | Separate RamFs namespace, fstab and read-only boot-share snapshot | Not kernel fd authority; do not host a parallel writer here |
| `ipc/src/lib.rs` VfsMsg | Existing filesystem IPC protocol and grants | Not a persistence engine or full native metadata ABI; separate namespace remains explicit |
| `sunlight-block/src/lib.rs:20` | BLOCK_SIZE512, errors Io/OutOfRange/Unsupported, sync read/write/flush/count | flush doc promises stable media; no geometry/uncertainty/FUA/cancellation API |
| `sunlight-block/src/lib.rs:99` | Direct-mapped write-back cache; flush drains slots then inner.flush | Cache write success is acceptance; dirty cleared after lower write, before barrier |
| `sunlight-block/src/partition.rs:15` | Bounded partition block view, checked offsets/capacity, flush forwarding | Usable for secondary volume; preserve partition extent guards |
| `sunlight-block/src/registry.rs:19` | Device parent/extent/operation identities; no authority | IDs are runtime only; feature bits are transport metadata, not permissions |
| `sunlight-virtio/src/blk.rs:73` | Legacy PCI VirtIO block,512-byte sectors, capacity, read-only/flush support | Boolean errors, no FUA, fixed polling timeout, DMA uncertainty on timeout |
| `kernel/src/main.rs:1885` | KernelBlkDev wraps VirtIO/USB; unsupported flush explicitly returned | KernelDisk caches VirtIO, USB bypasses cache at1974 |
| `sunlight-block/src/usb_msc.rs:262` | READ CAPACITY10; accepts512 sector size, READ/WRITE10 | Rejects4Kn/capacity16 sentinel; no general sector negotiation |
| `sunlight-block/src/usb_msc.rs:313` | Temporary read buffer avoids exposing failed/short reads; sync-cache flush | Failure marks offline; actual hardware power-loss durability not established here |
| `kernel/src/usb_storage.rs`, `kernel/src/usb_xhci.rs` | USB mass-storage and transport integration exists | Future SunFS must qualify removal/flush/late completion independently |
| filename scan + `kernel/Cargo.toml`, `Cargo.toml`, `drivers/src/lib.rs` | No dedicated AHCI/NVMe/ATA block driver located | Bounded negative finding, not a hardware support claim; use existing VirtIO/USB scope |

## Native libc and applications

`sunlight-libc/src/lib.rs:147-306` provides native open/create/read/write/dup/close/
fstat wrappers; resolve_path and resolve_directory_path are at221/226, file_sync and
dir_sync at500/506. These route to native syscalls. `sunlight-libc/src/c_file.rs:144`
uses native canonical lookup; `file_ops.rs` owns bounded copy/move helpers. Native
stat remains the24-byte subset discussed by PY-7; full sys/stat.h contract is absent.
`tools/native-std/overlay/fs/sunlightos.rs:338-356` still rejects symlink/lstat/
canonicalize. Do not infer C/POSIX support from a Rust wrapper sharing a name.

SunShell selected implementation is `sunshell/src/main.rs`, with shared directory
resolution at2794; Files uses `sunlight-files/src/main.rs:1273` and shared directory
listing at1316. Other helpers are still lexical/private. GUI open dispatch uses
application associations, not a second filesystem authority. Native Files clicks,
interactive shell behavior and canonical-link UI are NOT_RUN in SFS-0.

The current `kernel/src/arch/x86_64/syscall.rs:6574` linux_resolve_at still obtains
metadata and a handle_path, constructs text and later resolves again. getdents64 at6831
has related path-based enumeration. These are retained-directory/TOCTOU prerequisites;
SunFS alone cannot repair them. Native sync syscalls at9586/9620 are reusable entry
points, but require an enforceable backend barrier before a durability claim.

## Error, mount and recovery policy

Current errors are in `sunlight-fs/src/error.rs`; permission evaluation and actor
mutation policy in `permission.rs`/`policy.rs`. `kernel/src/capability/vfs_grants.rs`
separates executable grants from ordinary path operations. `kernel/src/storage.rs`
owns privileged runtime device/mount dispatch. Backend identity and permission grants
must not be conflated. VFS full-path routing and the component resolver are distinct.

Tmpfs sync is Unsupported (`tmpfs.rs:567`), correctly avoiding a persistence claim.
FAT mutation errors poison the adapter; unlink revokes before raw freeing. Close
still consumes revoked handles. FAT consistency repair and retained backing are not
provided; SunFS should reuse the fail-stop principle, not inherit immediate freeing.

## Existing tooling and historical findings

- `tools/disk.sh` actually creates a64 MiB FAT image using mkfs.fat/mtools, with no
  sudo/loop device. `tools/state-disk.sh` creates a raw SUNSTATE volume. This differs
  from the generic AGENTS description; neither script is a SunFS formatter and neither
  was executed here.
- `tools/test_partitioned_storage.py`, `tools/test_runtime_storage.py`,
  `tools/test_usb_storage.py` and `kernel/src/storage_partition_test.rs` provide
  disposable image/partition/privileged mount and USB test patterns. Runtime test
  source checks exact host contents and partition guards; it is not arbitrary power-
  loss injection. `sunlight-fs/src/vfs/storage_tests.rs`, block/partition/USB and FAT
  unit tests are reusable regression patterns. No tests were rerun in SFS-0.
- `docs/storage/storage-audit-fat32-block-vfs.md` (November7) records the earlier two-
  VFS design, fixed mounts and then-current host evidence. Its statement that runtime
  mounting is absent is superseded by current runtime target/admission code.
- `docs/storage/filesystem-boundaries-and-roadmap.md` documents authority boundaries;
  `docs/storage/partitioned-storage.md` documents selection/trust and partition guards.
- PY-5 (`docs/runtime/python-py5-vfs-posix-filesystem-integration.md`) found absent
  authoritative canonical lookup/metadata/link contracts and repaired handle_path
  mount prefix. Its lexical-only app finding is historical, superseded in part byPY-6.
- PY-6 (`docs/runtime/python-py6-unified-filesystem-foundation.md`) added the common
  existing-component resolver, metadata provenance and no-wrap handles, while retained
  directory and full POSIX metadata remained incomplete.
- PY-7 (`docs/runtime/python-py7-filesystem-lifetime-posix-links.md`) explicitly
  describes FAT revocation as fail-closed, not retained POSIX lifetime. It records
  remaining directory pinning, partial-failure cleanup, time/metadata/link and forced
  schedule gaps. Its historical host/native PASS cells are not SFS-0 executions.

Dependency chain: SFS-1 device model/codecs → SFS-2 checkpoint closure/identity →
SFS-3 directory/lifetime → SFS-4 reserve/GC → SFS-5 kernel blocking-I/O and driver
qualification → SFS-6 POSIX metadata/resolver/links → optional installer/RootFS.
CPython remains paused; resume after authoritative native contracts, independently
of whether RootFS installation has happened.
