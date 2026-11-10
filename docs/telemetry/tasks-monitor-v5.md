# Tasks Monitor and shared telemetry v5

## Transport and compatibility

`SYS_MAP_TELEMETRY` maps a read-only 16 KiB kernel publication. The existing
magic and odd/even sequence protocol are unchanged. Consumers read an even
sequence, copy fields with volatile loads, acquire-fence, then accept only if
the sequence is unchanged. Readers retry at most three times and keep the last
coherent sample. No service IPC or timezone lookup occurs during polling.

Page version **5** preserves every v4 offset and appends:

| Offset | Field | Layout |
| --- | --- | --- |
| 12128 | `process_metrics` | 64 × `ProcessMetricsV5`, 64 bytes each |
| 16224 | `sample_cost_ns` | u64 |

Page size is 16232 bytes, within the existing 16384-byte mapping. Array indices
match `procs` and `task_groups` under the same sequence. Old clients can read
the v4 prefix; new readers accept versions 3–5, read extensions only at the
appropriate version, and reject unknown versions. Private/shared metrics are
unavailable on pre-v5 kernels (zero extension, not a physical measurement).

`ProcessMetricsV5` uses native little-endian repr(C) layout:

| Offset | Field | Type / meaning |
| --- | --- | --- |
| 0 | app_id | 32 bytes, UTF-8 command/application key, zero padded |
| 32 | private_bytes | u64, exclusive UserPrivate physical frames |
| 40 | shared_bytes | u64, distinct mapped physical frames outside Private |
| 48 | mapped_bytes | u64, present lower-half virtual mappings, including aliases |
| 56 | uptime_secs | u32, seconds since task creation, saturating |
| 60 | threads | u32, one per raw task; summed after thread aggregation |

`ProcessSnapshot::record()` returns the stable consumer schema
`ipc::telemetry::ProcessRecordV1` (120 bytes, repr(C)):

| Field | Type |
| --- | --- |
| version | u32 = 1 |
| pid, ppid, state | u32 each |
| app_id, name | 32 bytes each |
| cpu_percent | u32, basis points (10000 = 100.00% of whole-machine capacity) |
| threads | u32 |
| private_bytes, shared_bytes, mapped_bytes | u64 each |
| uptime | u64, seconds |

State in the consumer record is 0 ready, 1 running, 2 blocked, 3 finished.
Raw suspended/timer/I/O blocked states map to blocked. Finished, reaped and
cleanup-pending tasks are excluded from new publications. The fixed table can
publish 64 tasks; ownership analysis includes all live tasks, including those
beyond the visible limit. Hardware topology remains capped at 64 cores.

## Identity and grouping

The kernel's per-PID `Process.name` is the executable basename, cached at
construction and replaced after **successful** exec from the requested command
path. Failed exec retains the old name. This fixes fork/exec children retaining
the launcher name, including multi-call executables invoked through a command
alias. No process name is inferred from a parent. The process-owned cache dies
with its process; exited rows and unused metric slots are cleared. Address-space
generation plus PID separates recycled identities in CPU/history sampling.

`app_id` defaults to that command name. An application can explicitly supply
`SUNLIGHT_APP_ID` in its environment to group differently named UI/backend
executables. This is a cosmetic label, never an authorization identity. Equal
commands group together across launches; this is an application group, not a
unique launch/session ID. PPID remains separately available for parent trees.
Linux threads are aggregated by TGID after computing individual runtime deltas.
Native processes that share an address space remain distinct processes.

## Memory accounting

Every publication walks each live address space once, collects present frame
addresses, and deduplicates aliases. Physical ownership is counted by distinct
process groups, so Linux threads do not multiply ownership and native borrowers
do not each claim the same frame as private. Non-present page-table entries
are not followed. Huge mappings are expanded into their 4 KiB backing frames.

* **Private:** distinct mapped physical UserPrivate frames with exactly one
  process owner. Row/app totals use this measure.
* **Shared:** distinct mapped physical frames outside Private, including shared
  UserPrivate/COW backing, SHM, graphics and kernel-provided read-only mappings.
  A shared frame appears in each mapping process's Shared value. Never sum this
  column as system RAM use.
* **Mapped:** present virtual mapping bytes; aliases count separately. This is
  neither unique physical use nor all reserved virtual address space.

The header uses the existing allocator ownership ledger, not a sum of Mapped:

| Header | Meaning |
| --- | --- |
| Tasks&svc | UserPrivate allocated bytes minus frames now shared by process groups |
| Shared | SHM allocator ownership plus those shared UserPrivate/COW frames, once |
| Kernel | Kernel core + heap + stacks |
| PT | Page-table frames, separate from process memory |
| RAMFS | File data plus metadata when measured |
| Cache | Explicit filesystem and other reclaimable cache ownership, never a residual |
| Gfx/dev | Graphics buffers and device DMA ownership |
| ZRAM | Physical compressed-memory storage; logical bytes are not added |
| Other | Other-accounted plus unclassified ownership |
| Free | Free managed physical frames |

These categories sum to managed RAM within the existing 16-page sampling
conservation tolerance. Installed, reserved and unmanaged RAM are not extra
used categories. Unknown RAMFS metadata uses the existing unavailable sentinel
and contributes zero, not `u64::MAX`. The retained boot image is diagnostic and
is not added again to RAMFS/used memory.

Rows cover **mapped** frames; Tasks&svc also covers allocated UserPrivate frames
not yet mapped, cleanup allocations, and tasks outside the visible limit.
Therefore the visible Private subtotal can be less than Tasks&svc. Row Shared
also includes backing owned by Gfx/dev/Kernel, so it is not equal to the header
Shared bucket. Group Shared/Mapped values are sums of mapping-process views and
can double-count; only group Private is a physical subtotal. Transfer of shared
UserPrivate frames between header Tasks&svc and Shared preserves conservation.

