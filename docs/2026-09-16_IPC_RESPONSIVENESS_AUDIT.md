# IPC and responsiveness audit — September 16, 2026

Scope: source audit against the supplied four-core QEMU boot/login/Welcome/terminal log. This is a focused repair, not a claim that all OS performance or lifecycle defects are resolved.

## Confirmed causes repaired

| Finding | Change | Expected effect |
| --- | --- | --- |
| `CONFIGURE_WINDOW` mapped the caller's title SHM page and never released its receiver reference. The caller freed its own reference, leaving the backing page resident. | Release the compositor mapping immediately after copying the title. | Removes the one-page-per-window-title leak consistent with `owned_frames=1` after GUI exit. |
| Notification ingestion retained every received SHM mapping, including malformed-notification paths. | Copy the fixed-size wire object locally and release SHM before validation. | Stops notification backing pages and mapping metadata accumulating in the long-lived compositor. |
| Token-only SHM release selected the oldest view, so a temporary payload map/free could invalidate an existing surface using the same token. | Release the newest view and retain ownership metadata until the last local view is gone. Add a boot regression with 16 owner and 16 peer map/free cycles. | Persistent mappings and peer contents survive temporary payload handling. |
| Launch traces were appended indefinitely, including launches that never produced a window. | Cap history at 128 entries, update existing PID records, and remove records when the last window closes. | Bounds storage and lookup cost; very old diagnostic traces may be evicted. |
| Notification expiry and dead-window/zombie maintenance ran only after IPC receive timed out. | Run maintenance on an elapsed-time check during incoming traffic; retain timeout-driven maintenance and the one-second process sweep cadence. | Continuous client polling/input can no longer indefinitely postpone cleanup. |
| Display queued/delivered keys, pointer edges, and terminal footer updates wrote serial diagnostics during ordinary input. Reaper retries logged while holding the scheduler lock. | Put input traces behind the existing/default-off diagnostic setting and reaper attempt/blocked messages behind `verbose_diag`. Preserve completion summaries and safety checks. | Removes synchronous serial work and key-log formatting from normal input/reaper retry paths. |
| Launch phase records used many separate debug syscalls and were interleaved/split in serial output. | Format each trace in a bounded 256-byte stack buffer and emit it with one syscall; batch Welcome app PASS markers too. | Removes repeated transitions and keeps individual launch records together. |
| Welcome advertised six words while register IPC transports four, truncating its app ID. | Add `SESSION_STARTUP_COMPLETE_V2`: a validated, zero-padded ID of up to 32 bytes occupies exactly four words. Bound endpoint lookup and completion waits. Migrate the known truncated Welcome ID alongside existing migrations. | Completion transmits the full ID without SHM or ABI-wide changes. Legacy completion rejects lengths beyond its actual 16-byte payload. |
| Welcome completion accepted any live PID as a replacement for the launched optional's PID. | Require an exact match with the optional process registered in the active plan. | An unrelated live process cannot complete another app's onboarding policy. |
| Hosted compositor tests invoked native debug syscall 99, which is Linux `sysinfo`, overwriting log buffers. | Make native debug logging a no-op outside `target_os = "none"`. | Host tests no longer corrupt memory when exercising logging paths. |

The reaper's `active_address_space` deferrals are necessary: another CPU may still use the address space. These safety checks were not removed. The log shows eventual reclamation, not proof of a stuck reaper.

## Verification

- Native `cargo check`: kernel, IPC, sessiond, Welcome, display, and terminal passed.
- Host IPC/session library suites: 54 tests passed, including exact register transport of Welcome and a 32-byte identifier, plus malformed payload rejection and bounded launch-log formatting.
- Compositor suite: 69 passed, five existing rendering/hit-test failures. An isolated HEAD baseline with only the hosted debug-log guard produced the same five failures (68 passed; the extra passing test in the working tree is the 10,000-launch bounded-history regression).
- Final `tools/test.sh phase2.6`: passed using its expected two-core configuration. Preserved serial also matches all 25 updated expected markers, including the new temporary-SHM-view alias regression and the final PMM baseline assertion in the SHM self-test. Raw serial: `target/responsiveness-ipc-final-serial.log`. The final ISO is a normal boot build without automatic-login injection.
- Four-core run: 1,000 IPC round trips, bounded queues/backpressure/reuse, notification coalescing, deadline/race/late-reply checks, capability lifecycle, and 64-iteration SMP terminal-state stress passed. Its wrapper failed solely on the expected file's hard-coded two-core boot line. Raw serial: `target/responsiveness-ipc-4cpu-serial.log`.
- Welcome QEMU integration reached `[SESSION-CONFIG] APP_COMPLETION_RECORDED PASS`, `[WELCOME-WIZARD] NO_REPEAT_AFTER_COMPLETION PASS`, and `[SESSION-CONFIG] COMPLETION_RECORDED PASS`. Welcome PID 38 exited with code 0 and was reaped with `owned_frames=0` (the supplied pre-fix log showed one retained frame). Its display instance was also removed. Raw serial: `target/responsiveness-welcome-serial.log`. This proves one complete lifecycle, not an extended memory soak. The legacy gate wrapper failed: its TTY and pre-batching Welcome markers are split across multiple lines, and the expected Wise Owl integration marker is absent. Its timer also emits FINAL before the wizard finishes; its RESOURCE_BASELINE/IDLE_CPU markers are not a quantitative leak/latency measurement.
- `git diff --check` and `bash -n tools/test.sh` passed.

`SUNLIGHT_TEST_SERIAL_LOG=target/<name>.log` now optionally preserves raw serial output from `tools/test.sh` before temporary-file cleanup.

## Remaining work and boundaries

- The general session profile/list IPC helpers still pack IDs beyond four register words. The new completion message fixes Welcome completion; it does not migrate that separate profile-management protocol. It needs a full client/server bulk or chunked transfer design, including sessionctl and TTY callers.
- The SHM virtual-address cursor is monotonic even after unmapping. Releasing references fixes physical-page retention, but sustained map/free churn can eventually exhaust the bounded SHM virtual range and grow page tables. Address reuse needs dedicated collision, rollback, peer-preservation, and SMP tests.
- PTY broker ownership cleanup after abrupt terminal death needs a separate lifecycle pass. The supplied log does not show the shell being reaped with its terminal; the broker has only 16 slots and no periodic owner-death sweep.
- FAT detection, volatile KV storage, startup readiness warnings, and network configuration broadcasts remain separate findings. No storage permissions were weakened to hide them.
- IPC fastpath is explicitly a stub. No implementation or throughput claim was made for it.
- No measured before/after input latency, hours-long soak, or physical hardware validation was performed. Boot completion alone is not evidence for those properties.
