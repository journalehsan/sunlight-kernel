# Yazi v26.9.1 Compatibility: Phase 1 Audit

Status: September 29 kernel-stack fix removes oversized mapping transaction
buffers. The extended QEMU run survives 30 parent/child navigation cycles;
its later `/home/user` selection and automated clean-exit check remain
unverified. See the dated regression record below for verification limits.

## Intended execution path

```text
Sunlight Terminal
        │
        ▼
   Helios process
        │
        ▼
 Linux ELF / musl Yazi
        │
        ├── TTY / ioctl
        ├── filesystem / VFS
        ├── mmap / allocator
        ├── clocks
        ├── polling
        ├── signals
        └── threads / futex if required
```

## Existing path and audited contracts

- Linux ELF classification is handled by `sunlight-elf`; `spawn.rs` selects a
  Linux process personality, loads the ELF segments, constructs the initial
  stack and auxiliary vector, and sets up the process address space. Helios
  Note demonstrates this with a stamped static `x86_64-unknown-linux-musl`
  `ET_EXEC` executable. The shared parser currently rejects all ELF types
  except `ET_EXEC`, before personality dispatch or syscall entry. The existing
  ELF-loader/static-runtime gates are `helios-proven-tier1` and
  `helios-static-runtime`.
- The x86-64 syscall entry in `kernel/src/arch/x86_64/syscall.rs` translates
  Linux syscall numbers through `compat-linux/src/abi.rs`. Linux-specific
  shims use internal syscall dispatch numbers; unsupported Linux calls should
  return `-ENOSYS`. The dispatcher currently logs unknown calls, but there is
  no bounded per-process syscall trace switch audited yet.
- Linux errors are encoded as negative errno results for musl. VFS failures
  pass through `linux_from_fs`; the error translation is in the syscall layer.
  The process owns an fd table containing VFS, TTY, pipe, and epoll handles.
  Duplicates refer to the underlying handle; fd-table teardown participates in
  process cleanup.
- Linux file calls route through the process fd table and `KERNEL_VFS`. The
  implementation includes open/openat, read/write, seek, stat/fstat,
  newfstatat, getdents64, readlink, access, mkdir, and several *at shims.
  `getdents64` synthesizes `.` and `..`, then emits VFS directory entry names
  as bytes; current source maps regular files and directories to Linux d_type.
  Symlink metadata and statx support have not yet been established by this
  audit. Path strings remain byte-oriented at the ABI boundary.
- The TTY endpoint uses the existing per-terminal stdin/stdout rings and ANSI
  parser, not an application-specific rendering path. `TCGETS`/`TCSETS*`,
  `TIOCGWINSZ`, and raw/cooked mode state are handled by the Linux ioctl shim.
  Window size comes from the process's terminal identity. PTY service support
  exists elsewhere in the kernel; Yazi should use its inherited terminal fds.
- `poll` has scheduler-backed waiting; epoll has a kernel instance table and
  readiness checks. Fcntl supports descriptor/status flags and duplication.
  Linux signal action/mask/altstack state is partly process-personality scoped;
  signal delivery and live SIGWINCH behavior are limited. Timer-backed
  `clock_gettime`/`nanosleep` exist. Linux clone/futex behavior, thread
  lifecycle, eventfd support, and Yazi's actual use of them remain to be
  checked against runtime evidence.
- mmap/munmap/mprotect are implemented by the process memory manager with
  Linux errno mapping; brk is supported for musl. `getrandom` is present.
- The prior Helios Note audit and regression cover TTY setup, geometry, input
  polling, ANSI rendering, file open/save, musl TLS/auxv startup, and restored
  terminal mode. They do not establish Yazi's syscall sequence or directory
  navigation behavior.
- This source audit has not established runtime availability/semantics for
  `/proc`, `/dev`, `/tmp`, `$HOME`, cwd mutation, directory fd-relative
  traversal, `statx`, or complete Unicode rendering. The initial Yazi run will
  determine which of these are relevant. Filesystem permissions must continue
  to be enforced by the existing VFS/security model.

## First-run record

The upstream release is Yazi v26.9.1, release tag `v26.9.1`, target
`x86_64-unknown-linux-musl`, asset
`yazi-x86_64-unknown-linux-musl.zip`. GitHub release asset ID `539261404`
publishes SHA-256
`9b9c39decccf8cb0ff53a7d637d38f8a79d93bbd0099f4ea9c619ef6bb392f5d`.
Because the direct GitHub release-assets endpoint failed local TLS hostname
verification, the archive was fetched from the SourceForge Yazi release mirror;
its checksum exactly matches GitHub's published digest and `unzip -t` passed.
The ELF is x86-64 ELF64, `ET_DYN`, static-PIE, OSABI System V.

**First-run blocker (classification J: environment/runtime ELF loader):** the
stock artifact cannot be loaded by the current shared `sunlight-elf` parser,
which requires `ET_EXEC`. It is rejected before a process is created, so this
attempt produces no Linux syscall trace. QEMU serial evidence after registering
the embedded payload with `sshl`:

```text
[ELF] header rejected: NotStaticExecutable
[SYSCALL] spawn: load failed: ElfLoadFailed
```

This is a concrete reason the official release cannot currently be used as-is.
It is static PIE with a `PT_DYNAMIC` table and 996 RELR entries expanding to
29,209 relocations; the shared parser only accepts `ET_EXEC`, and the current
loader has no PIE base/RELR relocation path. This limitation is documented
before switching to a source build. The experimental binary is built from the
exact upstream `v26.9.1` source commit `8dd895c695a5950330c2623eb43debf323b60654`
with static musl and the project's existing non-PIE linker flags; no Yazi
source changes are made. The official release artifact remains preserved in
ignored `target/` for comparison.

The pinned source-built image was launched by the QEMU `yazi-phase1` gate.
Helios created the Linux process and loaded all ELF segments. During Tokio
runtime construction, the trace reported unsupported Linux syscalls 28
(`madvise`), 204 (`sched_getaffinity`), and 290 (`eventfd2`). Yazi then panicked
with `Failed building the Runtime: ... code: 38 ... Function not implemented`,
followed by a general-protection fault and process exit 139. The trace places
`eventfd2` immediately before the runtime failure, making it the first concrete
blocker after the request. No Linux syscall semantics have yet been changed
based on this run. The smoke gate rejects a spawn followed by a Yazi runtime
panic/exit, so this attempt is not reported as a pass.

## Phase 1 result

Not met. Automated evidence confirms ELF load and process creation, but Yazi
does not complete Tokio initialization. Rendering, file listing, navigation,
UTF-8 display, clean exit, and idle behavior have not been exercised. The
first demonstrated generic work items are Linux `eventfd2`,
`sched_getaffinity`, and `madvise`, followed by rerunning the same unmodified
Yazi payload to reveal any subsequent requirements.

## Phase 1.1: observed runtime arguments and incremental reruns

