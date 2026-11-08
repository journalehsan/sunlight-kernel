# Phase 3.7 Implementation Summary: Unix Permissions & User Database

## Overview

Phase 3.7 adds a Unix-standard permission model to the VFS layer and migrates user/group
storage from a proprietary format to standard `/etc/passwd`, `/etc/group`, and `/etc/shadow`.

## Changes

### 1. Standard Unix etc/ files (`sunlight-fs/etc/`)

Three files are now embedded in the initramfs:

- **`/etc/passwd`** — standard `username:x:uid:gid:comment:home:shell` format, 2 entries
- **`/etc/group`** — standard `groupname:x:gid:members` format, 7 entries
- **`/etc/shadow`** — `username:password:...` format, plaintext for Phase 3.7 (hashing in Phase 5)

The legacy `/etc/sunlight/users` file is replaced entirely.

### 2. Unix permission bits in `FileStat` (`sunlight-fs/src/vfs.rs`)

`FileStat` gained four new fields:

```rust
pub uid: u32,    // owner user id
pub gid: u32,    // owner group id
pub mode: u16,   // Unix permission bits (rwxrwxrwx + type bits)
pub nlinks: u32, // hard link count
```

A `pub mod mode` was added with named constants (`S_IRUSR`, `S_IRGRP`, `FILE_644`,
`DIR_755`, `FILE_600`, etc.).

### 3. `RamEntry` with permissions (`sunlight-fs/src/ramfs.rs`)

`RamEntry` gained `uid`, `gid`, `mode`, and `is_dir` fields plus const constructors:

```rust
RamEntry::file(path, uid, gid, mode, data)
RamEntry::dir(path, uid, gid, mode)
```

The `INITRAMFS` now describes the full directory hierarchy (`/`, `/etc`, `/home/user`,
`/tmp` with sticky bit, etc.) with correct ownership and permissions. Opening a directory
entry via `open()` returns `Err(FsError::IsDir)`.

### 4. Permission checker (`sunlight-fs/src/permission.rs`)

```rust
pub fn check_permission(stat: &FileStat, cred: &Credential, want: PermCheck) -> bool
```

Root (uid 0) always passes. Other callers are checked against owner/group/other bits
in the standard Unix order.

### 5. Passwd/group/shadow parsers (`sunlight-fs/src/passwd.rs`)

No-heap, fixed-array parsers for all three files. Functions:

- `parse_passwd(data)` → `([PasswdEntry; 16], usize)`
- `parse_group(data)` → `([GroupEntry; 32], usize)`
- `parse_shadow(data)` → `([ShadowEntry; 16], usize)`
- `lookup_by_name(entries, name)`, `lookup_by_uid(entries, uid)`

### 6. VFS server Phase 3.7 self-tests (`services/vfs_server/src/main.rs`)

`run_phase37_tests()` runs at server startup and:

1. Logs `[VFS]  Permission model: Unix uid/gid/mode`
2. Reads and parses `/etc/passwd` → logs `[VFS]  /etc/passwd: 2 users loaded`
3. Reads and parses `/etc/group` → logs `[VFS]  /etc/group: 7 groups loaded`
4. Calls `check_permission` with root credential on `/etc/shadow` → logs bypass OK
5. Calls `check_permission` with user credential on `/etc/passwd` → logs read OK
6. Calls `check_permission` with user credential on `/etc/shadow` → logs EACCES OK

### 7. Login migration (`sunlight-tty/src/login.rs`)

`verify_login()` now:

1. Reads `/etc/passwd` via VFS IPC → logs `[TTY]  Login: reading /etc/passwd via VFS`
2. Finds the user entry to get uid/gid
3. Reads `/etc/shadow` via VFS IPC → logs `[TTY]  Login: auth from /etc/shadow`
4. Verifies the password against the shadow entry
5. Returns `Some((uid, gid))` on success

`LoginResult::Success` carries `uid` and `gid` in addition to the username.

A hardcoded fallback (`root:root`, `user:user`) activates if VFS is unreachable.

## Gates

```
✓ Phase 3.5 gate PASSED  (virtio-blk + FAT32, unchanged)
✓ Phase 3.6 gate PASSED  (TTY + keyboard + login, login success message is a substring)
✓ Phase 3.7 gate PASSED  (Unix permissions + /etc/passwd migration)
```

Run with: `./tools/test.sh phase3.7`
