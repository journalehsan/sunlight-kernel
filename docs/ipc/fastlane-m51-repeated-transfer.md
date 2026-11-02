# M5.1 repeated authenticated native transfers

2026-11-02. Experimental debug-only, kernel-mediated backend. Production
FastLane remains disabled. No shared-ring fast path or performance work.
No commit or push.

## Implementation and boundary

`fastlane_repeated_test` implies the existing transfer/notification/mapping/
authority gates. The separate userspace feature is `fastlane-repeated-test`.
Ordinary M5 channels keep their one-publication lifetime restriction, including
when linked into the M5.1 image. A fresh authenticated producer explicitly
selects repeated mode before publication. This does not create new authority.
All accesses retain exact handle, role, process incarnation, VM, session,
expiry, attachment and restricted-mapping checks. Existing scheduler/broker
serialization and M2 physical backing remain unchanged. No new lock, queue
allocation, retained mapping pointer or per-message heap allocation is added
by the kernel admission state or private-copy representation.

Three pages provide sixteen fixed 256-byte slots. One channel, independent
Account consent and immutable roles are established once; each role maps three
views once. Full channels return Capacity without waiting or changing a slot.
The userspace representation stages/copies in initialized private arrays and
uses existing checked identities `(channel generation, sequence, slot,
slot generation = sequence / 16 + 1)`. All peer offsets and lengths remain
untrusted, bounded and validated before copying.

## Private lifecycle

Each slot follows `Available -> Reserved -> Published -> Acquired -> Copied ->
Available`. The last transition acknowledges consumption and advances the
private consumer cursor. A cancelled reservation returns to Available without
publication or cursor advance. Publication and acknowledgement accept only
checked cursor + 1; stale/duplicate transitions fail. Acquisition and copy
completion must name the exact current consumer sequence. The sixteen-slot
array and two private cursors cannot grow. An exhausted sequence fails rather
than wrapping. Channel terminal state dominates every slot operation; close,
revocation and actual process retirement use existing M1/M2/M4 teardown.
Retired rows may retain bounded nonauthorizing diagnostic metadata.

Only an accepted acknowledgement makes capacity available. The producer's
normal algorithm cannot reserve/overwrite a full queue. A hostile producer
still owns writable M2 aliases and may mutate outstanding payload/descriptor
bytes; its byte writes cannot advance the private publication/ack cursors.
The mediated backend admits producer-owned mutation of nonavailable storage
for the adversarial fixture; this supplies no extra mapping authority.

The acquisition/copy-completion controls are authenticated consumer protocol
claims, not independent proof that a malicious consumer examined bytes. A
consumer can discard its own messages. These controls enforce admission order;
they do not authenticate application content.

| Integrity property | Contract |
|---|---|
| Memory safety | PROVIDED |
| Private snapshot stability | PROVIDED |
| Coherent publication | NOT PROVIDED |
| Application authenticity | NOT PROVIDED |

The M5R saved-interrupt-state correction is preserved. No scheduler assembly,
queue policy, continuation, or interrupt-state source is changed by M5.1.

## Native fixtures and evidence

The opt-in `FL-M51` supervisor coordinates actual independently authenticated
producer and consumer processes pinned to CPUs 0/1. Each channel obtains consent
and mappings once. Channels transfer 1, 16, 1,000 and 10,000 messages. Payload
length is sequence modulo 257 (including zero and 256); byte values depend on
sequence and byte position. Consumer checks exact private contents and identity.
Producer and consumer emit separate per-message receipts; kernel admission
records are joined to immutable role bindings. Repeated notifications precede
consumption, with a separate genuinely parked slow-producer case. A full batch
is withheld from consumption before each explicit full outcome. Work and wait
retries are bounded; no retry spin is used to wait for capacity.

After sixteen acknowledged messages, further independent channels test malformed
length, offset, slot generation and channel generation; producer mutation after
the first copied word; revocation during copying; actual consumer kill during
copying; closure while a private copy awaits acknowledgement; and producer exit
with a reused outstanding slot. These are real IPC-controlled operation
boundaries. Independent M4 tests cover forced notification/registration lock
contention; ordered lifecycle cases do not claim simultaneous lock contention.

The native assessor requires exact message counts/order from both processes,
private lifecycle ordering, acknowledgement before reuse, exact channel/backing/
view joins, NX and role rights, unmap requests/completions, request-qualified
remote TLB ACK sets and exactly-once physical free after both views retire.
Empty target masks are never counted as remote acknowledgements. Complete
successful cycles must retain the same SHM/table census; every terminal census
must have zero channel-specific objects/views/frames. Final channel/waiter/
completion/pending-wake inventories must be zero.

