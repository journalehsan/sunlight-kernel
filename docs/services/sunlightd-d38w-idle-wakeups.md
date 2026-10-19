# SunlightOS D3.8-W: idle wakeup audit

## Scope and status

This audit extends [D3.7](sunlightd-d37-resource-baseline.md). DialogD's existing
blocking receive is preserved. The only service optimization is SessionD's
**no-session** wait. Active sessions retain their 50 ms supervision deadline.
ClipD remains resident with its existing clipboard state ownership. Startup
owners, RecoveryBlocked, D1/D2 policies, priorities and C1.2F dispatch/timer policy
are unchanged. Test-only service stop/start operations use the current manager.

The implementation has paired wake-count evidence and passing native 1/2/4 CPU
lifecycle gates. Acceptance remains qualified by a session-foundation gate
failure reproduced on the original service, and unavailable precise latency
and physical audio measurements; details follow below.

Evidence is local under `target/d38w-evidence/`; generated ISOs, ELF files, serial
streams and JSON are deliberately not committed. `tools/d38w_idle.py` reproduces
the sequential frozen-image TCG observations and summarizes counter deltas.
Unavailable measurements are JSON `null`, never inferred from wakeup counts.

## Counter contract and attribution limits

The existing optional `cpu_accounting_diag` feature now also stores four
saturating `u64` counters in each process incarnation. No allocation, lock, UART
output or additional timer is added at a wake site. The existing scheduler lock
serializes the increments and snapshots. Ordinary builds omit these fields.

* `wakes`: publications through the existing `record_wakeup` sites. These are
  blocked/suspended-to-Ready transitions, not hardware interrupts, dispatches,
  successful requests or unique physical idle exits. New task creation and
  internal Ready rotation do not increment this counter.
  A busy-yielding task can consume CPU and dispatches while recording no wakes;
  its state/runtime must be examined alongside this counter.
* `ipc`: the subset identified by the existing IPC-state wake sites. It includes
  replies, incoming messages, notifications, deadline expiration, cancellation,
  endpoint closure and signal wakes from BlockedOnIpc. It must **not** be called
  a count of incoming client requests or exclusively message-triggered wakes.
* `recv_exp` / `call_exp`: generation-validated receive / call deadline
  expirations in the BSP expiry scanner. They count expiration processing, which
  need not imply a distinct Ready transition if a task is already runnable.
  Receive/call state is checked exactly as before; instrumentation does not make
  an arbitration decision. Immediately expired deadlines consumed directly at
  receive/call entry are outside these scanner counters.
* Context switches and scheduler invocations retain the existing CPU accounting
  definitions. A schedule invocation need not select a different task.
* `timer_s` counts delivered per-core local timer interrupts. The BSP's coalesced
  timer-server notifications and deadline scanner can wake zero, one or multiple
  tasks per interrupt. Interrupt rate is not wasted userspace work.
* Runnable count is an instantaneous Ready+Running snapshot, including the task
  current during capture; it is not a time-weighted run-queue length.
* IPC queue depth, enqueue/dequeue, coalescing, expiry, cancellation, late reply
  and peer-closure counters use existing relaxed atomic diagnostics. They are
  approximate concurrent global snapshots outside SCHEDULER, not transactional
  queue snapshots at exactly the process snapshot timestamp.
* Armed receive/call deadlines are instantaneous counts among sampled tasks.
  Native deadline delivery is a process-table scan, not a new timer queue.
  Neither this nor the userspace timer server exposes a general timer-queue
  occupancy statistic. Spurious wake causes, rejected duplicate wakes, and
  retries immediately blocking again are not separately measured.

Serial diagnostics are bounded by the existing 120-report budget, at most once
per five guest seconds, with at most 64 task rows per report. Snapshot matching
uses `(PID, full 64-bit task publication incarnation)` from the existing
`Foreground::allocate_incarnation` allocator; a new incarnation's lifetime
counter is never treated as a window delta. Counters reset in both process
constructors. Borrowers have distinct task incarnations even within one VM.
The legacy top-runtime diagnostic now also matches the full generation. Public
telemetry ABI v4 and its 32-bit generation field are unchanged. The parser flags
a full 64-row snapshot as potentially truncated. Exited tasks between samples
can prevent the attributed sum from matching system totals.

