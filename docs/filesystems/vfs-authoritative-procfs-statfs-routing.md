# VFS-C1E6: authoritative ProcFS mountinfo and Linux statfs

The opt-in C1E2 candidate routes ordinary Linux `/proc/self/mountinfo`
open and `statfs(2)` for callers selected for copied VFS file routing.
The routed operations require a current retained namespace receipt. The
service validates that receipt against the current registration and caller
binding, then uses the retained root and working directory directly.
Neither operation borrows the kernel pathname result for a service lookup.
Legacy callers keep their existing topology and FAT capacity paths.

Mountinfo samples the attachment graph under the service owner's lock.
The formatter uses the same epoch-qualified attachment IDs as StorageCtl,
including the parent ID. It escapes paths using the existing mountinfo
escaping rules. The closed publication gate keeps FAT preparations out of
the graph. Opening the file installs immutable bytes in a ProcFS description;
read, seek, dup and close continue using the existing descriptor operations.

Statfs resolves the supplied path from the retained root and directory and
reads FAT capacity under the same service VFS lock. The reply includes the
revision and exact mount incarnation. The kernel uses the existing 120-byte
Linux FAT layout and validates the destination before copying. The current
service capacity primitive supports device FAT only, so service tmpfs
returns EOPNOTSUPP. A shared Linux file-table worker without its own
supported retained view fails closed.

The copy transport correlates replies by operation, registration, caller
and request ID. On resume the kernel checks the registration, retained
receipt, credentials, file-table owner and copied pathname again. The
service loop has only one runtime topology mutation available to it:
withdrawal. Pulling that operation invalidates completed observations before
the service can mutate the graph. The scheduler guard serializes this pull
against installation of ProcFS bytes and statfs copy-back. No guard is held
across the parked wait. This topology fence depends on the closed
publication gate and the single service request loop; any new topology
writer must participate before publication can open.

The ownership guard remains closed. All four historical findings are
still live: ProcFS kernel topology and Linux statfs kernel backend access
for legacy callers, private mmap kernel backend access, and runtime FAT
construction. No baseline was changed.

See the [validation results](vfs-authoritative-procfs-statfs-routing-results.json)
for the precise evidence and remaining gates. C1E6A reproduced the archived
failure with the original kernel and fixture. The Linux probe exited with
status zero, then bulk exit cleanup retired fd 0's open-description row and
encountered fd 24, a stale alias of that row without an independent reference.
`FdTable::take_all_handles` unconditionally called `release_description` for
both entries, so the second call reached `expect("live description")` after
the first had removed the row. Normal close, dup, exec and backend-release
assertions were not involved.

C1E6B traced the alias to VFS registration, not descriptor duplication. The
fixture's raw Linux syscall 24 is `sched_yield`; the compatibility table maps
it to native `ProcessYield`, which never reads or writes the fd table. A QEMU
hardware watchpoint on the empty fd-24 slot instead fired in
`Ownership<64>::new` while it initialized its 6.5 KiB entries array. The
out-of-line constructor built that array in a temporary below the 32 KiB
kernel stack and its initialization overlapped the adjacent heap-allocated fd
table. The observed byte store changed fd 24's `Option<FileDescriptor>` tag
from empty to present while leaving a zero-filled payload: handle 0 and
open-description row 0, exactly matching fd 0. No `open_from`, `dup`, `dup2`,
spawn inheritance or thread snapshot operation ran, so neither
`retain_description` nor any backend retain could account for the alias.

The descriptor contract remains unchanged: every installed descriptor owns
one open-description reference; `dup`, `dup2`, spawn inheritance and committed
thread snapshots explicitly retain it; each close or retirement drops one;
only the final VFS reference returns a backend-release handle. C1E6B makes the
smallest source correction by forcing `Ownership::new` inline, so its array is
initialized directly in the caller's `RemoteRegistration` destination instead
of materializing the oversized stack temporary. This is an upstream memory
clobber correction, not a new descriptor ownership rule.

The C1E6A defense remains confined to bulk process-exit cleanup. It consumes
an entry whose ordinary open-description row is already absent without
attempting a second description or backend release. The strict
`release_description` assertion remains unchanged for every normal ownership
path. A focused fd-table regression constructs the observed fd-0/fd-24 alias
and checks that the first entry owns the release, the stale alias returns no
backend handle, and the table is empty. The native qualification separately
exercises an ordinary open/final close, a valid duplicate with shared offset
and surviving lifetime, `dup2` to fd 24, bulk retirement, exactly one final
backend release, an empty second retirement, and restoration of the exact
description-row/reference baseline. The harness rejects any stale-alias
diagnostic in the real fixture, requires exactly one PID 39002 reap, and
rejects any kernel panic.

The repaired single-vCPU, 2 GiB run at
`target/vfs-n1o-qemu-644nz9cp/serial.log` passed the original C1E6 scenario.
PID 39002 reaped with 22,299 user frames, 50 page-table frames, zero swap
blocks and zero IPC entries. Five explicit preparation releases and one exact
service retirement logged zero pending commands and mapping users with the
same 20 KiB DMA allocation. The harness continued through read-only fsck and
payload recovery; the closed FAT volume was byte-identical to the initial
volume and retained `KEEP.TXT`. A second repaired run at
`target/vfs-n1o-qemu-p2pd4p85/serial.log` produced the same result. Direct
host execution of the kernel unit-test binary remains unqualified because the
existing host target fails on unrelated private `ProcessState` imports and
duplicate `alloc_error_handler`/`panic_impl` items. Successful committed FAT
capacity and withdrawal tests through ordinary syscalls require publication
and are blocked.

The final C1E6B single-vCPU, 2 GiB run at
`target/vfs-n1o-qemu-rawhijfk/serial.log` passed the expanded qualification.
It emitted the descriptor-retirement attestation once, emitted no stale-alias
fallback, reaped PID 39002 with the same 22,299/50/0/0 resource counts, and
continued through five preparation releases, exact service-image retirement,
the C1E2 completion attestation, read-only fsck and payload recovery. SHA-256
for both the initial and closed FAT images was
`74cbf319149a80bab105c87cf43e1e7158f15da99f877db9515d299a838cf01e`.
