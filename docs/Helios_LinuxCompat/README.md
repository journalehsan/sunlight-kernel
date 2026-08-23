# Helios Linux compatibility

Helios is the bounded in-kernel Linux personality for static x86_64 Linux
ELF executables. It is a compatibility tier, not a Linux userspace or POSIX
rewrite of SunlightOS.

## Static Linux baseline (this phase)

After the static-runtime expansion, Helios claims the following subset and
nothing beyond it:

- architecture: x86_64
- image: static ELF `ET_EXEC`
- one process / one task
- single-threaded Linux userspace
- no dynamic linker, `PT_INTERP`, glibc, or `ld-linux`
- no `fork`, `vfork`, `clone`, `clone3`, pthreads, or futexes
- no asynchronous Linux signal-handler delivery
- native Sunlight VFS, scheduler, memory, and TTY underneath
- Linux syscall ABI validated and translated at the Helios boundary

Unsupported Linux programs fail closed (`-ENOSYS` or a specific Linux errno).
Do not infer generic Linux binary support from this list.

Architecture:

```text
Linux static ELF
  -> explicit Helios personality
  -> Linux ABI validation
  -> compatibility translation
  -> native Sunlight primitive
  -> Sunlight VFS / scheduler / memory / TTY / kernel
```

Linux-only state stays in `LinuxProcessState`. Native file handles, VFS
objects, and `Process` identity remain Sunlight-shaped.

## Personality selection

Personality is selected before userspace execution in
`kernel/src/process/spawn.rs`. `sunlight-elf::classify_personality` accepts:

- `EI_OSABI == 3` (`ELFOSABI_LINUX`) for backward-compatible Helios images;
- the explicit six-byte marker `HLNX01` in `e_ident[9..15]`, used by the tree's
  stamping/build scripts.

`EI_OSABI == 0` is native Sunlight. Other or malformed inputs are
`Unknown` and fail closed.

## Proven executable gates

- `tools/test.sh helios-proven-tier1` — startup, fd, brk, mmap, nanosleep,
  errno, pointer validation, getrandom.
- `tools/test.sh helios-note-regression` — Helios Note reaches interactive-ready.
- `tools/test.sh helios-static-runtime` — tier-1 probe, filesystem/identity
  runtime probe, unmodified sbase `echo(1)`, and Helios Note.

## Identity, time, and uname

`getuid` / `geteuid` / `getgid` / `getegid` read the process uid/gid.
Sunlight has no saved/effective split, so effective IDs equal real IDs.
`getppid` returns the recorded parent pid.

`uname(2)` returns Helios/Sunlight identity, not a fake Linux distribution:

| Field | Value |
| --- | --- |
| `sysname` | `SunlightOS` |
| `nodename` | `sunlight` |
| `release` | kernel `CARGO_PKG_VERSION` |
| `version` | `Helios static Linux ABI` |
| `machine` | `x86_64` |
| `domainname` | `(none)` |

`clock_gettime` accepts `CLOCK_REALTIME` (0) and `CLOCK_MONOTONIC` (1) only.
`gettimeofday` uses the same realtime source; a non-NULL timezone pointer is
filled with zeros. `nanosleep` remains timer-backed with timespec validation.

## Filesystem and directory entries

Native VFS now allows opening directories. `read`/`write`/`truncate` on a
directory still return `EISDIR`. `mkdir` reports `EEXIST` for an existing
path.

`getdents64` synthesizes Linux `linux_dirent64` records (never a native
`VfsDirEntry`). It emits `.` and `..` first, then VFS children. `d_ino` is a
stable FNV-1a hash of the absolute path — synthetic, not a disk inode.
`d_off` is the next entry index and is stored in the descriptor offset.
Partial records are never written; a buffer smaller than one record returns
`-EINVAL`. End of directory returns 0.

Sunlight VFS has no symbolic links. `readlink` / `readlinkat` return
`-EINVAL` for an existing object and `-ENOENT` when the path is missing.

`access` / `faccessat` use native mode bits via `check_permission`.
`AT_SYMLINK_NOFOLLOW` and `AT_EACCESS` are accepted as no-ops. Other flags
are rejected.

### open(2) flags

| Flag | Status |
| --- | --- |
| `O_RDONLY` / `O_WRONLY` / `O_RDWR` | SUPPORTED |
| `O_CREAT` / `O_EXCL` / `O_TRUNC` | SUPPORTED |
| `O_APPEND` | SUPPORTED |
| `O_CLOEXEC` | SUPPORTED (descriptor flag) |
| `O_NONBLOCK` | PARTIAL (pipes/TTY already non-blocking; regular files ignore it as Linux does) |
| `O_DIRECTORY` | SUPPORTED |
| `O_NOFOLLOW` | PARTIAL no-op (no symlinks) |
| `O_NOCTTY` | PARTIAL no-op (no controlling TTY assignment) |
| other bits | REJECTED (`-EINVAL`) |

Descriptor flags (`FD_CLOEXEC`) remain distinct from file status flags
(`O_APPEND`, `O_NONBLOCK`).

### dirfd

`openat`, `mkdirat`, `newfstatat`, `unlinkat`, `renameat`, `faccessat`, and
`readlinkat` resolve:

- `AT_FDCWD` + relative path against the process cwd
- absolute paths, ignoring dirfd
- a directory fd + relative path using the path captured at open
- bad fd → `-EBADF`
- non-directory fd → `-ENOTDIR`
- `.` / `..` segments → `-EINVAL` (native VFS path policy)

Directory-fd resolution is path-based, not a live inode walk. A directory
renamed after it is opened may fail. This is documented, not hidden.

`pread64` / `pwrite64` use the native positional VFS `read`/`write` and do
not move the shared descriptor offset. Non-seekable handles return `-ESPIPE`.
Linux `pwrite` on `O_APPEND` writes at the end of the file.

### stat

Linux `struct stat` is 144 bytes with compile-time size assertions. Fields
with a native source (`st_mode` type+perm, `st_nlink`, `st_uid`, `st_gid`,
`st_size`) are copied. `st_dev` is the synthetic device id `1`. `st_ino` is
the path hash described above. `st_rdev` is 0. `st_blksize` is 4096.
`st_blocks` is derived from size. atime/mtime/ctime stay 0 because the VFS
does not track timestamps.

## Signals and intentional limits

Single-threaded compatibility state records supported signal dispositions,
the signal mask, and `sigaltstack` metadata. Handler delivery, signal frames,
`rt_sigreturn`, process groups, and thread-directed asynchronous semantics are
not implemented. `set_tid_address` and `set_robust_list` validate pointers but
do not create thread/futex semantics.

Outside this tier: `fork`, `vfork`, `clone`, pthreads, futexes, dynamic ELF,
glibc, `/proc`, sockets/network expansion, namespaces, containers, and GUI
Linux applications.

TTY geometry is read from the process's generation-qualified controlling TTY.
`TIOCGWINSZ` translates that state into the Linux x86_64 ABI; live Linux
`SIGWINCH` handler delivery remains outside this compatibility tier.

## Embedded workloads

- `/bin/helios-probe`: `tools/helios-probes/linux-probe-all.S`
- `/bin/helios-probe-runtime` and `/bin/linux-*` names: runtime semantic probe
- `/bin/linux-echo`: unmodified sbase `echo.c` (MIT) with a tiny syscall libc
- `/bin/hello-linux`: static musl smoke test
- `/bin/note`: Helios Note

Run:

```text
tools/test.sh helios-proven-tier1
tools/test.sh helios-note-regression
tools/test.sh helios-static-runtime
```
