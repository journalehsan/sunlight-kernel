# M6.1B-R2E-P1R — nonblocking caller-owned exec retry

Status: **PARTIAL — BLOCKED by missing nonblocking call-start primitive**.
Date: 2026-11-04. Production FastLane remains **DISABLED**.
No commit, push, P2 race or 2/4/12 matrix was run.

## Implementation and exact blocking boundary

`handle_ipc_call` in `kernel/src/ipc/mod.rs` authenticates SEND, creates the
incarnation/generation-qualified `IpcCallId`, queues the actual message and
installs `PendingIpcCall`. It then unconditionally marks the original caller
`BlockedOnIpc`, removes it from ready queues and returns `WouldBlock`.
`sys_ipc_call` requests rescheduling. A raw invocation does not provide an
asynchronous continuation contract. Server-side defer retains an existing
reply target; it cannot create a runnable caller. `IpcTryRecv` only receives;
notification delivery does not install an ordinary call/reply target.
Hybrid operation 35 requires a live, actual `current.ipc_reply_target` and
validates it against the pending caller. Sending a notification or inventing a
token would not meet this contract. Cancel/deadline/peer closure terminates the
call and therefore cannot preserve an outstanding reply through exec.

The exact missing primitive is an authenticated caller-owned **nonblocking
call start**: reuse ordinary SEND validation, bounded enqueue, call generation,
pending-call identity and reply target installation, while leaving the owning
caller runnable. It would also need explicit completion collection/wait semantics
and existing cancel/exit/exec terminal cleanup. There is no exported API with
that contract in this source. Per the task's existing-API stop condition, no
new IPC ABI, artificial scheduler wake or reply authority was introduced.

The experimental `caller_exec.rs` implementation now removes the raw blocking
call and the entire mapped-page readiness loop. The Actor no longer writes the
unused readiness word. The caller reports the blocker over its authenticated
supervisor endpoint, then exits normally. The supervisor checks caller badge and
channel, acknowledges, reaps the caller and asks the original Actor to exit.
The ring Frame has already been published by the existing round setup, but the
hybrid control call is deliberately **not submitted**. No Actor receive,
outstanding hybrid reply, exec or replacement execution is claimed.

This removes the observed post-withdrawal dereference. The old P1 fault at
`0x300001ff8` was readiness polling after peer withdrawal, not an exec fault.
The blocked path now uses authenticated IPC and process lifecycle observations,
with no mapped-memory polling or permission changes.

## One actual native 2-CPU attempt

Evidence root: `.fastlane-evidence/m61b-r2e-p1r/`.
Native attempt: `native-20261104T130640.313755Z`.
Its matching image: `build-20261104T130621.543606Z`.
The driver verified source, ELF, ISO and embedded payload identities before boot.
Exact QEMU argv is in the native `validation-summary.json`.

Actual receipt:

```text
[FL-P1] event=blocked caller=40 channel=1 reason=nonblocking-call-start-unavailable request_submitted=0
```

Original caller PID/incarnation/VM: **40/52/90**.
Actor PID/incarnation/VM: **41/53/92**.
Supervisor PID/incarnation/VM: **39/51/88**.
There is **no replacement VM**. All eight requested acceptance observations
remain unproved as a joined sequence. In particular, ring publication alone is
not authenticated Actor request acceptance. The P1 parser correctly returns
FAIL; no synthetic success markers were added.

This attempt exposed a separate fixture cleanup-observer error:
`[SESSION-E3-CLIENT] checkpoint=FAIL line=321`. After caller reap,
`await_retirement` unwraps operation 38. Its authorization permits the creator
or a parent whose exiting creator is still present, and its inventory requires
an installed hybrid endpoint. Neither applies after this pre-call stop and
reap. The final blocked branch omits that unavailable observer and explicitly
reports `scoped_inventory=UNVERIFIED`. No observer authority was widened.

That final correction was compiled in
`build-20261104T130841.700537Z`; **final-source native validation is NOT_RUN**.
Only the one requested native attempt was run, and its original source snapshot,
assertion and serial output remain preserved. The final build is not presented
as a guest-tested image.

## Retirement evidence and limits

`retirement-audit.json` joins channel generation 1 mappings and backing frames:

| Region | Frame | Observed retirement |
| --- | --- | --- |
| 0 | 534974464 | Both mappings removed; TLB completions precede one free |
| 1 | 534986752 | Both mappings removed; TLB completions precede one free |
| 2 | 534990848 | Both mappings removed; TLB completions precede one free |