The original source-built, stamped `ET_EXEC` payload remains in
`target/x86_64-unknown-linux-musl/release/yazi` with SHA-256
`15af218a823a16da5a8bb16caae27b03bc7c13bdf83d7c875e3d3d61d2028513`.
The local source checkout and musl C compiler were unavailable during this
iteration. `YAZI_USE_CACHED_ET_EXEC=1` explicitly verifies the official
archive (`YAZI_RELEASE_ARCHIVE=target/yazi-v26.9.1-x86_64-unknown-linux-musl.zip`
on this host), the pinned experimental payload digest, and the ELF type before the
existing gate runs; the default builder still builds
from the pinned, unmodified upstream source.

The bounded `[HELIOS-ABI-TRACE]` is active only with
`SUNLIGHT_INJECT_PHASE=yazi-phase1`. Its first run recorded:

| Call | Raw x86-64 arguments observed | Mapping or purpose |
| --- | --- | --- |
| `eventfd2` (290) | `initval=0`, `flags=0x80800` | `EFD_CLOEXEC | EFD_NONBLOCK`, without `EFD_SEMAPHORE` |
| `sched_getaffinity` (204) | `pid=0`, `cpusetsize=0x80`, mask pointers `0x7fffffffed50`, `0x7fffffffbe60`, `0x7fffffffbcc0` | calling task, 128-byte buffer |
| `madvise` (28) | `addr=0x1000001000`, `len=0x1000`, `advice=4`; `addr=0x1000002000`, `len=0x1000`, `advice=8` | `MADV_DONTNEED` and `MADV_FREE` respectively |

Each advised page came from an immediately preceding `mmap` with flags
`0x22` (`MAP_PRIVATE | MAP_ANONYMOUS`), fd `-1`, offset `0`.
The `MADV_DONTNEED` page was initially read/write; the `MADV_FREE` page was
initially `PROT_NONE`. Both are in the process's own anonymous VM ledger.
Other argument registers printed on a three-argument syscall are stale and
do not belong to that syscall.

