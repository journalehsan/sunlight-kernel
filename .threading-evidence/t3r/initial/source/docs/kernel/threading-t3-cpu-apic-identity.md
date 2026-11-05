# T3 logical CPU and hardware APIC identity separation

Status: **PARTIAL — implementation and tested xAPIC scope pass; nonzero-BSP /
late-BSP hardware proof and a separately forced migration trace remain absent.**

T1 remains COMPLETE. T2 remains implemented and validated for its completed
scope. T2R remains PARTIAL; this work closes none of its deferred lifecycle
races. No commit or push was performed.

## Source identity and scope

Starting and final HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
Starting canonical source SHA-256:
`d5c076715b7db5eefa911274858d95705077a713b6b4fd14c34b0f64e00b4418`.

`.threading-evidence/t3/initial/` preserves the initial dirty status, binary
patch, complete source hash inventory, and exact initial copies of edited
files. The repository already contained substantial T1/T2/T2R and IPC changes;
those were preserved. `final/source.json` records final canonical identity,
`final/status.txt` records the final dirty tree, and `implementation.patch`
isolates T3 edits from the starting tree. Canonical hashing uses sorted JSON
of SHA-256 hashes for tracked and unignored files, excluding
`.threading-evidence/`. Each command and guest independently captures source
hashes, exact argv, results, and logs.

The existing limit remains **64 logical CPUs / 63 APs**. The implementation
accepts unique unicast xAPIC IDs **0..254**, with exact BSP identification.
APIC ID 255 is broadcast and rejected; wider IDs, disabled LAPIC, and active
x2APIC mode are rejected. No x2APIC support is claimed. CPU hotplug and dynamic
topology mutation are unsupported. Host tests cover 64 CPUs and APIC ID 254;
native tests cover 1/2/4/12 CPUs and hardware IDs including 64. A 64-CPU native
stress run and native APIC ID 254 are not claimed.

## Identifier inventory and former conversions

| Identifier / path | Classification | Before / after |
| --- | --- | --- |
| Limine `lapic_id`, `bsp_lapic_id` | Hardware APIC ID | Formerly used to skip BSP; now validated, unique and mapped |
| Limine `processor_id` | Physical topology identifier | Diagnostic only, never an array index |
| Limine enumeration position | Startup-order index | AP order retained after removing exact BSP; independent of hardware ID |
| `AP_ONLINE` | Startup readiness counter | Formerly TSS allocation index; now checked only against resolved AP slot |
| `sched::current_cpu_id()` | Logical CPU index | Formerly CPUID APIC ID clamped to 63; now checked hardware-to-logical lookup |
| Scheduler queues, `CORE_STATES`, idle stacks, accounting | Logical CPU index | Consistent mapped identity; idle/perf index clamping removed |
| Handoff owner, waiter/reschedule/Doorbell masks | Logical index / CPU mask bit | Dense logical bits; no hardware-ID shifts |
| Scheduler affinity, native threads, Linux clone/getaffinity | Logical index / CPU mask bit | Placement semantics retained; Linux reports dense logical masks |
| Measurement origins and thermal slots | Logical CPU index | Current-CPU lookup now mapped |
| Thermal APIC labels | Hardware APIC ID | Physical CPUID diagnostic; xAPIC width validated before sampling |
| T1 GS entry and NMI IST arrays | Logical CPU index | Existing T1 entry protocol and owned thread stacks preserved |
| BSP GDT/TSS | Logical CPU 0 | BSP explicitly normalized even if hardware ID is nonzero or enumerated later |
| AP GDT/TSS/kernel stack | Proven AP storage slot | `ap_slot(logical)`, exactly logical minus one for validated APs |
| LAPIC timer / timekeeping owner | Logical CPU index | Timer dispatch uses mapped current identity; BSP remains logical zero |
| Fixed IPI, bounded Doorbell IPI, NMI | Logical input / hardware destination | Formerly index shifted into ICR; now checked online translation |
| IOAPIC input routing | Hardware APIC ID | Actual local hardware destination with checked width |
| TLB active masks, mailboxes and ACK slots | CPU mask bit / logical index | Existing sequence/ACK protocol retained; translate only at LAPIC boundary |

`identity-consumers.txt` and `hardware-conversions.txt` preserve source-audit
searches. Unsafe implicit conversions were APIC-to-array clamping, startup-order
TSS allocation mixed with APIC-indexed updates, and scheduler-index-to-ICR.
Debug remote-wake/handoff/Doorbell assertions also equated logical and hardware
IDs; they now compare through the mapping.

