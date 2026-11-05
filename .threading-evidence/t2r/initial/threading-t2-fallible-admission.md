# T2: bounded, fallible thread creation

Status: **PARTIAL pending the explicit native race proofs below**. Native and
Linux thread admission, construction and publication are implemented. The
allocation/exhaustion, T1 and compatibility gates pass on the final implementation,
including allocation-free mapping cleanup and nonempty inherited authority.
The final machine-readable ledger is `.threading-evidence/t2/validation-summary.json`.
No commit or push is part of this work.

## Source identity and scope

The starting HEAD is `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`. The starting tree
was already substantially dirty, including completed T1 and unrelated FastLane
work. Its canonical source SHA-256 is
`42461a71ca634d63820c7b7db8a6ccc5b04aa8aeaf04e915c741edcc884c7e3a`.
`initial/source.json`, `initial/status.txt`, `initial/dirty.patch` and
`initial/source/` retain that identity and the original changed implementation
files. Canonical hashing sorts the SHA-256 map of tracked/untracked source files
and excludes `.threading-evidence/`; it is not a hash of HEAD alone.

Each command/guest attempt records its source map, exact argv, output, verdict,
timeout and, for guests, the frozen ISO hash and serial hash. Suite directories
retain their ELF and ISO. The final snapshot and T2-only patch are relative to
the initial dirty tree, preserving pre-existing changes. Changes to reports or
host-only tests do not establish new kernel coverage; source compatibility is
checked separately for compiled implementation inputs.

## Root cause and allocation inventory

Both native syscalls 22/158 and Linux raw clone reached `Process::new_thread`.
There was no worker admission limit. A 32 KiB `Box::new_uninit` kernel stack,
owned strings/collections and scheduler growth could invoke the fatal allocation
handler while the scheduler guard was held. `panic::alloc_error` disables
interrupts and halts, potentially retaining that guard and stopping other CPUs.
A userspace loop can reuse a static user stack/TLS region; it need not allocate
2 MiB of user memory for every accepted kernel stack.

| Site before publication | Old behavior | New behavior |
| --- | --- | --- |
| `process::new_kernel_stack` | Infallible 32 KiB Box | `try_kernel_stack`: null-checked raw allocation, in-place zero initialization, uniquely owned Box |
| `Process` metadata | Inline record plus growing task Vec | Inline record; fallible reservation to 512 task slots before construction |
| `EnvMap::inherit` | Infallible BTreeMap nodes and String clones | Sorted Vec, fallible entry reservation and individually fallible key/value copies |
| capability Vec | Infallible clone | Checked inherited budget, fallible exact reservation and Copy elements; no broker issuance or reference adoption |
| cwd | Infallible String clone and an unused default `/` allocation | Fallible inherited String; no default allocation in prepared construction |
| private FD inheritance (22) | Infallible Box, immediate backend/description retains | Fallible unretained snapshot; retains occur only at commit |
| shared FD paths (158/clone) | Infallible table plus three unused new open descriptions | Fallible empty placeholder; existing shared owner supplies the real table |
| task/queue publication | `add_process` could run an allocating reaper, grow task Vec and enqueue VecDeque | Reserve first; direct selected-slot insertion, without reaping or allocation |
| user stacks/TLS in libc | Fallible mmap, but rollback could allocate an unmap ledger image | Fallible mmap with native boundary-preserving flag; allocation-free whole-resource unmap |
| mmap journal, swap candidate capacity | Existing fallible Vec reservations | Retained; map failures cancel the unpublished region reservation and release installed pages |
| page tables/user frames | Existing checked PMM allocation and mapping rollback | Retained |
| unmap ledger plan | Fallible heap image even during cleanup | Small prevalidated edit; commit uses bounded in-place shifts, no heap or large stack temporary |

`ThreadResources` contains only already-owned values. Constructing a Process
from it allocates nothing. Empty IPC/deferred queues, signal arrays, FP state,
name, register frame and the checked shared-VM handle allocate nothing.
`Box::from_raw` assumes ownership of the successful allocation; it does not
allocate again. Capability mode and service restrictions are inherited; trusted
service provenance flags are not newly granted.

After the commit point, the creation syscall performs descriptor retains,
insertion, queue admission, diagnostics and returns the TID. None allocates.
Scheduling migrations/wakeups of admitted tasks use the reserved queues.
Subsequent unrelated syscalls and general resource retirement still contain
allocating operations, listed under limitations; they are not silently treated
as constructor allocations.

