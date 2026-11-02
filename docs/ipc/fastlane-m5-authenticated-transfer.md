# M5 first authenticated native message transfer

2026-11-02. Experimental, debug kernel only. Production FastLane remains disabled.
No general queue, batching, pipelining, performance optimization, commit or push.
This report supersedes earlier milestones' statements that native payload
transfer is absent; their original results and failures remain historical facts.
The M5 name follows the current task, rather than the older checkpoint's proposed
production-prerequisites numbering.

## Source boundary and implementation

Starting HEAD: `91a28f22aaa17d3e365398cc31f7d7db662e54e5`, plus the existing
uncommitted M4 tree. All 1,358 executable/configuration inputs exactly match the
accepted M4 build `build-20261101T221023.626575Z`. The initial source copies,
working diff, reports and comparison are retained in
`target/fastlane-m5-evidence/baseline/identity.json` and `baseline/source/`.
Existing M4 changes are preserved. Final identity and results are recorded below.

`fastlane_transfer_test` implies the unchanged M1/M2/M4 gates. Its only new kernel
entry points are private `0xf1c1` operations 18 (load) and 19 (store). Their
arguments are the complete M1 handle, a fixed page-relative word selector and a
scalar store value. They accept no userspace pointer, byte length or SHM bearer.
The UAC fixture enables `fastlane-transfer-test`; its optional dependency links
the existing hostile representation into userspace. The kernel does not link
`fastlane-ring`. Production images have neither new selector behavior nor code.
The existing release guard rejects the implied experimental channel feature.

The backend deliberately mediates each word through the kernel. This is a
native userspace-to-userspace message using actual M2 backing and independently
authenticated VMs; it is **not direct userspace mapping access for the copy**.
Userspace runs the producer representation, metadata validation, private-copy
acquisition and acknowledgement. Kernel word accesses use the exact backing of
the caller's recorded M2 mapping. This trades syscall overhead for a small
lifetime proof; no performance or strict end-to-end zero-copy claim is made.

`kernel/src/capability/channel_transfer.rs` uses existing
`SCHEDULER -> CAP_BROKER` serialization, with no new lock or allocation.
Every access performs the existing session/expiry/peer/issuer/VM sweep, exact
handle lookup, immutable role authentication, full attachment and role mapping
checks, access-direction checks, and exact backing/view lookup. Only descriptor
zero's six words, consumer cursor zero, and 32 payload words are admitted.
The selected frame must remain one initialized page. An aligned MOV accesses a
canonical word of its HHDM alias while both existing locks remain held. There is
no pointer retained after return, no Rust reference to hostile storage and no
lease that pins a native mapping between syscalls. Revocation between accesses
makes the next access fail; an in-progress native access finishes before unmap.
The backend cannot access a freed mapping or backing.

## One-message lifecycle and identities

1. Producer independently requests Account consent and proposes a fresh M1
   channel. Consumer independently obtains its exact consent receipt.
2. The exact producer consumes the receipt, creating the immutable pair.
3. Each process attaches its own role exactly once, with distinct process
   incarnations and VM generations. The controller has no channel membership.
4. Each role installs all three real M2 views, preserving producer-writable
   descriptor/payload, consumer-writable acknowledgement, other-role read-only
   views and NX. Existing sharing/alias exclusions remain in force.
5. The producer fills the existing initialized 256-byte staging lease. Commit
   writes its bounded payload and descriptor with the fresh trusted channel
   generation, message sequence 0 and slot generation 1.
6. Publishing cursor 1 admits the only transition `Writable -> Published` in a
   kernel-private inline byte in the existing channel row. Duplicate publication
   is refused; other ring slots are unavailable. Directly forging a writable
   shared cursor cannot admit publication: the consumer sees Empty while this
   private state is Writable. Direct bytes never grant authority.
7. The producer invokes ordinary authenticated M4 notify after commit.
8. The consumer observes notification sequence, checks publication, validates
   descriptor identity/offset/length into private scalars and copies complete
   aligned words into the existing initialized private lease buffer. Shared
   offsets never select an address. Native copy failures produce no delivered
   lease; malformed data terminally retires the local representation.
9. After application examination of its private copy, the exact consumer releases
   the lease, storing cursor 1 and admitting `Published -> Acknowledged`. A failed
   release after closure reports failure, despite the already stable private
   snapshot; it never asserts successful consumption.
10. Either member closes using existing M1 revocation. M4 waiters/completions,
    M2 mappings and SHM backing follow existing retirement. Physical reap may
    follow logical exit and waitpid; M2's bounded census wait observes completion.
    Only then is this cycle counted as retired. A fresh cycle needs a new token
    **and** channel generation, not reset cursors or a reconnected local role.