Reporting runs outside scheduler/PMM locks but still inside the existing BSP
diagnostic interrupt path. Serial output and the initial mapped-page walk can
affect timing. The page walk and startup windows are excluded from steady idle
comparisons; both images use identical diagnostics. These are diagnostic TCG
observations, not an instrumentation-free CPU or power benchmark.

## Wakeup-source inventory

Locations below identify actual native event loops, rather than searching only
for a `sleep` spelling. Measured per-incarnation counts, including zero-wakeup
tasks, are retained in each capture's JSON. Approximate observed rates are
reported separately from configured intervals in the comparison section.

| ID | Service / source | Mechanism and configured interval | Reason and work |
|---|---|---|---|
| S | SessionD, `services/sunlight-sessiond/src/main.rs`, `_start`, `supervise`, `sync_wiseowl_session_authority` | Receive deadline, 50 ms before; after, indefinite only if `active.is_none()` | Before login: no child/deferred/readiness/stop work; repeats clearing authority. With a session: child liveness, restart, readiness, logout and deferred optional startup. |
| D | DialogD, `services/sunlight-dialogd/src/main.rs`, `DialogServer::run` | Existing indefinite `ipc_recv` (D3.7) | No idle maintenance; synchronous request processing can enter a GUI loop. No duplicate optimization. |
| C | ClipD, `services/sunlight-clipd/src/main.rs`, `_start` | Indefinite receive/reply-and-wait | KV reads at startup and clipboard/history persistence on requests. No periodic clipboard maintenance. |
| M | SunlightD, `services/sunlightd/src/main.rs`, event loop; `demand_runtime.rs::receive_timeout` | Deadline clamped to 1–50 ms | Activation/dependency discovery, child exits, D2 grace/drain/stop deadlines; unavailable dependency endpoint lookups amplify manager and Init IPC wakes. |
| I | Init, `services/init/src/main.rs`, `_start` | Receive timeout, 100 ms | Nameserver requests plus bootstrap manager liveness / RecoveryBlocked monitoring. Client requests wake earlier than 100 ms. |
| P | PTY broker, `services/pty_server/src/main.rs`, `_start` | Receive timeout, 50 ms | Deferred read/write completion, dead caller/owner cleanup; multiple state sources. |
| Z | Mezzo, `services/mezzo/src/main.rs` | Bounded receive maintenance (50 ms) | Process supervision/reaping; cannot remove the exit observation source without checking lifecycle obligations. |
| N | NetworkD, `services/sunlight-networkd/src/main.rs`, event loop | Receive until live state sync: 1/2/5/10/30/60 s backoff; then indefinite | DHCP/live net_server state synchronization. Discovery's 30 s stable interval is request-driven, not an unconditional 30 s background wake. |
| R | Resolved, `services/sunlight-resolved/src/main.rs`, `_start` | Indefinite receive/reply-and-wait | DNS configuration updates and generated resolv.conf on requests. |
| V | DeviceD, `services/sunlight-deviced/src/main.rs`, `_start` | Indefinite receive/reply-and-wait | Hardware inventory/driver registration on requests. |
| A | AudioD, `services/sunlight-audiod/src/main.rs`, event loop | Receive timeout 8 ms plus nonblocking capture-endpoint check | Pump DMA playback/capture, tone deadlines, persistence, two request endpoints. 8 ms is rounded to the kernel clock granularity. No validated two-endpoint indefinite wait. |
| B | net_server, `services/net_server/src/main.rs`, event loop | 10 ms receive deadline after initial receive | Frame proxy/smoltcp progress, retransmission/socket waiters; hardware/protocol cadence. May remain blocked on its initial receive in this no-NIC profile. |
| U | USB mouse, `services/sunlight-usb-mouse/src/main.rs` | Private receive deadline 10 ms between empty polls | xHCI HID ring servicing. Current driver has no validated IRQ wait endpoint. One-time TTY discovery uses yields. |
| T | TTY, `services/tty_server/src/main.rs`; BSP timer handler | Kernel periodic wake every third global tick; receive timeout 10 ms in maintenance path | Login, clocks, session/input/compositor coordination. Actual observed rate depends on pending work and the kernel's ~33 Hz publication. |
| H | Display, `services/sunlight-display/src/main.rs::compositor_poll_timeout_ms` | Receive deadline at most 500 ms; shorter notification/overlay/mode deadlines | Dead application/window cleanup, notification expiry and display-mode revert; Wise Owl lifecycle discovery retries every 2 s. |
| F | Files GUI, `sunlight-files/src/main.rs`, main window loop; `src/thumbnails.rs` | Shared UI event polling, normally at most 200 ms; app can choose shorter waits | Input, asynchronous thumbnail replies and UI state. Disabled thumbnail daemon does not imply clients have no thumbnail work. |
| O | Terminal, `services/sunlight-terminal/src/main.rs::poll_timeout_ms` | Idle UI wait 16 ms; streaming 1 ms with an 80 ms hold | Input, bounded PTY reads/writes, child spawning, caret and rendering deadlines. Measured separately with one tab. |
| L | Native shell, `sunshell/src/main.rs::pty_read` / native PTY loop; `services/pty_server/src/main.rs::read_ring` | READ_SLAVE returns ERR_WOULD_BLOCK immediately; no-progress shell iterations yield, without a timed wait | Retries empty input while also checking child output, foreground signals and child exit. This can produce thousands of IPC wake publications per second with an idle Terminal. |
| Q | Thumbnail daemon, `services/sunlight-thumbd/src/main.rs` | Indefinite receive/reply-and-wait | File/decode/cache work only on requests; disabled by default. |
| W | Wise Owl MemoryDB / Memory / Index native `src/bin_parts/*-native-body.rs` | Indefinite receive/reply-and-wait; MemoryDB private receive when persistence unavailable | MemoryDB suspended in the no-state profile; dependent services unspawned. Error/panic paths with yield loops remain candidates, not observed steady work. Host Index configuration's idle scan period defaults to 300 s; host Unix socket handlers block on reads. |
| J | Wise Owl Brain, `wiseowl-brain/src/bin_parts/wiseowl-braind-native-body.rs`, main loop | Three `ipc_try_recv` attempts then `process_yield`; no timed interval | Request, display lifecycle and control-panel lifecycle endpoints, plus internal state. Not spawned while MemoryDB prerequisite unavailable. A single indefinite receive would lose other event sources. |
| E | ThermalD / PowerD, `services/sunlight-{thermald,powerd}/src/main.rs` | Thermal deadline 1000 ms; PowerD indefinite receive | Sensor sampling/thermal lease decisions; resulting thermal/power IPC is legitimate work. |
| G | NiceD / GCD, `services/sunlight-{niced,gcd}/src/main.rs` | Receive maintenance deadline 1000 ms | Watchdog/priority policy, zombie and memory-pressure checks. No priority or reclaim policy changes. |
| K | timed / timezone / storage manager | timed absolute retry deadline capped at 30 s; others block on IPC | Time synchronization retry/backoff, time-zone and protected storage operations. Required protocol retries must not be removed on idle counts alone. |

