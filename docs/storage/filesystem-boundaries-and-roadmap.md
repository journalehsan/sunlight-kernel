# Filesystem boundaries and improvement roadmap

Source audit: 2026-11-10. This is a design proposal, not an implementation or
runtime verification. Existing unrelated worktree changes were left untouched.

## Recommendation

Keep one authoritative kernel namespace and descriptor layer. Keep bootstrap
RAMFS, future tmpfs, virtual-memory ownership, reclaim and swap mechanisms in
the kernel. Move disk filesystem interpretation and allocation into isolated
ring 3 services, reached through a generic kernel filesystem backend. Initially
keep existing block transports and partition bounds in the kernel.

Start with bounded temporary storage and the generic service backend. Migrate
FAT32 on an explicit `/mnt` mount before migrating persistent system state.
Preserve RAM-root boot until the storage service's lifecycle and failure paths
are demonstrated. Disk-root boot is a later, independent milestone.

## What exists today

| Component | Current implementation and evidence |
| --- | --- |
| Boot image | `limine.conf` loads `boot():/boot/sunlight-kernel.elf`; `tools/make_hybrid_iso.sh` packages the kernel and BIOS/UEFI bootloader files. |
| `/` | `kernel/src/main.rs::init_kernel_vfs` mounts `RamFs::new(INITRAMFS)` at `/`. This is a memory root, not a selected disk root partition. |
| Initial files | `sunlight-fs/src/ramfs.rs::INITRAMFS` is a static Rust entry table with embedded payloads. There is no external initramfs archive in this boot configuration. Static file bytes remain borrowed from the image; mutable copies and dynamic files use `Vec<u8>`. |
| Native file I/O | `kernel/src/arch/x86_64/syscall.rs::open_resolved_path`, read/write and metadata operations use `KERNEL_VFS`. Permission and mutation policy are applied in this path. |
| Execution | Exec/spawn resolve embedded executables first, then `vfs_read_file` in `syscall.rs`. That fallback reads the whole file into a kernel heap buffer. The embedded registry is in `kernel/src/process/spawn.rs::embedded_bytes_for_path`. |
| Disk format | `sunlight-fat/src/fat.rs::Fat32` parses FAT32; `fat/write.rs` implements allocation, creation, writes, mkdir, truncate, unlink and rename. `sunlight-fs/src/vfs.rs::FatFs` adapts it to file operations. |
| Block access | `sunlight-block::BlockDevice` supplies 512-byte read/write/count/flush; partition adapters enforce extents. `KernelDisk` selects virtio or USB, with a write-back cache for virtio. |
| Runtime mounting | `kernel/src/storage.rs::mount` directly constructs FAT32 in the kernel. StorageCtl syscall 168 validates authority and operates on the same kernel VFS. |
| Configuration | `sunlight-fs/etc/fstab` is embedded at `/etc/fstab`. Init runs `storagectl fstab` before ordinary base services (`services/init/src/main.rs`). This mounts configured GPT FAT32 sources under `/mnt`; it does not select disk root. |
| Ring 3 VFS | `services/vfs_server/src/main.rs` creates another `Vfs` and RAMFS. Its `BootFs` reads a kernel-built share-page snapshot. This is not a proxy to authoritative file syscalls. |
| Swap | `kernel/src/memory/swap.rs`, `zram.rs`, `swap_slot.rs`, `zram_codec.rs`: kernel anonymous-page reclaim and ZRAM storage. `services/sunlight-swapd` calculates boot policy, configures it and exits. |

The two VFS instances are a major migration constraint. They borrow the same
initial static data but have separate mutable state, mount views and handle
tables. The legacy boot snapshot allows eight files with 128 payload bytes each
(`sunlight-fat/src/share.rs`); it is not live disk filesystem access.
Existing VFS authorization/grant operations do not by themselves move ordinary
file I/O to ring 3.

## Boot, `/boot`, root and partitions

Current flow:

