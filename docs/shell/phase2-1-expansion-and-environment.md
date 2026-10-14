# Shell Phase 2.1: environment delivery boundary

Date: October 14, 2026. The current delivery adds Stage B native `$(...)`
command substitution to Stage A and the Phase 2.1A liveness/reclaim repairs.
See "Phase 2.1B" at the end for architecture, limits and current validation.
Arithmetic expansion, pathname expansion and compound pipeline stages remain
unsupported. Earlier sections retain their historical validation boundaries.

The October 14 independent audit below reproduces the retained pre-repair
image's intermittent output deadlock and identifies an additional memory-
pressure stall in the full Stage A pipeline. Allocation-driven reclaim now
shares a page and elapsed-work budget across the entire locked mapping.
See "Additional reclaim critical-section repair" and its final validation
for the current evidence; earlier successful matrices remain historical.

The checkout was clean at the audit. The reported Phase 2 results were rerun;
the earlier report was not used as proof of current runtime behavior.
The semantic reference is POSIX.1-2024 Shell Command Language, particularly
2.9.1.2 Variable Assignments, 2.5.3 Shell Variables, and the export/unset
special built-in descriptions:
`https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html`.

## Behavior and shared execution

Scripts, `sshl -c`, and native interactive simple commands support:

```sh
NAME=Sunlight
export NAME
sh -c 'echo "$NAME"'
NAME=temporary sh -c 'echo "$NAME"'
echo "$NAME"
unset NAME
sh -c 'echo "<$NAME>"'
```

Expected output is `Sunlight`, `temporary`, `Sunlight`, and `<>`, each on its
own line. Local unexported values are absent in external children. Temporary
prefixes restore the parent's value and attributes after launch, failure, or
function/builtin execution; unrelated function changes remain visible.
An exported name with no assigned value remains unset until assigned.
If restoring temporary names would exceed variable limits after a function
adds other values, the variable context rolls back to its pre-command snapshot
and reports status 2. This exceptional resource failure does not undo filesystem
or other external effects already performed by the function.

Assignments preceding assignment-only commands and special built-ins persist
before the built-in runs. Existing export attributes are preserved, but these
prefixes do not automatically acquire export attributes. POSIX leaves this
attribute choice unspecified for special built-ins when `set -a` is off.
`set -a` remains unsupported. Readonly assignment errors are diagnosed before
launch. Existing special-builtin/script error behavior is otherwise unchanged.

Simple pipeline stages receive separate environment snapshots and independent
prefixes, including native interactive pipelines:

```sh
NAME=parent
export NAME
NAME=left sh -c 'echo "$NAME"' |
    NAME=right sh -c 'echo "$NAME"; cat'
echo "$NAME"
```

This produces `right`, `left`, and `parent`. Argument expansion precedes prefix
assignment; prefix RHS values expand sequentially. Redirection operands then
expand against the command's assignment environment. All pipeline environments
are checked before opening pipes or starting producers. Scripting special
built-ins/functions and compound pipeline stages remain deferred in script
mode, as documented in the Phase 2 compatibility matrix.

`script::prepare_arguments` / `prepare_pipeline_stage` are shared with the
native interactive dispatcher. Interactive execution remains asynchronous:
it launches the child, restores temporary names, and lets the existing terminal
loop drain output and report completion. The language is not delegated to a
host shell. Host external processes use `Command::env_clear/envs`, with the
same validation limits as native execution.
The host development REPL retains its earlier line dispatcher; environment
semantics are validated through its shared script-file/`-c` interpreter. Native
interactive simple commands and pipelines use the shared preparation helpers.

## Native transport and security

The audit found a PCB `EnvMap`, normal spawn inheritance, initial SysV envp
stack/register marshalling, `sunlight_libc::env::getenv/getenv_bytes`, and native
std's indexed environment accessor. Native script startup had an explicit
changed-export rejection. Interactive startup used defaults instead of the
complete process snapshot, and its export builtin did not mark attributes.

Spawn syscall 39 now accepts a second optional extension marker in r9:

| r9 | rsi | Environment behavior |
| --- | --- | --- |
| Legacy / unrecognized marker | Existing argv pointer array | Inherit parent PCB environment; extra stdio registers ignored |
| Existing `STDIO_EXTENSION` | Existing argv pointer array | Inherit environment; honor stdin/stdout/stderr mappings |
| New `ENV_EXTENSION` | Pointer to 24-byte `SpawnEnvironment` | Replace child environment; honor the same stdio mappings |

`SpawnEnvironment` contains three little-endian u64 fields: argv pointer,
environment blob pointer, and blob length. The blob is a sequence of UTF-8
`NAME=VALUE\0` entries. Zero length explicitly means an empty environment;
it does not mean inheritance. This extension requires the updated kernel and
libc together; existing callers need no migration. Direct guest tests exercise
legacy unrecognized registers, legacy stdio inheritance, and the new extension.
The Helios/Linux syscall ABI and compatibility implementation are unchanged.

The kernel copies the header/blob through existing checked user-memory APIs
and validates them before scheduler/PMM locks, ELF loading, or child creation.
The child receives a new PCB map and matching initial envp. No shell export
updates the parent PCB, a global environment, or the libc startup envp pointer.
Child script startup imports the supplied snapshot without recreating unset
HOME/PATH defaults. Pipeline builtin children also import exactly that snapshot.
Interactive login retains session defaults and overlays inherited exports.

Credentials, capability tables, lookup restrictions, and trusted-binary identity
follow the existing spawn path. Environment values are not used to populate
those fields. Guest probes pass forged UID/GID/CAPABILITIES/ROLE values and
compare actual credentials and numbered-shell lookup permission with the
parent. Raw malformed requests test the kernel validator independently of libc.

The shell's canonical GUI launcher gains an optional environment argument;
registered application and Chronos resolution/display authorization remain in
the existing launcher. Its original entry point retains inheritance. This path
has compilation and host validation evidence, but no GUI export/window runtime
claim is made. Internal z-history children use the shell snapshot as well.

The libc APIs here expose the initial process snapshot. This work does not add
C `environ`, C `setenv/unsetenv`, or a general kernel environment-mutation API.
The separate native exec syscall and future `exec` builtin need their own
PCB/envp consistency audit before process replacement is claimed.

## Limits and deviations

| Resource | Limit / failure behavior |
| --- | --- |
| Transport entries | 128, rejected before spawn when exceeded |
| Transport name | Portable ASCII shell identifier, at most 255 bytes |
| Transport entry | 4096 bytes including `=`, value and terminating NUL |
| Transport total | 16384 bytes including all NULs |
| Duplicate names | Reject the entire request |
| Invalid entries | Reject missing `=`, empty/invalid names, invalid UTF-8, empty records, or missing NUL terminators |
| Shell values | 256 stored names, 16 KiB per value, 64 KiB aggregate including names/separators |
| Export attributes | 256 names, including exported unset names |
| Pipeline | 16 stages, at most 15 pipes; all stage environments validated first |

Empty values, equals signs, embedded newlines, and Unicode **values** are valid.
Values cannot contain NUL. Limits are measured in bytes. Names outside the
portable ASCII identifier set and non-UTF-8 environments are deliberate native
transport deviations from a general POSIX process environment. Inherited
shell-invalid names are not imported into native shell variables.
Special-variable startup is still incomplete: the shell imports an inherited
IFS instead of resetting it to space/tab/newline as POSIX requires. The existing
readonly builtin also creates an empty value when marking an unset name.
These remain explicit follow-up items alongside the builtin completion stage;
this delivery does not claim complete POSIX shell initialization.

Oversized/invalid environments fail without truncation. Raw kernel errors use
the existing invalid-argument/range sentinels; libc returns `Inval`/`TooBig`.
Shared runtime validation reports status 2. After a limit error an interactive
shell/context can unset or shorten values and run another command. Native
argv limits remain 15 entries, a 1024-byte aggregate arena, and the kernel's
per-argument C-string bound; environment capacity does not enlarge argv.
Native `env` validates the snapshot before formatting it into its 16 KiB output
buffer, so an oversized environment produces a diagnostic rather than clipped
output.

Serialization and the kernel's byte-buffer copy use fallible reservation.
Existing BTreeMap/BTreeSet nodes, String/context/AST clones and several parser
allocations are still infallible. This is bounded environment transport, not
complete OOM recovery. Existing parser/source/call bounds remain in force;
substitution/glob resource policies await their implementation stages.

## Stage A validation before the Phase 2.1A liveness repair

Current commands (the native gates rebuild their images):

```sh
cargo test -p sunshell --no-default-features --features std --target x86_64-unknown-linux-gnu
cargo build -p sunshell --no-default-features --features std --target x86_64-unknown-linux-gnu
SSHL_REFERENCE_SHELL=/tmp/sshl-reference-dash python3 tools/test_shell_phase2_host.py
cargo test -p sunlight-ipc --target x86_64-unknown-linux-gnu spawn_env
cargo test -p sunlight-libc --target x86_64-unknown-linux-gnu sun_exec::tests --lib
cargo test --manifest-path tools/tests/fd-description/Cargo.toml --target x86_64-unknown-linux-gnu --target-dir target/fd-description --offline
cargo check -p sunlight-kernel
SUNLIGHT_TEST_CPUS=1 SUNLIGHT_TEST_SERIAL_LOG=target/shell-stage-a-smp1-final.log bash tools/test.sh shell-phase2
SUNLIGHT_TEST_CPUS=2 SUNLIGHT_TEST_SERIAL_LOG=target/shell-stage-a-smp2-final.log bash tools/test.sh shell-phase2
SUNLIGHT_TEST_CPUS=4 SUNLIGHT_TEST_SERIAL_LOG=target/shell-stage-a-smp4-final.log bash tools/test.sh shell-phase2
SUNLIGHT_TEST_CPUS=2 SUNLIGHT_TEST_SERIAL_LOG=target/shell-stage-a-phase1_5.log bash tools/test.sh shell-phase1.5
SUNLIGHT_TEST_CPUS=2 SUNLIGHT_TEST_SERIAL_LOG=target/shell-stage-a-helios.log bash tools/test.sh helios-note-regression
```

