# FastLane F1.1B: restricted grants prerequisite gate

Date: 2026-10-20. Baseline HEAD: `4c70511d09ec42a915f869c33f56c552c6def959`,
with the existing, uncommitted F1.1P implementation present. **Blocked at
F1.1B-0. No restricted object, grant, syscall, or production interface was added.**
F1.1B and FastLane F1.1 are not complete.

## Blocking finding: lifecycle validation does not cover publication

The random issuance repair is present, but the prerequisite's safe publication
contract is incomplete in two existing authority paths:

* `sys_mint_auth_session_grant`, `kernel/src/arch/x86_64/syscall.rs:1504`,
  checks the trusted broker and generation-tagged pending IPC relationship under
  SCHEDULER. At line 1542 it drops SCHEDULER, then acquires CAP_BROKER and mints
  using only the captured requester PID, credentials, and expiry.
* `sys_account_grant`, in the same file at line 1671, captures a PID/VM-generation
  binding and checks the pending IPC relationship, but drops SCHEDULER at line
  1707 before the mint at line 1718. Capturing a generation is not a lifecycle
  reservation or a revalidation of that generation at publication.

Permitted interleaving:

1. Broker task validates recipient R and pending request under SCHEDULER.
2. Broker task releases SCHEDULER before acquiring CAP_BROKER.
3. Another CPU terminates/reaps R under SCHEDULER. The reaper calls
   `revoke_endpoints_owned_by(R.pid)` under CAP_BROKER, removing R's existing
   auth-session/account grants. It subsequently clears the pending request and
   reclaims R's old VM when safe.
4. Broker task resumes and publishes a fresh grant for the captured identity.
   Neither mint API checks a lifecycle reservation, tombstone, or live process.
   Cleanup has already passed, so this newly published record survives it.

See `Scheduler::terminate_process_by_pid`, `reap_process_resources`, and broker
`revoke_endpoints_owned_by`. R may already be blocked in synchronous IPC and
unowned by a CPU, so waiting for R's execution stack does not serialize the
separate broker task. CAP_BROKER serializes individual table operations but
does not cover the preceding scheduler authorization.

This establishes stale publication and incomplete exit cleanup, not a demonstrated
credential theft or PID-reuse exploit. Account consumption still checks its
binding and auth-session consumption still checks PID and expiry. The audit
does not claim these checks are bypassed, or that ordinary random tokens remain
predictable. The blocking requirement is that authority cannot be newly
published after its validated lifecycle has ended.

Repair these syscall transactions before proceeding: retain existing lifecycle
serialization through bounded broker commit, or reserve an operation tied to the
exact process incarnation, VM identity/generation and IPC request, and revalidate
it at commit. Check `is_exiting()` for relevant participants. Coordinate cleanup
with these reservations. Do not add userspace PID checks or permanently ban a PID
in the broker. Add actual syscall/lifecycle interleaving tests for both orders,
including exit and exec, then rerun F1.1P acceptance. The broker-only reproducer
below is insufficient to validate that repair.

## Prerequisite review

Read the F1 report, F1.1 authority audit, F1.1P report, existing validation
summaries, and the broker, entropy, IPC, SHM, process identity, VM and reaper
paths. Existing summaries are historical evidence, not fresh F1.1B passes.

| Property | Current observation | Gate assessment |
| --- | --- | --- |
| Unpredictable ordinary issuance | `issue_token` obtains conditioned entropy; no counter-XOR fallback; 62 payload bits | Repair present; no new prediction exploit found |
| Collision/exhaustion | 16 attempts, cross-table active equality exclusion, 8192 active tokens, 4096 endpoints, fallible reservations | Existing host regressions pass |
| Transactional endpoint/SHM creation | Endpoint reserves both tables before commit; borrowed SHM preparation holds broker through backing/PTE setup | Repair present; existing rollback checks pass |
| Publication vs lifecycle | Auth-session/account validation unlocks before commit | **Blocked**, interleaving above |
| Early boot entropy | Boot initialization precedes user authority; virtio-rng/RDSEED/RDRAND trust policy, no timing fallback; conditioner exhaustion fails closed | Source audited; historical no-source native result retained, not rerun |
| Authenticated process identity | `Foreground::allocate_incarnation` uses checked atomic allocation; scheduler assigns incarnation | Available; not bound by auth-session record |
| VM identity | `AddressSpaceIdentity` includes physical root and checked generation; shared borrowers inherit it | Available; account record captures generation but does not pin lifecycle |
| Compatibility | Fresh focused kernel and IPC/service host suites and kernel target check pass | Does not establish all native/application regressions |

