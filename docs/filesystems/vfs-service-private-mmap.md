# VFS-D2A service-owned private file-backed mmap

VFS-D2A adds an isolated native path for eagerly populating private mappings
from service-owned regular-file descriptions. The isolated Ring-3 matrix
passed on one, two, and four vCPUs. Overall qualification is **PARTIAL**
because live retirement during a pending native mapping remains untested.
The historical Linux-compatible private mmap still reads from the kernel
backend. The unchanged ownership
guard continues to report that live dependency.

## Admission, population, and commit

The debug-only `vfs_d2a_test` feature extends D1B's authenticated pre-dispatch
fixture. The native mmap syscall selects the route only for the fixture's
published, file-routed native caller. Admission takes the scheduler and
capability broker locks, checks the retained namespace receipt and live
registration, and verifies that the descriptor grants READ and belongs to
that service epoch. A transaction holds a reference to the same kernel
service-description row and uses its exact caller, epoch, and token for
every read. The service's open description pins the backend handle.

Admission checks flags, the W^X protection rules, rounded length, file-offset
alignment and overflow, virtual range, and placement. The existing region
ledger preflight checks available ledger capacity or replacement policy. For a
nonreplacement mapping, the preliminary ledger reservation is cancelled
before publishing IPC. The fixture requires a private address space with one
executor. It records the candidate base and exact image identity and takes a
fresh ledger reservation when installing the map. Failed admission leaves the
cursor and ledger unchanged.

The continuation stages bytes in a zero-initialized kernel buffer, bounded
to 1 MiB per mapping, with one outstanding fixture transaction. Each copied
PREAD asks for at most 4096 bytes at an explicit file offset. A short reply advances by its actual size and a zero
reply ends population. The scheduler and broker serialize publishing and
parking, then both locks are released before the syscall yields. The post-SYSRET
park interrupt ensures dispatch can restore the Running state before replay
consumes a completion and publishes the next chunk. PMM,
topology, and kernel VFS locks are absent from this IPC phase. The service
PREAD operates on the retained description without a pathname lookup or a
shared cursor update.

Replay matches the complete syscall arguments and exact caller identity. A
live epoch and the placement preflight are checked again before installation
while lifecycle serialization remains held. The eager mapper allocates and
accounts private frames under the scheduler and PMM locks. It copies the
staged bytes into those frames and installs leaves with their final
protection. The single executor is in the syscall until commit; no executor
can observe the intermediate leaves. Fixed replacement uses the existing
ownership and TLB transaction. Successful publication updates the mapping
ledger and, for an automatically placed range, the mmap cursor.

A read failure drops the staging buffer before any private frame or page
table is allocated. Installation failure rolls back provisional leaves,
private frames, and the fresh ledger reservation. Fixed replacement records
newly attached tables and retires them after restoring the prior leaves and
resolving the fixed transaction. Successful eager mappings own their private
frames independently of the source token. Dropping the transient pin after
success or failure publishes the existing exact Close if it was the final
reference. Exact image retirement discards outstanding staging and lets the
existing file-owner barrier dispose of the retired image's tokens. Stale
replay abandons only the matching PREAD publication.

## Mapping contract

- The mapper copies through the rounded span and zero-fills bytes beyond EOF,
  including pages wholly beyond EOF. Linux can signal SIGBUS on access to
  such pages. The copied snapshot also does not track later file changes.
- The historical eager mapper samples size with FSTAT before copying. The
  service route terminates on zero PREAD and takes no whole-file snapshot
  lock. Concurrent file mutation can change what later chunks contain.
- Private writes affect the copied frames. Existing protection rules reject
  writable-executable and write-only or execute-only requests. PROT_NONE
  produces supervisor-only NX leaves. Unsupported sharing modes fail
  admission.
- Nonfixed hints are ignored, as on the historical file-backed route.
  Protected ranges remain protected. The 1 MiB staging limit and single
  executor are fixture boundaries.

## Qualification

The QEMU caller made actual syscall 50 calls and touched the mapped pages
in Ring 3. Each of the one-, two-, and four-vCPU runs passed the same
assertions. It checked a 4105-byte file in an 8192-byte mapping,
an aligned nonzero file offset, zero padding, private writes and unchanged
file bytes, cursor preservation, exact placement and replacement, and source
descriptor closure. Kernel checkpoints checked RW/NX, RO/NX, RX, and
supervisor-only PROT_NONE leaves and no-replace collisions. Four ordinary
map/unmap cycles returned to 537 owned frames, four regions, and one
description row and reference while the source FD remained open. At exit
the caller held one remaining private page and had released its source
descriptor. Reap recorded zero owned frames and service references, with
the service backend handle count back to two.

Six Ring-3 rollback calls exercised an injected read failure after a full
page of staging, initial frame exhaustion, first page-table allocation
failure, an injected stale epoch at commit, partial fixed table allocation,
and allocation failure after the first leaf was installed. The fixture
checked both leaves were absent and owned frame counts returned to baseline.
The installer also compared free frames, owned frames, region count, and
mmap cursor while still holding the scheduler and PMM locks. The stale-epoch
injection exercises the commit check. Live endpoint withdrawal or caller
exec during a native mmap remain unqualified.

Focused host tests cover bounded reply assembly and EOF, positional reads,
exact cancellation and late-reply rejection, and descriptor final release.
The registration host harness tests copied transport state using host
scheduler records. Ring-3 QEMU supplies the actual page table and frame
evidence. The first SMP controls exposed a Ready-before-park race. The
post-SYSRET park and replay state check eliminated the reproduced Busy
failure in the final two- and four-vCPU controls. The results include
serial receipts, resource counts, and regression evidence.
[Machine-readable results](vfs-service-private-mmap-results.json)

## Cutover prerequisites

Test real service retirement and image retirement during a blocked mmap.
Ordinary launch adapters still need
authenticated pre-dispatch publication and shared-table callers need an
ownership handoff and an address-space commit protocol. Coordinate removal
of the historical private mmap dependency with the other remaining guard
findings before ordinary production boot. D3 still requires native
reclaim/fault restoration evidence for a modified service-populated private
page.

## Reproduction

Build with
`CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo build -p sunlight-kernel --features vfs_d2a_test`.
Use `tools/make_hybrid_iso.sh` on the resulting debug kernel and boot the
diskless image with the recorded TCG arguments. The first three callers
exercise the original D1B cycles. The fourth caller runs D2A. The terminal
`[VFS-D1B] complete` marker follows all four settlements.
