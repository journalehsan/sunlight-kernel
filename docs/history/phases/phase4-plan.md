# SunlightOS Phase 4 — Restructured for AI Agent Execution

## Overview & Agent Guidance

This document splits Phase 4 into **6 discrete agent sessions**, each with clear entry conditions, explicit deliverables, and verification gates. Each session is designed to fit within a single AI agent context window without ambiguity.

**Critical rules for every agent session:**
- Read the **Entry Conditions** block first — if they fail, STOP and report which gate is broken
- Follow **Implementation Order** within each session exactly
- Every `unsafe` block requires a `// SAFETY:` comment explaining why it is sound
- Run the gate test at the end — if it fails, debug before proceeding
- Never modify files outside the listed **Files to Create/Modify** section
- If you encounter an ambiguity not covered by this document, choose the most conservative/defensive option and leave a `// AGENT-NOTE:` comment

---

## Session 4.0 — File Descriptor Table & Capability Rights Foundation

### Purpose

Build the `FdTable` and `CapRights` infrastructure that every subsequent session depends on. This session has no kernel execution — it is pure data structure and type work that compiles cleanly.

### Entry Conditions (verify before writing any code)

```bash
# These must all pass before starting:
cargo test -p sunlight-fs    # Phase 3.x fs tests green
cargo test -p sunlight-kernel # Phase 3.x kernel tests green
grep -r "CapRights" kernel/src/ # Should find the STUB from Phase 3.8
```

If any entry condition fails: **STOP. Do not write code. Report the failure.**

### Context

Phase 3.8 left a `CapRights` stub that was defined but not enforced. This session replaces that stub with a full, enforced implementation. The `FdTable` is the single source of truth for all open file descriptors in a process. Every subsequent syscall that touches a file (read, write, mmap, dup, pipe) goes through `FdTable::check_rights()` — if this is wrong, everything downstream is wrong.

### Files to Create

```
sunlight-fs/src/capability.rs     # CapRights + FileDescriptor + FdTable
sunlight-fs/src/fd_table.rs       # FdTable methods (split for readability)
kernel/src/process/fd.rs          # process-level fd helpers
```

### Files to Modify

```
sunlight-fs/src/lib.rs            # pub mod capability; pub mod fd_table;
kernel/src/process/mod.rs         # pub mod fd; add FdTable to Process struct
kernel/src/process/process.rs     # add fd_table: FdTable field to Process
```

### Implementation — `sunlight-fs/src/capability.rs`

```rust
//! Capsicum-style capability rights for file descriptors.
//!
//! Rights flow in one direction only: they can be reduced, never expanded.
//! This is enforced by `FdTable::reduce_rights` which returns an error
//! if the caller attempts to add a right that was not already present.

use bitflags::bitflags;

bitflags! {
    /// The complete set of operations a file descriptor may perform.
    ///
    /// When a fd is duplicated (dup/dup2/fork), the child fd inherits
    /// exactly the parent's rights — it cannot gain new rights.
    ///
    /// Capsicum rule: rights ⊆ parent_rights (monotone reduction only).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CapRights: u64 {
        const READ      = 1 << 0;
        const WRITE     = 1 << 1;
        const SEEK      = 1 << 2;
        const FSTAT     = 1 << 3;
        const FCHMOD    = 1 << 4;
        const FCHOWN    = 1 << 5;
        const FTRUNCATE = 1 << 6;
        const MMAP_R    = 1 << 7;
        const MMAP_W    = 1 << 8;
        const MMAP_X    = 1 << 9;
        // Phase 5 network rights — defined now, unused until Phase 5
        const CONNECT   = 1 << 10;
        const BIND      = 1 << 11;
        const ACCEPT    = 1 << 12;
    }
}

impl CapRights {
    /// Rights for a read-only file descriptor (stdin, O_RDONLY).
    pub fn read_only() -> Self {
        Self::READ | Self::SEEK | Self::FSTAT
    }

    /// Rights for a write-only file descriptor (stdout, stderr, O_WRONLY).
    pub fn write_only() -> Self {
        Self::WRITE | Self::SEEK | Self::FSTAT
    }

    /// Rights for a read-write file descriptor (O_RDWR).
    pub fn read_write() -> Self {
        Self::READ | Self::WRITE | Self::SEEK | Self::FSTAT
    }

    /// Rights for stdin specifically (read, no seek — terminals are not seekable).
    pub fn stdin() -> Self {
        Self::READ | Self::FSTAT
    }

    /// Rights for stdout/stderr (write, no seek).
    pub fn stdout() -> Self {
        Self::WRITE | Self::FSTAT
    }

    /// Rights for a pipe read end.
    pub fn pipe_read() -> Self {
        Self::READ | Self::FSTAT
    }

    /// Rights for a pipe write end.
    pub fn pipe_write() -> Self {
        Self::WRITE | Self::FSTAT
    }

    /// Returns true if `self` is a subset of `other`.
    ///
    /// Used to verify: new_rights ⊆ current_rights before reduce_rights.
    pub fn is_subset_of(self, other: Self) -> bool {
        (self & other) == self
    }
}

/// File descriptor open flags (matching POSIX O_* constants numerically
/// where they will be exposed to Linux-compat layer in Phase 4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdFlags(pub u32);

impl FdFlags {
    pub const RDONLY:   u32 = 0o0;
    pub const WRONLY:   u32 = 0o1;
    pub const RDWR:     u32 = 0o2;
    pub const APPEND:   u32 = 0o2000;
    pub const CLOEXEC:  u32 = 0o2000000;
    pub const NONBLOCK: u32 = 0o4000;

    pub fn is_cloexec(self) -> bool {
        self.0 & Self::CLOEXEC != 0
    }

    pub fn is_append(self) -> bool {
        self.0 & Self::APPEND != 0
    }
}

/// An open file handle — the actual VFS object behind a fd number.
///
/// This wraps the VFS node reference and tracks seek position.
/// The kernel owns FileHandle; FdTable owns FileDescriptor.
#[derive(Debug)]
pub struct FileHandle {
    /// Inode ID in the VFS. Uniquely identifies the file across the filesystem.
    pub inode:    u64,
    /// Current seek offset. For non-seekable fds (pipes, terminals),
    /// this field is present but ignored — CapRights::SEEK is absent.
    pub offset:   u64,
    /// Type tag so syscall handlers know what backend to call.
    pub kind:     FileHandleKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileHandleKind {
    RegularFile,
    Directory,
    Pipe,       // read or write end — distinguished by CapRights
    Terminal,   // /dev/tty, /dev/console
    DevNull,    // /dev/null
}

/// One entry in a process's file descriptor table.
#[derive(Debug)]
pub struct FileDescriptor {
    /// The userspace-visible fd number (0, 1, 2, ..., 255).
    pub fd:     i32,
    /// The underlying VFS object.
    pub handle: FileHandle,
    /// Operations this fd is permitted to perform.
    /// Can only be reduced via `capability_limit()`, never expanded.
    pub rights: CapRights,
    /// Open flags (O_RDONLY etc., O_CLOEXEC).
    pub flags:  FdFlags,
}

/// Error type for fd and capability operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdError {
    /// The fd number is out of range or not open.
    BadFd,
    /// The fd table is full (>= 256 open fds).
    TooManyOpenFiles,
    /// The target fd number (for dup2) is out of range.
    InvalidFdNumber,
    /// Attempt to expand capability rights (Capsicum violation).
    CapabilityExpansionDenied,
    /// Process is in capability mode; path-based open is forbidden.
    CapabilityModeViolation,
}

pub type FdResult<T> = Result<T, FdError>;
```

### Implementation — `sunlight-fs/src/fd_table.rs`

```rust
//! FdTable: per-process file descriptor table.
//!
//! Layout: fixed array of 256 slots. Slot index == fd number.
//! Slot 0, 1, 2 are pre-populated by the kernel for stdin/stdout/stderr.
//!
//! Agent note: The array size (256) is a deliberate simplification.
//! Real kernels use dynamic structures. Do not change this for Phase 4.

use super::capability::{CapRights, FdError, FdFlags,
                        FdResult, FileDescriptor, FileHandle, FileHandleKind};

/// Maximum number of simultaneously open file descriptors per process.
pub const FD_TABLE_SIZE: usize = 256;

/// Per-process file descriptor table.
///
/// # Invariants
///
/// - `entries[n]` is `Some(fd)` iff fd number `n` is open.
/// - `fd.fd == n` for all `Some(fd)` entries (fd number matches slot).
/// - Rights in any entry are always a subset of the rights it was opened with.
/// - `stdin_handle`, `stdout_handle`, `stderr_handle` (slots 0, 1, 2) are
///   always valid while the process is alive (except after explicit close).
pub struct FdTable {
    entries: [Option<FileDescriptor>; FD_TABLE_SIZE],
}

impl FdTable {
    /// Create a new FdTable pre-populated with stdin/stdout/stderr
    /// connected to the terminal.
    ///
    /// Every process starts with these three fds. They are the only
    /// fds created without going through `open()`.
    pub fn new_with_stdio(terminal_inode: u64) -> Self {
        // SAFETY: Option<FileDescriptor> is valid when None.
        // We initialize explicitly below before any access.
        let mut entries: [Option<FileDescriptor>; FD_TABLE_SIZE] =
            core::array::from_fn(|_| None);

        entries[0] = Some(FileDescriptor {
            fd:     0,
            handle: FileHandle {
                inode:  terminal_inode,
                offset: 0,
                kind:   FileHandleKind::Terminal,
            },
            rights: CapRights::stdin(),
            flags:  FdFlags(FdFlags::RDONLY),
        });

        entries[1] = Some(FileDescriptor {
            fd:     1,
            handle: FileHandle {
                inode:  terminal_inode,
                offset: 0,
                kind:   FileHandleKind::Terminal,
            },
            rights: CapRights::stdout(),
            flags:  FdFlags(FdFlags::WRONLY),
        });

        entries[2] = Some(FileDescriptor {
            fd:     2,
            handle: FileHandle {
                inode:  terminal_inode,
                offset: 0,
                kind:   FileHandleKind::Terminal,
            },
            rights: CapRights::stdout(),
            flags:  FdFlags(FdFlags::WRONLY),
        });

        Self { entries }
    }

    /// Allocate the lowest available fd number >= `min_fd`.
    ///
    /// Returns `FdError::TooManyOpenFiles` if all slots are occupied.
    fn alloc_fd(&self, min_fd: usize) -> FdResult<i32> {
        for i in min_fd..FD_TABLE_SIZE {
            if self.entries[i].is_none() {
                return Ok(i as i32);
            }
        }
        Err(FdError::TooManyOpenFiles)
    }

    /// Open a new file descriptor with the given handle and rights.
    ///
    /// Returns the allocated fd number.
    /// `min_fd` allows callers to request fd >= N (used by dup2).
    pub fn open(
        &mut self,
        handle: FileHandle,
        rights: CapRights,
        flags:  FdFlags,
    ) -> FdResult<i32> {
        let fd_num = self.alloc_fd(0)?;
        self.entries[fd_num as usize] = Some(FileDescriptor {
            fd: fd_num,
            handle,
            rights,
            flags,
        });
        Ok(fd_num)
    }

    /// Close fd `fd`. Returns `FdError::BadFd` if not open.
    pub fn close(&mut self, fd: i32) -> FdResult<()> {
        let idx = self.validate_idx(fd)?;
        if self.entries[idx].is_none() {
            return Err(FdError::BadFd);
        }
        self.entries[idx] = None;
        Ok(())
    }

    /// Get an immutable reference to the descriptor for `fd`.
    pub fn get(&self, fd: i32) -> FdResult<&FileDescriptor> {
        let idx = self.validate_idx(fd)?;
        self.entries[idx].as_ref().ok_or(FdError::BadFd)
    }

    /// Get a mutable reference to the descriptor for `fd`.
    pub fn get_mut(&mut self, fd: i32) -> FdResult<&mut FileDescriptor> {
        let idx = self.validate_idx(fd)?;
        self.entries[idx].as_mut().ok_or(FdError::BadFd)
    }

    /// Check that `fd` has at least `required` rights.
    ///
    /// Returns `FdError::CapabilityExpansionDenied` (repurposed as
    /// "insufficient rights") if any bit in `required` is absent.
    ///
    /// # Naming note
    /// The error name is technically wrong here (this is a rights check,
    /// not an expansion attempt), but we reuse the same error enum to
    /// avoid proliferating error variants. A future pass can rename this
    /// to `InsufficientRights`.
    pub fn check_rights(&self, fd: i32, required: CapRights) -> FdResult<()> {
        let desc = self.get(fd)?;
        if !required.is_subset_of(desc.rights) {
            return Err(FdError::CapabilityExpansionDenied);
        }
        Ok(())
    }

    /// Reduce the rights on `fd` to `new_rights`.
    ///
    /// Returns `FdError::CapabilityExpansionDenied` if `new_rights`
    /// contains any bit not present in the current rights — this is the
    /// core Capsicum invariant.
    pub fn reduce_rights(&mut self, fd: i32, new_rights: CapRights) -> FdResult<()> {
        let desc = self.get_mut(fd)?;
        if !new_rights.is_subset_of(desc.rights) {
            return Err(FdError::CapabilityExpansionDenied);
        }
        desc.rights = new_rights;
        Ok(())
    }

    /// Duplicate `old_fd` to the lowest available fd number.
    ///
    /// The new fd inherits exactly the same rights as `old_fd`.
    pub fn dup(&mut self, old_fd: i32) -> FdResult<i32> {
        let old_idx = self.validate_idx(old_fd)?;
        let old = self.entries[old_idx].as_ref().ok_or(FdError::BadFd)?;

        // Clone the handle (new seek offset starts at same position).
        let new_handle = FileHandle {
            inode:  old.handle.inode,
            offset: old.handle.offset,
            kind:   old.handle.kind,
        };
        let rights = old.rights;
        // O_CLOEXEC is NOT inherited by dup() — POSIX rule.
        let flags = FdFlags(old.flags.0 & !FdFlags::CLOEXEC);

        let new_fd = self.alloc_fd(0)?;
        self.entries[new_fd as usize] = Some(FileDescriptor {
            fd: new_fd,
            handle: new_handle,
            rights,
            flags,
        });
        Ok(new_fd)
    }

    /// Duplicate `old_fd` to exactly `new_fd`.
    ///
    /// If `new_fd` is already open, it is closed silently first (POSIX).
    /// Returns `FdError::InvalidFdNumber` if `new_fd` is out of range.
    pub fn dup2(&mut self, old_fd: i32, new_fd: i32) -> FdResult<i32> {
        if new_fd < 0 || new_fd as usize >= FD_TABLE_SIZE {
            return Err(FdError::InvalidFdNumber);
        }
        if old_fd == new_fd {
            // dup2(x, x) is a no-op if x is open; error if closed.
            let _ = self.get(old_fd)?;
            return Ok(new_fd);
        }

        let old_idx = self.validate_idx(old_fd)?;
        let old = self.entries[old_idx].as_ref().ok_or(FdError::BadFd)?;

        let new_handle = FileHandle {
            inode:  old.handle.inode,
            offset: old.handle.offset,
            kind:   old.handle.kind,
        };
        let rights = old.rights;
        let flags  = FdFlags(old.flags.0 & !FdFlags::CLOEXEC);

        // Close new_fd if already open (silently — POSIX).
        self.entries[new_fd as usize] = None;

        self.entries[new_fd as usize] = Some(FileDescriptor {
            fd: new_fd,
            handle: new_handle,
            rights,
            flags,
        });
        Ok(new_fd)
    }

    /// Clone this FdTable for use in fork().
    ///
    /// All fds are duplicated. O_CLOEXEC fds are KEPT — they are only
    /// closed by execve(), not by fork(). This matches POSIX behavior.
    pub fn clone_for_fork(&self) -> Self {
        let mut new_table = Self {
            entries: core::array::from_fn(|_| None),
        };
        for (i, slot) in self.entries.iter().enumerate() {
            if let Some(desc) = slot {
                new_table.entries[i] = Some(FileDescriptor {
                    fd:     desc.fd,
                    handle: FileHandle {
                        inode:  desc.handle.inode,
                        offset: desc.handle.offset,
                        kind:   desc.handle.kind,
                    },
                    rights: desc.rights,
                    flags:  desc.flags,
                });
            }
        }
        new_table
    }

    /// Close all FD_CLOEXEC fds.
    ///
    /// Called by execve() after the new image is loaded.
    /// Non-CLOEXEC fds survive exec (shell pipes, etc.).
    pub fn close_cloexec_fds(&mut self) {
        for slot in self.entries.iter_mut() {
            if let Some(desc) = slot {
                if desc.flags.is_cloexec() {
                    *slot = None;
                }
            }
        }
    }

    /// Iterate over all open file descriptors.
    pub fn iter(&self) -> impl Iterator<Item = &FileDescriptor> {
        self.entries.iter().filter_map(|s| s.as_ref())
    }

    // ─── Private helpers ──────────────────────────────────────────────────

    fn validate_idx(&self, fd: i32) -> FdResult<usize> {
        if fd < 0 || fd as usize >= FD_TABLE_SIZE {
            Err(FdError::BadFd)
        } else {
            Ok(fd as usize)
        }
    }
}
```