The guest suite checks exact output/status, exported and local values,
temporary/special assignments, pipeline stage isolation, unset/default behavior,
readonly errors, empty/maximal environments, malformed/duplicate/raw-pointer
requests, unchanged identity/lookup restrictions, and legacy spawn behavior.
Existing repeated processes, 100 interpreter redirection scopes, 100 PTY
pipeline repetitions, 1 MiB streams, failed producer cleanup, and 64 KiB live
output remain part of the full gate. Heap/descriptor comparisons retain their
original driver-vs-remote-shell validation boundary.

The first two two-CPU full gates passed the environment and PTY export checks
but timed out at the 64 KiB live output probe. The producer completed its first
write and stayed alive; the driver received only the 22-byte app-start marker.
Those failures remain evidence, rather than being hidden by later retries.
One- and four-CPU full gates also passed during development. A bounded telemetry
snapshot on prompt timeout records process states/runtime when available; the
test does not extend its timeout or weaken any capability checks.

Verification on October 14, 2026:

| Check | Result |
| --- | --- |
| Shell host tests | 43 passed: 23 library + 20 binary |
| Differential suite | 32 portable cases matched dash output/status; entry-point and unsupported-syntax checks also passed |
| Environment wire validator | 3 passed |
| Canonical launcher tests | 9 passed |
| Independent descriptor harness | 13 passed; inert pipe/eventfd adapters do not establish kernel I/O behavior |
| Kernel check / native compilation | Passed; final QEMU images rebuilt the kernel and native shell |
| One-CPU final full scripting gate | Passed, including environment security/boundary probes and PTY 64 KiB live output |
| Four-CPU final full scripting gate | Passed, including independent pipeline environments, repeated execution and PTY 64 KiB live output |
| Two-CPU scripting gates | Environment, script repetition and PTY export checks passed; full gate failed on PTY 64 KiB output timeout |
| Phase 1.5 TTY recheck, two CPUs | Failed: repeated nameserver lookup denials and `sshl6` registration timeout |
| Helios Note runtime, two CPUs | Passed Linux ELF loading, geometry, terminal initialization and interactive-ready markers |

The two-CPU timeout telemetry sampled `pty_server` and the relay shell as
BlockedOnIpc (state 2), with the producer and test driver Running (state 1).
The producer had completed its first write but not all writes; the relay's
first pull returned zero. This narrows the failure to PTY/IPC/backpressure
liveness. It does not establish a scheduler root cause, nor prove that the
failure predates this change. The earlier reported four-CPU retry cannot be
called harmless from the current evidence. Resolve the liveness issue before
claiming a reliable SMP gate or adding substitution capture to this path.
The last two-CPU attempt preceded the final restoration-limit guard; the final
one- and four-CPU passes include that guard. No passing full two-CPU result is
claimed for either revision.

The TTY registration blocker remains an authorization failure; capability
profiles were not broadened. `tools/test.sh` now stops Phase 1.5 on its explicit
FAIL marker, preserving failed status and missing-marker diagnostics.
The Helios gate establishes runtime startup, not interactive editing, clean
exit, signal handling, or a comprehensive Linux compatibility regression.

An unfiltered libc host run hung in the two secret-store tests
`default_options_are_private_and_bounded` and
`path_policy_rejects_traversal_and_staging_names`. Running the remainder with
`--test-threads=1` and those two names skipped passed 82 tests. A parallel run
of that remainder failed two allocator assertions; both passed in the serial
run. These results do not establish an unfiltered libc-suite pass.

Logs are under ignored `target/shell-stage-a-*.log`; no generated images or
serial artifacts are included in the source changes.

After the Phase 2.1A liveness validation below, the next implementation slice
is Stage B command substitution: structured
nested parsing, isolated execution contexts, stdout-only capture, trailing-LF
removal, quote/IFS behavior, child status/descriptor cleanup, and a bounded
collector that drains concurrently or terminates/reaps producers on overflow.
Arithmetic, filesystem globbing, compound pipelines, and remaining builtin
modes follow in the requested order. No SSH integration is included. SSH still
needs authenticated session creation, cwd/env/stdin/stdout/stderr handoff,
PTY authority/winsize and foreground forwarding, signal/disconnect cancellation,
child reaping, and backpressure tests.

## Phase 2.1A: PTY output liveness and SMP regression

Committed repair status: the repaired image passes three complete boots each with 1, 2
and 4 CPUs, including exact 1 KiB–1 MiB content, slow/partial transfers,
concurrency and closure. Stage A passes all three CPU configurations; relevant
Helios and audio DMA gates pass. The `sshl6` lookup authorization blocker is
documented separately and no security policy is weakened. Command substitution
remains unimplemented. Reproduction and all intermediate failures follow,
with final results and limitations at the end of this section.

The independent revalidation at the end of this document records an additional
four-CPU full Stage A pipeline IPC failure, its reclaim critical-section cause,
and the allocation-budget correction. The earlier focused PTY size matrix
alone did not clear that broader failure.

The October 14, 2026 investigation reproduced the failure without changing
the 15-second guest deadline, the 2-second PTY IPC deadline, ring capacities,
or CPU count. The earlier Stage A results above are historical evidence.
Command substitution is not implemented by this repair.

### Reproduction and blocked actors

Three fresh two-CPU full Stage A gates passed before the repair. That did
not clear the earlier failures: a focused image, retaining the broken wait
implementation, subsequently failed all three recorded repeated boots.
One failed at 256 KiB, and two failed on the original 64 KiB `emit` command.
Both 64 KiB failures delivered exactly the 22-byte app-start OSC marker and
no payload. A separate initial focused gate failed at 16 KiB.

The 64 KiB timeout snapshot identifies these single-threaded native actors:

```text
pid=36 name=sshl7 state=BlockedOnIpc owner=255 queued=255
tab=Some(7) rings=0/8192 wait_child=Some(37)
pending=None outcome=None target=None reply_wait=None
pid=37 name=sshl state=Running owner=1 tab=Some(7) rings=0/8192
pty endpoint depth=0 callers=0; pty_server waiting for another request
[SSHL-TEST] FAIL pty prompt timeout, bytes=22
```

The producer completed its first write and filled the bounded TTY stdout
ring. The relay had consumed its last IPC reply and then parked on that
producer's exit. The master test driver continued receiving empty-live-read
`ERR_WOULD_BLOCK` replies. The PTY server was servicing requests; there was no
orphaned queued request or missing reply required to explain the stall.
The producer and driver occupied the two CPUs while the relay was blocked.
This is a circular child-exit/output-drain wait, not proof of unfair SMP
dispatch or an IPC lost wakeup. Timing determines whether the relay happens
to drain enough data before it is descheduled, explaining occasional passes.

Evidence files, all ignored build artifacts:

- `target/pty-liveness-baseline-2cpu.log`,
  `target/pty-liveness-diag-2cpu.log`, and
  `target/pty-liveness-baseline-repeat/shell-phase2-cpu2-run1.serial.log`
  record the three full pre-repair passes.
- `target/pty-liveness-focused-pre-2cpu.log` records the 16 KiB stall.
- `target/pty-liveness-confirm-pre-2cpu.log` records the child-wait snapshot
  at 256 KiB.
- `target/pty-liveness-pre-fix-repeats/` contains every repeated failure,
  its bounded host deadline and elapsed time, and the pre-repair ISO digest
  `803e73209bb91c0a068b7b6b89428eb59feb7b841b50448335821ef9106f4446`.
  Runs 2 and 3 are the exact 64 KiB reproductions. The expected-marker file
  was extended while this initial harness was running; its missing-marker
  lists therefore differ. All three failures contain explicit guest timeout
  markers, independently of those lists. The final harness snapshots its
  expected markers once before running the matrix.

### Root cause and repair

`start_pty_shell` drains `tty_stdout_pull`, relays accepted bytes through
`WRITE_SLAVE_BULK`, polls PTY input, and calls libc `try_waitpid` to check
foreground completion. Both libc and IPC `try_waitpid` previously called
native syscall 32 in its legacy mode. Despite the helpers' documented
nonblocking contract, `sys_waitpid` installs `wait_child`, marks the caller
`BlockedOnIpc`, and removes it from ready queues whenever the child is live.
Only child exit normally wakes that wait. A child with more output than the
TTY ring cannot exit when its only drain is parked there.

Native WaitPid now recognizes one explicit rsi marker,
`WAITPID_NOHANG_EXTENSION = 0x53554e574e4f4847`. With that exact marker, a
live-child check returns `EAGAIN` without installing a child wait, changing
process state, or removing ready-queue membership. Zero and unrecognized rsi
values keep the existing parked wait, including old one-argument binaries
whose unused registers are unspecified. Unknown/non-child PIDs still fail;
completed children still return the native exit code.

Both public `try_waitpid` helpers send the marker. Both blocking `waitpid`
helpers explicitly use the legacy mode and yield only while that kernel wait
is parked. This does not replace synchronous child waits with polling. The
kernel and userspace helpers must be upgraded together. The Linux wait4 ABI
and its existing explicit unsupported result are unchanged.

The graphical terminal also uses libc `try_waitpid` in `refresh_status`; it
receives the same corrected nonblocking semantics. PTY protocol labels,
shared-page limits, accepted-byte handling, service authorization, descriptor
ownership, and scheduler policy are unchanged by this waitpid repair.

The follow-up matrix exposed heavy retry traffic on full TTY rings: blocking
TTY descriptors returned `EAGAIN`, and producers could issue over 145000
write/yield retries during one slow 1 MiB transfer. Blocking TTY writes now
park using the same syscall rewind/retry mechanism as native pipes. The wait
records the tab in `tty_write_wait`; a drain wakes matching writers to recheck
their descriptor and available space. Accepted partial writes still return
their actual count immediately. `O_NONBLOCK` and explicit nonblocking writes
retain `EAGAIN`. This applies to native and Linux TTY descriptors through the
shared write path; it does not add a shell- or application-specific exception.
No TTY or broker ring capacity changes.