```text
Firmware -> Limine -> kernel ELF with embedded files and executable images
                         |
                         +-> PMM/VMM/heap and storage discovery
                         +-> kernel RAMFS mounted at /
                         +-> optional kernel FAT32 boot mount
                         +-> init -> storagectl fstab -> ordinary services
```

The bootloader's `boot():/boot/...` is a path on its source volume. It is not
the same operation as the running kernel mounting a filesystem at `/boot`.
The hybrid ISO also contains an EFI boot image; that does not make the live
root a FAT32 partition. See `limine.conf` and `tools/make_hybrid_iso.sh`.

Kernel boot selection in `init_block_and_fat` / `init_kernel_vfs`:

| Media/configuration | Kernel namespace result |
| --- | --- |
| No usable filesystem | RAM root remains available. |
| Legacy unpartitioned FAT32 volume | Usually `/boot`. |
| Legacy SUNSTATE volume plus explicit `SUNLIGHT_TRUST_LEGACY_STATE=1` | `/state`, subject to the legacy first-disk provisioning assumption. A label alone is insufficient. |
| Unambiguous eligible MBR/GPT FAT32 partition | `/mnt`, unless GPT configuration defers selection to storagectl. |
| Explicit GPT PARTUUID fstab entry | Init-launched storagectl resolves it and mounts at an allowed `/mnt` target. |
| USB storage | Registered without implicit mounting; explicit runtime/fstab selection is required. |

Mount routing uses the longest matching path prefix
(`sunlight-fs/src/vfs.rs::resolve_mount`). Mounting `/state` hides the RAMFS
subtree at that path while the mount exists; it does not copy those files to
disk. A RAMFS `/state` placeholder is not evidence of durable storage.

PARTUUID is an identifier, not authentication. Runtime mounting requires root
and StorageAdmin; existing targets are restricted to `/mnt` or descendants.
Normal runtime mount authority cannot replace `/`, `/boot` or `/state`.
The embedded fstab's `/dev/sda1` and `/dev/ram0` entries are legacy selectors,
not actual device-node discovery. See `docs/storage/fstab-inventory.md` and
`docs/storage/runtime-storage.md` for the exact accepted format and policy.

## FAT32: mechanism and present limitations

The current stack is:

```text
app -> syscall / kernel policy -> kernel VFS -> FatFs -> Fat32
    -> partition-relative BlockDevice -> cache/transport -> disk sectors
```

`Fat32::mount` reads the volume's boot parameter block at relative sector zero
and derives cluster geometry, FAT location and root cluster. Directory entries
name objects and record their first cluster and size. FAT entries link clusters
into file/directory chains. Reads follow those chains; writes may allocate and
zero clusters, update FAT copies, then update directory records. The partition
adapter translates relative sectors into a bounded range of the physical disk.
These details are implemented in `sunlight-fat/src/fat.rs`, `fat/write.rs` and
`sunlight-block/src/partition.rs`.

FAT32 is already writable despite stale "read-only adapter" wording above
`FatFs`. Ownership/mode are synthesized by the adapter, with a special
`/users/<uid>` policy for state storage; they are not ordinary persistent Unix
inode metadata. `create_file` ignores supplied uid/gid/mode at the FAT adapter.
Do not treat this backend as a general Unix root filesystem without deciding
how persistent permissions, executable identity and links will work.

The implementation is deliberately small. Current mkdir allocates a cluster
and links it without creating `.` / `..`; timestamps, FSInfo upkeep and volume
dirty-state handling remain improvement items documented in
`storage-audit-fat32-block-vfs.md`. There is no configurable read-only FAT mount
mode. Native mount paths and file handles have small fixed bounds: eight VFS
mounts, sixteen FAT handles per volume, and stored FAT paths up to 128 bytes.
Directory reporting truncates names to `VFS_NAME_MAX` (64 bytes), while FAT
lookup/creation has separate longer-name bounds and ASCII creation restrictions.