## Implementation and translation contract

`cpu_topology.rs` provides a bounded allocation-free model shared with host
tests. Logical zero is the exact Limine BSP; remaining IDs follow enumeration
order. Both lookup directions are checked. Empty/oversized enumeration,
duplicate IDs, missing BSP and unaddressable IDs fail deterministically.

`cpu_identity.rs` publishes the map through `spin::Once` before CPU feature,
TSS, syscall and scheduler initialization. Normal lookup takes no new global
lock. Full-width Limine identities are validated before any AP starts. CPUID
leaf 1's 8-bit initial APIC ID is used only inside this validated xAPIC scope;
unsupported full-width identities cannot silently alias through that read.
The BSP checks Limine against CPUID and the actual LAPIC register. Each AP
checks its record, resolved logical ownership, readiness slot, and LAPIC ID.

The immutable map never changes. A separate atomic online bitmap only publishes
monotonic startup readiness; there is no offline/hotplug transition. AP readiness
precedes scheduler online publication. Scheduler initialization requires the
exact validated CPU count rather than clamping it.

Fixed, bounded and NMI send paths resolve logical destinations to exact hardware
IDs and reject invalid/offline destinations before ICR access. xAPIC ICR width
is enforced by topology validation. Existing local interrupt exclusion,
Doorbell publish/notify ordering and TLB publication/sequence/ACK ordering are
unchanged. No fairness, quantum, nice, work-stealing or affinity policy changed.

TSS updates and AP initialization use the same logical-to-storage-slot rule.
T1 GS slots and NMI stacks consequently share that identity. Boot serial
witnesses record actual APIC IDs and TSS/double-fault/NMI IST addresses; the
validator requires unique nonzero ownership for every CPU, including BSP.

## Host evidence

| Controls | Result |
| --- | --- |
| Topology model: 7 tests | PASS |
| Kernel host library: 171 tests | PASS |
| Admission extraction: 12 tests | PASS |
| Linux compatibility: 29 tests | PASS |
| T1/T2/T2R/T3 parser controls: final 14 tests | PASS |

The topology controls cover contiguous/sparse IDs, nonzero BSP, BSP later in
enumeration, IDs above 63 (including 128/254), 64 CPUs with every mask bit unique,
duplicates, missing BSP, empty input, 65 CPUs, IDs 255/256/u32::MAX, both lookup
directions, AP storage slots, and invalid/offline IPI destinations. Parser
negatives reject aliased or missing identities/ownership, incorrect BSP/AP
observations, incomplete markers, panics and unobserved requested profiles.
Host tests are model/source evidence, not hardware proof.

## Native standard matrix

All runs use QEMU TCG and frozen ELF/ISO images. TCG supplies functional evidence,
not performance evidence. `validation-summary.json` records exact suite paths
and rechecks the actual guest serial records for every cell.

| Gate | 1 CPU | 2 CPUs | 4 CPUs | 12 CPUs |
| --- | --- | --- | --- | --- |
| T1 trusted syscall stack + scheduler lifecycle | PASS | PASS | PASS | PASS |
| T2 quotas, both ABIs' 75 allocation failures, recovery | PASS | PASS | PASS | PASS |
| Authority, IPC and exec replacement | PASS | PASS | PASS | PASS |
| Linux clone/TLS/FDs/child-TID + affinity | PASS | PASS | PASS | PASS |
| x87/XMM/MXCSR preemption + PDF/vector raster | PASS | PASS | PASS | PASS |
| Native std join/detach/TLS/OOM + Hayro workers/IPC | PASS | PASS | PASS | PASS |
| Remote wake | PASS | PASS | PASS | PASS |
| Stack handoff/pin release | PASS | PASS | PASS | PASS |
| Native Doorbell (42 rounds, external assertions) | PASS | PASS | PASS | PASS |
| Final release production boot | PASS | PASS | PASS | PASS |

Standard hardware IDs observed were 0 through CPU-count minus one. All TSS/IST
ownership checks passed. Remote fixtures exercise progress on every CPU and
cross-CPU execution/wakeup with timers masked where required. Work-stealing and
lifecycle ownership assertions pass, but no independently forced per-thread
migration trace was collected; that narrower proof remains unclaimed.

The raw Linux thread probe additionally checks an 8-byte `sched_getaffinity`
return, a nonempty dense mask, EINVAL for a 4-byte buffer, and prints all 64 mask
bits. The guest gate requires exact masks 1/3/f/fff. This adds no kernel hook or
policy. There is no newly implemented Linux `sched_setaffinity`.

