# M6.1B-R2 — hybrid lifecycle security validation

Status: **PARTIAL**. Production FastLane is DISABLED.
No production readiness, Ring 3 integration acceptance, commit or push follows
from this fixture. Evidence is retained under `.fastlane-evidence/m61b-r2/`.
Final canonical code-source SHA-256:
`44a37f07d9463a525822b86fcc424a0502ee3de8e850fa9487705fe1b3d97a19`.

The hardened source passes focused 2-CPU acceptance and two complete 2/4/12-CPU
matrices. All four original controls pass. Seven accepted guests demonstrate
98 unique operation terminals, 98 zero active-resource inventories, 294 ordered
hybrid backing releases and seven additional remote-witness releases. Both
supported race winner orders occur in every guest. R2 remains PARTIAL because
concurrent successful exec is not demonstrated under the existing exclusive
service-owner constraints; numeric PID reuse remains an explicit boundary.

## Preserved source and evidence

Resume HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
`resume-20261104T111620.516550Z/baseline.json` contains the dirty status,
per-file source hashes and canonical source hash
`0ad3c7c807d94152beb173a458988d9607ad3f5f5fc4faaac9d6dee81f7d5349`.
The adjacent binary patch preserves tracked changes. Existing untracked source
and prior evidence were preserved. Source hashes use the existing driver's
selection and sorted compact JSON SHA-256 convention; documentation and ignored
evidence are outside that code identity.

The original outstanding controls retain their identities: stale policy
generation; invalid directional contract; caller exit before IPC reply; peer
closure during transfer. Cancellation before establishment is additional
coverage. Earlier R1E-R1 and R2 results have different source identities and
are historical evidence only. The successful R1E-R1 pilot was not rerun.

## Operation state and terminal decisions

The supported operation is single use: `TRANSFORM_16`, request
`FastLaneRequired`, reply `KernelIPC`. Authentication and operation permission
remain separate from transport selection. M1 membership remains unchanged.

| State | Transition and authority |
| --- | --- |
| Proposed/consented | Existing Account authentication binds exact creator and consumer incarnations and VMs. |
| Establishing | Exact directional encoding starts the contract in Preparing; cancellation fences commit. |
| Attached | Both channel roles and restricted mappings are attached using existing admission checks. |
| Ready | Contract finish commits the required native request transport. |
| Request published | Producer publishes the ring Frame; Doorbell carries notification only. |
| Reply authority bound | Operation 35 authenticates the actual IPC requester and binds its endpoint/call identity to the channel under SCHEDULER and CAP_BROKER. |
| Private receipt/execution | Actor defers its real IPC target, privately copies and validates the ring Frame, and executes the operation. Execution alone is not terminal publication. |
| Reply publication | Existing deferred completion consumes the Actor-owned token, validates caller identity and parent channel/policy, then attempts the IPC terminal decision. |
| Completed | `finish_call` records ReplyDelivered for the original pending generation and clears its channel correlation. |
| Cancelled | `finish_call` records ExplicitlyCancelled and fences the channel and retained reply target. |
| Closed/revoked | Channel retirement sets Retired/Closed under existing serialization; associated reply publication immediately fails the parent check. Metadata draining may follow. |
| Caller exit | Logical exit prevents delivery to the old incarnation; outgoing IPC retirement removes retained targets without publishing into a replacement image. |
| Service exit/exec | Logical death prevents completion; successful exec retires old incoming/outgoing reply authority and image roles. Failed exec preserves old authority. |
| Policy revoked | Catalog withdrawal makes the bound generation unresolvable; later publication fails. Reused slots have a new nonwrapping generation. |
| Final retirement | Pending IPC, reply metadata, channel receipts, waiters, restricted views, SHM objects and physical frames drain through their existing lifetimes. |

Completion linearizes at the existing serialized `finish_call` terminal decision.
Ring revocation linearizes when `Channel::retire` fences the row; policy revocation
linearizes at catalog withdrawal. Completion and these decisions hold the existing
scheduler/capability lifecycle guards, so revocation cannot slip between the parent
check and accepted delivery. A completed pending generation cannot be completed
again. Parent revocation makes the retained token unusable immediately, while
`fastlane_hybrid_sweep` independently drains pending/deferred metadata. A row with
an undrained hybrid target cannot be recycled. No new global lock is introduced.

## Resume findings and changes

The dirty tree already contained the channel/reply lifecycle bridge, native
contention fixture and remote SHM witness. These were audited before editing.
The bridge checks channel state and policy identity in `deliver_reply_to_target`,
binds the exact IPC target during operation authorization, and clears that binding
at the existing IPC terminal decision. Sequential host tests alone do not prove
its concurrency behavior.

