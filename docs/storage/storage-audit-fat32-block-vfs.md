# Storage, Block, VFS and Bootloader Audit

Prepared 2026-11-07. Scope: readiness for mounting real external FAT32 disks
(virtio/IDE block device) and USB flash drives.

Evidence boundary: static source review plus the storage crates' host unit
tests. No QEMU boot, no real hardware, and no USB mass-storage device was
exercised here.

Host test baseline (explicit host target; the workspace default target is
`x86_64-unknown-none`, where `cargo test` cannot link the `test` crate):

```
cargo test -p sunlight-block -p sunlight-fat -p sunlight-fs --target x86_64-unknown-linux-gnu
# sunlight-block 4 passed, sunlight-fat 8 passed, sunlight-fs 74 passed
```

## 1. Bootloader and memory mapping (Limine)

### Configuration

| File | Content | Used |
| --- | --- | --- |
| `limine.conf` | `protocol: limine`, `kernel_path: boot():/boot/sunlight-kernel.elf`, `kaslr: no` | Yes, copied into the ISO by `tools/make_hybrid_iso.sh:28` |
| `limine.cfg` | `PROTOCOL=limine`, `KERNEL_PATH=boot:///boot/sunlight-kernel.elf`, `KASLR=no` | No, legacy leftover; the ISO build ignores it |

The ISO is a hybrid BIOS plus x86_64 UEFI image (`tools/make_hybrid_iso.sh`),
built by `tools/build.sh:167`.

### Boot modules and initramfs

- The kernel declares no `limine::request::ModuleRequest`, and neither config
  file lists an `initramfs` or `ramdisk`. Limine loads exactly one file: the
  kernel ELF.
- The initramfs is a compile-time constant inside the kernel image:
  `sunlight-fs/src/ramfs.rs:746` `pub static INITRAMFS: &[RamEntry]`, built from
  `include_bytes!` payloads and `RamEntry::file`/`dir` literals.
  `sunlight-fs/src/ramfs.rs:35` sets
  `INITRAMFS_EMBEDDED_IN_KERNEL_IMAGE = true`, and
  `initramfs_static_file_data_bytes()` (`ramfs.rs:23`) measures the payload for
  accounting.
- Userspace service ELFs are also embedded in the kernel image, for example
  `kernel/src/main.rs:66-140` (`INIT_ELF_BYTES`, `VFS_SERVER_ELF_BYTES`).

### Limine requests actually issued (`kernel/src/main.rs:43-64`)

| Request | Purpose |
| --- | --- |
| `MemmapRequest` | PMM frame discovery |
| `HhdmRequest` | HHDM offset for physical-to-virtual translation |
| `FramebufferRequest` | boot splash and display fallback |
| `RsdpRequest` | ACPI |
| `FirmwareTypeRequest` | BIOS vs UEFI diagnostics |
| `MpRequest(0)` | SMP bring-up |
| `SmbiosRequest` | SMBIOS 3.x |
| `StackSizeRequest(1 MiB)` | boot stack; the comment warns that the RAMFS/FAT bootstrap is stack-heavy |

`kernel/src/arch/x86_64/boot.rs` is a 6-line stub that only initializes the
serial port; there is no separate early entry or boot protocol module.

### How the root filesystem is mapped

- There is no physical or virtual mapping step for the initramfs: `RamFs`
  borrows `'static` slices that live in the kernel's rodata, so they are already
  in the kernel address space (`sunlight-fs/src/ramfs.rs:1-40`,
  `RamFs::new(entries)` at `ramfs.rs:94`).
- The PMM treats `MEMMAP_EXECUTABLE_AND_MODULES` as installed RAM, but only
  `MEMMAP_USABLE` frames become allocatable and the loaded kernel image span is
  reserved (`kernel/src/memory/pmm.rs:79-140`).
- Supporting a Limine-loaded initrd or module is not implemented but is cheap: a
  `ModuleRequest` plus `MemDisk::new(&mut module_slice)` would give an in-RAM
  block device for a FAT image.

## 2. VFS architecture and mount points