### Process struct update — `kernel/src/process/process.rs`

Add this field to the existing `Process` struct. Do not change any other field:

```rust
// In Process { ... }:
pub fd_table:         FdTable,
pub capability_mode:  bool,   // true after capability_enter(); irreversible
```

Initialize in `Process::new()`:

```rust
fd_table:        FdTable::new_with_stdio(TERMINAL_INODE),
capability_mode: false,
```

Where `TERMINAL_INODE` is the inode number of `/dev/console` in your RamFs. If you don't have a `/dev/console` yet, use inode `0` and add a `// AGENT-NOTE: replace with real terminal inode` comment.

### Gate Test — `tests/phase4_0_fd_table.rs`

```rust
//! Gate test: FdTable and CapRights correctness.
//! Run with: cargo test -p sunlight-fs phase4_0

#[cfg(test)]
mod tests {
    use sunlight_fs::fd_table::FdTable;
    use sunlight_fs::capability::{CapRights, FdFlags,
                                   FileHandle, FileHandleKind};

    fn test_handle() -> FileHandle {
        FileHandle { inode: 42, offset: 0, kind: FileHandleKind::RegularFile }
    }

    #[test]
    fn stdio_pre_populated() {
        let table = FdTable::new_with_stdio(0);
        assert!(table.get(0).is_ok(), "stdin missing");
        assert!(table.get(1).is_ok(), "stdout missing");
        assert!(table.get(2).is_ok(), "stderr missing");
        assert!(table.get(3).is_err(), "fd 3 should be empty");
    }

    #[test]
    fn rights_read_only_cannot_write() {
        let table = FdTable::new_with_stdio(0);
        // stdin (fd=0) has READ rights, not WRITE
        assert!(table.check_rights(0, CapRights::READ).is_ok());
        assert!(table.check_rights(0, CapRights::WRITE).is_err());
    }

    #[test]
    fn reduce_rights_works() {
        let mut table = FdTable::new_with_stdio(0);
        // Open a R/W fd, then reduce to R only
        let fd = table.open(
            test_handle(),
            CapRights::read_write(),
            FdFlags(0),
        ).unwrap();
        table.reduce_rights(fd, CapRights::read_only()).unwrap();
        assert!(table.check_rights(fd, CapRights::WRITE).is_err());
        assert!(table.check_rights(fd, CapRights::READ).is_ok());
    }

    #[test]
    fn cannot_expand_rights() {
        let mut table = FdTable::new_with_stdio(0);
        let fd = table.open(
            test_handle(),
            CapRights::READ,
            FdFlags(0),
        ).unwrap();
        // Attempt to add WRITE — must fail
        assert!(table.reduce_rights(fd, CapRights::READ | CapRights::WRITE).is_err());
    }

    #[test]
    fn dup_inherits_rights_not_cloexec() {
        let mut table = FdTable::new_with_stdio(0);
        let fd = table.open(
            test_handle(),
            CapRights::read_only(),
            FdFlags(FdFlags::CLOEXEC),
        ).unwrap();
        let fd2 = table.dup(fd).unwrap();
        let desc2 = table.get(fd2).unwrap();
        assert_eq!(desc2.rights, CapRights::read_only());
        assert!(!desc2.flags.is_cloexec(), "dup must clear CLOEXEC");
    }

    #[test]
    fn dup2_closes_existing() {
        let mut table = FdTable::new_with_stdio(0);
        let fd = table.open(test_handle(), CapRights::READ, FdFlags(0)).unwrap();
        // dup2 onto stdout (fd=1) — should close stdout first
        table.dup2(fd, 1).unwrap();
        // fd=1 is now a clone of fd
        assert_eq!(table.get(1).unwrap().handle.inode, 42);
    }

    #[test]
    fn clone_for_fork_keeps_cloexec() {
        let mut table = FdTable::new_with_stdio(0);
        let fd = table.open(
            test_handle(),
            CapRights::READ,
            FdFlags(FdFlags::CLOEXEC),
        ).unwrap();
        let child = table.clone_for_fork();
        // fork: CLOEXEC fds ARE present in child
        assert!(child.get(fd).is_ok(), "fork keeps CLOEXEC fds");
    }

    #[test]
    fn close_cloexec_removes_cloexec_only() {
        let mut table = FdTable::new_with_stdio(0);
        let fd_cloexec = table.open(
            test_handle(),
            CapRights::READ,
            FdFlags(FdFlags::CLOEXEC),
        ).unwrap();
        let fd_normal = table.open(
            test_handle(),
            CapRights::READ,
            FdFlags(0),
        ).unwrap();
        table.close_cloexec_fds();
        assert!(table.get(fd_cloexec).is_err(), "CLOEXEC fd must be closed by exec");
        assert!(table.get(fd_normal).is_ok(),   "normal fd must survive exec");
    }

    #[test]
    fn table_full_returns_error() {
        let mut table = FdTable::new_with_stdio(0);
        // Fill slots 3..255
        for _ in 3..256 {
            let _ = table.open(test_handle(), CapRights::READ, FdFlags(0));
        }
        // Now table is full
        assert!(table.open(test_handle(), CapRights::READ, FdFlags(0)).is_err());
    }
}
```

### Session 4.0 Gate

```bash
cargo test -p sunlight-fs phase4_0
# Expected: 8 tests pass, 0 fail
# If any test fails: fix before proceeding to Session 4.1
```

---

## Session 4.1 — Pipe Syscall & Kernel Pipe Buffer

### Purpose

Implement the `pipe()` syscall and the kernel-side ring buffer. This session is a prerequisite for fork (child/parent communicate via pipes) and for sunshell v0.2.

### Entry Conditions

```bash
cargo test -p sunlight-fs phase4_0   # Session 4.0 gate must be green
grep "FdTable" kernel/src/process/process.rs  # Must exist
```

### Context

A pipe is a unidirectional byte channel with two ends: a read fd and a write fd. The kernel owns the buffer. When the buffer is empty, `read()` on the read end blocks the process (or returns `EAGAIN` if O_NONBLOCK). When the buffer is full, `write()` on the write end blocks (or raises SIGPIPE if no readers remain).

**Agent decision guide:**
- Pipe buffer is exactly 4096 bytes. Do not make it configurable yet.
- Use a ring buffer (head/tail indices), not a VecDeque.
- SIGPIPE delivery is stubbed here (just set a flag); real delivery happens in Session 4.3.
- Blocking behavior in Phase 4 means: spin-yield in the scheduler. Full async blocking comes in Phase 5.

### Files to Create

```
kernel/src/process/pipe.rs    # Pipe struct, ring buffer, syscall handler
```

### Files to Modify

```
kernel/src/process/mod.rs     # pub mod pipe;
kernel/src/syscall/mod.rs     # route SunlightSyscall::Pipe
sunlight-fs/src/capability.rs # (no change — Pipe FileHandleKind already defined)
```

### Syscall enum additions — `kernel/src/syscall/nr.rs`

```rust
// Add to SunlightSyscall:
Pipe  = 47,   // (i32*, i32*) -> i32
```

### Implementation — `kernel/src/process/pipe.rs`

```rust
//! Kernel pipe: unidirectional byte channel with a 4096-byte ring buffer.
//!
//! # Lifetime
//!
//! A Pipe is reference-counted by reader_count / writer_count.
//! When writer_count drops to 0, subsequent reads return EOF (0 bytes).
//! When reader_count drops to 0, subsequent writes generate SIGPIPE.
//!
//! # Thread safety
//!
//! In Phase 4 the kernel is single-core, so no locking is needed.
//! A `// AGENT-NOTE: add spinlock before SMP` comment marks every place
//! that will need a lock in Phase 6.

/// Size of the pipe ring buffer in bytes.
/// Must be a power of two for cheap modulo arithmetic.
pub const PIPE_BUF_SIZE: usize = 4096;

/// Kernel pipe buffer.
///
/// Shared between the read end and write end file handles via a global
/// pipe table (see `PipeTable` below). The file handles store a pipe ID;
/// the pipe table maps IDs to `Pipe` instances.
pub struct Pipe {
    buf:          [u8; PIPE_BUF_SIZE],
    /// Index of the next byte to read.
    head:         usize,
    /// Number of bytes currently in the buffer.
    len:          usize,
    /// Number of open read-end file descriptors referencing this pipe.
    /// 0 → writer should get SIGPIPE on next write.
    pub reader_count: u32,
    /// Number of open write-end file descriptors referencing this pipe.
    /// 0 → reader gets EOF (read returns 0).
    pub writer_count: u32,
}

impl Pipe {
    pub fn new() -> Self {
        Self {
            // SAFETY: [u8; N] is valid when zero-initialized.
            buf:          [0u8; PIPE_BUF_SIZE],
            head:         0,
            len:          0,
            reader_count: 1,
            writer_count: 1,
        }
    }

    /// Read up to `dst.len()` bytes from the pipe.
    ///
    /// Returns:
    /// - `Ok(n)` where n > 0: bytes read into `dst[..n]`.
    /// - `Ok(0)`: pipe is empty AND writer_count == 0 (EOF).
    /// - `Err(PipeError::WouldBlock)`: empty, writers still alive → caller must retry.
    pub fn read(&mut self, dst: &mut [u8]) -> Result<usize, PipeError> {
        if self.len == 0 {
            if self.writer_count == 0 {
                return Ok(0); // EOF
            }
            return Err(PipeError::WouldBlock);
        }
        let n = dst.len().min(self.len);
        for i in 0..n {
            dst[i] = self.buf[(self.head + i) % PIPE_BUF_SIZE];
        }
        self.head = (self.head + n) % PIPE_BUF_SIZE;
        self.len  -= n;
        Ok(n)
    }

    /// Write `src` bytes into the pipe.
    ///
    /// Returns:
    /// - `Ok(n)`: bytes written (may be < src.len() if buffer fills up).
    /// - `Err(PipeError::NoReaders)`: reader_count == 0 → caller must send SIGPIPE.
    /// - `Err(PipeError::WouldBlock)`: buffer full → caller must retry.
    pub fn write(&mut self, src: &[u8]) -> Result<usize, PipeError> {
        if self.reader_count == 0 {
            return Err(PipeError::NoReaders);
        }
        if self.len == PIPE_BUF_SIZE {
            return Err(PipeError::WouldBlock);
        }
        let space = PIPE_BUF_SIZE - self.len;
        let n = src.len().min(space);
        let tail = (self.head + self.len) % PIPE_BUF_SIZE;
        for i in 0..n {
            self.buf[(tail + i) % PIPE_BUF_SIZE] = src[i];
        }
        self.len += n;
        Ok(n)
    }

    pub fn bytes_available(&self) -> usize { self.len }
    pub fn space_available(&self) -> usize { PIPE_BUF_SIZE - self.len }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeError {
    /// No data available and writers are still alive.
    WouldBlock,
    /// All readers closed; writer should receive SIGPIPE.
    NoReaders,
    /// Invalid pipe ID.
    InvalidPipeId,
}

// ─── Pipe Table ────────────────────────────────────────────────────────────
//
// Global table mapping pipe IDs to Pipe instances.
// In Phase 4: static array, max 64 concurrent pipes.
// AGENT-NOTE: replace with heap-allocated table when allocator is mature.

pub const MAX_PIPES: usize = 64;

/// Opaque identifier for a kernel pipe.
/// Stored in FileHandle::inode for Pipe-kind handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipeId(pub u32);

pub struct PipeTable {
    pipes: [Option<Pipe>; MAX_PIPES],
    /// Monotonically increasing counter to avoid ABA with freed slots.
    next_id: u32,
}

impl PipeTable {
    pub const fn new() -> Self {
        // SAFETY: Option<Pipe> is valid when None.
        Self {
            pipes:   [const { None }; MAX_PIPES],
            next_id: 1,
        }
    }

