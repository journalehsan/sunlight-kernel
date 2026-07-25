# `sunlight-utils` Phase 1 migration trace

This is the repository trace and migration record for the narrow Tier 0
utility migration. It intentionally does not cover shell, TTY, PTY, terminal,
job-control, signal, pipeline, redirection, or filesystem redesign work.

## Repository and build trace

`sunlight-utils` is a workspace member in the root `Cargo.toml`. Its package is
`sunlight-utils`, and before this phase it produced one static, `no_std`,
busybox-style binary: `sunlight-utils` from `sunlight-utils/src/main.rs`.
The applet is selected from `argv[0]`; an explicit `sunlight-utils <applet>`
form is also accepted. The package depends on `sunlight-libc`,
`sunlight-ipc`, and `memchr`.

The complete pre-edit dispatch set was 29 implemented applets:
`ls`, `cat`, `mkdir`, `echo`, `whoami`, `id`, `kill`, `killall`, `pkill`,
`free`, `freezram`, `nice`, `renice`, `pwd`, `stat`, `file`, `head`, `wc`,
`uname`, `touch`, `rm`, `rmdir`, `cp`, `mv`, `chmod`, `chown`, `date`,
`grep`, and `display-status`. The source also recognizes `find`, `sort`,
`uniq`, `cut`, and `tail` as “not implemented yet”; they are not usable
utilities. No `true` or `false` command or binary exists in this repository.

The kernel build script builds the package for `x86_64-unknown-none` with the
user-space linker script and embeds the resulting ELF files in the kernel.
Before this phase, `/bin/*`, `/usr/bin/*`, and `/sunlight-utils/*` utility
paths selected the multicall ELF. This phase adds a separately embedded
`echo` ELF while leaving the multicall ELF and all non-migrated applets intact.
The existing `tools/test.sh phase6.5.3` image/QEMU gate remains the packaging
and boot-path check for the unchanged utilities.

## New libc entry path

The kernel's native entry ABI passes `argc` in `rdi`, `argv` in `rsi`, and
`envp` in `rdx`; it also constructs the corresponding SysV stack layout.
`sunlight-libc::crt0` provides bounded raw argument collection and, for this
phase, bounded UTF-8 argument collection. Environment delivery is supported by
`sunlight_libc::env::init(envp)` and `getenv*`, but `echo` does not initialize
or consume the environment. `sunlight_libc::write_all` writes through the
libc `write` syscall wrapper, handles short writes, retries `EAGAIN` with
`yield_now`, and propagates other errors. `sunlight_libc::exit` performs the
process-exit syscall. No allocator is linked for this Tier 0 binary; the
optional libc global allocator is feature-gated and unused here.

Before this phase, the multicall binary had its own `_start`, argument
collector, output loop, and raw debug-log syscall. The new standalone `echo`
binary has its own libc-based `_start`, uses the shared libc argument helper,
uses `write_all` for stdout and panic diagnostics, and terminates only through
libc `exit`.

## Complete utility migration matrix

“Direct usage” records the relevant kernel-facing or IPC-facing path observed
in the utility source. “Blocked” here means the command cannot be migrated to
the requested tier without functionality outside this phase; an unimplemented
applet is separately marked “not implemented”.