There are two independent VFS implementations in the tree. This is the single
most important architectural fact for this audit.

### 2.1 Kernel VFS (authoritative; backs all file syscalls)

Defined in `sunlight-fs/src/vfs.rs`:

- `pub trait FileSystem` at `vfs.rs:118`: open, create, create_file_exclusive,
  read, write, reserve, truncate, close, sync_file, sync_dir, fstat_handle,
  stat, mkdir, chmod, chown, unlink, rename, read_dir, handle_path.
- `pub enum FsNode<D: BlockDevice>` at `vfs.rs:559`: `Ram(RamFs)` or
  `Fat(FatFs<D>)`. Enum dispatch, no trait objects, no allocation.
- `pub struct Vfs<D: BlockDevice = NullDevice>` at `vfs.rs:725`, with
  `mounts: [Option<Mount<D>>; MAX_MOUNTS]` and `MAX_MOUNTS = 8` (`vfs.rs:5`).
- Mount registration: `Vfs::mount` (`vfs.rs:740`), `mount_ramfs` (`vfs.rs:754`),
  `mount_fat` (`vfs.rs:758`). Mount points are `&'static str` and must be
  unique.
- Resolution: `resolve_mount` (`vfs.rs:1059`) picks the longest matching prefix,
  so nested mounts shadow parents; `resolve_mount_for_create` (`vfs.rs:1078`)
  resolves the parent directory first.
- Handles are packed as `mount_idx << 24 | local_handle` (`pack_handle`,
  `vfs.rs:1129`); local handles carry a generation tag to defeat stale-handle
  reuse.

Global instance: `kernel/src/main.rs:1881`
`pub static KERNEL_VFS: spin::Mutex<Option<sunlight_fs::Vfs<KernelDisk>>>`.

Boot construction: `init_kernel_vfs()` at `kernel/src/main.rs:2134`, called once
at `main.rs:696` before any userspace process starts:

```
/       -> RamFs::new(INITRAMFS)                              (main.rs:2141)
/state  -> Fat32 over virtio-blk when BPB label == SUNSTATE    (main.rs:2150-2153)
/boot   -> Fat32 over virtio-blk otherwise                     (main.rs:2151-2153)
```

The mount set is hard-coded and boot-time only. There is no `mount(2)` syscall,
no mount message in `VfsMsg` (`ipc/src/lib.rs:1445`), and no runtime mount table
mutation from userspace.

Consumers: every native file syscall in `kernel/src/arch/x86_64/syscall.rs`
(`sys_open:4967`, `sys_stat_path:8481`, `sys_readdir:8394`, `sys_mkdir:8523`,
`sys_unlink:8572`, `sys_rename:8603`, `sys_file_sync:8700`,
`sys_dir_sync:8734`, `sys_read:8946`, `sys_write:9228`), plus `exec` binary
loading through `vfs_read_file` (`syscall.rs:3670`) after embedded-image lookup
(`process/spawn.rs:836`).

### 2.2 Userspace vfs_server (IPC, legacy)

`services/vfs_server/src/main.rs`:

- Owns a second `Vfs` (`State.vfs`, `main.rs:183`) populated only with a `RamFs`
  root (`mount_from_fstab`, `main.rs:262-279`), driven by `/etc/fstab` from the
  initramfs.
- `/boot` is not a `Vfs` mount here: it is a side-table `BootFs`
  (`main.rs:76-100`) over the snapshot share page at `FAT_SHARE_VADDR`
  (`sunlight-fat/src/share.rs:3`), which the kernel fills at boot with at most
  8 files of 48-byte path and 128-byte data (`share.rs:6-26`).
- Supports `ramfs` and `bootfs` only; the match falls through to `None` in
  `mount_fstab_entry` (`main.rs:281-302`).
- `/state` is not visible through this IPC service at all.

`/etc/fstab` content is static and fake-device oriented
(`sunlight-fs/src/ramfs.rs:1176-1183`): `/dev/sda1 /boot bootfs` and
`/dev/ram0 / ramfs`. Parser: `sunlight-fs/src/fstab.rs::parse_fstab` (max 8
entries, requires 4 fields, `#` comments).

