# T3R: native topology and thread migration verification

Status: **T3R PARTIAL — migration verified; nonzero-BSP hardware proof unavailable**.

The migration fixture is IMPLEMENTED and passes native QEMU TCG execution on
2/4/12 CPUs. Genuine nonzero physical BSP and late firmware-enumerated BSP
proofs are **BLOCKED / NOT_RUN**. Host topology models do not close either gap.
T1/T2/T3 evidence and failed attempts remain under their original evidence roots.
T2R stays PARTIAL. T4–T7 and FastLane production status are unchanged. No commit,
push, privileged disk operation, or `cargo clean` was performed.

## Initial source identity and preservation

HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
Initial canonical SHA-256:
`83e4156e1d80e559b2c4a2cafe1500889517c299d01bd6eee95dadc5e5cc7067`.
This exactly equals `.threading-evidence/t3/final/source.json`: no changed,
missing or added canonical source paths at entry. The existing dirty tree
includes T1/T2/T2R, IPC, version/manifests and other work; it was not reset.

`.threading-evidence/t3r/initial/` contains the full initial hash inventory,
HEAD comparison, dirty status/binary patch, and exact kernel/tool/doc copies.
`implementation.patch` isolates T3R changes against that dirty source, and
`final/` records final hashes and status. Canonical hashing follows T3: sorted
JSON of SHA-256 hashes of tracked/unignored source, excluding the evidence root.
Command attempts preserve source hashes, exact argv, output and exit status.
Native attempts preserve serial, ISO hash, CPU count and validator verdict.
Frozen kernels/ISOs are retained; later source edits never silently qualify old
binaries as current.

## Independent audit

| Path | Finding / contract |
| --- | --- |
| Firmware enumeration | Limine `mp.cpus()` order and `processor_id`/`lapic_id` are distinct from logical indices. The fixture records original slot order before normalization. |
| BSP and logical zero | Exact `bsp_lapic_id` lookup assigns logical zero even when its input position is later; missing/duplicate/unaddressable IDs fail. CPUID is checked against firmware, then the live LAPIC against the map. |
| Physical APIC lookup | Immutable `spin::Once<CpuTopology>` is published before CPU-local setup. Dense logical IDs are translated only at hardware boundaries. Online readiness is a separate monotonic bitmap. |
| CPU-local initialization | Current logical CPU derives from validated CPUID APIC identity. AP `extra_argument`, startup slot, GDT/TSS and GS entry slot resolve through that identity. |
| TSS/RSP0/IST | BSP uses its global TSS; AP logical `n` uses validated slot `n-1`. The fixture reads STR/SGDT and decodes the actually loaded TSS, comparing RSP0 and both IST pointers with boot ownership receipts. It also checks its live kernel RSP lies in the current task's allocation. |
| Scheduler selection | Ready placement uses existing affinity and least-loaded eligible selection. No fairness, quantum, stealing, priority, load balancing or migration policy changed. |
| Remote wake/handoff | Native futex compare/register/wake serializes under the existing scheduler guard and exact VM identity. The fixture waits for a real blocked, unowned, unpinned child before setting its affinity. Ordinary WAKE chooses CPU 1, publishes Ready and requests rescheduling. The production physical stack pin protocol remains unchanged. |
| TLS and FP | Switch-out saves FS_BASE and FXSAVE state; switch-in restores FXRSTOR and FS_BASE before installing TSS RSP0. No continuation retains the old CPU's temporary GS entry slot. |

Audit search output is in `t3r/audit/`. The xAPIC-only boundary remains 64 logical
CPUs, unique unicast IDs 0..254; x2APIC, broadcast 255, hotplug and topology
mutation remain unsupported. There is no dynamic topology generation; the
validator checks every execution/dispatch against the single boot mapping and
firmware set. VM generations are authenticated independently.

Nonzero BSP, late BSP, maximum 64-CPU capacity, APIC 254 and invalid/offline
translations retain host-model coverage only for those exact cases. Native
T3 high/sparse topology evidence remains historical evidence for its recorded
source. It is never relabeled as native nonzero-BSP evidence.