Full token and channel generation authenticate kernel operations; participant
PID/incarnation/VM plus session/issuer authenticate roles. Shared descriptor
identity is `(channel_generation, sequence=0, slot_generation=1, slot=0)`.
Generations do not wrap under the existing checked allocators. Diagnostic logs
never print channel tokens or consent receipts. Host receipts join the exact
channel/message identities to immutable role bindings and physical objects.
There is at most one admitted message in the entire lifetime of an M5 channel.

## Publication, sleeping and integrity

The aligned x86-64 MOV backend uses compiler Release/Acquire fences and normal
x86 TSO ordering. Payload and descriptor stores precede the publication cursor;
publication finishes before the producer's notification syscall. The syscall
assembly has memory effects (no `nomem`), so compiler reordering across it is
not permitted. Consumer Acquire word loads and existing lock serialization
preserve this ordering for a cooperating producer.

The consumer observes the M4 sequence **before** testing publication. If Empty,
it conditionally waits on that observed sequence. Notification after observation
but before registration produces Changed; notification during/after registration
uses M4's serialized receipt/wake transaction. Every wake and Changed response
rechecks publication and metadata. A notification without a message is spurious
and causes another conditional wait, not a receive. There are at most 16 wake
iterations in this fixture; the representation has bounded work per operation.
M4 cancellation releases the current wait; channel close/revocation/peer exit
makes subsequent data admission fail and terminally releases a sleeping peer.
Notification identity never replaces channel/message validation. M4's forced
registration races and remote scheduler dispatch remain independent regressions.

An authorized producer retains a writable payload/descriptor alias after
publication, including access outside this backend through its own M2 view.
It can mutate during copying or forge structurally valid content. The returned
`Integrity::PRIVATE_COPY` therefore promises **memory safety and private snapshot
stability only**. **Coherent publication and application authenticity are not
provided.** Acquire loads, authenticated Account roles, the private publication
state and Doorbell sequences do not remove that alias or establish coherence.

The consumer returns the original M3.2 private ReadLease, not `try_recv` into an
extra buffer. Its initialized fixed 256-byte array needs no heap allocation,
so allocation is bounded without an infallible dynamic allocation. M2 setup
retains its existing fallible frame/vector reservation. There are no unchecked
peer-byte-to-reference conversions, shared mutable Rust references, shared
payload slices or uninitialized-byte exposure. Remaining native words are copied
using validated local bounds even if descriptor words change after validation.

## Native fixture and adversarial scope

`services/sunlight-uac/src/bin/channel_fixture/transfer.rs` uses ordinary IPC to
coordinate producer, consumer and foreign processes, pinning independent peers
to CPUs 0/1 under M4's existing test-only placement. Every channel repeats real
Account authorization and independent attachments. The host never acts as a
producer, consumer or authority issuer. Userspace assertions and host joins
require actual consumer execution, exact kernel publication/ack events, private
snapshot witnesses, logical exit acknowledgements and physical retirement.

Each intended guest contains 20 pairs, eight complete repeated successful cycles,
18 admitted publications, 10 acknowledgements, 11 stable private-copy witnesses,
60 backing objects and 120 mappings. The non-success cases deliberately produce
fewer acknowledgements than publications. Per-case outcomes are independent:

| Exercise | Expected native result |
|---|---|
| Eight complete cycles | Notify without publication wakes/rechecks/sleeps; later publication/notify transfers 256 bytes; producer observes acknowledgement; all ownership retires |
| Unauthorized producer/consumer and foreign exact handle | M1 role denial before word access; invalid selector and stale channel generation also denied |
| Duplicate publication | Kernel Duplicate for second publication cursor; representation cannot publish a second message |
| Invalid length, offset, stale slot generation, wrong channel generation in descriptor | Bounded Length/Offset/Identity error, no copy receipt or ack |
| Mutation during copying | Consumer pauses after first payload load; actual producer replaces all payload plus length/offset; consumer returns old first word plus new remaining words using saved local bounds |
| Mutation after copying | Consumer holds its private lease while actual producer overwrites all shared bytes; all private bytes remain unchanged |
| Close before ack / ack before close | First case has stable private snapshot but release fails Closed and produces no ack; complete cycles demonstrate the opposite order |
| Revoke during copying | Close after first payload load; next access fails Closed, no delivered copy or ack |
| Cancellation during pending transfer wait | M4 Cancelled receipt collected; no publication or copy; close retires resources |
| Producer exit before / after publication | Actual exit/waitpid, consumer Terminal; unpublished or unnotified data cannot survive retired authority |
| Consumer exit before copying | Actual exit/waitpid; subsequent producer notify Terminal; backing drains |
| Remote retirement of transferred channel | Existing M2 sacrificial active-VM stale-load probe; real remote flush ACK, expected user fault and waitpid status 139 before final census |

