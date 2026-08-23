# Helios Linux compatibility

Helios is the bounded in-kernel Linux personality for static x86_64 Linux
ELF executables. It is a compatibility tier, not a Linux userspace or POSIX
rewrite of SunlightOS.

The current executable evidence is `PROVEN TIER 1`: the QEMU gate
`tools/test.sh helios-proven-tier1` launches `/bin/helios-probe` and verifies
startup metadata, file descriptors, brk, mmap, nanosleep, errno, pointer
validation, and getrandom. `tools/test.sh helios-note-regression` separately
proves that Helios Note reaches its terminal interactive-ready state.

## Personality selection

Personality is selected before userspace execution in
`kernel/src/process/spawn.rs`. `sunlight-elf::classify_personality` accepts:

- `EI_OSABI == 3` (`ELFOSABI_LINUX`) for backward-compatible Helios images;
- the explicit six-byte marker `HLNX01` in `e_ident[9..15]`, used by the tree's
  stamping/build scripts.

`EI_OSABI == 0` is native Sunlight. Other or malformed inputs are
`Unknown` and fail closed; they are never tried under the native ABI after
Linux execution begins. Native images therefore cannot enter Linux syscall
translation through a heuristic.

## Architecture

```text
ELF bytes
  -> explicit Personality selection
  -> ProcessPersonality::Linux(LinuxProcessState)
  -> validated Linux ABI structures
  -> bounded syscall shim/translation
  -> native VFS, scheduler, TTY, and memory primitives
```

Linux-only state is held in `LinuxProcessState`: brk metadata, termios,
poll/timer wake bookkeeping, alternate-stack metadata, and the bounded
Helios Note readiness marker. Native processes use `ProcessPersonality::Native`.

## Proven static subset

The probe currently covers:

`read`, `write`, `close`, `dup`, `fcntl(F_GETFD)`, `brk`, `mmap`,
`munmap`, `nanosleep`, invalid syscall (`-ENOSYS`), invalid pointer
(`-EFAULT`), and `getrandom(flags=0)`.

The static startup surface also includes validated `arch_prctl`,
`set_tid_address`, `set_robust_list`, `rseq` (intentional `-ENOSYS`),
`clock_gettime`, `poll`, `ioctl`/termios, Linux stat translation, and the
bounded `openat`/`newfstatat`/`unlinkat`/`renameat` family.

`AT_FDCWD` and absolute paths are supported. Relative paths with an
unsupported directory fd return `-ENOSYS`; the fd is never silently ignored.
Unknown `getrandom` flag bits return `-EINVAL`.

`brk(0)` reports the process break. Growth maps owned pages, shrink removes
only the owned brk region, and failed growth returns the previous break.
Anonymous Linux mmap allocations use a cursor separated from the brk arena.

## Signals and intentional limits

Single-threaded compatibility state records supported signal dispositions,
the signal mask, and `sigaltstack` metadata. Handler delivery, signal frames,
`rt_sigreturn`, process groups, and thread-directed asynchronous semantics are
not implemented and are not claimed. Unsupported signal operations fail with
Linux errors. `set_tid_address` and `set_robust_list` validate pointers but do
not create thread/futex semantics.

The following remain outside this tier: `fork`, `vfork`, `clone`, pthreads,
futexes, dynamic ELF interpreters/PT_INTERP, glibc, `/proc`, sockets and
network expansion, namespaces, containers, and GUI Linux applications.
Unsupported calls return `-ENOSYS`.

TTY geometry is currently a stable synthetic 80x25 contract because no live
window-size source is exposed by the native TTY path. Termios raw/cooked state
is preserved per Linux process. Stat fields without native inode/timestamp
sources remain documented synthetic values rather than being presented as
authoritative filesystem identity.

## Embedded workloads and gates

- `/bin/helios-probe`: source `tools/helios-probes/linux-probe-all.S`,
  built by `tools/build_helios_probes.sh`.
- `/bin/hello-linux`: static musl smoke test.
- `/bin/note`: static musl Helios Note (`helios-note/`).

The build scripts stamp the explicit marker and retain OSABI 3 for existing
images. Do not commit generated `target/` outputs or opaque binaries other
than the existing checked-in hello-linux fixture.

Run:

```text
tools/test.sh helios-proven-tier1
tools/test.sh helios-note-regression
```

These gates require actual process execution and PASS markers; loading an ELF
or surviving a boot is not sufficient evidence.