## Genuine nonzero-BSP investigation

The installed emulator is QEMU 11.1.2. PC and Q35 paused QMP introspection shows physical
APIC 0 after both `-cpu max,apic-id=42` and
`-global max-x86_64-cpu.apic-id=42`. Attempting `-smp 0,maxcpus=2` to avoid the
automatically instantiated first CPU is rejected before boot. Machine/CPU
property help and exact QMP request/response transcripts are retained in
`topology-environment/`.

Upstream QEMU x86 machine source is retained as explanatory reference with its
URL; it constructs boot CPUs from possible CPU topology slots, overriding the
CPU property's initial APIC ID. This is reference source, not a claim that it
is the installed binary's exact source revision:
https://github.com/qemu/qemu/blob/master/hw/i386/x86-common.c

A fresh boot of the frozen migration image with the requested APIC 42 property
records the actual firmware order and BSP. The observed BSP remains APIC 0 in
slot 0. No remapped firmware table, fake CPUID receipt, CPU logical relabeling,
or synthetic native topology is used. This environment has not produced the
required topology. Nonzero-BSP initialization with AP bring-up, fixed/bounded/NMI
IPI and remote TLB validation is therefore **BLOCKED / NOT_RUN**; late-BSP
firmware initialization is independently **BLOCKED / NOT_RUN**.

## Migration mechanism and authenticated execution

`thread_migration_test` plus `debug_assertions` loads a small static ring-3 ELF
before normal services. A real parent creates two workers via ordinary native
ThreadSpawn. Authorization uses the registered PID, nonzero thread incarnation,
and full address-space identity, with a separately registered exact parent
identity. Naming alone authorizes no command. Creation uses normal admission,
publication and queue insertion. Debug placement sets both initial affinities
to logical CPU 0 before publication.

The migrating child seeds its state and reports actual source execution. It
then blocks through the supported native futex WAIT. The independent sibling
continues computing and polling; once the child is blocked, no longer current,
and physically unpinned, an authenticated debug command sets only the child's
normal affinity mask to CPU 1. The sibling invokes the ordinary native futex
WAKE. No saved context, task state, ready queue, ownership field or physical
handoff pin is directly manipulated to manufacture migration.

The wake receipt checks publication on CPU 1 and a pending remote reschedule.
Scheduler dispatch receipts run after architectural switch-in setup. A distinct
ring-3 syscall observation proves the same thread actually resumed on CPU 1.
The validator requires source execution < request < wake < destination dispatch
< destination execution < successful state receipt. Merely requesting placement
or observing a scheduler selection cannot pass. Destination execution may be
triggered by the IPI or a periodic timer; the fixture does not attribute causal
IPI delivery from the request bit alone. Separate remote wake gates test delivery.

## State preservation

The assembly preservation interval crosses SYSCALL and asynchronous scheduling,
with no ordinary function calls. RAX and RCX/R11 are legitimately clobbered by
the syscall ABI and are excluded. RBX and R12–R15 carry distinct fixed 64-bit
patterns. RBP is a bounded rendezvous counter, not a fixed-value claim.

The child checks two deterministic FS-relative TLS words, the live kernel-read
FS base, a stack-local canary, seeded x87 control/status/tag and ten-byte ST0,
XMM0/XMM15, and MXCSR `0x3f80`. FXSAVE comparisons exclude reserved padding and
instruction/data pointers. The independent sibling uses different XMM/x87 state
and repeatedly checks its own XMM value. TLS and stack allocation identities are
unchanged at source and destination; actual loaded TSS/IST ownership changes to
the destination CPU's distinct boot-recorded storage. These are functional TCG
results, with no scheduler performance claim.

## Lifecycle and resource reconciliation

Both workers follow native creation, dispatch, exit and supported
NativeThreadReaped polling. The fixture independently confirms their exact
registered identities in Reaped slots, success exit codes and single retirement
callbacks. No parent-exit, VM teardown, exec race or retained exit-status join
claim is made.