Blocking, latency and cancellation dispositions for the same candidates:

| IDs | Existing blocking alternative / maximum response requirement | Shutdown, cancellation and risk |
|---|---|---|
| S | Existing single endpoint receive. No-session work has no deadline; queued requests wake immediately. Keep the existing 50 ms bound in **all** active states. | Single writer owns `active`; only an incoming request creates a session. Stop is a kernel signal, independent of messages. Checked fatal receive errors exit instead of retrying. Low risk in the restricted no-session state. |
| D,C,R,V,Q | Already blocking; no periodic deadline in their idle server loops. New requests should run at normal IPC dispatch latency. | Owner endpoint/capability validation and kernel teardown remain authoritative. Dialog GUI cancellation/client-exit behavior is a separate synchronous GUI contract. Clipboard data ownership must survive idling. No changes. |
| M,I,P,Z | Existing bounded IPC waits multiplex maintenance with requests. Preserve the configured maintenance latency, including exit/stop/recovery progress. | Blocking forever can strand supervision, deferred replies or RecoveryBlocked handling. Medium/high risk; unchanged. |
| N | Already transitions to blocking after successful synchronization. Retry bounds are the configured protocol backoff. | Peer loss and boot-order delays require retry progress. No replacement event verified; unchanged. |
| A,B,U | Current bounded waits service hardware, deadlines or multiple endpoints. Preserve 8/10 ms configured progress budgets. | Removing waits can cause playback underruns, lost capture requests, network stalls or frozen USB input. High risk; unchanged. |
| T,H,F | Existing waits bound input, repaint and cleanup. Preserve current cadence and all internal deadlines. | GUI closure, ownership cleanup and asynchronous replies are independent event sources. Medium/high risk; unchanged. |
| O,L | Terminal preserves its configured 16 ms idle budget. No validated shell wait multiplexes slave input, child stdout/exit and foreground signals; no stricter shell numeric contract established. | Indefinite receive on one source can strand another. PTY deferred writes do not establish a validated deferred-read contract. High-priority follow-up, medium/high implementation risk; unchanged. |
| W,J | Memory/Index already block; Brain needs a validated wait over three endpoints. No stricter numeric latency contract established by this audit. | Persistence prerequisites, lifecycle acknowledgments and internal execution must progress. Brain busy-yield is a high-priority future candidate only when actually resident. Unchanged. |
| E,G,K | Existing bounded maintenance waits or indefinite receive appropriate to each service. Preserve sample/watchdog/time-sync deadlines. | Required policy/hardware/retry work; no demonstrated redundant deadline selected. Unchanged. |