## Genuine nonstandard hardware and TLB/IPI proof

| Hardware profile | Actual observed map | Gates |
| --- | --- | --- |
| Sparse 4 CPUs | logical 0,1,2,3 → APIC 0,1,2,4 | T1/scheduler, wake, handoff, Doorbell, Linux clone/affinity PASS |
| High-ID 2 CPUs | logical 0,1 → APIC 0,64 | Same gates PASS |
| Sparse 12 CPUs | logical 0..11 → APIC 0..6,8..12 | MM-2B replacement/retirement/ACK gate PASS |
| High-ID 12 CPUs | logical 0..11 → APIC 0..10,64 | MM-2B replacement/retirement/ACK gate PASS |
| Nonzero BSP requested | requested 42, actual BSP 0 | NOT_RUN for requested hardware |
| BSP later in firmware enumeration | Not produced | NOT_RUN; host model PASS |

Sparse 4 uses `-smp 4,maxcpus=6,sockets=2,cores=3,threads=1`.
Sparse 12 uses `-smp 12,maxcpus=14,sockets=2,cores=7,threads=1`.
High-ID uses `-smp N-1,maxcpus=128,sockets=2,cores=64,threads=1` plus
`-device max-x86_64-cpu,socket-id=1,core-id=0,thread-id=0`, with N=2 or 12.
All CPUs are present at initial boot. QEMU machine slots are not kernel hotplug
and are not the number of online CPUs admitted by the kernel.

The standard/sparse/high 12-CPU TLB gates each report:

```
[MM-2B-DIAG] requests=2 targets=22 local=2 remote=22 range=24 full=0 acks=22 switch_retries=0 no_remote=0 failures=0
[MM-2B] 12 CPUs pre-touched, replaced, acknowledged, and observed new content: OK
[MM-2B] PMM accounting returned to baseline: OK
```

Each request targets logical CPUs 1..11, mask `0xffe`, and waits for each target
mailbox's matching sequence ACK. The high profile translates logical 11 to
hardware 64; the sparse profile translates logical 7..11 to hardware 8..12.
Those target/ACK sets follow the fixture's per-mailbox assertions and source
loop, with 22 actual aggregate ACKs. The existing fixture emits no individual
ACK-mask serial events; none are invented here. Fixed/NMI delivery and the
remote-wake/bounded IPI fixtures collectively exercise all delivery boundaries.

QEMU `-cpu max,apic-id=42` still exposed actual BSP ID 0. That attempt supplies
no nonzero-BSP proof. Its original functional result is explicitly overridden
for hardware acceptance by `nonzero-interpretation.json`; the final receipt
classifies it NOT_RUN. The runner now checks requested profiles against actual
observations. No hardware support was simulated by changing messages.

The smallest remaining topology proof is a genuine nonzero-BSP Limine boot,
preferably with the BSP later in enumeration, exercising current identity,
TSS/IST ownership, remote delivery/TLB ACKs and scheduler progress. A separately
forced migration witness remains an additional requested native control.

## Production, diagnostics and security audit

Ordinary final debug build, bare-metal kernel check, release build and final
1/2/4/12 production boots pass. The ordinary debug Cargo build used for direct
Clippy also passed. Production image exclusions for existing T1/T2 test markers
pass. No topology mutation/override syscall, kernel test feature or production
debug topology hook was added. `[CPU-ID]` lines are ordinary boot diagnostics.
`production-topology-exclusion.json` additionally confirms remote-wake, handoff,
Doorbell and T1/T2 fixture PASS markers are absent from the final release ELF.

Strict Cargo Clippy fails in the existing native-SVG build script's nested std
build (`cfg_select` / platform module errors under clippy-driver). Direct kernel
Clippy was subsequently captured from a current ordinary Cargo build and reports
154 diagnostics, zero with primary spans in T3 changed lines or new modules.
Both failures remain FAIL with baseline attribution, not PASS. Exact argv,
environment and diagnostic classifications are in `direct-clippy/` and
`current-clippy/`. Full-file formatting reports baseline discrepancies; all T3
changed Rust spans and new modules pass the corrected focused formatting check.
Four existing `sunlight-net` module-name warnings remain.

The map itself adds no unsafe storage access. The new MSR read is confined to
architecture controller validation. AP static storage remains under existing
serialized startup and now has checked logical ownership. No new global hot-path
lock, allocation, topology mutation or interrupt-controller mode switch was
introduced. T1 SWAPGS stack selection, per-thread stack ownership, NMI entry,
Capability checks, admission quotas, VM/SHM retirement and handoff protections
were not weakened. There are no new dependencies or privileged tools.

