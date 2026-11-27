# VFS-D1B: Native service-owned descriptor lifecycle

**PASS in the isolated single-vCPU QEMU fixture (November 27, 2026).**
Three native callers installed service-owned FDs and completed the
descriptor workload through Ring-3 syscalls. One caller exited with
256 live references to one service description. Two callers exec'd,
checked the inherited descriptors and exited with both duplicates open.
Every cycle settled back to the initial resource counts.

| Gate | Result | Native evidence |
| --- | --- | --- |
| Authenticated caller readiness | PASS | The retained-child publication installed the service receipt before scheduler enqueue. The first caller syscall matched the entire receipt, including registration, PID, process incarnation, address-space generation, namespace ID and namespace generation. |
| Descriptor installation and I/O | PASS | The native OPEN returned a usable FD. READ, WRITE, SEEK, DUP and CLOSE exercised the installed row. The caller wrote `HELLO`, read it back, restored `hello` and checked the payload through the duplicate. |
| Shared cursor and positional reads | PASS | Three copied PREAD requests used explicit offsets through the isolated syscall. Reads through both duplicates left the shared cursor at byte five. Ordinary reads and writes through either duplicate used the common cursor. |
| Final Close | PASS | The first close left the duplicate readable and the service handle live. The last close produced one acknowledged Close for that token. The service count reached zero before the next OPEN. |
| Full-table installation failure | PASS | Native DUP2 filled 256 slots with references to one row. The next service OPEN returned a new token, then kernel installation returned errno 24. Before and after failure, the table held 256 slots, one service row and 256 references. A subsequent native read still used the original token and cursor. |
| Compensation while uncertain | PASS | The compensating Close reused the successful OPEN's request ID and matched its original PID, incarnation, image generation, token and epoch. The fixture withheld the first acknowledgement after the service closed the handle. The kernel kept the publication charged, and the exact retry received an acknowledged BadHandle. The service count went from one to two to one and stayed at one on the retry. |
| Exit and exec | PASS | Ordinary exit retired the exact owner of the remaining description. Each exec dropped an independently opened CLOEXEC description and transferred one surviving token for two descriptors to the replacement image. The new image immediately observed EBADF on the CLOEXEC FD and read successive bytes through both survivors. Old-image retirement reduced service handles from two to one. Replacement exit reduced them from one to zero. |

The initial native smoke run was **PARTIAL** and covered readiness and
basic I/O. The first full run **FAILED** final Close: consuming a non-final
duplicate decremented the generic FD description but skipped the service
row because the decrement was behind a short-circuit test. Explicit close
now decrements both references for each consumed service FD and uses the
service row's final release to claim remote Close. The FD-table host
regression covers the first release and the final release. The final native
run observed the remote Close before retirement on all three cycles.
An intervening QEMU invocation with an invalid USB controller identifier
did not start the guest.

## Resource accounting

| Resource | Before each cycle | Live | After each cycle |
| --- | ---: | ---: | ---: |
| Kernel generic descriptions / references | 72 / 72 | Full table with 256 caller slots | 72 / 72 |
| Kernel service rows / references | 0 / 0 | 1 / 256 at saturation, 2 / 3 before exec, 1 / 2 after exec | 0 / 0 |
| Service file handles | 0 | 1 for a duplicated description, 2 during failed installation or before exec | 0 |
| Service backend open handles | 2 | Peak 4, falling to 3 after compensation | 2 |
| Frames charged to caller PID | Caller not yet published | 535 at saturation and on the exec replacement samples | 0 |

Init consumed each exit status once. The settled checkpoint waited for
reap and for the caller's VFS publications, lifecycle barriers and retained
namespace releases to drain. The reaped record had an empty FD table,
reclaimed address space and no kernel stack. Service counts come from the
file table at dispatch and exact-owner retirement. Backend counts come
from the service's shared namespace VFS around its control poll. A poll
can batch the final Close and subsequent OPEN, so the per-request trace
provides the finer-grained service-handle evidence.

## Scope and remaining gates

The fixture uses a debug kernel with `vfs_d1b_test`, the existing
private retained-child publication path and a disposable boot. Only the
fixture selects copied file routing. It uses the existing service table,
FD table, file continuation and retirement paths. The test-only PREAD
entry uses existing retained positional transport and releases the
scheduler and broker locks before waiting. Production namespace authority,
runtime FAT ownership and default stack policy retain their existing
configuration.

The host runs matched all 481 baseline, 59 N1H, seven N1D, three N1L, 14 FAT
and 522 D1A tests by name. The FD-table harness passed 19 tests. N1N
split-phase and unsettled withdrawal passed. The N1P native FAT32 and
closed-image evidence remains valid from its recorded disposable runs.
A new N1P native run was **NOT_RUN**. Production kernel and release VFS
service compilation passed.

Native compensation across a full file timeout, requester retirement and
registration replacement was **NOT_RUN**. D1A's host cases preserve those
retry and retirement contracts. Cross-caller ownership across threads or
fork and successor handoff remain unqualified. Both ordinary launch
adapters still need authenticated pre-dispatch namespace publication
before a production cutover. General VFS-D1 routing, broad L1C-3, mmap and
`/vfs-scratch` remain **BLOCKED** at their existing gates. The unchanged
ownership guard **FAILS** with 71 references and four unmatched live sites
in statfs, ProcFS mountinfo, private mmap and runtime FAT mount. Ordinary
production boot remains blocked by that guard.

## Reproduction

Build the isolated kernel with
`CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo build -p sunlight-kernel --features vfs_d1b_test`.
Pass the resulting debug ELF to `tools/make_hybrid_iso.sh` and boot the
ISO with the recorded single-vCPU TCG arguments. The run uses no guest
disk. The completion marker follows three reaps and VFS settlements.
Match the serial receipts and operation traces against the recorded
counts to verify the full gate.

The [machine-readable results](vfs-native-descriptor-lifecycle-results.json)
include each caller's identity, serial line numbers, resource counts,
build logs and artifact hashes.