The first resumed image built successfully. Its sandboxed native attempt failed
before boot because QMP socket creation returned `Operation not permitted`;
the attempt remains at `native-20261104T111720.178089Z`. The approved native retry
at `native-20261104T111737.840884Z` reached caller-exit mode 9 and failed with
`hybrid lifecycle bounded contention timeout`: the supervisor's polling child
wait kept CPU 0 runnable, preventing its idle method contender from entering.

The bounded supervisor wait now parks on its endpoint between child probes so
the contender can enter. Final inventory sampling waits within the existing
bounded fixture for actual metadata retirement: native waitpid accepts Finished
as well as Reaped and does not itself prove physical cleanup. Only zero final
inventories are emitted as final receipts. Raw channel capability output was
removed from the hybrid authorization diagnostic; generation and authenticated
identity correlations remain.

`native-20261104T112200.716070Z` preserved a second distinct failure: after
actual mode-9 contention and caller resource retirement, the supervisor tried
to reap the Actor, whose parent is the original caller. That request correctly
failed child ownership checks. The supervisor now requests Actor exit through
its already delegated endpoint, then observes scoped retirement without
adopting or reparenting the Actor. This attempt also recorded actual CPU-1
ACKs for three hybrid restricted views, with target mask 2 and physical release
after unmap completion. These observations remain within an overall failed run.

## Evidence boundaries

Modes 1/2 contend completion with cancellation; 3/4 with ring closure; 5/6
with policy withdrawal; 9/10 with caller exit; 11/12 with service exit;
13/14 with channel versus pending-reply retirement. Odd/even modes select
opposite winners using bounded coordination and actual failed scheduler
`try_lock` on distinct native CPUs. Retirement contenders invoke real kernel
methods against authenticated Ring 3 state. They are not foreign syscalls or
proof of two independent authorized userspace syscall owners.

Modes 7/8 exercise actual service exit and exec with outstanding reply authority.
Exec is sequential here; concurrent successful exec versus completion remains
NOT_RUN. The single service owns both its exec and completion syscalls; a
separate safe same-incarnation exec contender is not established by this fixture.
Numeric PID reuse is NOT_RUN; exact incarnation and generation rejection must
not be described as numeric reuse proof.

The separate remote witness activates an ordinary SHM address space on actual
APs, verifies pre-touch content, withdraws its mapping, observes target-specific
invalidation acknowledgements, then frees/reuses its backing and verifies
replacement content. It is a kernel SHM prerequisite, not a remote target on a
hybrid channel's own restricted view. Zero-target channel unmaps do not count
as remote ACK success.

## Accepted execution before physical observer hardening

Image: `build-20261104T112424.620090Z/validation-summary.json`. Focused Stage 1:
`native-20261104T112609.665882Z`, 2 CPUs, PASS. Stage 2 began only after that
acceptance: `native-20261104T112814.984038Z`, 2/4/12 CPUs, all PASS. A second
Stage 2 execution is retained at `native-20261104T113040.501364Z`; its final
results are recorded in the source-bound resume acceptance summary.

These guests verify source, ELF, ISO and embedded service identities.
Fourteen authenticated operations each have exactly one joined kernel terminal
receipt. Five commit ReplyDelivered, one ExplicitlyCancelled, seven PeerClosed,
and one CallerRetired. In mode 9 delivery commits before caller exit, but the
caller never resumes to accept the Frame; this is a kernel terminal decision,
not a userspace acceptance claim. Live successful callers verify the exact Frame.

| Pair / mode | First decision | Terminal / losing operation |
| --- | --- | --- |
| Completion/cancellation, 1 | Completion | ReplyDelivered; cancellation cannot create another terminal. |
| Completion/cancellation, 2 | Cancellation | ExplicitlyCancelled; completion and duplicate fail. |
| Completion/ring closure, 3 | Completion | ReplyDelivered; later ring retirement preserves completion. |
| Completion/ring closure, 4 | Ring retirement | PeerClosed; completion and duplicate fail. |
| Completion/policy revocation, 5 | Completion | ReplyDelivered; later policy withdrawal preserves completion. |
| Completion/policy revocation, 6 | Policy withdrawal | PeerClosed; completion and duplicate fail. |
| Completion/caller exit, 9 | Completion | ReplyDelivered, then original caller exits; duplicate fails. |
| Completion/caller exit, 10 | Caller exit | CallerRetired; completion and duplicate fail. |
| Completion/service exit, 11 | Completion | ReplyDelivered, then service exits. |
| Completion/service exit, 12 | Service exit | PeerClosed; completion and duplicate fail. |
| Pending-reply/channel retirement, 13 | Reply retirement | PeerClosed; channel retirement adds no terminal. |
| Pending-reply/channel retirement, 14 | Channel retirement | PeerClosed; subsequent reply retirement adds no terminal. |

