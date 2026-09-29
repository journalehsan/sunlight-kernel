# CPU accounting audit (2026-09-29)

## Phase 1: unchanged implementation

The path is `sched::{account_current_runtime,start_charging_runtime,
effective_runtime_ns}` -> `telemetry::capture_telemetry_snapshot` ->
`commit_telemetry_snapshot` -> shared read-only `TelemetryPage` ->
`sunlight_telemetry::Telemetry::poll/compute_cpu_usage` -> GUI
`services/sunlight-tasks` and TUI `sunlight-top` (which reexports the reader).

* Task `cpu_ticks` is actually accumulated **nanoseconds**, using calibrated TSC
  `interrupts::now_ns` with a 100 Hz monotonic tick fallback. It is not a dispatch
  count. Switch-out commits elapsed runtime; switch-in starts a new interval.
* Effective runtime includes an open interval only for `Running`, although a
  task can be marked blocked/ready before its context actually leaves the CPU.
  IPC blocking paths also stop charging before the eventual context switch.
* Total CPU is the sum of deltas for tasks present in both reader samples,
  divided by sample elapsed nanoseconds times the published CPU count. New,
  exited, reaped, and entries beyond the 64-task table are omitted. Idle is
  simply the complement, with no independent idle measurement.
* Per-task percentages already use total-machine normalization (one full core
  on four CPUs is 25%). Linux thread-group members are separate task rows.
* Per-core load is a stale uniprocessor placeholder: core 0 gets aggregate
  usage and all APs get zero. Timer IRQ counters are real per-core counters.
* `CSw` is in the **core** table, not a task counter. It counts lifetime
  dispatches from idle and changes to a different task, excludes same-task
  reselection, but also excludes transitions from a task to idle. It has no
  voluntary/involuntary split.
* The reader fetches `sample_time_ns` again after its sequence validation, so
  numerator and denominator can belong to different publications. Its retry
  loop is unbounded, and redundant PID caches do not use process generation.
* Task/core rows truncate basis points to integer percentages; overview keeps
  more precision. The reader filters Finished but misreads Reaped as Ready.
* Telemetry is triggered by `ticks_total % 100 == 0`. Global ticks are derived
  from monotonic time, so a delayed IRQ can skip the exact publication tick.

## Runtime candidates requiring measurement

The September 28 audio change retains bounded DMA catch-up and blocking
`ipc_recv_timeout`, not a yield-until-ready loop. The timer IRQ wakes
timer_server at 100 Hz and periodically wakes tty_server. Scheduler idle paths
return a saved/synthetic `sti; hlt` context. A high dispatch rate alone proves
neither CPU saturation nor an audio regression. None of these policies should
be changed without measured runtime/wakeup evidence.

The old total necessarily omits some execution; the identified reader flaws
alone do not establish why the reported utilization increased. An independent
per-CPU idle/busy measurement is required before making that claim.

## Findings from instrumenting the unchanged workload

The original four-core TCG login workload measured essentially **100% busy**
while the old task-sum display reported approximately 61–65% (Task Monitor
screenshot: 65.10%). Thus the apparent increase was not just a display bug:
the old display actually **underreported** busy time. The immediate reschedule
IPI added to `process_yield` in `31d661b65` made existing receive/yield polling
loops execute much more frequently. The responsiveness improvement is retained.

Measured offenders, isolated by staged rebuilds:

| Task/path | Evidence and bounded correction |
| --- | --- |
| mezzo, sessiond, sunlightd | Repeated nonblocking receive/yield; now receive with a 50 ms maintenance deadline. Incoming IPC wakes immediately. |
| niced, gcd | Same runnable polling; now block for their existing one-second maintenance cadence. |
| USB mouse | Empty hardware-event poll repeatedly yields; now block at most 10 ms after an empty poll. Nonempty queues still drain immediately. No hardware event-wait syscall exists. |
| memorydb failed startup | `PersistenceUnavailable` on the diskless guest entered a permanent yield loop. It now blocks on a private endpoint, preserving failure and restart semantics. |
| tty_server | Idle receive/drain/yield loop; now receive with a 10 ms maintenance deadline. Input still wakes immediately. |
| shared GUI idle wait | Yielded until the next input poll deadline, consuming 14–17% machine CPU in Vortex alone and tens of thousands of scheduler invocations/sec. Now parks on a private endpoint until the same deadline. |
| Welcome | Redrew the static page on every Tick. Normal Tick no longer requests a repaint; explicit automation and input still do. |

The staged login measurement fell to about 2–3% busy. Desktop rendering adds
real work under software emulation, especially with Task Monitor or playback
repainting. It is included in CPU usage, not subtracted or hidden. Audiod's
bounded DMA catch-up and timed IPC receive were already appropriate and are
unchanged. No audio-buffer, quantum, burst-score, run-queue, or yield-IPI tuning
was made. Other unexercised yield loops remain audit candidates, not proven bugs.

## Final accounting architecture

