# Native runtime R2B.5: security and retirement qualification

Date: 2026-11-14. **Verdict: PARTIAL. R2B.5 is not COMPLETE.**
Revision `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`, branch `main`, pre-existing dirty worktree.
No production corrections, commit or push. No condition variables, autonomous detached reclamation or CPython implementation.

Evidence root (**E**): `docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b5/`.
`validation-summary.json` is the gate assessment; `cpu-matrix.json` records each CPU scope.
All results below are fresh R2B.5 executions except where explicitly described as prior source/report review.

| Gate | Status | Evidence boundary |
|---|---|---|
| G1 baseline/integrity | PASS | Locked runtime rebuild; rebuilt inherited fixture ELFs; lifecycle 17/17 on 1/2/4; full original-file integrity check |
| G2 terminal race | UNQUALIFIED | No deterministic final-exit/forced-termination overlap witness |
| G3 IPC/capability | PARTIAL | Real legacy RPC, task authority and endpoint closure; authenticated channel continuity unexecuted |
| G4 borrower SHM | BLOCKED | All nine native attempts rejected at allocation authority prerequisite |
| G5 integrated retirement faults | UNQUALIFIED | Existing standalone memory controls only; no borrower-created backing/fault integration |
| G6 native PTY/session | NOT_RUN | Host tests pass; retained-owner terminal/session scenario absent |
| G7 identity/churn | PARTIAL | Existing group churn and stale endpoint-token operations; stale group/session/channel generation reuse absent |
| G8 security/progress | PARTIAL | Host and scoped native controls pass; integrated security/fault/race gaps remain |
| G9 native regressions | PASS | Fresh language/lifecycle/heap/TLS/ABI and supported TCP matrix |
| G10 final assessment | PASS | Integrity, limitations, report, machine-readable results and CPython assessment recorded |

## Baseline, preservation and source binding

Inspected the R2B.3/R2B.4 reports, R2B.4 validation and recheck summaries, source integrity maps,
retained-authority design and recorded failures. Reviewed threading T2R, KFP-2A.2 persistent
retirement, KFP-2A.6 and 2A.6D contention/cleanup boundaries, restricted-retirement and authenticated
membership contracts, and the terminal per-tab lifecycle documentation. Older document claims
are historical contracts/limitations, not fresh runtime results.

`source-baseline.json` records all 63,705 existing regular files and 5,638 symlink targets exposed
by Git's tracked/nonignored inventory. It includes the original uncommitted implementation,
lockfiles, vendored sources and earlier evidence. `baseline-worktree.log` records incoming status.
`source-integrity.json` compares the complete original inventory at final assessment. Generated,
ignored build products are outside that inventory. Git HEAD alone does not identify this dirty source.

`run-validation.py` records exact commands, return codes, source hashes and durations. It rebuilds
the native archive with `--locked`, compiles R2B.4 `lifecycle.c` and R2B.3 `exit-matrix.c` into fresh
R2B.5 target paths, then runs the matrix. The first attempt incorrectly looked for the exit fixture
under R2B.4; compiler failure and the successful preceding build are retained in `attempt-01/`.
The corrected attempt does not overwrite it. This is installed-toolchain rebuild reproducibility,
not a clean-room or bit-identical build claim.

Per-guest folders retain invocation, kernel build/hash, ELF hashes, ISO build, serial, stdout/stderr,
resource receipts and assessor results. `validated-source.json`, the full baseline, and final unchanged
source bind these to the same production source. New fixture revisions have distinct ELF paths and
attempt sources; an earlier fixture's PASS is never substituted for a later fixture.

## Identities and authority scopes