## Admission contract and heap budget

| Resource | Limit |
| --- | ---: |
| Unreaped workers per complete VM identity, excluding owner | 32 |
| Unreaped workers globally, native and Linux combined | 96 |
| Task slots reachable through userspace publication | 512 |
| Capacity reserved per CPU RR queue | 512 indices, all 64 supported CPU slots |
| BORE alternative | 512 indices per tier per CPU |
| Inherited data per child | 16 KiB including env String headers, capability bytes and cwd |
| Environment entries / capabilities | 128 / 256 |
| Heap headroom required after this transaction's conservative peak | 2 MiB |

A worker remains charged while Finished, physically pinned, or awaiting reaping.
Only Reaped returns its capacity. Counts are derived from retained Process
ownership; there is no separate counter or outstanding-ticket registry to leak
on failure. The existing scheduler guard spans reservation through publication.
Counts, lengths, inherited byte sums and TID allocation use checked arithmetic.
Native TIDs must fit u32 and Linux TIDs signed i32; overflow returns an error.
Publication incarnation allocation also cannot wrap.

The measured ordinary layout is 32,768 bytes per stack, 3,152 per Process and
10,240 per FD table. The fixed task Vec costs 1,613,824 bytes; RR queues cost
262,144 bytes for all 64 CPU slots. At 96 workers, stacks cost 3 MiB, FD storage
under 1 MiB, and inherited logical data at most 1.5 MiB plus conservatively
charged allocator rounding. Reaped FD boxes are bounded reusable slot storage;
per-thread environment, capabilities and cwd are actually dropped at reaping.

The heap check charges complete replacement task/queue buffers, current-thread
resources and short-string rounding before preserving 2 MiB. It is an admission
check, not a claim to reserve that memory against every other allocator user.
Fragmentation or concurrent independent allocation can still cause a checked
allocation to fail. Every such constructor allocation remains fallible.

The VM ledger already permits 64 address-space owners. The worker limit does
not consume those owner slots; 64 owners plus 96 workers fit well below the task
bound. Boot services start before untrusted applications. Already-live threads
are not killed to enforce a limit. One VM cannot consume the entire worker
budget. Limits accommodate the tested 8-worker FP probe, 4-worker concurrent
Hayro rendering and 32-worker std pressure fixture. They are maxima, not a
promise to accept a thread when its inherited data, VM mappings or physical
memory cannot be admitted. Rejections preserve the caller and return normally.

Native quota/allocation rejection is raw `u64::MAX-1`, decoded as
`Errno::Again`. Linux uses EAGAIN (11) for admission/cancellation/TID limits,
ENOMEM (12) for construction/storage failure, and EFAULT for invalid required
stores. No caller is terminated just because worker capacity is full.

## Commit, rollback and identity

1. Validate the native startup record or Linux clone flags/stack/required TID
   destination. Acquire the existing scheduler guard.
2. Reject an exiting/cancelled creator or unavailable exact VM owner. A Linux
   leader which has exited but retains the live group VM may still host new
   workers of that group; group teardown marks creators exiting and rejects.
3. Count unreaped ownership using the complete VM root/generation. Select only
   a Reaped, unwatched slot with no queue/current owner or physical handoff pin.
4. Check inherited sizes, heap headroom and publication incarnation. Fallibly
   reserve all task/queue storage. Bounded global capacity caches can remain
   after a cold failed reservation; they own no task and are not leaked tickets.
5. Clone owned state fallibly, allocate the private kernel stack, prepare all
   register/FP/FS state and establish exact local ownership. FD snapshots still
   own storage only, with no adopted references.
6. Execute the final injectable publication check. Linux's validated aligned
   parent-TID release store is the last fallible user operation. There is no
   fallible step after a successful store and no need to undo an exposed TID.
7. **Commit:** adopt FD references, install the complete Process in the selected
   slot, and enqueue it once, all under the same guard. Reserved capacity makes
   these operations allocation-free. The retained unreaped Process now owns
   the admission charge. Foreground parent-generation links qualify the PPID
   they actually name, including Linux's group-parent PPID convention.