* Each task has committed `cpu_runtime_ns` and an optional monotonic
  `cpu_charge_since`. Blocking/yield checkpoints do not stop execution charging;
  actual descheduling and exit do. Dispatch starts an interval. Migration
  commits on the source before starting on the destination under the scheduler
  lock. An open interval is sampled even if the task is already marked blocked.
  The old `last_start_ns` remains exclusively the existing burst-policy clock.
* Per-CPU halt intervals are measured independently of task state. Local,
  IRQ-disabled writers enter immediately before `sti; hlt`; maskable interrupt
  entry closes idle before handler/scheduler work. BSP, AP, synthetic idle,
  process-exit, and fault-exit paths share the measured idle loop. The audit
  initially missed the private exit trampoline; that instrumentation error was
  corrected before final measurements.
* `busy = elapsed - idle`. Busy includes kernel/IRQ work outside task execution
  intervals. Consequently the visible task sum need not equal system busy:
  short-lived tasks and the 64-row table limit also affect the visible sum.
  Exiting/truncated tasks never erase system busy time.
* Telemetry ABI v4 appends per-core counters and task-group IDs after the v3
  prefix: 12,128 bytes, within the existing variable-size read-only mapping.
  All task clocks use one publication timestamp. Each independent CPU idle
  snapshot has its own timestamp and an eight-attempt seqlock bound; contention
  retains the previous CPU sample rather than inventing idle time.
* Reader timestamp and counters are copied within the same sequence check.
  Three retries bound a preempted writer; no new publication retains the last
  sample. New task identities, PID generations, first samples, and CPU-count
  changes establish a baseline. Reaped rows are excluded and all blocked states
  are mapped correctly. Publication uses an elapsed deadline rather than exact
  modulo equality, so delayed timer IRQs cannot skip an entire reporting period.
* System utilization is `sum(delta busy) / sum(delta CPU elapsed)`; this is
  `busy / (T*N)` with the small inter-CPU snapshot skew explicitly accounted
  for. Core utilization is that core's busy/elapsed. Idle is the rounded
  complement, so Processor + Idle is exactly 100% at displayed precision.
* Process rows use **total-machine capacity**, `sum(thread runtime deltas) /
  (sample elapsed * online CPUs)`. One full core on four CPUs is 25%; four are
  100%. Linux thread groups aggregate before rounding; native tasks use their
  PID. Shared address-space memory is not summed per thread. ABI v3 fallback
  retains its explicitly legacy task-sum total and has no per-core runtime.
* Counter arithmetic saturates; percentage/rate math uses integer u128 outside
  scheduler hot paths. Nanosecond u64 capacity lasts roughly 584 years. The
  existing synchronized/calibrated TSC clock is used, with the existing 100 Hz
  monotonic fallback. The fallback's short-interval resolution is necessarily
  coarser. No calendar time enters CPU arithmetic. Tiny positive work is shown
  as `<0.1%`, ordinary work with one decimal; it is never forced to zero.

## Context switches and optional diagnostics

The GUI's core column now shows `CSw/s`. Lifetime `context_switches` remains
in the public snapshot: actual task-to-different-task, idle-to-task, and
task-to-idle transitions, including exit; same-task selection does not count.
There is no voluntary/involuntary split and no per-task switch counter.

Build `sunlight-kernel --features cpu_accounting_diag` to enable per-core
scheduler-invocation, successful wakeup, and IPC-wakeup counters and bounded
five-second serial reports (at most 120 reports). Wakeups are attributed to the
CPU **issuing** the wake, not its eventual destination. IPC wakeups include
reply, receive, and timeout completion; they are not hardware IRQs. Timer IRQs
are actual per-core handler counts. HDA currently uses DMA-progress polling,
not a wired audio IRQ, so audio diagnostics use the existing audiod progress,
frame, and underrun logs rather than a fabricated interrupt rate.

Normal builds omit these optional event increments and serial reports. Always-on
runtime/idle accounting uses fixed storage, integer arithmetic, no allocation,
and no hot-path logging. The pre-existing periodic telemetry snapshot allocation
is not new instrumentation. NMIs do not write idle clocks; NMI handling inside
a halt interval is a small known idle-accounting limitation.

`tools/cpu_accounting_capture.py` reads the same ABI via QMP without pausing the
guest. It checks sequence consistency and emits bounded JSONL samples with
capacity, busy/idle, per-core load, rates, and top task deltas. Pair `--kernel`
with the exact ELF embedded in the running ISO, not a later rebuild. Example:

```sh
python3 tools/cpu_accounting_capture.py --socket /tmp/cpu-audit-qmp.sock \
  --kernel target/cpu-audit-validated.elf --seconds 35 \
  --label desktop-idle --output target/cpu-audit-desktop.jsonl
```

The initial baseline logger predates the IPC terminal-wakeup hook: baseline
wakeup counts undercount replies/timeouts and must not be treated as directly
comparable to final wakeup rates. Baseline switch counts also omit task-to-idle,
although this is negligible in its continuously runnable workload.
