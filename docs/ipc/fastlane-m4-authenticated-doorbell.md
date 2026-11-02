# M4 authenticated native Doorbell

2026-11-02. Notification only. Production FastLane remains disabled. No Ring
payload transfer, permission expansion, desktop/audio changes, commit or push.
**M4 notification functionality is COMPLETE.** The final authenticated 2/4/12-CPU
matrix and all requested current regressions pass. Overall regression readiness
remains **PARTIAL** because historical census failures and the M1 debug timeout
remain open. Production readiness remains BLOCKED.

## Authority and interface

The existing `CapabilityBroker::Channel` owns the optional F1.3 Doorbell model.
There is no second token issuer, bearer grant or notification authority table.
Both processes independently request Account consent and attach through M1.
Every notification operation authenticates the exact token and channel
generation, process incarnation, VM generation, role and lifecycle. The native
adapter also fences logical death, replaced VMs, withdrawn sessions and issuer
identity before admission. A model is initialized only for a fully attached
pair. No shared-memory pointer or descriptor enters this path.

Private selectors and operations exist only with `debug_assertions` and
`fastlane_doorbell_test`, which implies M1/M2. Ordinary images have no selectors.
The existing release compile guard rejects experimental channel features.

| Entry | Operation / authority |
|---|---|
| `0xf1c1`, op 10 | Notify; exact producer |
| `0xf1c1`, op 11 | Observe sequence; exact consumer |
| `0xf1c2` | Conditional wait; exact consumer; token/generation/observed sequence |
| `0xf1c1`, op 13 | Consume one process-private completion for the exact original handle/VM |
| `0xf1c1`, op 14 | Cancel the channel's current wait; exact producer |
| Existing op 4 | Close/revoke; existing M1 member permission |
| Existing lifecycle | Peer exit, exec, session retirement, expiry; trusted kernel events |

Wait returns zero after a registered wait resumes, or `Changed` (10) without
registering. Collection returns the committed sequence, `Cancelled` (9),
`Terminal` (4), or `Exhausted` (6). Only the matching receipt can be collected;
collection grants no new authority even if the channel is already retired or
its slot reused. One uncollected receipt prevents another wait by that process.
Cancel addresses the **current channel wait**, not a transferable registration
ticket. Notify/cancel commits at most one completion; the later operation
cannot overwrite it. Sequence MAX is a valid final value. The next increment
commits sticky exhaustion and releases the waiter, without wrapping.

Fixture-only controls are separate from notification authority: ops 15/16/17
arm bounded timing, seed MAX-1 after producer authorization and open-state
checks, and inspect bounded accounting. Private selector `0xf1c3` restricts
fixture processes to CPUs 0/1 using the scheduler's existing `cpu_mask`;
it changes no channel identity or role. Without placement a debug rendezvous
stranded the controller behind its spinning consumer. That failed attempt is
retained. These timing hooks never substitute actors or decide method results.

## Serialization and progress

All channel operations retain existing `SCHEDULER -> CAP_BROKER` ordering.
Observe releases the lock normally. Wait reacquires it and rechecks the model's
sequence before inserting its waiter, process receipt and `BlockedOnIo` state.
Notification before insertion is observed as Changed. A contender during
registration cannot mutate the sequence until that transaction ends; it then
detaches the exact waiter, records its receipt and calls existing `wake_io_pid`
before releasing the scheduler lock. No subsequent unconditional Blocked write
exists. The native race hook holds the transaction after model/receipt insertion
until a real remote failed `try_lock` is observed.

F1.3N's kernel-stack park trampoline is shared, rather than duplicating its
unsafe assembly. It copies the syscall register frame to the process kernel
stack, releases guards, flushes dispatch obligations and enters the reschedule
vector. This preserves the outgoing stack pin/pivot protocol and avoids
suspending a continuation on an address-space-private userspace stack.
Nonwaiting results return without a context switch. Ordinary remote wake,
enqueue, live-owner, physical pin, ICR dispatch and scheduling policy are reused.