The relay's broker backpressure path also previously retried
`WRITE_SLAVE_BULK`/`ERR_WOULD_BLOCK` continuously while a slow master slept.
`WAIT_SLAVE_WRITABLE` now reuses the existing generation-bound deferred IPC
reply mechanism. The broker authorizes the slave identity and retains at most
one readiness waiter per session (16 total), without mapping or retaining a
payload page. It completes that waiter after a drain creates space, on endpoint
closure, or when the call is stale/cancelled. The relay then retries the original
unaccepted bytes. Nonblocking transfer requests keep their existing replies.
Broker mutation is serialized in its service loop, so readiness checks,
deferral, draining, and closure cannot pass one another. Existing kernel IPC
generation/terminal-state rules reject late replies; no capability check is
bypassed. Kernel, broker and shell must be upgraded together for the new wait.

A PTY input/output terminal error now ends the relay with status 129 and sends
SIGHUP to its tracked foreground children through the existing signal path.
It does not signal other session processes. This prevents a foreground writer
parked on TTY space from surviving a master disconnect indefinitely.
The first full hangup test (`target/pty-liveness-reordered-first-4cpu.log`)
found that syscall-return signal delivery considered only INT, TERM and CHLD.
The writer was woken by HUP, re-entered its blocking write, and parked again;
timer delivery skipped that non-Running state. Syscall-return delivery now
includes HUP with the same default/ignored/blocked disposition handling.
No signal permission check or capability policy is changed by this repair.

Source review also found that graphical `TerminalTab::close` sent SIGTERM
before `CLOSE_SESSION`. Default signal delivery could terminate the relay
before it forwarded hangup to its foreground children. Successful session
closure now lets the live relay observe the terminal error, send foreground
HUP and exit. The existing bounded SIGTERM fallback remains for a failed
close request or a tab without a session. This adds no GUI wait loop. Native
tests cover both `CLOSE_MASTER` and the graphical client's `CLOSE_SESSION`
protocol, including the relay's 129 exit and foreground-child termination.
Interactive GUI click/latency and abrupt client death are separate validation
boundaries; broker-unavailable fallback/orphan recovery is not established.

### Locks, wakeups, transfers, and lifetime

- Child existence/ownership, completion checks, and blocking-wait registration
  remain under `SCHEDULER`. Child exit uses the same lock to observe and wake
  the registered parent. No exit wake can pass between checking the live
  child and installing a blocking wait. A nonblocking probe registers no wait.
- `sys_waitpid` releases `SCHEDULER` before requesting rescheduling. IPC
  state transitions are serialized under `SCHEDULER`, then capability/endpoint
  locks. Reply identity stays bound to caller PID and IPC generation.
- Native pipe operations and readiness/wait registration run under
  `SCHEDULER`, with short pipe-pool critical sections. Reads, writes and final
  endpoint closure wake condition-qualified pipe waiters. Blocking syscall
  retries preserve their original syscall number and recheck the condition.
- TTY writes take `SCHEDULER` then the stdout ring lock. `TtyStdoutPull`
  releases the stdout ring lock, acquires scheduler state to wake writers,
  then copies to userspace. No ring lock crosses an IPC call or a parked wait.
- A full blocking TTY write registers its wait under the same `SCHEDULER`
  guard as the failed ring write. A concurrent drain either precedes the ring
  check (so the write accepts bytes), or acquires `SCHEDULER` after the wait
  is installed and wakes it. A drain notifies even if its subsequent user copy
  fails, because ring space has already been released. Tab clearing also
  notifies after releasing both ring guards. Wakeups requeue a detached task
  once; a task still owned by a CPU is redispatched by that owning CPU.
  Signal, timer and I/O wake paths clear the TTY wait record before making
  the task ready, so a later unrelated wait cannot inherit that subscription.
- The PTY broker processes one request at a time against fixed 8 KiB input
  and output rings. Bulk transfers are at most 4096 bytes. A full ring replies
  `ERR_WOULD_BLOCK`; partial transfers report their actual accepted count.
  The relay retains unaccepted bytes until accepted or a terminal error.
  On full output the relay parks in a deferred readiness call instead of
  repeatedly requesting transfers. The master remains able to drain or close
  the session while that reply is deferred.
- Pipe descriptor references remain attached to shared open descriptions.
  Final writer closure permits EOF after queued bytes drain; final reader
  closure wakes blocked writers with `BrokenPipe`. Broker slave closure
  likewise drains queued output before master EOF (`ERR_PEER_CLOSED` on
  the wire, mapped to a zero-byte read by terminal clients), while master closure
  rejects further slave writes with `ERR_PEER_CLOSED`.

Diagnostics print one bounded snapshot at failure or between completed test
scenarios. `SUNLIGHT_PTY_TRACE=1` additionally records a 32-entry ring of
numbered-shell IPC request, reply, and wake events (PID, generation, endpoint,
label, byte count, queue depth, and owning CPU). It does not print in the IPC
hot path. The driver and foreground shells are traced; eight separate retained
records capture startup syscalls exceeding 50 ms, including lock wait/hold
times, free frames, and mmap reclaim counter deltas. Ready-queue membership,
last dispatch age, priority and TTY write waits are included in snapshots.
The trace mutex is released before snapshot code acquires endpoint
or TTY locks. Normal images do not record these events. Notification counters are also
restricted to traced builds. Producer tests report
partial-transfer and EAGAIN counts once per completed workload.

### Independent sshl6 authorization investigation

The old `TTY shell registration` failure message describes a failed lookup,
not a demonstrated registration rejection. The denied request is
`InitMsg::LOOKUP` for `sshl6`, FNV hash `0x0a994a83b88a6d83`, from the driver
with service lookup mask `0x40000` (`ServiceCapability::UserSession`).
`mediate_nameserver_lookup` calls
`service_capability_allows_hashed_name`; `matches_user_session_service`
intentionally excludes numbered shell endpoints. A process owning its own
endpoint and registering its own `sshl6` identity satisfies the separate
registration ownership/identity check.

The 2-CPU authorization recheck in
`target/pty-liveness-authorization-2cpu.log` confirms
`[SSHL-AUTH] pid=36 name=sshl6 registered=true alias=true`, followed by the
driver's denied lookups with the hash and mask above. The legacy Phase 1.5
gate remains a reported failure at its five-second lookup deadline.
The test's denied-lookup retries now use a timed private futex park, with the
same five-second deadline, rather than flooding serial output with denials.
No capability profile or identity check is broadened. To run this legacy TTY
harness successfully requires an explicitly authorized endpoint handoff or a
separate reviewed policy design. PTY tests use existing broker-issued session
authority and need no numbered-shell nameserver lookup.

### Regression procedure and validation boundaries

`tools/test.sh shell-pty-liveness` builds the real kernel/services/image and
injects `sshl --pty-liveness-tests`. Its stream checker validates bytes as
they arrive, retaining one shared page and at most 192 bytes of OSC metadata.
The six sizes are 1024, 4096, 16384, 65536, 262144 and 1048576 bytes. Each boot
runs each size twice: 4096-byte producer/read chunks, then 997-byte producer
chunks and 257-byte reads with 10 ms timer parks after every eight reads.
It also runs the original 64 KiB probe, concurrent PTY sessions with a delayed
consumer, both public nonblocking-wait helpers with a pipe-blocked child,
two producers sharing a pipe with a slow consumer, early reader closure/
EPIPE, legacy blocking waits, and broker partial/full/close/EOF cases. A separate
64 KiB transfer delays draining until the producer parks on TTY space; another
uses `fcntl(F_SETFL, O_NONBLOCK)` to exercise partial transfers and EAGAIN.
An additional foreground-close scenario closes the master during a 1 MiB
producer and verifies both relay and foreground child exit within two seconds.
The corresponding whole-session closure repeats that check with the graphical
client's control-authorized operation. Teardown verifies child wait/status and
the close reply; transport timeouts or rejected replies fail explicitly.
Concurrent PTY sessions use disjoint payloads (A–W pattern and constant Z) so
cross-session mixing is detected as well as missing or reordered bytes.

Per-stream guest deadlines remain 15 seconds and per-call deadlines remain
two seconds. The host boot deadline remains 300 seconds, including scenarios
whose blocking syscalls cannot report their own local deadline. Every timed
out or failed run remains a failed result. No retry is silently substituted.

```sh
SUNLIGHT_TEST_CPUS=2 SUNLIGHT_PTY_TRACE=1 \
  SUNLIGHT_TEST_SERIAL_LOG=target/pty-liveness-fixed-final-2cpu.log \
  bash tools/test.sh shell-pty-liveness
cp target/sunlightos.iso target/pty-liveness-fixed.iso
python3 tools/test_pty_liveness.py --iso target/pty-liveness-fixed.iso \
  --cpus 1 2 4 --runs 3 --output target/pty-liveness-fixed-matrix
```

The matrix writes separate serial/QEMU logs and `results.json` with the image
digest, CPU count, repetition, elapsed time, deadline, and all missing markers.
With `--qmp-snapshot`, a failed guest is stopped after failure/deadline and its
CPU registers and stack words are saved separately. Socket creation requires
execution permission; launch failures are recorded as `QEMU_EXIT`, not passes.
The result also retains up to 32 stream summaries with exact verified byte
counts, read sizes, slow-consumer mode, read counts, and sleep/IPC timings.
Source changes, generated images and diagnostic artifacts are kept separate.
Host tests cannot establish SMP liveness; the native matrix provides that
evidence. Audio device/DMA tests do not establish human-audible playback, and
Helios startup does not establish interactive editing or clean exit.

The first repaired 2-CPU gate passed every stream, concurrency, and pipe case
but failed a newly added broker EOF assertion. That assertion incorrectly
expected a successful zero-byte wire reply. Existing broker tests and the
terminal client establish `ERR_PEER_CLOSED` after draining queued output.
The assertion was corrected to that protocol, without changing the broker.
`target/pty-liveness-fixed-first-2cpu.log` retains this harness failure;
`target/pty-liveness-fixed-final-2cpu.log` records the corrected passing gate.

The initial repaired matrix is retained in
`target/pty-liveness-fixed-matrix/results.json`. Its three 1-CPU and three
2-CPU boots passed. At 4 CPUs, run 1 passed, run 2 hit a two-second IPC deadline
at the concurrent-session boundary, and run 3 reached 991731/1048576 bytes in
the slow-consumer case before its 15-second deadline. Run 3's snapshot shows
`wait_child=None`, runnable relay and producer, and ongoing broker replies;
the original child-wait deadlock was absent. These 4-CPU boots overlapped a
separate QEMU regression on this TCG-only host. They remain failures, not
replaced results or evidence that host contention caused the IPC timeout.
IPC failures now take the same bounded snapshot as stream deadlines. Further
4-CPU runs use one guest at a time and retain the same deadlines and workload.

