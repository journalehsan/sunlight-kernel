# Native foreground/session lifecycle (Phase 2.1C.1)

This document describes the native cancellation boundary introduced on October
14, 2026. It does not define POSIX process groups or job control. Validation
results below distinguish actual guest execution from host checks.

## Audit and ownership

Changed implementation and validation components:

| Files | Responsibility |
| --- | --- |
| `kernel/src/process/foreground.rs`, `kernel/src/process/mod.rs` | Bounded scope/session state, unique publication IDs and authority |
| `kernel/src/sched/mod.rs` | Inherited membership, exit propagation, wake/preemption |
| `kernel/src/arch/x86_64/syscall.rs` | Native scope ABI, broker binding/delivery, I/O cancellation, numeric signal guard |
| `ipc/src/lib.rs` | Credential incarnation, PTY bind message and scope/session wrappers |
| `services/pty_server/src/main.rs` | Role authentication, ETX/hangup routing, deferred waiter release and counters |
| `services/sunlight-terminal/src/main.rs` | Existing keyboard path maps Ctrl+C to authenticated master input |
| `sunshell/src/main.rs` | Foreground scope lifetime, interrupted relay, exit status and prompt recovery |
| `sunshell/src/native_script.rs`, `sunshell/src/script.rs` | Execution hooks, checkpoints, cancellable capture and failure unwind |
| `sunshell/src/native_tests.rs` | Guest lifecycle, isolation, race and resource/latency gates |
| `tools/foreground-scope-proof.rs` | Deterministic tests of production scope state and authority |
| `tools/tests/shell_pty_liveness.expected`, `tools/test_pty_liveness.py` | Lifecycle assertions and correct matrix completion marker |
| `docs/shell/phase2-1-expansion-and-environment.md`, this document | Current architecture, evidence and limits |

Pre-existing workspace version/manifests were dirty before implementation;
normal build tooling also updates generated version metadata. No commit or
push was requested or performed, and no target artifacts are staged.

Before this change, the PTY relay kept direct child PIDs in `fg_pids`, forwarded
ETX as input, and hung up only those children. `NativeRuntime` drained capture
pipes with `poll(-1)` before waiting for children; the synchronous interpreter
could not read the PTY while collecting a substitution. Terminal closure could
therefore wait for the producer's natural completion. The capture-before-wait
ordering is required to avoid the previous full-pipe deadlock.

Each process now contains fixed-size `Foreground` state. A root can own one
execution scope. Scope IDs come from a checked, monotonically increasing u64
allocator that refuses wraparound. They are neither PIDs nor capabilities:
knowing an ID grants no authority. Only its current owner can interrupt or end
it. Membership additionally delegates reporting a fatal execution failure to
that same scope (status 2), without accepting a caller-selected ID. A member
cannot create a detached scope. The scheduler copies membership
when publishing a child or native thread, including descendants created by
`sh -c`. No userspace environment, argument, name, UID or numeric PID sets scope
authority. Scope completion cancels any surviving members and clears the
owner's state. Unique old IDs and retained member termination requests cannot
target a later scope or a reused process slot.

The older numeric signal API is also guarded for scoped tasks and session
controllers (including idle controllers). A member cannot use it to kill its
owner. An unrelated caller cannot use it across sessions. Authenticated ancestor
links use PID plus a checked publication incarnation that survives exec and is
unique for each process/thread publication; they are checked under the scheduler
lock. Existing kernel-installed trusted PTY, TTY, session and display service
identities retain their administrative delivery paths. No UID or name exception
is added. This guard does not establish authorization for every older operation
on unrelated **unscoped** processes; that independent signal-API audit remains
security work outside the scoped lifecycle contract.

The PTY loop begins a scope before executing a submitted line and ends it after
synchronous completion or asynchronous foreground wait/drain. A native runtime
creates a scope only if the caller is neither a member nor an existing owner;
the top-level execution hook releases it on success or error, with destructor
cleanup as a fallback. A reused runtime therefore starts a fresh execution.
Compound commands, functions, subshells and
nested substitutions executed by the shared interpreter consequently use the
same scope. The interactive legacy line dispatcher still requires `sh -c` for
general compound syntax. Background syntax is not introduced by this phase.
Work published outside a foreground scope is excluded from its interrupt.

