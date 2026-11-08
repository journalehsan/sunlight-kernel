# Phase 6.5 Step 3 — Next Session TODO (exec-from-VFS & friends)

> Written 2026-06-12 at the end of the Parts A & B session
> ([PHASE6_5_STEP3_VFS_ELF.md](PHASE6_5_STEP3_VFS_ELF.md)). Everything below
> builds on code that already exists and is tested; nothing here is blocked.

## ✅ STATUS (2026-06-12, Part C session): DONE — Step 3 complete

The `phase6.5.3` boot gate passes: `[VFS] kernel mount OK`, `[EXEC] ls
exit=0`, `[EXEC] mkdir exit=0`. What landed:

- Kernel-global `KERNEL_VFS` (`Vfs<CachedBlockDevice<KernelBlkDev, 16>>`)
  over the long-lived `VIRTIO_BLK` static: INITRAMFS at `/`, FAT at `/boot`.
- `sys_exec` falls back to the VFS after embedded images, with ELF-header
  validation *before* the old image is torn down (`exec /etc/passwd` fails
  cleanly).
- New syscalls: `Spawn`(39, posix_spawn-style with stdout-fd handoff),
  `ReadDir`(60), `StatPath`(61), `Mkdir`(62); `Open`(40)/`Read`(42)/
  `Close`(41) are now VFS-backed with per-fd offsets; `Waitpid`(32) returns
  real exit codes (non-blocking + EAGAIN, userland yields).
- `sunlight-utils` and `sunlight-net-utils` rewritten as no_std multi-call
  binaries on sunlight-libc (Option A), embedded in the kernel and resolved
  via `/sunlight-utils/<applet>` PATH stubs; argv[0] picks the applet.
- sshl runs externals via pipe + Spawn + Waitpid, streams output to the TTY,
  and sets `$?` (`echo $?` works).
- Boot fixes found along the way: 1 MiB Limine stack-size request (the VFS
  bootstrap overflowed the 64 KiB default, corrupting the memmap), and
  `process_exit` now pivots to the kernel stack + `sti` so the first real
  process exit doesn't hang/triple-fault the machine.

Still open (folded into Step 4): write-path syscalls (touch/rm/cp/mv),
net_server IPC for spawned processes (ping/ifconfig applets), per-process
cwd, `Vfs::unmount` + cache flush, SunlightFS `FsNode` variant.

## Goal

`ls`, `mkdir`, `cat`, … from `sunlight-utils` actually execute from disk via
PATH and return exit codes — closing checklist 3.3 in
[PHASE6_5_USERLAND_EXPECTATIONS.md](PHASE6_5_USERLAND_EXPECTATIONS.md).

## 1. Kernel-side VFS instance (the missing link)

`sys_exec` (`kernel/src/arch/x86_64/syscall.rs:588`) still resolves binaries
only through `embedded_bytes_for_path` (`kernel/src/process/spawn.rs`), so any
non-embedded path fails. Plan:

- [ ] Add `sunlight-fs` to `kernel/Cargo.toml`; create a kernel-global
      `Vfs<KernelDisk>` (e.g. `spin::Mutex` static, like `PMM`), where
      `KernelDisk = CachedBlockDevice<VirtioBootDisk, 16>` — the adapter
      already exists in `kernel/src/main.rs`. Caveat: `VirtioBootDisk`
      currently borrows the boot-scope `VirtioBlk`; the device must move into
      a long-lived static for post-boot reads (today it's dropped after
      `init_block_and_fat`).
- [ ] Mount at boot: ramfs `/` (reuse `sunlight_fs::INITRAMFS`) + FAT volume
      at `/boot` (or `/mnt/disk`). Keep the share-page path for vfs_server
      untouched for now.
- [ ] In `sys_exec`: try `embedded_bytes_for_path` first (boot servers,
      `/bin/sshl`), else `vfs.stat` → `open` → `read` the whole file into an
      `alloc::vec::Vec<u8>` → `exec_into_process`. Reject with a clean error
      (no panic) when stat fails or the loader returns None — the validation
      path is already in place.

## 2. Real binaries for sunlight-utils

The INITRAMFS entries `/sunlight-utils/ls` etc. are `#!` stub text files
(`sunlight-fs/src/ramfs.rs:380`), not ELFs — exec'ing them must fail cleanly
today. Two options, pick one next session:

- **Option A (fast):** build `sunlight-utils` as a no_std multi-call binary
  linking `sunlight-libc` (argv[0] decides the applet — busybox style), embed
  it via the FAT boot image (`tools/disk.sh` / ISO packaging) or INITRAMFS
  `include_bytes!`, and point the PATH stubs at it.
- **Option B (cleaner, more work):** one small no_std binary per applet in
  `sunlight-utils`, placed on the FAT image under `/sunlight-utils/`.

Note: current `sunlight-utils/src/main.rs` is a **std** binary (breaks
`cargo check --workspace` for the none-target — pre-existing). The no_std
rewrite should use `sunlight-libc` (`read_dir` will need a new syscall, see 4).

## 3. Exit codes & shell integration

- [ ] Verify `sys_waitpid` (`syscall.rs`) actually returns the child status to
      the shell; wire `echo $?` end-to-end.
- [ ] The TTY shell (`sunlight-tty/src/shell.rs`) PATH-resolution from Step 2
      should fork+exec the resolved path instead of treating applets as
      builtins — check how `#!/sunlight/...` stubs are currently special-cased
      before changing behavior.

## 4. Syscalls the utils will need (extend kernel + sunlight-libc)

- [ ] `Open`(40) is a stub returning ENOENT — back it with the kernel VFS +
      the process `fd_table` (`kernel/src/process/fd_table.rs`).
- [ ] `Read`(42)/`Close`(41) against VFS-backed fds (pipe/TTY paths already work).
- [ ] New `ReadDir` syscall (suggest 60) — fills a user buffer with packed
      `VfsDirEntry`-style records; needed by `ls`. Add wrapper in sunlight-libc.
- [ ] `Stat` via `Fstat`(48) or a new path-based syscall — needed by `ls -l`/`stat`.

## 5. Smaller leftovers from the Step 3 checklist

- [ ] `Vfs::unmount(path)` + `CachedBlockDevice::flush` on unmount.
- [ ] SunlightFS variant in `FsNode` (design TBD — currently only RamFs/Fat).
- [ ] In-VM FAT test against a real image per the expectations file:
      `mkfs.vfat` + `mcopy`, boot `./tools/run.sh -n --disk`, `ls /mnt`.
- [ ] New boot gate `phase6.5.3` + `tools/tests/phase6_5_3.expected`
      (serial markers: `[VFS] kernel mount OK`, `[EXEC] ls exit=0`).

## Verification bar for "Step 3 done"

```sh
cargo test -p sunlight-block -p sunlight-fat -p sunlight-fs -p sunlight-elf \
  --target x86_64-unknown-linux-gnu        # 55+ tests stay green
./tools/test.sh phase6.5.1                 # no regression
./tools/test.sh phase6.5.3                 # new gate
# interactive: ls /, mkdir /tmp/x && ls /tmp, cat /etc/passwd; echo $?,
# exec /etc/passwd → clean error (no panic)
```
