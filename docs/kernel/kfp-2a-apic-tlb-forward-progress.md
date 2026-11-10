# KFP-2A — APIC, TLB shootdown and reclamation forward progress

## 1. Executive summary

**Verdict: PARTIAL.** The production-reachable unbounded ICR wait is established
by source analysis and reproduced with the actual wait function against fake
MMIO. This is a demonstrated missing software progress bound, not a demonstrated
permanently busy physical APIC. The finite remote acknowledgment budget does not
cover delivery. No timeout-and-free behavior was found in the examined normal
retirement paths. Reclamation depends on synchronous successful invalidation,
or on an inactive address space with lifecycle exclusion.

One kernel source file, `kernel/src/memory/security.rs`, was modified only in
the feature-gated MM-2D fixture to declare its trusted system-allocation
prerequisite. Production runtime behavior was **not modified**. Existing dirty
working-tree files were preserved. FastLane production was not enabled. No commit or push was performed.
This report does not close previous FastLane findings or claim universal memory
safety from a collection of passing fixtures.

Evidence: [directory](../../.kfp-2a-evidence/), particularly source-before.json,
preexisting.patch, dependency-graph.json, icr-baseline.json, mailbox-faults.log,
kernel-host.log, guest-results.json, retry/guest-results.json, and
results.json. Logs of failed attempts are retained.

## 2. Original KFP-1 uncertainty

KFP-1 B20/B21 and KFP-01 identified `wait_for_icr_idle` before the acknowledgment
loop. KFP-1 used host tests and source inspection, with no current-source guest
execution. KFP-2A re-read its report and evidence inventory; old guest receipts
are not counted as new test results.

## 3. Source and build identity

Initial inspection HEAD: `9ec5a16484fe5d7024898737ca6c896f3345d6cf`, with
20 modified tracked paths and existing untracked KFP-1/tmpfs/sunlight-mm work.
Before the pre-test snapshot, an external commit changed HEAD to
`882b40a65f55951b5cd8b9978c05580e71202488` (tmpfs and KFP-1 evidence).
This task issued no commit or push. **All builds and tests here use the latter
HEAD**, qualified by source-before.json and, after the test-only correction,
source-retry.json. `revision-transition.json` preserves the initially observed
modified-file list; `revision-transition.log` records the commit metadata.

`status-before.txt` and `preexisting.patch` describe the pre-test snapshot,
after the external commit, so the patch is empty. They must not be represented
as a capture of the earlier dirty patch. The SHA-256 manifest was captured before
tests and covers tracked and nonignored untracked files at capture. Final
source-integrity.json confirms every captured file unchanged except the intended
three-line fixture correction. Generated target artifacts are excluded. Source
references use baseline line numbers; later security.rs locations shift by three
in the corrected fixture.

Existing QEMU, xorriso, objcopy, the Rust toolchain and built Limine artifacts
are available. The selected guest fixture is MM-2D (`mm2d_munmap_test`), whose
remote target mask adapts to online CPUs. MM-2B (`mm2b_smp_test`) asserts exactly
12 CPUs at tlb.rs:616 and cannot serve as the required 2-CPU pilot unchanged.
T4B and the FastLane runners exist but many build in shared locations or write
other milestone evidence. `tools/build.sh` runs version_manager and touches
source; it was deliberately not used.

Kernel Cargo output uses `.kfp-2a-evidence/build`; host Cargo output uses
`.kfp-2a-evidence/host-build`. Important limitation: kernel/build.rs:212–245 and
later steps unconditionally build embedded programs through the existing
`target/embedded-build` and copy them to `target/.../release`. Changing the outer
Cargo target does **not** isolate these artifacts. These are build outputs, not
source modifications. The ISO helper receives a dedicated project-root scratch
directory, keeping its destructive fresh ISO staging inside KFP evidence.

## 4. APIC implementation and ICR polling

The driver is xAPIC MMIO, not x2APIC. `cpu_identity::validate_controller`
(cpu_identity.rs:14) rejects disabled LAPIC and x2APIC mode. Topology maps logical
CPU IDs to validated online hardware destinations; ONLINE is monotonic and no
CPU hot-unplug protocol exists. AP startup is delegated to Limine rather than a
second locally implemented INIT/SIPI ICR loop.