## Session authority and interrupt path

The broker checks a role token, caller PID **and process generation**, session
ID and nonwrapping session generation. The process generation comes from the
kernel's unique publication incarnation, not a VM identity shared by threads.
A delegated bearer slave is claimed by its first
authenticated successful user. A trusted-broker-only syscall binds that caller
to the session on an explicit `BIND_FOREGROUND_OWNER` request, without altering
the existing kernel TTY byte-ring routing. Ordinary slave I/O does not bind a
controller and does not make a broker test client subject to session hangup.
Only this binding marks a session controller; ordinary descendants inherit the
session identity and scope membership, not controller authority.
Kernel bindings additionally carry the broker's publication incarnation.
Cancel, rebind and diagnostic operations qualify the session by that incarnation,
so a second genuine broker cannot select another broker's identically numbered
session. Broker restart creates a distinct authority domain. The PTY broker
also sweeps sessions whose creator PID and publication generation are no
longer alive, so terminal-client death does not require a GUI callback.

The desktop terminal sends ETX for Ctrl+C using its existing keyboard event
path. The broker consumes ETX only after validating the master role. It then
asks the kernel to interrupt scopes belonging to controllers of that exact
session generation. Other sessions and unrelated services are excluded. ETX
inside a master request cancels once and discards the rest of that request;
bytes in separate requests are independent. The native interrupt character is
fixed ETX; configurable termios VINTR/ISIG and raw-mode exceptions are extension
work. Idle Ctrl+C clears the desktop's local input line without terminating the
shell. Already queued slave input is not a complete canonical line discipline.

The owner records a cancellation reason and remains alive on interrupt. Scope
members receive an unmaskable **native cancellation**, yielding status 130 for
interrupt or 129 for hangup. This is not delivery of a userspace SIGINT handler.
Native signal handlers remain incomplete. A running task is preempted; parked
tasks wake. Forced scope exit takes precedence before syscall dispatch, before
SYSRET, and at the existing timer fatal-exit check, so a restarted blocking
operation or concurrent `exit(0)` cannot erase accepted cancellation. An exit
that already reached Finished/Reaped retains its recorded status.
The internal fatal-execution reason 126 produces status 2; it is not signal 126.
An interrupted asynchronous pipeline records 130 even when its final consumer
has already exited successfully and an earlier producer remains active.

## Wait, IPC, capture and descriptors

Cancellation serializes with publication, completion and exit under the
scheduler lock. It scans existing process metadata, queues termination, and
uses existing wake/IPI paths. It does not reclaim address spaces inside the
scan or change the scheduler's reclaim budget. Parent exit (ordinary, external
termination, or timer fatal signal) cancels an owned scope. Descendant members
remain identifiable even after their intermediate parent exits.

A blocked owner IPC call is cancelled through the existing generation-qualified
IPC terminal-outcome mechanism; an already delivered reply wins that race.
Late replies retain the existing tombstone/drop behavior. The PTY broker also
releases the affected deferred writable waiter. The relay treats cancellation
as an interrupted operation rather than hangup and discards pending relayed
output after interrupt. Restoring a visible prompt still requires the master
to consume its bounded output ring.

Native capture poll and blocking descriptor entry paths reject cancelled owner
work instead of registering another wait. Interpreter checkpoints stop loops
and nested execution. Capture failure cancels the owned tree, drops readers,
waits for all directly tracked children, restores cwd and returns a fatal
expansion error. Capture buffers unwind in nesting order. A cancellation
checkpoint after capture closure rejects partial output before assignment or
successful expansion. Drain-before-wait is retained. Pipe endpoint reference
counts, EPIPE and EOF remain in the existing descriptor implementation.

The existing kernel reaper closes all descriptors, IPC references, shared
regions and memory of terminated tasks. `waitpid` observes child status;
resource reclamation is scheduler-driven and can precede the wait. This phase
does not promise POSIX zombie retention: an old child status can still become
unavailable after a process slot is reused. On broker hangup the shell itself
terminates with 129; kernel cleanup, rather than a userspace wait for every
child, establishes teardown. Ordinary completion and interrupts retain direct
child waits. Complete allocation-failure recovery is not established.