Evidence root: `.fastlane-evidence/m51/`, ignored by Git, outside Cargo target.
The initial baseline contains revision, dirty-tree patch and verified copies of
all executable inputs. Timestamped attempts are never overwritten. Original
failed builds and sandbox/native failures are retained separately. No cleanup
or historical evidence deletion is performed.

## Validation status

**M5.1 native functionality: COMPLETE on 2/4/12 CPUs. Overall security and
regression readiness: PARTIAL (independent historical debt). Production
FastLane: DISABLED.**

Final evidence root (relative to the repository):
`.fastlane-evidence/m51/final-20261102T173617.452746Z/`.
Every accepted M5.1 guest uses the same frozen executable/configuration map and
image from `m51/build-20261102T173617.568916Z/validation-summary.json`.

| Configured / online CPUs | Channels | Publications | Acknowledgements | Full outcomes | Backings / mappings | Remote / zero-target unmaps | Result |
|---|---:|---:|---:|---:|---:|---:|---|
| 2 | 15 | 11,171 | 11,163 | 688 | 45 / 90 | 9 / 81 | PASS |
| 4 | 15 | 11,171 | 11,163 | 688 | 45 / 90 | 3 / 87 | PASS |
| 12 | 15 | 11,171 | 11,163 | 688 | 45 / 90 | 12 / 78 | PASS |

Each topology independently completes persistent-channel runs of exactly 1,
16, 1,000 and 10,000 messages. The 10,000-message channel reuses sixteen slots
through 625 slot generations. The eight publications without acknowledgement
are four malformed descriptors, revoke during copying, consumer exit during
copying, close before acknowledgement and producer exit with outstanding data.
They terminally retire; none is silently counted as consumed. The hostile mixed
copy is acknowledged only after checking its stable private contents. This is
explicit evidence against claiming hostile-peer coherence.

Native manifests, original serials, host ACKs and physical-object joins:

- 2 CPUs: `m51-2/native-20261102T173631.587982Z/`.
- 4 CPUs: `m51-4/native-20261102T173726.603137Z/`.
- 12 CPUs: `m51-12/native-20261102T173824.616362Z/`.

Each directory includes `<cpus>cpu.host-ack.json`,
`<cpus>cpu.physical-objects.json` and supplemental
`<cpus>cpu.channel-lifecycle.json`. The supplemental join records every admitted
message identity, lifecycle event, outstanding acknowledgement at terminal close,
Doorbell state and exact backing/mapping/TLB/free witnesses. Original per-attempt
indexes remain unchanged; the final aggregate index covers supplemental files.
All fifteen terminal censuses per guest have zero restricted objects, views and
frames; final active channel, waiter, completion and pending-wake gauges are zero.
The four successful complete cycles have identical global SHM/table censuses.
This is scoped reclamation evidence, not historical/global census reconciliation.

The source audit in `audit/source-audit.json` passes: authority, mapping,
Doorbell, syscall, scheduler interrupt and hostile representation files match
the baseline exactly; focused Rust formatting passes; no new unsafe block is
introduced. `audit/m51-reschedule.asm` independently confirms the final native
image preserves the M5R IF correction. Miri availability fails; execution is
NOT_RUN / UNAVAILABLE.

## Independent debt

| Debt | Classification |
|---|---|
| Historical M4 failures | UNRESOLVED; exact original artifacts unavailable |
| Re-audit deleted M4/M5 artifacts | BLOCKED without recovered originals |
| M1 debug coordination timeout | OPEN |
| Historical census drift | UNRESOLVED |
| Miri | UNAVAILABLE unless installed and executed |
| Production enablement | BLOCKED / disabled |

Fresh passing guests do not establish historical causal closure. Overall
security/regression readiness is separate from M5.1 functional acceptance.
Performance optimization remains deferred until repeated-transfer correctness
is demonstrated.

## Development failures retained

Each failed attempt below retains its original source map, logs and assessment;
later successes do not overwrite or relabel it.

- First development build: accidental visibility syntax in function parameters;
  compilation FAIL, corrected without changing the M5 algorithm.
- First sandboxed QEMU attempt: local QMP socket denied; execution FAIL, followed
  by a separately approved native run.
- First real native attempt: consumer acquisition control reached the legacy
  producer-only word-store check. Native assertion FAIL; control-specific role
  admission was moved ahead of word-access direction checks.
