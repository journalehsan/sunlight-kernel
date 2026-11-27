# VFS-C1A ordinary namespace publication

**PARTIAL overall.** VFS-C1A introduces authenticated pre-dispatch namespace publication behind
`vfs_c1a_candidate`. The native and capability adapters use the same
pending-child transaction. Production routing and bootstrap selection keep
their existing defaults. The ownership guard still reports the four C1
findings, so coordinated cutover remains **BLOCKED**.

## Launch audit

| Boundary | Native `sys_spawn` | Capability `SPAWN_TOKEN` |
| --- | --- | --- |
| Adapter admission | Copies path, argv and optional env, checks account allocation and resolves the ELF. Preserves environment, credentials, capability profile, TTY and explicit stdio. | Decodes the register IPC request and authenticates the launch, session attachment and supervision role. Uses the existing embedded image, default environment, entry arguments and per-binary roles. |
| Private construction | The adapter prepares the original image and leaves the initial streams intact during retain. Native PID return and caller-CPU placement follow publication. | The existing image preparation is shared with the legacy wrapper and returns a boxed child. Session bootstrap consumption, watcher and supervision setup precede the initial enqueue. The success reply includes PID and address-space generation. |
| Identity and visibility | `Process::try_new` assigns the process incarnation. Image loading establishes the final VM generation. The pending table reserves the PID alongside both values while the child is absent from the published process table and ready queues. | The same reservation and exact identity apply. The direct kernel wrapper used by a lifecycle observer remains a fixture path. Init, VFS and TTY still enter through their explicit bootstrap paths. |
| Previous boundary | Inserted the child, queued inherited retention and enqueued before the service acknowledged it. | Used the same asynchronous provisioning after insertion, followed by generic enqueue. |

The candidate captures inheritance from the exact launching image and pins its
current service receipt. The service derives a separate child binding using
the retained directory pair and keeps the source namespace ID. The new child
scope and receipt have their own generations. The adapter's copied cwd
agrees with that retained cwd. The retain path does not resolve a new root or
cwd by pathname.

The waiting syscall parks with one continuation in the existing transaction
row. The continuation check runs before another image can be built or an
authorization can be consumed. Scheduler, PMM, broker and VFS guards are
released before the parked caller yields. The receipt commit rechecks the
child ticket, incarnation and VM generation, the live parent and launch
attribution, the exact source receipt, the request and active service epoch.
It reserves scheduler storage before installing the binding. The adapters
finalize launch metadata and enqueue under the same scheduler guard.

On a lost continuation or failed commit, the transaction uses the existing
local reap and exact remote settlement. A delivered retain can require
compensation after local memory has been reclaimed. The PID stays reserved
until that remote obligation settles. The candidate cancels ordinary launches
whose exact caller retires. The independent retained-source fixture keeps
its detached transaction behavior. Native stdio snapshots are revalidated
at commit and wired only then. Service-owned descriptors cannot be passed
as cross-caller stdio under the current token ownership contract.

## L1C-3 prerequisites

| Prerequisite | C1A disposition |
| --- | --- |
| Unpublished PID and image, authenticated retain, binding commit, one scheduler owner | Integrated for both candidate adapters. The existing exact-generation and service-epoch checks govern publication. |
| Readiness and bootstrap | Init still services registrations before its authenticated root probe. The candidate launch fails through the native error or IPC ERROR path if it cannot derive an exact current binding. |
| Parent lifecycle and launch authority | The transaction already supports expiry, caller/endpoint retirement and local reclamation. Candidate publication requires a live exact caller. Detached publication and trusted successor activation remain **BLOCKED**. |
| Allocation and FD admission | The existing pending and remote capacities apply. Native stdio and the capability session authorization are rechecked before commit. General shared-table and cross-caller service description admission remain **BLOCKED**. |
| Broader platform acceptance | The one-executor contract remains in effect. Launch from a shared-process borrower needs an exact waiter and shared namespace owner contract. Production shell/session/manager workflows, late exec faults and interactive Btop and Qalculate were **NOT_RUN** for C1A. |

## Four-vCPU C1 evidence

The original three four-vCPU runs remain **FAIL** under the full verifier.
In the first, the exact D2A source Close was acknowledged and reduced the
service file count to zero. The required backend `->2` transition was
absent between D2A and the next enqueue. The old counter sampled retention
batches. A Close performed in the separate file drain could move the
backend count between those samples. This was incomplete evidence for the
required backend transition.

