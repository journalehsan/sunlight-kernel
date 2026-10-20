# FastLane F1.1P-R3: exec authority and lifecycle closure

Status: **implemented in part; acceptance blocked**. F1.1B-0 remains **blocked**.
This phase preserves the R2 kernel-stack transition and introduces no restricted
SHM, ring, Doorbell or production FastLane integration. Raw evidence and the
machine-readable gate decision are under `target/fastlane-f11pr3-evidence/`.
The final evidence section below distinguishes reached native assertions from
source reasoning and unsupported interleavings.

## Authority ownership and inheritance

Process identity is PID plus nonzero `foreground.incarnation`; VM identity is
`AddressSpaceIdentity { pml4_phys, generation }`. Exec preserves the former and
replaces the latter. Neither physical root nor PID alone proves continuity.
`GrantBinding.process_generation` is misleadingly named: the implementation
fills it from the **VM generation**, not the process incarnation.

The following is the implemented baseline with the narrow image-role correction
in this phase. “Preserve” concerns kernel authority, not restoration of token
bytes from the old userspace image. Failed exec below means failure before the
commit, not an exception after CR3 replacement.

| Capability class | Before exec | Successful exec | Failed exec | Process exit | Evidence |
| --- | --- | --- | --- | --- | --- |
| Ordinary IPC SEND | Bearer, endpoint rights; authenticated kernel sender badge | Preserve broker record; old userspace copies disappear | Preserve | Endpoint owner exit destroys all endpoint tokens; holder exit alone does not revoke peer tokens | `capability/mod.rs`: check, mint, revoke_endpoints_owned_by; `ipc/mod.rs`: enqueue_call |
| Endpoint ownership / RECEIVE / destroy | Owner PID plus suitable token; possession alone cannot receive as another owner | Preserve endpoint and owner PID | Preserve | Owner reaper removes endpoint, queued references and peer calls | `capability/mod.rs`: token_owner, destroy_endpoint; `sched/mod.rs`: reap_process |
| Derived IPC rights | Subset attenuation, same endpoint; no recipient binding or revocation tree | Preserve | Preserve | Endpoint removal invalidates all derivations | `capability/mod.rs`: derive |
| Auth-session grants | Trusted issuer plus matching pending request; recipient PID, expiry, one use | Withdraw, including unconsumed tokens | Preserve | Recipient cleanup withdraws; issuer exit does not withdraw already committed recipient-owned grants | `capability/lifecycle.rs`; `capability/mod.rs`: mint/consume_auth_session_grant, retire_recipient_grants |
| Account grants | Same authenticated issuance; PID + VM generation + account scope/session generation | Withdraw; fresh request needed | Preserve | Recipient cleanup withdraws | `capability/lifecycle.rs`: account_grant; `ipc/src/accounts.rs` |
| Wise Owl proofs / delegated sessions | Live caller/mediator VM generations, UID, purpose, session; pair proof, single use | Retire either participant's relationships | Preserve | Either participant retirement invalidates | `kernel/src/delegated_caller.rs`; retire_recipient_grants |
| VFS bearer capabilities | Prefix/rights checked; full numeric broker PID authenticates mint; transferable | Preserve bounded broker record | Preserve | No general owner-death revocation contract | `capability/mod.rs`: sys_grant_capability, grant_vfs_capability, revoke_endpoints_owned_by |
| Open native/Helios FDs | Current/shared FD table, rights; index is not a secret bearer | Preserve except CLOEXEC; Linux personality rebuilt | Preserve | Descriptor/pipe/handle cleanup through reaper | `process/fd_table.rs`; `syscall.rs`: sys_exec, close_file_handle |
| Legacy SHM token / object | Transferable bearer, whole backing RW/NX; PID owner is not recipient binding | Token may remain while peer views retain backing; no implicit remap | Preserve | Task views released; peers retain valid views; no blanket retirement of all owner objects | `memory/shared.rs`: cleanup_shared_pages; broker shared lifetime methods |
| SHM view / mapping ledger | Exact VM ledger + task view inventory; shared backing references | Remove old views with verified unmap/shootdown before last backing release | Old mappings remain | Same verified release, borrower/owner lifetime constraints | `memory/shared.rs`; `process/address_space.rs`; `memory/tlb.rs` |
| Private mappings / page tables / FS | VM generation; user memory and old TLS | Replace/reclaim; FS reset; no user pointer authority inherited | Candidate rollback; original VM retained | Safe reaper reclaims owner VM only | `process/spawn.rs`: exec_into_process; `sched/mod.rs` |
| Credentials, capability-mode, capability vector, cwd, environment, lookup restrictions | Kernel process state, authenticated current process; some credentials deliberately confer privilege | Preserve; restriction mask does not widen; environment follows existing exec ABI | Preserve | Process state retired; kernel references cleaned | `process/mod.rs`; sys_exec; thread_spawn |
| Service discovery and registration | Owner endpoint + name policy; restricted lookup mask; special display/manager checks | Existing endpoint/name authority persists; role-dependent new operations rechecked | Preserve | Endpoint liveness invalidated; service lifecycle handles stale registration | `ipc/mod.rs`: mediate_nameserver_register, mediate_nameserver_lookup |
| Display, swap/zram, PTY, session, Wise Owl and control-panel image roles | Embedded image provenance, with narrow per-operation checks | Existing reset/reinstall rules retained; swap admin old-generation record revoked | Restore saved roles | Owner cleanup; VM checks where present | `process/spawn.rs`; sys_exec; `memory/security.rs`, `memory/zram.rs` |
| Lock-service image role | Embedded Mezzo provenance | **R3:** clear at commit, reinstall only for matching embedded image | Preserve | Dead task cannot invoke syscall | exec_authority.rs; sys_exec; lock credential syscalls |
| TTY/login, auth-broker and service-manager launch roles | Kernel boot or authenticated parent launch chain; not ordinary transferable credentials | **R3:** withdraw at commit; path alone cannot regain launch-chain roles | Preserve | Dead issuer rejected; supervision also checks VM identity | exec_authority.rs; main.rs TTY creation; syscall.rs spawn trust chain; lifecycle.rs |
| Immediate pending call / reply target | Per-process request generation, kernel target identity | Clear as in R2; old recipient request cannot authorize issuance | Preserve | Cancellation/peer-close and reaper cleanup | exec_into_process; ipc reply matching |
| Deferred reply targets / IPC kernel queue state | Process-owned kernel messages and reply rights; no retained userspace pointer | Preserve existing process-lifetime behavior; counters not reset at exec | Preserve | Clear targets and references to dead PID | process/mod.rs; ipc/mod.rs: defer_current_reply, complete_deferred_reply; reaper |
| Signal/process-control authority | Current caller credentials, trusted roles and foreground policy; optional exact foreground incarnation | Credentials/foreground persist; dispositions/mask remain existing process state, not a VM grant | Preserve | Pending/default fatal signals and forced exit remain terminal; reaper owns state | `syscall.rs`: send_signal_generation, enter_exec_image, deliver_pending_signals; `process/signal.rs` |
| Supervision/reap watcher and foreground relationships | Kernel lifecycle records, often PID + VM generation | Process incarnation preserved, old VM-qualified supervision is not refreshed into new authority | Preserve | Lifecycle receipts/foreground cleanup | sched/mod.rs: supervision_identity, active_manager, reap_process |
| Boot special spawn token / fixed-service ABI | Distinguished public selector with separate caller policy, not random secret | Existing process policy retained | Preserve | No global selector revocation | capability/mod.rs: SPAWN_TOKEN; syscall.rs: spawn handling |
| Device/framebuffer/telemetry mappings and display authority | Kernel-selected frames; some PID/endpoint-scoped privilege | Page-table mappings disappear; existing endpoint authority needs separate policy review | Preserve mappings | Existing device/display cleanup | main.rs; memory/security.rs; process/address_space.rs |