## Sampling, cost and controls

Kernel publication remains once per second. The client sampling/delta window
is configurable with `set_interval_ms` (1–60 seconds, default 1 second).
`set_paused` pauses client sampling/history; resuming rebaselines CPU counters.
It does not stop the global kernel publisher used by other clients. Process
CPU is runtime delta divided by elapsed window × online cores. New/reused PIDs
establish a baseline; missing/exited rows cannot subtract system runtime.

`sample_cost_ns` reports elapsed capture-to-commit publication cost, including
frame collection, sorting, ownership classification and publication. It is
shown in the details panel in microseconds. Divide by the publication period
(and core count for whole-machine percent) for the overhead fraction. This is
elapsed cost, not a separate scheduled telemetry process. Memory work scales
with live mappings; no claim of a measured overhead budget is made without a
VM workload run. The GUI reads shared memory with bounded retries and performs
no blocking telemetry IPC. Per-process CPU and Private histories retain 30
accepted samples and reset on exit/PID reuse.

* Click a column header to sort; repeated click reverses the arrow. Default CPU descending.
* Drag a header boundary to resize; drag its body to another header to swap columns.
  Widths/order persist in `$HOME/.sunlight-tasks.columns` (validated TSK1 format).
* Ctrl+F or click search: match command, PID or app ID, case-insensitively.
  Backspace edits; Esc clears search and cancels termination.
* Click the Flat/Tree/Apps badge or Ctrl+G to cycle grouping; click an app group
  to expand/collapse it. Trees retain parent-before-child order.
* Up/Down select; Enter toggles details; Delete requests termination and a
  second Delete confirms. End task uses the same confirmation flow. PID 0 and
  the monitor itself are protected; system processes require confirmation too.
  A result toast reports signal delivery/failure; exit is verified by telemetry.
* F5 refreshes the current available sample. Ctrl+P pauses/resumes client
  sampling. Ctrl +/- doubles/halves the interval within its limits.
* `SUNLIGHT_TASKS_WARN_PERCENT` and `SUNLIGHT_TASKS_DANGER_PERCENT` configure
  cell colors (0–100, default 50/80). Memory thresholds are percent of managed
  RAM; CPU thresholds are percent of machine capacity. Existing theme warning,
  danger and accent colors are used. Numeric columns align right and IEC units
  display one decimal place.

If mapping fails, the monitor opens an error/empty view. If publications stop
for three configured intervals, it discards the displayed snapshot and shows
Telemetry degraded rather than stale rows. Paused samples are explicitly
marked paused. The status bar shows interval, last accepted update in monotonic
seconds, telemetry health, core count and GPU count.

## Manual validation checklist

1. Boot a v5 kernel and open Tasks Monitor. Verify Telemetry ready, the 1s
   interval, core/GPU counts and live per-core bars. Confirm default CPU sort.
2. Open melody-mina. Inspect both UI/backend PIDs, their own command names,
   PPIDs and app IDs. Switch to Apps, collapse/expand melody-mina, then inspect
   the parent tree. Confirm Linux threads are not duplicate process rows.
3. Open a PDF viewer and a browser tab. Sort Private descending, then ascending.
   Watch at least 30 samples. Private/Shared/Mapped must update as mappings
   change; shared backing must not inflate Private totals.
4. Search by name, PID and app ID. Resize/reorder columns, close/relaunch the
   monitor and verify saved widths/order. Use arrows, Enter, Ctrl+F and F5.
5. Close the PDF viewer, browser tab and melody-mina one at a time. Confirm their
   exited process rows disappear on the next publication and relevant physical
   totals drop. A browser tab may share a live browser process: only exited PIDs
   disappear, and retained browser caches can remain allocated.
6. Check End task/Delete confirmation on a disposable process and cancel on
   init/sunlightd/sunlight-display. Verify success/failure toasts and that stale
   selections do not target a recycled PID. Do not terminate essential services
   during the remainder of this checklist.
7. Pause/resume, change interval, verify CPU rebaselining and retained history.
   Simulate missing telemetry and check the clear degraded state after three
   intervals. Record sample cost at idle and with the three apps open; compare
   against the one-second publication period.

## Changed files

| File | Change |
| --- | --- |
| `ipc/src/telemetry.rs`, `ipc/src/lib.rs` | Shared v5 extension, stable process record, basename and physical-memory helpers with tests |
| `kernel/src/arch/x86_64/syscall.rs` | Refresh the cached command name only after exec succeeds |
| `kernel/src/process/mod.rs` | Cache normalized command name and task creation time |
| `kernel/src/process/address_space.rs` | Visit current present backing frames, including huge mappings; retain legacy page counter |
| `kernel/src/telemetry.rs` | Publish v5, deduplicate physical owners and aliases, clear exited rows, reconcile COW ownership, report sampling cost |
| `sunlight-telemetry/src/lib.rs` | Decode v5, expose stable records, configurable client windows/pause, bounded IPC-free refresh, thread aggregation and regression tests |
| `services/sunlight-tasks/src/main.rs` | Sorting, saved columns, search, groups/tree, histories, thresholds, details, keyboard controls, confirmations/toasts, charts, health and error states |
| `tools/cpu_accounting_capture.py` | Accept the unchanged v4 prefix in v5 publications |
| `docs/telemetry/tasks-monitor-v5.md` | Schema, accounting definitions, controls, limits and manual validation |
