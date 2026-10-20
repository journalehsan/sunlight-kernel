# FastLane F1.1P-R4: VFS issuance authority

Status: **partial implementation; R4 acceptance blocked; F1.1B-0 blocked**.
No restricted SHM mapping, active per-grant revocation, ring or Doorbell is
implemented. Evidence is in `target/fastlane-f11pr4-evidence/`. The working tree
already contained the uncommitted R3 implementation; those changes are retained.
Fresh compatibility results do not prove the missing R4 native race assertions.

## Finding and narrow repair

The numeric argument in the old `capability::sys_grant_capability(caller_pid,
cap)` was the **issuer**, not a recipient. Syscall 100 accepts a path pointer
and access flags; there is no recipient argument. Its only authorization was
full-width equality with PID 6. `sys_grant_capability_syscall` obtained that PID
from the scheduler before copying the path, then invoked the broker without
holding lifecycle serialization. It neither authenticated image provenance nor
rejected an exiting caller. A full-width PID check prevented truncation but did
not establish a trusted launch identity. This is a demonstrated source defect,
not evidence that a forged userspace sender badge can select the issuer.

The replacement `Scheduler::issue_vfs_capability` holds the existing scheduler
borrow through broker insertion. It requires the legacy slot **and** the current
process's kernel-installed `trusted_auth_broker`, nonzero process incarnation,
nonzero VM generation and non-exiting lifecycle state. Path copying stays before
lock acquisition. The public numeric-only helper is removed. No new registry,
reservation, lock, syscall number, recipient argument or userspace layout exists.
The raw broker mint remains a trusted internal primitive, as for other classes.

The slot restriction is deliberately retained to avoid widening mint privilege
to other UAC instances. This is a restrictive repair, **not completed VFS/UAC
service wiring**. The trusted chain in `syscall.rs::handle_spawn_call` is kernel
bootstrap identity → supervised sunlightd → embedded `/sbin/uac_service`.
`process/spawn.rs::embedded_bytes_for_path` supplies that embedded image. The scheduler contains a historical PID-6 sunlightd observation marker, but
the fresh resident 1-CPU serial trace actually assigns PID 6 to sunlight-kbd
and PID 11 to sunlightd. PID 6 is not evidence that its occupant is UAC. UAC's `OP_BASE_CAPS` and `OP_DELEGATE` handlers explicitly
leave kernel mint forwarding TODO. No repository userspace caller of syscall
100 was found. `vfs_allows` also has no production consumer found in the
repository: its existing raw `starts_with` prefix/flag helper must not be
reported as enforced end-to-end filesystem authorization. Native FD operations
continue to use their existing credentials/mode/handle policy. A production trusted-role-and-slot success is consequently not
established. An uncredentialed historical PID-6 caller is now denied. Enabling
mint for UAC at arbitrary slots would be a separate policy adjustment, requiring
an explicit decision and a tested delivery path; this patch does not do that.

## Authority-flow audit

All source paths below are relative to the repository. “Owner” metadata,
possession of a bearer, identity and authorization are separate facts.

