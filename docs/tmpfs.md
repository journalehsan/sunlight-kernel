# Temporary filesystem

`sunlight_fs::Tmpfs` replaces mutable RAMFS buffers. Kernel file payloads use
4 KiB frames from PMM, accessed through the bootloader's permanent direct map.
The `sunlight-mm` crate owns each frame and returns it to its original allocator
on drop. Only inode records, paths, and frame-address vectors use the kernel
heap; their allocations use `try_reserve_exact` or `Arc::try_new`.

The live defaults are **64 MiB of allocated payload pages** and **4096 inodes
per mount**, including the root and static seed entries. Both quotas can be
configured at construction:

```rust
let fs = sunlight_fs::Tmpfs::with_limits(
    sunlight_fs::INITRAMFS,
    sunlight_fs::TmpfsLimits::new(32 * 1024 * 1024, 2048),
)?;
vfs.mount_tmpfs("/", fs)?;
```

`usage()` returns `(allocated_page_bytes, inode_count)`. Byte charging rounds
up to full pages, including storage reserved without extending file length.
An allocation that exceeds a quota returns `FsError::NoSpace` (`ENOSPC`).
Metadata allocator or physical-frame exhaustion returns `FsError::NoMemory`
(`ENOMEM`). Offset overflow returns `NoSpace`; invalid paths return
`InvalidPath`. Paths are capped at 4096 bytes and names at `VFS_NAME_MAX`.

Growth reserves the complete quota and frame-vector capacity before changing
file contents. A partial PMM failure releases newly acquired frames and rolls
back the byte charge, preserving existing content and length. Fresh pages and
truncated tails are zeroed. `TmpfsInode::truncate_to` supports shrinking and
zero-filled extension; the existing VFS `truncate` operation reduces length to
zero. Deletion and filesystem drop release backing pages and charges.

Static initramfs payloads remain borrowed from the kernel image until first
modification. They are already resident image data and do not consume the
mutable-page quota. Their inodes do count. Existing seed metadata protection,
handle generations, rename behavior, and unsupported durability operations are
preserved. `RamFs` and `mount_ramfs` remain compatibility aliases; `/etc/fstab`
uses `tmpfs` and also accepts the historical `ramfs` spelling.

The separate legacy IPC VFS service uses the same filesystem with a page
provider backed by native anonymous `mmap`/`munmap`, with exact release
boundaries. It receives private mapped addresses, never physical addresses or
access to the kernel direct map. Its payloads also bypass its metadata heap.
Bare-metal code has no heap-backed payload fallback when a provider is absent.
Hosted Linux/macOS/Windows verification uses a separate aligned allocator.

This change provides explicit frame ownership; it does not implement eviction
or swap-in/swap-out. Open-file deletion retains the previous VFS behavior of
invalidating handles. Directory removal and hard links are outside the current
filesystem interface.

## Verification

```sh
cargo test -p sunlight-fs --lib --target x86_64-unknown-linux-gnu
cargo test -p sunlight-libc --lib --target x86_64-unknown-linux-gnu sys::tests::
SUNLIGHT_TEST_SERIAL_LOG=target/tmpfs-native-serial.log ./tools/test.sh tmpfs
```

Hosted tests include partial-frame fault injection, quota exhaustion and
concurrent atomic reservations, zero-fill, reserve/truncate/delete, rename
replacement accounting, inode identity, invalid paths, and existing VFS
regressions. Fault injection is a hosted-only `sunlight-mm/testutil` feature.

The opt-in `tmpfs_test` native gate checks cross-page data, zero-filled holes,
byte/inode limits, truncation and unlink. It checks both the PMM filesystem
class and actual free-page counts before allocation and after reclamation,
and emits `[TMPFS] native page/quota/zero-fill/reclaim PASS` on success.
Ordinary builds leave this gate disabled.

Verified on November 10, 2026: 88 filesystem tests, 3 targeted libc syscall
error tests, the bare-metal `sunlight-mm`/`sunlight-fs` check, and the QEMU
Tmpfs gate passed. Native serial evidence is in
`target/tmpfs-native-serial.log`; host evidence is in
`target/tmpfs-host-fs-tests.log` and `target/tmpfs-host-errno-tests.log`.
A broader libc suite reported a failure in the unmodified
`alloc::tests::free_list_exact_fit_unsplittable_tail_and_full_recovery` test
and then stalled; that attempt was interrupted and retained in
`target/tmpfs-broad-libc-attempt.log`. A complete libc suite pass is not claimed.