Every earlier error drops all local owned data and the kernel stack, adopts no
FD/capability reference, writes no successful TID and publishes no runnable child.
Consuming an incarnation number on rejection is intentional monotonic history,
not a live reservation. No scheduler guard is dropped/reacquired in this
transaction, so exit, exec, cancellation, descriptor mutation and VM teardown
cannot change its borrowed source state between preparation and commit.

The existing native shared owner / Linux TGID remains retained until borrowers
are physically reaped; PID selection remains monotonic and non-wrapping.
Admission compares VM generations rather than adopting a reused root/slot.
T1's private supervisor stacks, per-CPU entry bridge, outgoing stack pins,
post-pivot child-TID completion and Reaped native join boundary remain intact.

## User-stack/TLS cleanup

Native `MAP_NATIVE_THREAD=0x40000000` applies only to fresh anonymous mappings.
It sets ledger `NO_MERGE` and preserves the stack/TLS allocation boundaries.
Linux and fixed-address combinations reject the extension. It changes neither
page permissions nor physical stack layout. This is not a guard-page/T7 fix.
The updated native libc requires this T2 kernel ABI; it does not silently fall
back to a cleanup contract an older kernel cannot supply.

A complete stack or TLS release cannot require an extra ledger record, even at
256 occupied records. Unmap preflight now validates a compact bounded edit;
commit revalidates under the existing guard and updates the fixed ledger in
place. One contiguous middle removal can split only one record, so its
preflight capacity proof also bounds the temporary layout. Full/prefix/suffix
removals never grow it. Existing policy, leaf-ownership and synchronous TLB
retirement checks remain. General arbitrary middle unmaps may still return a
recoverable ledger-capacity error; owned thread-resource cleanup does not need
that split unless userspace itself has invalidated its resource contract.
Existing mmap cursor advancement and consumed publication numbers need not be
reused on rejection; no live mapping, stack, descriptor or quota reservation is
retained merely to preserve those historical numbers.

Join/detach still wait for physical Reaped before stack/TLS release. OS-stage
Builder failures drop the ThreadInit and return the actual errno; plain
`std::thread::spawn` retains Rust's documented panic-on-creation-failure API.
No process-wide fatal fallback was added for quota rejection. General userspace
Box/Arc allocation before the OS stage still follows Rust's global OOM/abort
contract; the existing native runtime containment test checks that such an abort
exits its process/group rather than halting the kernel.

## Fault and exhaustion fixtures

`thread_admission_test` is debug-only and boots immutable embedded ring-3 ELF
fixtures. Commands require the kernel-registered PID, publication incarnation
and full VM identity; this is not name-based authorization. The module and
selectors are excluded from ordinary builds.

For each ABI, the fixture injects exactly 75 rejection points: task storage;
64 CPU queues; environment entry storage; four key/value strings; capabilities;
cwd; FD snapshot; private kernel stack; final publication. Parent environments
and broker-issued capabilities are nonempty. A warmed capacity baseline permits
exact heap/reference comparison after every failure. It checks no live-child or
stack increase, unchanged description references/capability holders, unique
queues, and no consumed TID store for rejected Linux clones. Cold cache growth
is separately bounded by the host reservation controls.

Three VMs then fill 32 workers apiece using real native 22/158 and Linux clone.
The next request fails; a fourth independent owner remains runnable and is
rejected by the global limit. Before release, fresh worker pulses must establish
useful work on another CPU on SMP. Every cycle drains physical reaping before
checking restored capacity and exact heap equality. Loops and guest deadlines
are bounded. No host resource protection is disabled.

Host tests also make the real allocator return null for owned constructor
allocations and all allocations during unmap. Full-ledger boundary-preserving
release succeeds with allocation denied. Native std fills the real mapping
ledger, leaves one slot for a worker stack and forces TLS mapping failure. It
requires ENOMEM and proves the stack slot is reusable, three times. Separate
32-worker std cycles require EAGAIN and join/detach/TLS recovery. Four actual
native std workers independently open and render Hayro pages and join.

Strict parser negatives remove each required receipt and reject missing or
duplicate points, wrong stages, heap drift, absent remote work, CPU mismatch,
partial verdicts and panic output. Synthetic parser/model logs are never native
proof.

## Concurrent lifecycle proof boundary