All six scoped unmaps had `targets=0`; scoped remote ACK count is **zero**.
The boot-time generation-maximum remote SHM witness is excluded. Caller and
Actor are actually reaped; channel contract reports lifecycle closure.
These are blocked-path exit receipts, **not exec retirement evidence**.
Policy/grant/receipt/pending/deferred inventory zero, one hybrid terminal
outcome, stale-authority adoption rejection and replacement generation remain
**UNVERIFIED/NOT_RUN**. Physical release is not inferred from metadata zero.

## Focused checks and source identity

- Reply identity host tests: PASS, 2 tests.
- Exec parser tests: PASS, 9 tests.
- Existing R2 parser controls: PASS, 15 controls.
- P1 parser negatives: PASS, rejects empty, blocked, fabricated marker and
  fake-completion input; `p1-parser-negatives.json`.
- Experimental kernel check: PASS, including after the final correction.
- Experimental service/kernel builds and embedded-image identity: PASS.
- Ordinary kernel release build: PASS; `ordinary-release.json` and log.
- Production image/artifact exclusion: recorded separately in production
  manifests; default feature routing remains disabled.
- Native: FAIL/BLOCKED as described above; no exec lifecycle success.

HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
Final canonical executable/configuration source-map SHA-256:
`d94201ca4327615afeabfc2a372b83a883a6c5762b1e5d30a71c10e962b7b76b`.
`final-source.json` records the full map, dirty tree and native/final inequality.
Build and native directories archive their own sources, commands and artifacts.
`baseline/` and `final-tree.patch` preserve the pre-existing dirty worktree.
Prior P1 evidence was not modified or deleted.

Exact reproduction from the repository root (native command reproduces the
preserved attempted snapshot only; current final source intentionally differs):

```sh
cargo test --locked --offline --target x86_64-unknown-linux-gnu -p sunlight-kernel --lib reply_identity -- --test-threads=1
python3 -m unittest discover -s tools -p test_fastlane_account_exec.py
python3 tools/fastlane_m61b_r2_controls.py .fastlane-evidence/m61b-r2/native-20261104T113833.518916Z/2cpu.serial.log
cargo check --locked --offline -p sunlight-kernel --bin sunlight-kernel --features fastlane_hybrid_lifecycle_test,key_inject
python3 tools/fastlane_m61b_r2_validate.py --workload p1 --section build --cpus 2 --evidence-root .fastlane-evidence/m61b-r2e-p1r
python3 tools/fastlane_m61b_r2_validate.py --workload p1 --section native --cpus 2 --image-manifest .fastlane-evidence/m61b-r2e-p1r/build-20261104T130621.543606Z/validation-summary.json --timeout 90 --evidence-root .fastlane-evidence/m61b-r2e-p1r
cargo build --locked --offline --release -p sunlight-kernel --bin sunlight-kernel
python3 tools/fastlane_m61b_r2_validate.py --workload p1 --section production --cpus 2 --evidence-root .fastlane-evidence/m61b-r2e-p1r
```

The next implementation prerequisite is the missing caller-owned call-start
contract. A successful pilot cannot be obtained by fixture ordering alone with
the currently exported IPC primitives. P2 remains gated.

---

# M6.1B-R2E-P1 — caller-owned exec with outstanding hybrid reply

Status: **PARTIAL**. QEMU now boots and the authenticated hybrid operation runs,
but the caller does not reach exec while its reply is outstanding.
Production FastLane remains **DISABLED**. No commit or push was performed.
This is a sequential lifecycle pilot; it provides no concurrent-race proof.

## Retry after sandbox removal — 2026-11-04

The original QMP bind failure remains preserved. With the sandbox disabled,
the paused QEMU diagnostic passed. No conflicting QEMU process was running at
retry preflight, and no old process was killed. All subsequent guest attempts
used separate evidence directories and socket paths.

