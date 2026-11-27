# VFS namespace consistency and migration readiness

2026-11-27. **Split authority confirmed; fresh guest validation blocked.**
Selected callers can resolve metadata and cwd in the service while creating,
opening and loading files through the kernel. A shared boot seed supplies
initial names to both trees; it does not synchronize live directory entries.
L1C-3 remains blocked and VFS-D1 remains deferred.

## Operation and mount ownership

K = kernel VFS; S = VFS service. Selection is per caller/profile, not a
mount-wide handoff. Virtual and bootstrap routes are explicit exceptions.

| Scope | Current authoritative implementation |
| --- | --- |
| Root seed and mutable root-backed paths (including `/tmp`, with packaged `/bin` and `/Applications` in the seed) | K mounts a seeded Tmpfs. S constructs its own RamFs from the seed, including the independently decoded compressed resource module when configured. Each answers for its own entries. |
| `/boot` | K may mount live FAT; S legacy BootFs serves a bounded kernel-prepared read-only share when provisioned. It is neither live FAT authority nor a coherent mirror. |
| `/state`, `/mnt`, runtime FAT targets | K owns partition selection, FAT/cache, capacity and mount publication. S has no equivalent live FAT mount contract; under an absent mount a seed directory is not the volume. |
| `/proc`, Linux `/dev/null`, pipes, sockets and memfds | Kernel virtual handlers or descriptor objects, outside ordinary mutable mount ownership. |
| Create/write, mkdir, rename/unlink, enumeration, access | Ordinary native and Linux dispatch uses K with path/credential policy; legacy S IPC can operate on its own tree within its supported contract. Directory enumeration is not a service-backed syscall stream. |
| Open and handle I/O | Ordinary open and O_CREAT use K. Explicitly selected copied file routing uses S and supports existing regular files; unsupported flags fail closed. Read/write, seek and fstat follow the installed descriptor backend and rights; path stat may disagree with fstat. |
| Path lookup/stat, chdir | Native StatPath and selected Linux pathname stat use authenticated S when metadata routing is selected; unselected callers use K. Selected chdir commits an authenticated directory pin and the shared cwd string; K operations may still resolve that string in K. Linux real-dirfd and virtual cases have separate dispatch constraints. |
| Executable discovery and image loading | Shell probes ordinary stat for slash-qualified executable discovery; native spawn/exec resolve embedded images first and then read K. Capability launch has an explicit embedded path. Linux dynamic loading and private mappings retain kernel consumers. A successful S stat does not authorize or supply image bytes to the loader. |