File/directory sync reaches the device flush contract, whose failures must
propagate. A successful orderly sync/reboot test is not proof of arbitrary
power-loss recovery. The rename implementation publishes destination before
deleting source; complete crash consistency across allocation and metadata
updates needs a separate fault-injection and recovery analysis.

## Is tmpfs easy?

An initial bounded memory filesystem is manageable. A complete VM-backed,
swappable tmpfs is a substantial memory-management feature.

| Stage | Work | Assessment |
| --- | --- | --- |
| Bounded RAMFS | Byte/inode limits, fallible allocation throughout, accounting and clear quota/OOM errors. | Smallest useful step; does not supply swapping. |
| Page-backed temporary files | Allocate data in PMM-managed pages; stable inode identity; sparse offsets, truncate, open/unlink lifetime and reclaim-safe locking. | Moderate subsystem work; avoids putting all contents in the kernel heap. |
| Swappable tmpfs | Resident/swapped page states, inode-and-page ownership, swap-in/out races, pins and mappings, rollback, reserves and reclaim integration. | Significant work; current anonymous-page swapping is useful infrastructure but not a drop-in implementation. |

Current RAMFS uses contiguous heap vectors for mutable contents and shares the
16 MiB kernel heap (`kernel/src/memory/heap.rs`) with kernel metadata. It has
fallible data-buffer growth, but creation still includes infallible vector
allocation paths. It has no aggregate mount byte quota. File-data growth can
therefore compete with unrelated kernel work. Existing per-table limits do not
replace memory accounting.

Current reclaim eligibility requires an anonymous userspace ledger region and
matching process/frame ownership (`swap.rs::candidate_is_eligible`). RAMFS data
does not meet that condition. Adding a tmpfs name or mounting RAMFS at `/tmp`
does not make it reclaimable. Introduce a file-page object keyed by stable
inode generation and page index before integrating with swap. Shared mappings
must refer to that same object rather than independently swapping duplicate
copies of a file page.

For the first stage, propose a separate bounded `/tmp` instance, then `/run`
after inventorying existing seeded contents and service expectations. Keep
bootstrap RAMFS intact. Add `/dev/shm` only with explicit shared-memory and
mapping semantics. Quotas must account for metadata, resident pages and logical
file allocation; swapped pages still consume logical quota. A temporary file
must survive unlink while an open reference exists and release storage only
after its last reference/mapping. Do not advertise completed tmpfs semantics
until the corresponding gates pass.

Linux's documented distinction is useful as a reference: tmpfs supports size
and inode limits and optional swapping; it does not need an on-disk filesystem
format or a RAM block device. Swapping temporary contents does not make them
persistent across boot. See the official Linux tmpfs documentation.

## Proposed kernel / ring 3 boundary

| Keep in ring 0 | Put in ring 3 |
| --- | --- |
| Namespace/mount routing, descriptors, open descriptions and lifecycle pins | FAT32 and later disk-format parsers, directory structures and allocation algorithms |
| Caller credentials, authorization, mount grants and revocation | Provisioning, filesystem checking and repair tools, mount policy orchestration |
| Bootstrap RAMFS and future page-backed tmpfs | Per-volume filesystem service instances and format-specific caches |
| PMM/VMM, page ownership, faults, reclaim and ZRAM/disk-swap mechanism | Swap sizing/configuration policy, as already demonstrated by swapd |
| Initially: hardware block I/O, bounded partition grants and flush barriers | Block requests only within the service's granted extent and rights |

```text
App / native libc / Linux compatibility
                   |
          kernel syscall + policy
                   |
           single kernel VFS
             /             \
     local RAMFS/tmpfs     remote filesystem backend
                                  |
                      authenticated, bounded IPC
                                  |
                           ring 3 FAT service
                                  |
                      partition-scoped block grant
                                  |
                      kernel block I/O -> device
```

