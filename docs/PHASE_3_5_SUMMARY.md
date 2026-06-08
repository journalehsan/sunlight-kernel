# Phase 3.5 Summary: virtio-blk + FAT32 + VFS Integration

## Status

Passed. Phase 3.5 adds real read-only block storage: QEMU virtio-blk discovery,
FAT32 parsing, and `/boot` mounted through the existing VFS IPC service.

## Commands Run

- `cargo check --workspace` — Passed.
- `./tools/test.sh phase3.0` — Passed (regression gate).
- `./tools/test.sh phase3.5` — Passed: `✓ Phase 3.5 gate PASSED`.

## Changed Crates And Services

- Added `sunlight-virtio` workspace crate (PCI scan + legacy virtio-blk driver).
- Added `sunlight-fat` workspace crate (FAT32 parser + shared-memory page format).
- Modified `kernel` — added `init_block_and_fat()`: virtio init, FAT32 reading,
  shared-page population, mapping into vfs_server address space.
- Modified `kernel/src/memory/pmm.rs` — added `alloc_frames(n)` for contiguous
  physical frame allocation (needed for the virtio queue).
- Modified `services/vfs_server` — mounts `/boot` from shared page, adds Phase 3.5
  self-tests.
- Modified `tools/disk.sh` — replaced root-requiring parted/mount approach with
  mtools (`mkfs.fat`, `mmd`, `mcopy`). Wipes boot signature (offset 510-511) after
  formatting so SeaBIOS skips the data disk and boots the CD-ROM.
- Modified `tools/test.sh` — added `phase3.5` gate with virtio-blk QEMU flags.

## Architecture: Kernel-side FAT32 Bootstrap + Shared Memory Page

The kernel initializes virtio-blk in ring-0 during boot, reads FAT32 files, and
writes them into one physical page (the *share page*). The share page is then
mapped read-only into the vfs_server's address space at `FAT_SHARE_VADDR =
0x2000_0000`. The vfs_server reads the share page at startup and mounts `/boot`
from the pre-read data (a `BootFs` variant backed by the shared memory).

This avoids needing a user-space virtio driver or I/O privilege for the vfs_server.

## New Serial Gate Lines

- `[BLK]  Scanning PCI...`
- `[BLK]  Found virtio-blk`
- `[BLK]  Negotiated features`
- `[BLK]  Queue initialized`
- `[BLK]  Read LBA 0 OK`
- `[FAT]  FAT32 detected`
- `[VFS]  /boot OK`
- `[VFS]  Read: "SunlightOS FAT32 boot volume\n"`
- `[VFS]  Read: "Phase 3.5 FAT32 OK\n"`
- `[VFS]  /boot/MISSING.TXT ENOENT OK`
- `[SunlightOS] Phase 3.5 OK`

## Test Files

- `tools/disk.sh` — creates a 64 MiB FAT32 disk image deterministically using
  mtools (no root required). Writes `HELLO.TXT` and `BOOT/PHASE35.TXT`.
- `tools/tests/phase3_5.expected` — already present; wired into `test.sh phase3.5`.

## Known Limitations

- VirtioBlk uses legacy virtio (pre-1.0) with polling; no IRQ or DMA from user space.
- QEMU must use `disable-modern=on` so SeaBIOS negotiates the legacy interface.
- The boot signature at offset 510-511 of the disk image is zeroed to prevent
  SeaBIOS from booting the data disk instead of the CD-ROM.
- FAT32 reader supports 8.3 short names only; long file names (LFN) are skipped.
- Only the two test files (`/HELLO.TXT`, `/BOOT/PHASE35.TXT`) are pre-read; the
  VFS `/boot` mount only serves those two files plus ENOENT for anything else.
- FAT chain following is not implemented; files must fit in a single cluster.
- `alloc_frames(n)` in PMM is O(total_frames × n) — acceptable for boot time only.
- The `sti` before `enter_first_process()` was removed to prevent a timer-interrupt
  deadlock on the scheduler lock during boot. Interrupts are now re-enabled
  automatically via RFLAGS.IF=1 in the first `iretq` to user space.

## TODO State

- [x] Add `tools/disk.sh` and deterministic FAT32 test image.
- [x] Add QEMU virtio-blk flags to the test runner path (`-device virtio-blk-pci,disable-modern=on`).
- [x] Add `sunlight-virtio` crate.
- [x] Implement PCI scan and virtio-blk discovery.
- [x] Implement `read_block` for LBA 0 and subsequent sectors.
- [x] Add `alloc_frames(n)` to PMM for contiguous physical allocation.
- [x] Add `sunlight-fat` crate.
- [x] Parse BPB and validate FAT32 signature.
- [x] Implement directory lookup for known files (8.3 names).
- [x] Implement FAT32 read-only file reads (single-cluster).
- [x] Define `FatSharePage` shared-memory format (kernel → vfs_server).
- [x] Map share page into vfs_server address space at boot.
- [x] Mount `/boot → BootFs` (share-backed) in vfs_server.
- [x] Read `/boot/HELLO.TXT` — "SunlightOS FAT32 boot volume\n".
- [x] Read `/boot/BOOT/PHASE35.TXT` — "Phase 3.5 FAT32 OK\n".
- [x] `/boot/MISSING.TXT` ENOENT check.
- [x] Add `tools/tests/phase3_5.expected`.
- [x] Update `tools/test.sh phase3.5`.
- [x] Make `./tools/test.sh phase3.5` pass.
- [x] Fix timer-interrupt deadlock on scheduler lock at boot.
- [ ] Defer FAT write support to Phase 4+.
- [ ] Defer TTY, keyboard, login, and mux to Phase 3.6.

## Compatibility Notes For Next Session

Phase 3.6 may begin from a passing Phase 3.5 baseline. The IPC ABI is unchanged.
The fixed 80-byte `IpcMsg` was not modified. The vfs_server now serves both `/`
(RamFs) and `/boot` (BootFs); Phase 3.6 will use it to read `/etc/sunlight/users`
and `/etc/motd` for the login screen.

Key boot-sequence change: `sti` was removed before `enter_first_process()` to
avoid a deadlock where the timer interrupt fired while holding the scheduler lock.
Interrupts now re-enable naturally via the first `iretq` to user space.

The QEMU test command for phase3.5 requires:
```bash
-drive id=hd0,file=target/test.img,if=none,format=raw
-device virtio-blk-pci,disable-modern=on,drive=hd0
```

And `target/test.img` is built by `tools/disk.sh` (requires mtools: `mmd`, `mcopy`).