## SessionD blocking correctness

`supervise` and `tick_deferred_optionals` have no children or deadlines without
`state.active`. `sync_wiseowl_session_authority` clears authority and registrations
before the wait. Catalog/manifest discovery occurs at startup or on requests,
not on the old 50 ms idle tick. `last_closed` is historical state, not a pending
timer. After logout finalization takes `active`, the next iteration clears
authority before blocking. Starting/Running/Locked/Degraded/Stopping and failed
active records all retain bounded progress; no state-name shortcut is used.

The reply is attempted exactly once before receive, as in the previous
`ipc_reply_and_recv_timeout` helper. A departed caller's failed reply does not
prevent receiving the next request. Under the existing scheduler+IPC shard
serialization, receive checks the pending queue before registering the blocked
receiver; queued messages are consumed and later enqueue wakes the blocked
receiver. The optimization adds no check-then-sleep notification protocol.
Owner tokens, generation checks, authorization and capability rights are unchanged.

New additive `ipc_recv_result` / `ipc_recv_timeout_result` wrappers use the same
syscalls and WouldBlock/yield handoff as existing receive wrappers. The timed
wrapper arms one absolute deadline, never one per retry. Terminal errors remain
errors rather than fabricated empty messages. SessionD accepts expiration only
with an active session; all other receive failures log once and exit. Its existing
manager restart policy handles failure. Kernel fatal-signal delivery remains
independent of IPC and wakes/terminates blocked tasks. No shutdown message,
unbounded cancellation wait, or endpoint refresh loop is introduced.

## Measurement protocol

Before optimization, three steady windows from the initial one-CPU capture
attributed 19.92 wakes/s to SessionD, with 339 receive expirations across those
windows; DialogD and ClipD recorded zero steady wakes. This justified selecting
SessionD. The same capture measured 660 system wakes/s. This is the selection
observation, not the final paired multi-CPU result.
The `selection/` files retain the early observer using VM generations; there
was no task churn in that window. Final comparison images use task publication
incarnations and exact counter deltas and are not pooled with that selection run.

Frozen images and source/hash manifests are retained in `baseline-image/` and
`after-image/`. Captures explicitly use QEMU TCG multi-thread, `-cpu max`, 1024 MiB,
no disk, no NIC, virtio RNG, xHCI USB mouse and no host display. They use the host's
recorded affinity, no imposed host-core pinning or governor changes. Each context
JSON records exact command, accelerator, host platform, QEMU version, host load,
affinity, wall duration and image/serial hashes. Measurements use guest elapsed
counter time; host and guest clocks are not equated.

Each cold boot waits for SunlightD-ready, discards the first three post-ready CPU
reports, and retains the next three windows. Reports are checked on the existing
telemetry cadence, so a nominal five-second window can be about six guest seconds.
The resident boot also reaches login-ready; these are overlapping states of this
profile, not three independent workloads. Graphical desktop/application scenarios
must be labeled separately. A no-state boot leaves the Wise Owl dependent chain
unspawned and must not be used to estimate its enabled costs.

