# Helios Linux Compatibility Layer

Helios is SunlightOS's Linux ELF binary compatibility layer. It allows statically linked Linux musl binaries to run directly on the SunlightOS kernel without modification by translating Linux x86_64 syscall numbers to SunlightOS native syscalls.

## Architecture Overview

```
Linux musl binary
       │
       ▼ x86_64 syscall instruction
       │
   kernel/src/arch/x86_64/syscall.rs
       │
       │  check process.is_linux_compat
       │
       ▼
   compat-linux/src/lib.rs
       │
       │  translate_syscall(linux_nr)
       │
       ├── positive → native SunlightOS syscall number
       └── negative → special/internal handler code (1000+)
```

### Key Components

| Component | Path | Role |
|-----------|------|------|
| ELF Detection | `kernel/src/process/elf_loader.rs:173` | Checks `EI_OSABI == ELFOSABI_LINUX` |
| `is_linux_compat` flag | `kernel/src/process/mod.rs:64` | Per-process flag set at spawn |
| Syscall translation | `compat-linux/src/lib.rs:361` | `translate_syscall()` maps Linux NR → SunlightOS NR |
| Dispatch integration | `kernel/src/arch/x86_64/syscall.rs:289` | Checks flag, calls translator, routes to handlers |
| Linux brk emulation | `kernel/src/arch/x86_64/syscall.rs:1833` | `sys_brk()` with lazy heap init |
| Linux arch_prctl | `kernel/src/arch/x86_64/syscall.rs:1790` | FS/GS base setup for TLS |
| Capability broker | `compat-linux/src/lib.rs:212` | `MockCapabilityBroker` for POSIX→capability token minting |
| Rule engine | `compat-linux/src/lib.rs:86` | `RuleIndex` / `PathRule` for permission checks |

## ELF Detection

A binary is identified as a Linux ELF when its `e_ident[EI_OSABI]` byte equals `ELFOSABI_LINUX` (3):

```rust
// kernel/src/process/elf_loader.rs:175
pub fn is_linux_elf(elf_bytes: &[u8]) -> bool {
    const EI_OSABI: usize = 0x07;
    elf_bytes.len() >= 8
        && elf_bytes[0..4] == [0x7f, b'E', b'L', b'F']
        && elf_bytes[EI_OSABI] == sunlight_elf::ELFOSABI_LINUX
}
```

## Syscall Translation Table

### Tier 1: Core I/O (musl startup essential)

| Linux NR | Name | SunlightOS NR | Notes |
|----------|------|---------------|-------|
| 0 | `read` | 42 | |
| 1 | `write` | 43 | |
| 2 | `open` | 40 | |
| 3 | `close` | 41 | |
| 20 | `writev` | -14 (1013) | Special ABI shim |
| 24 | `sched_yield` | 21 | |
| 7 | `poll` | -7 (1006) | Bounded readiness stub |
| 16 | `ioctl` | -13 (1012) | ENOTTY stub for tty probing |
| 60 | `exit` | 20 | |
| 231 | `exit_group` | 20 | |

### Tier 2: File Descriptor Operations

| Linux NR | Name | SunlightOS NR | Notes |
|----------|------|---------------|-------|
| 5 | `fstat` | 48 | |
| 8 | `lseek` | 44 | |
| 32 | `dup` | 46 | |

### Tier 3: Process Management

| Linux NR | Name | SunlightOS NR | Notes |
|----------|------|---------------|-------|
| 39 | `getpid` | 33 | |
| 186 | `gettid` | 33 | Single-threaded → pid |
| 57 | `fork` | 30 | |
| 59 | `execve` | 31 | |
| 61 | `wait4` | 32 | |
| 218 | `set_tid_address` | -4 (1002) | No-op stub |
| 273 | `set_robust_list` | -5 (1003) | Accepted no-op |
| 334 | `rseq` | -6 (1004) | ENOSYS → libc disables |

### Tier 4: Memory Management

| Linux NR | Name | SunlightOS NR | Notes |
|----------|------|---------------|-------|
| 9 | `mmap` | -11 (1010) | Flag scrubber shim |
| 11 | `munmap` | 51 | |
| 12 | `brk` | -2 (1000) | Special: `sys_brk()` emulation |
| 10 | `mprotect` | 52 | |
| 158 | `arch_prctl` | -3 (1001) | Special: FS/GS base for TLS |

### Tier 5: Signals