Each paired race records authenticated caller/Actor incarnations, VM identities,
policy ID/generation, channel generation, endpoint/call generation, entry on CPUs
0 and 1, a held lifecycle lock and an actual failed competing try_lock. All
remaining CPUs remain online in the 4/12-CPU runs; this is not a claim that every
CPU independently acts as each hybrid participant.

The first matrix's 2-CPU mode 1 binds caller 39/incarnation 51/VM 88 to Actor
40/incarnation 52/VM 90, policy 1/generation 1, channel 3, endpoint 36/call
generation 12. Every operation's own joins are retained in the serial and parsed
receipts; numeric identifiers are not assumed constant between guests.

| Original outstanding control | Final-source native witness |
| --- | --- |
| Stale policy generation | Each round independently rejects stale policy resolution; reused policy slots reject the original generation. |
| Invalid directional contract | Mode 1 setup rejects the reserved directional encoding and subsequent establishment. |
| Caller exit before IPC reply | Mode 10 forces exit to win on the original caller before deferred completion; CallerRetired and rejected late/duplicate completion. |
| Peer closure during transfer | Service exit with a privately copied request and pending reply, including mode 12's exit-first contention, yields PeerClosed and rejects late completion. This is closure by peer exit, not a peer-initiated channel-close syscall. |

Additional controls cover cancellation before establishment commit, stale channel
generation, reused policy-slot references, duplicate terminal publication, old
Actor token use after real exec, and reattachment after closure. Exact incarnation
key mutation has host broker/reply coverage; numeric PID reuse is not forced.
The later hardened source adds a new native foreign-caller probe, described below.

## Resource and physical reconciliation

Each accepted guest emits fourteen final scoped inventories. All sixteen fields
are zero: channels, receipts, waiters, backing, pending_reply, grants, pending,
deferred, ordinary, views, policies, policy_refs, deliveries, queued, ipc_waiters
and total. Scope is the two authenticated participants and the operation endpoint,
not the entire system. Channel counts and policy-reference counts describe active
authority: bounded retired channel tombstones retain inert correlation/generation
fields until safe row reuse. They are not zero allocated kernel memory.

Each operation joins three SHM backing frames, six restricted mappings, six
unmap requests/completions, their TLB requests/target sets/ACKs, and three
observed physical frees. Thus each guest reconciles 42 hybrid backing lifetimes and
84 restricted unmaps. PMM reuse occurs only after release; metadata zeros alone
are not the physical proof. The separate SHM witness adds one backing lifetime.
An audit subsequently found that this initial observer silently omitted a second
free before reuse and a claim before first release. These earlier runs therefore
remain evidence of observed ordered releases, not exhaustive exactly-once proof.

The first matrix records total nonzero-target ACK events of 16/21/26 for
2/4/12 CPUs, including hybrid views where the Actor VM is still remote-active.
The separate prerequisite's target masks are 2, 14 and 4094 respectively:
CPU sets {1}, {1,2,3}, and {1..11}; each has the exact target ACK set. Its
generation is the trace-only `u64::MAX`, VM 13, VA 12884901888, backing frame
293539840 in these guests. These are observer correlations, not authority
generations or a generation-exhaustion bypass. Every accepted ACK proves a real
flush and observed request sequence before unmap completion and frame release.

## Validation before physical observer hardening

Final-source checks: `checks-20261104T112641.185245Z`. Kernel host 169 tests,
IPC 87, ring 39 plus five doctests, Doorbell nine and UAC 18 all pass. Scheduler
host, dispatch boundary and existing mapping/shared-ring parser controls pass.
The aggregate driver reports FAIL solely for the pre-existing extra blank line
at `services/sunlight-uac/Cargo.toml:691`; it is preserved.

`resume-20261104T111620.516550Z/parser-controls.log` records thirteen passing
R2 parser mutation controls against an accepted final-source native log, including
removed/duplicated terminals, removed contention, missing or unflushed ACK,
missing physical release, nonmatching original control identity and missing
final inventory. These mutations test the parser, not native attack behavior.

`validation-20261104T112642.282239Z` records bare-metal R2 type check PASS,
changed Rust file formatting PASS and ordinary release build PASS. Workspace
formatting FAILS on existing formatting differences. Strict kernel/IPC Clippy
FAILS on unchanged dependencies and IPC `native_poll` returning `Result<_, ()>`;
dependency failures prevent claiming clean strict analysis of the whole kernel.
Miri is NOT_RUN: cargo-miri is unavailable for the installed nightly toolchain.

Production image: `production-20261104T112621.705842Z`, PASS.
`resume-20261104T111620.516550Z/production-exclusion.json` hashes the ordinary
release ELF and production-image ELF and verifies absence of hybrid, R2 race,
inventory, remote, contract, native-ring, mapping, TLB and frame markers. These
fixtures remain opt-in behind debug/test gates. Production routing is disabled.