The F1.1P top-level summary reports `f11p_complete: false` with targeted native
1/2/4/12 CPU results. Separate desktop and network evidence also exists; its
existence is not ignored simply because the top-level acceptance list is stale.
No historical native result is relabelled as a fresh restricted isolation test.
The prior missing-media-WAV failure remains separate; media and workspace-wide
checks were not run in this increment.

## Safe validation and raw evidence

Evidence directory: `target/fastlane-f11b-evidence/`. The machine-readable result
is `validation-summary.json`; `checks.json` retains commands, exits and durations.

Two temporary host audit assertions compiled against the actual broker test
harness require rejection when mint follows recipient cleanup. Both fail:

```text
f11b_prerequisite_account_publication_after_cleanup_is_rejected ... FAILED
f11b_prerequisite_auth_publication_after_cleanup_is_rejected ... FAILED
assertion failed: b.mint_account_grant(binding, 1).is_none()
assertion failed: b.mint_auth_session_grant(7, 0, 0, 100).is_none()
```

This runs real broker methods in the audited operation order. It does not run
the syscall, scheduler, hardware entropy input, native process exit, or SMP
interleaving. The host conditioner uses the existing deterministic seed. A
trusted direct mint API accepting a request is not independently a vulnerability;
the syscall's released lifecycle lock makes this order relevant.

The assertions were removed from the source after capture: a correct syscall
serialization repair need not make direct trusted broker calls reject PID 7
forever. Keeping these as permanent ignored tests would test the wrong contract.
`broker-counterexamples.rs`, `tested-capability-tests.rs` and the frozen host
test executable retain the exact reproducer. Rerun the captured assertions with:

```sh
target/fastlane-f11b-evidence/broker-counterexamples-test \
  f11b_prerequisite_ --ignored --test-threads=1
```

Expected exit is 101 (two failed security assertions), not a passing grant gate.
To rebuild in a disposable checkout of the captured source, use the saved
`tested-capability-tests.rs` as `kernel/src/capability/tests.rs`, then run the
recorded Cargo command. The final host check uses the restored source and has
no temporary ignored tests.

| Fresh check | Result |
| --- | --- |
| Broker post-cleanup rejection assertions | **FAIL**, both paths publish |
| Final kernel host library | PASS, 63 tests |
| IPC host library | PASS, 61 tests |
| Audiod host library | PASS, 25 tests |
| SunlightD host library | PASS, 29 tests |
| Kernel bare-metal target check | PASS |
| Native restricted 1/2/4 CPUs and 12-CPU stress | Not run: prerequisite blocked |
| Native scheduler/D1/D2/D3/RecoveryBlocked/network/desktop regressions | Not rerun |
| Complete workspace check | Not run |

## Restricted authority contract — design only

Everything below is deferred and **not an implemented security property**.

Restricted and Legacy backing need immutable, kernel-owned provenance on the
backing record itself. Restricted creation must publish neither a legacy token
nor an automatic RW owner mapping. All legacy page/region resolution, validation,
mapping, token duplication and derivation paths must refuse Restricted backing
before installing any PTE. Ordinary Legacy behavior remains unchanged.

A bounded authoritative grant record must bind exact object ID/generation,
grant ID/generation, authorized issuer, revocation owner, recipient process
incarnation, recipient physical VM root/generation, access rights and lifetime
references. A PID only locates a candidate authenticated through kernel IPC or
explicit kernel policy. Names, headers and bearer possession cannot authorize
the recipient. Generation exhaustion retires a slot; no wrap or active overwrite.

Accept only explicit ReadOnly and ReadWrite requests, with requested rights a
subset of granted rights. Reject execute and unknown encodings; reject all
recipient delegation initially. Permit at most one live grant per object/VM
and initially one mapping view per grant, avoiding ambiguous writable aliases.
Separately authorized RW peers require explicit issuer policy.

| Proposed object | Producer | Consumer | Unrelated VM / legacy mapper |
| --- | --- | --- | --- |
| Producer cursor backing | RW/NX | RO/NX | Denied |
| Consumer cursor backing | RO/NX | RW/NX | Denied |
| Payload backing | RW/NX | RO/NX | Denied |

Use three separate whole-object backings; no cursor/ring semantics are added.
RO PTEs require PRESENT and USER_ACCESSIBLE, clear WRITABLE and set NX. RW adds
WRITABLE. NoAccess after completed withdrawal means no accessible user leaf PTE;
it cannot be advertised by merely deleting metadata.

