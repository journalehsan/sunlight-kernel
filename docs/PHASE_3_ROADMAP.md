# SunlightOS Phase 3 Roadmap: Storage, VFS, FAT32, TTY

## Purpose

Phase 3 is intentionally split into separately gated work packages. Do not
implement all of it in one session. Each sub-phase must leave the tree in a
bootable, testable state and must document what changed before moving on.

Current baseline: Phase 2.6 passes through `./tools/test.sh` with hardened IPC,
init name-server registration, timer registration, `ipc_reply_and_wait`, and the
IPC fastpath eligibility stub.

## Phase Split

```text
Phase 3.0 -> Storage Bootstrap + VFS/RamFs
Phase 3.5 -> virtio-blk + FAT32 + VFS service integration
Phase 3.6 -> SunlightTTY + keyboard + login/mux
```

Global rule: implement only the requested phase. If asked for Phase 3.0, do not
start Phase 3.5. If asked for Phase 3.5, assume Phase 3.0 is passing. If asked
for Phase 3.6, assume Phase 3.0 and Phase 3.5 are passing.

## Global Constraints

- Do not regress Phase 2.6 IPC boot gate.
- Do not change the fixed `IpcMsg` ABI during Phase 3.x.
- Do not add syscalls unless a design note explains why IPC cannot handle it.
- Prefer user-space services registered through init:
  - `nameserver_register("vfs", endpoint)`
  - `nameserver_lookup("vfs")`
- Keep serial logs deterministic because `tools/test.sh` gates on them.
- Keep `unsafe` blocks minimal and include `SAFETY:` comments in new code.
- Avoid heap allocation in FAT32 read paths, VT100 parsing, and keyboard IRQ
  handling.
- Keep UI changes consistent with `sunlight-tui`; do not redesign the boot TUI
  while implementing storage or TTY features.
- Every phase must end with both:
  - `[SunlightOS] Phase X OK`
  - `✓ Phase X gate PASSED`

## Required Handoff Document After Each Sub-Phase

At the end of every Phase 3.x implementation, create a summary file:

```text
docs/PHASE_3_0_SUMMARY.md
docs/PHASE_3_5_SUMMARY.md
docs/PHASE_3_6_SUMMARY.md
```

Each summary must include:

- status: passed, partial, or blocked
- commands run and results
- changed crates and services
- new serial gate lines
- test files added under `tools/tests/`
- known limitations
- follow-up TODO checklist with completed items checked
- compatibility notes for the next phase

Use this checklist shape:

```markdown
## TODO State

- [x] Implemented ...
- [x] Added gate ...
- [ ] Deferred ...
```

## Test Layout

`tools/test.sh` remains the main boot gate entry point.

Add reusable test definitions and helpers under:

```text
tools/tests/
├── README.md
├── phase2_6.expected
├── phase3_0.expected
├── phase3_5.expected
├── phase3_6.expected
└── injections/
    └── README.md
```

Expected files contain one required serial substring per line. Later phases can
add helper scripts for disk-image creation, keyboard injection, or kernel test
configuration, but the main public entry point should stay:

```bash
./tools/test.sh phase2.6
./tools/test.sh phase3.0
./tools/test.sh phase3.5
./tools/test.sh phase3.6
```

The default `./tools/test.sh` should keep running the latest stable gate.

## Phase 3.0: Storage Bootstrap + VFS/RamFs

### Goal

Prove the OS has a minimal read-only filesystem foundation without a real disk
driver. Phase 3.0 uses embedded initramfs data, a `RamFs`, a simple VFS, and a
user-space `vfs_server` over existing IPC.

### Explicit Non-Goals

- no virtio-blk
- no FAT32
- no keyboard
- no TTY/login/mux
- no write support
- no new storage syscalls

### New Workspace Members

Add:

```toml
"sunlight-fs",
"services/vfs_server",
```

Do not add `sunlight-virtio`, `sunlight-fat`, or `sunlight-tty` in Phase 3.0
unless they are unused placeholders with a written reason.