| Requirement | Initial state | Phase 1.1 result |
| --- | --- | --- |
| Stock static PIE `ET_DYN` | loader rejects ELF | Deferred: no loader change |
| `eventfd2` | `-ENOSYS` | Shared, reference-counted 64-bit counter with semaphore mode, `O_NONBLOCK` status, `CLOEXEC`, blocking read/write wakeups, checked capacity, and poll/epoll readiness. The observed request now returns a descriptor. |
| `sched_getaffinity` | `-ENOSYS` | Validates PID, mask buffer and size; returns the scheduler's online CPU mask. Linux's **raw** success result is the number of mask bytes copied (8 for Helios's 64-CPU maximum), not libc's zero. Only those 8 bytes are copied; the remaining 120 bytes in the observed user buffer are outside the raw syscall's returned region. |
| `madvise` | unclassified `-ENOSYS` | Validates advice, alignment, overflow, userspace range, complete VM coverage, and anonymous/private ownership. For the observed `MADV_DONTNEED` and `MADV_FREE`, returns explicit `-ENOSYS` (38) on valid mappings: eager Helios mappings cannot currently implement discard or lazy reclamation. No success is fabricated and no VM contents are changed. |
| Next observed blocker | none | Linux `clone(CLONE_VM)` used for Tokio worker creation; existing MM-0 containment rejects it. |

An intermediate test stalled during early allocations with either the
validated `-EOPNOTSUPP` or validated `-ENOSYS` advice failure. Restoring the
original unsupported-syscall dispatch produced the same stall, ruling out
the new advice handler as its cause. The final path reports validated
`-ENOSYS`, without claiming either memory-semantic operation succeeded.

The bounded follow-up trace located the stall in a large
`munmap(0x1000203000, 0x1fd000)` of a private anonymous mapping. Region and
leaf preflight and the first TLB shootdowns completed. After shootdown, the
existing cleanup called `swap::untrack(frame)` for every released page; each
call scanned the entire anonymous swap-candidate deque. Removing candidates
for the fully unmapped owner range with one scan per bounded chunk preserves
the ownership/shootdown ordering and avoids the repeated full-deque scans.
The repeat QEMU run completed those large unmaps, reached the affinity query
and eventfd creation, and then failed on the previously observed thread
clone. Temporary per-chunk VM logging was removed after that diagnosis.

Linux's raw `sched_getaffinity` returns `min(cpusetsize, cpumask_size())` and
intersects the task's mask with active CPUs (`kernel/sched/syscalls.c`). Helios
currently has no per-task CPU restrictions, so its scheduler's available CPUs
are the allowed mask. With two QEMU CPUs the traced mask was `0x3` and the
raw return was 8 bytes.
Linux's eventfd implementation limits ordinary writes to `UINT64_MAX - 1`,
supports semaphore reads, and reports `EPOLLIN`/`EPOLLOUT` from its counter
(`fs/eventfd.c`).

After eventfd was implemented, the same payload progressed past Tokio's
previous `Failed building the Runtime ... ENOSYS` error. The next run reached
Tokio's worker thread creation and panicked with
`OS can't spawn worker thread: Resource temporarily unavailable` after
`[MM-0] rejected unsafe Linux clone(CLONE_VM)`. A later run with affinity
enabled still reached that blocker; it did not initialize worker threads or
enter the event loop. This requires a real shared-address-space/thread
implementation and is not a small syscall-number addition. Phase 1 remains
below the original rendering/navigation acceptance criteria.

The next observed raw Linux call was `clone(0x7d0f00,
0x1000a08f48, 0x1000a09b68, 0x1e016b8, 0x1000a09b38)`; the first argument
contains `CLONE_VM`. The sixth saved register in the trace does not belong to
this five-argument syscall. Helios returned raw `-ENOSYS` (38) through its
existing unsafe-clone containment path; musl/Tokio surfaced worker creation
failure as `EAGAIN`. No Tokio worker was created. Further progress requires
sharing a live address space safely across Linux threads, including TLS,
parent/child TID storage, exit cleanup, and synchronization; this is the next
evidence-backed Phase 1 work item.

### Phase 1.1 regression evidence (September 28, 2026)

- `cargo check --package sunlight-kernel`: passed.
- `cargo test --package sunlight-compat-linux --target x86_64-unknown-linux-gnu`:
  20 tests passed, including eventfd counter/flags/readiness/duplicate lifetime,
  affinity mask/size, and advice/range validation.
- Stock Yazi artifact baseline QEMU gate: passed its expected static PIE
  rejection (`NotStaticExecutable`); no stock process was created.
- Pinned static `ET_EXEC` Yazi QEMU gate: intentionally **failed** its runtime
  criterion after reaching affinity (`mask=0x3`, raw return 8), eventfd creation
  (fd 4), and rejected `clone(CLONE_VM)`. The trace records the Tokio worker
  creation panic and process exit. No worker, event loop, terminal setup, or
  render was observed.
- Helios Note QEMU regression: passed `rows=38 cols=138`, terminal
  initialization, and `interactive-ready`.
- `git diff --check`: passed.

Eventfd readiness is wired into the existing poll/epoll readiness query and
its blocking read/write conditions use scheduler sleep and wakeup. The Yazi
run did not reach eventfd I/O or a poll wait, so the runtime gates do not yet
exercise blocking wakeup or establish idle-CPU behavior for an eventfd waiter.
The kernel's host `cargo test` target conflicts with its freestanding panic
handler; the pure counter, lifetime, and affinity tests run in compat-linux.

## Phase 1.2: Linux thread architecture audit (September 28, 2026)

The observed **raw x86-64** syscall is `clone(flags=0x7d0f00,
child_stack=0x1000a08f48, parent_tid=0x1000a09b68,
child_tid=0x1e016b8, tls=0x1000a09b38)`. Its flags decode as follows, using
Linux `include/uapi/linux/sched.h`, rather than matching the aggregate number:

| Flag | Value | Linux meaning | Current Helios status |
| --- | ---: | --- | --- |
| `CLONE_VM` | `0x00000100` | shared userspace VM | native borrower shares root and ledger; Linux group lifetime absent |
| `CLONE_FS` | `0x00000200` | shared cwd/root/umask | cwd stored separately in each task |
| `CLONE_FILES` | `0x00000400` | shared FD table | native borrower copies FD table |
| `CLONE_SIGHAND` | `0x00000800` | shared signal dispositions | signal handlers and mask both reside in task's `SignalState` |
| `CLONE_THREAD` | `0x00010000` | same TGID, separate TID | only a single task PID exists today |
| `CLONE_SYSVSEM` | `0x00040000` | shared SEM_UNDO adjustments | SysV semaphores and semadj absent; future group would hold empty semadj |
| `CLONE_SETTLS` | `0x00080000` | child FS base from `tls` | native spawn supports per-task FS base |
| `CLONE_PARENT_SETTID` | `0x00100000` | parent stores child TID | absent |
| `CLONE_CHILD_CLEARTID` | `0x00200000` | child zeroes TID and futex wakes at exit | absent |
| `CLONE_DETACHED` | `0x00400000` | legacy no-op for raw clone | ignored by modern Linux, no detached lifecycle to add |

The low-byte exit signal is zero. `CLONE_CHILD_SETTID` is absent: the child TID
pointer must be recorded for clearing, but not populated at creation. Linux
`kernel/fork.c` rejects `CLONE_SIGHAND` without `CLONE_VM` and `CLONE_THREAD`
without `CLONE_SIGHAND`; Linux also rejects a nonzero exit signal for a
thread-group clone. The x86-64 raw argument order uses the fifth argument
register `r8` for TLS and fourth register `r10` for `child_tid`. These facts
are distinct from glibc's `clone()` wrapper and `clone3()`.

### Ownership before implementation

- `Process` is both the scheduler task and resource holder (`pid`, saved
  `context_rsp`, kernel stack, FS base, `AddressSpace`, FDs, cwd, credentials,
  signals, brk and mmap cursor). The scheduler indexes these objects in a
  global locked vector, with per-CPU run queues and task ownership fields.
- `AddressSpace` holds a page-table root and a generation-tagged region
  ledger. Native `ThreadSpawn` constructs a non-owning borrower of that root;
  it **copies** FDs, cwd, environment and signal state and starts at a native
  trampoline. It is not a Linux `clone()` implementation.
- `LinuxProcessState` holds `tid_address`, robust list, signal alternate stack,
  termios, eventfd wait state, and brk counters in one per-task structure.
  `set_tid_address` currently returns `pid`; `getpid` and `gettid` both map
  to native `Getpid`. This is correct only while PID and TID coincide.
- The scheduler saves/restores FS base per task on dispatch, including CPU
  migration. Its syscall entry builds a frame **on the userspace stack**;
  `syscall_dispatch` sees saved GPRs including `rcx` (the address immediately
  following `syscall`) and `r11` (user RFLAGS). A Linux child needs its own
  kernel IRET frame copied from these registers, with `RAX=0`, supplied RSP,
  and supplied FS base. Native `ThreadSpawn` instead creates a trampoline frame.
- Exit marks the current task Finished and pivots to its per-CPU idle stack.
  Reaping owns FD closure, shared-memory cleanup, kernel-stack disposal, and
  page-table reclamation. If the page-table owner exits, the scheduler
  forcibly terminates all borrowers, including native workers. An ordinary
  worker exit therefore cannot safely reuse this owner/borrower policy as a
  Linux thread-group policy. Waitpid/zombies are task PID based.
- Syscall user-memory copy validates PTEs while holding the scheduler lock,
  then copies through HHDM, but mmap/munmap/brk mutation and userspace accesses
  across two CPUs still need a shared VM serialization/lifetime audit.
- There is **no Linux futex implementation**: syscall 202 translates to
  `ENOSYS`. `FUTEX_WAIT`, `FUTEX_WAKE`, and `FUTEX_PRIVATE_FLAG` must use the
  same VM-based wait key for group members and participate in child-TID exit.

The required ownership boundary is a Linux thread group with shared VM, FD,
FS, signal dispositions, credentials and process lifetime, and individual
schedulable threads with TID, register context, TLS/FS base, signal mask,
robust list, alternate stack, and clear-child-TID. A worker's `exit` must
perform child-TID cleanup and futex wake without closing FDs, killing its
siblings, or reclaiming the owner's VM. `exit_group` requires a separate
group-wide termination path. This paragraph records the pre-implementation
audit; the implementation and its remaining limitations follow below.

Sources: Linux `include/uapi/linux/sched.h`, `kernel/fork.c`, and
`man-pages/man2/clone.2` (raw x86-64 syscall ABI and historical
`CLONE_DETACHED`).

### Phase 1.2 implementation and ownership

Linux threads use distinct scheduler `Process` records with separate register
frames, kernel stacks, FS bases, signal masks, alternate stacks, robust-list
addresses, clear-child-TID addresses, and Linux-visible TIDs. The original
Linux task owns the group (`linux_tgid == pid`); later tasks carry its TGID
and borrow the **same** page-table root and VM ledger. The owner's FD table,
cwd and signal dispositions are accessed through `current_shared_process`.
The owner and its VM survive an individual worker's exit and are reclaimed
once all borrowers have reaped. The scheduler lock serializes shared resource
and VM metadata mutations. Native tasks continue using their own resources.
This is an ownership distinction within the existing `Process` structure,
not a scheduler rewrite.

The child starts at the raw caller's post-`syscall` instruction (`RCX`), with
copied GPRs and flags (`R11`), `RAX=0`, caller-provided RSP and its own FS base
(`tls` when requested). Clone checks the writable child stack and parent TID
pointer before publishing the parent TID and scheduling the child. Linux
`getpid` returns TGID; `gettid` and `set_tid_address` return the calling TID.
Linux `exit` finishes only its calling thread; `exit_group` marks group
members for termination. Exit clears its own `clear_child_tid` with checked
userspace access and wakes one VM-keyed futex waiter. Invalid addresses
cannot fault the kernel during teardown. Worker reaping does not reclaim
the group VM or close the group's FD table.

| Observed flag | Status | Boundary |
| --- | --- | --- |
| `CLONE_VM` | implemented | identical root and VM ledger; owner held until borrowers reap |
| `CLONE_FS` | implemented | shared cwd; root and umask have no separate mutable Linux state yet |
| `CLONE_FILES` | implemented | owner FD table under scheduler lock |
| `CLONE_SIGHAND` | implemented | owner dispositions, separate per-thread masks; delivery remains partial |
| `CLONE_THREAD` | implemented | separate TID and scheduler state, common TGID, no waitable worker process |
| `CLONE_SYSVSEM` | represented but currently vacuous | group owner holds an empty semadj context; SysV IPC does not yet exist |
| `CLONE_SETTLS` | implemented | child FS base from raw fifth argument, restored per task |
| `CLONE_PARENT_SETTID` | implemented | checked 32-bit parent store before child is runnable |
| `CLONE_CHILD_CLEARTID` | implemented | no initial store without `CHILD_SETTID`; zero and futex wake on exit |
| `CLONE_DETACHED` | represented but currently vacuous | Linux ignores this historical bit for this raw clone combination |

Flags are validated individually, including `SIGHAND`/`VM` and
`THREAD`/`SIGHAND` dependencies, zero thread exit signal, and supported
resource sharing. Namespace and process-copy modes return `-ENOSYS`;
malformed combinations return `-EINVAL`. There is no fork-style copy.
Futex `WAIT`/`WAKE` with `PRIVATE` use the shared VM identity and 32-bit
virtual address. Timed waits, robust-futex owner death, PI futexes, and
semadj operations remain unsupported.

The direct static Linux probe exercises group/worker identities, shared
memory, separate `%fs:0` values after scheduling, parent-TID publication,
zeroed child TID and futex wake, shared eventfd and FD close, cwd change,
shared signal handler with thread-local mask, and negative clone cases.
One successful two-CPU QEMU run printed:

```text
THREAD_PROBE main_pid=0x00000023 main_tid=0x00000023
THREAD_PROBE worker_pid=0x00000023 worker_tid=0x00000024
shared_memory=ok
fd_table=ok eventfd=ok
tls=ok
child_tid_clear=ok
THREAD_PROBE PASS
```

Worker TID 36 reaped without reclaiming user frames; group owner TGID 35
later reclaimed 520 user frames and 9 page tables. The final probe trace
records `clear_child_tid tid=36 addr=0x4020f8 woken=1`, confirming that the
zero store woke an actual waiter on the same VM-keyed futex. An attempted
in-kernel `sti; hlt; cli` deschedule faulted because x86-64 syscall entry
executes on the **userspace stack**; that change was removed. The final path
instead sends a dedicated reschedule IPI while syscall interrupts are
disabled. The IPI runs after `SYSRET` returns to ring 3, letting the normal
interrupt path save a userspace frame and schedule another task. This vector
does not advance the timer clock. The two-CPU probe passed with `woken=1`,
and the pinned Yazi trace no longer showed a run of immediate `FUTEX_WAIT`
successes on the same word. Idle CPU behavior remains unproven because Yazi
still encounters other runtime failures.

### Pinned Yazi rerun and next evidence-backed blocker

The verified unchanged `ET_EXEC` v26.9.1 payload created worker TIDs 36
and 37 under TGID 35, with distinct TLS bases. Both reached userspace.
Worker 36 returned from `epoll_pwait(fd=3, ...)` with two events. Its raw
`recvfrom` (45) with fd 8, buffer `0x1000a08b00`, length `0x80`, flags 0
and null address arguments returned `-ENOSYS`. In one run it entered an abort
path; thread ordering and later failure differed on the following rerun.
This points to socket I/O, potentially a larger subsystem. Main TID 35 also
got `-ENOSYS` on `prlimit64(302, pid=0, resource=7)` and
`getrlimit(97, resource=7)`. Worker TID 37 repeatedly called
`FUTEX_WAIT|PRIVATE` on `0x1000c0d8d8`, expecting 1, with null timeout.
Accepting clone does not prove Tokio initialized, terminal setup or idle
event-loop behavior. Validated `MADV_DONTNEED` and `MADV_FREE` still return
`-ENOSYS` on the previously observed private anonymous pages.

Milestones confirmed: ELF load, main process, affinity (`mask=0x3`), eventfd
creation, clone acceptance, worker TID allocation, parent-TID publication,
child scheduling, TLS setup, child userspace execution, worker runtime
syscalls and futex wait attempts. Tokio runtime initialization, Yazi main
event loop, terminal setup, rendering, directory enumeration and interactive
navigation remain unverified.

The final bounded trace on September 28, 2026 additionally captured main
TID 35 calling `recvfrom(45, fd=9, buf=0x7fffffff8de0, len=0x400,
flags=0x40, src=NULL, addrlen=NULL)` and
`sendto(44, fd=12, buf=0x1, len=0, flags=0x40, dest=NULL, addrlen=0)`;
both returned `-ENOSYS`. It then called
`ioctl(16, fd=11, request=0x5421 /* FIONBIO */, arg=0x7fffffff8de0)`,
which returned `-EINVAL` (`-22`). The command exited with
`Error: Invalid argument (os error 22)`, exit status 1, after worker TIDs
36 and 37 exited with status 0. Their reaping released zero user frames;
the group owner's final reap released its shared VM (10,246 user frames,
34 page tables in this run). Thus the immediate observed user-visible
failure on this rerun is the missing FIONBIO handling for the FD 11 pipe;
socket I/O is an independently observed missing compatibility subsystem.
There was no first render or idle event loop. The kernel host test target
still encounters its freestanding panic-handler conflict; the QEMU boot
gates run the native scheduler's MM-0 borrower lifecycle test and the new
userspace Linux thread probe instead.