| Path | Actual caller | Polling and completion |
|---|---|---|
| lapic.rs:145 `wait_for_icr_idle` | both blocking send functions | volatile ICR LOW bit 12, unbounded spin |
| lapic.rs:153 `send_fixed_ipi` | tlb.rs:319; feature-gated test broadcaster :501 | wait; HIGH destination; LOW fixed vector; wait |
| lapic.rs:240 `send_nmi` | tlb.rs:324 | wait; HIGH destination; LOW NMI mode; wait |
| lapic.rs:168 `try_send_fixed_ipi` | bounded scheduler wrapper | one busy read; returns false without writing when busy |
| lapic.rs:220 `send_fixed_ipi_bounded` | sched/mod.rs dispatch notification flush | 4096 attempts, returns false; caller fail-stops; acceptance is not target execution |

These classify all ICR accesses found in the driver and all blocking-send call
sites in kernel source. Every remote target in `invalidate` uses four unbounded
idle waits: two around fixed delivery and two around NMI delivery. Even an ack
already produced by the fixed handler cannot release the initiator while it
is stuck in the subsequent NMI send.

The [Intel SDM Volume 3A](https://cdrdv2-public.intel.com/835754/253668-sdm-vol-3a.pdf)
describes delivery status as idle/send pending and the x2APIC MSR interface.
Delivery status is not a software invalidation receipt. No numerical maximum
delivery duration was established from the material consulted; the PDF fetch
failed after search discovery, so this is not a complete architectural/errata
review. No claim is made that hardware is allowed to remain busy forever during
fault-free operation. Stuck device/emulator state is a controlled fault model.
QEMU success does not establish VMware or physical-hardware behavior; VMware
was not tested. Host scheduling can extend any iteration or TSC budget in wall
time.

There is no ICR timeout, retry escalation, APIC reset or backing quarantine API
in the blocking sends. Polls need hardware/hypervisor progress, not a target
thread obtaining the scheduler lock. However they retain the initiator CPU and
its guards, delaying dispatch, reclamation and potentially BSP timekeeping.

Other CPUs have independent local ICRs. The existing shootdown serializer
serializes shootdown initiators; it does not serialize every scheduler IPI.
Safety of HIGH/LOW pairing additionally depends on local IF remaining clear and
on the NMI handler not issuing IPIs. The normal NMI handler does not send IPIs.
The blocking unsafe send APIs do not enforce IF=0 themselves. Source-supported
normal syscall/page-fault/boot contexts are masked; universal future-caller
protection and every optional diagnostic feature combination are not proved.

## 5. Dependency graph and caller variants

The [machine-readable graph](../../.kfp-2a-evidence/dependency-graph.json)
records stage functions, contexts, guards, orderings, completion dependencies,
failure behavior and ownership. A DOT copy is adjacent.

```mermaid
flowchart LR
  A[SHM retirement: channel_mapping.rs:31] --> B[Verified withdrawal: shared.rs:284]
  B --> C[PTE clear: address_space.rs:1402]
  C --> D[Serializer and mailbox: tlb.rs:199]
  D --> E[Fixed IPI then NMI: lapic.rs:153,240]
  E --> F[Remote atomic handler: tlb.rs:408]
  F --> G[Exact sequence ack: tlb.rs:329]
  G --> H[Last view transfer: capability/mod.rs:1424]
  H --> I[Frame release then quota: capability/mod.rs:344]
```

Not every caller follows the entire graph:

* Explicit SHM free (syscall.rs:10960) and exit cleanup
  (shared.rs:470, sched/mod.rs:2963) converge on `free_shared_page`. The newest
  local view is preflighted against frames and the mapping ledger, each leaf is
  synchronously unmapped, empty tables reclaimed, and only then the view record
  and broker reference are removed. Failure before mutation returns an error;
  an unexpected failure after preflight panics with ownership still retained.
* Restricted FastLane sweep/revoke (channel_mapping.rs:31,44,226) first retires
  authority and rejects attachment, then drains actual process views. Revoking
  an object with zero views can transfer backing directly; there is no stale
  mapping to invalidate. Rows retain backing tokens until drainage completes.
  This is feature-gated diagnostic/restricted plumbing, not authorization to
  enable production FastLane.
* Anonymous munmap (mmap.rs:960–1015) records removed ownership in a bounded
  chunk, clears leaves, does one covering range shootdown, then frees frames or
  discards swap blocks and reclaims empty tables. It does not decrement SHM
  references. Permission updates invalidate but do not release backing.
* Replacement (address_space.rs:865–927), including swap-out
  (swap.rs:146), invalidates before `ReleaseOldFrame` frees the replaced frame.
  Swap metadata is prepared before replacement. Direct reclaim's page/TSC
  budget does not bound nested ICR waits.
* Rollback of published mappings (address_space.rs:1448 and :1920) invalidates
  before releasing installed paging structures. Unpublished fresh allocations
  can be released without a shootdown. These are different ownership cases.
* Process owner teardown (sched/mod.rs:2769–2978) defers while owned, queued,
  pinned, current, with live borrowers, or with an active address-space mask.
  `reclaim_user_space` (address_space.rs:1551) asserts inactive and returns early
  if already reclaimed. It releases an excluded inactive VM rather than
  broadcasting per leaf. Native thread borrowers skip owner user-space reclaim.
  Clone failure and image replacement also call owner reclaim and need their
  own lifecycle exclusion; the reaper's checks cannot simply be assumed for
  every direct caller.

## 6. Remote acknowledgment protocol

`invalidate` takes the existing SHOOTDOWN_SERIALIZER and obtains a nonzero,
checked monotonically increasing sequence using AcqRel fetch_update. Sequence
exhaustion panics. It updates MUTATION_EPOCH with Release after the PTE mutation,
snapshots active CPUs for `(pml4_phys, generation)`, flushes locally if active,
and excludes the local CPU from remote targets. PCID is disabled; changing CR3
removes non-global translations. This contract does not cover arbitrary global
user mappings.

Each target owns a distinct mailbox. `try_publish` (mm2b_state.rs:175) requires
the prior request acknowledged, rejects zero/non-increasing sequence or invalid
identity, writes payload Relaxed, then publishes request_sequence Release.
`pending` acquires the sequence. `try_claim` uses CAS AcqRel so fixed-vector and
NMI consumers do not both flush/ack the same current request. The target compares
the full current VM identity and flushes if it matches; another current root
needs no retained PCID translation. Ack is Release after the flush; initiator
uses Acquire equality against its exact sequence in each selected CPU mailbox.

Acknowledgment identity is therefore a protocol property: unique sequence plus
per-CPU storage and immutable single-outstanding payload bind the VM and range.
The ack integer itself does not encode VM, CPU or range, and `acknowledge` does
not independently validate its argument. Host controls establish stale equality
rejection and one claim winner; they do not establish all NMI/activation
interleavings. An adversarial old ack cannot satisfy a larger new sequence; it
can obstruct progress. Concurrent publication without the serializer is outside
the API's required usage.

Activation (tlb.rs:119) loads an epoch, reloads CR3, publishes the active identity
and retries if epoch changed. Active slot generation zero returns None rather
than waiting for the interrupted local writer. The full memory-model proof of
activation overlap, generation-transition snapshots and paging-structure cache
retirement remains open. A passing reuse fixture is not that proof.

## 7. Locks, interrupt state and yielding

Ordinary SHM retirement holds scheduler/VM exclusion, PMM and broker guards;
restricted retirement explicitly documents SCHEDULER → PMM → CAP_BROKER.
TLB then acquires SHOOTDOWN_SERIALIZER. Every one is a spinning mutex. Mapping-ledger locks are terminal and released
before TLB/PMM operations (address_space.rs:348), rather than held across ack. No sleep,
yield or scheduler priority adjustment was introduced. Normal syscall FMASK
(syscall.rs:218) clears IF, page-fault interrupt gates mask interrupts, and boot
self-tests precede the initial userspace IRET. These waits cannot safely park
the arbitrary current Rust stack under those guards.

The normal remote NMI handler (interrupts.rs:252 → tlb.rs:408) takes none of
these mutexes and does not schedule. This breaks the direct cycle where a target
spinning on SCHEDULER would otherwise need a maskable IPI. NMI blocking until
IRET, target/hypervisor execution and inactive/transition identity observation
are separate dependencies. Trace-enabled handlers require a separate audit;
the no-lock statement applies to the ordinary handler.

No new lock edge was added; the fixture-only boolean assignment takes no lock. Existing global lock-order completeness and maximum
IRQ-off/guard-hold durations remain unproven. There is no supported hot-unplug
or proof that a stopped target has lost all translations.

## 8. Reclamation invariants

| Invariant | Source support and limitation |
|---|---|
| INV-1 no premature reclaim | unmap returns physical ownership only after invalidate; replacement frees afterward; munmap waits before chunk release. Inactive owner reclaim has a different lifecycle precondition. Full activation/cache proof remains open. |
| INV-2 exact ack | checked unique sequence, per-target mailbox, Release/Acquire payload and completion, claim CAS, VM root plus generation. Host controls pass; arbitrary NMI interleavings not exhausted. |
| INV-3 safe delivery failure | ICR busy never returns success; ack exhaustion calls nonreturning invariant panic. No delivery-error success conversion exists. |
| INV-4 safe timeout | panic does not authorize free; kernel panic disables interrupts and never unwinds (panic.rs:27). Retention is fail-stop ownership, not a resumable quarantine subsystem. |
| INV-5 no lock regression | no production runtime change; ordinary NMI does not acquire retained locks. Existing global order is not fully proved. |
| INV-6 capability isolation | revoke_shared retires lifetime before drainage, cleanup access differs from new attachment, exact VM/identity checks remain unchanged. Host broker tests and guest security matrix have distinct scopes. |
| INV-7 single release | mapping removal, last-view swap_remove, consuming ReclaimedShared and deferred quota refund; reclaimed-root marker and borrower exclusion. PMM free_frame itself does not prevent duplicate frees, so caller proof matters. |
| INV-8 no false completion | exact sequence equality; sequence never wraps normally; panic never resumes abandoned stack; no async timeout receipt can later authorize release. |

An initiator stalled before release retains the frame under the examined
ownership chain. This is source-supported retention, not a measurement of
physical allocator state during an injected guest fault. PMM is an allocator,
not an independent shootdown authorization checker. No universal no-reuse claim
is made for every kernel caller.

## 9. Controlled fault investigation

All added controls are under evidence and use host processes. No production
feature or release binary contains injected failure behavior.

| Test ID | Trigger / expected behavior | Bound and cleanup | Result scope |
|---|---|---|---|
| KFP-ICR-baseline-delay | actual wait function; fake read returns busy 10000 times then idle; must return | 5 s subprocess watchdog; child reaped | 10001 reads, returns |
| KFP-ICR-baseline-stuck | fake ICR always busy; baseline has no completion | 5 s watchdog kills/reaps child | START without COMPLETE; demonstrates wait, not safe recovery |
| kfp_ack_missing_and_delayed | publish real mailbox; withhold ack for 10000 checks, forbid overwrite, then claim/ack | entire Rust binary 10 s watchdog; threads joined | exact helper behavior; no physical frame allocated |
| kfp_duplicate_doorbell_and_generation_reuse | fixed/NMI-style duplicate claim; new generation at same root | 10 s binary watchdog | second claim rejected; old ack does not complete new request |
| kfp_stale_ack_is_not_new_completion | explicitly inject old writer after newer publication | 10 s binary watchdog | new request incomplete; further publication Busy |
| kfp_concurrent_claims_one_winner_per_cpu | 12 host threads contend on actual claim CAS | 10 s binary watchdog; join all | exactly one winner; no automatic ack |

Mailbox controls execute the original source via a path module; the ICR control
extracts its function text and replaces only MMIO with a host stub. Four new
mailbox tests plus six original tests pass. These tests do not call PMM or the
production invalidate loop. Retained/released frame counts are NOT_MEASURED,
not zero. Missing/delayed target execution, concurrent shootdown initiators,
retire/exit/cleanup races and cancellation during pending reclamation have not
been fault-injected in a guest. CPU unavailability is unsupported recovery,
not a license to remove the target from the acknowledgment mask.

## 10. Host and guest results

The machine-readable results and raw logs are authoritative for actual attempts.
The guest runner starts with two CPUs, checks unchanged source before launch,
freezes ELF/ISO identities, uses TCG `-cpu max`, 1024 MiB, no network, entropy
device and a 120-second external watchdog. It requires **every** MM-2D marker
and stops expansion on any failing run. Successful marker collection is followed
by host termination; a negative QEMU exit is recorded as host termination, not
a clean guest shutdown. Watchdog termination never counts as safe recovery.

MM-2D checks remote old content, synchronous unmap, immediate allocator reuse of
the old frame as a guard, remapping to a different frame, remote new content,
and PMM restoration. On one CPU the remote target count is zero; local content
and allocator checks still apply. The fixture does not emit individual ack
sequences or dispatch latency samples, and does not force simultaneous competing
shootdown initiators. Per-run observations must not be promoted beyond that scope.

| Attempt | CPUs | Result | Watchdog | Fixture elapsed (s) |
|---|---:|---|---|---:|
| Original fixture | 2 | FAIL: SHM policy prerequisite | No; panic | 25.54 |
| Corrected fixture | 1 | PASS, all markers and topology | No | 25.03 |
| Corrected fixture | 2 | PASS, all markers and topology | No | 26.54 |
| Corrected fixture | 4 | PASS, all markers and topology | No | 69.26 |
| Corrected fixture | 12 | PASS, all markers and topology | No | 33.05 |

These are fixture/boot durations, not invalidation latency samples. All QEMU
processes were explicitly terminated by the host after panic or marker capture;
QEMU handled SIGTERM and returned 0. Raw qemu logs and corrected host_terminated
fields preserve that distinction; guest-results.raw.json retains original runner
output before this metadata correction. No guest watchdog expired. The baseline
panic happened before the remote-reuse subtest and before SHM backing allocation;
it is not evidence of a TLB safety violation.

Host controls: mm2b direct tests **6 PASS**; mailbox binary **10 PASS** including
those six; scheduler controls PASS; FastLane `test_fastlane_m*.py` discovery
**204 run, 67 skipped**, 137 cases pass. The kernel host suite also reports
**185 PASS**, covering broker lifecycle, VM generation and ownership helpers. Skipped prerequisite cases are
NOT_RUN, not PASS. The additional ring/doorbell/IPC/UAC library run reports **163 PASS** (9, 42,
94 and 18 tests by test-binary order); see security-libs.log. Boot logs also
contain terminal-arbitration/generation and IPC capability lifecycle assertions.
Full current-source M1–M5 authenticated guest/security suites
are not inferred from their synthetic receipt validators.

## 11. Production changes and correction policy

No production runtime correction. A missing software bound is established, but
no safe recovery or complete closure is claimed. The first 2-CPU guest panicked
at baseline security.rs:585 when SHM allocation returned InvalidArgument.
`Process::new` defaults system_allocation to false and the fixture supplies no
Account membership; shared.rs:98 rejects this before backing allocation. The
small fixture correction declares system_allocation on its private trusted
process, matching shared_lifetime_gate.rs and existing security fixtures. It is
entirely under `#[cfg(feature = "mm2d_munmap_test")]`; no authorization predicate
or assertion was weakened. Baseline logs remain in the evidence root, retry
logs in `retry/`; source-retry.json records the exact new source identity. A minimal candidate is a finite ICR read budget that enters
the existing nonreturning fail-stop path on exhaustion. It must never return a
normal completion or free backing. Such a change bounds polling work, not host
wall time, successful service, panic rendering, serializer acquisition or active
snapshot retries. A full correction should test pre-send and post-send busy,
late ack, exactly-once ownership retention and normal security/lifecycle paths
in an isolated guest before adoption. Resumable recovery needs retained
transaction records and a proven quiescence mechanism; that is larger work.

## 12. Measurements

No ICR MMIO, NMI dispatch, ack, shootdown, IRQ-off, lock-hold or reclamation-delay
latency distribution was instrumented. Kernel sample size is **0** for each;
p50/p95/p99/max are unavailable. Host fake-MMIO wall durations in
icr-baseline.json are harness overhead and fault-control timing, not kernel
performance. Guest elapsed time is boot/fixture duration, not shootdown latency.
There is no production runtime before/after comparison and no measured quarantine count.

## 13. Remaining uncertainties

Unbounded ICR wait, serializer fairness, active snapshot/epoch overlap proof,
NMI blocking, late/stale reader interleavings, hardware/VMware behavior,
paging-structure cache lifetime, full direct owner-reclaim caller exclusion,
guest fault retention accounting, simultaneous shootdowns and full security
regressions remain explicit gaps. No workaround changes scheduling priorities,
weakens assertions, adds a global lock or sleeps under guards.

## 14. Regression assessment

Host protocol, lifecycle, scheduler and synthetic receipt results must be read
separately from native guest results. Guest MM-2D covers anonymous memory policy,
SHM rejection, swap release and PMM accounting; it is not the complete SHM
retirement/security matrix. Full IPC closure, FastLane M1–M5 native authorization,
quota and generation-qualified forced-revoke races require their own fresh
receipts. Existing unresolved FastLane gates remain unresolved.

## 15. Final verdict

**PARTIAL**, because the acceptance requirement of bounded forward progress plus
safe lifetime under guest delivery/ack faults remains unproven. No demonstrated
premature free is reported; neither is its universal impossibility established.
Recommended next milestone: **KFP-2B — bounded fail-stop APIC delivery and
source-bound guest fault retention**, followed by activation/cache-lifetime
concurrency proof and the full native security/lifecycle matrix. Production
runtime was unchanged; the feature-gated fixture in security.rs was modified.
No commit or push occurred.