| Path / source functions | Initiator and issuer authority | PID origin and recipient identity | Ownership, possession and delivery |
| --- | --- | --- | --- |
| `kernel/src/arch/x86_64/syscall.rs::sys_grant_capability_syscall`; `capability/lifecycle.rs::issue_vfs_capability` | Actual current syscall process; legacy slot plus trusted launch role, live incarnation/VM at commit | Scheduler caller PID; **no recipient PID**. Current Process borrowed through insertion; numeric equality is only an additional restriction | Broker records issuer PID as metadata, not recipient restriction. Token returned in register; later holders are authorized by bearer/prefix/flags |
| `kernel/src/capability/mod.rs::grant_vfs_capability`, `grant_elevated_vfs` | Trusted internal caller; batch helper has no production call site found | Supplied owner PID is metadata, not authenticated recipient proof | Single mint commits on `vfs_caps.push`; elevated batch truncates appended records on failure. No delivery in these helpers |
| `services/sunlight-uac/src/bin/uac_service.rs::handle`, OP_BASE_CAPS / OP_DELEGATE | Claimed administrative policy; handlers test a non-invalid token, not a complete new kernel mint authorization chain | target UID from message; no recipient-incarnation resolution for new VFS issuance | Prefix count response / echo of existing token; forwarding real newly minted capabilities is TODO. This is not a proven secure delegation API |
| `kernel/src/ipc/mod.rs::mediate_nameserver_register`, `registration_authorized`, `mediate_nameserver_lookup` | Caller must own SEND_RECV endpoint and match allowed service name; special manager/display policy also applies | Caller is kernel syscall identity; VFS name maps to `vfs_server`; name/PID is not UAC mint authority | Derive SEND_ONLY for discovery; endpoint ownership remains separate from public token possession. Exec does not universally revoke service endpoints |
| `kernel/src/ipc/mod.rs::handle_ipc_call`, `IpcBus::enqueue_call`, reply delivery | SEND rights checked; kernel overwrites badge using actual calling PID; reply uses kernel target/generation | Payload PID/badge cannot select kernel caller. Pending call has endpoint and call generation, not an explicitly stored process incarnation | IPC copies token words, without creating recipient-bound VFS authority. An undelivered copied bearer has no automatic per-copy broker rollback |
| `kernel/src/capability/lifecycle.rs::grant_requester`, `issue_auth_session_grant`, `account_grant` | Current trusted auth broker plus exact pending request/reply relationship | Requested PID is candidate lookup only; live authoritative Process checked under scheduler borrow; account captures VM generation, auth record only PID | Recipient-bound records inserted under broker lock before lifecycle unlock. Token registers returned to issuer; recipient owns grant record, issuer possesses token for delivery |
| `kernel/src/arch/x86_64/syscall.rs::sys_wiseowl_delegate_caller`, `sys_wiseowl_validate_delegated_caller`; `kernel/src/delegated_caller.rs` | Existing trusted mediator/session/purpose policy; both participating VM identities | Scheduler resolves participants under lifecycle serialization; PID alone insufficient | Delegated registry owns relationship, either participant's retirement removes it. No new VFS recipient path |
| `services/vfs_server/src/main.rs::read_handle` | VFS serves an existing backend handle; not syscall-100 mint | No recipient PID argument or incarnation binding in large-read allocation | DATA_SHARED carries legacy whole-page transferable SHM bearer. Failed delivery has no new R4 retirement protocol. Future restricted grants cannot assume this is recipient binding |
| `kernel/src/capability/mod.rs::mint`, `derive`; IPC endpoint bind | Existing endpoint/source capability rights, subset attenuation | Endpoint owner PID is kernel metadata; no per-holder recipient binding | Transferable token. Broker serializes derive, revoke and endpoint destruction; no derivation revocation tree |
| `compat-linux/src/lib.rs::MockCapabilityBroker` | Mock compatibility policy, not kernel broker authority | Local mock table, no native scheduler identity | Separate deterministic mock token allocator; not a production native mint path |

## Identity and linearization contract

The authoritative identity is a live Process with nonzero
`foreground.incarnation`. VM identity is root plus generation; a generation-only
account binding must not be described as a process incarnation. Successful exec
preserves process incarnation but replaces VM identity and withdraws image roles.
A boolean role belongs to that Process object; it is not a PID-indexed second
registry. Fresh process constructors default it to false. `add_process` allocates
a fresh checked incarnation; exec only reinstalls roles according to existing
image/launch policy. R3 withdraws `trusted_auth_broker` on successful exec.

| Operation pair | Commit first | Lifecycle/revocation first | Limits |
| --- | --- | --- | --- |
| VFS mint / issuer exec | Broker push under SCHEDULER → CAP_BROKER completes before exec can clear role; existing bearer intentionally survives | Role cleared before admission; InvalidCaller, no record | Host barrier validates lock model; no native remote boundary hook |
| VFS mint / issuer exit | Commit precedes exit; existing transferable record survives by policy | `is_exiting` denies insertion, including before reaping | Does not establish recipient cleanup for a transferable bearer |
| Auth/account mint / recipient exit | Same scheduler borrow excludes cleanup; cleanup retires committed records | Exiting state rejects subsequent admission | Native R3 lifecycle fixture validates final cleanup; both remote race orderings remain unverified |
| Auth/account mint / recipient exec | Existing exec commit retires records before VM activation | Cleared pending request rejects old immediate reply relationship; new image must make fresh request | Equal generation/reuse and all deferred IPC identity combinations remain unproven |
| Auth/account mint / issuer exec or exit | Committed recipient-owned records remain until recipient retirement, consume or expiry | Lost launch role/exiting issuer rejects new commit | No issuer-lifetime pins or independent reservations |
| Mint / broker revoke | Broker lock totally orders insertion and removal for supported classes | Existing revoked source fails derive | VFS `revoke` is not an implemented generic bearer-revocation contract |