The during-copy rendezvous overlaps real userspace operation lifetimes; it is
not a host-thread substitute. Lifecycle boundary controls choose real operation
orders using IPC. They do not claim forced simultaneous syscall lock contention.
Publication/notification from the sole non-borrowed producer is ordered by its
program; deliberately notifying before publication exercises the hostile order.
The independent M4 suite supplies actual forced registration-lock contention.
The final sacrificial raw-access fault probe is separate from the mediated copy
backend and reuses M2's existing unsafe probe without adding raw userspace access
code to the transfer path.

## Physical retirement and bounded resources

Every M5 backing frame and role view uses existing `[FL-FRAME]`, `[FL-MAP]` and
`[FL-TLB]` events. The M5 host assessor requires both exact authorized views,
rights/NX, matching unmap request/completion, request-qualified complete remote
ACK sets, and exactly one final free after both views complete. Later backing
reuse must follow that free. Actual nonempty target masks and zero-target
unmaps are counted separately; zero-target completion is never a remote ACK.
The active remote consumer case requires an actual handler flush and correlated
stale-VA fault. Eight complete cycles require unchanged global SHM/table counts
without relaxing historical thresholds, plus zero channel-specific inventories.

There is no new allocation per word/message/wait, retained pointer or growing
message list. One inline transfer-state byte increases the debug channel row
from 336 to 344 bytes with padding; the existing table capacity remains eight.
Retired tombstones, bounded Vec capacity and the existing boot-lifetime token
nonreuse ledger are retained metadata, not live channel/grant/frame ownership.
Active channels, channel pair receipts, Restricted backing grants, mappings,
waiters, pending wakes and process completion receipts must all be zero at cycle
completion. The authenticated login session and unrelated system SHM remain live;
this is not whole-system heap/service census reconciliation.

## Validation receipts and decision

**M5 transfer functionality: COMPLETE on the final native matrix. Overall
security/regression readiness: PARTIAL. Production enablement: BLOCKED.**
The source-bound machine-readable aggregate is
`target/fastlane-m5-evidence/validation-summary.json`. Each original attempt has
its own source map/copies, command/environment logs, embedded service bytes,
ELF/ISO/extracted ELF hashes, original serial, assessment and artifact index.
An independent host ACK is emitted only for a passing unchanged-source guest.
Parser controls are synthetic and never substitute for native evidence.

Reproduction:

```sh
python3 tools/fastlane_m5_validate.py --section build
python3 tools/fastlane_m5_validate.py --section native --cpus 2 --image-manifest <build/validation-summary.json>
python3 tools/fastlane_m5_validate.py --section native --cpus 4 12 --image-manifest <build/validation-summary.json>
python3 tools/fastlane_m5_validate.py --section checks
python3 tools/fastlane_m5_validate.py --section production
```

Historical SHM census drift and M1 debug rendezvous timeout remain independent
unresolved debts. Miri is NOT_RUN because unavailable. A later successful replay
does not discharge an earlier FAIL. Overall security/regression readiness remains
PARTIAL; production enablement is BLOCKED. Stronger coherence would require a
separate enforced page-ownership/write-revocation design. No performance work
begins as part of M5.

## Final M5 native matrix

All peers are independently authenticated native processes with different VMs.
Configured CPU counts and all online AP identities are independently checked;
producer/consumer placement is CPU 0/1 on each topology. No claim is made that
the channel itself fans out across all configured CPUs.

| Configured / online CPUs | Pairs | Publications | Acks | Stable copies | Remote wakes | Backing objects / views | Remote / zero-target unmaps | Result |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| 2 | 20 | 18 | 10 | 11 | 17 | 60 / 120 | 15 / 105 | PASS |
| 4 | 20 | 18 | 10 | 11 | 17 | 60 / 120 | 9 / 111 | PASS |
| 12 | 20 | 18 | 10 | 11 | 17 | 60 / 120 | 18 / 102 | PASS |

Each guest executes eight successful complete transfer/retirement cycles, four
malformed descriptor cases, native hostile mutation, lifecycle boundary cases
and the remote stale-view probe. All 60 frames are released exactly once after
their two mappings and required remote acknowledgements. All twenty terminal
censuses have zero Restricted objects, views and frames. Final channel, waiter,
completion-receipt and pending-wake inventories are zero. The existing pair
receipt is consumed at creation; no new transfer grant or receipt class exists.