### Crate Structure

```text
sunlight-fs/
└── src/
    ├── lib.rs
    ├── error.rs
    ├── path.rs
    ├── ramfs.rs
    └── vfs.rs
```

### Minimal FS Types

```rust
pub trait FileSystem {
    fn open(&mut self, path: &str) -> Result<FileHandle, FsError>;
    fn read(&mut self, handle: FileHandle, offset: usize, buf: &mut [u8])
        -> Result<usize, FsError>;
    fn close(&mut self, handle: FileHandle) -> Result<(), FsError>;
    fn stat(&mut self, path: &str) -> Result<FileStat, FsError>;
    fn readdir(&mut self, path: &str) -> Result<DirIter, FsError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileHandle(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub file_type: FileType,
    pub size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    NotDir,
    IsDir,
    InvalidPath,
    BadHandle,
    TooManyOpenFiles,
    PermissionDenied,
    Io,
    Unsupported,
}
```

Keep `sunlight-fs` `no_std` compatible.

### RamFs Requirements

```rust
pub struct RamFs {
    entries: &'static [RamEntry],
    handles: [Option<usize>; 32],
}

pub struct RamEntry {
    pub path: &'static str,
    pub data: &'static [u8],
}
```

Requirements:

- read-only
- exact path matching is acceptable
- no heap allocation on the read path
- support offset reads
- support `open`, `read`, `close`, and `stat`
- `readdir("/")` may be minimal or return `Unsupported` if not used by tests

Embedded data:

```rust
pub static INITRAMFS: &[RamEntry] = &[
    RamEntry {
        path: "/etc/motd",
        data: b"Welcome to SunlightOS\n",
    },
    RamEntry {
        path: "/etc/sunlight/session.toml",
        data: br#"
[default]
mode = "terminal"

[terminal]
shell = "/bin/sh"
initial_tabs = 1
theme = "sunlight-dark"

[multi_user]
enabled = false
max_ttys = 6
"#,
    },
    RamEntry {
        path: "/etc/sunlight/users",
        data: b"root:root\nuser:user\n",
    },
    RamEntry {
        path: "/bin/sh",
        data: b"#!/sunlight/builtin-sh\n",
    },
];
```

### VFS Requirements

Use enum-backed filesystems unless allocation is already clearly appropriate:

```rust
pub enum FsNode {
    Ram(RamFs),
}
```

Mount layout:

```text
/ -> RamFs, read-only
```

Required API:

```rust
impl Vfs {
    pub fn new() -> Self;
    pub fn mount_ramfs(&mut self, path: &str, fs: RamFs) -> Result<(), FsError>;
    pub fn open(&mut self, path: &str) -> Result<FileHandle, FsError>;
    pub fn read(&mut self, handle: FileHandle, offset: usize, buf: &mut [u8])
        -> Result<usize, FsError>;
    pub fn close(&mut self, handle: FileHandle) -> Result<(), FsError>;
    pub fn stat(&mut self, path: &str) -> Result<FileStat, FsError>;
}
```

### VFS IPC Service

Add:

```text
services/vfs_server/
```

The service must:

- create an endpoint
- register as `vfs` through init
- serve through `ipc_reply_and_wait`
- support `Open`, `Read`, `Close`, and `Stat`

Use compact IPC requests. Phase 3.0 may limit paths to 48 bytes and reads to
small inline chunks.

Suggested opcodes:

```rust
#[repr(u32)]
pub enum VfsOpcode {
    Open = 1,
    Read = 2,
    Close = 3,
    Stat = 4,
}
```

### Phase 3.0 Measurable Tasks