The isolated nonblocking-wait-only recheck in
`target/pty-liveness-isolated-4cpu/results.json` passed runs 1 and 3, but run 2
still timed out in IPC at 492 payload bytes. Its broker was Ready with one
queue entry on CPU 0, and one relay request remained pending for 423 ticks;
`wait_child` was absent. Host overlap therefore does not explain that failure.
The retained-startup recheck in `target/pty-liveness-startup-repeats/results.json`
passed four of five boots; run 4 reached 901487 bytes before the slow-stream
deadline. These results preceded blocking TTY parking and remain recorded.

`target/pty-liveness-tty-park-first-4cpu.log` records the first gate with both
repairs. The delayed-drain snapshot shows a producer blocked with
`tty_write_wait=7`, and every blocking pattern producer reports `eagain=0`.
The nonblocking producer reports 13 partial transfers and 10484 EAGAIN results,
then delivers the exact 65536 bytes. Every scenario passes without changing
deadlines. The startup record also isolates a 415 ms mmap critical section:
free frames change from 1750 to 2007 while reclaim scans/reclaims 2048 pages
over eight activations. Reclaim and long startup critical sections remain
latency considerations; this work does not rewrite memory ownership or reclaim
policy and does not claim that one passing trace proves their worst-case bound.

The TTY-parking-only matrix in `target/pty-liveness-tty-park-matrix/results.json`
passed all three 1-CPU and all three 2-CPU boots. At 4 CPUs runs 2 and 3 passed;
run 1 reached 727567 bytes before the slow 1 MiB deadline. The producer was
parked on TTY space, while the relay continued broker-full retries. This
additional failure is retained and motivated the deferred broker readiness
wait; TTY parking alone is not reported as completing the SMP regression.

`target/pty-liveness-timing-4cpu.log` preserves the throughput diagnosis before
the HUP correction: the failed slow stream delivered 691555 bytes over 2692
reads. Of its 15-second budget, 12160 ms were IPC and 2840 ms were deliberate
timer sleeps; the longest read IPC took 30 ms. This run made continuous
forward progress and is still a failed deadline result. No timeout was enlarged
and no reduced workload replaced this case.

`target/pty-liveness-hangup-fixed-4cpu.log` records the first passing gate with
the waitpid, TTY, readiness and HUP repairs. It includes `[SIG] 1 delivered: terminating process` and the full
foreground-close pass. The slow 1 MiB transfer validates every byte in 8670 ms
(3240 ms sleeping, 4940 ms IPC, maximum read IPC 30 ms), with the original
257-byte reads and 15-second deadline. Closure/concurrency checks now precede
the size loop so a later throughput timeout cannot suppress their evidence.
The first complete three-repeat matrix used that copied image and unchanged workloads.

### Host validation

| Check | Result |
| --- | --- |
| Shell host tests | 43 passed (23 library, 20 binary) |
| Dash comparisons and entry-point/unsupported checks | 32 comparisons passed |
| Descriptor harness | 13 passed; inert host adapters do not prove guest I/O |
| IPC crate | 42 library and 9 integration tests passed |
| PTY broker | 9 passed, including readiness authorization/generation/closure |
| Graphical terminal host tests | 28 passed |
| TTY library host tests (`sunlight-tty`) | 37 passed |
| libc partial-write helper tests | 2 passed |
| Canonical launcher host tests | 9 passed |
| Harness syntax and source whitespace | `bash -n`, Python compilation and `git diff --check` passed |

Logs use the `target/pty-liveness-` prefix. Host results establish pure logic
and compilation only. The earlier Stage A libc-suite qualifications remain;
an unfiltered libc suite is not claimed here.

### Complete-suite matrix and teardown follow-up

`target/pty-liveness-final-matrix/results.json` retains the first full matrix
with the child-wait, TTY parking, deferred readiness and HUP repairs:

| CPUs | Complete boots | Recorded outcome |
| --- | --- | --- |
| 1 | 3/3 | PASS, 51.83 / 58.59 / 57.10 seconds |
| 2 | 2/3 | PASS, 55.34 / 55.59 seconds; run 3 TIMEOUT at 300.14 seconds |
| 4 | 3/3 | PASS, 60.09 / 68.85 / 60.60 seconds |

Every complete boot checks both repeats of every requested size, exact content,
slow/partial transfers, concurrent producers, EOF and early closure. The failed
2-CPU boot passes the original 64 KiB case, blocking/nonblocking 64 KiB cases,
and both concurrent 256 KiB streams. It then stops during `PtyRunner` teardown
after externally killing `sshl8` PID 40, before the concurrency completion
marker. The kill marks PID 40 Finished with status 137. Subsequent NICED IPC
traffic continues, but neither cleanup nor the driver resumes in this log.
At that stage this was a distinct unresolved teardown failure; that boot is
not a complete gate pass or proof that killed-process cleanup was correct.

The next diagnostic image adds bounded kill/wait/session-close markers and a
single kernel watchdog snapshot if the test driver remains blocked or Ready
without dispatch for 500 ticks. The watchdog is restricted to the traced PTY
test image and retains the same deadlines. Snapshots include per-CPU current
PIDs, parent IDs, pending cleanup and IPC states, queues and ring occupancy.
No failed boot is replaced or silently retried. Command substitution was kept deferred during this teardown investigation.

The independent full Stage A gate passes on 2 CPUs in
`target/pty-liveness-final-shell-phase2-cpu2.serial.log`. Copied-image 1/4-CPU
launches in `target/pty-liveness-final-stage-a-matrix/results.json` initially
fail before boot because the sandbox rejects QMP socket creation. These are
harness launch failures, with the exact error retained in `.qemu.log` files.
Separate authorized rechecks preserve those original outcomes.

The independent 2-CPU Helios I/O, thread and Note startup gates pass. Audio and
the teardown follow-up evidence are recorded below.

The teardown diagnostic matrix reproduces the timeout in its first 2-CPU
boot (`target/pty-liveness-teardown-diagnostic-matrix/`); runs 2 and 3 pass.
The last teardown marker is `kill returned`. Its snapshot proves resurrection:
PID 40 was marked Finished/status 137 by SIGKILL, then became Running on CPU 0
and continued tens of thousands of `READ_SLAVE` requests. PID 35 was parked
with `wait_child=Some(40)`. Broker queues/rings were empty and IPC replies and
wakeups continued. This identifies the lifecycle race independently of output
backpressure or host harness deadlines.

An external kill can finish a task while it enters a syscall on another CPU.
`handle_ipc_call` then unconditionally assigned `BlockedOnIpc`, overwriting
Finished; the next reply made it Ready and runnable again. The repair rejects
new call/receive operations from exiting tasks, rejects late replies to them,
and makes the shared scheduler state setter preserve terminal state. Syscall
return checks the sticky cleanup/terminal state under its existing scheduler
guard and pivots to the existing process-exit path with the recorded status.
It takes no extra scheduler lock and keeps resource reclamation deferred until
the owning CPU safely deschedules. The boot termination self-test now models
kill-before-IPC-entry, receive, generic blocked-state assignment and late reply;
each must leave the victim Finished and unqueued with its original exit code.
Capability validation and authorization policies remain intact.

The first watchdog image also reported newly spawned probe children whose
`last_run_tick` was still zero. This diagnostic false positive did not fail or
alter a gate result. Driver identification now uses its top-level shell parent
instead of treating every `sshl` on tab 0 as the test driver. QMP snapshots
provide an independent failure-only register capture.

The audio gate passes PCI binding, stream initialization, capture registration,
test-tone DMA progress and completion on 2 CPUs. It does not establish audible
playback or a latency bound under concurrent PTY load.

### Idle-CPU dispatch follow-up

The lifecycle repair matrix in
`target/pty-liveness-exit-guard-matrix/results.json` passed all three 1-CPU
and all three 2-CPU boots. At 4 CPUs runs 1 and 3 passed; run 2 reached
817260/1048576 bytes before the slow-stream deadline. No killed task resumed,
and the closure, concurrent-session and pipe cases completed before that
failure. This matrix is 8/9 complete passes, not a complete SMP result.

The failed slow stream performed 3181 reads. IPC consumed 12240 ms, deliberate
consumer sleeps 2700 ms, and the longest read IPC was 30 ms. Its snapshot
showed the relay Ready with one queue entry on CPU 2 while CPUs 1, 2 and 3
had no current task; the producer was parked on its full 8192-byte TTY ring.
The broker still serviced requests. There was no child wait or missing IPC
reply. The configured RoundRobin queue insertion set a reschedule flag but
sent no notification to the remote idle CPU, leaving dispatch to a periodic
tick. The BORE version of that helper also omitted the reschedule flag.
Repeated dispatch delays consumed the unchanged 15-second budget. Runtime
results below exercise the configured RoundRobin mode; BORE is not boot-tested
by this matrix.

Queue insertion now publishes the entry and requests rescheduling in both
scheduler modes. If a remote target CPU is idle, it sets a coalesced
notification bit. A local enqueue needs no interrupt because that CPU is
already executing the kernel; this also excludes transient requeue during
a local context switch. Syscall return and process exit flush these bits after scheduler/ring
guards are released, with local interrupts disabled. Each flush takes a
snapshot of the mask and attempts each target once, at most 64 attempts.
The APIC sender checks its busy bit once; it does not spin or wait for
completion. A busy APIC retains the notification bit and reschedule flag;
the periodic timer remains a fallback. Notifications use the existing
reschedule interrupt and do not change task ownership, affinity or queue
selection. Diagnostic images report sent/busy counts in bounded snapshots.

The enqueue decision is serialized with CPU ownership under `SCHEDULER`.
A concurrent flush can see a newly published notification before that guard
is released, but the target cannot consume the queue until it acquires the
same scheduler lock. Atomic mask exchange preserves concurrent publications;
a busy attempt restores its bit. No scheduler or ring lock crosses the
notification send or a blocking operation.