## Results and validation

Resident comparisons use two cold boots per side and six steady windows per
CPU configuration. Each side contributes approximately 31–35 guest seconds per
configuration. Rates below are arithmetic means of the individual window rates.

SessionD results; slash-separated values correspond to 1/2/4 CPUs:

| Metric | Before | After |
|---|---:|---:|
| Idle service wakeups/s | 19.95 / 19.94 / 19.96 | 0 / 0 / 0 |
| Idle service receive expirations | Present in every window | Zero in every window |
| Request latency p50 | Unavailable at useful precision | Unavailable at useful precision |
| Request latency p95 | Unavailable at useful precision | Unavailable at useful precision |
| Request latency p99 | Unavailable at useful precision | Unavailable at useful precision |
| Unique resident memory | Unavailable | Unavailable |
| Mapped user bytes, every boot | 2,236,416 | 2,224,128 |

Mapped user bytes are a virtual mapping count, not private physical RSS. The
12 KiB difference is not evidence of reclaimed service memory. No service was
unloaded. The existing installed-versus-managed memory-domain mismatch from
D3.7 remains unresolved; nominal QEMU RAM is not used as an accounting denominator.

Scheduler-wide observations for the same idle profile, before → after:

| CPUs | Total wakes/s | Context switches/s | Delivered timer IRQs/s | Busy core-ms per guest second |
|---:|---:|---:|---:|---:|
| 1 | 661.14 → 640.89 | 1010.46 → 990.46 | 99.26 → 99.32 | 119.88 → 120.64 |
| 2 | 661.88 → 642.60 | 1196.88 → 1212.15 | 198.75 → 198.61 | 193.06 → 192.93 |
| 4 | 661.28 → 641.08 | 1264.00 → 1256.34 | 397.47 → 397.23 | 284.13 → 271.11 |

The system wake decrease is 20.25/19.28/20.21 wakes/s. The attribution counters
show exactly zero SessionD wakes, receive expirations and charged runtime in
every after window; all its sampled states are BlockedOnIpc. The sum of sampled
task wake deltas reconciles exactly to system deltas in all 36 resident windows,
without truncation or task churn. DialogD and ClipD also have no steady idle
wakes. Full per-service results, including small/zero rates, are in
`idle-summary.json`; the source/contract overlay is `wakeup-inventory.json`.

Other dominant sources remain near 139.4–140.0 wakes/s for SunlightD,
120.5–121.6 for Init, 99 for timer_server, 98.6–99.0 for AudioD and USB mouse,
33 for TTY, and 20 each for Mezzo and PTY. ThermalD observes about three wakes/s:
one sampling timeout plus IPC interactions, not three independent sensor timers.
The measured timer delivery rates remain comparable across the paired boots.

Context-switch changes vary in sign, as do CPU busy-time changes. **No general
CPU utilization, throughput, power or battery saving is claimed.** TCG attributed
busy time includes host scheduling stalls while guest work is current. Even
matching affinity/settings cannot remove host migration, calibration and load
variation from these short observations.

The complete steady diagnostic report takes 8.48–22.15 ms in these resident
observations, including existing CPU/memory/top-task reporting and host stalls,
but excluding its final cost record. That is below 0.45% of a five-second window,
yet it can extend an IRQ-off interval beyond one nominal timer tick. These
instrumented observations support aggregate wake-count attribution; they must
not be used to assert unchanged extreme timer tails or input latency. The first
mapped-page-walk report is excluded by warm-up. No per-event UART output was added.

Precise request latency percentiles, unique resident memory, audio playback quality,
input-to-visible latency and C1.2F timer tail distributions require their own
workloads; absence of such samples is **unavailable**, not a successful timing
check. Existing C1.2F results are historical evidence and are not D3.8-W samples.

### Regression evidence

* Host libraries: IPC 61 tests, kernel memory 35, SessionD 26, dialog protocol 13,
  clipboard 3 and SunlightD core 29. SunlightD's dependency-manager host suite
  contributes 19 additional checks. The focused production scheduler runner
  executes 51 checks, including the C1.2F retained-wake, finite positive-nice
  progress and residual-entitlement regressions. Python scheduler validators
  pass 21 checks; D3.8-W evidence-integrity tests pass six.