Final image manifest:
`target/fastlane-m5-evidence/build-20261102T115514.013783Z/validation-summary.json`.
Original native manifests and independent host acknowledgements:

* 2 CPUs: `native-20261102T115533.557228Z/validation-summary.json` and
  `native-20261102T115533.557228Z/2cpu.host-ack.json`.
* 4/12 CPUs: `native-20261102T115845.602609Z/validation-summary.json` and
  `native-20261102T115845.602609Z/{4,12}cpu.host-ack.json`.

These paths are relative to `target/fastlane-m5-evidence/`. Each guest has a
`<cpus>cpu.physical-objects.json` containing the exact backing/view/unmap/TLB/free
join; original serials and artifact hashes remain alongside it. Host ACKs retain
exact channel/message cases and consumer private-copy receipts.

The final source contains 1363 executable/configuration inputs.
Canonical source-map SHA-256 (sorted compact JSON):
`95b5497d873d082373f3c8c69d1107e3706d8702a32b492c4e734f3df2f49158`.
Final experimental ELF SHA-256:
`674d3ecee45bdb3bb582180d7cec267071ef2519e79ebf9e339caaf2b42842c1`.
Final experimental ISO SHA-256:
`c558e6db347ba7c616efd926546b7e0f1cb9b03d7ce457fea40029116d6a49f1`.
Documentation is archived separately; report edits do not change runtime inputs.

## Host checks, failures and limitations

`checks-20261102T115526.620067Z/validation-summary.json` records PASS for 155
kernel tests, 39 Ring library tests and five doctests, nine Doorbell tests,
62 IPC tests, 18 Account tests, focused scheduler and extracted dispatch tests,
15 M5 parser controls and existing mapping parser controls. Synthetic parser
controls test rejection and are not native evidence.

`m32-final/host-20261102T120159.471892Z/validation-summary.json` records PASS for
the preserved M3.2 host suite, strict crate Clippy, bare-metal crate check,
formatting, all FastLane parser controls, ASan and TSan (27 hostile tests each).
Miri is independently NOT_RUN, with its failed availability log retained.
`audit/source-audit.json` records PASS for the two new Rust modules' formatting,
source equality, native online topology, dependency exclusion and unsafe review.

Original failed attempts are retained with their own source copies and results:
QMP socket creation was denied under the sandbox; a sandboxed ASan execution
passed its 27 test assertions but LeakSanitizer failed under ptrace. The
subsequent unsandboxed sanitizer pass does not rewrite that command failure.
An intermediate native 2-CPU run failed its fixture assertion after waitpid
while a producer was not yet physically reaped. That assertion was corrected to
use the existing M2 bounded physical-retirement census wait, without changing
kernel retirement or any acceptance threshold. Its original failure remains in
`native-20261102T115212.910390Z/`. Earlier successful development-source guests
are retained but are not substituted for the final-source matrix above.

The fresh broad regression run also reproduces the historical M1 debug
rendezvous deadline on 2 CPUs (mode 2, entered=1, second CPU never entered).
The original FAIL is in
`target/fastlane-m3-evidence/regressions-20261102T115540.099045Z/compatibility-2/`.
An identical-image independent replay passes in
`mapping2-replay/native-20261102T120305.862165Z/`, including the downstream M2
hostile-access/retirement and M3 word gates. It supplies missing coverage, not
causal closure. Historical census drift, this debug progress debt and unavailable
Miri remain three distinct issues. No failed native gate is relabelled PASS.

A separate fresh M4 4-CPU attempt also FAILS after producer exit: generation 9
commits a terminal completion, then the serial ends with an instruction-fetch
page fault at `rip=0`, `rsp=0x7fffffffd3c0`, error `0x10`. The early diagnostic
PID is unavailable (`u64::MAX`); it does not identify a particular process.
There is no matching completed guest/consumer collection. This is an unexplained
new failure, separate from the historical M1 timeout and census drift. The
original serial, failed assessment and source/image manifest are retained in
`m4/native-20261102T122333.183105Z/`. An independent 12-CPU execution and an
identical-image 4-CPU replay are recorded separately; a passing replay cannot
close this failure or promote the M4 4-CPU aggregate to PASS.

The next executable milestone is to diagnose the M4 producer-exit/terminal-wake
failure with exact outgoing/resume frame and physical stack-ownership witnesses,
without substituting a process identity for the unavailable fault PID. Repair
requires a causal explanation and the relevant authenticated 2/4/12 regressions.
Separately repair and validate M1's debug rendezvous progress without changing
authorization or required contended-lock witnesses, then causally reconcile the
retained historical census additions using exact frame/VM ownership. Production prerequisites and any stronger coherent message
publication mechanism remain separate work. No performance optimization is
included in this implementation or validation.