The host contention harness compiles the production reservation/publication
methods and real borrower/exec predicate. OS threads contend on lifecycle
serialization for the last global and per-VM slot; exactly one succeeds. It
also races cancellation, parent exit, unavailable owner/VM generation and exec
eligibility, tests pinned/watched slot reuse and Finished-versus-Reaped charging,
and rejects every reservation/publication prefix. Allocation and descriptor
ownership are independently exercised with production helpers.

Hardware context selection, pins and lifecycle mutations in that harness are
modeled. Native pressure, T1 lifecycle, pin-release, IPC/exec and Doorbell gates
exercise the real machinery, but they do **not** force and individually identify
all eight requested creation/exit interleavings. In particular, no accepted
receipt proves two distinct ring-3 creators competing for the final slot while
another CPU performs parent/VM retirement or cancellation at the preparation
boundary. This is why the report remains PARTIAL rather than promoting host
models to complete native proof.

The smallest remaining executable proof is a bounded 2-CPU ring-3 race fixture
using the real admission syscalls and real lifecycle operations, with receipts
for final-global-slot, final-VM-slot, parent exit, VM teardown, permitted exec,
immediate child exit, concurrent reap/reuse and pre-publication cancellation.
Each must identify the winning serialization and exactly one terminal owner.
Where concurrent exec is prohibited by the thread model, an attested native
rejection/lifetime proof is appropriate rather than forcing an unsupported
execution model. Run its pilot before expanding to 1/2/4/12 CPUs. The executable coverage gate reports these missing receipts explicitly:
`python3 tools/threading_t2_acceptance.py .threading-evidence/t2/validation-summary.json`.
Its expected nonzero result is PARTIAL; it is not promoted to a PASS.
Passing unrelated suites cannot discharge that gap.

## Validation, preserved failures and remaining allocations

The machine ledger links exact final commands, guest rows and source identities.
Native coverage comprises admission/fault/exhaustion, T1 hostile RSP and Linux
completion reuse, authenticated blocking IPC/authority/SHM/exec retirement,
original Linux clone/futex/FD/TLS lifecycle, native FP/parser/raster, native std
and actual Hayro, R2D handoff/remote wake and Doorbell continuation. Ordinary
no-feature production release boot and supervisor restoration are recorded.
Clippy and formatting baseline failures are classified separately from T2.

| Final native gate (QEMU TCG) | 1 CPU | 2 CPUs | 4 CPUs | 12 CPUs |
| --- | --- | --- | --- | --- |
| 75 fault points per ABI; 96-worker exhaustion and three exact recoveries | PASS | PASS | PASS | PASS |
| T1 hostile RSP, sibling corruption, Linux completion/reuse, lifecycle | PASS | PASS | PASS | PASS |
| Authenticated blocking IPC, authority/SHM/VM retirement, exec/exit | PASS | PASS | PASS | PASS |
| Original Linux clone/futex/shared FD/TLS lifecycle | PASS | PASS | PASS | PASS |
| Nine-context x87/XMM/MXCSR isolation and parser/raster | PASS | PASS | PASS | PASS |
| std quota, full-ledger TLS failure rollback, join/detach/TLS, four Hayro render workers | PASS | PASS | PASS | PASS |
| R2D remote wake, delayed physical handoff and reaping | PASS | PASS | PASS | PASS |
| Native Doorbell CPL0 continuation and lifecycle | PASS | PASS | PASS | PASS |

Single-CPU passes are not cross-CPU interleaving proof. Individual Doorbell
FORCED/NOT_FORCED classifications and all R2D assertions remain in their
receipts; the table reports overall gate results. None fills the named native
admission-race proof gap. These are functional emulated-native tests, not
physical-hardware/topology certification or performance measurements.

With real inherited authority, the 2-CPU pressure heap is 6,199,296 bytes
(free: 10,577,920 of 16,777,216). All three reaped-worker baselines are exactly
3,035,136 bytes. The difference is exactly 96 × 32,960 bytes: per worker,
32,768 stack + 160 environment + 16 capability + 16 cwd bytes, including the
allocator's short-allocation rounding. FD boxes are retained bounded slot
storage already included in the reaped baseline. Other CPU rows have their own exact per-cycle baselines in the
JSON ledger. The preserved earlier empty-capability baseline was 3,032,704;
that source-compatible historical number is not substituted for the final
fixture's measurements.