* D1/D2/D3 native integration passes on 1/2/4 CPUs (`d3/serial-*cpu.log`).
  RecoveryBlocked after-ready tests also pass on 1/2/4 CPUs (`recovery/`),
  including retained identities, blocked authority and exact reap requirements.
* Native scheduler lifecycle gates pass on 1/2/4 CPUs (`smp/`), with respectively
  5/6/8 deferred reaps, 1/2/4 reused slots and one injected exit race per boot.
  The two-CPU `phase_sec` security/IPC gate passes, including generation-bound
  deadlines, cancellation, endpoint revocation and SMP terminal arbitration.
* The isolated `idle_wakeup_test` image passes on 1/2/4 CPUs (`probe-gate/`).
  Each boot completes 32 SessionD requests separated by 100 ms idle waits, eight
  clipboard summary requests, and 16 malformed SHOW_DIALOG calls from two
  concurrent native clients. It checks explicit receive timeout/invalid endpoint
  errors, stops blocked SessionD and DialogD through their existing SunlightD
  owner, rejects old capabilities, and uses freshly registered endpoints after
  owner-controlled restart. ClipD is never stopped. This is a functional image,
  excluded from all idle-rate comparisons.

The probe's millisecond clock has roughly 10 ms resolution. Its recorded zero
millisecond quantiles mean completion within the same clock tick; they do not
establish submillisecond latency or a TSC-based p99 bound. Precise paired request
latency is unavailable. Idle-rate captures have no audio backend. A separate
two-CPU virtual HDA playback gate passes PCI binding, driver initialization,
DMA start/progress and finite-tone completion (`audio{,.serial}.log`,
`audio-result.json`). Its QEMU `audiodev none` backend cannot establish audible
quality, underrun-free physical playback or paired playback latency.

The existing two-CPU, 4096 MiB `session-foundation` gate **fails with both the
original and optimized SessionD**. Both traces exercise shell start, crash
restart, logout, return to login and another session creation. Neither reaches
the gate's 2 MiB/one-process resource-baseline and idle-CPU acceptance markers;
the driver therefore never reaches its final second-session assertion. Several
TTY markers are also split by separate serial writes. The raw streams do not
isolate the resource delta's cause. Thresholds and validators were not weakened.
See `session-baseline{,.serial}.log`, `session-baseline-result.json` and
`session-foundation{,.serial}.log`. This is an unresolved pre-existing gate
failure, so this report does not claim an entirely green regression suite.

The D3 wrapper's historical summary reports a source-hash mismatch because
observer/probe tooling was edited during that regression run. Its raw native
assertions pass; it is not an immutable-image performance comparison. The final
paired measurement images have separate frozen source/binary manifests. Their
embedded executable hashes differ **only for SessionD** (`image-differences.json`).

The first SMP probe assumed that a start reply implied immediate nameserver
registration. A two-CPU failure exposed that invalid test assumption: lookup's
timeout bounds one RPC. The probe now performs a bounded registration wait;
the failed trace is retained under `rejected/probe-one-shot-lookup/`. An
exploratory graphical run tried the uninstalled dialog CLI; its partial scenes
and logs remain under `rejected/gui-cli-not-installed/` and are excluded from
comparisons. Another rejected controller assumed Alt+F4 closed Welcome; this
shortcut is not implemented. The controller now uses the app's verified Escape
handler; that rejected stream is retained in `rejected/gui-alt-f4/`. These
failures were not hidden by accepting partial captures. A pending dialog keeps
OnTop keyboard focus above a normal Run window, so a Run-based client kill was
never submitted (`rejected/gui-pending-client-command/`,
`rejected/gui-ontop-focus/`). A host mouse-button command also failed to close
the exposed client window; mouse-only attempts and cursor traces are retained in
the rejected graphical runs. A final controller switched to the logged-in TTY,
but its typed `kill` command did not reach the shell and the target process exit
was never observed. That run is retained under
`rejected/gui-tty-kill/`; no graphical idle, cancellation, or client-exit result
is claimed. Thus graphical scene and GUI responsiveness measurements are
unavailable. No host input is sent outside QEMU.