    /// Create a new pipe, returning its ID.
    pub fn create(&mut self) -> Option<PipeId> {
        for (i, slot) in self.pipes.iter_mut().enumerate() {
            if slot.is_none() {
                let id = self.next_id;
                self.next_id = self.next_id.wrapping_add(1);
                // Use the index as the id for O(1) lookup.
                // We accept id collision risk at wrapping — 4 billion
                // pipe creations before an agent session ends is implausible.
                *slot = Some(Pipe::new());
                // Store index in low bits, counter in high — for now, just use index.
                let _ = id; // suppress unused warning
                return Some(PipeId(i as u32));
            }
        }
        None // too many pipes
    }

    pub fn get(&self, id: PipeId) -> Option<&Pipe> {
        self.pipes.get(id.0 as usize)?.as_ref()
    }

    pub fn get_mut(&mut self, id: PipeId) -> Option<&mut Pipe> {
        self.pipes.get_mut(id.0 as usize)?.as_mut()
    }

    pub fn destroy(&mut self, id: PipeId) {
        if let Some(slot) = self.pipes.get_mut(id.0 as usize) {
            *slot = None;
        }
    }
}

// ─── Syscall handler ───────────────────────────────────────────────────────

use crate::process::process::Process;
use sunlight_fs::capability::{CapRights, FdFlags, FileHandle, FileHandleKind};

/// sys_pipe: create a pipe, return (read_fd, write_fd) into the process's FdTable.
///
/// On Linux ABI (used by Helios in Phase 4.5), the two fds are written to
/// a user-space array. Here we return them as a Rust tuple; the syscall
/// dispatch layer writes them to the appropriate registers/memory.
pub fn sys_pipe(
    process: &mut Process,
    pipe_table: &mut PipeTable,
) -> Result<(i32, i32), PipeError> {
    let id = pipe_table.create().ok_or(PipeError::InvalidPipeId)?;

    // Read end: fd with READ rights, handle kind=Pipe, inode=pipe_id
    let read_fd = process.fd_table.open(
        FileHandle {
            inode:  id.0 as u64,
            offset: 0,
            kind:   FileHandleKind::Pipe,
        },
        CapRights::pipe_read(),
        FdFlags(FdFlags::RDONLY),
    ).map_err(|_| PipeError::InvalidPipeId)?;

    // Write end: fd with WRITE rights
    let write_fd = process.fd_table.open(
        FileHandle {
            inode:  id.0 as u64,
            offset: 0,
            kind:   FileHandleKind::Pipe,
        },
        CapRights::pipe_write(),
        FdFlags(FdFlags::WRONLY),
    ).map_err(|_| PipeError::InvalidPipeId)?;

    Ok((read_fd, write_fd))
}
```

### Gate Test — `tests/phase4_1_pipe.rs`

```rust
//! Gate test: Pipe ring buffer behavior.
//! Kernel integration (sys_pipe) is tested in phase4_4 (sunshell).

#[cfg(test)]
mod pipe_tests {
    use crate::process::pipe::{Pipe, PipeError};

    #[test]
    fn empty_pipe_read_blocks() {
        let mut p = Pipe::new();
        let mut buf = [0u8; 16];
        assert_eq!(p.read(&mut buf), Err(PipeError::WouldBlock));
    }

    #[test]
    fn write_then_read_roundtrip() {
        let mut p = Pipe::new();
        let n = p.write(b"hello pipe").unwrap();
        assert_eq!(n, 10);
        let mut buf = [0u8; 16];
        let n2 = p.read(&mut buf).unwrap();
        assert_eq!(&buf[..n2], b"hello pipe");
    }

    #[test]
    fn read_eof_when_writer_gone() {
        let mut p = Pipe::new();
        p.writer_count = 0;
        let mut buf = [0u8; 8];
        assert_eq!(p.read(&mut buf), Ok(0)); // EOF
    }

    #[test]
    fn write_error_when_reader_gone() {
        let mut p = Pipe::new();
        p.reader_count = 0;
        assert_eq!(p.write(b"x"), Err(PipeError::NoReaders));
    }

    #[test]
    fn partial_write_when_buffer_nearly_full() {
        let mut p = Pipe::new();
        // Fill to 4090 bytes
        let big = [0u8; 4090];
        p.write(&big).unwrap();
        // 6 bytes of space remain
        let n = p.write(b"1234567890").unwrap();
        assert_eq!(n, 6);
    }

