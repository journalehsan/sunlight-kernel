# Helios Note Audit & Compatibility Specification

## Overview

## Current source correction (August 23, 2026)

The original audit below is historical and its line numbers/statuses are no
longer authoritative. The current implementation has since:

- replaced OSABI-only detection with explicit `sunlight-elf::Personality`
  selection (`HLNX01` marker plus backward-compatible OSABI 3);
- moved Linux-only brk, poll wake, termios, signal-mask/disposition support,
  and altstack metadata into `ProcessPersonality::Linux`;
- implemented timer-backed nanosleep with checked Linux timespec validation;
- made `openat`/`*at` dirfd handling fail closed for unsupported relative
  directory handles;
- rejected unknown getrandom flags;
- added executable QEMU gates:
  `tools/test.sh helios-proven-tier1` and
  `tools/test.sh helios-note-regression`.

The current Note regression gate proves process start, Linux personality,
terminal initialization, and the first interactive-ready poll. Signal handler
delivery and live resize/SIGWINCH remain intentionally unsupported. The
current terminal geometry contract is stable synthetic 80x25.

This audit analyzes the existing Helios Note TUI, SunlightOS Linux ABI compatibility layer (Helios), kernel syscall dispatch, PTY/TTY architecture, and VFS file operations to support transforming Helios Note into a modern Ratatui text editor compiled as a static Linux musl executable (`x86_64-unknown-linux-musl`).

---

## 1. Repository & Subsystem Survey

The detailed survey and syscall trace in sections 1–4 are retained as
historical evidence from the original audit. Where they conflict with the
current-source correction above, the correction and executable QEMU gates are
authoritative.

### 1.1 Current Helios Note Source & Build Target
- **Location**: `helios-note/`
- **Entry point**: `helios-note/src/main.rs`
- **Crate configuration**: `helios-note/Cargo.toml`
- **Build Target**: `x86_64-unknown-linux-musl`
- **Linker Flags**: `-C relocation-model=static -C target-feature=+crt-static -C link-arg=-no-pie` (ensures `e_type` is `ET_EXEC` for SunlightOS `elf_loader`)
- **Personality marker**: `tools/stamp_helios_elf.sh` retains OSABI 3 for
  backward compatibility and writes the explicit `HLNX01` marker.
- **Kernel Embedding**: `static HELIOS_NOTE_ELF_BYTES: &[u8]` in `kernel/src/main.rs:180` and `kernel/src/process/spawn.rs:810` (serves `/bin/note` and `/usr/bin/note`).
- **Dependencies**: `libc = "0.2"`, `ratatui = "0.26"`, `crossterm = "0.27"`.

### 1.2 Helios Syscall Dispatch & Translation
- **ELF Identification**: `sunlight-elf::classify_personality` selects Native,
  Linux, or Unknown before execution. `spawn.rs` stores
  `ProcessPersonality::Linux(LinuxProcessState)`.
- **Syscall Trap**: `kernel/src/arch/x86_64/syscall.rs` intercepts `syscall`.
  If the current `ProcessPersonality` is Linux:
  1. Calls `sunlight_compat_linux::translate_syscall(linux_nr)`.
  2. Positive return value -> maps directly to SunlightOS native syscall number.
  3. Negative return value (-2..-18) -> maps to internal kernel codes (1000..1016) in `syscall.rs` for specialized ABI shims.
  4. Unsupported Linux syscall -> returns `-38` (`-ENOSYS`).

### 1.3 Linux Process Startup & musl Support
- **TLS Setup**: `arch_prctl` (Linux 158) -> internal code 1001 calling `sys_set_fs_base` to configure `FS_BASE` MSR for musl thread-local storage.
- **Heap Management**: `brk` (Linux 12) -> internal code 1000 calling `sys_brk` with lazy page allocation.
- **Startup Stubs**:
  - `set_tid_address` (Linux 218) -> returns PID.
  - `set_robust_list` (Linux 273) -> returns 0.
  - `rseq` (Linux 334) -> returns `-ENOSYS` (-38) causing musl to safely disable restartable sequences.
  - Auxiliary vector (`auxv`): `spawn.rs:69-72` populates `AT_PHDR`, `AT_PHENT`, `AT_PHNUM`, `AT_PAGESZ`, `AT_ENTRY`, `AT_UID`, `AT_EUID`, `AT_GID`, `AT_EGID`, `AT_SECURE`, `AT_RANDOM`, `AT_SYSINFO_EHDR`, `AT_EXECFN`.