Native pipe and TTY blocking syscalls also publish a local dispatch
notification when registering their wait. Their existing syscall rewind
rechecks readiness after wake. A successful notification is delivered after
SYSRET and parks the caller before the next replay; an APIC-busy attempt
retains the flag and falls back to the periodic timer. Publication is atomic-only under the scheduler
guard. The actual nonblocking APIC attempt remains outside all guards.
Nonblocking descriptors retain their explicit partial-transfer/EAGAIN path.

The first combined 2-CPU gate with remote idle-CPU notifications passes both
foreground closure protocols, exact teardown status/replies and every size
(`target/pty-liveness-complete-cpu2.serial.log`). Its slow 1 MiB stream verifies
1048576 bytes over 4082 reads in 5310 ms: 2500 ms sleeping, 2030 ms in IPC,
and a maximum read IPC of 20 ms. Snapshot counters show successful dispatch
notifications with no APIC-busy attempts. This is a single successful boot;
complete repeated results, including the later local parking change, follow.

The remote-notification matrix (`target/pty-liveness-complete-matrix/results.json`,
ISO SHA-256 `76da236f09c906d31fefe4f3c1db8c6532f7304e58592f6e598db5e61c4fbcd6`) completes 9/9 boots:

| CPUs | Complete boots | Boot seconds | Slow 1 MiB stream milliseconds |
| --- | --- | --- | --- |
| 1 | 3/3 PASS | 84.15 / 83.66 / 87.91 | 5420 / 5470 / 5550 |
| 2 | 3/3 PASS | 84.66 / 78.64 / 48.09 | 5420 / 4930 / 4850 |
| 4 | 3/3 PASS | 49.84 / 48.84 / 49.08 | 3450 / 3500 / 3460 |

Each boot includes both foreground closure protocols and checked kill/wait/
close replies. All six sizes pass normal and slow-consumer checks, without
enlarging deadlines. In the three 4-CPU slow 1 MiB cases, IPC consumes
720 / 680 / 730 ms, versus 12240 ms before notification when the failed
stream reached only 817260 bytes. This is observed dispatch/transfer evidence,
not a throughput benchmark or a worst-case latency guarantee.

A final fresh-image matrix adds immediate local parking for native pipe/TTY
retries. Its results are recorded separately below; the remote-notification
results are not relabeled as validation of that later source change.

### Independent regressions on the final source

These gates were rebuilt after the local native pipe/TTY parking publication
was added. No functional source changed during this validation.
`target/pty-liveness-complete-regressions.json` retains build-plus-gate times;
the Stage A copied-image matrix retains boot-only times.

| Gate | CPUs | Result | Evidence |
| --- | --- | --- | --- |
| Stage A environment/script/PTY output | 2 | PASS, 101.32 seconds including build | `pty-liveness-complete-shell-phase2-cpu2.serial.log` |
| Stage A environment/script/PTY output | 1 | PASS, 68.86 boot seconds | `pty-liveness-complete-stage-a-matrix/shell-phase2-cpu1-run1.serial.log` |
| Stage A environment/script/PTY output | 4 | PASS, 70.86 boot seconds | `pty-liveness-complete-stage-a-matrix/shell-phase2-cpu4-run1.serial.log` |
| Helios pipe/socket I/O | 2 | PASS, 70.39 seconds including build | `pty-liveness-complete-helios-io-probe-cpu2.serial.log` |
| Helios thread startup | 2 | PASS, 59.03 seconds including build | `pty-liveness-complete-helios-thread-probe-cpu2.serial.log` |
| Helios Note terminal startup | 2 | PASS, 60.82 seconds including build | `pty-liveness-complete-helios-note-regression-cpu2.serial.log` |
| Audio PCI/stream/capture/test-tone DMA | 2 | PASS, 36.07 seconds including build | `pty-liveness-complete-audio-cpu2.serial.log` |

Evidence names in this table are relative to `target/`. Stage A copied-image
SHA-256: `11c34e570412732b07072ddb2a5b07aa05742f744160e09fe7410f733f9347b4`.

Audio passes its existing PCI binding, stream initialization, capture
registration and test-tone DMA completion gate. No audible-output or
concurrent-audio latency claim is inferred. Helios startup milestones do not
prove interactive editing or clean application exit.

### Final Phase 2.1A result

`target/pty-liveness-dispatch-final-matrix/results.json` records the final
fresh-image matrix, including local native pipe/TTY parking notifications.
ISO SHA-256: `b003bd57d5495f3f33320473c890033e020ad52d2de5c4e77cd9554b381493aa`. The focused 2-CPU build/gate also passes
in `target/pty-liveness-dispatch-final-cpu2.serial.log` (67.39 seconds including
build). Every final repeated boot has all expected markers and 16 exact-content
stream summaries. No failed or timed-out boot was silently retried.

| CPUs | Complete boots | Boot seconds, runs 1 / 2 / 3 | Slow 1 MiB milliseconds, runs 1 / 2 / 3 |
| --- | --- | --- | --- |
| 1 | 3/3 PASS | 44.83 / 43.82 / 44.08 | 2550 / 2810 / 3010 |
| 2 | 3/3 PASS | 44.32 / 43.57 / 44.57 | 3070 / 2780 / 3180 |
| 4 | 3/3 PASS | 44.82 / 44.82 / 44.82 | 2760 / 2470 / 2950 |

Each size-loop count below includes three normal and three slow-consumer
checks per CPU configuration. Every check verifies the exact byte count and
A–W content pattern incrementally. Additional delayed/nonblocking 64 KiB and
concurrent 256 KiB streams are excluded from this table and pass separately.

| Output size | 1 CPU | 2 CPUs | 4 CPUs |
| --- | --- | --- | --- |
| 1 KiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |
| 4 KiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |
| 16 KiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |
| 64 KiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |
| 256 KiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |
| 1 MiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |

Every final boot also passes concurrent PTY sessions with disjoint payloads,
a delayed consumer, blocking TTY parking, nonblocking partial/EAGAIN writes,
both nonblocking child-wait helpers, two pipe producers with a slow consumer,
pipe EOF, early reader close/EPIPE, legacy blocking wait, broker full/partial/
slave EOF/master closure, and foreground master/session closure. Foreground
closure requires relay status 129 and foreground-child termination; normal
teardown requires the SIGKILL status 137 and an acknowledged control close.
The kernel's sticky-terminal-state self-test passes in every boot.

The 1-CPU delayed-drain snapshot records a blocked TTY writer and successful
local notifications (`sent=18`, `busy=0`); the pipe snapshot records the native
pipe wait with reader/writer counts. The final 2-CPU focused slow 1 MiB stream
completes in 3050 ms with 310 ms in IPC. These observations support actual
parking and wake/dispatch progress rather than merely hiding EAGAIN in a
userspace retry loop. The final matrix contains 108 size-loop checks plus
36 additional exact-content streams, with unchanged 15-second stream,
two-second call/closure, and 300-second host boot deadlines.

The demonstrated PTY liveness and SMP cleanup failures are resolved. Command
substitution has not been implemented; its explicit unsupported diagnostic
remains. The legacy `sshl6` lookup authorization failure remains independent
of the broker/pipe execution path and is not counted as a passing gate.

Remaining validation limits:

- QEMU uses TCG on this host, 1024 MiB RAM and 1/2/4 CPUs. These are correctness
  and bounded-deadline results, not a hardware throughput or worst-case latency
  guarantee. The configured scheduler is RoundRobin; BORE is not runtime-tested.
- Recorded mmap startup critical sections can hold the scheduler during page
  reclaim. Memory ownership/reclaim policy has not been redesigned.
- No APIC-busy attempt occurs in the final recorded snapshots. Its retained-bit
  and periodic-timer fallback is source-reviewed, not fault-injected here.
- Native tests cover the graphical client's close protocol; an interactive GUI
  close click, GUI latency, abrupt client disappearance and broker-unavailable
  fallback/orphan recovery are not established by these tests. The test drains
  the separate retained TTY ring before reusing its fixed test tab.
- Audio evidence covers device/stream/capture/DMA progress. Audible playback
  and latency under concurrent PTY load remain unmeasured. Helios evidence
  stops at its specified I/O/thread/terminal startup milestones.
- The legacy Phase 1.5 harness needs an authorized numbered-shell endpoint
  handoff or a separately reviewed policy change; security checks remain intact.

### Independent revalidation of commit `45f027ed2`

On October 14, 2026, a new audit found the Phase 2.1A repairs above already
present in the clean checkout at `45f027ed2`. The audit reviewed the native
pipe, TTY ring, PTY broker, foreground relay, waitpid, IPC completion, process
exit and scheduler notification paths before rerunning the gates. The existing
runtime repair was retained. A subsequent full Stage A IPC failure prompted
additional bounded failure diagnostics, described below. Command substitution
remains deferred.

The retained pre-repair ISO was booted three more times with two CPUs.
`target/pty-liveness-reaudit-baseline/results.json` identifies image SHA-256
`803e73209bb91c0a068b7b6b89428eb59feb7b841b50448335821ef9106f4446`.
All three boots explicitly failed their unchanged guest deadlines:

| Run | Outcome | Payload received before the failure | Boot seconds |
| --- | --- | --- | --- |
| 1 | FAIL, 1 MiB stream deadline | 102400 / 1048576 bytes | 51.82 |
| 2 | FAIL, original 64 KiB prompt deadline | 22-byte app-start marker, no payload | 56.84 |
| 3 | FAIL, 1 MiB stream deadline | 610304 / 1048576 bytes | 62.84 |

Runs 1 and 3 first completed the original 64 KiB probe. The exact 64 KiB
failure is therefore intermittent; the circular wait reproduced in every
boot, at different transfer boundaries. In run 2, relay PID 36 (`sshl7`) is
`BlockedOnIpc`, detached from every CPU/ready queue, with
`wait_child=Some(37)` and no pending IPC request or reply. Producer PID 37
is Running on CPU 1, after its first write, with the TTY stdout ring full
at 8192 bytes. The PTY server is waiting for another request. This is direct
evidence of the child-exit/output-drain dependency, rather than an inferred
IPC reply loss. These native actors are single-threaded processes.