    #[test]
    fn ring_buffer_wraps_correctly() {
        let mut p = Pipe::new();
        // Write 4000 bytes, read 4000 (advances head to 4000)
        p.write(&[0xAAu8; 4000]).unwrap();
        let mut buf = [0u8; 4000];
        p.read(&mut buf).unwrap();
        // Now write 200 bytes — wraps around the ring
        p.write(&[0xBBu8; 200]).unwrap();
        let mut buf2 = [0u8; 200];
        p.read(&mut buf2).unwrap();
        assert!(buf2.iter().all(|&b| b == 0xBB));
    }
}
```

### Session 4.1 Gate

```bash
cargo test -p sunlight-kernel phase4_1
# Expected: 6 pipe tests pass
```

---

## Session 4.2 — fork() with Copy-on-Write Address Space

### Purpose

Implement `fork()`: clone a process with Copy-on-Write (CoW) memory, new PID, inherited FdTable. This is the most complex session. Read every sub-section before writing code.

### Entry Conditions

```bash
cargo test -p sunlight-fs   phase4_0    # FdTable green
cargo test -p sunlight-kernel phase4_1  # Pipe green
grep "PhysicalMemoryManager" kernel/src/ -r  # PMM must exist from Phase 3
grep "AddressSpace" kernel/src/ -r           # Paging must exist
```

### Context & Decisions for Agent

**What CoW means in hardware terms:**

When `fork()` is called, the kernel does NOT copy any memory pages. Instead, it:

1. Walks the parent's page table entries
2. Marks every writable page as read-only in BOTH parent and child page tables
3. Marks the page table entries with a custom `COW` bit (use bit 9 of PTE, which is available to software on x86_64)
4. When either process tries to write to a CoW page, the CPU raises a page fault (error code has bit 1 set = write fault)
5. The page fault handler detects the CoW bit, allocates a new physical frame, copies the page contents, remaps the faulting address as writable, clears the CoW bit, and resumes the instruction

**x86_64 PTE bit layout:**

```
Bit 0: Present
Bit 1: Writable
Bit 2: User-accessible
Bit 3: Write-through
Bit 4: Cache-disable
Bit 5: Accessed
Bit 6: Dirty
Bit 7: Huge page (must be 0 for 4KB pages)
Bit 8: Global
Bit 9: (available) ← USE THIS for COW flag
Bits 10: (available) ← reserved for future use
Bits 11: (available)
Bits 12–51: Physical frame address (>> 12)
Bits 52–62: (available)
Bit 63: No-execute
```

**Agent decision: what address space type to expect**

Your Phase 3.8 has an `AddressSpace` struct. If it does not have a `cow_fork()` method, you are adding one. If `AddressSpace` does not exist as a struct (unlikely given Phase 3.8 passed), STOP and report.

**Agent decision: page fault handler location**

The page fault handler in Phase 3 likely lives in `kernel/src/arch/x86_64/interrupts.rs` as `handle_page_fault` or similar. Find it with:
```bash
grep -r "page_fault\|PageFault\|#PF\|14 =>" kernel/src/arch/ --include="*.rs" -l
```
Add the CoW check at the TOP of this function, before the existing panic/kill logic.

### Files to Create

```
kernel/src/process/fork.rs          # sys_fork + CoW logic
kernel/src/mm/cow.rs                # CoW page fault handler (extracted)
```

### Files to Modify

```
kernel/src/process/mod.rs           # pub mod fork;
kernel/src/mm/mod.rs                # pub mod cow;
kernel/src/arch/x86_64/interrupts.rs # call cow::handle_cow_fault in page fault
kernel/src/process/process.rs       # add ppid, child_pids fields to Process
kernel/src/sched/mod.rs             # expose add_process()
```

### Process struct additions

```rust
// In Process struct, add:
pub ppid:       Option<Pid>,
pub children:   alloc::vec::Vec<Pid>,
pub exit_status: Option<i32>,       // Some(n) when process has exited (zombie)
```

### Implementation — `kernel/src/mm/cow.rs`

```rust
//! Copy-on-Write page fault handling.
//!
//! This module is called by the page fault interrupt handler (interrupt 14).
//! It is the ONLY place that resolves CoW faults. Do not inline this logic
//! into the interrupt handler — the handler is already complex.

use x86_64::structures::paging::{PageTableFlags, PhysFrame};
use x86_64::VirtAddr;

/// Bit 9 of the page table entry, available for OS use.
/// We use it to mark pages as Copy-on-Write.
pub const PTE_COW: u64 = 1 << 9;

/// x86_64 page fault error code bit 1: the fault was caused by a write.
pub const PF_WRITE: u64 = 1 << 1;

/// x86_64 page fault error code bit 2: fault occurred in user mode.
pub const PF_USER: u64 = 1 << 2;

/// Attempt to handle a CoW page fault.
///
/// Returns `true` if the fault was a valid CoW fault and has been resolved.
/// Returns `false` if this is not a CoW fault (caller should handle normally).
///
/// # Arguments
/// - `fault_addr`: the virtual address that caused the fault (from CR2)
/// - `error_code`: x86_64 page fault error code from the interrupt frame
///
/// # When this function is called
/// Only when `error_code & PF_WRITE != 0` (write fault) AND the PTE
/// for `fault_addr` has `PTE_COW` set. The check for PTE_COW must be
/// done in the page fault handler BEFORE calling this function.
///
/// # Safety contract
/// - `fault_addr` must be page-aligned (round down before calling).
/// - Current process must have a valid address space.
/// - PMM must have at least one free frame; OOM here is a kernel panic.
pub fn handle_cow_fault(
    fault_addr: VirtAddr,
    _error_code: u64,
    // In your kernel, these come from global state or are passed as refs.
    // Use whatever pattern your Phase 3 code established.
) -> bool {
    // SAFETY: We access the current process's page table only while
    // interrupts are disabled (we are in the interrupt handler).
    // No other CPU core can modify this page table concurrently
    // (single-core in Phase 4).
    let current = match crate::process::current_process_mut() {
        Some(p) => p,
        None    => return false, // fault before first process — kernel bug
    };

    let page_addr = fault_addr.align_down(4096u64);

    // Step 1: Look up the PTE for fault_addr.
    let pte_value = match current.address_space.get_pte(page_addr) {
        Some(v) => v,
        None    => return false, // not mapped at all
    };

    // Step 2: Check if CoW bit is set.
    if pte_value & PTE_COW == 0 {
        return false; // not a CoW page — let caller handle
    }

    let old_frame_phys = pte_value & 0x000F_FFFF_FFFF_F000; // bits 12..51

    // Step 3: Allocate a new physical frame.
    let new_frame = crate::mm::PMM
        .lock()
        .alloc_frame()
        .expect("[CoW] Out of memory during CoW fault — kernel panic");

    // Step 4: Copy the old page contents into the new frame.
    // SAFETY: Both physical addresses are valid (PMM guarantees allocation
    // success above). We map them temporarily via the kernel's identity
    // mapping (first 4GiB is identity-mapped in Phase 3 kernel space).
    unsafe {
        let src = old_frame_phys as *const u8;
        let dst = new_frame.start_address().as_u64() as *mut u8;
        core::ptr::copy_nonoverlapping(src, dst, 4096);
    }

    // Step 5: Remap the page as writable, clear the CoW bit.
    let new_flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER_ACCESSIBLE;
    // Do NOT set PTE_COW — this page is now private to this process.

    current.address_space
        .remap_page(page_addr, new_frame, new_flags)
        .expect("[CoW] Failed to remap page after CoW — kernel panic");

    // Step 6: Flush the TLB entry for this page.
    x86_64::instructions::tlb::flush(page_addr);

    true // fault resolved — CPU will re-execute the faulting instruction
}
```

### Implementation — `kernel/src/process/fork.rs`

```rust
//! fork() system call implementation.
//!
//! # What fork does
//!
//! Creates a child process that is an exact copy of the parent at the
//! moment fork() is called, with these differences:
//! - Child has a new PID
//! - Child's ppid = parent's pid
//! - In the parent, fork() returns the child's PID
//! - In the child, fork() returns 0
//! - Memory is shared Copy-on-Write (not copied eagerly)
//! - FdTable is cloned (both parent and child have independent tables
//!   pointing at the same underlying files initially)
//! - Signal handlers are inherited
//! - Pending signals are NOT inherited (child starts clean)
//!
//! # What fork does NOT do
//! - Does NOT copy memory (CoW handles that lazily)
//! - Does NOT run the child immediately (scheduler decides)
//! - Does NOT clone threads (Phase 4 is single-threaded per process)

use crate::process::process::{Process, Pid};
use crate::sched::Scheduler;
use crate::mm::cow::PTE_COW;
use x86_64::structures::paging::PageTableFlags;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkError {
    /// Process table is full.
    TooManyProcesses,
    /// Address space cloning failed.
    AddressSpaceCloneFailed,
    /// Out of memory for child process struct.
    OutOfMemory,
}

/// sys_fork: create a child process.
///
/// Returns `Ok((child_pid, 0))` — the caller (syscall dispatch) puts
/// `child_pid` in the parent's return register and `0` in the child's.
///
/// # Implementation note
///
/// After this function returns, both parent and child are in the
/// scheduler's run queue. The scheduler picks which one runs first.
/// The child's saved registers must have rax=0 (fork returns 0 in child).
pub fn sys_fork(
    parent: &mut Process,
    sched: &mut Scheduler,
) -> Result<Pid, ForkError> {
    // Step 1: Clone the address space with CoW semantics.
    // This walks every PTE, marks writable pages as read-only + COW,
    // and returns a new page table that shares all frames.
    let child_address_space = parent.address_space
        .cow_clone()
        .map_err(|_| ForkError::AddressSpaceCloneFailed)?;

    // Step 2: Allocate a new PID.
    let child_pid = sched.alloc_pid()
        .ok_or(ForkError::TooManyProcesses)?;

    // Step 3: Clone the FdTable.
    // fork() keeps CLOEXEC fds — execve() will remove them.
    let child_fd_table = parent.fd_table.clone_for_fork();

    // Step 4: Clone credential.
    // Child starts with identical uid/gid to parent.
    let child_cred = parent.credential.clone();

    // Step 5: Clone signal handlers (not pending signals).
    let child_sig_handlers = parent.signal_handlers.clone();

    // Step 6: Build child Process struct.
    let child = Process {
        pid:              child_pid,
        ppid:             Some(parent.pid),
        credential:       child_cred,
        address_space:    child_address_space,
        fd_table:         child_fd_table,
        capability_mode:  parent.capability_mode,
        signal_handlers:  child_sig_handlers,
        pending_signals:  0u64,  // start clean
        children:         alloc::vec::Vec::new(),
        exit_status:      None,
        // Copy saved register state from parent — child will return 0 from fork.
        // The syscall dispatch layer sets child's rax=0 after this function.
        saved_regs:       parent.saved_regs.clone(),
        // ... other fields your Process struct has from Phase 3 ...
        name:             parent.name.clone(),
        state:            crate::process::ProcessState::Ready,
    };

    // Step 7: Register child as parent's child.
    parent.children.push(child_pid);

    // Step 8: Add child to scheduler.
    sched.add_process(child)
        .map_err(|_| ForkError::TooManyProcesses)?;

    // Step 9: Mark parent's writable pages as CoW (if not already done).
    // cow_clone() on the address space already did this — but we must
    // also flush the parent's TLB so the hardware sees the new read-only PTEs.
    x86_64::instructions::tlb::flush_all();

    serial_println!("[FORK] pid={} forked, child pid={}", parent.pid, child_pid);
    Ok(child_pid)
}
```

### Address space CoW clone — add to your AddressSpace impl

```rust
// In kernel/src/mm/address_space.rs (or wherever AddressSpace lives):

impl AddressSpace {
    /// Clone this address space for fork() using Copy-on-Write.
    ///
    /// # What this does
    ///
    /// 1. Creates a new empty page table.
    /// 2. For every present PTE in this address space:
    ///    - If the page is writable: mark it read-only in BOTH the parent
    ///      and the new child PTE, and set PTE_COW bit in both.
    ///    - If the page is already read-only: copy the PTE as-is (no CoW
    ///      needed — a write will fault and we'll CoW it then).
    ///    - Copy the physical frame reference (no allocation yet).
    /// 3. Return the new child AddressSpace.
    ///
    /// # Safety
    ///
    /// Must be called with interrupts disabled or while holding the
    /// address space lock. In Phase 4 (single-core), interrupts are
    /// disabled during fork() execution.
    pub fn cow_clone(&mut self) -> Result<AddressSpace, AddressSpaceError> {
        let mut child = AddressSpace::new_empty()?;

        // Walk every page in the parent's virtual address space.
        // Implementation depends on your paging structure from Phase 3.
        // The pattern is: iterate over L4 → L3 → L2 → L1 entries.
        for (vaddr, pte_flags, phys_frame) in self.iter_mapped_pages() {
            let is_writable = pte_flags.contains(PageTableFlags::WRITABLE);

            if is_writable {
                // Mark parent page as read-only + CoW.
                let cow_flags = (pte_flags - PageTableFlags::WRITABLE)
                    | PageTableFlags::from_bits_truncate(PTE_COW);
                self.remap_page(vaddr, phys_frame, cow_flags)?;

                // Map same frame in child as read-only + CoW.
                child.map_page(vaddr, phys_frame, cow_flags)?;
            } else {
                // Already read-only — share as-is (could be text segment).
                // AGENT-NOTE: if the page already has PTE_COW, keep it.
                child.map_page(vaddr, phys_frame, pte_flags)?;
            }
        }

        Ok(child)
    }
}
```

### Gate Test

```bash
# Integration test: fork creates child, CoW fault resolves correctly.
# This test runs in QEMU via the kernel test harness established in Phase 3.
./tools/test.sh phase4_2
```

Expected serial output:
```
[FORK] pid=1 forked, child pid=2
[CoW]  write fault at 0x... resolved (new frame allocated)
[FORK] gate PASSED
```

---

## Session 4.3 — execve() + waitpid()

### Purpose

Implement `execve()` (static ELF path only; dynamic path comes in Session 4.4) and `waitpid()` with zombie reaping. Together with fork, this completes the Unix process lifecycle.

### Entry Conditions

```bash
./tools/test.sh phase4_2   # fork + CoW must pass
grep "load_static_elf"  sunlight-elf/src/lib.rs   # must exist from Phase 3.8
grep "setup_stack"      sunlight-elf/src/lib.rs   # must exist from Phase 3.8
```

### Context

**execve() replaces the calling process image.** After a successful execve:
- Old address space is destroyed
- New address space contains the new binary
- Signal handlers reset to SIG_DFL (except SIG_IGN, which survives exec)
- FD_CLOEXEC fds are closed
- PID, PPID, credential are preserved
- The function NEVER returns on success (it diverges into the new process)

**waitpid() collects exit status from child processes.** A child that has exited but not been waited on is a "zombie" — it retains its PID and exit status in the process table until the parent calls waitpid. In Phase 4, zombies are allowed. Orphan reaping (when parent exits first) is deferred to Phase 5.

**execve argv/envp stack layout (SysV ABI):**
```
High address (stack top)
┌──────────────────────┐
│ null terminator      │  end of envp strings
│ envp strings         │  actual string bytes
│ null terminator      │  end of argv strings
│ argv strings         │  actual string bytes
├──────────────────────┤
│ 0 (null)             │  end of envp[]
│ envp[n-1] ptr        │
│ ...                  │
│ envp[0] ptr          │
│ 0 (null)             │  end of argv[]
│ argv[argc-1] ptr     │
│ ...                  │
│ argv[0] ptr          │
├──────────────────────┤
│ argc                 │  ← RSP points here after setup_stack
└──────────────────────┘
Low address
```

Your `setup_stack()` from Phase 3.8 already handles this. Pass argv/envp from the execve call.

### Files to Create

```
kernel/src/process/exec.rs    # sys_execve
kernel/src/process/wait.rs    # sys_waitpid + zombie management
```

### Files to Modify

```
kernel/src/process/mod.rs     # pub mod exec; pub mod wait;
kernel/src/syscall/mod.rs     # route Exec=31, Waitpid=32
kernel/src/process/process.rs # add ProcessState::Zombie variant
```

### ProcessState additions

```rust
// In ProcessState enum:
pub enum ProcessState {
    Running,
    Ready,
    Blocked,       // waiting on I/O or signal
    Zombie(i32),   // exited, waiting for parent waitpid; i32 = exit code
}
```

### Implementation — `kernel/src/process/exec.rs`

```rust
//! execve() system call.
//!
//! # Agent implementation guide
//!
//! This function is the most structurally complex in Phase 4 because it
//! must be a "point of no return": once we start destroying the old address
//! space, we cannot fail gracefully. To handle this:
//!
//! 1. Do ALL validation and loading FIRST (before touching current addr space).
//! 2. Only after the new address space is fully built, destroy the old one.
//! 3. Install the new address space atomically.
//! 4. Jump to entry — never return.
//!
//! If step 1 fails, return Err and leave the process untouched.
//! If step 2+ fails, kernel panic — we are past the point of no return.

use crate::process::process::{Process, ProcessError};
use sunlight_elf::{load_static_elf, setup_stack, ElfError};
use crate::mm::AddressSpace;

#[derive(Debug)]
pub enum ExecError {
    /// File not found at path.
    NotFound,
    /// Not a valid ELF file.
    InvalidElf(ElfError),
    /// Process does not have execute permission on the file.
    PermissionDenied,
    /// Only static ELF supported in Phase 4.0-4.3 (dynamic: Session 4.4).
    DynamicElfNotYetSupported,
    /// Path too long.
    PathTooLong,
}

/// sys_execve: replace the current process image with a new ELF binary.
///
/// # Arguments
/// - `process`:  the calling process (will be mutated in-place)
/// - `path`:     VFS path to the executable
/// - `argv`:     argument vector (argv[0] = program name)
/// - `envp`:     environment strings ("KEY=VALUE")
///
/// # Return
/// - `Err(ExecError)`: on failure — process is UNCHANGED
/// - This function diverges on success (jumps to new entry point)
///
/// # AGENT NOTE
/// The `-> !` return type means the function must not return on the
/// success path. Use `crate::arch::jump_to_user(entry, sp)` to transfer
/// control. If your kernel does not have this function yet, create a stub
/// that calls `todo!()` and mark it with `// AGENT-NOTE: implement`.
pub fn sys_execve(
    process: &mut Process,
    path:    &str,
    argv:    &[&str],
    envp:    &[&str],
) -> Result<!, ExecError> {
    // ── Phase 1: Validate and load (do not touch current address space yet) ──

    if path.len() > 4096 {
        return Err(ExecError::PathTooLong);
    }

    // Read the ELF from VFS.
    let elf_bytes = crate::vfs::read_file(path)
        .ok_or(ExecError::NotFound)?;

    // Check execute permission (VFS + Unix permission check from Phase 3.7).
    // Use the process credential for the permission check.
    crate::vfs::check_exec_permission(path, &process.credential)
        .map_err(|_| ExecError::PermissionDenied)?;

    // Detect static vs dynamic ELF.
    // Static: e_type == ET_EXEC, no PT_INTERP segment.
    // Dynamic: e_type == ET_DYN, has PT_INTERP segment → Phase 4.4.
    let has_interp = elf_has_pt_interp(&elf_bytes);
    if has_interp {
        return Err(ExecError::DynamicElfNotYetSupported);
    }

    // Load the ELF into a NEW (not yet installed) address space.
    let mut new_addr_space = AddressSpace::new_empty()
        .map_err(|_| ExecError::InvalidElf(ElfError::OutOfMemory))?;

    let loaded = load_static_elf(&elf_bytes, &mut new_addr_space)
        .map_err(ExecError::InvalidElf)?;

    // Set up the stack in the new address space.
    let stack_pointer = setup_stack(
        &mut new_addr_space,
        loaded.stack_top,
        argv,
        envp,
        &[],  // auxv: empty for static ELF
    ).map_err(|_| ExecError::InvalidElf(ElfError::StackSetupFailed))?;

    // ── Phase 2: Point of no return — destroy old, install new ──

    // Reset signal handlers: SIG_IGN survives exec, others reset to SIG_DFL.
    for (i, handler) in process.signal_handlers.iter_mut().enumerate() {
        if *handler != SignalHandler::Ignore {
            *handler = SignalHandler::Default;
        }
        // Pending signals are cleared.
        process.pending_signals &= !(1u64 << i);
    }

    // Close FD_CLOEXEC file descriptors.
    process.fd_table.close_cloexec_fds();

    // Destroy old address space (all pages freed to PMM).
    // SAFETY: We have fully validated and loaded the new image above.
    // Destruction is safe because the process is not executing user code
    // (we are in kernel mode in the syscall handler).
    unsafe { process.address_space.destroy() };

    // Install new address space.
    process.address_space = new_addr_space;

    // Update process name to the new binary.
    process.name = path.split('/').last().unwrap_or(path).into();

    serial_println!("[EXEC] execve({}) pid={} entry={:#x} sp={:#x}",
        path, process.pid, loaded.entry_point, stack_pointer);

    // ── Phase 3: Jump to new process entry — never returns ──

    // SAFETY: entry_point and stack_pointer were set up by load_static_elf
    // and setup_stack above. The new address space is installed. Jumping
    // to user space is safe here — we are transitioning from kernel to
    // user mode via the normal iretq path.
    unsafe {
        crate::arch::x86_64::jump_to_user(loaded.entry_point, stack_pointer)
    }
}

/// Returns true if the ELF has a PT_INTERP program header.
/// Used to distinguish static from dynamic binaries.
fn elf_has_pt_interp(bytes: &[u8]) -> bool {
    // Parse just the program headers — no full ELF parse needed.
    // PT_INTERP = 3
    const PT_INTERP: u32 = 3;
    // Minimal ELF header size check.
    if bytes.len() < 64 { return false; }
    let e_phoff = u64::from_le_bytes(bytes[32..40].try_into().unwrap_or([0;8]));
    let e_phentsize = u16::from_le_bytes(bytes[54..56].try_into().unwrap_or([0;2]));
    let e_phnum = u16::from_le_bytes(bytes[56..58].try_into().unwrap_or([0;2]));

    for i in 0..e_phnum as usize {
        let off = e_phoff as usize + i * e_phentsize as usize;
        if off + 4 > bytes.len() { break; }
        let p_type = u32::from_le_bytes(bytes[off..off+4].try_into().unwrap_or([0;4]));
        if p_type == PT_INTERP { return true; }
    }
    false
}
```

### Implementation — `kernel/src/process/wait.rs`

```rust
//! waitpid() system call and zombie process reaping.

use crate::process::process::{Process, Pid, ProcessState};
use crate::sched::Scheduler;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitOptions(pub u32);
impl WaitOptions {
    /// Return immediately if no child has changed state.
    pub const WNOHANG: u32    = 1;
    /// Also report children that have stopped (Phase 4.3 signals).
    pub const WUNTRACED: u32  = 2;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitError {
    /// No children exist (ECHILD).
    NoChildren,
    /// WNOHANG set and no child ready.
    WouldBlock,
    /// Specified pid does not exist or is not a child.
    NotAChild,
}

/// sys_waitpid: wait for a child process to change state.
///
/// # Arguments
/// - `caller_pid`: PID of the waiting process (used to verify parenthood)
/// - `target_pid`: -1 = any child; >0 = specific child pid
/// - `options`:    WNOHANG | WUNTRACED
/// - `sched`:      scheduler (to remove zombie from process table)
///
/// # Returns
/// - `Ok((child_pid, exit_status_word))` where exit_status_word is:
///   - `(exit_code & 0xFF) << 8` for normal exit (matches POSIX WIFEXITED)
///   - `signal_number & 0x7F` for signal-killed (matches POSIX WIFSIGNALED)
///
/// # Blocking behavior
/// In Phase 4, "blocking" means the scheduler re-queues the parent and
/// tries again when a child becomes a zombie. Full event-based blocking
/// is Phase 5. For Phase 4: if WNOHANG is not set and no zombie is ready,
/// return `WaitError::WouldBlock` and the syscall dispatch layer will
/// re-run waitpid on the next scheduler tick.
pub fn sys_waitpid(
    caller_pid:  Pid,
    target_pid:  i32,
    options:     u32,
    sched:       &mut Scheduler,
) -> Result<(Pid, i32), WaitError> {
    // Collect candidate zombie children.
    let caller = sched.get_process(caller_pid)
        .ok_or(WaitError::NoChildren)?;

    if caller.children.is_empty() {
        return Err(WaitError::NoChildren);
    }

    // Find a zombie child matching the criteria.
    let zombie_pid = find_zombie_child(caller, target_pid, sched)?;

    match zombie_pid {
        Some(cpid) => {
            // Reap: remove zombie from process table.
            let child = sched.remove_process(cpid)
                .expect("[WAIT] zombie disappeared — process table inconsistency");

            let exit_status = match child.state {
                ProcessState::Zombie(code) => (code & 0xFF) << 8,
                _ => 0, // should not happen
            };

            // Remove from parent's children list.
            let parent = sched.get_process_mut(caller_pid).unwrap();
            parent.children.retain(|&p| p != cpid);

            serial_println!("[WAIT] pid={} reaped child={} status={:#x}",
                caller_pid, cpid, exit_status);
            Ok((cpid, exit_status))
        }
        None => {
            if options & WaitOptions::WNOHANG != 0 {
                Ok((Pid(0), 0)) // WNOHANG: return immediately with pid=0
            } else {
                Err(WaitError::WouldBlock) // caller will retry
            }
        }
    }
}

fn find_zombie_child(
    parent: &Process,
    target: i32,
    sched:  &Scheduler,
) -> Result<Option<Pid>, WaitError> {
    if target == -1 {
        // Any zombie child.
        for &cpid in &parent.children {
            if let Some(child) = sched.get_process(cpid) {
                if matches!(child.state, ProcessState::Zombie(_)) {
                    return Ok(Some(cpid));
                }
            }
        }
        Ok(None) // no zombie yet
    } else if target > 0 {
        let target_pid = Pid(target as u64);
        // Verify it's actually our child.
        if !parent.children.contains(&target_pid) {
            return Err(WaitError::NotAChild);
        }
        if let Some(child) = sched.get_process(target_pid) {
            if matches!(child.state, ProcessState::Zombie(_)) {
                return Ok(Some(target_pid));
            }
        }
        Ok(None) // not a zombie yet
    } else {
        // target == 0: process group — not implemented in Phase 4.
        // AGENT-NOTE: implement process groups in Phase 5.
        Err(WaitError::NoChildren)
    }
}
```

### Gate Test

```bash
./tools/test.sh phase4_3
```

Expected serial output:
```
[EXEC]  execve(/bin/true) pid=2 entry=0x400000 sp=0x7fff0000
[WAIT]  pid=1 reaped child=2 status=0x0000
[EXEC]  execve gate PASSED
[WAIT]  waitpid gate PASSED
```

---

## Session 4.4 — Signal Infrastructure + Ctrl+C

### Purpose

Implement signal delivery: `sigaction()`, `kill()`, `SIGKILL`/`SIGTERM`/`SIGINT`/`SIGCHLD`, and Ctrl+C from the keyboard driver. Also implement `mmap()`/`munmap()` (needed by ld-musl in Session 4.5, and by the shell for file-backed maps).

### Entry Conditions

```bash
./tools/test.sh phase4_3   # exec + wait must pass
grep "keyboard\|PS2\|scan" kernel/src/ -r --include="*.rs" -l  # keyboard driver must exist
```

### Agent decision guide for signals

**SIGKILL and SIGSTOP are special:** they cannot be caught, blocked, or ignored. If a process's `sigaction` for SIGKILL is anything other than Default, the kernel ignores the setting. The delivery path bypasses the signal mask.

**Signal delivery happens at the transition from kernel mode to user mode.** Specifically: at the end of every syscall handler and at every interrupt return to user space, call `deliver_pending_signals()`. This function checks if any signals are pending and not blocked, and if so, sets up the signal frame on the user stack.

**Signal frame layout for user handlers:**
```
Before delivering signal, save:
- All general-purpose registers (ucontext)
- Signal number (siginfo_t)
- Signal mask at time of delivery (saved to restore after sigreturn)

Set:
- RSP = (current user RSP) - sizeof(signal_frame), 16-byte aligned
- RIP = user's signal handler address
- When handler returns, it calls sigreturn syscall which restores all of the above
```

### Files to Create

```
kernel/src/process/signal.rs    # Signal types, SigAction, delivery
kernel/src/mm/mmap.rs           # sys_mmap, sys_munmap, sys_mprotect
```

### Files to Modify

```
kernel/src/process/process.rs   # add signal_handlers, pending_signals, signal_mask
kernel/src/drivers/keyboard.rs  # send SIGINT on Ctrl+C
kernel/src/syscall/mod.rs       # route signal syscalls + mmap syscalls
kernel/src/arch/x86_64/interrupts.rs  # call deliver_pending_signals on exit
```

### Implementation — `kernel/src/process/signal.rs`

```rust
//! Signal infrastructure for SunlightOS.
//!
//! # Signal delivery invariants
//!
//! 1. SIGKILL and SIGSTOP are ALWAYS delivered regardless of mask or handler.
//! 2. A signal is "pending" if it has been sent but not yet delivered.
//! 3. A signal is "blocked" if it is in the process's signal_mask.
//! 4. Blocked signals remain pending until unblocked.
//! 5. Signal handlers run on the user stack (with a signal frame).
//! 6. After the handler returns, sigreturn restores saved state.

/// Signal numbers (matches Linux signal numbers for Helios compat).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Signal {
    SIGHUP   = 1,
    SIGINT   = 2,
    SIGQUIT  = 3,
    SIGILL   = 4,
    SIGTRAP  = 5,
    SIGABRT  = 6,
    SIGBUS   = 7,
    SIGFPE   = 8,
    SIGKILL  = 9,   // cannot be caught, blocked, or ignored
    SIGUSR1  = 10,
    SIGSEGV  = 11,
    SIGUSR2  = 12,
    SIGPIPE  = 13,
    SIGALRM  = 14,
    SIGTERM  = 15,
    SIGCHLD  = 17,
    SIGCONT  = 18,
    SIGSTOP  = 19,  // cannot be caught, blocked, or ignored
    SIGTSTP  = 20,
    SIGTTIN  = 21,
    SIGTTOU  = 22,
    SIGWINCH = 28,
}

impl Signal {
    pub fn from_u8(n: u8) -> Option<Self> {
        match n {
             1 => Some(Self::SIGHUP),
             2 => Some(Self::SIGINT),
             3 => Some(Self::SIGQUIT),
             4 => Some(Self::SIGILL),
             5 => Some(Self::SIGTRAP),
             6 => Some(Self::SIGABRT),
             7 => Some(Self::SIGBUS),
             8 => Some(Self::SIGFPE),
             9 => Some(Self::SIGKILL),
            10 => Some(Self::SIGUSR1),
            11 => Some(Self::SIGSEGV),
            12 => Some(Self::SIGUSR2),
            13 => Some(Self::SIGPIPE),
            14 => Some(Self::SIGALRM),
            15 => Some(Self::SIGTERM),
            17 => Some(Self::SIGCHLD),
            18 => Some(Self::SIGCONT),
            19 => Some(Self::SIGSTOP),
            20 => Some(Self::SIGTSTP),
            21 => Some(Self::SIGTTIN),
            22 => Some(Self::SIGTTOU),
            28 => Some(Self::SIGWINCH),
            _  => None,
        }
    }

