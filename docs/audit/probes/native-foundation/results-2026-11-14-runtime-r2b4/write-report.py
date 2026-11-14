from pathlib import Path
import json
E=Path(__file__).resolve().parent;R=E.parents[4]
summary=json.loads((E/'validation-summary.json').read_text());matrix=json.loads((E/'cpu-matrix.json').read_text());integrity=json.loads((E/'source-integrity.json').read_text());life=json.loads((E/'lifecycle-witnesses.json').read_text());retire=json.loads((E/'resource-retirement.json').read_text())
rows='\n'.join('| '+k+' | '+v+' |' for k,v in summary['gates'].items())
cpu='\n'.join(f"| {x['cpus']} | {x['lifecycle']['passed']}/{x['lifecycle']['total']} | {x['C']['passed']}/73 | {x['Cxx']['passed']}/21 | {x['heap_abi']} | {','.join(sorted(set(x['tls_once'].values())))} |" for x in matrix['cells'])
controls=retire['existing_native_controls'].get('guests',[])
fault='; '.join(f"mode {x['mode']}, {x['cpus']} CPUs: {x['status']} ({x['reason']})" for x in controls) or 'NOT_RUN; see preserved runner/build logs'
modified='\n'.join('- `'+p+'`' for p in integrity['changed_from_baseline']+list(integrity['new_kernel_file']))
text=f'''# Native runtime R2B.4: retained thread-group authority

Date: 2026-11-14. Baseline/final Git revision `{summary['revision']}`, branch `main`.
Changes are uncommitted. **Overall: PARTIAL. R2B.4 is not COMPLETE.**

The proven initial-thread/VM lifetime defect is repaired in the exercised native
paths. The original creator can execute pthread_exit while one or two borrowers
continue, create/join a further thread, and eventually retire the original group.
Forced SIGKILL remains effective after the original task is physically quiescent.
The required terminal race, authenticated session/IPC/generation and integrated
retirement fault witnesses remain open; successful C/C++ guests do not close them.

Evidence root (**E**):
`docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b4/`.
`validated/` contains the authoritative final-source executions. Initial pilot/matrix
observations remain separate and are never substituted for final-source results.

| Gate | Status |
|---|---|
{rows}

G3 stays PARTIAL despite successful final-thread reclamation: externally observed
parent/group completion and exact terminal contention need additional native
qualification. G4-G7 retain the uncovered authority, session and retirement edges.
G5 is not promoted from balanced resource counters or standalone memory controls.

## Baseline, design and source preservation

Read the R2B.2/R2B.3 reports, R2B.3 summary and ownership plan, threading T2R race
closure and KFP-2A.6D concurrent-cleanup evidence. Inspected actual process,
admission, scheduler, syscall, VM/ledger, retirement, IPC/capability and foreground
source. `source-baseline.json` records all {integrity['baseline_file_count']} regular
file hashes and {integrity['symlink_count']} symlink targets; `baseline-worktree.log`
records the full original Git status. The installed-toolchain locked native build
passes before changes and against final source. This is not a clean-room or
bit-identical reproducibility claim.

A fresh two-CPU negative baseline is preserved at `initial-baseline/`: the worker
is alive and initial TLS cleanup runs, followed by the kernel's
`address-space-owner-exit` kill. Guest runner PASS only means that the negative
observation completed; the required worker-survival contract FAILS there.

Selected **Option A: retain Process as the shared lifetime authority**. The
pre-implementation selection, alternatives, invariants, files, failure and rollback
strategy are in `migration-decision.md`; before/after diagrams and the complete
resource/identity map are in `ownership-before-after.md`.

Option B would move resource fields out of Process, add allocation and migration
paths, and duplicate adapters. Option C's VM/retirement identity is reusable for
membership/barriers but does not itself express group terminal status or parent
completion. The retained Linux leader is an existing structural precedent; its
personality-specific rules are not reused as native authority. A retained native
Process plus one terminal receipt is sufficient for the implemented contract.
No redundant ThreadGroup object, owner promotion or refcount is added.

## Implementation and exit contracts

`native_group_exit` is independent of task state. None permits live membership;
Some(first code) records one accepted group terminal transition under SCHEDULER.
Finished plus pending cleanup can therefore describe an initial task whose group
is still live. Existing Reaped remains the physical/reuse boundary. Bounded scans
use exact VM root/generation, rather than a raw creator TID, for membership.

* Syscall20 is caller-only. Native pthread and worker trampolines keep that ABI;
  their TLS/once cleanup is unchanged. Both old implicit owner-kill branches are
  removed. Initial-owned foreground cancellation and parent completion are deferred.
* Existing syscall157 and internal fatal exits take explicit native group scope.
  C exit/main return and std exit retain157. Existing IPC ProcessExit::exit and
  Rust libc exit helpers migrate to157 so process-exit helpers retain group intent.
* The original Process keeps shared VM, page tables, mapping placement, opt-in
  shared descriptors/cwd and quota ownership. Native descriptor-owner resolution
  now checks the exact VM identity as well as the retained PID.
* Finished initial owners with no group receipt can accept authorized SIGKILL.
  Existing signal/foreground/manager/generation checks still run first. Task
  termination of a borrower remains distinct from original-owner group termination.
* Natural self-exit and external termination of the final live member both publish
  group completion. The original thread never has to run again. Parent wait uses
  the native group receipt rather than initial task Finished alone.
* ProcessIsAlive, including its privileged PTY incarnation qualification, follows
  group lifetime. Exact-session termination can find a retained initial owner.
  Borrower-owned foreground scopes retain their existing task cleanup.

Capabilities and endpoints do not move to another thread. Task-scoped original
endpoints close when that task exits; a dead receiver does not become a group
receiver. Workers keep their own authorized resources, not owner impersonation.
The existing is_exiting authority guard still rejects dead tasks. Group liveness
is used explicitly for supervision/PTY, rather than globally weakening that guard.
An authenticated endpoint-owning retained-group workload has not been executed.

## Native lifecycle witnesses and coordination

`lifecycle.c` establishes live workers before initial exit; initial TLS cleanup
publishes its announcement. A same-VM worker then queries existing syscall156.
The syscall still rejects the original owner as a non-borrower: no reap result is
fabricated and no public ABI changes. An observation-only native_c_guest hook
records the actual owner PID/incarnation/VM, Finished state, no physical stack pin,
no owning/queued core and no current CPU before returning to that worker.

Only an observed `quiescent=1` snapshot followed by actual worker output qualifies
post-owner progress. Sleeps are scheduling opportunities, never proof. The source
and ELF hashes bind the query-before-output order. Multiple-borrower fixtures
observe each worker independently; the final borrower waits for syscall156 to
acknowledge the first borrower's Reaped state before its own exit. The terminal
trace and final owner resource receipt then establish final group retirement.
This is an observed handoff/progress order, not a simultaneous terminal-contender
race or a claim that delays force a particular SMP schedule.

| Witness | Result |
|---|---|
| W1 initial exits, borrower survives | {life['coverage']['W1']} on 1/2/4 |
| W2 two borrowers survive | {life['coverage']['W2']} on 1/2/4 |
| W3 final borrower and owner retirement | {life['coverage']['W3']} on 1/2/4 |
| W4 explicit group exit/main return | {life['coverage']['W4']} on 1/2/4; returns17, exit23, raw15729 |
| W5 forced termination | {life['coverage']['W5']} on 1/2/4; live owner and physically quiescent retained owner, exit137 |
| W6 concurrent final exit versus forced group kill | NOT_RUN; no independently coordinated external terminal actor |
| W7 session/PTY | PARTIAL; host exact-session scan, real retained-owner terminal-tab witness NOT_RUN |
| W8 stale group identity/reuse | PARTIAL; existing host identity checks, new native stale operations NOT_RUN |
| W9 retirement failure | PARTIAL; standalone native memory controls below, integrated group failure NOT_RUN |
| W10 churn | PARTIAL; ten sequential groups per CPU balance measured resources; full heap/ledger census NOT_RUN |

Every recorded witness in `lifecycle-witnesses.json` includes CPU, exact identities,
coordination, kernel/ELF identity, output/accounting, outcome and evidence path.
No W6 overlap is inferred from W5. `retained-admission.c` separately verifies that
ProcessIsAlive remains1 and a surviving worker can create and join a new native
thread after original task retirement. TLS/once tests are separate regressions.

## Memory, failure and forward progress

The owner reaper still waits for every same-VM borrower to be Reaped, execution
pins to clear, all queue/current/owning-core references to end and VM active mask
to be zero. Only then does the existing descriptor, IPC/capability, shared mapping,
backing, page-table and private-state cleanup run. Existing mapping retirement,
remote acknowledgment and quota paths are unchanged; no security assertion or
capacity was weakened. reclaim_user_space still refuses persistent retirement
obligations or active CPUs; uncertain shared-page cleanup retains backing and
fails stop. Cleanup-pending is cleared and the stack removed before Reaped/reuse.

Normal, final-borrower, explicit and forced guests record zero owner frames,
descriptors and kernel stack at final reap and balance file/private/page-table
classes, VFS handles and temporary names. These are not a full heap census or
proof of every shared mapping/quota edge. W10 also shows repeated original and
borrower Reaped transitions and fresh VM generations; it does not substitute for
stale-generation operations against a live successor.

Fresh existing two-CPU retirement controls: {fault}.
Their exact PTE/owner/acknowledgment/backing observations and failures remain in
`validated/retirement-controls/`. These controls use the existing memory fixture,
not a native borrower group. A failed control is retained as a failure; no earlier
historical PASS is substituted. Group-specific failure injection remains NOT_RUN.

The first standalone control attempt at `final/retirement-controls/` FAILS and is
preserved. Its MM-0 assertion expected an unreaped borrower, but calling the new
forced helper synchronously reaped that synthetic task. The fixture now stages a
Finished, cleanup-pending borrower explicitly; both retention assertions remain,
and physical release still waits for explicit borrower reap. The corrected
`validated/retirement-controls/` attempt is a separate execution, not an overwrite.

A further source boundary remains: SHM syscalls use current_process_mut and
per-task mapped_shared tracking. This milestone retains initial-owner views by
retaining that Process, but does not migrate borrower-created SHM views into a
new group-wide mapping authority. Worker-owned SHM view continuity across that
worker's death requires a native witness and authority decision; it is not
covered by anonymous-mapping C tests. No broad SHM/IPC refactor was attempted.

Admission remains the existing fallible prepublication RAII transaction under one
scheduler guard; no new attachment allocation or counter can underflow/overflow.
The existing bounded task table bounds scans. Terminal receipt precedes nested
termination/callbacks, so a reentrant contender cannot publish a second status.
No new spin loop, blocking wait, IPI-ack cycle or callback interface is added.
A failed/uncertain cleanup keeps the existing fail-stop ownership contract;
callback-panic recovery and exact concurrent cleanup schedules remain unqualified.
The retained original kernel stack/resource row is intentional until final cleanup,
not a runnable zombie required to perform that cleanup.

## CPU, language and security regressions

| CPUs | Lifecycle cases | C | C++ | Heap/ABI | TLS/once |
|---|---:|---:|---:|---|---|
{cpu}

TCP loopback executes on2/4; TCP1 is NOT_RUN because the existing runner accepts
only2/4. Native12 is NOT_RUN because its runner accepts only1/2/4. No acceptance
criteria were weakened to add CPU coverage. Full C++ includes static initialization,
exceptions/unwind, RTTI and expected fatal cases. Fresh pthread ABI and C++
ABI/PROVENANCE checks pass; protected vendor hashes are preserved. The old libm
packaging difference is not silently relabeled as an exact old upstream manifest.

Kernel/security host and production-check executions are individually recorded in
`security-regressions.json` and `validated/executions.json`. Kernel host uses the actual
broker/pure lifecycle logic with hardware/process adapters; it does not execute
the production scheduler reaper. The foreground host adapter executes the actual
session scan with mock task termination; it does not prove live PTY behavior.
No broad kernel security PASS follows from C/C++ regressions. Missing authenticated
IPC/capability, PTY, generation-reuse and final-exit/kill schedules remain explicit.

## Files, integrity and handoff

Files changed relative to the captured baseline, including the new adapter:

{modified}

No production TLS/once/handle algorithm was changed. The evidence C fixtures and
batch-runner support are test infrastructure. Full original worktree changes are
preserved; inverse-hashing the single edit in the already dirty libc lib.rs
reproduces its baseline hash. `source-integrity.json` records the exact final
source, all expected deltas, missing/symlink checks, protected vendor/lock/report
preservation and final full status. The main validation checks the frozen source map at command boundaries and
assessment verifies it again; each guest retains its own ELF/kernel/serial identity and raw build output.
Earlier negative/pilot attempts remain separate. No commit, push, reset or clean.

Recommended **R2B.5: retained-authority and retirement qualification**. First close
W6-W9 and the uncovered R2B.4 gates: real external final-exit/kill overlap,
authenticated IPC/capability continuity, real PTY/tab/session teardown, stale
generations across reuse, and group-integrated uncertain retirement with exact
mapping/backing/quota/heap observations. Preserve failures and first terminal
ownership. Keep Condvar, autonomous detached reclamation, CPython and unrelated
refactoring outside that work. R2B.4 must not be called COMPLETE until G1-G9 PASS.
'''
(R/'docs/runtime/native-runtime-r2b4-group-authority-migration.md').write_text(text)
print('Wrote main report')