- Expanded native run: all 1/16/1,000/10,000 cases passed, then replacement actor
  receive failed because the supervisor retained the killed copying caller's
  ordinary IPC reply identity. The checked receive diagnosed InvalidArgument;
  source inspection confirmed ordinary servers must resolve or attempt replies
  before receiving again. The fixture now explicitly attempts its retained late
  reply, which existing exact-generation IPC safely discards. No IPC change.
- Subsequent complete guest exited 0, but assessment FAIL because two physical
  park diagnostic snapshots were absent. Snapshot requests were added; the
  physical-parking acceptance requirement was retained.

These are development-source failures, not final-source acceptance. The initial
host-kernel development suite passed 157 tests, including the private-state
10,000-message, premature/stale acknowledgement, cancellation and exhaustion
controls. Synthetic parser tests are acceptance-parser controls only.

## Frozen source and reproduction

Revision: `d3372b248e7332d72f36584826345ec67a436f5d` plus the M5.1 dirty tree.
The final executable/configuration map contains 1,369 files; sorted compact JSON
SHA-256 is `cb527e4a53a021f1fc66763d792d7cac325e3b3aee87b69308ac0f39a030003a`.
Experimental ELF SHA-256:
`86e5ac6a715f4172269059a414887d0023a52353e4603dc03416d80f93f4890e`.
Experimental ISO SHA-256:
`979298ea0b3bfbab828beb205ecd408d879fdb3ead4da2095efe0b21e95b1d47`.
Documentation edits are archived separately and do not change that runtime map.

```sh
python3 tools/fastlane_m51_validate.py --section build
python3 tools/fastlane_m51_validate.py --section native --cpus 2 4 12 --timeout 600 --image-manifest <build/validation-summary.json>
python3 tools/fastlane_m51_validate.py --section checks
python3 tools/fastlane_m51_validate.py --section production
```

The new validator defaults to durable evidence outside Cargo target. Existing
M5/M4 and compatibility tools receive an explicit durable `--evidence-root`.
QEMU requires permission to create its local QMP socket when sandboxed.
Every failed attempt keeps its original classification and original source.

## Next smallest executable milestone

Investigate the **OPEN M1 debug coordination timeout** with a bounded,
source-bound native control on two CPUs. Capture both actors' entry and return,
coordination deadline ownership and scheduler progress without changing
capability admission, timeout acceptance or the M5R saved-IF correction. Preserve
all failed attempts before expanding to 4/12 CPUs. Fresh compatibility passes
supply current coverage; they do not explain the historical timeout.

Shared-ring acceleration and throughput/latency work remain deferred. Historical
M4 attribution, global census reconciliation, unavailable Miri and independent
production security gates remain separate from completed repeated-transfer
functionality.

The experimental channel row is 384 bytes (the M5 row was 344 bytes); its table
limit remains eight. Slot/cursor state is inline and fixed-size. There is no new
per-message kernel allocation, grant, receipt, waiter class or history ledger.
Diagnostic formatting allocates bounded transient userspace strings, and host
serial/evidence files grow with the finite fixture count; neither is a kernel
message queue. Existing bounded table capacity, tombstones and the boot token
nonreuse ledger remain metadata rather than live frame ownership.


## Complete final-source validation matrix

All rows below have independent accepted receipts on the same 1,369-file runtime
map. There are **45 accepted native guests**: three M5.1, three M5, three M4,
33 compatibility/lifecycle/scheduler guests and three M3.1B guests. The M3 wrapper
and its nested M2 receipt describe the same guest and are counted once. Two
contaminated scheduler serials are excluded from accepted counts.

| Gate | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| M5.1 persistent authenticated repeated transfer | PASS | PASS | PASS |
| M5 authenticated single-transfer regression | PASS | PASS | PASS |
| M4 Doorbell, forced registration races, remote wake | PASS | PASS | PASS |
| M1 authority / M2 mappings / M3 native word probes | PASS | PASS | PASS |
| IPC deferred/reply races, VM and SHM retirement | PASS | PASS | PASS |
| Authenticated Account contention S1 and S2 | PASS (2 guests) | PASS (2 guests) | PASS (2 guests) |
| Account successful/failed exec, three cycles | PASS | PASS | PASS |
| Actual exit mode 11, three cycles | PASS | PASS | PASS |
| Actual exit mode 12, three cycles | PASS | PASS | PASS |
| Actual exit mode 13, three cycles | PASS | PASS | PASS |
| Scheduler lifecycle (corrected image selection) | PASS | PASS | PASS |
| R2D remote wake / physical handoff | PASS | PASS | PASS |
| Native kernel Doorbell mechanism | PASS | PASS | PASS |
| M3.1B reclamation / VM teardown, eight boundary crossings | PASS | PASS | PASS |