Evidence paths in the table are relative to `kernel/src/` except where stated.
There is no generic restricted recipient memory grant in this tree. No existing
capability class should be mistaken for such a grant. Auth grants do **not**
carry process incarnation or VM identity in their own record: their protection
relies on serialized recipient retirement and authenticated pending calls.
Account grants carry VM generation, but not incarnation or physical root.

The narrow new policy follows the field documentation and actual installers:
TTY is installed for the boot-loaded TTY ELF; auth/service-manager roles require
a trusted parent launch; lock authority is installed for embedded Mezzo. An
arbitrary replacement image has none of those provenance facts. The consumers
are credential-query/session establishment and broker issuance operations.
No consumer requires transferring these provenance markers into an unrelated
image. Normal credentials and transferable open resources intentionally have
a different lifetime. Deferred kernel reply targets remain process-scoped
because endpoint ownership survives exec; this is documented existing behavior,
not a claim that stale request generation wrap/reuse is fully secured.

## Identity, cleanup and compatibility qualifications

| Authority group | Authenticated owner / binding | Incarnation / VM coupling and reuse | User memory / cleanup owner | Compatibility expectation |
| --- | --- | --- | --- | --- |
| IPC owner, SEND, derivations | Endpoint owner PID; SEND bearer; RECEIVE also caller ownership | No per-holder incarnation/VM binding; checked endpoint ID allocation, no derivation tree | Tokens are scalars; broker/reaper removes endpoint inventory, queue cancellation is IPC-owned | Process endpoint continuity on native and Helios exec; copied token must be deliberately supplied to replacement |
| Auth session | Issuer trusted role plus exact pending reply relationship; recipient PID | No stored incarnation or VM generation; retirement under scheduler lock supplies image fencing; PID/call reuse remains acceptance gap | No user pointers; broker consumes/recipient retires, issuer exit does not revoke committed grants | Both personalities must explicitly reauthorize after exec |
| Account | Authenticated issuer/request, recipient PID + VM generation + scope/session | VM replacement invalidates binding and retires record; no stored incarnation; generations never intentionally reused | Scalar token and scope; broker consumes even on mismatch, recipient retires | Same binding rule for native and Helios |
| Wise Owl delegation/proofs | Caller/mediator identity, session/purpose, trusted validator | Both VM generations checked; pair uniqueness among active bounded records, no historical proof registry | Scalar proof; participant retirement and one-use validator own cleanup | Exec withdraws participation regardless of personality; native race matrix incomplete |
| VFS token | Numeric broker PID authorizes mint, prefix/flags constrain bearer use | No recipient incarnation/VM binding or historical nonreuse; fixed 64-record store | No user mapping pointer; broker rollback/revoke owns records; no general issuer-exit revoke | Existing transferable behavior retained for both personalities; numeric issuer check blocks security acceptance |
| FD/open resource | Current or shared process descriptor table | Process state retained; FD slot reuse after close intentional, not cryptographic identity | Kernel resource handle, no old userspace buffer retained by exec; CLOEXEC and reaper own closes | Conventional open-FD exec inheritance; Linux personality replacement does not imply all FDs close |
| SHM backing handle | Legacy bearer; owner PID is metadata, not recipient authorization | No recipient incarnation/VM binding; active-token exclusion only; backing retained by legitimate peer views | Handle scalar; last verified view release owns backing return; retirement fences admission only | Native/Helios retain legacy semantics, no restricted or RO grant implied |
| SHM view, private pages, page tables | Exact VM/ledger and task view inventory | New root/generation on exec; borrowers share root/generation; delayed release must use old object | User mappings themselves; VM/SHM teardown and TLB acknowledgements precede PMM release | Failed preparation preserves image; owner/borrower exec restriction applies to both personalities |
| Credentials, restrictions and environment | Kernel Process object, current task/shared resource selection | Owned by process lifetime; exec preserves incarnation; fresh process construction initializes new state | Kernel copies, no retained old-image token buffer; process cleanup drops state | Native identity/environment and existing Helios process continuity preserved |
| Embedded/launch roles | Trusted ELF resolver or authenticated parent launch; not bearer | Flags are Process fields, not separate generation records; R3 clears four roles only on successful commit | No user pointer; commit/reset/reinstall or process retirement owns withdrawal | Image provenance cannot be transferred by arbitrary exec; embedded lock service may regain only lock role |
| Service registration/discovery | Endpoint owner plus kernel name policy; special role checks for display/manager | General registration is process-name/PID based, not an incarnation-bound image grant; manager adds VM identity | Scalars/kernel service records; endpoint death/lifecycle removes liveness | Existing process service identity persists; broader arbitrary-image registration review remains blocked |
| Immediate/deferred IPC authority | Kernel process queue and reply target + pending-call generation | Immediate relationship cleared on exec; deferred records/counters retained; wrapping counters and PID reuse not fully proven | Kernel message copies, no old userspace buffer; IPC cancellation/reaper owns retirement | Process-owned deferred service replies intentionally retained under current contract |
| Supervision/foreground | Supervision uses PID + VM generation; foreground incarnation allocated independently | Exec preserves foreground incarnation; does not upgrade old VM supervisor records | Kernel records, scheduler/foreground lifecycle cleanup | Existing supervision semantics retained; fresh service image cannot inherit manager launch flag |
| Boot selectors and device/display grants | Separate fixed-service caller policy / kernel-selected mapping authority | Heterogeneous PID, endpoint and mapping lifetimes; no claim all are incarnation-bound | Mappings retire with VM; kernel device/display cleanup owns separate records | Existing ABI retained; complete device authority inheritance remains audited only |