    /// Returns true for signals that cannot be caught, blocked, or ignored.
    pub fn is_uncatchable(self) -> bool {
        matches!(self, Self::SIGKILL | Self::SIGSTOP)
    }

    /// Default action when no handler is installed.
    pub fn default_action(self) -> DefaultAction {
        match self {
            Self::SIGCHLD | Self::SIGWINCH => DefaultAction::Ignore,
            Self::SIGSTOP | Self::SIGTSTP |
            Self::SIGTTIN | Self::SIGTTOU   => DefaultAction::Stop,
            Self::SIGCONT                   => DefaultAction::Continue,
            _                               => DefaultAction::Terminate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultAction {
    Terminate,  // kill the process
    Ignore,     // do nothing
    Stop,       // suspend the process
    Continue,   // resume a stopped process
    CoreDump,   // Phase 5: generate core
}

/// A signal handler entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalHandler {
    /// SIG_DFL: use the signal's default action.
    Default,
    /// SIG_IGN: ignore this signal.
    Ignore,
    /// User-defined handler at this virtual address.
    UserHandler {
        /// Address of the handler function in user space.
        handler_addr: u64,
        /// Signals to additionally block while this handler runs.
        sa_mask:      u64,
        /// SA_RESTART, SA_SIGINFO flags.
        sa_flags:     u32,
    },
}

/// Maximum signal number (we support signals 1..=31).
pub const NSIG: usize = 32;

/// Per-process signal table.
pub type SignalTable = [SignalHandler; NSIG];

/// Create a new signal table with all handlers set to Default.
pub fn new_signal_table() -> SignalTable {
    [SignalHandler::Default; NSIG]
}

// ─── sys_sigaction ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct SigAction {
    pub handler: SignalHandler,
    pub mask:    u64,
    pub flags:   u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigactionError {
    InvalidSignal,
    CannotOverrideUncatchable,
}

/// sys_sigaction: set or query the handler for a signal.
///
/// Returns the old SigAction for the signal (so caller can restore it).
pub fn sys_sigaction(
    process:    &mut crate::process::process::Process,
    sig:        Signal,
    new_action: Option<SigAction>,
) -> Result<SigAction, SigactionError> {
    if sig.is_uncatchable() && new_action.is_some() {
        return Err(SigactionError::CannotOverrideUncatchable);
    }

    let idx = sig as usize;
    let old = process.signal_handlers[idx];
    let old_action = SigAction {
        handler: old,
        mask:    0, // simplified: we don't store sa_mask per-handler yet
        flags:   0,
    };

    if let Some(action) = new_action {
        process.signal_handlers[idx] = action.handler;
    }

    Ok(old_action)
}

// ─── sys_kill ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillError {
    /// Target process does not exist.
    NoSuchProcess,
    /// Caller does not have permission to send this signal.
    PermissionDenied,
    /// Invalid signal number.
    InvalidSignal,
}

/// sys_kill: send a signal to a process.
///
/// # Permission rules
/// - A process can always send a signal to itself.
/// - Root (uid=0) can send to any process.
/// - Otherwise: sender effective uid must match target's real or saved uid.
pub fn sys_kill(
    target_pid:  i32,
    sig:         Signal,
    sender_cred: &crate::process::credential::Credential,
    sched:       &mut crate::sched::Scheduler,
) -> Result<(), KillError> {
    let pids_to_signal: alloc::vec::Vec<_> = if target_pid == -1 {
        // Broadcast: all processes (root only).
        if sender_cred.euid != 0 {
            return Err(KillError::PermissionDenied);
        }
        sched.all_pids().collect()
    } else if target_pid > 0 {
        alloc::vec![crate::process::process::Pid(target_pid as u64)]
    } else {
        // target_pid == 0 or < -1: process group — Phase 5.
        return Ok(());
    };

    for pid in pids_to_signal {
        let target = sched.get_process_mut(pid)
            .ok_or(KillError::NoSuchProcess)?;

        // Permission check.
        if sender_cred.euid != 0
            && sender_cred.euid != target.credential.uid
            && sender_cred.euid != target.credential.saved_uid
        {
            return Err(KillError::PermissionDenied);
        }

        send_signal_to_process(target, sig);
    }

    Ok(())
}

/// Queue a signal to a process.
///
/// SIGKILL/SIGSTOP: always set, cannot be cleared by the process.
/// Other signals: set the bit in pending_signals (bitmask, signal N = bit N).
pub fn send_signal_to_process(
    target: &mut crate::process::process::Process,
    sig:    Signal,
) {
    let bit = 1u64 << (sig as u8);
    target.pending_signals |= bit;

    // Wake the process if it is blocked waiting for signals (Phase 5 detail).
    // For Phase 4: just mark pending; delivery happens at next kernel exit.
    if sig == Signal::SIGKILL {
        // SIGKILL: transition to zombie immediately (simplification).
        // Real kernels deliver SIGKILL at the next kernel exit, but for
        // Phase 4 correctness we can terminate immediately here.
        target.state = crate::process::process::ProcessState::Zombie(
            -(Signal::SIGKILL as i32)
        );
    }
}

// ─── Signal delivery at kernel exit ────────────────────────────────────────

/// Called before every return from kernel mode to user mode.
///
/// Checks for pending, unblocked signals and delivers one.
/// Multiple signals may be pending; we deliver them one at a time
/// (on the next kernel exit for each subsequent signal).
///
/// # Safety
/// Must be called with interrupts disabled (we are modifying user stack).
pub fn deliver_pending_signals(
    process: &mut crate::process::process::Process,
    saved_regs: &mut crate::arch::x86_64::SavedRegs,
) {
    for sig_num in 1u8..NSIG as u8 {
        let bit = 1u64 << sig_num;

        // Skip if not pending.
        if process.pending_signals & bit == 0 { continue; }

        // Skip if blocked (except SIGKILL/SIGSTOP).
        let sig = match Signal::from_u8(sig_num) {
            Some(s) => s,
            None    => continue,
        };
        if !sig.is_uncatchable() && (process.signal_mask & bit != 0) {
            continue; // blocked
        }

        // Clear the pending bit.
        process.pending_signals &= !bit;

        match process.signal_handlers[sig_num as usize] {
            SignalHandler::Ignore => {
                // Do nothing — signal consumed.
            }
            SignalHandler::Default => {
                apply_default_action(process, sig, saved_regs);
            }
            SignalHandler::UserHandler { handler_addr, sa_mask, sa_flags: _ } => {
                // Set up signal frame on user stack and redirect to handler.
                // SAFETY: saved_regs contains valid user register state.
                // We are modifying the user stack (rsp) to insert the frame.
                unsafe {
                    setup_signal_frame(
                        process,
                        saved_regs,
                        sig,
                        handler_addr,
                        sa_mask,
                    );
                }
                // Only deliver one signal per kernel exit.
                // The handler will call sigreturn, which will re-check
                // pending signals on the next kernel exit.
                return;
            }
        }
    }
}

fn apply_default_action(
    process:    &mut crate::process::process::Process,
    sig:        Signal,
    _saved_regs: &mut crate::arch::x86_64::SavedRegs,
) {
    match sig.default_action() {
        DefaultAction::Terminate | DefaultAction::CoreDump => {
            // Terminate: set process to zombie with signal-killed exit status.
            // Signal-killed exit status: low 7 bits = signal number.
            let status = (sig as i32) & 0x7F;
            process.state = crate::process::process::ProcessState::Zombie(status);
            serial_println!("[SIG]  pid={} killed by {:?}", process.pid, sig);
        }
        DefaultAction::Ignore => { /* nothing */ }
        DefaultAction::Stop => {
            process.state = crate::process::process::ProcessState::Blocked;
            serial_println!("[SIG]  pid={} stopped by {:?}", process.pid, sig);
        }
        DefaultAction::Continue => {
            if process.state == crate::process::process::ProcessState::Blocked {
                process.state = crate::process::process::ProcessState::Ready;
            }
        }
    }
}

/// Set up a signal frame on the user stack so that when the handler returns,
/// it calls the sigreturn syscall to restore state.
///
/// # Signal frame layout on user stack (x86_64)
/// ```text
///  (old RSP) → [retaddr: sigreturn trampoline]  ← pushed first (high)
///              [ucontext: all saved registers]
///              [siginfo: signal number, etc.]    ← RSP after setup (low)
/// ```
///
/// We simplify by using a fixed trampoline address in a read-only kernel page
/// that contains the `syscall` instruction for sigreturn.
///
/// # Safety
/// `saved_regs.rsp` must be a valid user-mode stack pointer with at least
/// `size_of::<SignalFrame>()` bytes of space below it.
unsafe fn setup_signal_frame(
    process:      &mut crate::process::process::Process,
    saved_regs:   &mut crate::arch::x86_64::SavedRegs,
    sig:          Signal,
    handler_addr: u64,
    sa_mask:      u64,
) {
    const SIGNAL_FRAME_SIZE: u64 = 256; // conservative estimate
    // Align down to 16 bytes (ABI requirement).
    let new_rsp = (saved_regs.rsp - SIGNAL_FRAME_SIZE) & !0xF;

    // Write signal frame to user stack.
    // In a real kernel, this is done via copy_to_user().
    // For Phase 4, we use the process's address space directly.
    let frame_ptr = new_rsp as *mut SignalFrame;
    // SAFETY: new_rsp is 16-byte aligned, within user stack bounds (we trust
    // the ELF loader placed the stack correctly). If this faults, the
    // double-fault handler will catch it.
    frame_ptr.write(SignalFrame {
        saved_rip:    saved_regs.rip,
        saved_rsp:    saved_regs.rsp,
        saved_rflags: saved_regs.rflags,
        saved_rax:    saved_regs.rax,
        saved_rbx:    saved_regs.rbx,
        saved_rcx:    saved_regs.rcx,
        saved_rdx:    saved_regs.rdx,
        saved_rsi:    saved_regs.rsi,
        saved_rdi:    saved_regs.rdi,
        saved_r8:     saved_regs.r8,
        saved_r9:     saved_regs.r9,
        saved_r10:    saved_regs.r10,
        saved_r11:    saved_regs.r11,
        saved_r12:    saved_regs.r12,
        saved_r13:    saved_regs.r13,
        saved_r14:    saved_regs.r14,
        saved_r15:    saved_regs.r15,
        saved_rbp:    saved_regs.rbp,
        old_sig_mask: process.signal_mask,
        sig_number:   sig as u64,
        _padding:     0,
    });

    // Block signals in sa_mask while handler runs.
    process.signal_mask |= sa_mask;
    // Also block the signal itself while its handler runs (SA_NODEFER not set).
    process.signal_mask |= 1u64 << (sig as u8);

    // Redirect execution to the handler.
    // RDI = signal number (first argument on System V AMD64 ABI).
    saved_regs.rdi = sig as u64;
    saved_regs.rip = handler_addr;
    saved_regs.rsp = new_rsp;
    // Return address: the trampoline calls sigreturn syscall.
    // We write the trampoline address as if it were pushed by a call.
    // SAFETY: same as frame_ptr write above.
    let ret_addr_ptr = new_rsp as *mut u64;
    ret_addr_ptr.write(SIGRETURN_TRAMPOLINE_ADDR);
}

/// Address of a small kernel-provided trampoline page containing:
/// ```asm
///   mov rax, SIGRETURN_SYSCALL_NR   ; SunlightSyscall::Sigreturn = 74
///   syscall
/// ```
/// This page is mapped read-only+execute into every process's address space.
/// AGENT-NOTE: create this trampoline in kernel/src/process/signal_trampoline.rs
const SIGRETURN_TRAMPOLINE_ADDR: u64 = 0xFFFF_F000; // fixed VA in user space

/// The signal frame saved on the user stack.
#[repr(C)]
struct SignalFrame {
    sig_number:   u64,
    old_sig_mask: u64,
    saved_rip:    u64,
    saved_rsp:    u64,
    saved_rflags: u64,
    saved_rax:    u64,
    saved_rbx:    u64,
    saved_rcx:    u64,
    saved_rdx:    u64,
    saved_rsi:    u64,
    saved_rdi:    u64,
    saved_r8:     u64,
    saved_r9:     u64,
    saved_r10:    u64,
    saved_r11:    u64,
    saved_r12:    u64,
    saved_r13:    u64,
    saved_r14:    u64,
    saved_r15:    u64,
    saved_rbp:    u64,
    _padding:     u64,
}

// ─── sys_sigreturn ─────────────────────────────────────────────────────────

/// sys_sigreturn: called by the trampoline after a signal handler returns.
///
/// Restores the saved register state from the signal frame.
///
/// # Safety
/// RSP must point to a valid SignalFrame that was written by setup_signal_frame.
pub unsafe fn sys_sigreturn(
    process:    &mut crate::process::process::Process,
    saved_regs: &mut crate::arch::x86_64::SavedRegs,
) {
    // SAFETY: saved_regs.rsp points to the SignalFrame written by
    // setup_signal_frame. We trust this because sigreturn is only
    // reachable from the trampoline, which is only reachable after
    // setup_signal_frame writes the frame.
    let frame = &*(saved_regs.rsp as *const SignalFrame);

    // Restore signal mask.
    process.signal_mask = frame.old_sig_mask;

    // Restore registers.
    saved_regs.rip    = frame.saved_rip;
    saved_regs.rsp    = frame.saved_rsp;
    saved_regs.rflags = frame.saved_rflags;
    saved_regs.rax    = frame.saved_rax;
    saved_regs.rbx    = frame.saved_rbx;
    saved_regs.rcx    = frame.saved_rcx;
    saved_regs.rdx    = frame.saved_rdx;
    saved_regs.rsi    = frame.saved_rsi;
    saved_regs.rdi    = frame.saved_rdi;
    saved_regs.r8     = frame.saved_r8;
    saved_regs.r9     = frame.saved_r9;
    saved_regs.r10    = frame.saved_r10;
    saved_regs.r11    = frame.saved_r11;
    saved_regs.r12    = frame.saved_r12;
    saved_regs.r13    = frame.saved_r13;
    saved_regs.r14    = frame.saved_r14;
    saved_regs.r15    = frame.saved_r15;
    saved_regs.rbp    = frame.saved_rbp;
}
```

### Keyboard Ctrl+C integration

In `kernel/src/drivers/keyboard.rs`, find where scan codes are converted to characters and add:

```rust
// In the keyboard interrupt handler, after decoding modifier + key:
if modifiers.contains(Modifiers::CTRL) && key == Key::C {
    // Send SIGINT to the foreground process group.
    // In Phase 4: send to pid=1 (init/shell). Phase 5 will add proper
    // foreground process group tracking.
    if let Some(fg_pid) = crate::process::FOREGROUND_PID.load() {
        let target = crate::sched::SCHEDULER.lock().get_process_mut(fg_pid);
        if let Some(proc) = target {
            crate::process::signal::send_signal_to_process(
                proc,
                crate::process::signal::Signal::SIGINT,
            );
            serial_println!("[KBD]  Ctrl+C → SIGINT → pid={}", fg_pid);
        }
    }
    return; // consume the keypress
}
```

### mmap implementation — `kernel/src/mm/mmap.rs`

```rust
//! mmap/munmap/mprotect system calls.
//!
//! Phase 4 supports:
//! - Anonymous mmap (MAP_ANONYMOUS | MAP_PRIVATE): allocates zeroed pages.
//! - File-backed mmap: read from fd, copy into pages (simplified — not true mmap).
//!   Full demand-paging file mmap is Phase 5.
//!
//! mmap is required by ld-musl (Phase 4.5) for:
//! - Loading shared library segments (file-backed)
//! - Allocating TLS and other runtime memory (anonymous)

use x86_64::VirtAddr;
use x86_64::structures::paging::PageTableFlags;

pub const PROT_NONE:  u32 = 0;
pub const PROT_READ:  u32 = 1;
pub const PROT_WRITE: u32 = 2;
pub const PROT_EXEC:  u32 = 4;

pub const MAP_SHARED:    u32 = 0x01;
pub const MAP_PRIVATE:   u32 = 0x02;
pub const MAP_FIXED:     u32 = 0x10;
pub const MAP_ANONYMOUS: u32 = 0x20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmapError {
    InvalidArguments,
    OutOfMemory,
    AddressMappingFailed,
    InvalidFd,
    PermissionDenied,
}

/// sys_mmap: map memory into the process's address space.
///
/// Returns the virtual address of the mapping (page-aligned).
/// On failure, POSIX specifies returning MAP_FAILED (-1 as usize).
pub fn sys_mmap(
    process: &mut crate::process::process::Process,
    addr:    u64,    // hint: 0 = kernel chooses
    length:  u64,
    prot:    u32,
    flags:   u32,
    fd:      i32,
    offset:  u64,
) -> Result<u64, MmapError> {
    if length == 0 {
        return Err(MmapError::InvalidArguments);
    }

    // Round length up to page boundary.
    let num_pages = (length as usize + 4095) / 4096;

    // Determine page flags from prot.
    let mut page_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if prot & PROT_WRITE != 0 { page_flags |= PageTableFlags::WRITABLE; }
    if prot & PROT_EXEC  == 0 { page_flags |= PageTableFlags::NO_EXECUTE; }

    let is_anonymous = flags & MAP_ANONYMOUS != 0;
    let is_fixed     = flags & MAP_FIXED != 0;

    // Choose virtual address.
    let map_addr = if is_fixed && addr != 0 {
        // MAP_FIXED: use exact address (risky but required by ld-musl).
        if addr % 4096 != 0 { return Err(MmapError::InvalidArguments); }
        VirtAddr::new(addr)
    } else {
        // Let the kernel choose an address in the mmap region.
        process.address_space
            .find_free_region(num_pages)
            .ok_or(MmapError::OutOfMemory)?
    };

    if is_anonymous {
        // Anonymous mapping: allocate zeroed frames.
        for i in 0..num_pages {
            let page_va = map_addr + (i * 4096) as u64;
            let frame = crate::mm::PMM
                .lock()
                .alloc_frame()
                .ok_or(MmapError::OutOfMemory)?;

            // Zero the frame.
            // SAFETY: frame.start_address() is a valid physical address from PMM.
            // Identity mapping makes it accessible as a virtual address in kernel.
            unsafe {
                let ptr = frame.start_address().as_u64() as *mut u8;
                core::ptr::write_bytes(ptr, 0, 4096);
            }

            process.address_space
                .map_page(page_va, frame, page_flags)
                .map_err(|_| MmapError::AddressMappingFailed)?;
        }
    } else {
        // File-backed mapping: read from fd.
        if fd < 0 { return Err(MmapError::InvalidFd); }

        // Check that fd has MMAP_R rights (and MMAP_X if PROT_EXEC).
        let required_rights = if prot & PROT_EXEC != 0 {
            sunlight_fs::capability::CapRights::MMAP_R | sunlight_fs::capability::CapRights::MMAP_X
        } else {
            sunlight_fs::capability::CapRights::MMAP_R
        };
        process.fd_table.check_rights(fd, required_rights)
            .map_err(|_| MmapError::PermissionDenied)?;

        let desc = process.fd_table.get(fd).map_err(|_| MmapError::InvalidFd)?;
        let inode = desc.handle.inode;

        for i in 0..num_pages {
            let page_va = map_addr + (i * 4096) as u64;
            let frame = crate::mm::PMM
                .lock()
                .alloc_frame()
                .ok_or(MmapError::OutOfMemory)?;

            // Read page from VFS.
            let file_offset = offset + (i * 4096) as u64;
            let frame_ptr = frame.start_address().as_u64() as *mut u8;
            // SAFETY: frame_ptr is valid (PMM allocation), writable (kernel identity map).
            unsafe { core::ptr::write_bytes(frame_ptr, 0, 4096); }
            let dst = unsafe { core::slice::from_raw_parts_mut(frame_ptr, 4096) };
            let _ = crate::vfs::read_at_inode(inode, file_offset, dst);
            // Partial reads (past EOF) leave the rest zeroed — correct behavior.

            process.address_space
                .map_page(page_va, frame, page_flags)
                .map_err(|_| MmapError::AddressMappingFailed)?;
        }
    }

    Ok(map_addr.as_u64())
}

/// sys_munmap: remove a memory mapping.
pub fn sys_munmap(
    process: &mut crate::process::process::Process,
    addr:    u64,
    length:  u64,
) -> Result<(), MmapError> {
    if addr % 4096 != 0 || length == 0 {
        return Err(MmapError::InvalidArguments);
    }
    let num_pages = (length as usize + 4095) / 4096;
    for i in 0..num_pages {
        let page_va = VirtAddr::new(addr + (i * 4096) as u64);
        // unmap_page returns the physical frame; free it to PMM.
        if let Some(frame) = process.address_space.unmap_page(page_va) {
            crate::mm::PMM.lock().free_frame(frame);
        }
        // If the page was not mapped, silently ignore (POSIX allows this).
    }
    Ok(())
}

/// sys_mprotect: change protection on a memory region.
///
/// Phase 4: simplified implementation that remaps pages with new flags.
pub fn sys_mprotect(
    process: &mut crate::process::process::Process,
    addr:    u64,
    length:  u64,
    prot:    u32,
) -> Result<(), MmapError> {
    if addr % 4096 != 0 { return Err(MmapError::InvalidArguments); }

    let mut new_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if prot & PROT_WRITE != 0 { new_flags |= PageTableFlags::WRITABLE; }
    if prot & PROT_EXEC  == 0 { new_flags |= PageTableFlags::NO_EXECUTE; }

    let num_pages = (length as usize + 4095) / 4096;
    for i in 0..num_pages {
        let page_va = VirtAddr::new(addr + (i * 4096) as u64);
        if let Some(frame) = process.address_space.get_frame(page_va) {
            process.address_space
                .remap_page(page_va, frame, new_flags)
                .map_err(|_| MmapError::AddressMappingFailed)?;
        }
    }
    Ok(())
}
```

### Gate Test

```bash
./tools/test.sh phase4_4
```

Expected serial output:
```
[SIG]   Signal delivery active: SIGINT SIGCHLD SIGTERM SIGKILL
[SIG]   SIGKILL: cannot be caught (verified)
[SIG]   Ctrl+C → SIGINT delivered
[MMAP]  anonymous mmap: OK
[MMAP]  munmap: OK
✓ Phase 4.4 gate PASSED
```

---

## Session 4.5 — Dynamic ELF + ld-musl + Helios Linux Compat

### Purpose

Enable dynamically-linked musl binaries to run by: (1) adding PT_INTERP detection + ld-musl loading to the ELF loader, and (2) implementing the Helios minimal Linux syscall translation layer.

### Entry Conditions

```bash
./tools/test.sh phase4_4   # All previous gates must pass
# Verify ld-musl binary exists (must be pre-fetched, NOT compiled):
ls tools/lib/ld-musl-x86_64.so.1   || echo "FETCH REQUIRED"
```

### Fetching ld-musl — `tools/fetch-musl.sh`

```bash
#!/usr/bin/env bash
# tools/fetch-musl.sh
# Fetch a prebuilt ld-musl-x86_64.so.1 from Alpine Linux's apk.
# Do NOT compile musl from source — that would take hours and is out of scope.

set -euo pipefail

ALPINE_VERSION="3.19"
MUSL_PKG_URL="https://dl-cdn.alpinelinux.org/alpine/v${ALPINE_VERSION}/main/x86_64/musl-1.2.4-r2.apk"
OUT_DIR="tools/lib"
OUT_FILE="${OUT_DIR}/ld-musl-x86_64.so.1"

mkdir -p "${OUT_DIR}"

if [ -f "${OUT_FILE}" ]; then
    echo "ld-musl already present at ${OUT_FILE}"
    exit 0
fi

echo "Fetching musl apk from Alpine ${ALPINE_VERSION}..."
curl -fL "${MUSL_PKG_URL}" -o /tmp/musl.apk

echo "Extracting ld-musl-x86_64.so.1..."
# Alpine apks are tar.gz archives.
mkdir -p /tmp/musl_extract
tar -xzf /tmp/musl.apk -C /tmp/musl_extract

cp "/tmp/musl_extract/lib/ld-musl-x86_64.so.1" "${OUT_FILE}"
chmod 755 "${OUT_FILE}"

echo "Done: ${OUT_FILE} ($(stat -c%s ${OUT_FILE}) bytes)"
```

Add to `build.rs` or to the RamFs builder:

```rust
// In initramfs/src/lib.rs or wherever RamFs entries are defined:

// AGENT-NOTE: This include_bytes! path is relative to the crate root.
// Run tools/fetch-musl.sh before building, or the build will fail.
RamEntry::file(
    "/lib/ld-musl-x86_64.so.1",
    0, 0, 0o755,
    include_bytes!("../../tools/lib/ld-musl-x86_64.so.1"),
),
RamEntry::dir("/lib",     0, 0, 0o755),
RamEntry::dir("/usr/lib", 0, 0, 0o755),
```

### Dynamic ELF loader — `sunlight-elf/src/dynamic.rs`

```rust
//! Dynamic ELF loader for Phase 4.5.
//!
//! Loads a PIE or ET_DYN ELF that has a PT_INTERP segment.
//! The "interpreter" (ld-musl) is loaded separately and runs first,
//! receiving auxv pointers to the main ELF's program headers.
//!
//! # What this does NOT do
//! - Does not implement full RTLD/symbol resolution (ld-musl does that).
//! - Does not support glibc (musl only in Phase 4).
//! - Does not support dlopen() (Phase 6).

use crate::{ElfError, LoadedElf};

/// Result of loading a dynamic ELF.
pub struct DynamicLoadResult {
    /// Entry point of the INTERPRETER (ld-musl), NOT the main binary.
    /// ld-musl's entry point is e_entry in the ld-musl ELF header.
    pub entry_point:   u64,
    /// Stack pointer after setup_stack() has run.
    pub stack_pointer: u64,
    /// Auxiliary vector entries to pass to ld-musl on the stack.
    pub auxv:          alloc::vec::Vec<(u64, u64)>,
}

/// Auxiliary vector entry types (AT_* constants).
/// These match Linux's asm/auxvec.h.
pub mod at {
    pub const NULL:    u64 = 0;
    pub const PHDR:    u64 = 3;   // address of program headers
    pub const PHENT:   u64 = 4;   // size of one program header entry
    pub const PHNUM:   u64 = 5;   // number of program headers
    pub const PAGESZ:  u64 = 6;   // system page size (4096)
    pub const BASE:    u64 = 7;   // interpreter load address
    pub const FLAGS:   u64 = 8;   // 0
    pub const ENTRY:   u64 = 9;   // main ELF entry point
    pub const SECURE:  u64 = 23;  // 0 for non-setuid
    pub const RANDOM:  u64 = 25;  // 16 bytes of random data (address)
    pub const HWCAP:   u64 = 16;  // CPU capability flags
    pub const HWCAP2:  u64 = 26;
}

/// Load a dynamic (PT_INTERP) ELF binary.
///
/// # Arguments
/// - `elf_bytes`:          the main binary's bytes
/// - `interp_bytes`:       ld-musl bytes (read from /lib/ld-musl-x86_64.so.1)
/// - `address_space`:      the new address space to load into
/// - `argv`, `envp`:       forwarded to setup_stack
///
/// # Load layout in virtual memory
/// ```text
/// 0x0000_0000_0040_0000  main ELF load address (ET_EXEC: fixed; ET_DYN: PIE base)
/// 0x0000_7F00_0000_0000  ld-musl load address (always here for predictability)
/// 0x0000_7FFF_F000_0000  stack top
/// ```
///
/// The exact ld-musl base can be any free region; we use 0x7F00_0000_0000.
pub fn load_dynamic_elf<A>(
    elf_bytes:    &[u8],
    interp_bytes: &[u8],
    address_space: &mut A,
    argv:         &[&str],
    envp:         &[&str],
    random_bytes: &[u8; 16],
) -> Result<DynamicLoadResult, ElfError>
where
    A: AddressSpaceTrait,
{
    // ── Load the main ELF ──────────────────────────────────────────────────

    let main_elf = parse_elf_header(elf_bytes)?;

    // For ET_DYN (PIE): load at a fixed base address.
    // For ET_EXEC: load at the address specified in program headers.
    let main_base = if main_elf.e_type == ET_DYN { 0x0040_0000u64 } else { 0 };

    let mut main_phdr_addr = 0u64;
    let mut main_entry     = main_elf.e_entry + main_base;

    for ph in main_elf.program_headers(elf_bytes) {
        if ph.p_type != PT_LOAD { continue; }
        let vaddr  = ph.p_vaddr + main_base;
        let filesz = ph.p_filesz as usize;
        let memsz  = ph.p_memsz  as usize;
        let offset = ph.p_offset as usize;

        let page_flags = ph_flags_to_page_flags(ph.p_flags);

        let num_pages = (memsz + 4095) / 4096;
        for i in 0..num_pages {
            let frame = alloc_and_zero_frame()?;
            let page_va_offset = i * 4096;
            // Copy file content into the page (partial pages get zeros for the rest).
            if offset + page_va_offset < offset + filesz {
                let src_start = offset + page_va_offset;
                let src_end   = (src_start + 4096).min(offset + filesz);
                let copy_len  = src_end - src_start;
                let dst = frame_as_slice_mut(frame);
                dst[..copy_len].copy_from_slice(&elf_bytes[src_start..src_end]);
            }
            address_space.map_page(
                VirtAddr::new(vaddr + page_va_offset as u64),
                frame,
                page_flags,
            )?;
        }

        // Find PHDR location (first PT_LOAD that contains program headers).
        if main_elf.e_phoff >= ph.p_offset as u64
            && main_elf.e_phoff < ph.p_offset as u64 + ph.p_filesz as u64
        {
            main_phdr_addr = main_base + main_elf.e_phoff
                - ph.p_offset as u64 + ph.p_vaddr;
        }
    }

    // ── Load ld-musl ───────────────────────────────────────────────────────

    const INTERP_BASE: u64 = 0x0000_7F00_0000_0000;

    let interp_elf = parse_elf_header(interp_bytes)?;
    let interp_entry = interp_elf.e_entry + INTERP_BASE;

    for ph in interp_elf.program_headers(interp_bytes) {
        if ph.p_type != PT_LOAD { continue; }
        let vaddr     = ph.p_vaddr + INTERP_BASE;
        let filesz    = ph.p_filesz as usize;
        let memsz     = ph.p_memsz  as usize;
        let offset    = ph.p_offset as usize;
        let page_flags = ph_flags_to_page_flags(ph.p_flags);

        let num_pages = (memsz + 4095) / 4096;
        for i in 0..num_pages {
            let frame = alloc_and_zero_frame()?;
            let pv = i * 4096;
            if offset + pv < offset + filesz {
                let src_start = offset + pv;
                let src_end   = (src_start + 4096).min(offset + filesz);
                let copy_len  = src_end - src_start;
                let dst = frame_as_slice_mut(frame);
                dst[..copy_len].copy_from_slice(&interp_bytes[src_start..src_end]);
            }
            address_space.map_page(
                VirtAddr::new(vaddr + pv as u64),
                frame,
                page_flags,
            )?;
        }
    }

    // ── Write random bytes for AT_RANDOM ───────────────────────────────────

    // Allocate one page for random bytes (at a known address).
    const RANDOM_PAGE_VA: u64 = 0x0000_7FFF_F000_0000;
    let random_frame = alloc_and_zero_frame()?;
    let random_dst = frame_as_slice_mut(random_frame);
    random_dst[..16].copy_from_slice(random_bytes);
    address_space.map_page(
        VirtAddr::new(RANDOM_PAGE_VA),
        random_frame,
        PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE,
    )?;

    // ── Build auxiliary vector ─────────────────────────────────────────────

    let auxv = alloc::vec![
        (at::PHDR,   main_phdr_addr),
        (at::PHENT,  main_elf.e_phentsize as u64),
        (at::PHNUM,  main_elf.e_phnum as u64),
        (at::PAGESZ, 4096u64),
        (at::BASE,   INTERP_BASE),
        (at::FLAGS,  0u64),
        (at::ENTRY,  main_entry),
        (at::SECURE, 0u64),
        (at::RANDOM, RANDOM_PAGE_VA),
        (at::HWCAP,  0u64),
        (at::HWCAP2, 0u64),
        (at::NULL,   0u64),
    ];

    // ── Setup stack ────────────────────────────────────────────────────────

    // The stack layout with auxv is:
    //   argc, argv[], NULL, envp[], NULL, auxv pairs, NULL, NULL
    // setup_stack must handle auxv pairs now.
    // AGENT-NOTE: if your setup_stack does not yet accept auxv,
    // extend it here. The auxv pairs go after envp[].
    let stack_pointer = crate::setup_stack_with_auxv(
        address_space,
        crate::STACK_TOP,
        argv,
        envp,
        &auxv,
    )?;

    Ok(DynamicLoadResult {
        entry_point:   interp_entry,  // ld-musl runs first
        stack_pointer,
        auxv,
    })
}

const ET_DYN: u16 = 3;
const PT_LOAD: u32 = 1;
```

### Helios Linux Compat — `services/helios/src/main.rs`

```rust
//! Helios: minimal Linux syscall translation layer.
//!
//! # Scope (Phase 4.5)
//! - Static musl binaries ONLY (no glibc, no shared libs except ld-musl).
//! - Translates the 20 most common Linux syscalls to SunlightOS equivalents.
//! - Unknown syscalls return -ENOSYS (logged to serial).
//!
//! # How Helios is invoked
//! When the kernel loads an ELF with e_ident[EI_OSABI] == ELFOSABI_LINUX (3)
//! OR with e_ident[EI_ABIVERSION] indicating Linux, the syscall dispatch
//! layer routes syscalls through `helios_translate()` instead of the
//! native SunlightOS handler.
//!
//! # Return value convention
//! Linux syscalls return negative errno on failure.
//! SunlightOS syscalls return a Result<T, E>.
//! Helios translates: Ok(v) → v as i64; Err(e) → -(errno(e)) as i64.

#![no_std]
extern crate alloc;

use kernel_api::{Process, Scheduler};

/// Entry point for Helios syscall translation.
///
/// Called by the kernel's syscall dispatch when a Linux-compat process
/// makes a syscall. `linux_nr` is the raw syscall number from `rax`.
pub fn helios_translate(
    linux_nr:    u64,
    args:        [u64; 6],
    process:     &mut Process,
    sched:       &mut Scheduler,
) -> i64 {
    match linux_nr {
        // ── Tier 1: output + exit (must work first for any test) ──────────
        1  => helios_write(args, process),
        60 => helios_exit(args, process, sched),
        231 => helios_exit_group(args, process, sched),

        // ── Tier 2: file I/O ───────────────────────────────────────────────
        0  => helios_read(args, process),
        2  => helios_open(args, process),
        3  => helios_close(args, process),
        5  => helios_fstat(args, process),
        8  => helios_lseek(args, process),
        17 => helios_pread64(args, process),

        // ── Tier 3: process ────────────────────────────────────────────────
        39 => helios_getpid(process),
        40 => helios_getppid(process),
        57 => helios_fork(process, sched),
        59 => helios_execve(args, process),
        61 => helios_wait4(args, process, sched),

        // ── Tier 4: memory ─────────────────────────────────────────────────
        9  => helios_mmap(args, process),
        11 => helios_munmap(args, process),
        12 => helios_brk(args, process),
        25 => helios_mremap(args, process),

        // ── Tier 5: signals ────────────────────────────────────────────────
        13 => helios_rt_sigaction(args, process),
        14 => helios_rt_sigprocmask(args, process),
        62 => helios_kill(args, process, sched),
        234 => helios_tgkill(args, process, sched), // same as kill for single-threaded

        // ── Unknown ────────────────────────────────────────────────────────
        nr => {
            serial_println!("[HELIOS] unimplemented Linux syscall {:#x} ({})", nr, nr);
            -libc_errno::ENOSYS as i64
        }
    }
}

// ── Tier 1 implementations ─────────────────────────────────────────────────

fn helios_write(args: [u64; 6], process: &mut Process) -> i64 {
    let fd    = args[0] as i32;
    let buf   = args[1] as *const u8;
    let count = args[2] as usize;

    // Check fd has WRITE rights.
    use sunlight_fs::capability::CapRights;
    if process.fd_table.check_rights(fd, CapRights::WRITE).is_err() {
        return -9; // EBADF
    }

    // SAFETY: buf and count come from user space. In Phase 4 we trust them
    // (no user/kernel separation enforcement yet — Phase 5 adds that).
    // The pointer is valid for `count` bytes as provided by the musl runtime.
    let data = unsafe { core::slice::from_raw_parts(buf, count) };

    match process.fd_table.get(fd) {
        Ok(desc) => {
            crate::vfs::write_to_handle(&desc.handle, data)
                .map(|n| n as i64)
                .unwrap_or(-5) // EIO
        }
        Err(_) => -9, // EBADF
    }
}

fn helios_exit(args: [u64; 6], process: &mut Process, _sched: &mut Scheduler) -> i64 {
    let code = args[0] as i32;
    process.state = kernel_api::ProcessState::Zombie(code);
    serial_println!("[HELIOS] pid={} exit({})", process.pid, code);
    // This does not return; the scheduler will notice the Zombie state.
    0
}

fn helios_exit_group(args: [u64; 6], process: &mut Process, sched: &mut Scheduler) -> i64 {
    // Phase 4: single-threaded, so exit_group == exit.
    helios_exit(args, process, sched)
}

// ── Tier 4: memory ─────────────────────────────────────────────────────────

fn helios_mmap(args: [u64; 6], process: &mut Process) -> i64 {
    use crate::mm::mmap::{sys_mmap, MmapError};
    match sys_mmap(
        process,
        args[0],         // addr hint
        args[1],         // length
        args[2] as u32,  // prot
        args[3] as u32,  // flags
        args[4] as i32,  // fd
        args[5],         // offset
    ) {
        Ok(addr)                   => addr as i64,
        Err(MmapError::OutOfMemory) => -12, // ENOMEM
        Err(_)                     => -22,  // EINVAL
    }
}

fn helios_brk(args: [u64; 6], process: &mut Process) -> i64 {
    let new_brk = args[0];
    // Simplified brk: extend the heap region.
    // If new_brk == 0, return current brk.
    // If new_brk > current: extend (allocate pages).
    // If new_brk < current: shrink (free pages) — Phase 4: just return current.
    let current_brk = process.heap_end;
    if new_brk == 0 || new_brk < current_brk {
        return current_brk as i64;
    }
    // Allocate pages from current_brk to new_brk.
    let pages_needed = ((new_brk - current_brk + 4095) / 4096) as usize;
    for i in 0..pages_needed {
        let va = VirtAddr::new(current_brk + (i * 4096) as u64);
        if let Ok(frame) = crate::mm::PMM.lock().alloc_frame() {
            // SAFETY: va is in the heap region, not overlapping existing mappings.
            unsafe { core::ptr::write_bytes(
                frame.start_address().as_u64() as *mut u8, 0, 4096
            ); }
            let _ = process.address_space.map_page(
                va, frame,
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE
                    | PageTableFlags::USER_ACCESSIBLE
                    | PageTableFlags::NO_EXECUTE,
            );
        }
    }
    process.heap_end = new_brk;
    new_brk as i64
}

// ── Remaining Tier 2/3/5 implementations ────────────────────────────────────
// AGENT-NOTE: Implement helios_read, helios_open, helios_close, helios_fstat,
// helios_lseek, helios_getpid, helios_getppid, helios_fork, helios_execve,
// helios_wait4, helios_munmap, helios_mremap, helios_rt_sigaction,
// helios_rt_sigprocmask, helios_kill, helios_tgkill following the same
// pattern as helios_write above: translate args → call SunlightOS function
// → convert Result to i64 errno.
```

### Gate Test

```bash
# Build a static musl test binary (host-side, requires musl-gcc):
cat > /tmp/hello_musl.c << 'EOF'
#include <stdio.h>
#include <stdlib.h>
int main(void) {
    puts("hello from musl");
    return 0;
}
EOF
musl-gcc -static -o tools/test-bins/hello_musl /tmp/hello_musl.c

# Embed in test disk image and run:
./tools/test.sh phase4_5
```

Expected serial output:
```
[HELIOS] Linux compat layer started
[HELIOS] loading ld-musl-x86_64.so.1 at 0x7f0000000000
[HELIOS] musl binary: hello from musl
[HELIOS] pid=3 exit(0)
✓ Phase 4.5 gate PASSED
```

---

## Session 4.6 — sunshell v0.2 & Full Integration Gate

### Purpose

Update sunshell to use pipe(), dup2(), and I/O redirection now that the kernel supports them. Run all Phase 4 gates together as a final integration check.

### Entry Conditions

```bash
./tools/test.sh phase4_5   # all previous gates must pass
```

### sunshell v0.2 additions — `userspace/sunshell/src/main.rs`

Add these capabilities to sunshell (still zero external crates, no_std):

```rust
//! sunshell v0.2 — adds: pipes, I/O redirection, quoted strings, exit N.
//!
//! # New parsing rules
//!
//! A command line is tokenized by split_tokens() into:
//!   Token::Word(s)     — bare word or quoted string
//!   Token::Pipe        — `|`
//!   Token::RedirOut    — `>`
//!   Token::RedirAppend — `>>`
//!   Token::RedirIn     — `<`
//!
//! A pipeline is: Cmd (`|` Cmd)*
//! A Cmd is: Word+ (redir)*
//!
//! # Execution model
//!
//! Single command:
//!   fork() → in child: apply redirects → execve()
//!   parent: waitpid()
//!
//! Pipeline `A | B`:
//!   pipe() → (r, w)
//!   fork() → child1: dup2(w, 1); close(r); execve(A)
//!   fork() → child2: dup2(r, 0); close(w); execve(B)
//!   parent: close(r); close(w); waitpid(child1); waitpid(child2)
//!
//! Multi-stage pipelines `A | B | C`:
//!   Generalize: create N-1 pipes for N commands.
//!   Each intermediate process gets stdin=prev_read, stdout=next_write.

/// Token type for shell command parsing.
#[derive(Debug, Clone, PartialEq)]
enum Token<'a> {
    Word(&'a str),
    Pipe,
    RedirOut,
    RedirAppend,
    RedirIn,
}

/// Split a command line into tokens, respecting single and double quotes.
///
/// Rules:
/// - Single quotes: everything inside is literal (no escapes).
/// - Double quotes: everything inside is literal (no variable expansion in v0.2).
/// - `|`, `>`, `>>`, `<`: operators (only when unquoted).
/// - Whitespace separates tokens (when unquoted).
fn split_tokens(line: &str) -> alloc::vec::Vec<Token<'_>> {
    let mut tokens = alloc::vec::Vec::new();
    let mut chars = line.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        match ch {
            ' ' | '\t' => continue,
            '\'' => {
                // Single-quoted string: consume until closing '.
                let start = i + 1;
                let mut end = start;
                for (j, c) in &mut chars {
                    if c == '\'' { end = j; break; }
                }
                tokens.push(Token::Word(&line[start..end]));
            }
            '"' => {
                // Double-quoted string (no expansion).
                let start = i + 1;
                let mut end = start;
                for (j, c) in &mut chars {
                    if c == '"' { end = j; break; }
                }
                tokens.push(Token::Word(&line[start..end]));
            }
            '|' => tokens.push(Token::Pipe),
            '>' => {
                if chars.peek().map(|&(_, c)| c) == Some('>') {
                    chars.next();
                    tokens.push(Token::RedirAppend);
                } else {
                    tokens.push(Token::RedirOut);
                }
            }
            '<' => tokens.push(Token::RedirIn),
            _ => {
                // Bare word: consume until whitespace or operator.
                let start = i;
                let mut end = line.len();
                while let Some(&(j, c)) = chars.peek() {
                    if c == ' ' || c == '\t' || c == '|'
                        || c == '>' || c == '<' || c == '\''
                        || c == '"'
                    {
                        end = j;
                        break;
                    }
                    chars.next();
                }
                tokens.push(Token::Word(&line[start..end]));
            }
        }
    }