### 1.4 Stdio, PTY & Graphical Terminal
- **Standard File Descriptors**: `fd 0` (stdin), `fd 1` (stdout), `fd 2` (stderr).
- **PTY Buffering**: `kernel/src/process/tty_io.rs` maintains per-tab `STDIN` (1KB) and `STDOUT` (8KB) lock-free byte rings.
- **Graphical Terminal**: `services/sunlight-terminal` and `services/tty_server` parse VT100/ANSI sequences via `sunlight_tty::TerminalGrid`.
- **ANSI Support**: Alternate screen (`CSI ?1049h / ?1049l`), cursor position (`CSI H`), clear line (`CSI K`), 16-color palette SGR, cursor visibility (`CSI ?25h / ?25l`), cursor save/restore.

### 1.5 Terminal Control & Raw Mode
- **Terminal Detection & Control**: `sys_linux_ioctl` (internal code 1012) intercepts `TCGETS` (0x5401), `TCSETS` (0x5402), `TCSETSW` (0x5403), `TCSETSF` (0x5404).
- **Raw Mode**: Crossterm toggles `ICANON` in `LinuxProcessState.termios`;
  `sys_linux_ioctl` records the mode and logs transitions.
- **Terminal Dimensions**: `TIOCGWINSZ` (0x5413) returns `ws_row = 25`, `ws_col = 80`. `TIOCSWINSZ` (0x5414) returns 0. `SIGWINCH` resize signal propagation is currently unhandled for Linux tasks.

---

## 2. Empirical Syscall Trace from Pinned Musl Binary

Tracing `target/x86_64-unknown-linux-musl/debug/helios-note` via Python ptrace on Linux yielded the following exact syscall sequence:

```
  Syscall   0 (read           ): Stdio input polling & file reading
  Syscall   1 (write          ): Terminal sequence output & error messages
  Syscall   2 (open) / 257 (openat): File open (AT_FDCWD-relative or absolute)
  Syscall   3 (close          ): File handle closure
  Syscall   5 (fstat) / 262 (newfstatat): File metadata query
  Syscall   7 (poll           ): Crossterm input event readiness check
  Syscall   8 (lseek          ): File position movement
  Syscall   9 (mmap           ): Heap & stack memory allocations
  Syscall  10 (mprotect       ): Memory protection flags
  Syscall  11 (munmap         ): Memory unmapping
  Syscall  12 (brk            ): Dynamic heap allocation
  Syscall  13 (rt_sigaction   ): Signal handler setup (SIGINT, SIGTERM, SIGWINCH)
  Syscall  14 (rt_sigprocmask ): Signal mask modification
  Syscall  16 (ioctl          ): Terminal termios (TCGETS/TCSETSW) & size (TIOCGWINSZ)
  Syscall 131 (sigaltstack    ): Signal stack setup
  Syscall 158 (arch_prctl     ): FS_BASE setup for TLS
  Syscall 218 (set_tid_address): Musl thread ID registration
  Syscall 228 (clock_gettime  ): High-resolution monotonic clock for frame timing
  Syscall 231 (exit_group     ): Process termination
```

---

## 3. Strict ABI Translation & Verification Requirements

Each missing or updated Linux syscall is bound by explicit ABI constraints:

1. **`clock_gettime` (Linux 228)**:
   - Must validate `clockid`: `CLOCK_REALTIME` (0) and `CLOCK_MONOTONIC` (1).
   - Must output Linux `struct timespec` (16 bytes: `tv_sec: i64`, `tv_nsec: i64`).
   - Must return 0 on success, `-EFAULT` (errno 14) on invalid user pointer, or `-EINVAL` (errno 22) on unsupported clock ID.