The scheduler's established bounded ICR retry tests cover temporary busy ICR,
coalescing, delivery acknowledgement and retained work on diagnostic exhaustion.
Permanent hardware nonprogress fails diagnostically; no unbounded eventual
delivery guarantee is claimed. Controlled ICR busy states are **host hardware
simulations**, not a claim of forcing a physical ICR condition in an
authenticated guest. Native receipt collection separately proves execution of
the awakened consumer. No production poller or new global hot-path lock is added.

## Lifecycle and resources

Retirement synchronously removes the model waiter and retains at most one
bounded wake obligation in its broker row. Slot reuse rejects a pending wake
as well as M2 backing. The scheduler drains obligations before another native
operation, at the existing mapping sweep and on the timekeeper lifecycle sweep.
A completion requires exact handle, process incarnation and receipt VM, and
cannot replace an existing outcome. Dead/replaced process receipts are cleared.
There is no deferred live-registration callback: notify, cancel and signal
effects commit before releasing SCHEDULER. A retained retirement obligation
names a retired handle, which cannot admit a new wait or be recycled while
that obligation exists. Together with one outstanding receipt, this prevents
an old completion from targeting a later wait without exporting another
registration token/authority. A future asynchronous callback design would need
its own exact registration epoch; this proof does not cover such an extension.
Unrelated I/O/signal wakes withdraw the registration; an already committed
notification/terminal outcome is preserved. SIGCONT cancellation follows the
existing foreground signal authorization, rather than granting the signal
sender a Doorbell role. Successful exec invalidates old operations; failed exec
does not retire an existing role.

The table retains M1's eight-row bound and fallible initial reservation. The
Doorbell model and pending obligation are inline; each process has at most one
inline receipt. Doorbell state introduces no per-notification allocation;
existing scheduler queue/process allocation policies remain in effect. There
is no retained process/VM
reference, asynchronous object pointer or dynamically growing waiter list.
Broker token nonreuse history remains charged under its existing boot limit.
`[FL-BELL-RESOURCE]` records actual row/capacity, active channels, model waiters,
process receipts, pending obligations and type sizes. Repeated cycles require
zero active/waiter/receipt/pending counts after closure. This is channel-local
resource accounting, not reconciliation of whole-system heap, PMM or SHM drift.
M2 shared mappings, final-view release, synchronous TLB acknowledgements and
frame retirement are unchanged.

## Native fixture and evidence

`fastlane-doorbell-test` in UAC's existing authenticated userspace fixture
creates separate producer, consumer and foreign processes with independent VMs.
The controller has no channel membership. Every pair uses actual Account IPC,
fresh consent receipts and ordinary participant syscalls. Private IPC coordinates
actors; an ordinary IPC deadline parks the controller while checking sleeping
waits. There is no notification-state busy polling. Bounded spins occur only
inside explicit debug race rendezvous.

The final intended guest covers 30 authenticated pairs, 33 registrations,
32 completion/collection pairs and one actual blocked consumer death. Cases
include notify-before-wait, eight repeated sequences, registration in both
orders, unauthorized roles and stolen/stale handles, producer exit, consumer
exit/notify in both forced orders, revocation, producer cancellation in both
orders, actual signal cancellation/notify in both forced orders, MAX exhaustion,
16 reclamation cycles and failed/successful exec. Assertions require actual
consumer execution or confirmed death; kernel method fixtures cannot satisfy
these receipts.

Reproduction:

```sh
python3 tools/fastlane_m4_validate.py --section build
python3 tools/fastlane_m4_validate.py --section native --cpus 2 --image-manifest <build/validation-summary.json>
python3 tools/fastlane_m4_validate.py --section native --cpus 4 12 --image-manifest <build/validation-summary.json>
python3 tools/fastlane_m4_validate.py --section checks
python3 tools/fastlane_m4_validate.py --section production
```

Final accepted source: HEAD `91a28f22aaa17d3e365398cc31f7d7db662e54e5`
plus the archived working tree, 1,358 executable/configuration inputs. No commit
was made. The final image manifest is
`target/fastlane-m4-evidence/build-20261101T221023.626575Z/validation-summary.json`.