Current receipt: `.fastlane-evidence/m61b-r2e-p1/retry-20261104T124523Z/summary.json`.
The evidence root's `latest.json` points to this receipt; the initial root
`summary.json` is preserved as historical evidence.
HEAD remains `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
The exact final source map equals both final build and native manifests.
Canonical source SHA-256:
`a69a5831128e79ebf31bb9859366d66ab2289ed4f4f6263f82a113f093091e84`.
Current dirty status/hash, full tracked patch, source map, reproducible build
commands, QEMU command and disassembly command are retained in the retry receipt.

| Attempt | Actual result |
| --- | --- |
| `native-20261104T124004.460639Z` | QEMU boots; assertion at `lifecycle.rs:219` before request publication. The standalone fixture incorrectly assumed policy generation 1 was stale. |
| `native-20261104T124239.220125Z` | QEMU boots; assertion at `caller_exec.rs:55` before raw IPC. The fixture attempted to map a view already mapped during connection setup. |
| `native-20261104T124523.069476Z` | Real authenticated ring request and held deferred reply; no exec entry; supervisor and Actor timeout, followed by caller polling fault after view withdrawal. |

The reused-slot assertion is now restricted to the existing multi-round R2
modes: P1's first standalone policy legitimately has generation 1. P1 still
checks stale policy generation 0 and stale channel generation 0. No reused-slot
P1 proof is claimed. Both peers now retain their original mapped consumer-page
address rather than invoking the map-once syscall again. Neither change alters
kernel authorization, mapping, scheduler or exec behavior. Final service/kernel
build and ISO identity checks pass in `build-20261104T124447.647879Z`.

The final guest records caller PID **40**, incarnation **52**, original VM
generation **90**; Actor PID **41**, incarnation **53**, VM generation **92**.
Channel generation **1**, policy ID/generation **1/1**, endpoint **37**, call
generation **7**, request ID **1** and operation **65552** (`TRANSFORM_16`) join
the native ring request to an authenticated deferred IPC reply. The Actor
records `event=held`. No bearer token is needed in the public correlation.

There is **no** `[FL-P1-K] event=exec-enter`, installation receipt or replacement
entry. The observed terminal is **PeerClosed(7)** when the Actor exits after its
command wait expires, not the required exec-driven `CallerRetired`. The caller
then faults at RIP `0x437660`, VA `0x300001ff8`, in VM 90. Archived disassembly
maps that RIP to `mov (%rbx),%rcx` in the fixture readiness polling loop.
The mapping has already been withdrawn. This is a fixture polling failure after
peer closure, not a crash inside exec or rejection by its ownership checks.

Source and observed ordering identify the blocker: raw IPC marks the caller
`BlockedOnIpc` and requests rescheduling. The Actor's memory flag does not wake
that blocked process. In this guest, the continuation cannot reach exec before
the pending call becomes terminal. No larger timeout, scheduler wake override,
worker exec, supervisor impersonation or relaxed ownership check was introduced.
This observation does not prove every possible authorized caller-exec design
impossible; it rejects this observed raw-continuation execution.

Three channel backing frees and zero channel remote ACKs occur during failed
attempt cleanup. They do not establish successful-exec retirement. Final scoped
zero inventory, exec-driven exactly-once release, replacement adoption and late
reply after exec remain unproved. The separate boot-time remote SHM witness is
not a remote ACK for this hybrid channel. Ordinary release and fresh production
artifact exclusion checks were not run after the failed native gate; production
feature defaults and routing remain unchanged and disabled.

The smallest remaining proof is a legal original-caller continuation that reaches
ordinary exec with the reply still pending, followed by replacement entry and
all stale-authority and physical-retirement checks. P2 remains gated on that
sequential success. The following sections preserve the initial delivery
snapshot, including its then-current sandbox stop and source identities.

## Initial delivery snapshot

## Preserved source and artifacts

Date: 2026-11-04. HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
Evidence root: `.fastlane-evidence/m61b-r2e-p1/`.

- `baseline/identity.json` records the original dirty tree and complete source
  map; `baseline/tree.patch` preserves tracked changes before implementation.
- Baseline canonical source SHA-256:
  `44a37f07d9463a525822b86fcc424a0502ee3de8e850fa9487705fe1b3d97a19`.
- Built and attempted source SHA-256:
  `7b065f186232cb2c6399418f6ecbdf3e0d77c1380b0db226644fcf5bd9ac15d9`.
- `summary.json` records the current source map, dirty status/hash, prior-R2
  differences, build-source equality, failures and validation boundaries.
  `final-tree.patch` preserves the tracked tree after implementation.
- Hashes follow `fastlane_f11b0e_validate.sources`: sorted compact JSON of the
  executable/configuration source map. Markdown and ignored evidence are outside
  that map. Build/native directories also archive all selected sources,
  including pre-existing untracked files, with artifact indices.

Accepted experimental build:
`build-20261104T122734.422295Z/validation-summary.json`.
Kernel ELF SHA-256:
`1b209897b2c97599897077a1df6fcf4bd4c6eae79af7cc67211c94093f407770`.
ISO SHA-256:
`7985b4221b89a8db87648d93015e68c86d558b24422232ae311f57c324557d12`.
Embedded `session-race-probe` SHA-256:
`57070dcf7fc30f160ca088fc8d0c3c0927967e9e66457bee2091bd98ef7bd1ab`.
The driver verified extraction of the same kernel ELF from the ISO and the
embedded service payload identities. This proves packaging, not execution.

## Ownership audit and chosen contract

The audit covers the existing R2/R2E documents, authenticated hybrid caller and
Actor in `shared_ring/operation/lifecycle.rs`, Account exec replacement fixtures,
`sys_exec`, `exec_into_process`, the IPC exec-retirement functions, channel/policy
retirement and the independent R2 physical-lifetime observer.

Only the original current process can exec its image. `sys_exec` rejects trusted
service managers, exiting processes, non-owning tasks and address-space owners
with unreaped borrowers. A shared-VM worker cannot exec its owner's image and
also prevents owner exec until reaped. No worker or privileged impersonation is
used here. Successful image preparation withdraws grants, image roles and
channel/policy participation before VM activation. Failed preparation preserves
the old image's authority. Successful replacement keeps PID/incarnation and
changes VM generation, then retires incoming and outgoing IPC authority.

The ordinary blocking IPC wrapper yields after `WouldBlock`. The chosen candidate
issues one raw register-IPC call from the original owning caller, then waits for
the Actor's retained-response signal before invoking `sunlight_libc::exec`.
The kernel still marks that caller `BlockedOnIpc` and requests rescheduling.
The candidate therefore depends on an ordinary continuation window; its
reachability is **unproven**, not an ownership exception or a proven impossibility.
The timer, ownership checks, borrower checks and scheduler remain unchanged.

Chosen ordering: **exec first, old reply publication second**. The expected
single operation decision is `CallerRetired`, with no reply delivered into the
replacement. This is an old-call withdrawal, not a terminal message consumed by
the replacement. The existing hybrid sweep records it after old call retirement.

| Identity | Intended joined evidence | Actual native observation |
| --- | --- | --- |
| Caller PID/incarnation | Hybrid authorization and exec entry/install | NOT_RUN |
| Original VM | Authorization `cv`, exec-entry `old_vm` | NOT_RUN |
| Replacement VM | Exec-install `new_vm`, same PID/incarnation, userspace entry | NOT_RUN |
| Executable | Ordinary exec path `/bin/session-racer`, embedded probe hash above | Build identity only; authorization/success NOT_RUN |
| Channel generation | Original authenticated handle and authorization | NOT_RUN |
| Policy identity/generation | Authorization `policy`/`pg` | NOT_RUN |
| Operation | `TRANSFORM_16`, request ID 1, native request ring | NOT_RUN |
| Reply authority | Endpoint, caller PID/incarnation and call generation; Actor-local deferred token | NOT_RUN |

Bearer tokens are retained privately for the fixture and are not emitted in its
new receipts. No numeric PID reuse is forced.

## Implemented sequential fixture

`FL-R2E-P1` starts a supervisor and an original caller using the existing
authenticated caller route. That caller starts the existing trusted service
Actor, establishes its authorized directional connection, publishes a real
`TRANSFORM_16` ring Frame and issues the corresponding authenticated IPC call.

Mode 15 makes the Actor authenticate the operation, defer its reply, consume the
ring Frame, execute the transform and retain its response privately. It signals
readiness in an unused word of its consumer-owned page, visible read-only to the
caller. This is fixture coordination, not a new production ring protocol.
The caller bounds its polling loop and invokes ordinary exec itself. A timer
may park it before exec; the supervisor separately bounds replacement-entry
waiting at ten seconds. Neither wait fabricates exec success.

Debug-only kernel receipts report current caller/incarnation, old VM, ownership,
pending-call presence, executable path, installed VM and entry address. The
replacement route checks the same PID, rejects old-channel attachment/operation
adoption and sends an authenticated entry message to the supervisor. Only after
that message does the supervisor ask the original Actor to attempt the old reply
and a duplicate. Both must fail. The Actor exits through its delegated endpoint;
the supervisor does not impersonate its parent or reap a foreign child.

The existing scoped inventory observer is sampled after caller replacement exit
and Actor exit. No exec, reply-authority, capability, scheduler or retirement
behavior was weakened. Changes outside fixture/tool code are two gated exec
diagnostic receipts in `sys_exec`.

## Controls and retirement boundaries

The pilot includes late-publication and duplicate-publication rejection after
replacement entry, plus rejection of old-image channel adoption. Existing round
setup retains stale policy, reused policy slot and stale channel generation
controls. Existing reply-identity tests and R2 parser mutations passed.
Prior R2 foreign-caller/reply-adoption evidence is historical; the relevant
channel/IPC authorization implementation is unchanged, but it is not a new P1
native observation. None of these native controls executed in this attempt.

For acceptance, join the authorization, exec and terminal identities to all
grant, receipt, pending/deferred/ordinary reply, waiter, policy, view and queued
call inventories. Every scoped field must reach zero. Independently join all
three backing frames and both roles' mappings through removal requests, TLB
completion and exactly-once physical release. Check for reuse only after that
retirement. Zero metadata alone does not prove physical reclamation.

P1 has **no** observed inventories, physical release, frame reuse or TLB evidence.
Zero-target retirement must not be described as a remote ACK. Accepted R2's
remote-witness implementation is unchanged, but the whole source map differs;
its seven historical witnesses are not counted as P1 releases or remote targets.
The new receipt parser checks the sequential identity contract and explicitly
leaves independent physical-lifetime verification required; its PASS alone
would not establish P1 acceptance.

## Validation and preserved failures

| Stage | Result |
| --- | --- |
| Existing reply-identity host tests | PASS, 2 tests; `host/reply-identity.log` |
| Account exec parser controls | PASS, 9 tests; `host/exec-parser.log` |
| Existing R2 parser mutations | PASS, 15 controls; `host/r2-parser.log` |
| Experimental kernel type-check | PASS; `experimental-check.log` |
| Fixture service/kernel build and ISO identity | PASS; accepted build above |
| One bounded 2-CPU pilot | Launch failed; guest NOT_RUN |
| Caller-owned exec, replacement entry, stale publication | NOT_RUN |
| Targeted physical/resource verification | NOT_RUN |
| Ordinary release build | NOT_RUN; stopped at sandbox blocker |
| Fresh production artifact exclusion audit | NOT_RUN; stopped at sandbox blocker |

First build `build-20261104T122713.863278Z` failed with Rust E0603 because the
release-command handler's visibility excluded its caller module. The visibility
was corrected to the containing shared-ring module; the next build passed. Both
source snapshots and logs remain preserved. No guest ran for either build.

Native attempt: `native-20261104T122810.959154Z/validation-summary.json`.
The retained `2cpu.qemu.log` reports:

```text
Failed to bind socket to /tmp/fl-channel-native-20261104T122810.959154Z-2.sock: Operation not permitted
```

`2cpu.serial.log` contains **0 bytes**. This is a host sandbox launch failure,
not a guest exec failure and not evidence of a process-ownership blocker.
There are no actual caller, replacement, channel or reply identities to report.
No permission escalation, alternate control transport or second native attempt
was used. Per the explicit stop instruction, subsequent execution checks were
not run. No attempts were aborted or deleted.

## Reproduction and remaining scope

Exact argument arrays are in `commands.json`; build summaries include environment
and per-command logs, and the native summary preserves the exact QEMU invocation.
From the repository root:

```sh
cargo test --locked --offline --target x86_64-unknown-linux-gnu -p sunlight-kernel --lib reply_identity -- --test-threads=1
python3 -m unittest discover -s tools -p test_fastlane_account_exec.py
python3 tools/fastlane_m61b_r2_controls.py .fastlane-evidence/m61b-r2/native-20261104T113833.518916Z/2cpu.serial.log
cargo check --locked --offline -p sunlight-kernel --bin sunlight-kernel --features fastlane_hybrid_lifecycle_test,key_inject
python3 tools/fastlane_m61b_r2_validate.py --workload p1 --section build --cpus 2 --evidence-root .fastlane-evidence/m61b-r2e-p1
python3 tools/fastlane_m61b_r2_validate.py --workload p1 --section native --cpus 2 --image-manifest .fastlane-evidence/m61b-r2e-p1/build-20261104T122734.422295Z/validation-summary.json --timeout 90 --evidence-root .fastlane-evidence/m61b-r2e-p1
```

Production feature defaults and production routing were not changed. New
userspace routes are behind `fastlane-contract-test`; kernel exec diagnostics
require debug assertions and `fastlane_hybrid_lifecycle_test`. This is a source
exclusion statement, not a fresh release-artifact verification.

The smallest missing executable proof is a guest launch in an environment that
permits this QMP socket, followed by observation that the original caller reaches
ordinary exec while its authenticated hybrid reply remains pending. Then require
replacement entry, late rejection, unique retirement, independent physical/TLB
checks, ordinary release build and production exclusion before accepting P1.
If the continuation is parked, preserve that first guest failure and diagnose
the scheduling boundary without changing ownership or quiescence checks.

P2 remains gated on P1. P2 must contend the original caller's ordinary exec with
the original Actor's authenticated reply publication on distinct CPUs, observe
actual competing lifecycle serialization, exercise both winner orders repeatedly
and repeat identity/security/resource checks. Sequential timestamps do not prove
contention. A full 2/4/12 matrix is outside this P1 delivery.