### 2.3 Filesystem drivers that exist

| Driver | Location | Notes |
| --- | --- | --- |
| `RamFs` (initramfs plus dynamic entries in kernel heap) | `sunlight-fs/src/ramfs.rs:78` | Static entries are immutable; dynamic entries are heap `Vec<u8>`; `sync_*` returns `Unsupported` |
| `Fat32` plus `FatFs` adapter | `sunlight-fat/src/fat.rs`, `sunlight-fat/src/fat/write.rs`, `sunlight-fs/src/vfs.rs:196` | The only disk filesystem |
| `BootFs` (share-page snapshot) | `services/vfs_server/src/main.rs:76` | Read-only, 8 files, 128 bytes each |
| devfs, tmpfs, procfs, sysfs | none | Do not exist; there is no `/dev` directory in `INITRAMFS` at all |

### 2.4 Mount points that actually exist

`/` (ramfs), `/tmp` (ramfs, 1777), `/run`, `/var`, `/var/lib`, `/state`, `/etc`,
`/home`, `/root`, `/bin`, `/sbin`, `/usr/bin`, `/Applications`, `/srv`,
`/system`, `/boot` (FAT when the label is not `SUNSTATE`), `/state` (FAT when the
label is `SUNSTATE`).

## 3. History and current implementation of /state

### Timeline (git)

| Commit | Meaning |
| --- | --- |
| `03d3bf498` | `sunlight-fat`: FAT32 parser plus share page |
| `efdb5e21c` | `sunlight-virtio`: PCI scan plus legacy virtio-blk |
| `61520aedd`, `49bc467f5` | kernel-populated boot share page, `/boot` mount |
| `1b46be6f6` | Phase 6.5.3: BlockDevice, VFS mount table, FAT mounts, kernel VFS |
| `0764531ae` | "fs: enforce immutable write policy"; `/state` appears in `INITRAMFS` |
| `2c6a661f4` | Wise Owl welcome flow; `/state/wiseowl-braind` placeholder added |
| `c9ca27e55`, `993c23d19` | Wise Owl identity Phase A and persistence hardening |
| `b0c5fb21d` | "persistent Wise Owl state and writable FAT storage": `SUNSTATE` label, writable FAT at `/state` |

### Current structure

`/state` exists at two levels simultaneously:

1. Initramfs placeholder, always present: `sunlight-fs/src/ramfs.rs:989-1026`
   creates `/state`, `/state/sunlightd`, `/state/sunlight-kv`,
   `/state/wiseowl-memoryd`, `/state/wiseowl-memorydb`,
   `/state/wiseowl-braind`, `/state/sunlight-tls`, `/state/sunlight-uac` and
   `/state/capability-broker`, all `DIR_700` except `/state` (`DIR_755`).
2. Disk mount, only when a virtio-blk volume carries the FAT32 label `SUNSTATE`
   at BPB offset 71-78: mounted read/write at `/state`, shadowing the ramfs
   directory via longest-prefix resolution. The kernel then creates
   `/state/users` (`kernel/src/main.rs:2158-2163`).

Detection is a byte comparison against the BPB label
(`kernel/src/main.rs:2057-2062`), not a partition type, UUID, or fstab choice.
`tools/state-disk.sh` produces the matching image: raw FAT32, `-n SUNSTATE`,
256 MiB default, BIOS boot signature zeroed.

### How services reach /state