Both retries recorded D2B's normal Close with errno zero and zero service
file handles, but with `ack=0`. Exact owner retirement then saw zero
handles. The first rejected run also contains that sequence after the
earlier D2A rejection point. A normal Close is fire-and-forget. Caller
retirement can remove its publication before the server submits its reply.
That explains the acknowledgement gap and shows cleanup in those records.
The verifier correctly rejected the missing positive acknowledgement for
the normal-close case.

The qualification build samples actual backend handles around each file
dispatch and keeps the normal D2A and D2B callers alive while their Close
publication drains. The serial verifier still requires the acknowledgement
and every original lifecycle, allocation and backend assertion. The
caller-exit and endpoint-withdrawal cases still retire in flight.

A fresh four-vCPU run on the final image passed the full verifier.
One fresh four-vCPU attempt and one two-vCPU attempt still failed its
withdrawal ordering check. The caller can report its terminal result on
another CPU before the service logs backend settlement and its rejected
late reply. The serial record describes several events, so its position
also depends on service scheduling. The accepted run contains the required
order. A separate two-vCPU retry stopped at a D2A seek assertion after a
successful service write. Its cause remains unconfirmed.

## Evidence and remaining gates

| Gate | Status | Result |
| --- | --- | --- |
| Ordinary native and capability publication | **PASS** | 2 GiB on one, two and four vCPUs. Four successful launches and one rejected publication per adapter per qualified guest. |
| D1B, D2A and D2B | **PASS** | The original full verifiers passed on the same three qualified guests. Failed repetitions retain their own disposition. |
| Pending-child, parent and exec regressions | **PASS** | The registration suite passed 112 host tests. The separate L1C fixture passed five lifecycle cases on each CPU topology, including rollback, exact launch attribution and replacement exec. |
| N1P and caller-scoped capacity | **PASS** | Populated and unknown FSInfo each passed four native cycles at one vCPU and 2 GiB. Read-only image verification found the expected free-cluster change from 127004 to 127003. The host snapshot and capacity tests passed. |
| D1A and host regressions | **PASS** | All recorded host groups matched by case name. The filesystem, FAT, IPC, VFS, VirtIO, kernel, scheduler and FD description suites passed. |
| Release builds | **PASS** | Production kernel and VFS service, their compressed-resource configurations, and the opt-in candidate kernel compiled. |
| Ownership guard | **FAIL** | The unchanged inventory reports 71 references. Both enforced invocations exit 1 with the four C1 findings still live. |
| Ordinary boot acceptance and StorageCtl transfer | **BLOCKED** | Ordinary boot and transfer were **NOT_RUN**. |

The [results index](vfs-ordinary-namespace-publication-results.json) records
builds, guests, full verifier outputs, rejected attempts and preserved
historical evidence. The ordinary launch probe changes init's cwd to
`/etc`, waits for the previous cwd lease to retire, and uses both public
launch ABIs. It correlates the binding, first scheduler dispatch, first
probe syscall, relative service OPEN/READ, private mapping, exit and reap
by exact child identity. It injects a publication rejection on each
adapter after authenticated retain. Those attempts must settle the remote
obligation and leave no dispatch, child frames or service description
references. The [ordinary verifier](../../tools/check_vfs_c1a.py) enforces
these conditions separately from the complete D1B/D2A/D2B verifier.

Build the isolated guest with
`CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo build -p sunlight-kernel --features vfs_c1a_test`
and package the ELF with `tools/make_hybrid_iso.sh`. The results index
lists each QEMU argument file. Check the ordinary serial record with
`python3 tools/check_vfs_c1a.py <serial-log> <runner-exit-file>` and apply
the full C1 descriptor and mmap verifiers to the same run.

StorageCtl integration still requires a real registry extent and an
exclusive alias-fenced lease, a single block-capable ordinary service
namespace, and retained FAT directory/file identity across mutation and
withdrawal. ProcFS mountinfo, copied Linux statfs and runtime mount
publication must agree on the same attachment incarnation. Native and
Linux private mmap still need ordinary service descriptor and shared-owner
admission. The unchanged guard and ordinary boot acceptance remain
required once the live ownership dependencies have moved. `/vfs-scratch`
remains inactive.