On the later rerun with immediate post-`SYSRET` futex descheduling, the
probe still passed, and Yazi again reached two worker threads and the same
missing socket operations. Worker TID 36's
`recvfrom(45, fd=8, buf=0x1000a08b00, len=0x80, flags=0)` returned
`-ENOSYS`; its userspace abort ended in a general-protection fault at
`0x17d8c30`. Main TID 35's
`ioctl(16, fd=11, request=0x5421 /* FIONBIO */, arg=0x7fffffff8de0)`
returned `-EINVAL`, then main blocked on
`FUTEX_WAIT|PRIVATE(addr=0x1000000968, expected=1, timeout=NULL)`; worker
TID 37 exited. The bounded QEMU gate timed out without a Yazi exit or first
render. No repeated immediate WAIT loop appeared in that trace. Socket I/O
and FIONBIO/pipe status handling are the next concrete Linux runtime gaps;
the abort signal/fault path is a separate signal-delivery limitation.
Neither the earlier exit 1 nor this timeout establishes runtime readiness.

### Phase 1.2 verification (September 28, 2026)

- `cargo test --package sunlight-compat-linux --target x86_64-unknown-linux-gnu`:
  24 passed, including the clone-flag combinations, affinity, eventfd and
  advice tests.
- `cargo check --package sunlight-kernel` and `git diff --check`: passed.
- `tools/test.sh helios-thread-probe`: passed under two online QEMU CPUs;
  actual userspace showed main PID/TID 35/35, worker PID/TID 35/36,
  independent TLS values, shared memory/FD/FS/signal behavior, and
  `clear_child_tid ... woken=1`. The scheduler's native MM-0 borrower and
  address-space lifecycle boot tests passed in the same run.