Final-source existing IPC/deferred-reply/reply-race and remote SHM regressions
are at `ipc-regression/native-20261104T113111.766885Z`: 2/4/12 CPUs PASS.
Shared-ring/channel/Doorbell regression results are recorded separately under
`shared-ring-regression/` and in the resume acceptance summary.

## Hardened final source

The audit finding is retained in the resume directory's
`physical-observer-audit.json`. The opt-in mapping observer now fails stop on a
second tracked free before reuse, and on a tracked claim before release. After
the legitimate first reuse it forgets the old physical trace binding; a newly
registered backing establishes its own lifetime. This changes neither production
PMM behavior nor SHM authorization. The first set of accepted matrices above was
preserved and superseded rather than relabelled as validation of this change.

The hardened native fixture also reuses the existing forwarded-call handler to
ask the actual service for operation authority from a distinct foreign process;
it independently asserts Denied before the real caller's operation starts. A
separate late-token probe rejects stale completion in the Actor and rejects use
of that token from the original caller process. The latter is combined stale/
foreign evidence, not an isolated test of transferring a still-live reply token.
Neither control reruns the R1E-R1 positive pilot or logs reply tokens.

Hardened image: `build-20261104T113744.703803Z`. Its Stage 1 and repeated
2/4/12-CPU acceptance manifests, parser controls, production exclusion, object
traces and final source identity are indexed in the resume directory's
`acceptance-summary.json` and `operation-traces.json`. Earlier source-incompatible
results are never used to accept this source.

| Hardened source run | 2 CPUs | 4 CPUs | 12 CPUs |
| --- | --- | --- | --- |
| Focused `native-20261104T113833.518916Z` | PASS | Not scheduled in Stage 1 | Not scheduled in Stage 1 |
| Matrix `native-20261104T114023.907548Z` | PASS | PASS | PASS |
| Repeat `native-20261104T114245.413418Z` | PASS | PASS | PASS |

Total observed remote-target ACKs for these seven guests are respectively
16; 13/15/29; and 16/15/23. The prerequisite target masks remain 2/14/4094
with exact ACK CPU sets {1}/{1,2,3}/{1..11}. Additional ACKs belong to
remote-active hybrid views and are joined to their individual operation,
VM, VA, region, frame and retirement request in `operation-traces.json`.

The final-source existing IPC/deferred-reply/reply-race and remote SHM regression
matrix at `ipc-regression/native-20261104T114227.122996Z` passes on 2/4/12 CPUs.
The hardened shared-ring/channel/Doorbell matrix is retained at
`shared-ring-regression/native-20261104T114444.823686Z`: 2/4/12 CPUs PASS.
Its final statuses are indexed in `acceptance-summary.json`.

Hardened host checks are at `checks-20261104T113745.870486Z` and
`validation-20261104T113807.412246Z`. Kernel/IPC/ring/Doorbell/UAC tests and
scheduler/parser checks pass. Bare-metal type check, changed-file formatting
and ordinary release build pass. The pre-existing workspace formatting,
Cargo.toml EOF and strict Clippy diagnostics remain; Miri remains NOT_RUN.
The fifteen hardened parser mutations pass against
`native-20261104T113833.518916Z/2cpu.serial.log`; receipts are in
`hardened-parser-controls.json` and `.log` in the resume directory.
The hardened production image at `production-20261104T113841.140154Z` passes
exclusion, as does the ordinary release ELF: see `hardened-production-exclusion.json`.

Relative to the resumed code snapshot, this session changes only
`channel_contract_native.rs` (redacted authorization diagnostic and formatting),
`hybrid_race.rs` (emit final inventories only at zero), `mapping_trace.rs`
(fail-stop tracked duplicate-free and premature-claim observation), the userspace
`operation/lifecycle.rs` fixture (parked child wait, preserve child ownership,
bounded final inventory polling and native foreign probes), the R2 receipt
auditor (stricter rejection/ACK/map-order checks), and the new R2 parser mutation
controls. The already-present atomic channel/reply bridge was preserved; no
second lifecycle lock or production transport behavior was added.

## Remaining executable boundary

R2 is PARTIAL even when the recorded CPU matrices pass: concurrent successful
exec is not established by this fixture. Exact old-incarnation replacement under
numeric PID reuse and isolated live-token foreign adoption remain outside its
native coverage. No numeric PID reuse proof or production service
integration is claimed. Earlier failed combined runs retain their FAIL status;
later passing runs do not explain unrelated historical failures.

The smallest next executable milestone is an explicitly scoped exec/publication
fixture that respects exclusive service ownership, retaining an unreachable
winner order as NOT_RUN. An isolated live-token foreign-owner probe can reuse
the existing forwarded endpoint mechanism. Then connect one bounded experimental Ring 3 service to the
same authenticated single-operation contract with production routing still off.