The old image predates several current test scenarios and the richer stream
summary format. Its JSON therefore has missing new markers and empty parsed
stream summaries. The explicit guest FAIL markers and bounded state snapshots
above establish the reproductions independently of that format difference.
Every failed boot remains recorded; no retry replaces it.

A fresh traced two-CPU build passes in
`target/pty-liveness-reaudit-fresh-cpu2.serial.log`; its copied image is
`target/pty-liveness-reaudit-traced.iso`. The delayed-drain test observes
`tty_write_wait=7`, then verifies all 65536 bytes. Its blocking producer
reports 12 partial writes and zero EAGAIN returns; the nonblocking producer
reports 12 partial writes and 3865 EAGAIN returns, and also delivers exactly
65536 bytes. Both foreground closure protocols and the terminal-state
self-test pass. Dispatch counters record successful notifications with zero
busy attempts. The traced slow 1 MiB stream finishes in 2790 ms, within its
unchanged 15000 ms deadline.

Host revalidation passes 43 shell tests, 32 dash comparisons, 13 descriptor
tests, 42 IPC library and 9 integration tests, 9 broker tests, 28 graphical
terminal tests, 37 `sunlight-tty` library tests, 2 partial-write helper tests,
and 9 launcher tests. `cargo check -p sunlight-kernel` also passes. Evidence
uses `target/pty-liveness-reaudit-*.log`. Dash was extracted from an existing
local Ubuntu 24.04 image with networking disabled; the comparisons use
`SSHL_REFERENCE_SHELL="$PWD/target/sshl-reference-dash"` so the path remains
absolute when the suite executes its cases in temporary directories.

The earlier host table called the 37 tests "TTY service" tests; these are
actually the `sunlight-tty` library tests. A direct host invocation of
`cargo test -p sunlight-tty-server` fails to compile its native binary because
its panic handler conflicts with the host test harness (`E0152`). The native
service is compiled and exercised by the QEMU gates. This audit does not
claim a host test pass for that binary or an unfiltered libc-suite pass.

The focused gate was rebuilt again with `SUNLIGHT_PTY_TRACE=0`, then copied
to `target/pty-liveness-reaudit-untraced.iso`. Its two-CPU build/gate passes.
The subsequent serial matrix, with one QEMU guest at a time, completes all
nine boots without IPC event tracing. Image SHA-256:
`89b1ca9ae5a7448e0ed73907e9fad09da0691310b7e80f9a375d025e879df145`.
Results and per-boot logs are in `target/pty-liveness-reaudit-matrix/`.

| CPUs | Complete boots | Boot seconds, runs 1 / 2 / 3 | Slow 1 MiB milliseconds, runs 1 / 2 / 3 |
| --- | --- | --- | --- |
| 1 | 3/3 PASS | 41.83 / 42.32 / 41.07 | 2590 / 2660 / 2440 |
| 2 | 3/3 PASS | 43.07 / 43.32 / 43.33 | 2740 / 2690 / 2480 |
| 4 | 3/3 PASS | 46.08 / 46.34 / 46.32 | 2890 / 2630 / 2640 |

Each boot retains all expected markers and 16 exact-content stream summaries.
The six requested sizes each pass three normal and three slow/partial checks
per CPU configuration: 6/6 per size at 1, 2 and 4 CPUs. Across the matrix,
there are 108 size-loop checks plus 36 additional delayed/nonblocking and
concurrent-session streams. Every boot also passes concurrent pipe producers,
EOF, early reader close/EPIPE, both wait helpers, legacy blocking wait,
broker partial/full/close cases, both foreground closure protocols, and exact
teardown status/replies. Blocking producers report zero EAGAIN; the dedicated
nonblocking scenario exercises EAGAIN explicitly. The 15000 ms stream,
2000 ms IPC/closure and 300-second host deadlines remain unchanged.
These boots confirm the committed repair and precede the additional
reclaim-budget correction below.

The broader Stage A gate was rebuilt with tracing disabled. The two-CPU gate
passes (94.07 seconds including build), and its copied image passes at one CPU
(75.12 boot seconds). At four CPUs its PTY master read IPC fails during the
initial 1 MiB three-stage pipeline (52.09 boot seconds), before reaching the
100 small-pipeline repetitions, PTY environment and live-output checks.
`target/pty-liveness-reaudit-stage-a-matrix/results.json` retains both copied-
image outcomes and SHA-256
`59b806692b6286d622d68c38f91db358ca1a478f7df153881dba95a45767d27c`.
The failed log ends with foreground child PID 165 and `FAIL pty read timeout`.
That original error site labeled every `IpcCallError` as a "timeout" and did
not capture a state snapshot. Thus it establishes neither the transport error
kind nor its cause; actual deadline expiry was unproven in that log. It is not
dismissed as host contention or silently replaced by another run.

The test's prompt-read IPC failure now takes the same bounded snapshot as
the stream-read failure and reports the IPC error, session, generation,
received byte count and elapsed time. The existing one-shot traced watchdog
also covers `shell-phase2`. Neither change adds hot-path serial printing,
changes workload size, alters authorization, nor enlarges a deadline.

Independent rebuilt two-CPU regressions pass: Helios I/O (78.92 seconds),
Helios thread startup (70.00 seconds), Helios Note startup (65.00 seconds),
and audio DMA (47.85 seconds); all times include their builds. These results
are in `target/pty-liveness-reaudit-regressions.json` and corresponding logs.
The independent Phase 1.5 gate fails (75.31 seconds including build), with
`sshl6 registered=true alias=true` followed by the driver's denied LOOKUP for
hash `0x0a994a83b88a6d83`, mask `0x40000`. The exact rejecting check remains
`mediate_nameserver_lookup` -> `service_capability_allows_hashed_name` ->
`matches_user_session_service`. No capability or registration check is weakened.

### Additional reclaim critical-section repair

The instrumented four-CPU gate passes, then its copied image completes four
of five repeated boots. Run 4 reproduces the initial 1 MiB pipeline failure
with an actual `IpcCallError::Timeout` and a bounded state/event snapshot.
`target/pty-liveness-reaudit-phase2-diag-matrix/results.json` retains all five
outcomes (77.88 / 90.14 / 85.63 / 55.84 / 91.64 boot seconds); run 4 is a
failed result. Image SHA-256:
`e6a11688263586ec93cef310055ffb2399abfcda725303279eb94f28eb640537`.

The failing command is:

```sh
sshl --probe emit 1048576 | sshl --probe copy | sshl --probe count > /tmp/sshl-count
```

Foreground child PID 165 enters its initial 1 MiB heap `mmap`. The snapshot
records scheduler-lock wait 1 microsecond, hold 2564320 microseconds, 1792
candidate scans, 1792 reclaimed pages and seven watermark activations.
Driver PID 35's PTY master read generation 332 is requested at tick 2945;
its deadline wake occurs at tick 3201, after the same 256-tick interval.
It then reports `pty read IPC Timeout`, zero received bytes and 2660 ms elapsed.
The broker is Ready with one queue entry and an intact reply/wait state; the
relay has no child wait. Pipe producer PID 163 is correctly parked on a full
4096-byte ring, PID 164 is runnable, and PID 165 subsequently waits for input
on the empty second pipe. This capture establishes scheduler-lock exclusion
during synchronous reclaim, rather than a child-exit deadlock, missing reply,
capability failure, or host-harness stall.

`map_anonymous_kind` called `maybe_reclaim_for_anonymous_allocation` before
each installed page. Each call restarted the 256-page direct-reclaim
allowance. Thus the supposedly bounded reclaim activation could repeat
throughout one mapping while retaining `SCHEDULER` and `PMM`, preventing the
PTY broker, client and pipe consumers from entering their kernel paths.

Each anonymous mapping transaction now owns one 256-page direct-reclaim
budget, shared across its allocation loop. Reclaim charges its requested
allowance before attempting eviction, including unsuccessful attempts, so
incompressible or stale candidates cannot repeatedly renew the allowance.
An exhausted budget skips further direct reclaim in that transaction. Frame
allocation, sanitization, reservation/commit, rollback, ownership, shootdowns,
watermarks and ZRAM pool policies remain on their existing paths. There is
no new lock, blocking wait, output buffer, capability exception, or syscall ABI.

When the allocation cannot obtain enough frames after bounded reclaim, the
existing `ENOMEM` path rolls back rather than publishing a partial mapping.
This is an intentional per-operation reclaim limit: a large eager allocation
under pressure can fail instead of doing further eviction in the same locked
syscall. It is not a maximum mapping-size limit when enough frames are free.
The change does not establish a worst-case time bound for every mapping,
compression or shootdown, and does not redesign eager allocation or VM locking.

Traced images assert that each `mmap` reclaims at most 256 pages and retain
one maximum observed mmap hold record. A single snapshot after the 100
completed PTY pipeline repetitions prints that record. No per-request serial
printing is added. The first corrected four-CPU full gate passes in
`target/pty-liveness-reclaim-fixed-phase2-cpu4.serial.log`. Its initial PID 165
mapping holds the lock for 171385 microseconds with 256 scans/reclaims and
one activation. The largest observed hold in that boot is 543121 microseconds,
also with 256 pages and one activation. These are measurements on this host,
not worst-case timing guarantees; all deadlines remain unchanged.

The count-only correction's complete Stage A matrix is retained in
`target/pty-liveness-reclaim-fixed-phase2-matrix/results.json`, SHA-256
`d0e84587a69c728f00918d68bfdac787066a3fb6e22eb76e3e706dff7500e115`.
One CPU passes all three boots (67.36 / 75.13 / 74.87 seconds); two CPUs pass
all three (81.13 / 74.88 / 77.38). Four CPUs pass runs 1 and 2 (86.13 / 84.63),
but run 3 fails at 78.15 seconds during the small-pipeline repetition loop.
It reports an actual read IPC timeout after 3240 ms, with the 22-byte app-start
marker received. PID 342's mmap performed exactly 256 scans/reclaims and one
activation, but held the scheduler lock for 2482235 microseconds. The count
bound worked; it did not sufficiently bound synchronous latency. This is an
8/9 matrix, not a completed SMP validation. The following validation runner
stopped on that failure instead of continuing or silently retrying.