| Identity | Authority and lifetime | Evidence limit |
|---|---|---|
| Process | PID plus `foreground.incarnation`; grant recipients additionally bind VM generation | Numeric PID alone is not authentication; `RecipientIdentity::of` is an exact tuple |
| Thread | Independent Process PID/incarnation; runtime pthread token is a separate nonwrapping public handle | Native guest logs print TIDs and kernel handoff observations print incarnations |
| Native group | Retained original Process, exact `AddressSpaceIdentity` root/generation; terminal receipt under SCHEDULER | No separately allocated group ID; same root without generation is insufficient |
| VM | Physical root plus generation and generation-qualified ledger handle | Ordinary handoff logs expose generation, not the physical root; no root value is invented |
| Session | PTY broker publication/session/generation; Account `SessionRef(id,epoch)` has separate owner/issuer authorization | Neither a PTY number nor Account SessionRef alone authorizes an operation |
| IPC endpoint | Opaque token, broker rights, endpoint ID and owner PID; receive/destroy additionally require current owner | Legacy `Endpoint` has no standalone generation field; token tests do not prove numeric endpoint-generation reuse |
| Restricted channel | Handle token/generation, exact recipient/issuer tuples, Account session and role checks | Existing VM-exclusivity gate excludes unreaped shared-VM peers; it is preserved |
| SHM backing | Broker token/object and retained physical frame list; immutable process/session quota receipt | Creator task, quota payer and final reclaimer are distinct; no backing was allocated by new attempts |
| Mapping/view | VM identity plus VA/extent, ledger kind/backing; task-local `(token,base,size)` tracking | Shared VM does not imply two thread-local copies of the page table or independent mappings |
| Cleanup owner | Group terminal receipt; each task's pending cleanup and pin/reap state; broker backing/view lifetime; retirement slot/generation | Group terminal publication does not imply completed physical reclamation |

Source anchors: `kernel/src/process/mod.rs:434`, `kernel/src/sched/native_group.rs:43`,
`kernel/src/sched/mod.rs:2084`, `kernel/src/capability/delivery.rs:3`,
`kernel/src/capability/mod.rs:1153`, `kernel/src/capability/mod.rs:1606`,
`kernel/src/capability/channel_native.rs:28`, `kernel/src/memory/shared.rs:54`,
`kernel/src/memory/retirement.rs:37`, and `kernel/src/process/foreground.rs:142`.

Affected production paths inspected include thread admission/publication, both native spawn ABIs,
task-only and group exit, external/pending fatal termination, parent wait/liveness, owner resolution,
endpoint broker/queue closure, SHM create/map/free/exit cleanup, shared quota, shared ledger, page
unmap/root reclaim, retirement/TLB mailbox/ACK handling, session scanning and physical reaping.
No existing task authority guard was generalized to group liveness.

## W6: final exit versus forced termination

**UNQUALIFIED.** W6-A/B/C have no controlled native race execution. W6-D repeated terminal
requests and W6-E stale-generation requests after reuse are NOT_RUN. Fresh normal final-borrower
exit and forced retained-owner termination pass on 1/2/4, but use independent groups without the
required competing invocation intervals. They are controls, not the requested W6-A/B witnesses.

`begin_native_group_exit` stores its first receipt before nested sibling termination and endpoint
callbacks. Both final-thread paths and forced termination reach it under SCHEDULER. The inherited
syscall156 observer reports handoff quiescence; it provides no independent external contender or
precommit suspension. A scheduler-held pause awaiting another lifecycle operation would create a
lock dependency cycle. No such pause, IF-disabled rendezvous, production split or new wait was added.

A safe future coordinator needs separately authenticated actor/subject identities, independently
observable invocation intervals, owned resumable continuations outside lifecycle guards, identity
revalidation on resume, actual terminal/cleanup decisions and independent resource receipts.
This run did not construct that mechanism. The source serialization argument is not a proof that
all future coordinators are impossible and is not accepted as race qualification. Cleanup-winner,
refund and quarantine fields remain absent, rather than inferred from one terminal log line.

## W8: native IPC continuity and task authority

**PARTIAL.** `ipc-v2-cpu-{1,2,4}/` passes actual guest assertions:

1. Borrower A creates its own endpoint and SEND_ONLY derivation before initial-owner exit.
2. Borrower B completes a request/reply exchange with A before the owner exits.
3. The owner exits via pthread_exit. Each borrower performs syscall156; the kernel records the
   original task Finished and physically quiescent before that borrower's subsequent progress.
4. A second real request/reply exchange succeeds with the existing tokens after owner exit.
5. B's attempts to destroy A's owner endpoint, destroy via SEND_ONLY, and receive on A's endpoint
   fail. Same-rights SEND_ONLY derivation returns the existing token, as the broker specifies.
6. A exits. B waits for syscall156 to acknowledge A physically Reaped, then checks old endpoint
   derivation rejection. B creates a fresh endpoint, rejects old-token destruction against it,
   verifies fresh-token derivation and performs authorized destruction.