The Process capability vector is retained kernel bookkeeping, not an independent
recipient-binding check that can replace broker authorization.

Signal dispositions can retain a `UserHandler(u64)` address in kernel process
state. Current delivery explicitly leaves custom-handler frame setup unimplemented;
this is an existing compatibility limitation, not an inherited executable VM
mapping. No POSIX-complete disposition-reset or custom-signal handoff claim is
made, and this phase does not broaden the signal subsystem.

Generation behavior is deliberately not inferred from aggregate frame counts.
Userspace pointer arguments are copied before retirement; scalar token copies
are neither a mapping nor an independent broker grant. No new native/Helios
ABI or error number is introduced by the role correction.

## Linearization and failure contract

1. The R2 trampoline copies the syscall input to the kernel stack; sys_exec
   copies bounded path/argv/env input and resolves/validates the ELF.
2. Under SCHEDULER, reject exiting tasks, VM borrowers, and owners with any
   unreaped borrower of the exact VM identity. PMM is acquired next.
3. Construct a fresh VM and image under those locks. Temporarily replacing the
   Process address-space field is not publication: other lifecycle participants
   cannot acquire SCHEDULER. Hardware still uses the old root.
4. Build failure restores the old address space/personality/saved image roles
   and destroys the candidate. The new role-withdrawal helper is not reached.
   Allocation failure at every image-construction stage is not natively proven.