Allocation-driven reclaim now also shares a 10 ms execution budget across
the transaction. The reclaimer checks elapsed time before starting each
candidate; it finishes an in-flight compression/mapping replacement/shootdown
before releasing its frame or slot. It retains unprocessed candidates in the
queue. Manual privileged reclaim uses its existing page-count contract.
No caller deadline is extended, and there is no sleep, polling retry, new lock,
or release/reacquisition of a partially installed VM transaction.

Elapsed time uses the calibrated local TSC (`interrupts::now_ns`), rather
than `timekeeping::monotonic_ns`: the latter depends on BSP ticks, which can
stop while this transaction holds `SCHEDULER` with interrupts disabled.
The executing transaction cannot migrate, so this duration does not compare
timestamps from different CPUs. Without calibrated TSC, opportunistic
allocation reclaim is skipped and allocation/ENOMEM rollback remains usable.
The 10 ms allowance is a work budget checked between atomic page operations,
not a promise that a single compression or shootdown cannot exceed it.
Large eager allocations under pressure may encounter the documented ENOMEM
boundary sooner; ample-memory mappings retain their size and semantics.

### Validation of the shared page and elapsed-work budgets

The timed-budget four-CPU build/gate passes. Its copied Stage A image then
passes nine of nine boots under the original deadlines, including the 1 MiB
three-stage pipeline and 100 small pipelines in every boot. Shell heap usage
remains 3043 bytes before/after and the descriptor probe remains FD 3.
Results: `target/pty-liveness-reclaim-timed-phase2-matrix/results.json`.
Image SHA-256:
`2a8f6d99c27a538f15b067585d8d52c0b253ede6c358fc7fab19a3186c9213e8`.

| CPUs | Full Stage A boots | Boot seconds, runs 1 / 2 / 3 |
| --- | --- | --- |
| 1 | 3/3 PASS | 64.36 / 71.88 / 81.38 |
| 2 | 3/3 PASS | 87.63 / 81.87 / 81.13 |
| 4 | 3/3 PASS | 92.13 / 93.38 / 91.38 |

The bounded snapshots retain no mmap hold meeting the 50 ms tracing
threshold. The sole recorded mmap waited 61734 microseconds for the scheduler
lock and held it for 883 microseconds, with no reclaim. The recording threshold
includes combined lock wait/hold, and the maximum-hold slot prevents later
events from overwriting a long recorded mmap. This is evidence for these
instrumented workloads, not a global scheduler latency guarantee.

The focused gate was rebuilt with `SUNLIGHT_PTY_TRACE=0`; its two-CPU build/
gate passes in 77.44 seconds. Its copied image also completes nine of nine
boots without IPC event tracing. Results and individual serial logs are in
`target/pty-liveness-reclaim-timed-final-pty-matrix/`. Image SHA-256:
`7cf75cf01dc4e72aae2a7f579ef0c50d527d4df62904d9b0b18f2185953a5227`.

| CPUs | Focused PTY boots | Boot seconds, runs 1 / 2 / 3 | Slow 1 MiB milliseconds, runs 1 / 2 / 3 |
| --- | --- | --- | --- |
| 1 | 3/3 PASS | 49.59 / 43.57 / 44.33 | 3170 / 2790 / 2690 |
| 2 | 3/3 PASS | 45.58 / 46.58 / 53.07 | 2770 / 2830 / 3350 |
| 4 | 3/3 PASS | 53.58 / 47.57 / 48.32 | 2820 / 2890 / 2950 |

| Exact output size | 1 CPU | 2 CPUs | 4 CPUs |
| --- | --- | --- | --- |
| 1 KiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |
| 4 KiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |
| 16 KiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |
| 64 KiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |
| 256 KiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |
| 1 MiB | 6/6 PASS | 6/6 PASS | 6/6 PASS |

Each size has three normal and three slow/257-byte-consumer runs per CPU
configuration, verified incrementally against the A–W pattern. Every boot
retains all expected markers and 16 stream summaries: 108 size-loop checks
and 36 delayed/nonblocking/concurrent-session streams in the matrix. Every
boot also passes blocking TTY parking, explicit nonblocking partial/EAGAIN
handling, concurrent pipe producers, pipe EOF and early reader close/EPIPE,
both nonblocking wait helpers, legacy blocking wait, broker partial/full/EOF/
close semantics, foreground master/session closure and checked teardown.
Guest stream deadlines remain 15000 ms, IPC/closure deadlines 2000 ms, and
host boot deadlines 300 seconds. Failed diagnostic and page-only-budget
boots remain separate failed results; none was silently retried or excluded.

Independent gates were rebuilt from the timed-budget source with two CPUs
and tracing disabled. `target/pty-liveness-reclaim-timed-final-results.json`
retains their explicit outcomes and build-plus-gate durations. Corresponding
serial and gate logs use `target/pty-liveness-reclaim-timed-final-<phase>`.

| Gate | CPUs | Result | Seconds including build |
| --- | --- | --- | --- |
| Security / MM-0 / MM-2A | 2 | PASS | 48.18 |
| SWAP-1 multi-pool pressure | 2 | PASS | 47.05 |
| Helios pipe/socket I/O | 2 | PASS | 75.12 |
| Helios thread startup | 2 | PASS | 65.93 |
| Helios Note startup | 2 | PASS | 65.04 |
| Audio stream/capture/test-tone DMA | 2 | PASS | 42.88 |
| Legacy Phase 1.5 TTY lookup | 2 | FAIL, independent authorization blocker | 70.21 |

The security gate includes injected mmap/SHM allocation rollback, native and
thread address-space lifecycle, IPC multi-caller queues, timeout cancellation,
bounded queue reuse, notification coalescing, generation/late-reply races,
capability lifecycle, REGISTER ownership/identity, and 64 SMP terminal-state
stress iterations. SWAP-1 verifies exact restored content, RW/NX metadata,
incompressible-page ownership, corrupted-payload rejection, swapped munmap,
stale slot generations and exit accounting. Final `cargo check -p
sunlight-kernel` and `git diff --check` pass.

The Phase 1.5 failure again records `sshl6 registered=true alias=true`, then
PID 35's rejected LOOKUP for hash `0x0a994a83b88a6d83` with UserSession mask
`0x40000`. The rejecting check returns `IpcError::InvalidCapability` through
`mediate_nameserver_lookup` -> `service_capability_allows_hashed_name` ->
`matches_user_session_service`. This needs a separate authorized endpoint
handoff or reviewed policy design; registration and capability checks retain
their existing boundaries. The failed gate is not counted as a pass.

The reproduced PTY execution failures now pass both final repeated matrices.
This does not establish arbitrary workload scheduling bounds, real-hardware
performance, GUI interaction latency, or audible/concurrent-audio latency.
Helios milestones establish the tested I/O and startup behavior, not Note
editing or clean application exit. Manual privileged reclaim retains its
existing count-only contract. Atomic page work can exceed the elapsed-work
allowance, and eager allocation under pressure can return ENOMEM earlier.
Unfiltered host libc and the native TTY-server host panic-handler conflict
remain outside the claimed host passes. Command substitution remains deferred;
this change implements only the liveness investigation, repair and regressions.


## Phase 2.1B: native POSIX command substitution

October 14, 2026. This section describes the current substitution delivery;
Stage A/2.1A results above are historical and are not counted as new passes.

### Parser and expansion architecture

`WordPart` carries an optional `Rc<Command>` alongside its quote provenance.
The shared lexer enters a fresh lexical region after `$(`. At candidate
unquoted right parentheses it asks the existing recursive-descent grammar
whether the command list is complete. The grammar distinguishes a substitution
terminator from a case-pattern terminator, function header or subshell close.
Quoted/escaped parentheses and comments remain lexical content. Nested
substitutions recursively enter the same bounded lexer; there is no regular
expression or parenthesis-depth counter. Parsing rejects malformed contents
before executing any outer command and translates nested byte offsets back
into the containing source for line/column diagnostics. `$((` produces the
separate arithmetic-unsupported diagnostic. Single quotes and escaped dollars
suppress substitution; double quotes allow it with independently parsed quotes
inside the nested command list. Multiline content is supported. Backticks and
here-documents remain unsupported.

`script::expand` invokes `expand_word_with` with a runtime-backed AST callback.
Captured characters enter the existing atom/IFS/quote-removal machinery directly;
they are never reparsed for dollars, quotes, backticks or escapes. Unquoted
output is field-split using the current IFS. Quoted empty results retain an
empty argument. Assignment values and redirection operands are scalar. Every
trailing LF is removed after capture; embedded LFs and carriage returns remain.
Generated wildcard characters remain literal in this phase; literal unquoted
pathname patterns retain the explicit unsupported diagnostic. This is a
purposeful pathname-expansion limitation, not a glob implementation. Case
patterns still honor their actual quote masks.

### Isolation and descriptor capture

Substitution runs the shared `run` interpreter with an isolated context. It
copies bounded variable values/attributes, positional parameters and cwd once
for that scope; function bodies share their existing `Rc` ASTs. It does not
spawn a second shell merely to interpret the substitution. Variables, function
definitions, positional changes and `cd` stay local, while filesystem effects
remain observable. Exported values and temporary prefixes reach external
children through the existing validated native environment/spawn path. Native
external execution temporarily changes the process cwd for spawn and restores
it on completion/error. This remains the shell's existing single-threaded
execution model; no interpreter/collector worker thread is introduced.

`capture::Captures` assigns stack-scoped negative sink handles. Builtin writes
append through the same bounded collector. Negative handles never cross spawn:
the platform adapter maps each distinct referenced sink to an anonymous pipe.
Nested stdout and explicitly redirected stderr can refer to different active
collectors. Each external pipeline starts all stages first, drops the parent's
write ends, then polls/drains all captured readers before reaping any child.
Reads use a 4 KiB stack buffer and handle partial data, EOF, readiness wakeups
and transient EAGAIN. The existing writer helpers retain partial-write/EPIPE
handling. Collectors use fixed three-entry readiness arrays rather than
allocating on every read.