7. Final group retirement reports owner frames=0, descriptors=0, stack=0 and balanced runner
   resource classes/handles/names. One group terminal trace is recorded in each guest.

These are legacy task-owned capabilities in an ordinary unprivileged native group, not authenticated
Account/restricted-channel qualification. No original-owner endpoint is reclassified as group-owned.
A and B share memory containing the bearer token; they do not gain each other's receive/destroy
ownership. No separate channel generation or numeric endpoint-ID reuse was exercised. Endpoint and
capability global census/refund counts and controlled cleanup contention remain unmeasured.

The initial IPC fixture incorrectly expected rederiving SEND_ONLY to fail. All 1/2/4 failures remain
in `ipc-cpu-*/`, with `ipc-continuity-attempt-01.c`. Actual `CapabilityBroker::derive` permits equal
rights and reuses an existing matching token (`capability/mod.rs:1038`); the endpoint_bind comment is
stricter than that implementation. The corrected fixture preserves all receive/destroy denials and
runs separately in `ipc-v2-*`. No privilege escalation or production defect was demonstrated.

Authenticated channels require exact participant identities and a live authorized Account session.
`channel_vm_exclusive` rejects an owner with an unreaped same-VM peer. Survival does not justify
bypassing that gate or extending a dead task's `is_exiting` authority. This milestone does not change
that policy to make a thread-group continuity scenario pass.

## W9: borrower-created SHM and integrated faults

**BLOCKED at allocation authorization.** `resources-cpu-{1,2,4}/` preserves nine failed executions:
explicit view cleanup, creator exit retaining a peer view, and forced termination with views,
on every supported CPU count. All establish owner exit and successful post-owner IPC, then exit91
at the borrower-allocation assertion. None reaches map, read/write, revocation or backing retirement.

The root cause is the runner's authority context: `native_c_guest.rs:152` explicitly leaves ordinary
headless cases at uid1000 without Account membership or system allocation. Only network fixtures
opt into system allocation for their transport. Native thread creation preserves the parent's fields
(`syscall.rs:2910`). `alloc_shared_region` rejects absent Account membership when `system_allocation`
is false (`shared.rs:89`). This rejection is the existing security policy, not a lifetime defect.
The network flag was not repurposed to grant this resource fixture authority. No trust/session flags
or synthetic authenticated memberships were installed.

Actual VM model: group members have one root/generation and shared region ledger/bump. SHM syscalls
operate on `current_process_mut`; their `mapped_shared` and `owned_shared` lists remain task-local.
An explicit map would create another view/VA within that same VM, not another address space merely
because another thread called it. Creator identity, broker backing owner and original quota payer
remain distinct. These are inspected source contracts; new successful mappings are NOT_RUN.

Group-integrated delivery uncertainty, partial unmap, interrupted retirement, concurrent revocation,
cleanup/final-exit contention and last-view release/refund are **NOT_RUN / UNQUALIFIED**. Standalone
retirement controls in `retirement-controls/` retain their own exact PTE, owner, sequence, target/ACK,
PMM retention/release/reuse and expected-stop assessments. They do not qualify this group workload.
The current per-control results are in `retirement-faults.json`; no timeout, disappearance, host test
or source audit is promoted to safe integrated reclamation.

## W7: session and PTY

**Native retained-owner scenario NOT_RUN.** Existing PTY host tests pass 11/11 and actual foreground
scan host adapter passes 8/8. The first host command used a missing top-level manifest path; the
corrected `services/pty_server/Cargo.toml` execution has a separate log. Those host models do not
execute real terminal tabs or the production scheduler reaper.

The headless C runner captures VFS streams and does not start pty_server, bind an authenticated
foreground controller, or create terminal tabs. The existing PTY liveness runner executes shell
stream scenarios and has no retained-owner borrower payload. No integrated runner was supplied.
All requested owner-exit I/O, foreground membership, final borrower, forced group, tab teardown and
stale-session-generation guest scenarios remain NOT_RUN. Source inspection shows exact-session
matching and retained-owner handling, and borrower publication clears session_owner/owned scope;
it does not establish live PTY closure or absence of lingering session records.

## W10, resource accounting and forward progress