## Complete regression and exclusion matrix

| Required gate | 2 CPUs | 4 CPUs | 12 CPUs | Qualification |
|---|---|---|---|---|
| M5 authenticated transfer / hostile metadata / mutation / retirement | PASS | PASS | PASS | Final-source independent host ACKs above |
| Original M1/M2/M3 compatibility suite | FAIL | PASS | PASS | 2-CPU M1 rendezvous deadline; downstream M2/M3 stage initially NOT_RUN |
| Independent identical-image M1/M2/M3 replay | PASS | NOT_RUN | NOT_RUN | 2-CPU missing downstream coverage supplied; original FAIL unresolved |
| IPC deferred/reply, VM and SHM lifecycle | PASS | PASS | PASS | Original authenticated/source-bound regression receipts |
| Account S1/S2 authority contention | PASS | PASS | PASS | Both cases per topology |
| Account exec and actual-exit modes 11/12/13 | PASS | PASS | PASS | Three cycles per topology/mode |
| Scheduler lifecycle, R2D remote wake, kernel Doorbell | PASS | PASS | PASS | Separate existing native mechanism fixtures |
| Fresh M4 authenticated Doorbell, original attempt | PASS | FAIL | NOT_RUN | Runner stopped after unexplained 4-CPU fetch fault |
| Fresh M4 independent remaining topology | NOT_RUN | NOT_RUN | PASS | Original independent 12-CPU host ACK |
| M4 identical-image diagnostic replay | NOT_RUN | FAIL | NOT_RUN | Timeout after generation 2 registration-race completion, before consumer take |
| M3.1B reclamation / teardown | PASS | PASS | PASS | Zero retained empty P1 at all eight crossings |
| M3.1B pre-repair control | PASS | PASS | PASS | Expected retention mechanism; not a production gate |

Broad regression commands, individual manifests/host ACKs and their original
artifact index are in
`target/fastlane-m3-evidence/regressions-20261102T115540.099045Z/orchestration.json`.
Its aggregate remains FAIL, with unchanged source hashes and the 2-CPU M1
failure retained. The separate M4 manifests are:

* Original 2/4 CPUs: `m4/native-20261102T122333.183105Z/validation-summary.json`.
* Independent 12 CPUs: `m4-independent12/native-20261102T122843.666809Z/validation-summary.json`.
* Diagnostic 4-CPU replay: `m4-replay4/native-20261102T122843.668828Z/validation-summary.json`.

The replay's failure is distinct: it commits a completion during registration
race mode 1 (generation 2), then never reaches consumer collection or terminal
userspace completion before the 180-second host deadline. No new fault line is
observed in that replay. Both 4-CPU failures remain unresolved. The core M4
Doorbell/scheduler/process modules compared in `audit/m4-unexplained-fault.json`
match the accepted M4 baseline; this comparison does not establish causal
independence from the current image or attribute either failure to an actor.

M3.1B originals are
`m31b/native-20261102T122637.411823Z/validation-summary.json` and
`m31b-control/prerepair-native-20261102T122714.272366Z/validation-summary.json`.
These are boot-time kernel/VM mechanism regressions, not substitutes for the
independent native userspace M5 transfer receipts.

`production-20261102T122714.288618Z/validation-summary.json` and
`audit/production-exclusion.json` record PASS for empty ordinary feature lists,
exact normal embedded service bytes, absence of experimental markers in ELF and
ISO-extracted ELF, ordinary debug production symbol exclusion and a successful
full supported ordinary release build. Release ELF marker/service checks PASS;
its symbol inspection is independently NOT_RUN because fully stripped.
`audit/release-feature-rejection.json` records the expected exit-101 debug-only
compile guard. Its 32 diagnostics match the retained M4 unsupported release
feature configuration; this is not a successful experimental release build.

| Separate decision | Result |
|---|---|
| M5 demonstrated single-message fixture functionality | COMPLETE / PASS on 2/4/12 |
| M4 current 4-CPU native regression | FAIL, two original attempts retained |
| Historical census drift | UNRESOLVED; no causal closure claimed |
| M1 debug rendezvous | FAIL reproduced; independent passing replay retained |
| Miri execution | NOT_RUN; unavailable |
| Overall security and regression readiness | PARTIAL |
| Production FastLane transfer | BLOCKED / DISABLED |
| Commit, push, performance optimization | NOT_PERFORMED |
