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

The fix is confined to bulk process-exit cleanup. It consumes an entry whose
ordinary open-description row is already absent without attempting a second
description or backend release. The strict `release_description` assertion
remains unchanged for every normal ownership path. A focused fd-table
regression constructs the observed fd-0/fd-24 alias and checks that the first
entry owns the release, the stale alias returns no backend handle, and the
table is empty. The QEMU harness now also requires exactly one PID 39002 reap
and rejects any kernel panic.

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