| Configured / online CPUs | Forced peer CPUs | Pairs | Registrations | Physical parks | Completions / collections | Cross-CPU completions | Outcome |
|---|---|---:|---:|---:|---:|---:|---|
| 2 | 0,1 | 30 | 33 | 32 | 32 / 32 | 31 | PASS |
| 4 | 0,1 | 30 | 33 | 32 | 32 / 32 | 31 | PASS |
| 12 | 0,1 | 30 | 33 | 32 | 32 / 32 | 31 | PASS |

The remaining registration belongs to the deliberately killed blocked
consumer; confirmed ProcessExit/reaping and zero terminal registrations are
required instead of waking a dead task. Each physical-park witness checks
`BlockedOnIo`, owner/queued CPU both 255, no physical stack pin and zero ready
queue occurrences. The registration race intentionally also exercises the
interval before physical parking. All six forced races include actual remote
failed lock acquisition, normal authorization decisions and ordinary syscall
outcomes. Cross-CPU count refers to registration CPU versus completion CPU;
consumer collection separately witnesses resumed userspace execution.

Original final host receipts:

* 2 CPUs: `target/fastlane-m4-evidence/native-20261101T221103.914774Z/2cpu.host-ack.json`.
* 4 CPUs: `target/fastlane-m4-evidence/native-20261101T221230.669380Z/4cpu.host-ack.json`.
* 12 CPUs: `target/fastlane-m4-evidence/native-20261101T221230.669380Z/12cpu.host-ack.json`.

Each guest ends with one retained tombstone row, capacity eight, active
channels/waiters/receipts/pending wakes all zero. Actual channel row size is
336 bytes and receipt value size 56 bytes. The bounded channel buffer is at
most 2,688 bytes of logical row storage, excluding allocator overhead, the
existing Vec header/token ledger and process table. The 16 repeated cycles
separately require zero terminal resource counts.
M4 creates no SHM backing or mappings; full M2/TLB accounting belongs to the
independent regression matrix.

Final host checks are
`target/fastlane-m4-evidence/checks-20261101T221105.074609Z/validation-summary.json`:
155 kernel tests, nine Doorbell tests, 62 IPC tests, 18 Account tests, focused
scheduler tests, 29 extracted dispatch/handoff tests and parser controls PASS.
Controlled busy ICRs dispatch within the existing 4,096-attempt budget; retained
obligations on exhaustion are tested as diagnostic fail-stop cases. The release
feature command exits 101 with the existing debug-only compile guard; its
expected-rejection receipt is PASS, not an accepted release feature build.
Its additional missing-`sched` scope diagnostics match the existing unsupported
M1 release-feature configuration exactly; the original compiler output and
comparison are retained. Supported ordinary release validation is separate.
`target/fastlane-m4-evidence/audit/` retains that original failure output,
dependency-tree exclusion of Ring, source/unsafe audit and new-module rustfmt
receipts. No unsafe code is added to broker state or lifecycle handling; the
only kernel unsafe boundary is the reused stack trampoline and its dispatcher
call. Userspace unsafe code is the normal syscall ABI, not shared-data access.

Each attempt has independent timestamps, source hashes/copies, commands, image
digests, exact embedded service bytes, extracted ISO ELF, serial logs, outcomes
and artifact hashes. Native acceptance requires an unchanged complete source
map and original host receipt. Parser controls are explicitly synthetic.
Development failures include an actor-command collision, rejected source drift,
sandbox QMP socket denial and an unplaced debug rendezvous timeout. They remain
failed attempts and are not rewritten as accepted native executions.

## Final regression decision and identities

| Gate | 2 CPUs | 4 CPUs | 12 CPUs | Qualification |
|---|---|---|---|---|
| M4 independent authenticated peers | PASS | PASS | PASS | Final source/image, original host ACKs above |
| M1 authority + M2 mapping/retirement + native width probes | PASS | PASS | PASS | Unchanged census thresholds; fresh results only |
| IPC deferred-call/reply races | PASS | PASS | PASS | Existing fixture scope, separate from M4 authentication proof |
| Authenticated S1/S2 grant contention | PASS | PASS | PASS | Six real authenticated guests |
| Account exec and actual-exit modes 11/12/13 | PASS | PASS | PASS | Twelve lifecycle guests |
| Scheduler lifecycle + R2D remote wake | PASS | PASS | PASS | Existing pin/queue/handoff regressions |
| Existing F1.3N kernel Doorbell fixture | PASS | PASS | PASS | 42 rounds each, 43/43 objects, zero final resources, equal frame counts |
| M3.1B repaired reclamation | PASS | PASS | PASS | All eight boundary crossings reclaim empty tables; VM teardown verified |
| M3.1B pre-repair control | PASS | PASS | PASS | Expected retained-table mechanism reproduced; not a production gate or historical-failure reclassification |