Host results: 171 kernel library tests; 12 production-admission host tests in
both RR and BORE reservation modes; 8 real-allocator/unmap property controls;
16 descriptor tests; 29 unchanged Linux library tests; 2 focused native thread
API tests; 4 native mapping API tests; and 7 strict parser/coverage controls.
The unmap property control checks 1,584 bounded range/pattern combinations with
all heap allocation denied. BORE reservation paths have host coverage; the
native matrix uses the existing production RR scheduler.

Bare-metal checking, ordinary production debug/release builds, emitted T1 ABI
checks, fixture-exclusion scans and a 2-CPU ordinary release boot pass. Boot
requires Foundation Complete, sunlightd, login input and timezone readiness.
The ordinary supervisor is rebuilt after document-service tests.

Strict Cargo Clippy fails in existing dependency diagnostics. The captured,
actual production-kernel compiler invocation run through clippy-driver reports
155 existing diagnostics and zero on T2's new/changed spans. No suppression is
added. A preliminary pass caught two new queue-iteration lints and a redundant
borrow; they were corrected and native gates repeated. T2 formatting checks
pass; whole-workspace rustfmt and `git diff --check` retain pre-existing failures
(the latter is the sunlight-uac manifest's trailing blank line). Formatting-only
normalization has identical complete rustfmt output before/after, retained in
`format-normalization/equivalence.json`.

The two standalone test/probe lockfiles refreshed local package versions during
builds. Generated copies are preserved under `generated-locks/`; repository
files are restored to their exact initial hashes. Root Cargo.lock and version
state retain their initial contents.

Preserved failed/aborted attempts include: native fixture using the Linux bridge
syscall tag; a progress assertion before secondary CPUs had a chance to run;
an outdated host T1 validator stub; a misplaced runner `--timeout` option; the
full libc host suite hanging in existing raw-syscall secret-store tests; and
initial allocator-harness API-name mistakes. Their logs remain failures.
Intermediate pre-cleanup matrices remain historical; the final matrix includes
the mapping boundary and allocation-free unmap implementation.

Infallible paths still outside construction include ordinary process/boot
`Process::try_new`'s older stack/FD/default-env allocations, generic EnvMap
mutation/inheritance/serialization, ordinary FD open-description growth,
exit-group PID collection, endpoint-owned-vector retirement, and unrelated IPC
queues/other kernel users. Trusted boot MM-0 self-tests and opt-in prebuilt scheduler fixtures still use
infallible `Process::new_thread`; userspace thread syscalls do not. T2 is not a
claim of kernel-wide allocator exhaustion containment. The explicit headroom
also cannot reserve allocator capacity against all other kernel users.

Unsafe review covers null-checked heap ownership conversion, full in-place FD
initialization and the existing private initial context/FP setup. No new global
production lock, scheduler policy, capability authorization bypass, user-memory
trust, IRQ handoff rule or FastLane route is introduced. The unmap edit stays
within the fixed 256-record ledger and holds the original lifecycle guards.

T3 APIC topology mapping, T4 Linux futex comparison atomicity, T5 robust-list
owner-death handling, T6 retained waitpid receipts and T7 user stack guard pages
remain open. T1's completed protections are preserved, including its atomic
child-TID completion portion of T4.

## Reproduction

From the repository root, use a fresh attempt name/evidence directory each time:

```sh
python3 tools/threading_t2_validate.py command admission-host -- python3 tools/threading_t2_host.py
python3 tools/threading_t2_validate.py command allocation-host -- python3 tools/threading_t2_allocation_host.py
python3 tools/threading_t2_validate.py command kernel-host -- cargo test -p sunlight-kernel --lib --target x86_64-unknown-linux-gnu --offline
python3 tools/threading_t2_validate.py command parser -- python3 -m unittest discover -s tools -p 'test_threading_t*.py'
python3 tools/threading_t2_regressions.py admission
python3 tools/threading_t2_regressions.py t1
python3 tools/threading_t2_regressions.py auth
python3 tools/threading_t2_regressions.py linux
python3 tools/threading_t2_regressions.py fp
python3 tools/threading_t2_regressions.py std
python3 tools/threading_t2_regressions.py production --cpus 2
```

The admission runner requires a passing 2-CPU pilot before SMP expansion.
Subcommands with an explicit command timeout place `--timeout` before the
attempt name. Guest images are frozen immediately after each feature build;
do not build different kernels concurrently into the shared Cargo output path.
The ordinary supervisor must be restored after document-service fixture builds.