| Command | Current runtime path | Direct kernel/IPC usage | Required libc APIs | Migration tier | Status |
|---|---|---|---|---|---|
| `ls` | Multicall `_start`; local argv/output | `read_dir` | startup, argv, stdout, `read_dir` | Tier 1 | Not migrated |
| `cat` | Multicall `_start`; local output | `open`, `read`, `close` | startup, argv, stdout, file I/O | Tier 2 | Not migrated |
| `mkdir` | Multicall `_start`; local output | `mkdir` | startup, argv, stdout/stderr, mkdir | Tier 1 | Not migrated |
| `echo` | Standalone `/bin/echo`, `/usr/bin/echo`, and `/sunlight-utils/echo` now use libc `_start`; explicit multicall form remains compatibility code | libc `write`/`exit` only in the standalone binary | startup, argv, stdout, exit | Tier 0 | Migrated and image-embedded |
| `whoami` | Multicall `_start`; local output | UID lookup / passwd IPC helper | startup, stdout, identity lookup | Tier 1 | Not migrated |
| `id` | Multicall `_start`; local output | UID/GID lookup and passwd/group IPC helpers | startup, argv, stdout, identity lookup | Tier 1 | Not migrated |
| `kill` | Multicall `_start`; local output | `kill` syscall | startup, argv, stdout/stderr, process control | Tier 3 | Not migrated |
| `killall` | Multicall `_start`; local output | process-stat enumeration and `kill` | startup, argv, stdout/stderr, process control | Tier 3 | Not migrated |
| `pkill` | Multicall `_start`; local output | process-stat enumeration and `kill` | startup, argv, stdout/stderr, process control | Tier 3 | Not migrated |
| `free` | Multicall `_start`; local output | `sysinfo` | startup, argv, stdout, system metadata | Tier 1 | Not migrated |
| `freezram` | Multicall `_start`; local output | `sysinfo`, zram syscall, swap IPC | startup, argv, stdout/stderr, swap control | Tier 3 | Not migrated |
| `nice` | Multicall `_start`; local output | get/set process priority | startup, argv, stdout/stderr, process control | Tier 3 | Not migrated |
| `renice` | Multicall `_start`; local output | get/set process priority | startup, argv, stdout/stderr, process control | Tier 3 | Not migrated |
| `pwd` | Multicall `_start`; local output | none beyond stdout | startup, stdout, exit | Tier 0 | Not migrated |
| `stat` | Multicall `_start`; local output | `stat` | startup, argv, stdout/stderr, metadata | Tier 1 | Not migrated |
| `file` | Multicall `_start`; local output | `stat` | startup, argv, stdout/stderr, metadata | Tier 1 | Not migrated |
| `head` | Multicall `_start`; local output | `open`, `read`, `close` | startup, argv, stdout/stderr, file I/O | Tier 2 | Not migrated |
| `wc` | Multicall `_start`; local output | `open`, `read`, `close` | startup, argv, stdout/stderr, file I/O | Tier 2 | Not migrated |
| `uname` | Multicall `_start`; local output | `sysinfo` | startup, argv, stdout, system metadata | Tier 1 | Not migrated |
| `touch` | Multicall `_start`; local output | `open`, `close` | startup, argv, stdout/stderr, file I/O | Tier 1 | Not migrated |
| `rm` | Multicall `_start`; local output | `stat`, `read_dir`, `unlink` | startup, argv, stdout/stderr, file operations | Tier 1 | Not migrated |
| `rmdir` | Multicall `_start`; local output | `stat`, `unlink` | startup, argv, stdout/stderr, file operations | Tier 1 | Not migrated |
| `cp` | Multicall `_start`; local output | `open`, `read`, `write`, `close` | startup, argv, stdout/stderr, file I/O | Tier 2 | Not migrated |
| `mv` | Multicall `_start`; local output | `rename`, fallback `unlink` | startup, argv, stdout/stderr, file operations | Tier 1 | Not migrated |
| `chmod` | Multicall `_start`; local output | `chmod` | startup, argv, stdout/stderr, metadata operations | Tier 1 | Not migrated |
| `chown` | Multicall `_start`; local output | `chown` | startup, argv, stdout/stderr, metadata operations | Tier 1 | Not migrated |
| `date` | Multicall `_start`; local output | time syscall and nameserver/time IPC | startup, argv, stdout/stderr, time service | Tier 1 | Not migrated |
| `find` | Multicall dispatch only | none | unavailable | Tier 2 | Not implemented |
| `sort` | Multicall dispatch only | none | unavailable | Tier 2 | Not implemented |
| `uniq` | Multicall dispatch only | none | unavailable | Tier 2 | Not implemented |
| `cut` | Multicall dispatch only | none | unavailable | Tier 2 | Not implemented |
| `tail` | Multicall dispatch only | none | unavailable | Tier 2 | Not implemented |
| `grep` | Multicall `_start`; local output | `open`, `read`, `close` and `memchr` | startup, argv, stdout/stderr, file I/O | Tier 2 | Not migrated |
| `display-status` | Multicall `_start`; local output | display-server nameserver lookup and IPC | startup, stdout/stderr, display IPC | Tier 3 | Not migrated |
| `true` | No package binary or applet | None | N/A | N/A | Absent; not invented |
| `false` | No package binary or applet | None | N/A | N/A | Absent; not invented |

## Phase 1 implementation and evidence

The implementation is deliberately limited to `echo`, the only requested
command that exists. Its argument semantics are unchanged: it writes each
argument literally, separates arguments with one space, appends one newline,
accepts `-n` and backslashes as ordinary text, accepts empty and UTF-8
arguments, ignores stdout write errors, and returns zero. The no-argument
case therefore remains exactly one newline. `true` and `false` were absent,
so no replacement command was introduced.

The focused libc corrections are reusable rather than command-specific:

* `crt0::collect_utf8_args` provides bounded argv string delivery for native
  user programs and tests null termination and invalid UTF-8 handling.
* `write_all` now handles the existing libc `EAGAIN` result as a retry while
  retaining short-write and non-retryable-error behavior.

The standalone ELF is built with the real `x86_64-unknown-none` service
flags. ELF inspection shows a static executable with only the expected write,
yield, and exit syscall sites; it contains no debug-log syscall site, direct
IPC call, or legacy utility-runtime string. A linker map retains
`sunlight_libc::write_all` and `sunlight_utils::echo::run`; argument collection
is inlined into the entry path. The full kernel build embeds this ELF and the
path resolver maps the installed echo paths to it.

Post-fix manual QEMU execution through the produced ISO verified `echo hi`
renders exactly `hi` followed by one newline, while `echo` renders exactly one
newline. The serial trace showed both separate `echo` processes exiting with
code 0; each was reaped with zero owned frames and zero cleared IPC entries.
The earlier multi-argument run exposed the argv[0] defect that this fix
corrects. Empty-string, UTF-8, long
argument-list, failed-output, descriptor-leak, and memory-growth cases were
covered by host-side focused logic tests where possible, but their complete
interactive image execution was not verified. No QEMU process-table residue
or utility temporary files were observed in these runs.

The existing `tools/test.sh phase6.5.3` gate was also attempted. Its kernel
build completed, but its pre-existing automatic keystrokes arrived while the
login screen was locked, so its `ls`/`mkdir` assertions were not reached. That
gate result is not claimed as passing; the direct manual QEMU runs above are
the real-target evidence for the migrated command.