This is a proposed native service interface, not a commitment to implementing
the Linux FUSE ABI. FUSE is a useful reference for kernel-visible files whose
contents and metadata are supplied by a userspace daemon, including failure
and cancellation semantics. Start without a second generic dirty page cache;
define where each accepted write lives and what sync guarantees before adding
writeback optimization.

### Required contracts before moving FAT

1. **One namespace.** Add a generic remote backend alongside local RAMFS/tmpfs.
   Retain ordinary file syscall and Linux compatibility entry points. Convert
   legacy VFS clients to the authoritative view; retire its independent RAMFS
   and boot snapshot only after client and authorization coverage is complete.
2. **Blocking discipline.** Existing methods are synchronous under a global
   spin mutex. Never wait for a ring 3 reply while holding KERNEL_VFS, scheduler,
   PMM or other locks the service needs. Pin mount/open/request objects, release
   locks, park the caller, then revalidate object generations on completion.
   The callback-style `read_dir` needs a bounded reply/cursor representation.
3. **Kernel-authenticated identity.** Bind registration to the actual service
   process and address-space incarnation and an explicit mount grant. A
   nameserver name or PID is insufficient. Preserve existing trusted-VFS
   authorization roles; replacement servers must not inherit authority by name.
4. **Block isolation.** Expose read/write/flush using an opaque grant for one
   validated partition range, rights, geometry and media generation. Current
   StorageCtl mount operations are not a raw block-service interface. Reject
   out-of-range and stale requests; exclude mounted and reserved swap extents
   from conflicting raw access.
5. **Bounded protocol.** Version requests/replies; cover lookup/stat, open,
   create/exclusive create, read/write, readdir, close, truncate, rename/unlink,
   mkdir, sync and supported metadata operations. Validate lengths, offsets,
   entry counts and transferred bytes. Initially reject unsupported operations
   explicitly. Never send raw kernel/user pointers; use owned bounded buffers
   or lifetime-controlled shared-memory grants.
6. **Lifetime and concurrency.** Pin mounts and open descriptions across I/O;
   retain dup/fork/final-close semantics, busy unmount, process-exit cleanup,
   media retirement and non-aliasing identities across service restarts. Use
   stable file identities so rename/unlink do not invalidate the wrong object.
7. **Failure semantics.** Define bounded queues/backpressure, deadlines,
   cancellation and service death. Wake blocked callers on abort; reject late
   replies. Do not automatically replay writes whose execution is uncertain.
   Restart need not restore old handles: an explicit failed mount plus fresh
   mount incarnation is a valid first policy.
8. **Durability and authority.** Preserve file and directory sync through the
   filesystem service to real block flush. Keep kernel write-policy decisions
   and descriptor rights; service-provided metadata must not mint elevated
   execution authority. Distinguish mount protection, read-only mount policy
   and hardware write protection.
9. **Boot and memory progress.** Load initial filesystem services and their
   dependencies from bootstrap memory. Reserve bounded working memory for I/O
   service progress before allowing its pages to depend on the same storage
   path being reclaimed. Keep a RAM-root recovery route.

Moving format parsing to ring 3 limits a parser fault's direct kernel impact.
It does not make a granted storage service trustworthy for its own volume's
contents, or remove the need to validate replies and enforce block bounds.

## Swap and future disk-root boot

Swap stays part of the VM mechanism. Current ZRAM has generation-tagged slots,
page validation and bounded anonymous reclaim. Swapd already demonstrates a
useful policy/mechanism split: it may exit after configuration. ZRAM uses RAM;
it does not provide disk persistence. This audit found ZRAM storage, not a disk
swap backend, in the current swap path.

Plan disk swap as a separately granted raw partition first. Keep slot ownership,
fault handling, page-table updates and swap I/O progress in the kernel. If a
swapfile is later required, ring 3 may allocate it, but the kernel needs a
validated, pinned extent map and exclusions against truncate, relocation,
unmount and conflicting writes. Do not fault through ordinary filesystem IPC
to swap in the filesystem service's own memory. Disk swap and hibernation are
separate features.