## Legacy bypass and process/VM boundary audit

Current `resolve_shared_page`, `resolve_shared_region`, `validate_shared_page`,
and `preflight_shared_mapping` resolve live legacy bearer authority. `sys_shm_map`
ignores its reserved protection argument; `map_shared_page` maps whole backing
with READ_WRITE/NX and permits aliases. Broker derivation is IPC-only. Anonymous
mprotect/replacement policy and CoW exclude SHM. None of these facts provides
restricted provenance fencing: there is no Restricted backing type yet.

The actual isolation boundary is a VM. Existing thread/clone borrowers share
root, generation, ledger and PTEs. Future restricted admission must atomically
check a live nonzero incarnation and exact live VM under lifecycle serialization,
reject already shared VMs, reserve exclusive participation even for an unmapped
valid grant, and prevent later borrowing on every native/Linux clone path.
Exec must invalidate old authority before changing VM identity. Ordinary Helios
and threading behavior for unrestricted processes must remain compatible.
Shared-VM support requires VM-wide participant membership, ownership, reference
inventory and lifecycle rules; task-specific grant records alone cannot isolate
tasks sharing PTEs.

## Lifecycle, accounting and lock contract — design only

Track owner, grant, mapping, in-flight and pending-retirement pins separately.
An unmapped valid grant still pins its object. Do not reuse legacy final-view
reclamation for restricted backing. Reserve bounded objects/pages, grants per
object, total grants, views, pending operations and retirement records with
checked accounting before allocation. Reserve grant/view inventory before PTE
installation; rollback must retain no authority or leaked frames/ledger records.

Use existing lifecycle serialization and broker where practical, with documented
SCHEDULER -> PMM -> CAP_BROKER ordering for current SHM setup and broker ->
entropy for issuance. VM ledger mutation and TLB completion must preserve the
existing F1 contracts. Do not introduce another global lock. Future lock-drop
operations need generation-qualified operation pins; do not hold a global
scheduler/VM lock over an unbounded wait.

Close invalidates future admission and releases each reference exactly once.
Mapped views must remain inventoried/pinned until verified exact unmap and all
required TLB acknowledgements. Process exit/exec must serialize invalidation and
cleanup before VM/backing recycling. A failed preflight or acknowledgement must
retain backing. Record data frames, page-table frames, grant/ledger metadata,
operation pins and retired backing separately; zero aggregate drift is not proof
that heap metadata capacity was returned.

General live-grant revocation must remain unavailable or return an explicit
unsupported error until F1.1C/D proves withdrawal. Legacy retirement only fences
new maps and preserves existing PTEs. It cannot implement NoAccess. If safe
production mapping depends on withdrawal, keep restricted fixtures internal.

## Status and next gates

| Increment / deliverable | Status |
| --- | --- |
| F1.1B-0 prerequisite source audit and host counterexamples | Audit completed; security gate **blocked** |
| F1.1P random issuance/reservation repair | Existing implementation; focused host validation passes, complete prerequisite not accepted |
| F1.1B-1 provenance / bypass fencing | Deferred behind gate |
| F1.1B-2 recipient/VM grants and bounded accounting | Deferred behind gate |
| F1.1B-3 protected PTE mappings | Deferred behind gate |
| F1.1B-4 native isolated Ring-3 read/write/fault fixture | Deferred behind gate |
| F1.1B-5 close/exit cleanup and failure injection | Deferred behind gate |
| F1.1B-6 native 1/2/4 CPU gates and 12-CPU stress | Deferred behind gate |
| Implemented but unvalidated restricted functionality | None |
| Rings, Doorbell, production integration and throughput | Deferred; outside this phase |

After the issuance fix, validate grant-create/exit, map/exit, concurrent mapping,
multiple recipients, owner close/map, duplicate close, stale identity/generation,
resource exhaustion and partial-PTE rollback. Native isolated processes must
prove shared pattern reads, authorized RW writes, an expected RO write fault,
third-party denial, RW escalation denial, legacy bypass denial and execute
rejection. Inspect leaf permissions and preserve the kernel after expected
faults. Run all specified compatibility and SMP gates on the final source.

F1.1C/D additionally needs a complete live-view inventory, admission/withdrawal
serialization, bounded pinned revocation work, exact PTE removal, delayed-ack
and migration tests, post-revoke Ring-3 faults and no reclaim before verified
shootdown completion. These remain prerequisites for claiming FastLane F1.1.
Restricted creation/issuance/map counts, frames, cleanup drift and setup latency
are unavailable (JSON null), not zero-cost results. No transport benchmark ran.