2. **`poll` (Linux 7)**:
   - Must parse `struct pollfd` array (`fd: i32`, `events: i16`, `revents: i16`).
   - For `timeout == 0`: immediately check input readiness (e.g. `tty_io::has_stdin(tab)` for fd 0), populate `revents` (`POLLIN`), and return count of ready descriptors without sleeping.
   - For `timeout > 0` or `timeout < 0` (infinite): if no descriptors are ready, perform a non-spinning sleep/yield until input arrives or the requested timeout elapses.

3. **`renameat` (Linux 264)**:
   - Must validate `olddirfd` and `newdirfd` as `AT_FDCWD` (-100).
   - Must validate `oldpath` and `newpath` string pointers.
   - Must execute path translation and invoke native VFS rename (`sys_rename`, code 66).

---

## 4. Helios Linux Compatibility Audit Table

| Requirement Domain | Linux ABI / Syscall | Current Support Status | Classification | Detailed Notes |
|---|---|---|---|---|
| **Static Musl Execution** | `arch_prctl`, `brk`, `set_tid_address`, `set_robust_list`, `rseq`, `auxv` | Fully supported for single-threaded musl `_start` | `reusable` | TLS, lazy heap, and auxv are complete. `rseq` returns ENOSYS as expected by musl. |
| **Stdin / Stdout / Stderr** | `read(0)`, `write(1)`, `write(2)`, `writev` | Fully supported via PTY rings | `reusable` | `sys_read` drains `tty_io::read_stdin`, `sys_write` and `sys_linux_writev` populate `tty_io::write_stdout`. |
| **Terminal Detection (isatty)** | `ioctl(fd, TCGETS)` | Supported for fds 0, 1, 2 | `reusable` | `sys_linux_ioctl` returns `LinuxTermios` struct for stdio fds and `ENOTTY` (errno 25) for other fds. |
| **Raw Mode Enable/Disable** | `ioctl(fd, TCSETS/TCSETSW/TCSETSF)` | Supported | `reusable` | Correctly toggles `ICANON` and updates `process.linux_termios`. Exiting app restores original flags. |
| **Terminal Dimensions** | `ioctl(fd, TIOCGWINSZ)` | Fixed 80x25 geometry | `partially supported` | Returns `ws_row=25, ws_col=80`. Live resize propagation (`SIGWINCH`) is missing. |
| **Input Readiness / Polling** | `poll(7)`, `read(0)` when empty | Bounded readiness + scheduler-backed timeout | `supported for tier 1` | `sys_linux_poll` inspects TTY/pipe readiness, writes `revents`, and blocks through the Linux timer bookkeeping when no descriptor is ready. |
| **Timing & Clock** | `clock_gettime(228)`, `nanosleep(35)` | Supported bounded shims | `supported for tier 1` | Timespec pointers and ranges are validated; nanosleep uses a scheduler timer wait. Signal interruption remains outside this tier. |
| **File Open & Read** | `open(2)`, `openat(257)`, `read(0)` | Fully supported via `KERNEL_VFS` | `reusable` | Relative and absolute paths work. `openat` frame-shift handles `dirfd` safely. |
| **File Create / Truncate / Write** | `open` with `O_CREAT`/`O_TRUNC`, `write(1)` | Fully supported | `reusable` | Creating missing files and overwriting existing files via `sys_open` + `sys_write` works cleanly. |
| **Seek & Stat** | `lseek(8)`, `fstat(5)`, `stat(4)`, `lstat(6)` | Fully supported | `reusable` | `SEEK_SET/CUR/END` work; `fstat` populates Linux stat layout. |
| **Rename / Atomic Save** | `rename(82)`, `renameat(264)` | Native `sys_rename(66)` plus bounded `AT_FDCWD`/absolute-path shim | `supported for tier 1` | Unsupported relative directory-fd resolution returns `-ENOSYS` instead of silently using cwd. |
| **Process Exit & Cleanup** | `exit(60)`, `exit_group(231)` | Fully supported | `reusable` | `process_exit` reclaims process slot, closes file descriptors, and returns exit code to parent. |