| Service | Path used | Mechanism | Durability behavior |
| --- | --- | --- | --- |
| `wiseowl-memorydb` | `/state/wiseowl-memorydb/...` | `sunlight_libc` file syscalls into the kernel VFS | Fails closed: `NativeIdentityStorage::open` requires `libc::dir_sync(STATE_DIR)` to succeed and RAMFS returns `Unsupported`, so without the disk it logs `persistent /state unavailable; identity startup suspended` (`wiseowl-memorydb/src/bin_parts/wiseowl-memorydb-native-body.rs:44-60`, `466-479`) |
| `wiseowl-memoryd` | `/state/wiseowl-memoryd/generation.bin`, `sNNNNNNNN.owls` | file syscalls | Best effort only, no sync, no fail-closed (`wiseowl-memory/src/bin_parts/wiseowl-memoryd-native-body.rs:143-200`) |
| `wiseowl-indexd` | `/state/wiseowl-indexd/...` | file syscalls | Directory is not pre-created in `INITRAMFS`; created with best-effort `mkdir` |
| `sunlightd` | `/state/sunlightd/enabled-services` | temp file plus `rename` | No fsync before rename (`services/sunlightd/src/main.rs:68-69`, `269-318`) |
| `wiseowl-braind` | none directly | Uses `sunlight-kv` (`kv_client`) and `wiseowl-memorydb` over IPC | `/state/wiseowl-braind` is only a reserved placeholder; nothing in the tree writes it |
| `sunlight-kv` | `/var/lib/sunlight/kv.store`, `/var/lib/sunlight-kv/broker.store` | kernel VFS via direct syscalls plus the `sunlight-sm` IPC storage manager | Volatile: `/var/lib` is initramfs RAM, not the FAT volume |

Authorization is name-based in `sunlight-fs/src/policy.rs`:

- `Actor`: `User { uid, name }`, `Service { name }`, `Unknown`
  (`policy.rs:4-9`).
- `can_write(actor, path, op, capability, uac_approved)` (`policy.rs:72`).
- `service_state_owner()` (`policy.rs:348`) returns the first path segment under
  `/state/` or `/var/lib/`, and `service_decision` allows a service exactly when
  it owns that segment (`policy.rs:225-247`).
- The kernel maps process name to actor in `current_fs_actor()`
  (`kernel/src/arch/x86_64/syscall.rs:5428-5469`), covering `wiseowl-memorydb`,
  `wiseowl-memoryd`, `wiseowl-indexd`, `wiseowl-braind`, `sunlightd`,
  `sunlight-tls`, `capability-broker`, plus flag-based `sunlight-uac`,
  `sunlight-sm`, `sunlight-kv`.
- Users may write only `/tmp`, their `/home/<user>`, and
  `/state/users/<their-uid>/...` (`policy.rs:170-220`). `/boot`, `/etc`, `/bin`,
  `/sbin`, `/services`, `/kernel`, `/proc`, `/sys` are always immutable.

`FatFs` sets `user_storage = (mountpoint == "/state")` (`vfs.rs:760`) and then
derives uid/gid/mode purely from the `/users/<uid>/...` path rather than from
on-disk metadata (`vfs.rs:233-256`). `chmod`/`chown` on FAT return `Unsupported`
(`vfs.rs:484-490`).

## 4. Block device and driver subsystems

### 4.1 Block abstraction

`sunlight-block/src/lib.rs`:

```rust
pub const BLOCK_SIZE: usize = 512;                  // :15

pub trait BlockDevice {                             // :24
    fn read_block(&mut self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError>;
    fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), BlockError>;
    fn flush(&mut self) -> Result<(), BlockError>;  // must return Unsupported if no real barrier
    fn block_count(&self) -> u64;
}
```

Implementations:

- `&mut D` passthrough (`lib.rs:36`).
- `NullDevice` (`lib.rs:54`) rejects all I/O; used for RAM-only VFS instances.
- `CachedBlockDevice<D, N>` (`lib.rs:92`): direct-mapped write-back cache,
  slot = `lba % N`, tag = `lba / N`, flush-on-evict, explicit `flush()` that
  writes dirty slots and then calls the device barrier (`lib.rs:139`).
- `MemDisk<'a>` (`lib.rs:186`) borrows `&mut [u8]` and is explicitly documented
  as usable for initrd-style images.

Kernel wiring:

- `KernelBlkDev` (`kernel/src/main.rs:1823`) adapts the long-lived
  `VIRTIO_BLK: spin::Mutex<BlkCell>` (`main.rs:1815`) to `BlockDevice`.
- `type KernelDisk = CachedBlockDevice<KernelBlkDev, 16>` (`main.rs:1876`): a
  16-sector (8 KiB) write-back cache.