    tokens
}

/// A single command with its arguments and redirections.
struct Command<'a> {
    argv:       alloc::vec::Vec<&'a str>,
    redir_in:   Option<&'a str>,   // < filename
    redir_out:  Option<&'a str>,   // > filename
    redir_app:  bool,               // >> (append) vs > (truncate)
}

/// Execute a pipeline of commands.
/// `commands`: list of Command structs, left to right.
fn execute_pipeline(commands: &[Command<'_>]) {
    if commands.is_empty() { return; }
    if commands.len() == 1 {
        execute_single(&commands[0]);
        return;
    }

    // Multi-command pipeline.
    let n = commands.len();
    let mut pipes: alloc::vec::Vec<(i32, i32)> = alloc::vec::Vec::new();

    // Create N-1 pipes.
    for _ in 0..n-1 {
        let (r, w) = sunlight_syscall::pipe().expect("pipe() failed");
        pipes.push((r, w));
    }

    let mut children: alloc::vec::Vec<i32> = alloc::vec::Vec::new();

    for (i, cmd) in commands.iter().enumerate() {
        match sunlight_syscall::fork().expect("fork() failed") {
            0 => {
                // Child: set up stdin/stdout for pipeline position.
                if i > 0 {
                    // Not first: stdin = read end of previous pipe.
                    sunlight_syscall::dup2(pipes[i-1].0, 0).unwrap();
                }
                if i < n - 1 {
                    // Not last: stdout = write end of current pipe.
                    sunlight_syscall::dup2(pipes[i].1, 1).unwrap();
                }
                // Close all pipe fds in the child (we've dup2'd what we need).
                for &(r, w) in &pipes {
                    sunlight_syscall::close(r).ok();
                    sunlight_syscall::close(w).ok();
                }
                // Apply file redirections (override pipe setup if present).
                apply_redirects(cmd);
                // Exec the command.
                do_exec(cmd.argv[0], &cmd.argv);
                unreachable!("exec failed");
            }
            child_pid => {
                children.push(child_pid);
            }
        }
    }

    // Parent: close all pipe fds.
    for (r, w) in pipes {
        sunlight_syscall::close(r).ok();
        sunlight_syscall::close(w).ok();
    }

    // Wait for all children.
    for pid in children {
        sunlight_syscall::waitpid(pid, 0).ok();
    }
}

fn apply_redirects(cmd: &Command<'_>) {
    if let Some(path) = cmd.redir_in {
        let fd = sunlight_syscall::open(path, 0 /*O_RDONLY*/, 0).unwrap();
        sunlight_syscall::dup2(fd, 0).unwrap();
        sunlight_syscall::close(fd).unwrap();
    }
    if let Some(path) = cmd.redir_out {
        let flags = if cmd.redir_app {
            0o1 | 0o2000  // O_WRONLY | O_APPEND
        } else {
            0o1 | 0o100 | 0o1000  // O_WRONLY | O_CREAT | O_TRUNC
        };
        let fd = sunlight_syscall::open(path, flags, 0o644).unwrap();
        sunlight_syscall::dup2(fd, 1).unwrap();
        sunlight_syscall::close(fd).unwrap();
    }
}

fn do_exec(path: &str, argv: &[&str]) -> ! {
    // Resolve PATH if needed.
    let full_path = resolve_path(path);
    sunlight_syscall::execve(&full_path, argv, &[]).unwrap();
    unreachable!()
}
```

### Final Integration Gate — `tools/test.sh phase4`

```bash
#!/usr/bin/env bash
# tools/test.sh — Phase 4 final integration gate

set -euo pipefail

PASS=0
FAIL=0

run_gate() {
    local name="$1"
    local pattern="$2"
    echo -n "  Testing ${name}... "
    if timeout 30 qemu-system-x86_64 \
        -kernel target/x86_64-sunlight/debug/kernel \
        -initrd target/initramfs.img \
        -nographic -no-reboot \
        -append "test=${name}" \
        2>/dev/null | grep -q "${pattern}"; then
        echo "PASS"
        PASS=$((PASS+1))
    else
        echo "FAIL"
        FAIL=$((FAIL+1))
    fi
}

echo "=== SunlightOS Phase 4 Gate Tests ==="
run_gate "phase4_0_fd"      "FdTable gate PASSED"
run_gate "phase4_1_pipe"    "Pipe gate PASSED"
run_gate "phase4_2_fork"    "FORK.*gate PASSED"
run_gate "phase4_3_exec"    "EXEC.*gate PASSED"
run_gate "phase4_3_wait"    "WAIT.*gate PASSED"
run_gate "phase4_4_signal"  "Signal delivery active"
run_gate "phase4_4_mmap"    "mmap.*OK"
run_gate "phase4_5_helios"  "hello from musl"
run_gate "phase4_6_shell"   "sunshell v0.2 pipe OK"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
if [ "${FAIL}" -eq 0 ]; then
    echo "✓ Phase 4 PASSED"
    exit 0
else
    echo "✗ Phase 4 FAILED (${FAIL} gates)"
    exit 1
fi
```

### Expected final serial output

```
[FORK]  fork() implemented, CoW page fault handler active
[EXEC]  execve() implemented, static + dynamic ELF supported
[WAIT]  waitpid() implemented, zombie reaping works
[CAP]   Capsicum fd rights enforced
[SIG]   Signal delivery active: SIGINT SIGCHLD SIGTERM SIGKILL
[PIPE]  pipe() + dup2() working
[HELIOS] Linux compat layer started
[HELIOS] musl binary: hello from musl
[SHELL] sunshell v0.2 pipe OK
[SunlightOS] Phase 4 OK

✓ Phase 4.0 gate PASSED
✓ Phase 4.1 gate PASSED
✓ Phase 4.2 gate PASSED
✓ Phase 4.3 gate PASSED
✓ Phase 4.4 gate PASSED
✓ Phase 4.5 gate PASSED
✓ Phase 4.6 gate PASSED
```

---

## Cross-Session Constraints (apply to all sessions)

| Constraint | Enforcement |
|---|---|
| `SAFETY:` on every `unsafe` block | Enforced: no unsafe without comment |
| SIGKILL + SIGSTOP never blockable | Enforced: `is_uncatchable()` checked before mask |
| Capsicum rights monotone decrease only | Enforced: `reduce_rights()` checks subset |
| `capability_enter()` irreversible | Enforced: no syscall clears `capability_mode` |
| CoW: parent memory never corrupted by child | Enforced: CoW fault allocates new frame before write |
| execve: resets handlers, closes CLOEXEC | Enforced: both called unconditionally on exec |
| pipe write with no readers → SIGPIPE | Enforced: `PipeError::NoReaders` triggers signal |
| ld-musl: prebuilt only, never compile | Enforced: `fetch-musl.sh` downloads Alpine apk |
| sunshell v0.2: zero external crates | Enforced: `Cargo.toml` has no dependencies |
| Phase 3.x gates must not regress | Enforced: CI runs all previous gates before Phase 4 |
| `AGENT-NOTE:` on every Phase 5 deferral | Convention: marks known incomplete items |

## Session Execution Order

```
Session 4.0 → Session 4.1 → Session 4.2 → Session 4.3 → Session 4.4 → Session 4.5 → Session 4.6
    FdTable       Pipe         fork/CoW     exec+wait     Signals+mmap   Dynamic ELF   Integration
```

Each session must pass its gate test before the next session begins. Do not parallelize sessions — they have strict dependencies.