| Linux NR | Name | SunlightOS NR | Notes |
|----------|------|---------------|-------|
| 13 | `rt_sigaction` | -8 (1007) | ABI shim |
| 14 | `rt_sigprocmask` | -9 (1008) | ABI shim |
| 62 | `kill` | 72 | |
| 131 | `sigaltstack` | -12 (1011) | ABI shim |
| 200 | `tkill` | -10 (1009) | ABI shim |
| 4 | `stat` | 48 | Approximated via fstat |
| 6 | `lstat` | 48 | Approximated via fstat |

### Tier 6: Modern Linux FS (used by Rust std)

| Linux NR | Name | SunlightOS NR | Notes |
|----------|------|---------------|-------|
| 72 | `fcntl` | 49 | Same arg layout |
| 257 | `openat` | -15 (40) | Frame-shifted: dirfd dropped, args rearranged |
| 79 | `getcwd` | 64 | |
| 80 | `chdir` | 63 | |
| 318 | `getrandom` | -16 (1014) | Kernel entropy backed |

### Special Handler Internal Codes

| Internal Code | Linux Syscall | Handler |
|---------------|---------------|---------|
| 1000 | `brk` (12) | `sys_brk()` — emulated Linux brk with lazy heap |
| 1001 | `arch_prctl` (158) | FS/GS base MSR setup for TLS |
| 1002 | `set_tid_address` (218) | No-op for single-thread |
| 1003 | `set_robust_list` (273) | No-op accepted for musl startup |
| 1004 | `rseq` (334) | ENOSYS so libc disables rseq |
| 1005 | Any unsupported | ENOSYS |
| 1006 | `poll` (7) | Bounded readiness stub |
| 1007 | `rt_sigaction` (13) | Signal action shim |
| 1008 | `rt_sigprocmask` (14) | Signal mask shim |
| 1009 | `tkill` (200) | Thread kill shim |
| 1010 | `mmap` (9) | Flag scrubber + SunlightOS mmap |
| 1011 | `sigaltstack` (131) | Alternate signal stack shim |
| 1012 | `ioctl` (16) | ENOTTY stub |
| 1013 | `writev` (20) | Scatter/gather write shim |
| 1014 | `getrandom` (318) | Kernel entropy via RDRAND/CPUID |

## Permission & Capability Model

The compat layer includes a mock POSIX capability system for intercepted file opens:

- **`RuleIndex`**: Stores per-UID and per-GID access rules with path prefixes
- **`MockCapabilityBroker`**: Mints `CapabilityToken` values validated during opens
- **`sys_open_intercepted()`**: Validates path against POSIX mode bits + rule table

Initialization via `init_phase3_demo_rules()` sets up demo paths (`/home/user/notes.txt`, `/tmp/public`, etc.).

## Demo Applications

### hello-linux

A minimal "Hello from Linux musl on SunlightOS!" smoke-test binary.

- **Source**: `hello-linux/src/main.rs`
- **Build target**: `x86_64-unknown-linux-musl`
- **Embedded at**: `HELLO_LINUX_ELF_BYTES` in `kernel/src/main.rs:141`
- **Spawned via**: `/bin/hello-linux` or `/usr/bin/hello-linux`
- **Binary**: `hello-linux/hello-linux.elf` (pre-built, committed)

### helios-note

A full terminal file viewer using `crossterm` + `tui-rs` with vi-style navigation.

- **Source**: `helios-note/src/main.rs`
- **Build target**: `x86_64-unknown-linux-musl`
- **Dependencies**: `libc`, `tui` (crossterm backend), `crossterm`
- **Embedded at**: `HELIOS_NOTE_ELF_BYTES` in `kernel/src/main.rs:143`
- **Spawned via**: `/bin/note` or `/usr/bin/note`
- **Features**: Scrollable pager with line numbers, arrow/page/home/end keys, status bar

### Running Demo Apps

```
sunlightos$ /bin/hello-linux
Hello from Linux musl on SunlightOS!
Helios compat layer is working.
```

```
sunlightos$ /bin/note /path/to/file.txt
```

## Build Configuration

- **musl target**: `x86_64-unknown-linux-musl` (requires `rustup target add`)
- **hello-linux**: Compiled offline, `hello-linux.elf` checked into repo
- **helios-note**: Built via `cargo build --target x86_64-unknown-linux-musl` in `helios-note/`

## What Can Be Added

### Missing Syscalls (High Priority for Broader Linux App Support)