5. Commit is the successful-preparation branch under SCHEDULER: CAP_BROKER
   withdraws recipient/participant grants, the new helper clears launch/lock
   provenance, and pending call/immediate reply target are cleared. New VM
   identity and execution context become the only admitted image state.
6. Activate the new exact VM identity through the unchanged epoch protocol.
   Temporarily selecting the old AddressSpace object for SHM cleanup never
   reloads the retired CR3. Release old views before reclaiming private pages
   and page tables, after exact active-CPU tracking excludes execution there.
7. sys_exec applies CLOEXEC and existing embedded-image roles, plus lock role
   reinstatement. Drop guards and temporaries; the R2 trampoline transfers
   directly to the prepared iret context. It cannot return to old user frames.

Issuance holds SCHEDULER from live issuer/recipient/pending-call validation
through CAP_BROKER insertion. Insertion is its publication point. Expensive
external request preparation carries request data, never a validated recipient
reference. There is no grant reservation registry or lock-drop window here.
If publication wins, exec sees and withdraws that grant. If exec wins, cleared
pending identity rejects the old issuer relationship. A fresh request can
explicitly authorize a grant using the replacement VM generation. Exec of an
issuer now also withdraws its ability to mint, while committed recipient-owned
grants remain valid until recipient cleanup/expiry/consumption.