The broader matrix contains 33 guests, all PASS, in
`target/fastlane-m3-evidence/regressions-20261101T221231.817901Z/orchestration.json`.
It retains exact commands, individual manifests/ACKs, sources, images, hashes
and original serials. M3.1B repaired and control manifests are respectively:

* `target/fastlane-m4-evidence/m31b/native-20261101T223704.537142Z/validation-summary.json`.
* `target/fastlane-m4-evidence/m31b-control/prerepair-native-20261101T224013.161782Z/validation-summary.json`.

The normal production image has an empty experimental feature list and exact
normal embedded service bytes. Both its ELF and ISO-extracted ELF contain no
M4 markers or adapter symbols; the audit is
`target/fastlane-m4-evidence/audit/production-exclusion.json`. The full supported
ordinary `cargo build --release` succeeds, with exact normal embedded services
and no M4 markers. Its ELF is fully stripped, so its symbol test is **NOT_RUN**;
the ordinary debug production ELF provides the independent symbol audit.
The full release receipt and retained ELF are indexed by
`target/fastlane-m4-evidence/audit/ordinary-release-build.json`.
Preserved pre-existing M3.2 host tests and five doctests also PASS; their log is
`audit/preserved-m32-host.log` under the same evidence root.

Final native ELF SHA-256:
`bb77f012aae52f03bc9a15f8da6ac9156daf25f1ac647ae38bf96d5685b13246`.

Final native ISO SHA-256:
`d80636fb5935284e220b1ed8ff8624da6255511a982af872f0273f7ff69b3248`.

Canonical executable/configuration source-map SHA-256 (sorted compact JSON):
`1657745af54bdf84571cc49a9507eb3be3229f76c7ea06dec44581e0d15ad8c5`.
Documentation is archived separately; editing this report does not change
runtime input hashes. The complete receipt join is
`target/fastlane-m4-evidence/validation-summary.json`.

| Separate status | Decision |
|---|---|
| Current M4 functionality and requested fresh regressions | PASS / COMPLETE |
| Historical census failures | FAIL observations retained, root attribution unresolved |
| Historical M1 debug rendezvous timeout | FAIL observation retained, repair remains open |
| Overall regression readiness | PARTIAL; fresh passes do not discharge historical failures |
| Production FastLane / public Doorbell / native Ring | BLOCKED; disabled |
| Native Ring payload transfer and stronger publication/authenticity claims | NOT_RUN / not implemented in M4 |
| Physical busy-ICR injection in authenticated guests | NOT_RUN; controlled extracted-code host tests PASS |

## Preserved boundaries and next slice

M3.2 memory safety and private snapshot stability remain provided. Coherent
publication and application authenticity remain **not provided**. A Doorbell
authenticates who may signal; it says nothing about hostile payload bytes.
Historical SHM/page-table census failures and the M1 debug rendezvous timeout
remain unresolved, independent debt. Historical D3/N3M and broad production
security gates retain their prior classifications.

After M4 acceptance, the smallest M5 slice is one opt-in native, single
outstanding bounded small-message copy between these exact authenticated peers:
use M2's unchanged asymmetric pages, M3.2's private initialized staging and
bounded hostile-word backend, then this Doorbell for empty/full notification.
Start with one fixed slot and a small fixed payload, checked channel/slot/sequence
identity, explicit commit/reclaim and no batching, leases into shared bytes,
large pages or production integration. Report the result as untrusted stable
private bytes, never coherent/authentic publication. Force mutation during copy,
exit/revoke during access, stale-slot reuse and notification races, with operation
admission/quiescence before unmap. A stronger immutable-publication profile needs
a separate ownership/sealing protocol and its VM/TLB proofs first.