- `VirtioBootDisk<'a>` (`main.rs:1909`) is the temporary boot-time adapter used
  for the first FAT mount.

### 4.2 virtio-blk

`sunlight-virtio/src/blk.rs`:

- Legacy (transitional) virtio-pci only: port-I/O registers, `QUEUE_PFN`, no
  modern PCI capabilities, no MSI-X, no multiqueue (`VirtioBlk::init`, `blk.rs:64`).
- One queue, one fixed 512-byte request buffer, fully synchronous polling
  (`submit`, `blk.rs:139`, spin limit 50,000,000), no IRQ path.
- Feature negotiation writes `features & !((1 << 5) | (1 << 7))` (RO and SCSI
  cleared) but keeps every other advertised bit (`blk.rs:78-80`), including
  features the driver does not implement (BLK_SIZE, GEOMETRY, TOPOLOGY,
  CONFIG_WCE, MQ).
- `VIRTIO_BLK_F_FLUSH` is bit 9 and `flush()` returns false unless negotiated
  (`blk.rs:32`, `blk.rs:238`). `block_count()` is capacity in 512-byte sectors.
- Discovery: `find_virtio_blk()` (`sunlight-virtio/src/pci.rs:270`) scans buses
  0-7, slots 0-31, function 0 only, and requires BAR0 to be an I/O BAR. A modern
  virtio-blk-pci with a memory BAR is ignored.
- QEMU invocations always pass `disable-modern=on` (`tools/test.sh:850`).

### 4.3 ATA/IDE, AHCI, NVMe, SCSI

None exist.

- No ATA/IDE, AHCI, SATA, NVMe, or SCSI code anywhere in the workspace. Grep for
  `0x1F0`, AHCI, NVMe, and PCI class `0x0106` returns nothing outside unrelated
  identifiers.
- `kernel/src/main.rs:1785-1795` prints `[BTRFS] Superblock found` and
  `[NVME] Controller found (stub)` from an env-gated (`SUNLIGHT_INJECT_PHASE`)
  serial-print block. These are cosmetic strings, not drivers.
- Ring-3 port I/O exists (`sys_io_port_read`/`sys_io_port_write`, syscalls
  162/163) but is hard-locked to the audiod/ES1371 port whitelist
  (`kernel/src/arch/x86_64/syscall.rs:12074-12112`), so it cannot be reused for
  ATA PIO today.
- `hardware_inventory::enumerate_boot_hardware`
  (`kernel/src/hardware_inventory.rs:36`) does enumerate every PCI function into
  the generic inventory with `NoDriver`, so undiscovered ATA/AHCI/NVMe
  controllers are already visible to device reporting even though nothing binds
  them.

### 4.4 USB

- Only xHCI, only HID boot mouse: `services/sunlight-usb-mouse`
  (`src/usb_mouse.rs:1-25`). It is a ring-3 driver; controller access goes
  through three syscalls (`sys_xhci_info` 130, `sys_map_mmio` 131,
  `sys_dma_alloc` 132).
- The driver owns one slot, endpoint 0 (control) and one interrupt-IN endpoint
  (`usb_mouse.rs:15-30`, `158-250`). Control transfers exist (SETUP/DATA/STATUS
  TRBs), but there is no generic bulk endpoint support, no mass-storage class, no
  SCSI transparent command set, and no BOT/CBWM/CSW handling.
- `DeviceGrant` (`kernel/src/arch/x86_64/syscall.rs:11642`) has exactly two
  variants, `UsbMouse` and `Audiod`, resolved from the process name; `dma_alloc`
  and `map_mmio` are capped per grant. Adding a USB storage service requires a
  new grant plus new gated syscalls.
- EHCI/UHCI/OHCI: absent.

## 5. Filesystem driver capabilities

### 5.1 What exists

`sunlight-fat` is a hand-written `no_std`, no-heap FAT32 driver over
`BlockDevice`:

- Mount: `Fat32::mount` (`sunlight-fat/src/fat.rs:68`) parses the BPB at LBA 0
  and requires `bytes_per_sector == 512`, `root_ent_cnt == 0`,
  `fat16_size == 0`, non-zero FAT32 size, and the literal `"FAT32   "` string at
  offset 82.