Reconciliation checks kernel stack/top/context and FS fields are retired; no
worker is current, queued, live, futex-waiting or cleanup-pending. Every online
CPU's handoff pin and waiter mask is empty. Admission reservations are local
RAII values under the scheduler guard, with no persistent reservation object.
There are zero remaining worker quota owners and exactly the parent's three FD
description references. PMM free frames return exactly to baseline. Heap usage
returns to baseline plus two normal cached FD-table boxes (20,480 bytes),
explicitly accounted reusable process-slot storage.

User stacks and TLS are static backing in the surviving owner's mapped ELF,
provided by ThreadSpawn's normal ABI; the kernel allocates no separate user
TLS/stack backing here. They remain owned by that process. This fixture does
not claim to retire the owner's VM. The native-std compatibility gates separately
exercise their normal dynamically allocated stack/TLS, join/detach and rollback.
Dormant idle TSS RSP0 may retain the last selection until the next dispatch;
CPL0 idle interrupts do not consume it. Active ring-3 ownership is verified on
every observed switch-in; no claim of zeroing every idle hardware pointer is made.

## Negative controls

The positive parser transcript is explicitly a host-only synthetic unit fixture.
It is never saved or presented as native proof. Controls reject request-only
migration, wrong parent incarnation, wrong thread incarnation, equal physical
APICs, missing destination execution, invalid topology mapping, wrong RSP0/IST,
missing TLS, corrupt/missing FP/state validation, missing/duplicate retirement,
heap drift, no independent progress, abandoned handoff waiters, stale VM
generation, panic and truncated completion. Every control fails closed. In addition, 12 separately labeled HOST_ONLY mutations
of the actual final two-CPU native transcript are rejected; their inputs and
results are retained under `negative-controls/`.

## Accepted native receipts and validation table

Final frozen migration image: `0a24f0e93cbdcc34985d9c5f811e8aaa1c0d4671d4bd8ad7fef60ae06bbe7da3`; ISO: `d429226b3951232c61b9a6113c682401827e6c6279c6e97b6f321bd958bf479e`.
The final source-bound suite is `.threading-evidence/t3r/20261105T164405.530264Z-migration`. `migration-receipts.json` extracts
its actual serial observations; the full serial remains authoritative.

| Native run | Parent PID/incarnation | Migrating TID/incarnation | Source → destination logical/APIC | Independent pulses before/during/after | Retirement |
| --- | --- | --- | --- | --- | --- |
| 2 CPUs | 58112/1 | 58113/2 | 0/APIC 0 → 1/APIC 1 | 1,3,1 | PASS, both exactly once |
| 4 CPUs | 58112/1 | 58113/2 | 0/APIC 0 → 1/APIC 1 | 1,9,1 | PASS, both exactly once |
| 12 CPUs | 58112/1 | 58113/2 | 0/APIC 0 → 1/APIC 1 | 1,6,1 | PASS, both exactly once |

The two-CPU pilot's exact VM is PML4 `269684736`, generation `1`.
The migrating thread's kernel stack top is `0xffffffff901e0520` and FS/TLS base
is `0x4020a0` on both CPUs. Its loaded TSS changes from
`0xffffffff8e6e7dc8` to `0xffffffff8e6e53c8`.

```text
[T3R] execution tid=58113 inc=2 cpu=0 apic=0 stack=0xffffffff901e0520 tls=0x4020a0 tss=0xffffffff8e6e7dc8 rsp0=0xffffffff901e0520 df=0xffffffff8f124250 nmi=0xffffffff8e9b6080
[T3R] execution tid=58113 inc=2 cpu=1 apic=1 stack=0xffffffff901e0520 tls=0x4020a0 tss=0xffffffff8e6e53c8 rsp0=0xffffffff901e0520 df=0xffffffff8ecc4250 nmi=0xffffffff8e9ba080
[T3R] state tid=58113 inc=2 tls=true fs=true gpr=true x87=true xmm=true mxcsr=true canary=true
[T3R] resources heap_base=1919184 heap_final=1939664 cache_bytes=20480 frame_base=333547 frame_final=333547 workers=0 reservations=0 stacks=1 fd_refs=3 handoff_pins=0 handoff_waiters=0 user_backing=owner_static pulses=1,3,1
[T3R] PASS cpus=2 executions=0,1 physical_distinct=true
```