No validated recipient reference escapes the scheduler borrow in recipient-bound
issuance. There is no recipient-targeted VFS publication operation to which a new
recipient-incarnation field can simply be attached. Inventing one would change
the ABI and bearer lifetime policy. In particular, revoking all VFS tokens at
exec or issuer death would break explicitly retained transfer semantics.

The syscall publishes the scalar in a return register, with no fallible user
copy after commit. A later IPC failure is distinct: it does not undo the earlier
successful mint. A future combined mint-and-deliver transaction needs explicit
rollback/ownership policy. VFS failed reservation, unavailable entropy, collision
limit or capacity limit inserts no partial record. Bounded Vec capacity retained
for reuse is allocation capacity, not an issuance reservation. Existing two-prefix
batch rollback remains unchanged. Scheduler → broker → conditioned entropy is
the added path's lock order. It performs bounded existing store reservation and
at most 16 token candidates, with no source collection, IPC, user fault, VM work,
TLB wait or retry loop. Lock duration distributions were not measured.

## Regression coverage and acceptance boundary

R4 adds production-method host tests for uncredentialed numeric slot, full-width
PID mismatch, absent incarnation/VM generation, exiting issuer, role withdrawal
between preparation and commit, replacement-slot role reset, commit-before-role
withdrawal with barriers, and reservation/entropy failure without partial state.
The barrier confirms the scheduler-model mutex is unavailable at the exact
validated-before-insert point and the broker has no record yet. After resume it
checks owner metadata, incarnation and role, then denies further issuance after
withdrawal. Existing transferable authority survives duplicate issuer cleanup,
as required by its policy. This is not a claim that bearer owner metadata stores
an incarnation. Existing auth/account post-cleanup regressions are preserved.

The native compatibility fixture is unchanged. It can validate real exec role
withdrawal, grant cleanup, repeated image replacements, borrower exclusion and
resource accounting. It cannot deterministically pause remote VFS issuance on
both sides of recipient exit/exec, cannot demonstrate recipient-bound VFS
publication (no such API exists), and does not establish fresh PID reuse or a
failed combined token delivery transaction. Those are exact R4 blockers.
No boot is counted as a new R4 native race test.

Required interleaving disposition:

| R4-E case | Evidence disposition |
| --- | --- |
| 1 commit before recipient cleanup; 2 cleanup before commit; 3 exit during preparation | Existing auth/account production-method host tests retained; new VFS bearer is not recipient-bound; native deterministic remote orderings unverified |
| 4 recipient exec before publish; 5 grant before exec | Existing host request/exec barriers and retirement tests retained; process-scoped VFS bearer intentionally inherited; native remote race unverified |
| 6 issuer exec before publish; 7 authority valid through publish | New VFS host preparation and commit barrier tests; production native role withdrawal compatibility only |
| 8 issuer exit during preparation; 9 PID slot reuse | New VFS host tests reject exiting/roleless replacement issuer; recipient request reuse remains unverified natively |
| 10 multiple grants; 11 duplicate cleanup | Existing bounded auth/account exhaustion and idempotent cleanup; VFS duplicate cleanup preserves transferable token |
| 12 reservation failure | New VFS production-method host test plus existing broker fault tests |
| 13 publication/delivery failure | Entropy/reservation precommit failure covered; register return has no fallible copy; combined IPC delivery rollback unsupported/unverified |
| 14 concurrent issuance/revocation | Existing broker serialization and auth/account cleanup tests; no new deterministic native VFS per-token revocation fixture |

## Independent F1.1B-0 decision

Already implemented prerequisites: conditioned entropy with explicit readiness,
bounded collisions/exhaustion, fallible ordinary mint, transactional endpoint and
SHM preparation, scheduler-serialized auth/account commit/retirement, checked VM
and process identity allocators, R3 image-role withdrawal. Their fresh test scope
is recorded separately; none implies restricted mapping isolation.

Repaired here: numeric-only VFS issuer authorization and the syscall's lifecycle
check-to-insertion gap. Implemented but insufficiently validated: native VFS mint
success through the actual trusted service launch/delivery chain and remote
issuer lifecycle orderings. Audited only: transferable VFS lifetime, daemon
backend handles, IPC discovery/badges, Wise Owl and mock compatibility policy.