Sources: [kernel root and FAT initialization](../../kernel/src/main.rs#L2563),
[runtime storage](../../kernel/src/storage.rs#L102),
[service mount selection](../../services/vfs_server/src/main.rs#L388),
[open dispatch](../../kernel/src/arch/x86_64/syscall.rs#L5796),
[native stat](../../kernel/src/arch/x86_64/syscall.rs#L11085),
[Linux stat](../../kernel/src/arch/x86_64/syscall.rs#L10190),
[chdir](../../kernel/src/arch/x86_64/syscall.rs#L6374),
[native spawn resolution](../../kernel/src/arch/x86_64/syscall.rs#L4366)
and the [Helios operation matrix](../Helios_LinuxCompat/filesystem-namespace.md).

## Failure boundaries

- **Downloaded executable: namespace mismatch confirmed for metadata.** Fetch
  uses [native create](../../sunlight-fetch/src/downloader.rs#L263); the ordinary
  path writes through K. The archived create policy messages use a shared
  library prefix and do not establish S ownership. Both STACK-2A stat
  diagnostics show a parked S request and a validated errno 2 reply for the
  downloaded path after fetch exited zero: `eager-stat/serial.log:2100` and
  `demand-stat/serial.log:2097` under `target/stack-2a`. The original invocation
  has no fixture load; shell discovery can fail before native loading starts.
  Neither stored byte integrity nor direct loader behavior was established.
- **PTY marker: same ownership split.** The producer creates through K and the
  consumer's routed metadata lookup misses in S; ordinary open/read sees the
  record through K. The repaired [PTY fixture](../memory/tty-pty-functional-qualification.md)
  proves marker readability and the bounded PTY workload, not namespace
  coherence. Its repeated wait after a successful wait was a separate defect.
- **Git mkdir/chdir: same split, plus a distinct admission failure.** Linux
  mkdirat writes K; selected chdir traverses S and returns authenticated
  ENOENT for the new directory. Separate Git callers saw EAGAIN before delivery
  (`Prepare`); a later binding install supports a readiness race but does not
  distinguish absent from rejected binding at admission. See the
  [run4 analysis](../Helios_LinuxCompat/filesystem-namespace.md).
- **Packaged executable: unresolved.** In the STACK-2B short eager replay the
  stat request parks and receives validated success; metadata is mode 0755,
  size 9,304, and the normal foreground launch exits zero. The matched demand
  replay and isolated demand retry exit 1 from the stat utility with no
  corresponding parked request or validated reply. The initial longer
  sequence failed in both configurations. The utility
  [collapses errors](../../sunlight-utils/src/main.rs#L1208); missing publication
  cannot distinguish syscall validation, binding/admission error or another
  path before publication, and is not evidence of a service ENOENT. Capture
  raw syscall result, path/argument validation, exact caller and lease state,
  request publication and completion before changing policy. The historical
  session catalog timeout also lacks a first failing return code; fresh
  readiness success and a similar missing log do not identify a shared cause.
  See [STACK-2B](../memory/demand-stack-startup-readiness.md) and
  [STACK-2A](../memory/demand-stack-ring3-launch-closure.md).

## Readiness and lifecycle

Endpoint naming precedes mount construction, authenticated registration,
retained root/cwd installation and init's authenticated probe. Embedded init
must run the control plane before it can satisfy the namespace gate. Ordinary
native and capability spawn still insert/publish before retention settles.
Image identity (PID, incarnation and VM), registration epoch, credentials,
retained root/cwd and operation grants must match; cwd and credential changes,
exec and retirement affect different lifetime domains. Namespace withdrawal is
not descriptor close. Exact child rollback and remote receipt settlement are
separate, as are parent reply abandonment and child source-pin lifetime.

The [integration matrix](vfs-l1c-integration-readiness.md) is still blocked.
[Parent lifecycle](vfs-l1c-production-parent-lifecycle.md),
[exec lifecycle](vfs-l1c-production-exec-lifecycle.md) and
[file/IPC accounting](vfs-l1c-file-ipc-accounting.md) narrow component gaps;
the reported ordinary resource fixture waits for binding and therefore does
not demonstrate binding before first execution. Late exec failure, trusted
successor row reuse, detached launch authorization/revocation, remote races,
queued IPC and peer-held resources, service heap attribution, populated
transaction swap and interactive application gates remain open in their
respective scopes. Keep the historical 160 B residual unattributed and SWAP-1
separate. Qualify both ordinary adapters under one private reservation/commit
protocol, including exact cancellation, bounded drain, compatible errors and
rollback before exposure. No runtime replacement or detached publication.

[VFS-D1](vfs-l1.md) additionally needs descriptor epoch validation at lookup
and authenticated compensating Close after Open succeeds but descriptor
installation fails. Validate token ownership, reserved cleanup credit,
exact-once settlement, exit/exec and stale-epoch behavior before extending
dispatch. Neither a ready nameserver entry nor a reusable FD number grants
authority.

## Fresh checks and guard findings

`tools/audit_vfs_cutover.py --no-growth` and the unmodified `tools/test.sh`
exit 1 (71 references, five unmatched); the latter stops before build and
QEMU. Registration host suite: 107 passed, covering components rather than
guest file operation routing. Fresh matched native operations **NOT_RUN**.
Logs: `target/vfs-namespace-audit/{ownership,boot-gate,registration-host}.log`.

| Guard finding | Reassessment |
| --- | --- |
| `sys_linux_statfs`, syscall.rs:6309 | Live K capacity consumer. Requires authoritative mount capacity and equivalent error semantics. |
| `linux_procfs.rs:173` | Kernel virtual textual presentation reads K topology. Keep virtual presentation; source topology from the actual mount owner. |
| `main.rs:2567` | Baseline matches the old `Tmpfs::new(BOOT_INITRAMFS)` expression; current `Tmpfs::new(seed)` is the existing root constructor with a selected seed. Textual delta, no extra owner, but the existing owner remains real and the gate still fails. Preserve compressed seeding; review classification explicitly within inventory work. |
| `process/mmap.rs:272` | Private file mapping reads K positionally. Needs a pinned authoritative description and positional read with stable lifetime, rights and no shared-offset mutation. |
| `storage.rs:123` | Runtime FAT engine constructs and publishes under K. Requires one coherent storage handoff (device rights, cache, capacity, topology, sync and teardown), separate from the first root consistency slice. |

## Next milestone

First reproduce with an unchanged guard and readiness gates on qualifying
source. Use identical native fixture bytes and commands in fresh eager and
opt-in demand guests. In each, create a unique directory and known file via
ordinary calls, check parent enumeration, pathname stat and fstat, chdir and
relative lookup, then reopen and byte-compare from a second process. Pair an
immutable packaged executable with the same ELF delivered via localhost HTTP;
record byte count/content, mode, shell stat, and ordinary foreground resolution
and terminal exit separately. Pair selected and unselected control callers
without changing their normal admission. Record raw errors, caller/epoch,
mount/descriptor owner and first failed transition. Preserve historical logs.

The smallest proposed implementation slice is a **bounded, nonpersistent
mount with one authority**, using S and an existing service namespace. Build
and qualify the complete operation contract off the production launch path
first. Switch every exposed reader and writer for that mount together:
mutation, enumeration, cwd pins, metadata and descriptor I/O. For executable
surfaces require the loader and mapping consumers to use the same object
authority before enabling them; a data-only qualification mount must reject
execution consistently. Publish one mount generation, fail closed on retirement,
and prevent K and legacy IPC from reaching a competing backing tree at that
path. Do not selectively roll back one operation. This establishes a reusable
consistency boundary before changing the production root or FAT. No memory
policies or production routing changed in this audit; STACK-2 is unqualified.