Final host checks pass: 157 kernel tests (including the new private-state
controls), 39 Ring tests and five doctests, nine Doorbell tests, 62 IPC tests,
18 Account tests, scheduler and extracted dispatch checks, mapping parser
controls, and all 247 FastLane parser controls. New Rust modules and the extended
backend pass focused rustfmt checks; `git diff --check` passes. Synthetic parser
controls do not count as native guests.

The ordinary debug production image has an empty experimental feature list,
exact normal embedded service bytes in both original and ISO-extracted ELF,
and no experimental authority/mapping/Doorbell/transfer/repeated markers or
symbols. The full ordinary release build passes; its retained ELF contains the
exact normal service bytes and no experimental markers. The experimental release
command fails with exit 101 at the existing debug-only guard: **exclusion gate
PASS, original command FAIL**. Release symbols are stripped; symbol inspection
of that ELF is NOT_RUN, with the independent ordinary debug symbol check PASS.
The retained release ELF and production audits are under `audit/`.

The machine aggregate is `.fastlane-evidence/m51/validation-summary.json`.
Its `functional_status` is COMPLETE, `new_regression_matrix` is PASS,
`overall_security_regression_readiness` is PARTIAL and production is DISABLED.
It references every receipt, native topology and count, source identity, original
failed command and historical classification. `matrix.json` inside the final
run remains FAIL because it retains the terminated broad command and expected
release rejection; this original orchestration result is not overwritten by the
accepted-coverage aggregate.

### Retained orchestration failure and corrected direct-image evidence

The first broad runner inherited `CARGO_TARGET_DIR` pointing at a shared cache.
Three older direct kernel scripts build using that environment but copy an ELF
from the hard-coded default `target/` path. The scheduler therefore booted an
older ordinary image. Its 2-CPU run timed out and remains FAIL; the 4-CPU guest
and parent broad process were deliberately stopped and remain ABORTED. The
12-CPU scheduler and downstream direct suites were NOT_RUN in that attempt.
Completed source-bound authority/IPC/exec/exit receipts remain valid.

The bad ELF, ISO, serials, build log and command environment are preserved.
`audit/contaminated-broad-abort.json` records the interruption;
`audit/scheduler-image-discrimination.json` proves the bad ELF lacks the scheduler
fixture marker and the corrected ELF contains it. This is an independently
observed image-selection error, not an unexplained kernel failure.

`direct-recovery-20261102T182711.805773Z/` runs the three existing scripts with
Cargo output explicitly aligned to their default paths. Each archived ELF is
attested equal to the just-built output before the next feature build. Scheduler,
R2D and native Doorbell all pass on 2/4/12 CPUs. The wrapper retains the full
before/after runtime map, source copies and immutable result/index files. The
original failed/aborted attempt is never substituted or relabelled. All mapping
host checks skipped by the aborted wrapper are independently covered by the
passing final host suite; the normal release build also ran independently.

| Classification | Retained outcome |
|---|---|
| PASS | All 45 accepted final-source native guests; host/audit/production/release gates |
| FAIL | Development attempts; contaminated 2-CPU scheduler; expected experimental release command |
| ABORTED | Contaminated 4-CPU scheduler and initial broad orchestration |
| BLOCKED | Production enablement; exact re-audit of deleted original M4/M5 artifacts |
| NOT_RUN | Miri execution (UNAVAILABLE); stripped release symbol inspection; out-of-scope optimization/shared-ring work |

### Evidence preservation

All new attempts and original failures are retained outside Cargo target. No
cleanup, historical deletion, commit or push occurred. Temporary development
logs and their synthetic-control source were copied into
`development-driver-logs/` and hash-verified. Existing user evidence was untouched.
Final reports, orchestration/recovery sources, per-message joins, per-object TLB
traces, images and source copies are sealed in a separate archive:
`.fastlane-evidence/m51-archives/final-20261102T173617.452746Z.tar.gz`.
The adjacent `.verification.json` is the authoritative full-member readability/
SHA-256 receipt. Original attempt indexes remain unchanged; a final aggregate
index covers supplemental lifecycle joins and audit files.

Overall security/regression readiness remains PARTIAL. Historical M4 failures
and census drift are UNRESOLVED, the M1 debug coordination timeout is OPEN,
and Miri remains UNAVAILABLE. Fresh passing guests do not establish historical
causal closure. The next executable milestone remains the bounded M1 debug
coordination progress investigation described above; performance work is deferred.