| Linux NR | Name | Priority | Reason |
|----------|------|----------|--------|
| 17 | `pread64` | Medium | Used by Rust std for file I/O |
| 18 | `pwrite64` | Medium | Used by Rust std for file I/O |
| 22 | `pipe` | High | Needed for shell pipelines |
| 33 | `access` | Medium | File accessibility checks |
| 35 | `nanosleep` | Medium | Timer/sleep calls in libc |
| 41 | `socket` | High | Network apps |
| 42 | `connect` | High | Network apps |
| 44 | `sendto` | High | Network apps |
| 45 | `recvfrom` | High | Network apps |
| 46 | `sendmsg` | High | Network apps |
| 47 | `recvmsg` | High | Network apps |
| 49 | `bind` | High | Network apps |
| 50 | `listen` | Medium | Server apps |
| 51 | `accept` | High | Server apps |
| 52 | `getsockname` | Low | Diagnostic |
| 54 | `setsockopt` | High | Socket configuration |
| 55 | `getsockopt` | Medium | Socket configuration |
| 56 | `clone` | High | Thread creation |
| 63 | `uname` | Medium | `uname()` in libc |
| 78 | `getdents` | High | Directory listing |
| 82 | `select` | Medium | Alternative to poll |
| 87 | `poll` | Medium | Already stubbed, needs real impl |
| 89 | `readlink` | Medium | Symlink resolution |
| 102 | `getuid` | Low | User ID queries |
| 104 | `getgid` | Low | Group ID queries |
| 105 | `setuid` | Low | User identity |
| 106 | `setgid` | Low | Group identity |
| 110 | `getppid` | Low | Parent PID |
| 201 | `time` | Medium | Current time |
| 202 | `futex` | High | Thread synchronization |
| 228 | `clock_gettime` | Medium | High-resolution time |
| 232 | `epoll_create` | Medium | Event notification |
| 233 | `epoll_ctl` | Medium | Event notification |
| 234 | `epoll_wait` | Medium | Event notification |
| 302 | `prlimit64` | Low | Resource limits |

### Infrastructure Improvements

1. **Network syscall family** (socket/connect/bind/accept/setsockopt): Requires a working network stack or socket abstraction in SunlightOS.

2. **Dynamic linking support**: Currently only static musl binaries work. Adding ELF interpreter (`ld-linux-x86-64.so.2`) loading would unlock dynamically linked binaries.

3. **`/proc` and `/sys` filesystem emulation**: Many Linux programs probe `/proc/self/...` and `/sys/...`. A synthetic filesystem handler would improve compatibility.

4. **Signal delivery**: Real signal handling (not just no-op stubs) for SIGTERM, SIGINT, SIGCHLD.

5. **Threading** (`clone`/`futex`): Full thread support requires a working `clone(2)` implementation with shared address space and futex-based synchronization.

6. **Environmental variables**: Set up proper Linux environment (HOME, PATH, etc.) for compat processes.

7. **`/dev/*` device nodes**: Emulate `/dev/null`, `/dev/zero`, `/dev/random`, `/dev/tty`.

8. **`stat` struct layout**: Ensure the Linux `struct stat` (144-byte x86_64 format) is returned for fstat/stat/lstat syscalls.

9. **File descriptor table**: Proper emulation of Linux FD semantics (FD_CLOEXEC, dup2 behavior, etc.).

10. **errno translation**: Map SunlightOS error codes to Linux errno values for `libc`.

### POSIX Capability Model Enhancements

The current `MockCapabilityBroker` is a static demo. A production version would:

- Support dynamic rule updates from a userspace daemon
- Persist rules across reboots
- Integrate with the VFS layer for real file access control
- Support capability delegation between processes

## File System Layout for Linux Apps

| Path | Maps To | Purpose |
|------|---------|---------|
| `/bin/hello-linux` | `HELLO_LINUX_ELF_BYTES` | Linux compat smoke test |
| `/bin/note` | `HELIOS_NOTE_ELF_BYTES` | Terminal note viewer |

## Integration Points

- **Detection**: `kernel/src/process/elf_loader.rs` — checks `EI_OSABI`
- **Flag**: `Process.is_linux_compat` — set during spawn, inherited on fork
- **Spawn**: `kernel/src/process/spawn.rs:41` — sets flag; `:69-72` — builds Linux auxv
- **Dispatch**: `kernel/src/arch/x86_64/syscall.rs:287-387` — translates then routes
- **brk**: `kernel/src/arch/x86_64/syscall.rs:1833` — Linux brk emulation
- **arch_prctl**: `kernel/src/arch/x86_64/syscall.rs:1790` — FS/GS base setup
- **Auxv**: Linux `AT_PHDR`, `AT_PHENT`, `AT_PHNUM`, `AT_PAGESZ`, `AT_ENTRY`, `AT_UID`, `AT_EUID`, `AT_GID`, `AT_EGID`, `AT_SECURE`, `AT_RANDOM`, `AT_SYSINFO_EHDR`, `AT_EXECFN` are set for musl's `_start` to work