## Failed / preliminary attempts and source variants

All attempt records remain under `.threading-evidence/t3/`:

- The preliminary T1 matrix predates final ownership diagnostics/idle-clamp
  cleanup; later T1 matrices supersede it.
- The first nonzero-profile check accepted functional markers without requiring
  the requested hardware profile. Its hardware classification is corrected
  to NOT_RUN; the original record remains intact.
- A line-limited rustfmt check missed new long expressions. Full formatting
  exposed them; a whitespace-only patch corrected them. The final checker
  compares full rustfmt output with changed spans. Canonical T1/production and
  architecture runs use formatted code.
- Waiting architecture orchestration was restarted before commands began to
  adjust fixture profiles and final-source checks. No active native guest was
  aborted in those restarts.
- The initial architecture wrapper misplaced `--timeout` after an argparse
  REMAINDER positional; commands failed to launch. `arch-run.log` and those
  failed records remain; `arch-retry-run.log` records the corrected full run.
- The first added affinity output exceeded the shell's captured-output limit,
  hiding later FD/TLS/child-TID markers. Those four attempts timed out and remain
  FAIL. Compact probe output then passed all four standard and both nonstandard
  guests. No shell/desktop behavior was changed.
- An intermediate receipt used wrapper `--iso` rather than QEMU `-cdrom` when
  associating guest records. It was corrected and now rechecks every suite cell.
- Cargo refreshed six native-probe lockfile package-version labels. The refresh
  is retained in `native-std-lock-refresh/`; exact initial bytes were restored.
  No unrelated source or dependency change remains.

`implementation-hashes.json` records pre-format implementation;
`formatted-implementation-hashes.json` records the formatted variant;
`formatting-only.patch` and exact prior copies establish identical non-whitespace
content. `startup-comment.patch` corrects a stale startup-order description
without changing code or line count. `final-implementation-hashes.json` records
current kernel files plus the compact affinity probe. All guest source identities
remain explicit; earlier embedded probe variants are not silently called final.

## Reproduction

These commands create new timestamped suite/guest attempt directories. Reuse
frozen images for exact ELF/ISO reproduction, or rebuild the indicated feature
from the recorded source identity first:

```
rustc --edition=2021 --test kernel/src/arch/x86_64/cpu_topology.rs -o target/t3-topology-test
target/t3-topology-test
cargo test -p sunlight-kernel --lib --target x86_64-unknown-linux-gnu --offline
cargo test -p sunlight-compat-linux --target x86_64-unknown-linux-gnu --offline
python3 tools/threading_t2_host.py
python3 -m unittest discover -s tools -p 'test_threading_t*.py'
python3 tools/threading_t3_format.py
python3 tools/threading_t3_regressions.py t1
python3 tools/threading_t3_regressions.py admission
python3 tools/threading_t3_regressions.py auth
python3 tools/threading_t3_regressions.py linux
python3 tools/threading_t3_regressions.py fp
python3 tools/threading_t3_regressions.py std
python3 tools/threading_t3_regressions.py production
python3 tools/threading_t3_validate.py guest tlb-high-rerun --iso .threading-evidence/t3/tlb/boot.iso --cpus 12 --topology high --timeout 180 --marker '[MM-2B] 12 CPUs pre-touched, replaced, acknowledged, and observed new content: OK' --marker '[MM-2B] PMM accounting returned to baseline: OK'
python3 tools/threading_t3_receipt.py
```

`threading_t3_arch.py` records the original architecture build sequence;
`threading_t3_delivery.py` and `threading_t3_affinity.py` record nonstandard
fixture invocations. Aggregate batch tools now refuse to overwrite retained
matrix/architecture paths. For fresh fixture builds, use the original fixture
tools with new `--evidence-dir` paths. Serialize shared-target build/freeze/ISO
steps; frozen guest images can run concurrently. Exact host/guest commands and
image hashes are present in each attempt's `result.json` and suite record.

## Remaining findings

T2R remains PARTIAL: parent exit, VM teardown, exec, concurrent reap/reuse and
prepublication cancellation lack their deferred native lifecycle controls.
Its accepted bounded pilots remain bounded pilots. T4 Linux futex comparison
atomicity, T5 robust owner-death cleanup, T6 retained child exit status, and
T7 native worker stack guards are unchanged and unresolved. No test here
reclassifies them.
