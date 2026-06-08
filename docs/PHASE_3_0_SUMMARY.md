# Phase 3.0 Summary: Storage Bootstrap + VFS Foundation

## Status

Passed. Phase 3.0 now has `sunlight-fs`, an embedded read-only RamFs/VFS
foundation, a user-space `vfs_server`, compact fixed-ABI VFS IPC messages, and
a passing deterministic boot gate.

## Commands Run

- `cargo test --package sunlight-fs`
  - Failed because the workspace default target is `x86_64-unknown-none`, which
    has no host `test` crate.
- `cargo test --target x86_64-unknown-linux-gnu --package sunlight-fs`
  - Passed: 14 unit tests.
- `cargo check --workspace`
  - Passed.
- `./tools/test.sh phase3.0`
  - Passed: `✓ Phase 3.0 gate PASSED`.

## Changed Crates And Services

- Added `sunlight-fs` workspace crate.
- Added `services/vfs_server` workspace crate.
- Added shared `VfsMsg` labels to `sunlight-ipc`.
- Embedded and spawned `vfs_server` during boot.

## New Serial Gate Lines

- `[VFS]  Registered as 'vfs'`
- `[VFS]  Test open /etc/motd`
- `[VFS]  Test read /etc/motd`
- `[VFS]  Read: "Welcome to SunlightOS\n"`
- `[VFS]  ENOENT test OK`
- `[VFS]  Bad handle test OK`
- `[VFS]  Stat OK`
- `[SunlightOS] Phase 3.0 OK`

## Test Files

- Added unit tests inside:
  - `sunlight-fs/src/path.rs`
  - `sunlight-fs/src/ramfs.rs`
  - `sunlight-fs/src/vfs.rs`
- Wired `tools/tests/phase3_0.expected` into `tools/test.sh phase3.0`.

No QEMU injection helpers have been added yet.

Note: run `sunlight-fs` unit tests with an explicit host target while the
workspace default target remains `x86_64-unknown-none`.

## Known Limitations

- `RamFs` is read-only and uses exact path matching.
- `RamFs::readdir` currently returns `FsError::Unsupported`.
- `Vfs` currently supports enum-backed `RamFs` mounts only.
- `Vfs` uses a packed global handle with the high byte as mount index.
- VFS IPC paths are capped at 32 inline bytes.
- VFS IPC read replies return up to 16 inline bytes per reply.
- `vfs_server` is read-only and uses embedded initramfs data only.
- Phase 3.5 storage work is intentionally not started.

## TODO State

- [x] Add `sunlight-fs` crate to workspace.
- [x] Implement `FsError`, `FileHandle`, `FileType`, and `FileStat`.
- [x] Implement `RamFs::open/read/close/stat`.
- [x] Add unit tests for RamFs path success and errors.
- [x] Implement `Vfs` with enum-backed `RamFs` mounts.
- [x] Add basic VFS route/read/stat/error tests.
- [x] Add `services/vfs_server`.
- [x] Register `vfs_server` as `vfs`.
- [x] Add VFS IPC Open/Read/Close/Stat.
- [x] Spawn `vfs_server` during boot.
- [x] Add VFS serial self-tests.
- [x] Add ENOENT and bad-handle boot checks.
- [x] Update `tools/test.sh phase3.0`.
- [x] Make `./tools/test.sh phase3.0` pass.
- [ ] Defer virtio-blk and FAT32 to Phase 3.5.
- [ ] Defer TTY, keyboard, login, and mux to Phase 3.6.

## Compatibility Notes For Next Session

The next slice may begin Phase 3.5 from a passing Phase 3.0 baseline. Keep using
the existing Phase 2.6 IPC API:

- `endpoint_create`
- `nameserver_register("vfs", endpoint)`
- `ipc_recv`
- `ipc_reply_and_wait`

The fixed 80-byte `IpcMsg` ABI was not changed. The current VFS protocol uses
four register-carried words: Open/Stat pack a 32-byte path, Read uses
handle/offset/length, Close uses handle, and Read replies pack 16 data bytes.