- Read: `stat_path` (`fat.rs:283`), `read_at` (`fat.rs:309`), `read_file`
  (`fat.rs:357`), `read_dir_raw` (`fat.rs:368`), FAT-chain walk (`next_cluster`,
  `fat.rs:120`).
- Write: `create_file_path` (`fat/write.rs:333`), `mkdir_path` (`fat/write.rs:342`),
  `write_path` (`fat/write.rs:390`), `truncate_path` (`fat/write.rs:433`),
  `unlink_path` (`fat/write.rs:461`), `rename_path` (`fat/write.rs:491`), cluster
  allocate/free (`fat/write.rs:80`, `fat/write.rs:91`), LFN slot creation
  (`fat/write.rs:290-331`).
- LFN: read and write up to 255 UTF-16 code units (`MAX_LFN`, 20 slots).
- Both FAT copies are updated on allocation (`write_fat_entry`,
  `fat/write.rs:41-73`); reads use the first copy only.
- `flush()` (`fat.rs:388`) delegates to `BlockDevice::flush`, so the durability
  chain is `FatFs::sync_*` -> `Fat32::flush` -> `CachedBlockDevice::flush` ->
  `VirtioBlk::flush` (VIRTIO_BLK_T_FLUSH).

### 5.2 What does not exist

- No partition table support. No MBR or GPT parsing anywhere; the FAT BPB must
  start at LBA 0 (a "superfloppy"). Every real USB stick or disk image that
  carries an MBR/GPT partition table is rejected by `Fat32::mount`.
- No FAT12 or FAT16; explicitly rejected by mount validation.
- No exFAT.
- No filesystem time stamps: directory entries are written with zero date/time
  fields and reads ignore them.
- No FSInfo (free-cluster count / next-free hint) maintenance, so other OSes
  show stale free space and `fsck.fat` may warn. Cluster allocation is a linear
  scan from cluster 2 per allocation (`fat/write.rs:80-88`), which is O(n) and
  slow on multi-GB volumes.
- `mkdir_path` allocates the directory cluster but does not write the `.` and
  `..` entries (`fat/write.rs:342-351`), an interop and fsck hazard.
- No volume-label API, no chkdsk/dirty-bit handling (the clean-shutdown bit in
  `FAT[1]`), and no read-only mount option: a `FatFs` mount is always writable
  at the VFS layer, and write denial comes only from policy.
- Name limits at the VFS boundary: `VFS_NAME_MAX = 64` (`vfs.rs:7`) truncates
  LFN names in `read_dir` results (`VfsDirEntry::from_bytes`, `vfs.rs:97`) and
  `sunlight-libc::DirEntry.name` is `[u8; 64]` (`sunlight-libc/src/lib.rs:287`),
  so FAT's 255-character names are not fully usable.
- Handle and table limits: 8 mounts (`vfs.rs:5`), 16 open FAT handles per mount
  (`vfs.rs:9`), a 128-byte stored path per open handle (`FatOpen.path`,
  `vfs.rs:190`), 32 RAMFS handles, 512 static RAMFS entries.

### 5.3 How a filesystem plugs into the VFS

Two integration points, both without trait objects:

1. Implement `FileSystem` (`vfs.rs:118`) for a type `MyFs<D: BlockDevice>`.
2. Add a variant to `FsNode` (`vfs.rs:559`) and a `mount_*` helper on `Vfs`.

Adding a new disk filesystem therefore touches `sunlight-fs/src/vfs.rs`
(`FsNode` plus mount helper), the mount selection in
`kernel/src/main.rs::init_kernel_vfs`, and any policy in
`sunlight-fs/src/policy.rs`. There is no registry, no module loader, and no
filesystem autodetection.

## 6. Reuse versus build for a real external FAT32 disk

### Directly reusable