## Disconnect policy and Helios boundary

Authenticated CLOSE_MASTER and CLOSE_SESSION cancel active foreground work and
terminate every live member of the exact generation-qualified session,
including an idle controller and descendants that were reparented. Closing the
shell's slave endpoint is normal shell-side termination; its process exit
cancels remaining owned work. Repeated or simultaneous cancellation is
idempotent. Session slot reuse requires a new generation; old role tokens and
bindings cannot select the new controller. Terminal application/tab close uses
its existing PTY closure path and bounded wait path. The broker's owner-death
sweep handles an unexpected terminal-client exit; broker loss itself remains a
separate service-availability failure boundary.

The Linux signal state, process groups and termios remain separate. A Helios
task launched inside a native scope inherits scope membership and is subject
to the scope's forced cancellation policy, irrespective of Linux SIGINT
dispositions. This is deliberately not POSIX-compatible signal delivery.
Ordinary Helios signal syscalls are unchanged; complete interactive Helios
signal behavior is not claimed.

## Numbered TTY endpoint gate

`sshl6` registration and lookup have different trust boundaries. Registration
recognizes shell endpoints; UserSession discovery permits specific services
(`tty`, `pty`, etc.), not arbitrary numbered shell names. Adding `sshl*` to the
allowlist or bypassing lookup would let authenticated user processes discover
other sessions. No allowlist change is made. A future TTY handoff must carry a
broker-authenticated endpoint tied to session and process generations and
validate it at the receiver, comparable to PTY slave delegation. The legacy
Phase 1.5 numbered-shell lookup gate remains blocked pending that handoff.
The legacy framebuffer TTY keyboard path does not gain Ctrl+C integration here.

## Verification and extension points

Final validation uses a copied ISO, with one completed boot per CPU count and
no automatic retry or enlarged deadline. All combined images use 1536 MiB.
The guest lifecycle gate uses producer PID files as startup barriers, 60-second
producers and an unchanged 2-second cancellation limit. It checks simple work,
pipelines, blocking stdin, output backpressure, descendants, `sh -c` compounds,
nested substitutions, hangup, repeated interrupts, scope authority, stale IDs,
spawn rejection and repeated reclamation. Scope counters report live members
and descriptor references owned by that scope; they do not expose another
owner's scope. Host fixtures exercise the production scope scan and interpreter
capture unwind, while native tests establish scheduling and descriptor behavior.
The diagnostic scope operation reports both live members and remaining member
descriptor references, including references awaiting reclamation after exit.
The repeated gate requires both counters to return to zero and the owner's
next available descriptor to return to its baseline.
The existing control-authorized PTY foreground query additionally reports
session live-process and descriptor-reference counts. Each interactive scenario
must return to the idle shell baseline (one process, three descriptors), and
each normal shell exit must reach zero processes and descriptors before the
session is closed. These counters include exited references awaiting reaping.
They are diagnostics, not a public process-enumeration privilege.

Host validation passes 55 shell tests (28 library and 27 binary), 178 dash
comparisons including 95 arithmetic and 51 substitution cases, 13 descriptor
tests, 11 PTY tests, 42 IPC unit tests (including three environment tests), nine
IPC integration tests, and six production-scope model tests. The standalone
allocator model passes nine tests. `tools/test-alloc-proof.sh` fails to compile
because the existing host/global-allocator configuration lacks `GLOBAL_ALLOC`;
the standalone model does not replace that missing full proof.

Coverage boundaries: reused PID/parent-incarnation authorization is exercised
by the production-code host model, and reused role identity/session generations
by broker tests. The guest checks stale scope IDs and repeated real session
slots, rather than forcing a numeric PID wrap. Spawn rejection covers cancelled
owners and missing executables; allocation failure during cancellation is not
fault-injected. After CLOSE_SESSION destroys the control role, the guest checks
controller/producer termination and reaper evidence, not an authorized query of
the destroyed session. The per-scenario descriptor baselines and repeated normal
exits do not claim a global physical-allocation leak proof.