| Current-source gate | 2 CPUs | 4 CPUs | 12 CPUs |
| --- | --- | --- | --- |
| Migration, TLS/FP/GPR/canary, exact identities | PASS | PASS | PASS |
| Exit, reaping and exact resource reconciliation | PASS | PASS | PASS |
| Independent progress, no retained handoff pins/waiters | PASS | PASS | PASS |
| Remote wake and physical handoff gates | PASS | NOT_RUN (focused scope) | PASS |
| Native-std TLS/join/detach, failure rollback and Hayro | PASS | PASS | PASS |
| Dedicated FP preemption and PDF/vector raster compatibility | PASS | PASS | PASS |
| Remote TLB replace/ACK/content and PMM reconciliation | NOT_RUN (focused scope) | NOT_RUN (focused scope) | PASS |
| Ordinary release production boot | PASS | NOT_RUN (focused scope) | NOT_RUN (focused scope) |
| Genuine nonzero or late firmware BSP | BLOCKED / NOT_RUN | BLOCKED / NOT_RUN | BLOCKED / NOT_RUN |

Host checks pass: topology 7, kernel library 171, Linux compatibility 29,
admission extraction 12, handoff 7, reschedule requests 5, scheduler state/policy
harnesses, and 16 T1/T2/T2R/T3/T3R parser test methods (including T3R's negative
subcases). Additional handoff/policy host harnesses were captured after the
first native matrix; their timing is retained rather than claimed as pilot
prerequisites. All are host evidence, not native architectural proof.

## Production separation and diagnostics

Ordinary debug build, ordinary release build, and a release build explicitly
selecting `thread_migration_test` all pass. The fixture module, controls,
callbacks, enumeration diagnostics and embedded ring-3 ELF require both the
feature and debug assertions; the build script also excludes assembly in the
ordinary release profile. No syscall number or production affinity API was
added. `production-exclusion.json` checks all three stripped images for fixture
markers, control strings and module symbols: PASS. The ordinary production
serial contains no T3R execution.

Changed Rust spans and the new module pass rustfmt. Existing full-file formatting
differences remain listed in `formatting.json`. Direct actual-kernel Clippy with
the fixture enabled remains **FAIL**, matching T3's diagnostic message/primary-
file multiset exactly (155 records including the terminal summary). Zero primary
spans intersect T3R changes. Four existing `sunlight-net` module-name warnings
remain. Clippy argv/environment, full diagnostics and baseline comparison are
retained under `clippy/`; neither baseline failure is relabeled PASS.

The initial fixture build remains FAIL because instrumentation used the wrong
LAPIC accessor name and had not enabled an existing FD-reference diagnostic for
this new feature. Those integration errors were corrected before native testing.
The first complete native variant passed 2/4/12; its original receipts remain
bound to that version. Final validation adds global pin/waiter reconciliation
and removes an irrelevant counter comparison. No production correctness fix or
scheduling-policy change was necessary. The two accepted final workers were
reaped normally without stack/TLS/FD/frame leaks or duplicate retirement.

## Reproduction and boundaries

```
python3 tools/threading_t3r_run.py
python3 -m unittest discover -s tools -p 'test_threading_t*.py' -v
python3 tools/threading_t3r_regressions.py std --cpus 2 4 12
python3 tools/threading_t3r_regressions.py fp --cpus 2 4 12
python3 tools/threading_t3r_regressions.py production --cpus 2
```

The main runner creates new timestamped attempts and stops native expansion on
a failed two-CPU pilot. Builds sharing `target/` must be serialized; guest images
are frozen before another feature is built. Historical failed build attempts
remain FAIL. Native or parser results from the first fixture variant remain
bound to their own source; final receipts additionally require empty global
handoff pins/waiters. All formal T1/T2/T3/T2R evidence remains unchanged.