- [ ] Add `sunlight-fs` crate to workspace.
- [ ] Implement `FsError`, `FileHandle`, `FileType`, and `FileStat`.
- [ ] Implement `RamFs::open/read/close/stat`.
- [ ] Add unit tests for RamFs path success and errors.
- [ ] Implement `Vfs` with `/ -> RamFs`.
- [ ] Add `services/vfs_server` and link it as a user service.
- [ ] Register `vfs_server` as `vfs`.
- [ ] Add IPC operations for open/read/close/stat.
- [ ] Add ENOENT and bad-handle boot checks.
- [ ] Add `tools/tests/phase3_0.expected`.
- [ ] Update `tools/test.sh phase3.0`.
- [ ] Create `docs/PHASE_3_0_SUMMARY.md`.

### Phase 3.0 Gate Lines

`./tools/test.sh phase3.0` must require:

```text
[VFS]  Registered as 'vfs'
[VFS]  Test open /etc/motd
[VFS]  Test read /etc/motd
[VFS]  Read: "Welcome to SunlightOS\n"
[VFS]  ENOENT test OK
[VFS]  Bad handle test OK
[VFS]  Stat OK
[SunlightOS] Phase 3.0 OK
```

The test script output must end with:

```text
✓ Phase 3.0 gate PASSED
```

### Phase 3.0 Sub-Prompt

Use this prompt in a future session:

```text
Implement SunlightOS Phase 3.0 only, following docs/PHASE_3_ROADMAP.md.
Do not start virtio, FAT32, keyboard, or TTY work.

Deliver:
- sunlight-fs crate with RamFs and Vfs
- services/vfs_server registered as "vfs"
- VFS IPC open/read/close/stat
- deterministic serial gate lines
- tools/tests/phase3_0.expected
- tools/test.sh phase3.0 support
- docs/PHASE_3_0_SUMMARY.md with completed TODO state

Run:
- cargo check --workspace
- ./tools/test.sh phase2.6 or equivalent baseline
- ./tools/test.sh phase3.0
```

## Phase 3.5: virtio-blk + FAT32 + VFS Integration

### Goal

Extend Phase 3.0 with real read-only block storage. The system must discover a
QEMU virtio-blk disk, read blocks, parse FAT32, mount `/boot`, and read known
files through VFS.

### Explicit Non-Goals

- no FAT write support
- no file creation/deletion
- no ELF loading from disk
- no TTY/login/mux
- no USB HID

### New Workspace Members

Add:

```toml
"sunlight-virtio",
"sunlight-fat",
```

### Disk Image Tool

Add or update:

```text
tools/disk.sh
```

Prefer `mtools` to avoid root:

```bash
dd if=/dev/zero of=target/test.img bs=1M count=64
mkfs.fat -F32 target/test.img
mcopy -i target/test.img target/disk-root/HELLO.TXT ::HELLO.TXT
mmd   -i target/test.img ::BOOT
mcopy -i target/test.img target/disk-root/BOOT/PHASE35.TXT ::BOOT/PHASE35.TXT
```

If `mtools` is unavailable, print a clear error and exit non-zero.

### QEMU Disk Flags

Use explicit virtio-blk attachment:

```bash
-drive id=hd0,file=target/test.img,if=none,format=raw
-device virtio-blk-pci,drive=hd0
```

### virtio-blk Requirements

Target:

```text
QEMU virtio-blk-pci
```

Required:

- PCI scan
- device identification
- feature negotiation
- one queue
- `read_block(lba, &mut [u8; 512])`
- `write_block` may return `Unsupported`

Serial checks:

```text
[BLK]  Scanning PCI...
[BLK]  Found virtio-blk
[BLK]  Negotiated features
[BLK]  Queue initialized
[BLK]  Read LBA 0 OK
```

### FAT32 Requirements

Add:

```text
sunlight-fat/
└── src/
    ├── lib.rs
    ├── boot.rs
    ├── fat.rs
    ├── dir.rs
    ├── file.rs
    └── error.rs
```

Required:

- FAT32 only
- parse BPB
- validate FAT32 signature
- read root directory
- support 8.3 names at minimum
- open/read:
  - `/HELLO.TXT`
  - `/BOOT/PHASE35.TXT`
- implement the `sunlight-fs::FileSystem` trait
- no heap allocation during file reads