Still blocked: intended production VFS/UAC mint wiring; native deterministic
remote grant/exec/exit and PID-reuse assertions; complete pending/deferred request
reuse and generation exhaustion proof; rollback for a future combined targeted
mint/delivery operation; fresh create/authorize/exec/exit cycle accounting; legacy
SHM recipient/rights restrictions needed by future restricted backing. Active
collision exclusion does not prove historical token nonrecurrence. UAC's nonzero
admin-token checks do not establish a secure delegated mint chain.

Deferred to F1.1B: restricted backing, exact recipient-bound memory grants,
rights attenuation and active mapping revocation. F1.1B-0 remains **blocked**;
R4 is **not complete**. Stop at these authority/native-proof boundaries without
automatically implementing restricted SHM or widening UAC's mint authority.

## Fresh validation and measurements

Compatibility run: `compatibility/20261020T185320.901739Z` beneath the R4
evidence directory. This invokes the existing R3 runner with its evidence base
overridden and `--native --regressions`; its internal R3 acceptance fields are
not R4 decisions. Its captured source hashes were verified unchanged throughout
native validation. Afterwards only host-test formatting/EOF whitespace changed;
all captured production Rust source hashes still match. Host tests and the
kernel target check were rerun after that cleanup. Raw first-attempt failures
remain intact.

| Check | Fresh outcome |
| --- | --- |
| Kernel host, including broker, authorization and lifecycle | 82 passed, zero failed; final rerun also 82 passed |
| IPC / audiod / sunlightd libraries | 61 / 25 / 29 passed (115 total) |
| ELF library | 11 passed |
| Scheduler host | Passed |
| Kernel target check | Passed, including final rerun |
| No-entropy native boot | Reached explicit blocked-authority marker; passed |
| Authority/exec/cleanup fixture | Passed on 1/2/4/12 CPUs |
| Resident boot, VFS registration, SHM retirement | Passed on 1/2/4/12 CPUs |
| Scheduler native | Passed on 1/2/4/12 CPUs |
| SunlightD / RecoveryBlocked gate | Passed on 1/2/4 CPUs |
| VM/TLB SMP | Passed on 12 CPUs |
| VM unmap / protection | Passed on 4 CPUs |
| Workspace formatting | Failed; pre-existing workspace differences retained. New lifecycle tests formatted |
| Media library | Known missing repository WAV fixture: 36 passed, 1 failed |
| `git diff --check` | First run caught newly introduced blank EOF line; corrected; final check passed |

The native authority fixture reached nine role-withdrawal/VM generation records
per CPU configuration, eight replacement-image entry assertions, live-borrower
exec rejection, and the existing grant cleanup marker:
`auth=16 account=16 post_reap_records=0 late_auth=denied late_account=denied`.
Each serial record includes process incarnation and old/new VM generation.
This covers repeated images in one process, not repeated fresh recipient/PID
incarnations. Its local fault injection covers existing entropy/reservation/SHM
rollback; it is not new VFS combined delivery rollback evidence.

| CPUs | Workers | Existing issuance cycles | Free frames before / after | Shared data delta | Page table delta | Final mappings / tokens |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 2 | 256 | 68318 / 68318 | 0 | 0 | 0 / 0 |
| 2 | 4 | 512 | 68060 / 68060 | 0 | 0 | 0 / 0 |
| 4 | 8 | 1024 | 67528 / 67528 | 0 | 0 | 0 / 0 |
| 12 | 24 | 3072 | 65500 / 65500 | 0 | 0 | 0 / 0 |

These are measured fixture counters, not whole-kernel leak freedom. VFS issuance
success/rejection rates, stale-recipient/issuer-role native rejection counters,
reservation gauges, exact cleanup latency, issuance latency and scheduler/broker
lock-hold distributions are **unavailable**, not zero. Existing local endpoint
TSC samples exclude lifecycle lock acquisition and are not VFS grant latency.
No FastLane throughput or general CPU improvement is claimed.

TCP/TLS passed on **1/2/4/12 CPUs**, including untrusted certificate rejection,
CA installation, verified HTTPS, SOCKS hostname policy, direct resolution and
wrong-hostname rejection. The first 1-CPU attempt failed before boot because the
sandbox denied localhost socket creation; the approved retry passed. The other
three CPU runs passed on their first approved attempt. Each run retains serial,
QEMU and packet evidence plus command-result logs. No failure was suppressed.
All functional native compatibility probes above passed on their first attempt.
The runner's initial aggregate failure contains formatting and the subsequently
fixed diff whitespace failure; it is retained verbatim, not relabelled a pass.
The top-level R4 summary distinguishes these from security acceptance.