System regression evidence uses `target/foreground-regression-*.log` and serial
logs. The scripting image (`target/foreground-shell-phase2.iso`, SHA-256
`fc17d47e9e02c799bda4403635272df40286e5820422d3474a67d55f6c4f0319`)
passes its build gate and one frozen-image boot on each of 1, 2 and 4 CPUs.
`phase_sec` passes capability/IPC security, 64 SMP terminal-state iterations,
late reply/deferred reply lifecycle and injected mmap/SHM allocation rollback.
`mm2b` passes the 12-CPU TLB shootdown and PMM accounting-baseline gate.
The native `shell-phase1.5` gate fails with `FAIL TTY shell registration` and
UserSession nameserver denial; this is a retained architectural blocker.

One execution mistake started an overlapping regression loop. Its swap and
Helios outcomes, including an I/O failure, are retained in
`target/foreground-regressions.status`; shared build/log artifacts make those
overlapping results inconclusive. The extra loop was stopped. A sequential
validation with distinct `target/foreground-clean-*.log` artifacts establishes
the affected gates below. This is correction of invalid test orchestration,
not an automatic retry of a valid intermittent guest failure.

The sequential final-source SWAP-1 gate passes at 4 CPUs/1536 MiB, including
multi-pool pressure, stale slot generations, exit cleanup and PMM/slot baselines.
The final-source Helios I/O and threading probes pass at 2 CPUs/1536 MiB:
`IO_PIPE PASS`, `IO_SOCKET PASS`, child-TID clearing and `THREAD_PROBE PASS`.
These regress ordinary Linux behavior; they do not verify POSIX SIGINT delivery
to a Helios task inside a native scope.
The final-source audio gate passes at 2 CPUs/1536 MiB with the `none` host
backend, Intel HDA DMA progress and completed test tone. The gate establishes
DMA/service progression, not audible output or a desktop-interaction benchmark.
After the final process/broker credential refinement, `phase_sec` and `mm2b`
were run sequentially again and pass. Their final artifacts are
`target/foreground-final-phase_sec.{log,serial.log}` and
`target/foreground-final-mm2b.{log,serial.log}`; the first uses 2 CPUs and the
second the gate's required 12 CPUs, both with 1536 MiB.

| Regression | Result and boundary |
| --- | --- |
| Host shell / dash | 55 tests / 178 comparisons PASS |
| Environment / descriptor | 3 wire tests / 13 descriptor tests PASS |
| Host IPC / PTY / scope model | 42 unit + 9 integration / 11 / 6 PASS |
| Native lifecycle | Final secure image: 1, 2, 4 CPUs PASS |
| Native scripting/arithmetic/PTy | Frozen scripting image: 1, 2, 4 CPUs PASS |
| Capability / IPC / allocation rollback | Final-source `phase_sec` PASS |
| Reclaim / TLB / accounting | Final-source 12-CPU `mm2b` PASS |
| Swap pressure | Final-source 4-CPU SWAP-1 PASS |
| Helios I/O / threading | Sequential final-source probes PASS |
| Audio DMA | Sequential final-source HDA test-tone gate PASS |
| Numbered native TTY | BLOCKED: UserSession discovery rejects shell endpoint |
| Full allocator proof | BLOCKED: host `GLOBAL_ALLOC` compilation configuration |
| Standalone allocator model | 9 tests PASS; does not replace full proof |
| 1 GiB combined image | Not investigated or declared fixed |

Recommended next focused phase: design broker-authenticated legacy TTY endpoint
delegation, add durable generation-qualified child wait records, and fault-inject
allocation failure and broker death during cancellation. Cooperative native
signal delivery and configurable VINTR/ISIG can follow those lifecycle contracts.
Audit legacy numeric signal authorization for unrelated unscoped processes as
separate security work. Full POSIX job control remains outside this design.

Desktop Ctrl+C event translation is source-reviewed and compiled. The native
gate injects authenticated ETX through the PTY master; it does not establish a
human keyboard/UI interaction result or audible audio output.