Native syscall 159 exposes only bounded pipe-reader readiness: 1–3 descriptors,
POLLIN, and timeout 0 or -1. It validates caller-owned readable pipe handles
and checked writable userspace memory. The readiness check and native waiter
registration share the producer's scheduler lock. Pipe transitions wake the
registered task; it rechecks on syscall restart with the original syscall
number preserved. It uses the existing native park protocol and pipe readiness
logic without attaching Linux state or altering process personality. Scheduler
lock work is bounded to the small descriptor set; capture reads/allocations
occur outside that lock. Host capture uses host poll through the same interpreter.

Stderr follows the inherited descriptor unless an inner redirection remaps it.
Assignment-only commands establish their redirections before RHS substitutions,
so `v=$(echo err >&2) 2>file` writes the error to `file`. Regular command arguments
expand before command redirections, matching the tested dash behavior.
Assignment-only status is the last substitution's exit status, including failed
execution and inner redirection; an outer command such as `echo` supplies its
own status. Nonzero substitution status alone does not abort expansion.

On collector failure, adapters close readers, terminate every directly launched
pipeline stage and reap them, then return an explicit expansion error. Every
scope ends/pops its collector on interpreter failure. Redirection scopes and
capture pipes close through their existing cleanup/owned-descriptor paths.
There is no wait-before-drain dependency, temporary capture file or silent
truncation. Spawn credentials/capabilities and trusted executable identity
remain in the existing authorized native launch path; no security allowlist is
broadened. The page-reclaim budget and existing PTY/IPC repairs are unchanged.

### Resource policy and compatibility boundaries

| Resource | Limit / policy |
| --- | --- |
| Raw output per substitution | 128 KiB, including trailing LF bytes before trimming |
| Aggregate active captured output | 256 KiB across nested scopes |
| Runtime substitution nesting | 16 scopes, independent of bounded function/source recursion |
| Lexical substitution nesting | 32 levels, existing 64 KiB source / 8192 token limits |
| Final expanded word / variable | Existing 16 KiB byte limit; quoted empty anchors do not consume output bytes |
| Native process arguments | Existing 15-entry / 1024-byte arena limits still apply |
| Capture reservation | Fallible Vec reservation, explicit status 2 on allocation/limit failure |
| Embedded NUL | Reject the entire expansion with status 2; do not discard bytes |
| Non-UTF-8 output | Reject the entire expansion with status 2 |

Vec capacity can exceed populated bytes due to geometric growth; the byte
limits bound contents, with collector capacity below twice those limits under
the current growth policy. Parser AST/context/BTree allocations and existing
String/builtin formatting are still not fully fallible. This is bounded capture,
not a claim of complete shell OOM recovery. The tests exercise an existing
interpreter/context successfully after overflow, NUL and word-limit errors.

A failed inner redirection retains SunlightOS's established status 1; dash uses
2. The differential harness asserts both exact results and stderr presence as
one documented difference. Syntax/command-not-found/open diagnostic wording
is platform-specific; those cases compare stdout/status and stderr presence.
Stable substitution cases compare stdout, stderr and exit status exactly.

### Validation and retained failures

Final-source shell gates passed once each with 1, 2 and 4 CPUs. Build-plus-boot
durations were 109.79, 115.28 and 139.93 seconds; no test deadlines were increased.
The final matrix summary is `target/substitution-final-results.json`, with
per-phase serial and driver logs under `target/substitution-final-*.log`.

| Check | Current result |
| --- | --- |
| Shell host tests | 48 passed: 24 library and 24 binary |
| Dash differential | 83 matching cases, including 51 substitution cases; 1 explicit redirection-status difference |
| Spawn environment validator | 3 passed |
| Independent descriptor harness | 13 passed; host adapters do not establish native I/O behavior |
| Kernel check | Passed |
| Final shell-phase2 | 1 CPU PASS; 2 CPUs PASS; 4 CPUs PASS |
| Dedicated shell-pty-liveness | 1 CPU PASS; 2 CPUs PASS; 4 CPUs PASS |
| IPC foundation/security/lifecycle | 2 CPUs PASS |
| SWAP-1 pressure and reclaim | 4 CPUs PASS |
| Helios I/O and thread startup | 2 CPUs PASS for each |
| Audio stream/input initialization and test-tone DMA | 2 CPUs PASS; no capture-frame or audible-output claim |
| Security with allocation rollback injection | 2 CPUs PASS, including injected mmap/SHM rollback and IPC SMP stress |

Host artifacts are `target/substitution-host-tests.log` and the differential
command uses an extracted dash at `/tmp/sshl-dash-reference/usr/bin/dash`.
The first one-CPU boot (`target/substitution-smp1.log`) failed the first
substitution output assertion and logged `Unknown syscall 0`: the initial new
native readiness handler rewound the syscall instruction but returned zero
instead of the original syscall number. The focused fix uses the existing
native pipe restart convention. This failed boot is retained and is not counted
as a pass. The next one-CPU development boot
(`target/substitution-smp1-restart.log`) reached the wc/redirection check and
failed because the test expected `6\n`, while native wc emitted seven spaces
before `6\n`. Quoted capture preserved those spaces correctly; the test now
checks the native utility's exact output. A subsequent development one-CPU gate
passed. Final gates rebuild from the corrected source and run the additional
compound-error routing/environment probes.

Guest additions verify nested/single/double quoting, trimming and embedded LFs,
empty output and splitting, functions/control flow/case grammar, cwd/variable
isolation, assignment/pipeline/nested statuses, missing commands, failed opens,
grep/wc, stderr and redirection targets. Native capture checks exact 8192-byte
A–W output through 257-byte producer writes, two active 8192-byte stdout/stderr
sinks, exactly 131072 trailing LF bytes, a 1 MiB producer with an early 17-byte
consumer, explicit overflow/NUL/word-limit failures, poll argument validation,
100 repeated substitutions in one interpreter with heap/descriptor checks,
and post-error recovery. Host tests additionally verify the aggregate bound
using three nested 96 KiB LF producers: each remains under the per-capture cap,
while combined output exceeds 256 KiB and fails without preventing a subsequent
successful substitution. This aggregate-overflow case has host evidence only.
PTY tests check direct/nested substitution, an 8192-byte
pipeline count, assignment status, overflow recovery and subsequent input.
Existing Stage A environment security, repeated process/scope, full PTY/live
output workloads remain part of the same gate.

Reproduce the focused checks with:

```sh
cargo test -p sunshell --no-default-features --features std --target x86_64-unknown-linux-gnu --offline
cargo build -p sunshell --no-default-features --features std --target x86_64-unknown-linux-gnu --offline
SSHL_REFERENCE_SHELL=/tmp/sshl-dash-reference/usr/bin/dash python3 tools/test_shell_phase2_host.py
# Repeat with SUNLIGHT_TEST_CPUS=2 and 4 for both shell gates.
SUNLIGHT_TEST_CPUS=1 bash tools/test.sh shell-phase2
SUNLIGHT_TEST_CPUS=1 bash tools/test.sh shell-pty-liveness
SUNLIGHT_TEST_CPUS=2 bash tools/test.sh phase2.6
SUNLIGHT_TEST_CPUS=2 bash tools/test.sh phase_sec
SUNLIGHT_TEST_CPUS=4 bash tools/test.sh swap1
SUNLIGHT_TEST_CPUS=2 bash tools/test.sh helios-io-probe
SUNLIGHT_TEST_CPUS=2 bash tools/test.sh helios-thread-probe
SUNLIGHT_TEST_CPUS=2 bash tools/test.sh audio
```

Use any locally installed dash for the reference; the `/tmp` path identifies
this run's extracted Debian dash 0.5.12 reference, not a repository dependency.
No generated images or serial logs are added to source control. The original
pre-existing workspace version changes are left intact.

The final matrix contains 12 passing gates. The allocation-injection run
records `[MM-2A] injected mmap/SHM allocation rollback: OK`, thread/address-space
lifecycle, capability checks and 64 IPC SMP stress iterations. Dedicated PTY
gates each verify streams through 1 MiB, partial/backpressured transfers,
terminal blocking input, EOF and early closure. Audio establishes initialized
input/output streams and test-tone DMA progress; it does not establish recorded
capture frames or human-audible output. Kernel check and `git diff --check` pass.
The unrelated historical Phase 1.5 `sshl6` authorization blocker and unfiltered
libc-suite gaps are not reclassified as passes by this delivery.

### Changed components

| Component | Files |
| --- | --- |
| Shared language/capture | `sunshell/src/parser.rs`, `script_parser.rs`, `expansion.rs`, `script.rs`, new `capture.rs`, `lib.rs` |
| Platform execution and interactive preparation | `sunshell/src/host_script.rs`, `native_script.rs`, `exec.rs`, `main.rs` |
| Native pipe readiness / park wakeup | `kernel/src/arch/x86_64/syscall.rs`, `kernel/src/process/mod.rs`, `kernel/src/sched/mod.rs` |
| Libc ABI | `sunlight-libc/src/lib.rs`, `sunlight-libc/src/sys.rs` |
| Regression tests | `sunshell/src/native_tests.rs`, `tools/test_shell_phase2_host.py`, `tools/tests/shell_phase2.expected` |
| Documentation | this report and `docs/shell/phase2-posix-scripting.md` |

### Remaining Stage C and limitations

Compound commands and functions work inside a substitution command list.
Function/scripting-builtin/compound *pipeline stages* remain the existing Stage C
transport requirement; this phase supports the existing native simple pipelines.
Native interactive outer command lists/compound grammar and the host development
REPL retain their earlier entry-point limitations; scripts and `-c` use the full
shared interpreter, and native interactive words use execution-aware preparation.

Overflow cancellation covers the directly spawned pipeline children. This phase
does not implement process-group cancellation of arbitrary grandchildren,
signal-backed traps, interactive Ctrl-C forwarding during synchronous expansion,
job control, arithmetic expansion or pathname expansion. PTY input forwarding
to stdin-reading producers, sustained inherited stderr during synchronous
interactive expansion and session-disconnect teardown are not established by
the prompt/recovery tests. Infinite output-free
commands retain normal shell wait behavior; byte limits are not execution
timeouts. Those signal/session teardown cases need separate foreground/PTY
integration gates. Continue with Stage C compound/function pipeline context
transport and foreground cancellation, then add later expansion stages separately.