For a future installed system, first choose whether `/` remains a memory
bootstrap with disk-backed submounts or transitions to a disk root. Both fit the
proposed boundary. A real root transition needs explicit root-device selection,
service readiness, mount handoff, CWD/open-handle rules, embedded-executable
precedence, service executable provenance and recovery behavior. Mounting a
disk somewhere under `/mnt` implements none of those automatically. Prioritize
disk-backed data mounts and a deliberate Unix metadata policy before disk root.

## Suggested milestones and acceptance evidence

| Milestone | Deliverable | Required evidence |
| --- | --- | --- |
| A: local storage bounds | Bounded temporary mount and accounting; documented volatile/durable distinction. | Exhaust bytes/inodes without kernel panic; rollback after allocation failure; zero-fill/truncate and cleanup; ordinary boot and filesystem consumers. |
| B: generic remote backend | One kernel namespace with a limited ring 3 test backend and scoped block API. | Real native calls block/resume without locks held; identity/rights/bounds rejection; server death and late replies; queue saturation and unrelated task progress. |
| C: FAT32 migration | Reused FAT algorithm runs in ring 3 for an explicit `/mnt` volume. | Native read/write/create/readdir/sync; duplicate-fd and busy-unmount behavior; restart/remount; independent host byte checks and partition guard checks; virtio and USB reported separately. |
| D: filesystem correctness | Read-only mounts, metadata/interop policy, bounded names and resource limits, failure handling. | Host filesystem-tool interoperability; allocation/flush failure injection; malformed media; interrupted-write recovery with outcomes recorded explicitly. |
| E: page-backed tmpfs | Kernel file-page storage, then optional swap integration. | Memory pressure reclaims actual temporary-file pages; exact bytes after swap-in; shared mappings/unlink/truncate races; quota remains charged while swapped; no VM progress cycle. |
| F: persistent system mounts | Deliberately provisioned state and service ordering. | No silent volatile fallback; missing/ambiguous/untrusted media; durable state across independent boots; service failure/recovery. |
| G: optional disk root / disk swap | Independently specified root transition and raw swap backend. | Early-service bootstrap/recovery and authority preserved; swap fault progress under low memory and I/O failure. |

Work on FAT interoperability can proceed before or alongside migration. Keep
the initial service backend small; file-backed mmap, coherent generic page
cache/writeback and transparent server recovery require explicit later gates.

## Evidence boundary and references

This report traces current source and uses previous repository documents as
context. No build, host test, QEMU boot, real-device test, power-loss test or
performance measurement was run for this audit. Historical test results in
other documents do not validate the current dirty tree.

Local sources: `kernel/src/main.rs`, `kernel/src/storage.rs`,
`kernel/src/arch/x86_64/syscall.rs`, `kernel/src/process/spawn.rs`,
`sunlight-fs/src/{ramfs,vfs,fstab}.rs`, `sunlight-block/src/{lib,partition}.rs`,
`sunlight-fat/src/{fat,share}.rs`, `sunlight-fat/src/fat/write.rs`,
`services/{init,vfs_server,sunlight-swapd}/src/main.rs`,
`kernel/src/memory/{heap,swap,zram}.rs`, `ipc/src/swap_policy.rs`,
`docs/storage/{partitioned-storage,runtime-storage,usb-storage,fstab-inventory}.md`.

External primary references consulted:

- Linux kernel documentation, Tmpfs: `https://docs.kernel.org/filesystems/tmpfs.html`
- Linux kernel documentation, FUSE Overview: `https://docs.kernel.org/filesystems/fuse/fuse.html`
- Linux kernel documentation, FUSE I/O Modes: `https://docs.kernel.org/filesystems/fuse/fuse-io.html`

Older swap/FAT design documents include superseded descriptions. In particular,
the current kernel heap is 16 MiB, FAT has write support, and current ZRAM policy
and backing storage differ from the early fixed-pool/heap-budget documents.
Use live source before implementing the next milestone.