Exit admission and physical reaping are distinct. `is_exiting` rejects issuance
before deferred reclamation completes. Reaper revocation runs under the same
scheduler serialization. Duplicate retirement/revocation is idempotent. A
post-commit kernel invariant failure follows existing fail-stop behavior; it
must never resurrect an old VM or resume its syscall frame. No new recovery
path, syscall stack mechanism, TLB mechanism or lock order is introduced.

## Shared VM, migration and resource lifetime

Native thread creation and supported Linux clone publish borrowers under
SCHEDULER. Native/compatible tasks sharing an address space are one memory
protection domain. An owner cannot exec until **all** exact-VM borrowers are
Reaped, including Finished borrowers. A borrower cannot exec independently.
Clone first means exec rejection; exec first means a subsequent admitted clone
uses the replacement owner's VM. This is source-grounded serialization; it is
not a newly completed native deterministic clone/exec race test. Native error
is `u64::MAX`; existing Linux-compatible rejection is ENOSYS (38), preserved.

Owner exit terminates borrowers and postpones reclaim until their lifetime and
CPU/stack references permit reaping. Borrowers never reclaim owner page tables.
The active VM root/generation, activation mutation epoch, synchronous remote
TLB acknowledgements and fail-stop timeout remain unchanged. VM-bound mappings
and shared backing are different resources: a peer's retained view legitimately
pins backing after owner exec, whereas an old owner ledger cannot remain as an
unexplained mapping. VM generation allocation is checked/nonzero; root reuse
must get a new generation. Mapping ledgers and SHM cleanup use the old exact
address-space object, not the replacement generation.

## Secure issuance review and exact remaining blockers

The production broker uses conditioned entropy, 62 payload bits, 16 candidate
attempts, 8192 active tokens and 4096 endpoints. Capacity/reservation precede
publication; active collisions across all broker tables, including retired SHM,
are rejected. Failure does not fall back to counters or zero. Boot without all
approved entropy sources is explicitly tested. Conditioner known-answer and
exhaustion tests are independent of lifecycle tests. SHM preparation holds an
exclusive broker borrow through commit and rolls back without publishing a
valid token. VFS batch rollback and single-frame ownership failure tests remain.
These establish useful prerequisites, not complete F1.1B-0 acceptance.

* Ordinary bearer secrecy is probabilistic in a 62-bit namespace. The broker
  excludes active equality, not historical recurrence after revocation. A
  future generation-bound memory grant cannot treat bearer secrecy as identity.
* VFS mint still authorizes numeric `CAPABILITY_BROKER_PID=6`, not a launch-chain
  incarnation/VM-qualified role. The full-width comparison fixes truncation,
  but cannot establish authority after arbitrary image/PID replacement.
* Pending IPC identities and deferred reply counters use wrapping generations;
  PID-reuse, equal pending-generation and counter-exhaustion authorization need
  a complete authority-boundary review. Existing synthetic reuse tests with a
  deliberately distinct call generation do not prove rejection of equal IDs.
* Native deterministic remote publication immediately on both sides of exec,
  issuer/recipient exit during preparation, PID reuse, forced migration and
  delayed TLB acknowledgement during this exec path remain unproven. No new
  unsafe lock-held remote barrier was inserted to manufacture such evidence.
* Native allocation failure at every preparation stage, concurrent account/
  session mutations, Wise Owl races, Linux TLS handoff and stale-generation
  metadata release are incompletely covered.
* PID-scoped display/device and service-registration authority survives through
  different contracts; broad revocation would change compatibility. Their full
  cross-exec security review is audited only here.

These are the explicit security boundary. Further production redesign is
stopped here; passing bounded probes cannot unblock the restricted-grant phase.

## Deterministic and native coverage interpretation

The host suite compiles the production broker, issuance methods, conditioner
and the new image-role helper. Scheduler/Process are explicitly a host model.
Existing commit hooks park after recipient validation while holding lifecycle
serialization; `try_lock` proves exec/cleanup cannot enter before publication.
The new preparation test parks **outside** lifecycle serialization, completes
exec invalidation, then resumes and asserts no broker record. Both grant types
are checked. Existing tests cover cleanup-first, commit-first, issuer exit,
multiple grants/recipients, duplicate cleanup, bounded collision/capacity and
rollback. The role test asserts that replacement of the issuer withdraws mint
permission while an already committed recipient-owned grant remains present.
None of these host tests is represented as a native scheduler/CR3 test.