- `tools/test.sh yazi-baseline`: passed its expected official `ET_DYN`
  loader rejection. The stock binary was not run as a process.
- `tools/test.sh helios-note-regression`: passed with `rows=38 cols=138`,
  terminal initialization and `interactive-ready` after the IPI change.
- `tools/test.sh helios-static-runtime`: passed the independent single-thread
  Linux syscall probe (`LINUX-PROBE RUNTIME PASS`) and Helios Note readiness.
- `tools/test.sh yazi-phase1`, with the pinned, unchanged `ET_EXEC` payload:
  intentionally failed its runtime criterion after creating two workers and
  reaching futex/socket I/O; the last run timed out. No Yazi render, terminal
  setup, directory navigation or idle event-loop acceptance was observed.

The kernel host test target remains blocked by its existing freestanding
panic-handler collision. Kernel scheduler/VM and futex lifecycle are covered
here by the boot-time MM-0 tests and the live two-CPU userspace probe rather
than by host-only tests that would mirror internal fields.

## Phase 1.3: nonblocking descriptions and observed Unix stream receive

The bounded *pre-change* pinned Yazi run traced FD creation before modifying
`recvfrom` or `FIONBIO`:

| FD | Creation and duplication | Object and flags at first observed call |
| --- | --- | --- |
| 8 | `socketpair(53, AF_UNIX=1, SOCK_STREAM\|SOCK_NONBLOCK\|SOCK_CLOEXEC=0x80801, protocol=0, pair=0x7fffffffbed8)` created FDs 6 and 7; `fcntl(6, F_DUPFD_CLOEXEC=1030, 3)` produced FD 8; `fcntl(8, F_SETFD, FD_CLOEXEC)` followed. | FD 8 shares FD 6's **open description**. The prior kernel represented its inbound direction with pipe ring 0 (`handle=0x80000000`) and reported flags `0x80000`, accidentally omitting `O_NONBLOCK` from its status. Peer FD 7 writes into that ring; the socketpair is bidirectional. |
| 11 | `socketpair(53, AF_UNIX=1, SOCK_STREAM\|SOCK_CLOEXEC=0x80001, protocol=0, pair=0x7fffffff8748)` created FD 11 and FD 12. | FD 11 is a bidirectional socket endpoint with inbound pipe ring 2 (`handle=0x80000002` in the old kernel), initially represented with `O_RDONLY` plus descriptor `CLOEXEC` (`0x80000` in the old combined field). No duplicate of FD 11 appears before FIONBIO; FD 12 is its peer. |

The FD 8 pair is created before the Tokio worker clones and FD 8 is consumed
by worker TID 36 immediately after `epoll_pwait(281, fd=3)` returned events.
This creation/use sequence supports an inference that FD 8 is a runtime
reactor/wake channel, rather than an Internet socket. The later FD 9/10 pair
is involved in the signal-handling sequence, and the FD 11/12 pair is created
just before `sendto(fd=12, len=0, flags=MSG_DONTWAIT)` and FIONBIO. The trace
does not identify a specific upstream crate as the owner of each pair.

All six **raw** arguments of the worker's first call are
`recvfrom(45, fd=8, buf=0x1000a08b00, len=0x80, flags=0,
src_addr=NULL, addrlen=NULL)`. The result before this phase is raw `-ENOSYS`
(`0xffffffffffffffda`). Its socket was connected to peer FD 7, created with
`SOCK_NONBLOCK`. The old endpoint had a pipe receive ring, but the prior trace
did not record its queued byte count or peer-close state at the instant of the
call. The first main-thread FIONBIO was
`ioctl(16, fd=11, request=0x5421, arg=0x7fffffff8de0)` and returned
`-EINVAL`. Its pointed-to integer value must be read by the new syscall path;
the old trace only captured the pointer.

The FD-table audit found `FD_CLOEXEC` and `O_NONBLOCK` mixed in each copied
`FileDescriptor.flags`; `fcntl(F_SETFL)` changed the copy, `pipe2(O_NONBLOCK)`
ignored its creation flag, and both pipe reads and writes returned `EAGAIN`
regardless of the flag. Eventfd kept its own status flag. Pipe rings contain
queued bytes and reader/writer counts; eventfd owns a counter and semaphore
mode. No general Unix socket object existed: the old `socketpair` returned
plain pipe descriptors, so `recvfrom` could not validate socket identity.

The implementation now gives every descriptor an index into a reference-counted
open-description status pool. Dup, dup2/dup3, F_DUPFD, and copied native FD
tables retain the index; descriptor CLOEXEC remains on the individual entry.
`F_GETFL`, `F_SETFL`, `FIONBIO`, and creation flags consult the same status;
the eventfd counter no longer owns a parallel `O_NONBLOCK` field. An
AF_UNIX/SOCK_STREAM socketpair uses two existing pipe rings, one per direction,
with tagged socket endpoints, separate status descriptions and reference
counts. `read`, `write`, `sendto`, and `recvfrom` use the same underlying byte
transfer path; poll/epoll readiness asks the rings about data, capacity, or
EOF independently of `O_NONBLOCK`. Blocking calls register a ring wait while
holding the scheduler lock, rewind the syscall and sleep until a writer,
reader or close wakes them. The scope remains Unix stream socketpair; other
socket families, datagrams and source-address copyout are not claimed.

Progress ledger (runtime items only change after pinned Yazi evidence):

```text
[x] ELF loaded
[x] main process created
[x] affinity query succeeded
[x] eventfd created
[x] Linux threads created
[x] workers execute userspace
[x] TLS verified
[x] futex wait/wake observed
[x] shared FD table verified
[x] FIONBIO pipe-backed socket path succeeds in pinned Yazi
[x] recvfrom observed nonblocking empty-queue path returns EAGAIN
[ ] runtime I/O driver remains operational
[x] Tokio workers/reactor and main terminal setup positively established
[ ] Yazi main event loop entered
[ ] terminal initialized
[ ] first render
[ ] directory enumerated
[ ] interactive navigation
```

### Phase 1.3 runtime evidence (September 28, 2026)

The exact FD 8 receive was captured after the change on worker TID 36:
`recvfrom(45, fd=8, buf=0x1000a08b00, len=0x80, flags=0,
src_addr=NULL, addrlen=NULL)` returned `-EAGAIN` (`0xfffffffffffffff5`).
Its Unix stream receive ring 0 contained **zero** bytes; FD 8 and its
duplicate were both readers, and peer FD 7 was still open (ring counts
`queued=0, readers=2, writers=1`). The open description had `O_NONBLOCK`
from `SOCK_NONBLOCK` at creation. This is the required absence-of-data
result, not successful delivery of bytes. The separate guest socket probe
verified payload receive, duplicated status, nonblocking empty receive,
peer EOF, bad buffer `EFAULT`, nonsocket `ENOTSOCK`, and bad FD `EBADF`.