**PARTIAL.** The 17-case lifecycle suite includes ten sequential two-borrower groups on each of
1/2/4 CPUs. Serial logs show Finished/Reaped transitions, borrower slot reuse and new VM generations.
Post-owner progress and one terminal trace per group are independently parsed in `lifecycle-witnesses.json`.
New IPC guests reject stale endpoint tokens while a fresh endpoint remains usable.

This does not test a generation-qualified operation against a reused process/group/session/channel
identity. Ordinary kill(pid,sig) and thread-reaped(TID) are numeric APIs. The generation-qualified kill
extension requires an active manager and checks target VM generation (`syscall.rs:10895`, `:392`).
No manager actor or stale-generation successor fixture was created. Matching numbers are never
accepted as sufficient identity evidence.

The runner's balanced resources are RamFsFileData, UserPrivate and PageTable frame classes, VFS
handles and temporary names. Owner reap independently checks PMM-owned frames=0, FD table empty,
cleanup pending cleared and kernel stack absent. SharedMemory frames, all capability/endpoint rows,
quota refund multiplicity, sessions, total heap and group/VM census are not part of that complete
measurement. No claim of system-wide zero growth follows from the existing bounded churn.

Reviewed order: SCHEDULER precedes PMM/broker and serialized shootdown; retirement metadata sections
are bounded and do not acquire outer locks or wait for peers. Group receipt publication precedes
callbacks. Reaping checks pins, current/queued/core ownership, borrower Reaped and VM active mask
before root reclamation. Shared free verifies the actual backing/PTE and waits for synchronous
invalidation before reference removal; uncertain cleanup retains backing/fails stop. Persistent
retirement obligations block root reclaim. Real cross-CPU activation, NMI/IPI uncertainty and exact
callback/cleanup overlap remain broader proof obligations. No new lock, spin, wait, capacity increase,
security relaxation or production FastLane enablement was introduced.

## Fresh native regression matrix and CPython

| CPUs | Lifecycle | C | C++ | Heap/ABI | TLS/once | Legacy IPC | TCP loopback |
|---|---:|---:|---:|---|---|---|---|
| 1 | 17/17 | 73/73 | 21/21 | PASS | 4/4 | PASS | NOT_RUN |
| 2 | 17/17 | 73/73 | 21/21 | PASS | 4/4 | PASS | PASS |
| 4 | 17/17 | 73/73 | 21/21 | PASS | 4/4 | PASS | PASS |
| 12 | NOT_RUN | NOT_RUN | NOT_RUN | NOT_RUN | NOT_RUN | NOT_RUN | NOT_RUN |

The native guest runner supports only 1/2/4; TCP supports 2/4. No unsupported cell was relabeled.
C++ includes expected fatal cases, unwinding, RTTI and static initialization. Retained-owner admission,
pthread ABI and C++ ABI/vendor checks pass. Kernel host tests pass 212/212, IPC host tests 97/97,
scheduler host controls and production kernel check pass. Security host controls are supplemental,
not native terminal-race/PTY/fault execution. Raw warnings remain in build logs.

CPython remains **BLOCKED**. Fresh strict compile probes fail for pthread condition variables,
strncpy and _Exit (`cpython-blocker-*.log`). No full interpreter build was attempted. Existing
platform/header gaps, detached autonomous reclamation and unqualified TLS/futex schedules remain
explicit blockers. Functional native lifecycle/security progress is not interpreter readiness.

## Changes and next milestone

Only this report and fresh R2B.5 evidence/fixture/runner/assessment files were added. All incoming
source/evidence files and symlink targets remain unchanged. No reproducible production correctness
defect was established, so no production repair was made. Failed attempts remain available.

Recommended next milestone: **R2B.5 continuation: authenticated fixture and safe contender coordination**.
First provide the normal authenticated launch/session chain for the borrower workload without
policy bypasses, and a validated resumable terminal-contender observer outside scheduler guards.
Then execute W6-A–E and real PTY/session teardown, authorized borrower SHM/view/revocation with
independent backing/quota receipts, integrated uncertainty and bounded full resource/identity reuse.
Keep restricted-channel VM exclusivity and task authority explicit; any changed contract needs its
own demonstrated defect and review. Retain the current PARTIAL verdict until every acceptance gate
has the required native evidence. Condvar, detached reclamation and CPython remain separate work.