The opt-in native fixture now retains a live delegated SHM view across its
first exec. It creates an actual native borrower, attempts valid-image exec
while the borrower waits on an explicit shared release word, requires rejection,
releases/joins that borrower, then executes the original eight-image R2 chain
and final /bin/true. This ordering requires no arbitrary sleep. Synthetic auth-broker, TTY and lock image
provenance is installed only for that isolated reader; manager-role withdrawal
is host-tested only (a manager cannot use the native thread syscall). At all nine real exec
commits the kernel asserts withdrawn roles, zero recipient grants, unchanged
UID and valid preserved RECEIVE/owner endpoint authority, and distinct VM
generations. The first commit requires one live old SHM view. R2 entry markers
require actual replacement execution; final reaping checks empty tokens/grants/
SHM/view inventory and exact physical, shared-data and page-table baselines.
The existing issuer then attempts issuance after completed recipient reaping
and must be denied. These assertions do not prove user-level inherited token
recovery, remote grant/exec boundary injection or repeated fresh process creation.

Final command outcomes, CPU rows, resource measurements and limitations are
recorded below and in validation-summary.json.

## Code changes and targeted results

Source revision: `a61f5474466cf3175a5d60ac086dd1dd12209643` plus the retained
working-tree patch and copied new sources. No commit or destructive reset was
performed. `exec_authority.rs` is shared with the host harness so its test does
not duplicate the production role-clear implementation. `spawn.rs` invokes it
only in the live successful commit branch. `syscall.rs` reinstalls the embedded
lock role consistently with the existing image-path roles. The capability gate
and assembly fixture add assertions and the borrower/live-view scenarios only
under the existing opt-in test configuration. The R2 trampoline, frame copying,
iret handoff, VM allocator, TLB implementation and scheduler policy are unchanged.

Final native run: `target/fastlane-f11pr3-evidence/20261020T182034.705392Z/`.

| CPUs | Authority commits / user entries / zero-active-VM reclaims | Free frames before → after | Shared backing / page-table drift | Stress workers / issuance cycles |
| --- | --- | --- | --- | --- |
| 1 | 9 / 8 / 9 | 68306 → 68306 | 0 / 0 | 2 / 256 |
| 2 | 9 / 8 / 9 | 68048 → 68048 | 0 / 0 | 4 / 512 |
| 4 | 9 / 8 / 9 | 67536 → 67536 | 0 / 0 | 8 / 1024 |
| 12 | 9 / 8 / 9 | 65488 → 65488 | 0 / 0 | 24 / 3072 |

Every row reached borrower-rejection/join, old-grant withdrawal, replacement
entry and post-reap late-issuance denial. The initial retired image reclaimed
518 private frames and 11 page-table frames; each of eight subsequent images
reclaimed 515 private and 9 page-table frames: 4638 private frames and 83 table
frames across the nine retirements, separately from shared backing. Native
terminal broker token, auth/account record, SHM object and task-view inventories
were zero. This does not measure retained heap-vector capacity or enumerate all
global VM references. There is no separate retired-grant gauge or measured
in-flight reservation counter.

The host suites passed 79 kernel, 115 IPC/service and 11 ELF tests. Local broker
128-sample endpoint-create/mint guest-TSC measurements remain in each serial
log; they exclude lock acquisition and do not measure auth/account publication
latency. Exec latency, scheduler-lock hold distributions and cleanup completion
latency are unavailable. No FastLane performance improvement is claimed.