### VFS Mount Layout

```text
/     -> RamFs, read-only
/boot -> FAT32 on virtio-blk, read-only
```

### Phase 3.5 Measurable Tasks

- [ ] Add `tools/disk.sh` and deterministic FAT32 test image.
- [ ] Add QEMU virtio-blk flags to the test runner path.
- [ ] Add `sunlight-virtio` crate.
- [ ] Implement PCI scan and virtio-blk discovery.
- [ ] Implement `read_block` for LBA 0.
- [ ] Add `sunlight-fat` crate.
- [ ] Parse BPB and validate FAT32.
- [ ] Implement directory lookup for known files.
- [ ] Implement FAT32 read-only file reads.
- [ ] Mount `/boot -> FAT32`.
- [ ] Read `/boot/HELLO.TXT`.
- [ ] Read `/boot/BOOT/PHASE35.TXT`.
- [ ] Add `/boot/MISSING.TXT` ENOENT check.
- [ ] Add `tools/tests/phase3_5.expected`.
- [ ] Update `tools/test.sh phase3.5`.
- [ ] Create `docs/PHASE_3_5_SUMMARY.md`.

### Phase 3.5 Gate Lines

`./tools/test.sh phase3.5` must require:

```text
[BLK]  Found virtio-blk
[BLK]  Read LBA 0 OK
[FAT]  FAT32 detected
[VFS]  /boot OK
[VFS]  Read: "SunlightOS FAT32 boot volume\n"
[VFS]  Read: "Phase 3.5 FAT32 OK\n"
[VFS]  /boot/MISSING.TXT ENOENT OK
[SunlightOS] Phase 3.5 OK
```

The test script output must end with:

```text
✓ Phase 3.5 gate PASSED
```

### Phase 3.5 Sub-Prompt

```text
Implement SunlightOS Phase 3.5 only, following docs/PHASE_3_ROADMAP.md.
Assume Phase 3.0 passes. Do not start keyboard, TTY, login, mux, fork, exec, or
FAT write support.

Deliver:
- tools/disk.sh deterministic FAT32 image creation
- sunlight-virtio read-only virtio-blk path
- sunlight-fat read-only FAT32 implementation
- /boot mounted through VFS
- boot checks for HELLO.TXT, BOOT/PHASE35.TXT, and missing file ENOENT
- tools/tests/phase3_5.expected
- tools/test.sh phase3.5 support
- docs/PHASE_3_5_SUMMARY.md with completed TODO state

Run:
- cargo check --workspace
- ./tools/test.sh phase3.0
- ./tools/test.sh phase3.5
```

## Phase 3.6: SunlightTTY + Keyboard + Login/Mux

### Goal

Implement the first interactive user-facing environment: PS/2 keyboard input,
`tty_server`, login screen, terminal mux, and a minimal built-in shell.

### Explicit Non-Goals

- no `fork()`
- no `exec()`
- no password hashing
- no USB HID
- no Wayland compositor
- no full POSIX shell
- no persistent scrollback
- no xterm-256color

### New Workspace Members

Add:

```toml
"sunlight-tty",
"services/tty_server",
```

### Kernel Keyboard Driver

Add:

```text
kernel/src/arch/x86_64/keyboard.rs
```

Implement PS/2 scancode set 1:

- IRQ1 handler
- key press/release
- Shift/Ctrl/Alt modifier state
- basic US ASCII mapping
- event route to active TTY endpoint through IPC
- deterministic test injection path if QEMU keyboard input is hard to automate

Required logs:

```text
[KBD]  PS/2 keyboard initialized
[KBD]  IRQ1 handler installed
```

### sunlight-tty Crate

Add:

```text
sunlight-tty/
└── src/
    ├── lib.rs
    ├── login.rs
    ├── mux.rs
    ├── pane.rs
    ├── pty.rs
    ├── session.rs
    ├── shell.rs
    └── vt100.rs
```

### Login Requirements

