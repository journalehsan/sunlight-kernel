# M6.1B-R2E — authenticated exec versus hybrid reply publication

Status: **PARTIAL**. Native exec/reply contention is **NOT_RUN**.
Production FastLane remains disabled. No commit or push is authorized.

On 2026-11-04 the user narrowed execution to less sensitive checks. This
delivery records the source audit and offline evidence checks; it does not add
a native contention hook or change process, scheduler, capability or IPC code.
The 2-CPU pilot has not passed, so repeated winner-order and 2/4/12-CPU R2E
execution remain gated. This is not a native failure or a proof of impossibility.

## Identity and preserved evidence

HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
Durable audit receipts are under `.fastlane-evidence/m61b-r2e/audit-*/`.
Each `summary.json` records exact dirty status, per-file executable/configuration
SHA-256 values, canonical sorted compact JSON hash, commands and return codes.
`tree.patch` preserves tracked changes. Existing untracked sources and all prior
evidence remain in place. Markdown and evidence are outside the existing R2
code identity selection in `tools/fastlane_f11b0e_validate.py::sources`.

Current audit: `audit-20261104T120658.493587Z/summary.json`.
Canonical source SHA-256:
`44a37f07d9463a525822b86fcc424a0502ee3de8e850fa9487705fe1b3d97a19`.
The complete source map equals accepted R2 and remained unchanged during checks.
Two reply-identity host tests, nine Account-exec parser tests and fifteen R2
parser mutations PASS. Both archived production ELF hashes match the prior
exclusion manifest, whose source map also matches. These results do not add a
native R2E execution or expand the prior accepted security scope.

Accepted R2 references:

- `.fastlane-evidence/m61b-r2/resume-20261104T111620.516550Z/acceptance-summary.json`
- `.fastlane-evidence/m61b-r2/build-20261104T113744.703803Z/validation-summary.json`
- `.fastlane-evidence/m61b-r2/native-20261104T113833.518916Z/validation-summary.json`
- `.fastlane-evidence/m61b-r2/native-20261104T114023.907548Z/validation-summary.json`
- `.fastlane-evidence/m61b-r2/native-20261104T114245.413418Z/validation-summary.json`

Reuse requires exact equality of the source hash maps, recorded in the audit
receipt. R2's 98 terminal decisions, 294 backing releases and seven separate
remote witnesses are prior R2 scope, never R2E exec/reply contention evidence.
Earlier failed attempts remain indexed by the prior acceptance manifest.

Existing experimental kernel feature chain:
`fastlane_hybrid_lifecycle_test -> fastlane_contract_test -> fastlane_shared_ring_test`.
The native driver also selects `key_inject`; service builds select
`fastlane-contract-test`. The prior authenticated Account fixture uses
`authenticated_exec_test` and `session-account-exec-test`. These are distinct
fixtures and their results cannot be combined into an observed R2E race.

## Ownership and quiescence audit

`kernel/src/arch/x86_64/syscall.rs::sys_exec` executes only the current process.
It rejects trusted service managers, exiting tasks, tasks that do not own their
address space, and owners with unreaped address-space borrowers. The borrower
check counts matching non-owning VM tasks until `Reaped`, including `Finished`.
A worker cannot exec its owner's image; merely finishing a worker is insufficient
to establish quiescence. Caller-supplied PID metadata does not select an exec target.

Image preparation and installation hold the existing `SCHEDULER -> PMM` guards.
Successful preparation withdraws recipient grants, channel/policy participation
and image roles through the broker before activation. Failed preparation restores
the old image and does not perform successful-exec authority withdrawal.
Successful exec activates a new VM, cleans old shared mappings, reclaims the old
VM, and retires incoming and outgoing reply authority before returning to the
replacement image. PID and process incarnation are retained; VM generation changes.

The existing Actor holds the deferred reply token in its own process-local
`deferred_reply_targets`. The completion syscall derives its server identity from
the current process. A second process cannot publish with that token simply by
receiving its numeric value. The same Actor cannot execute two syscalls at once.
A shared-VM worker has a separate process identity and also blocks owner exec
until reaped. Accordingly the existing service-exec mode 8 is sequential.

Caller-side exec deserves separate investigation. `handle_ipc_call` records the
original call and sets the caller `BlockedOnIpc`; the ordinary wrapper yields and
retries until a terminal result. A worker's call belongs to the worker, not the
owner. However, the raw syscall returns `WouldBlock`, and the IPC path requests
rescheduling; source inspection alone does not establish that every instruction
after SYSRET is unreachable before delivery. A custom raw-syscall continuation
could be a legal candidate, but its actual scheduling window, successful exec,
and competing publication have not been tested. Do not label this possibility
BLOCKED by ownership without a bounded native observation.