| Component | Reuse |
| --- | --- |
| `BlockDevice` trait and `CachedBlockDevice` | Yes, unchanged, for any new driver |
| `MemDisk` | Yes, for a Limine module or an in-RAM FAT image |
| `Fat32` read/write core | Yes, for a volume whose BPB is at LBA 0 with 512-byte sectors |
| `FatFs` plus `Vfs` mount table | Yes; `mount_fat` already works for arbitrary mount points (host test `mounts_fat32_volume_at_directory`) |
| `FileSystem::sync_file`/`sync_dir` chain | Yes, provided the new driver implements `flush()` honestly |
| Name-based `/state` policy | Yes, if the new volume mounts at `/state` and keeps the service-directory convention |
| PCI inventory (`hardware_inventory`) | Yes, for discovering candidate controllers |

### Must be built

1. Partition layer: MBR (and ideally GPT) parsing plus a
   `PartitionBlockDevice` adapter that offsets LBAs. Without this, no stock USB
   stick or disk image mounts.
2. Multi-device block registry: today there is exactly one `VIRTIO_BLK` static
   and one boot-time mount call. Needed for several disks, hot-plug, and
   mount-point selection by label, UUID, or partition.
3. Runtime mount API: a syscall or privileged IPC service for
   `mount`/`umount`/`statfs`, plus a userspace device namespace. Today there is
   no `/dev` directory, no device nodes, and no mount message in `VfsMsg`.
4. ATA/IDE driver (PIO or DMA): nothing exists. Either a kernel driver, or a
   ring-3 driver modeled on the xHCI pattern (new `DeviceGrant`, generic
   port-I/O or MMIO plus IRQ grant).
5. AHCI/NVMe: nothing exists; both need MMIO BAR mapping, DMA queues, and
   interrupt or polling completion.
6. USB mass storage: the xHCI driver has no bulk endpoints and no SCSI/BOT
   layer, and the USB stack is HID-only. Needed when the flash drive is USB
   rather than virtio/IDE attached.
7. FAT interop hardening: `.`/`..` entries in `mkdir`, timestamps, FSInfo
   updates, free-cluster hint caching, FAT12/16 (optional), exFAT (optional),
   and sector sizes other than 512 if 4Kn media is in scope.
8. Policy and namespace decisions: whether removable media mounts under `/mnt`,
   `/media`, or `/state`; how uid/gid/mode are derived (today FAT metadata is
   synthesized from the path); and whether `/var/lib` should move onto the
   persistent volume (currently `sunlight-kv`'s stores are RAM-backed).

### Suggested first milestone (smallest real-disk proof)

1. Add MBR/GPT parsing plus `PartitionBlockDevice`.
2. Add a second virtio-blk device path (or a QEMU `-drive` with an MBR-partitioned
   FAT32 image) and mount it at `/mnt` with `mount_fat`.
3. Mount it read-only in policy, verify `read_dir`/`stat`/`read`/`exec`, then
   allow a writable mount with `sync_dir` before any service writes.
4. Only then start ATA/AHCI/NVMe/USB transport work.

## 7. Concrete risks and acceptance gaps

- A normal FAT32 USB stick will not mount: BPB-at-LBA-0 assumption, no MBR/GPT.
- A modern (non-legacy) virtio-blk device will not be found: an I/O BAR is
  required.
- Only one block device is supported, and only at boot.
- Removable media cannot be granted to users: no `/dev`, no mount API, no
  `statfs`, and `/boot` is denied even to root.
- `/var/lib` is RAM, so `sunlight-kv`'s `kv.store` and `broker.store` are
  volatile even though policy calls them service state.
- `/state` exists in `INITRAMFS`, so "path exists" is not proof of persistence;
  only `wiseowl-memorydb` currently fails closed on that distinction via
  `dir_sync` returning `Unsupported`.
- FAT writes lack timestamps, `.`/`..` entries, and FSInfo updates, so a volume
  written by SunlightOS may not be clean under `fsck.fat` or on Windows.
- Long FAT names (over 64 bytes) are truncated in directory listings.
- No crash-consistency testing exists for FAT outside the `tools/test.sh`
  identity-reboot scenario, which uses host `cache=writeback`, so only guest
  FLUSH exercises real durability.

Status: audit complete; no code changed.