- Render using existing `sunlight-tui` primitives/theme.
- Read users from `/etc/sunlight/users` through VFS.
- Fallback users:
  - `root:root`
  - `user:user`
- maximum 3 failed attempts, then 30 second lockout.
- plaintext credentials are acceptable in Phase 3.6.

### Mux and Shell Requirements

Minimum keybindings:

```text
Ctrl+T          -> new tab
Ctrl+1..0       -> switch tab
Ctrl+L          -> clear active pane
Enter           -> submit shell line
Backspace       -> edit shell line
Printable keys  -> append to shell line
```

Minimum shell commands:

```text
help
echo
clear
cat /etc/motd
cat /etc/sunlight/session.toml
whoami
uname
```

`cat` must use VFS IPC.

### VT100 Minimum

Support:

```text
\r
\n
\r\n
\x1b[A
\x1b[B
\x1b[C
\x1b[D
\x1b[H
\x1b[J
\x1b[K
\x1b[0m
\x1b[1m
\x1b[30m through \x1b[37m
\x1b[90m through \x1b[97m
\x1b[40m through \x1b[47m
```

No heap allocation in the parser.

### tty_server

Responsibilities:

- register as `tty`
- load session config from VFS
- render login
- receive keyboard events
- authenticate `root/root`
- switch to terminal mux
- run built-in shell tests

### Phase 3.6 Measurable Tasks

- [ ] Add `kernel/src/arch/x86_64/keyboard.rs`.
- [ ] Install IRQ1 handler.
- [ ] Define fixed `KeyEvent` type.
- [ ] Route key events to active TTY endpoint.
- [ ] Add deterministic key injection path.
- [ ] Add `sunlight-tty` crate.
- [ ] Implement no-alloc VT100 parser.
- [ ] Implement login state machine and auth fallback.
- [ ] Implement terminal mux with at least one tab.
- [ ] Implement built-in shell commands.
- [ ] Add `services/tty_server`.
- [ ] Register `tty_server` as `tty`.
- [ ] Load session config from VFS.
- [ ] Add `Ctrl+T` test.
- [ ] Add `tools/tests/phase3_6.expected`.
- [ ] Update `tools/test.sh phase3.6`.
- [ ] Create `docs/PHASE_3_6_SUMMARY.md`.

### Phase 3.6 Gate Lines

`./tools/test.sh phase3.6` must require:

```text
[KBD]  PS/2 keyboard initialized
[KBD]  IRQ1 handler installed
[TTY]  Registered as 'tty'
[TTY]  Login screen ready
[TTY]  Login success: root
[TTY]  Built-in shell ready
[TTY]  Output: root
[TTY]  Output: Welcome to SunlightOS
[TTY]  Ctrl+T test: new tab OK
[SunlightOS] Phase 3.6 OK
```

The test script output must end with:

```text
✓ Phase 3.6 gate PASSED
```

### Phase 3.6 Sub-Prompt

```text
Implement SunlightOS Phase 3.6 only, following docs/PHASE_3_ROADMAP.md.
Assume Phase 3.0 and Phase 3.5 pass. Do not implement fork, exec, FAT writes,
USB HID, Wayland, or a full shell.

Deliver:
- PS/2 keyboard IRQ1 path
- deterministic key injection test path
- sunlight-tty crate with login, mux, VT100, and built-in shell
- services/tty_server registered as "tty"
- VFS-backed users/session/motd reads
- tools/tests/phase3_6.expected
- tools/test.sh phase3.6 support
- docs/PHASE_3_6_SUMMARY.md with completed TODO state

Run:
- cargo check --workspace
- ./tools/test.sh phase3.5
- ./tools/test.sh phase3.6
```

## Future Phase 4+ Notes

Keep these out of Phase 3.x unless explicitly requested later:

- real `fork()` / `exec()`
- ELF loading from VFS
- IPC direct thread-switch fastpath
- write-capable filesystem operations
- users, groups, ACLs, password hashing
- USB HID
- compositor / Wayland
- network stack