The prior Account exec fixture is useful for replacement-entry and retirement
receipts: the Account itself calls `sunlight_libc::exec`, and an independent
caller's borrower does not violate Account quiescence. It does not provide a
second authorized publisher for the Account's own token.

## Required operation contract and linearization

For a future executed operation, join all of these from actual receipts:

| Identity | Required correlation |
| --- | --- |
| Caller | PID, incarnation, original VM generation |
| Actor | PID, incarnation, original VM generation and service role |
| Replacement | Same execing PID/incarnation, distinct new VM generation |
| Channel | Handle generation and exact creator/consumer identities |
| Policy | Policy ID and generation resolved against authenticated service |
| Reply | Endpoint and original call PID/incarnation/generation; server-local token retained privately |
| Terminal | Exactly one decision for that original channel/call |

Completion linearizes at the existing serialized `finish_call` decision.
Successful image preparation is not itself retirement: image authority withdrawal
occurs in `exec_into_process` before VM activation under the same scheduler guard.
`retire_exec_replies` consumes incoming ordinary/deferred targets; live callers
receive `PeerClosed`. `retire_exec_call` withdraws the replaced caller's outgoing
target without publishing a result into the replacement. The hybrid bridge and
sweep reconcile its channel correlation. A future caller-exec receipt must
distinguish withdrawn outgoing calls from delivered terminal messages.

Both decisions serialize through existing lifecycle guards. The test must observe
actual competing acquisition on distinct CPUs and the committed transition;
timestamps or ordered entry messages alone do not prove contention. No new
global lock or scheduler ownership override is justified.

Completion-first must show one accepted original result followed by successful
exec and denied replay. Exec-first must show successful old-authority retirement,
rejected late publication, and no replacement delivery. The test must retain
old identities privately for negative probes without logging bearer tokens.
Native caller/Actor incarnation mutation, old VM and stale channel/policy checks,
live foreign-token ownership, duplicates and replacement adoption each need
separately joined rejection receipts. They are currently NOT_RUN for R2E.
Numeric PID reuse remains NOT_RUN; changing the allocator to force it is outside
scope.

## Resource and validation boundaries

No R2E operation has executed, so no R2E zero inventory, exactly-once release,
remote ACK set, frame reuse safety or successful syscall witness is claimed.
A future receipt must correlate grants, pending replies, channel receipts,
Doorbell waiters, restricted views, backing frames, unmaps, target-specific TLB
ACKs and physical release. Inert channel tombstones are not active authority;
zero metadata is not physical reclamation proof. A zero-target unmap supplies
no remote ACK witness.

The offline audit runs existing reply-identity host tests, Account-exec parser
tests and R2 parser mutations against a preserved accepted native log. Mutated
logs are parser controls only. Return codes and complete output are retained.
Archived production ELF hashes are checked against their exclusion manifest;
that is artifact revalidation, not a fresh release build. Source equality is
checked separately. Previously recorded formatting/Clippy baseline failures and
unavailable Miri remain prior evidence, not checks rerun in this audit.

| R2E stage | Result |
| --- | --- |
| 2-CPU exec/reply pilot | NOT_RUN; deferred after user narrowed tests |
| Repeated winner-order coverage | NOT_RUN; pilot gate unsatisfied |
| 2/4/12-CPU exec/reply matrix | NOT_RUN; pilot gate unsatisfied |
| Native eight negative controls | NOT_RUN for R2E |
| Physical retirement/TLB proof | NOT_RUN for R2E |
| Fresh formatting, Clippy, bare-metal and release build | NOT_RUN; executable source unchanged |
| Miri | NOT_RUN; previous R2 reports unavailable toolchain component |

Reproduce offline checks from the repository root:

```sh
cargo test --locked --offline --target x86_64-unknown-linux-gnu -p sunlight-kernel --lib reply_identity -- --test-threads=1
python3 -m unittest discover -s tools -p test_fastlane_account_exec.py
python3 tools/fastlane_m61b_r2_controls.py .fastlane-evidence/m61b-r2/native-20261104T113833.518916Z/2cpu.serial.log
```

The smallest next step is a separately reviewed bounded caller-side raw-IPC
continuation pilot with the real exec syscall, retaining all normal ownership
and borrower checks. It must first establish whether the original caller can
reach exec while its authenticated call remains live. If it cannot, record the
actual scheduling/authority blocker. Experimental Ring 3 service integration
remains gated on that pilot and the required lifecycle/security proof; this
audit establishes no production readiness.
