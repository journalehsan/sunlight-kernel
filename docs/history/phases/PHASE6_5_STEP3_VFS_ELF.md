# Phase 6.5 Step 3 — VFS Block Layer, FAT32 Mounts & Secure ELF Loading

> Parts A & B delivered 2026-06-12. Covers checkpoints ①–⑤ of the Step 3 plan.
> Remaining Step 3 scope (exec-from-VFS in `sys_exec`, SunlightFS partitions,
> umount) tracked in [PHASE6_5_USERLAND_EXPECTATIONS.md](PHASE6_5_USERLAND_EXPECTATIONS.md).

## 3.1 Luxos reference analysis (lxfs + lucerna)

Read: `~/Projects/luxos/servers/fs/lxfs/src/{blockio,mount,open,read}.c`,
`~/Projects/luxos/lucerna/src/unistd/exec.c`, `~/Projects/luxos/kernel/src/syscalls/`.

**Adopted:**
- *Direct-mapped write-back block cache* (`blockio.c`): slot = `block % CACHE_SIZE`,
  tag = `block / CACHE_SIZE`, per-slot dirty bit, flush-on-evict. Rust version:
  `CachedBlockDevice<D, const N: usize>` in `sunlight-block`.
- *Mountpoint as device + cache + geometry* (`mount.c`): our `Mount` pairs a
  mount path with an `FsNode`; geometry lives inside the filesystem driver.
- *Path→entry resolution walking directory clusters* (`open.c`/`dirtree.c`):
  `Fat32::stat_path` walks N path components through `find_in_dir`.

**Rejected / replaced:**
- lxfs lazily `malloc`s each cache slot buffer; we use fixed `[CacheSlot; N]`
  arrays — no allocation in the I/O path (the TTY freeze taught us why).
- Luxos mountpoints form a heap-allocated linked list keyed by device string;
  we keep the existing fixed `[Option<Mount>; 8]` table with longest-prefix
  routing — bounded, no_std-friendly.
- Luxos's VFS is message-passing to an fs server for *all* I/O; SunlightOS
  keeps the VFS as a library crate usable both in-kernel (future exec path)
  and in `vfs_server`.
- errno-style integer returns → `Result<T, FsError>` everywhere.

## Architecture delivered

```
sunlight-block        BlockDevice trait, BlockError, CachedBlockDevice<D, N>,
  ↑          ↑        MemDisk (test), NullDevice (RAM-only VFS default)
sunlight-fat │        Fat32<D: BlockDevice> — FAT-chain walking, stat_path,
  ↑          │        read_at (offset reads), read_dir_raw, N-deep paths
sunlight-fs ─┘        re-exports as sunlight_fs::block; FatFs<D> adapter
                      (handle table over Fat32); FsNode<D>::{Ram, Fat};
                      Vfs<D>::mount/mount_ramfs/mount_fat/read_dir;
                      VfsDirEntry callback API replaces the DirIter stub
kernel                VirtioBootDisk adapter (main.rs) replaces the
                      closure-based FAT reader
sunlight-libc         userland syscall wrappers: open/close/read/write/
                      exec/getpid/exit over inline-asm SYSCALL stubs
```

The crate split exists because `BlockDevice` must sit *below* both
`sunlight-fat` (which is generic over it) and `sunlight-fs` (whose
`FsNode::Fat` holds a `Fat32<D>`); putting the trait inside `sunlight-fs`
would be a dependency cycle.

`Vfs`, `FsNode`, `Mount` are now generic over `D: BlockDevice` with default
`NullDevice`, so existing RAM-only users (`vfs_server`) compile unchanged.

## ELF loader: validation before mapping

`sunlight-elf` gained `plan_segments(bytes, header, user_lo, user_hi, emit)`:
nothing is emitted until **every** PT_LOAD passes —

- virtual range inside `[0x1000, USER_HEAP_START)` with overflow-checked
  arithmetic (closes the hole where a hostile ELF could request kernel-half
  pages and the old loader would map them `USER_ACCESSIBLE`),
- W^X (no segment both writable and executable),
- `p_filesz ≤ p_memsz`, file range inside the binary,
- entry point inside an executable PT_LOAD,
- header: ELF64 magic, `ET_EXEC`, `EM_X86_64` (header also carries `osabi`).

`kernel/src/process/elf_loader.rs` now consumes the crate instead of
duplicating offset parsing. Two latent bugs fixed in the mapping loop:

1. **Shared-page protection union** — when two segments share a 4 KiB page
   (sshl: R ends at `0x411ee8`, RW starts there), the old code kept the first
   mapping's flags, leaving the first 0x20 bytes of `.data` read-only.
   Now flags are unioned (`AddressSpace::update_flags`; NX drops out if either
   side is executable).
2. **.bss overcopy** — the old loop copied file bytes up to `p_memsz` instead
   of `p_filesz`, smearing adjacent file content into memory that must be
   zero-initialized.

## SysV exec stack

`setup_exec_stack` (spawn.rs) was a stub returning bare `USER_STACK_TOP`.
It now marshals, top-down: NUL-terminated argv/envp strings, then the table
`[argc][argv…][NULL][envp…][NULL]` with final RSP 16-byte aligned at argc,
written through the page tables via HHDM (`copy_to_user`). Registers still
carry rdi=argc, rsi=argv, rdx=envp as a convenience. `sys_exec` now reads
envp from userspace; a NULL envp inherits the process `EnvMap` (Step 2).

## Verification (2026-06-12)

- `cargo test -p sunlight-block -p sunlight-fat -p sunlight-fs -p sunlight-elf
  --target x86_64-unknown-linux-gnu` → **55 passed** (cache eviction/flush,
  multi-cluster FAT-chain reads, offset reads, nested paths, dir listing,
  VFS FAT mounts, kernel-half/W^X/entry/overflow ELF rejections, ramfs read_dir)
- `./tools/test.sh phase6.5.1` → **PASSED** (all six embedded boot servers
  load through the new validating loader; sshl runs with unioned page flags
  and a marshalled stack)

## Still open in Step 3

- Route `sys_exec` through the VFS (drop `embedded_bytes_for_path` for user
  binaries) so `sunlight-utils` `ls`/`mkdir` actually execute from disk.
- `umount` + flush; SunlightFS partitions behind `FsNode`.
- `mount` shell command + FAT32 image test per the expectations file.