### Code and reproduction

* `services/sunlight-sessiond/src/main.rs`: select indefinite receive only with
  no active session, preserve 50 ms maintenance otherwise, and fail-stop once
  on unexpected receive errors. Native entry/panic handlers are excluded from
  host test builds; the native behavior is unchanged by those test guards.
* `ipc/src/lib.rs`: additive checked receive wrappers and error propagation
  test. Existing DialogD/ClipD receive wrappers are unchanged.
* `kernel/src/{process/mod.rs,sched/mod.rs,ipc/mod.rs,telemetry.rs}`: optional
  per-incarnation counters, deadline snapshots and bounded reports. Scheduling,
  terminal-state arbitration, timer expiry policy and public telemetry ABI are
  unchanged. `kernel/src/main.rs` and `kernel/Cargo.toml` add only the isolated
  opt-in probe image; ordinary kernel startup still has three direct defaults.
* `services/sunlight-startup-fixture`: opt-in native probe; no production unit
  enables it. Ownership audit still reconciles 55 services, three kernel defaults,
  16 Init defaults and 20 manager definitions. The additional test ELF/target
  makes the source census 160 embedded artifacts and 151 Cargo targets.
* `tools/d38w_build.py`, `tools/d38w_idle.py`, `tools/test_d38w_idle.py`: matched
  image builds, bounded captures, GUI interaction and evidence-integrity checks.

The pre-change revision is `e96b9aac45a38e4ff4083d5c3b49499e92b506be`.
Shared production services are built separately, matching `tools/test.sh`;
combining allocator-incompatible service feature sets in one Cargo invocation
fails. The builder restores the optimized source after its baseline build and
preserves a backup if another writer changes the source concurrently.

```sh
# First prepare the repository's userspace artifacts with tools/test.sh sunlightd.
git show e96b9aac45a38e4ff4083d5c3b49499e92b506be:services/sunlight-sessiond/src/main.rs > /tmp/sessiond-before.rs
python3 tools/d38w_build.py --baseline-sessiond /tmp/sessiond-before.rs
python3 tools/d38w_idle.py gate --iso target/d38w-evidence/probe-image/sunlightos.iso --cpus 1 2 4
python3 tools/d38w_idle.py capture --iso target/d38w-evidence/baseline-image/sunlightos.iso --phase baseline --cpus 1 2 4 --repetitions 2
python3 tools/d38w_idle.py capture --iso target/d38w-evidence/after-image/sunlightos.iso --phase after --cpus 1 2 4 --repetitions 2
python3 tools/d38w_idle.py graph --iso target/d38w-evidence/baseline-image/sunlightos.iso --phase baseline --cpus 1 2 4
python3 tools/d38w_idle.py graph --iso target/d38w-evidence/after-image/sunlightos.iso --phase after --cpus 1 2 4
python3 tools/d38w_idle.py summarize
python3 -m unittest discover -s tools -p test_d38w_idle.py
```

Graphical observations use 4096 MiB and their own warm-up periods. They are
compared only to matching graphical profiles, never pooled with the 1024 MiB
resident observations. The initial Welcome window is closed before desktop
sampling. One Terminal remains open for the Terminal and post-activity scenes.
All VM interaction is isolated test input through QMP; no host GUI is launched.

## Remaining high-frequency work

The initial attribution shows about 139.5 manager wakes/s and 121.1 Init wakes/s,
although SunlightD's maintenance timeout is nominally 20 Hz. In this profile,
unavailable dependent endpoints cause repeated nameserver RPCs; replies wake the
manager and requests wake Init. Changing their timeout alone would conflate
maintenance, startup latency, and dependency/recovery semantics. A future bounded
discovery backoff needs its own readiness latency contract and paired workload.

Audio and USB empty-poll paths each observe approximately 98.5 wakes/s; the timer
server receives approximately 99 notifications/s. PTY and Mezzo each observe
approximately 19.9 wakes/s. These are measured attribution, not assertions that
every wake is unnecessary. Brain's busy-yield and graphical UI polling need
enabled-workload measurements and validated multiplexing. No synchronization,
hardware IRQ or scheduler redesign is smuggled into this service optimization.