Main TID 35's `ioctl(16, fd=11, FIONBIO=0x5421,
arg=0x7fffffff8de0)` read userspace integer **1**, changed its shared
open-description status from `0x2` to `0x802`, and succeeded. FD 11 is the
first endpoint of the AF_UNIX stream pair 11/12, implemented with pipe
rings; it is a socket descriptor backed by a pipe, not a `pipe2` descriptor.
No duplicate of FD 11 was observed. The guest pipe probe separately tests
`pipe`, `pipe2(O_NONBLOCK)`, `dup`, FIONBIO set/clear, F_GETFL, empty-read
`EAGAIN`, successful transfer, full-pipe `EAGAIN`, and EOF. Later FIONBIO
calls on FDs 13 and 14 likewise changed `0x2` to `0x802`.

The first follow-on error was on the terminal path: `ioctl(fd=1,
TCGETS2=0x802c542a, arg=0x7fffffffb560)` returned `-EINVAL` and Yazi
exited 1. A correct 44-byte x86-64 `termios2` copyout advanced Yazi to
`TCSETSF2=0x402c542d` and `TCSETS2=0x402c542b`, whose original `-EINVAL`
again caused exit 1. Supporting the observed 44-byte termios2 input path
let the main thread enter **raw TTY mode** and continue into additional
runtime work. No Yazi render or directory listing was visible. Worker
epoll calls also revealed a shared-FD-table lookup error: initially they
used a worker's placeholder FD table and reported live socket interests
as `EPOLLERR|EPOLLHUP`. The lookup now uses the thread group's shared table.
Infinite empty poll/epoll waits now recheck on wakeup and request immediate
post-SYSRET scheduling; the final bounded Yazi rerun confirmed that the
reactor waits after receiving its initial edge event.

The former `yazi-phase1` gate could pass on spawn, affinity and eventfd
markers alone even when no Yazi render had occurred. Its expected result
now also requires `[HELIOS-YAZI] first render confirmed`, a milestone that
must be backed by real renderer evidence before being emitted. An absent
render marker fails the gate at its bounded timeout. A previous run with
working termios2 passed the *old* launch-only check, but it does **not**
establish Yazi Phase 1 acceptance. Eventfd creation at FD 4 is observed;
an actual Tokio eventfd read/write cycle was not observed, and a persistent
reactor idle wait remained unverified at that checkpoint (see the final
reactor trace below). Validated `MADV_DONTNEED` and `MADV_FREE` continue to
return `-ENOSYS` rather than claiming discard/lazy-free success.

The corrected 180-second acceptance gate **failed** on its next run. Yazi
entered raw mode, workers 36 and 37 continued to run, then the main thread
restored cooked mode and all three threads exited cleanly (main status 1).
The TTY output reported `Failed to read config
"/root/.config/yazi/yazi.toml"`; the shell serial summary truncates the
following `Caused by: In...`. The kernel also logged repeated `open(2)`
rejections with flags `0x88000` and one `0x880c2`. Both include the Linux
`O_LARGEFILE` bit `0x8000`, which Helios currently excludes from
`OPEN_SUPPORTED_FLAGS`, so these calls return `-EINVAL` before VFS lookup.
The `0x880c2` request additionally contains `O_CREAT|O_EXCL|O_RDWR`; do not
expand this phase into config-file creation or filesystem mutation. A
bounded path/flag trace was then used to identify precisely which rejected
read caused the visible error. The `ppoll(271)` `-ENOSYS` calls seen just
before teardown are another observed, unclassified fallback; do not infer
they caused the config failure without more evidence. The corrected gate
reports failure for exit status 1 and the missing first-render marker.

The follow-up path trace confirms the exact failing read: main TID 35's
`open(2, path="/root/.config/yazi/yazi.toml", flags=0x88000,
mode=0)` was rejected because `unsupported=0x8000` (`O_LARGEFILE`), returning
`-EINVAL` before VFS path lookup. Earlier read-only probes for
`/proc/self/cgroup` and `/proc/sys/kernel/osrelease` received the same
error; a `0x880c2` create attempt targeted a `/dev/shm/yazi-*` object.
Accepting or emulating file creation belongs to later, separately scoped
filesystem work. TID 35's visible config error and exit status 1 are
directly consistent with this rejected read. The gate now exits promptly
on a finished Yazi process while continuing to fail without a real render.

The final reactor trace found another directly observed I/O gap. Tokio
registered FD 4 (eventfd) and FDs 8/9 (Unix streams) with `EPOLLET=0x80000000`;
FD 9 also requested `EPOLLOUT`. The old epoll readiness query returned the
same two events on each `epoll_pwait` while the eventfd remained readable and
FD 9 remained writable, creating a worker busy loop. Interests now remember
delivered edge readiness. When actual eventfd or pipe/socket I/O drains a
condition, the descriptor/readiness code rearms that edge; a subsequent
event can wake the reactor. `EPOLL_CTL_MOD` explicitly rearms. The static
guest socket probe also verifies eventfd `EPOLLET`: write -> one event,
second wait while still readable -> zero, drain via read, write again ->
one new event. Both guest I/O probes pass.

After this correction, unchanged Yazi worker TID 37 received **one**
epoll event from the writable Unix stream; the next infinite
`epoll_pwait(fd=3, events=0x100060c000, maxevents=0x400, timeout=-1,
sigmask=NULL, sigsetsize=8)` returned zero ready events and entered the
scheduler wait. The subsequent config read failed at the exact `open` path
and status above; Yazi exited 1 without a first render. The workers and
reactor operated while main began terminal setup. A real eventfd read/write
cycle, directory enumeration and interactive navigation remain unverified.
The epoll empty-readiness check and wait registration now run under the
same scheduler lock used by event producers and wakeups, closing the
check-to-sleep lost-wakeup window on the observed reactor path.
Generic `poll` also rechecks readiness and registers its wait under that
lock after userspace copyout, so pipe, eventfd and TTY producers cannot
miss a poller between those operations. Infinite waits retry the syscall
on wakeup instead of returning a false zero-ready result.

The final bounded eventfd trace captured two actual Yazi
`write(fd=4, value=1)` operations, with FD 4 changing from unreadable to
readable in the reactor's epoll query. No matching Yazi eventfd read was
recorded before the main thread's config-read failure and orderly worker
teardown, so a complete Tokio eventfd drain cycle remains unproven.

The last pinned QEMU rerun after the atomic generic `poll` change again
returned zero from `ioctl(fd=11, FIONBIO, &one)` and `-EAGAIN` from an empty
nonblocking Unix stream receive. Worker TID 37 obtained two initial reactor
events, then the next `epoll_pwait(fd=3, maxevents=1024, timeout=-1)` found
zero ready events and slept; no repeated edge-event loop occurred. Main TID
35 again rejected `open("/root/.config/yazi/yazi.toml", 0x88000, 0)` for
unsupported `O_LARGEFILE=0x8000` before lookup, printed the config error,
restored cooked TTY mode and exited 1. Workers 36 and 37 exited cleanly.
The gate failed because no first render was observed, as required. The
two-CPU thread probe, guest I/O probes, Helios Note QEMU regression and
single-thread Linux runtime gate all passed after the wait change; the
24-test compatibility host suite, kernel package check and `git diff --check`
also passed.

### Phase 1.3 acceptance ledger

| Criterion | Evidence and current result |
| --- | --- |
| FD 8 provenance, arguments | AF_UNIX SOCK_STREAM socketpair 6/7, FD 8 duplicates 6; six raw arguments and `-EAGAIN` in the worker trace above. |
| FD 11 FIONBIO | AF_UNIX stream endpoint 11, backed by pipe rings; userspace `int=1` changed status `0x2 -> 0x802` and returned zero. |
| Generic status | Open-description pool shares `O_NONBLOCK` among dup and cloned FD tables; FIONBIO, F_SETFL, F_GETFL, pipe2 and socket creation consult it. |
| Pipe/socket receive | Live Linux probes pass empty/nonempty/EOF, full-pipe write and duplicate cases; socket receives preserve Unix stream semantics and distinct `EBADF`/`ENOTSOCK`/`EFAULT`. |
| Worker/reactor | Two CPU threads run with shared FD state; Tokio epoll interest includes `EPOLLET`, and a worker goes from one returned event to a zero-ready wait after edge handling. |
| Eventfd runtime I/O | Real Yazi created FD 4, registered it with epoll, wrote value 1 twice and observed readability. A direct Yazi read/drain was **not** established; the live guest edge probe exercised that cycle separately. |
| Yazi UI acceptance | **Failed**: TID 35 `open("/root/.config/yazi/yazi.toml", 0x88000, 0)` returned `-EINVAL` for unsupported `O_LARGEFILE=0x8000`, and Yazi exited 1 with no first render. This is small read-side Linux open flag work, but the observed `/dev/shm/yazi-*` `O_CREAT|O_EXCL|O_RDWR` request touches the separately deferred filesystem mutation scope. |

Final probes: `helios-io-probe` (`IO_PIPE PASS`, `IO_SOCKET PASS`,
including the eventfd edge cycle), `helios-thread-probe` (two CPUs, shared
memory/FD/TLS/child-TID clear), stock `yazi-baseline` (expected ET_DYN
rejection), `helios-note-regression` (geometry and interactive-ready),
and `helios-static-runtime` (Linux userspace and Note) passed. The
`sunlight-compat-linux` host suite passed all 24 tests. Kernel package
`cargo check` and `git diff --check` passed. The freestanding kernel host
test target retains the previously recorded duplicate panic-handler
conflict; native boot MM-0 checks and the guest thread/I/O probes exercise
the relevant behavior. `yazi-phase1` failed its corrected render gate, as
required. No Yazi files or filesystem mutation behavior were changed.

### Phase 1.4 open flag audit (September 28, 2026)

The captured main-thread call was `open(2,
"/root/.config/yazi/yazi.toml", 0x88000, 0)`. The Linux x86-64
`asm-generic/fcntl.h` definitions and musl's x86-64 open contract give the
following complete decoding of the observed word:

| Field | Value | Meaning |
| --- | --- | --- |
| Access mode (`O_ACCMODE`) | `0` | `O_RDONLY` |
| `O_LARGEFILE` | `0x8000` | 64-bit file offset contract |
| `O_CLOEXEC` | `0x80000` | descriptor-local close on exec |
| All other bits | `0` | no further requested behavior |

The adjacent, independent bits are `O_DIRECT=0x4000` (unsupported),
`O_LARGEFILE=0x8000` (valid), and `O_DIRECTORY=0x10000` (validated as a
directory constraint). The Linux `open` and `openat` shims both reach
`open_resolved_path`; Linux `creat(2)` has no translation into that helper.
Previously its `OPEN_SUPPORTED_FLAGS` mask omitted `0x8000`, and the helper
returned native `ERR_EINVAL` before VFS pathname lookup. The syscall
adapter translated that to Linux `-EINVAL`.

The parser now validates the supported Linux bit mask and access/creation
combinations, then keeps `O_CLOEXEC` descriptor-local and the Linux-visible
`O_LARGEFILE` status on the shared open description for `F_GETFL`. `F_GETFD`
still reads the descriptor's close-on-exec bit. `F_SETFL` still changes only
`O_APPEND`/`O_NONBLOCK`; it cannot clear `O_LARGEFILE`. The VFS receives a
path, requested access/creation operation and permissions, not a Linux
numeric flag word. Its x86-64 read/write offsets are `usize` (64-bit); no
32-bit truncation, 2 GiB boundary, or separate large-file operation was added.
`F_GETFL` omits the one-time creation flags `O_CREAT`, `O_EXCL`, `O_TRUNC`
and `O_NOCTTY`; directory and no-follow constraints remain visible.

The guest `linux-open-largefile-probe` opens `/etc/passwd` with exactly
`0x88000`, verifies its `root:` prefix, `F_GETFD=FD_CLOEXEC`, the
`O_LARGEFILE` `F_GETFL` bit without `O_CLOEXEC`, and closes the descriptor.
It also calls `openat` on `/definitely/not/present` with the same flags and
requires `-ENOENT`. `O_DIRECT` and an unknown bit must still give `-EINVAL`.

Phase 1.4 milestone ledger (only check a runtime item after guest evidence):

```text
[x] ELF loaded; process created; affinity query succeeded; eventfd created
[x] Linux threads created; workers execute userspace; TLS verified; futex wait/wake
[x] shared FD semantics; Unix socket recvfrom; FIONBIO / O_NONBLOCK
[x] epoll initial events observed; terminal raw-mode setup began
[x] O_LARGEFILE config lookup accepted
[x] config lookup reaches normal VFS result (ENOENT)
[x] Tokio workers/reactor active after config fallback
[x] Yazi startup continues past config loading
[ ] Yazi main event loop established
[x] first application frame observed (Loading... pane; bounded runs vary)
[x] getdents64 on cwd / began (worker TIDs 38, 41 and 40)
[ ] interactive navigation
```

The pinned payload and upstream archive were verified via
`YAZI_USE_CACHED_ET_EXEC=1` and
`YAZI_RELEASE_ARCHIVE=target/yazi-v26.9.1-x86_64-unknown-linux-musl.zip`.
This host does not have `musl-gcc`; the cached ET_EXEC SHA-256 is
`15af218a823a16da5a8bb16caae27b03bc7c13bdf83d7c875e3d3d61d2028513`.
The payload and Yazi source were not patched. The open probe's shell command
was added to the existing allowlist of embedded Linux test executables.

Main TID 35 now accepts `open("/root/.config/yazi/yazi.toml", 0x88000, 0)`
and returns Linux `-ENOENT` from the VFS lookup, not `-EINVAL` from flag
validation. The inherited root session has `HOME=/root`, uid/euid=0,
`cwd=/`, and no `XDG_CONFIG_HOME`. Yazi selects `/root` consistently with
that environment. No config file or VFS alias was added.

The next actual fatal call was Linux x86-64 `fchmod(91, fd=15, mode=0700)`
on the open `/tmp/yazi-0` cache directory. It initially returned
`-ENOSYS` (`0xffffffffffffffda`); Yazi printed `Failed to create cache
directory: /tmp/yazi-0: Function not implemented (os error 38)` and offered
to continue with preset settings. This proves the missing config itself
was not the fatal dependency. A small generic fd-based chmod shim now
resolves the live VFS handle, enforces Sunlight's write policy and
owner/root rule, preserves the file-type bits, and passes errors through
Linux errno translation. Static RAM filesystem entries now return an
explicit read-only error instead of falsely succeeding on chmod. A rerun
confirmed `fchmod(fd=15, mode=0700, /tmp/yazi-0) -> 0`; a worker also
successfully chmodded `/tmp/yazi+0`. This supports Yazi's internal startup
cache without implementing any user-requested Phase 2 filesystem operation.

The earlier TID 35 request in the recorded run is
`open("/dev/shm//yazi-CABZDEI3IWVGY4O4GWVEIXS2", 0x880c2, 0600)`:
`O_RDWR|O_CREAT|O_EXCL|O_LARGEFILE|O_CLOEXEC`, without other bits. TID 35
received `-EACCES` from the existing immutable-root policy before a file
was created. Yazi then used `/tmp/yazi-0`, so this request did not become
the fatal blocker. The observed fallback suggests runtime scratch/cache
storage; the trace does not establish unlink, mmap sharing, or locking of
the denied object. `/dev/shm` was not implemented.

After `fchmod` succeeded, real Yazi/Tokio workers remained active and the
main thread sent terminal alternate-screen, cursor-hide and clear-screen
escapes, followed on one bounded run by positioned application content:
`Yazi: /` title, a `Loading...` pane with borders, and a mode/status bar
(`NOR`, `0/0`). The actual frame content, rather than the clear-screen
escape, establishes the **first render** milestone. The gate's first-render
marker is emitted only after the observed alternate-screen and positioned
application-frame bytes. A later bounded run ended before that frame was
emitted, so render timing remains variable; no clean Phase 1 acceptance is
claimed. No directory entries, interactive navigation or clean Yazi exit
were observed. Eventfd FD 4 was written and epoll reported readability;
no real Yazi eventfd read/drain/rearm cycle is established yet. A subsequent
bounded run logged `getdents64` on the current directory `/` from worker
TIDs 38, 41 and 40: the first call used `count=2048, skip=0` and the next
used `skip=16`. This establishes **directory enumeration began**; the
rendered frame had not been emitted by that run's deadline, and no completed
interactive directory listing or navigation has been verified. A later pinned
`yazi-phase1` gate passed the first-render marker with the existing
`SUNLIGHT_TEST_TIMEOUT=360` override. The gate was corrected to distinguish
the leader's `process_mark_finished` from a worker TID exiting normally;
it still rejects a leader exit or runtime panic and still requires the
observed application frame. This is a first-render smoke pass, not completed
directory navigation or clean Yazi exit. Unsupported
`inotify_init1(294)`, `socket(41)`, `prctl(157)` and `ppoll(271)` calls were
seen, but none has been demonstrated as the next hard blocker. Previously
validated `MADV_DONTNEED`/`MADV_FREE` still return `-ENOSYS`.

Phase 1.4 verification: `linux-open-largefile-probe` (open/read/F_GETFD/
F_GETFL/close, absent openat `ENOENT`, unsupported-bit rejection),
`helios-thread-probe`, `helios-io-probe` (pipe and socket recvfrom), stock
`yazi-baseline`, `helios-note-regression`, `helios-static-runtime` and the
pinned `yazi-phase1` first-render gate passed. The compat-linux host suite
passed 29 tests, a focused RAM filesystem chmod test passed, kernel package
`cargo check` passed and `git diff --check` was clean. The previously
documented duplicate panic-handler conflict still prevents claiming the
freestanding kernel's host test target passed.

### September 29 interaction regression: kernel stack usage

The one-arrow `yazi-phase1` smoke gate passes on the pre-fix kernel, but a
longer QMP keyboard run reproduced a kernel instruction-fetch fault twice
while repeatedly moving between `/` and `/root`. The second run stopped
after key 53. The saved trace is `target/yazi-navigation-baseline.log`, with
registers and the interrupted stack in
`target/yazi-navigation-baseline-registers.txt`. Both faults reported
`rip=0xffffffffffffffff`, `rsp=0xffffffff907b0e60`; the scheduler lock was
unavailable to the fault diagnostic (`pid=usize::MAX`). This is a kernel
fault, not proof that the unrelated unsupported socket or prctl calls caused
Yazi to fail. The supplied interactive log ends after cooked-mode restoration
without an exit status or fault, so it does not establish the identical cause.

Disassembly of the pre-fix test kernel establishes a concrete stack overflow:
`process::mmap::sys_munmap` reserves 31,128 bytes and calls
`RegionLedger::preflight_unmap`, which reserves another 10,312 bytes, before
counting saved registers and other callers. Task kernel stacks are 32 KiB.
The 256-entry staging images for unmap, fixed replacement and protection
changes were returned by value through several layers. These images now use
fallible heap storage; the committed ledger and its 256-region capacity remain
unchanged. Allocation failure maps to the existing capacity/ENOMEM error
before PTE mutation, and committing a staged image requires no allocation.
No Yazi source or payload changes are involved.

The rebuilt kernel reserves 1,208 bytes in `sys_munmap`, 168 in its staging
helper, and 264 in `sys_mprotect` (previously 20,712). The host memory suite
includes a plan-size guard against reintroducing capacity-sized stack images.

`tools/test_yazi_navigation.py` adds an extended test beyond the one-key smoke
check. It boots the smoke-test ISO, sends real keyboard events through QMP,
waits for actual directory changes, visits `/home/user`, and checks a requested
quit for leader status zero and cooked TTY restoration. Faults and unexpected
exits fail the test; serial logs, screenshots and failure registers are kept
in a new output directory. Build the `yazi-phase1` ISO first, then run:

```sh
python3 tools/test_yazi_navigation.py \
  --iso target/sunlightos.iso \
  --output-dir target/yazi-navigation-check
```

The MM-2E gate's expected shootdown text was stale: the kernel emits
`permission shootdowns acknowledged: online=4 remote=3: OK`. The expectation
now checks that exact four-CPU result; the underlying permission and remote
invalidation assertions are unchanged.

Verification at the local commit:

- Kernel package check, 35 host memory tests, and 29 compatibility host tests
  passed. Yazi one-arrow smoke, Helios Note, and MM-2D native unmap gates passed.
- The first MM-2E run completed all kernel assertions, including four-CPU
  shootdowns. Its saved serial log passes the corrected expected-message list.
  A subsequent VM rerun produced no serial output and timed out.
- `target/yazi-navigation-acceptance/serial.log` records all 30 confirmed
  parent/child cycles (60 directory changes) without the reproduced fault.
  The test subsequently reached `/home` but timed out selecting `/home/user`,
  so the **overall extended gate did not pass** and its quit check did not run.
- A separate two-CPU extended run timed out waiting for the first-render
  marker; no interactive acceptance is inferred from that run.
- The additional thread/I/O reruns were not reached after the MM-2E retry
  stopped the regression batch. Their earlier historical results above are
  not new verification of this change.