The initial frozen lifecycle matrix (`target/foreground-lifecycle-final-matrix/results.json`)
passes on 1, 2 and 4 CPUs. Its ISO is `target/foreground-lifecycle.iso`, SHA-256
`8e7e921ebb6346f440d692adcaf9b1c988643d1fe472ac5815cd027d5265bacc`.
This predates the final refinement of PTY credentials from VM generations to
per-process publication incarnations; the final secure matrix is recorded below.
Every boot verifies eight interrupt scenarios, 32 scope reuse/exit races,
eight normal session exits, producer and descendant termination, prompt/status
recovery, cross-session isolation and all pre-existing PTY/pipe liveness gates.
Each interrupt returns session counts to one shell process and three descriptor
references; normal exit and owned-scope cleanup return their counts to zero.

| CPUs | Lifecycle gate | Worst producer interrupt | Substitution disconnect |
| --- | --- | --- | --- |
| 1 | PASS | 40 ms | 0 ms |
| 2 | PASS | 140 ms | 110 ms |
| 4 | PASS | 120 ms | 130 ms |

Interrupt latency runs from the authenticated ETX request until the witnessed
producer is no longer live. Descendant death, direct-child status, prompt
recovery and descriptor reclamation have separate assertions. Scope-repeat
latency also includes wait and resource-quiescence checks. The guest monotonic
clock resolves 10 ms: zero means within one clock interval. These are observed
values, not scheduling guarantees. Across this matrix the worst interrupt is
140 ms, disconnect 130 ms, and scope wait/reclamation 120 ms.
Startup uses PID-file barriers; full-output
and delayed-input cases retain short bounded fill delays. The cancellation
deadline is the existing two seconds, and the boot deadline stays 300 seconds.

Development failures remain in `target/foreground-*.serial.log`: the syscall
argument-register mismatch, cancellation/exit arbitration, cancelled IPC being
mistaken for hangup, ordinary slave I/O incorrectly binding a controller, and
the interrupted capture-read assertion were corrected before the final image.
`foreground-delivery-2cpu.serial.log` exposed a test sampling resource counts
immediately after prompt delivery; the final test waits for scheduler reaping
within the unchanged two-second limit before starting another scenario.
The original matrix artifacts record MISSING_MARKERS because its harness
mistook the final expected-file entry for completion and stopped guests early.
The corrected harness waits for the actual successful shell exit marker; those
prematurely stopped runs are not counted as successful guest boots.

The **final secure lifecycle image** includes publication-qualified process
credentials and broker-qualified sessions. Its ISO is
`target/foreground-lifecycle-secure.iso`, SHA-256
`d1171fd1ca3280e4b61ac8e5a0168cbfce3bbe619e33ed5cf6b35f71a6f1a773`.
`target/foreground-lifecycle-secure-matrix/results.json` records one successful
boot each on 1, 2 and 4 CPUs at 1536 MiB, with every expected assertion present.
The final build gate also passes on 2 CPUs. Source formatting afterward changes
whitespace only.

| CPUs | Final gate | Worst producer interrupt | Substitution disconnect | Scope wait/reclaim maximum |
| --- | --- | --- | --- | --- |
| 1 | PASS (77.90 s) | 60 ms | 0 ms | 40 ms |
| 2 | PASS (50.09 s) | 60 ms | 0 ms | 40 ms |
| 4 | PASS (57.83 s) | 190 ms | 90 ms | 60 ms |

Across the final matrix, all 24 interrupt scenarios restore one shell process
and three descriptors, all 96 scope reuse checks finish at zero members and
descriptor references, and all 24 normal session exits reach zero before
closure. The same earlier measurement boundaries apply. The final observed
worst-case producer cancellation is 190 ms; it is not a universal bound.

Remaining extensions: secure legacy TTY endpoint handoff, configurable interrupt
discipline, cooperative native signal delivery, durable wait-status storage,
session disconnect notification when the broker itself dies, comprehensive OOM
rollback, and broader race fault injection. POSIX jobs, stop/continue and SSH
are outside this phase. The 1536 MiB combined-image baseline does not resolve
the separately documented 1 GiB image-capacity failures.