The initial development run retained two compile failures (the host helper's
module import and the probe's endpoint-owner Result tuple). The corrected final
native run has verified unchanged source hashes. Its only check failure is the
workspace formatting gate. The missing WAV fixture is a separately retained
36-pass/1-fail media result. New standalone helper and changed lifecycle host
files pass rustfmt checks; unrelated formatting was not rewritten.

The TCP/TLS gate first failed before guest start because sandbox socket creation
was denied. The approved run passed all local TCP/TLS assertions on 12 CPUs.
Both logs are retained; there was no guest timeout retry. Network evidence is in
`network-12cpu.log` and `network-12cpu-approved/` under the R3 evidence directory.

Final regression run: `target/fastlane-f11pr3-evidence/20261020T183020.322039Z/`.
The copied source hashes match the final native run. Each ELF/ISO hash is
rechecked by the consolidation script before the gate decision is written.

| Gate | CPUs | Result |
| --- | --- | --- |
| Exec authority, borrower rejection, live-view cleanup, late-issuance denial | 1 / 2 / 4 / 12 | PASS, explicit assertion counts above |
| No approved entropy source | 1 | PASS, blocked before userspace authority publication |
| Resident / F1 SHM retirement | 1 / 2 / 4 / 12 | PASS, required remote acknowledgements 0 / 1 / 3 / 11 |
| Scheduler lifecycle/timing gate | 1 / 2 / 4 / 12 | PASS |
| Combined D1 / D2 / D3 / RecoveryBlocked | 1 / 2 / 4 | PASS, completion markers required |
| Exact-VM TLB replacement/accounting | 12 | PASS |
| Anonymous unmap/shootdown | 4, supported gate | PASS |
| Permission/mprotect shootdown | 4, supported gate | PASS |
| Local native TCP/TLS | 12 | PASS after sandbox permission correction; original failure retained |
| Target default check and feature builds | n/a | PASS |
| Workspace formatting | n/a | FAIL, pre-existing differences retained separately |
| Media WAV fixture | n/a | 36 passed / 1 failed, missing pre-existing fixture |
| Diff whitespace / changed host-source rustfmt / Python compile | n/a | PASS |

Commands are retained in the JSON and per-command logs:

```sh
python3 tools/fastlane_f11pr3_validate.py --native
python3 tools/fastlane_f11pr3_validate.py --regressions
cargo test --locked --offline --target x86_64-unknown-linux-gnu -p sunlight-elf --lib
python3 tools/network_tls_gate.py --iso target/fastlane-f11pr3-evidence/20261020T182034.705392Z/resident.iso --cpus 12 --evidence target/fastlane-f11pr3-evidence/network-12cpu-approved
python3 target/fastlane-f11pr3-evidence/assess.py
```

The runner exits nonzero for the workspace formatting failure; the consolidation
does not erase it. Its assessment distinguishes passing functional gates from
that pre-existing failure and the missing WAV fixture. Historical development
failures and both socket-harness attempts remain in the evidence directory.

| Work item | Classification |
| --- | --- |
| Source-grounded inheritance/ownership inventory and lifecycle boundaries | Audited only, with explicit unresolved authority boundaries |
| Auth/TTY/lock provenance withdrawal | Implemented and host/native validated on 1/2/4/12 CPUs |
| Manager provenance withdrawal | Implemented and host validated; native withdrawal not directly seeded |
| Embedded lock-role reinstatement | Implemented, partially validated; native lock-image reexec not exercised |
| Deterministic grant/exec/cleanup ordering | Host barriers validated; native boundary-interleaving matrix blocked |
| Existing exclusive-VM/borrower rejection and live-view exec cleanup | Newly validated natively; concurrent clone/exit boundary hooks deferred |
| Complete fresh-process create/grant/exec/access/exit cycle matrix | Blocked, not replaced with repeated exec evidence |
| Secure-token prerequisite | Bounded entropy/collision/capacity/rollback checks passed; complete F1.1B-0 blocked |
| Formatting / missing WAV / unmatched pre-R attribution | Pre-existing issue / unavailable historical comparison |
| Restricted grants / SPSC / Doorbell / production FastLane | Deferred, not implemented |

**Gate decision: full R3 acceptance and F1.1B-0 remain blocked.** The exact
remaining security prerequisites are listed above and in the consolidated
`validation-summary.json`. The phase stops at that boundary. There is no claim
that zero observed drift or passing bounded tests proves absence of all races,
and no automatic progression to restricted SHM.
