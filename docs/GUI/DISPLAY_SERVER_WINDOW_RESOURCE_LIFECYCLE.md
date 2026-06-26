# SunlightOS Display Server / Compositor — Window Resource Lifecycle Strategy

**Status:** Design document for Day 21 GUI hardening  
**Date:** 2026-06-26  
**Scope:** `sunlight-display` (compositor), SGP clients, sunlight-ui, windowed apps (sunlight-terminal, sunlight-tasks, sunlight-runner, eyes, etc.)  
**Philosophy:** Align with SunlightOS microkernel/capability style — explicit ownership, deterministic cleanup, small trusted core, no silent leaks, graceful degradation instead of cascading failure.

---

## 1. Current Strategy

The compositor must **not** be restarted simply because window memory or window slots fill up.

- Restart-on-failure is a **supervisor fallback** only. It applies when the compositor process itself has crashed or become unrecoverable (e.g. via sunlightd / service supervisor policy).
- Normal resource pressure (too many windows, large framebuffers, leaked clients) is handled by **window lifecycle cleanup**.
- Each window owns its own SHM-backed framebuffer (capability-style). The compositor holds the authoritative reference(s). Clients hold mapping rights.
- Closing a window must release its resources promptly so that slots and memory become reusable without restarting the display service.

Restarting the entire graphical session is a heavy hammer and loses user state. It is unacceptable as the primary response to ordinary resource exhaustion.

---

## 2. Window Lifecycle

A typical window follows this sequence:

1. **CREATE_WINDOW**  
   Client requests a new window (width, height, config flags, initial title hint, pid/ppid context).

2. **Allocate WindowEntry**  
   Compositor creates internal tracking state for the window.

3. **Allocate or attach SHM framebuffer**  
   Compositor creates (or accepts) a shared-memory region sized for the client area. The capability token is returned to the client.

4. **Assign win_id**  
   A unique identifier is minted and returned to the client. The client uses this for all subsequent calls.

5. **Add to z_order**  
   The window is inserted into the stacking order (respecting `z_index_type` OnTop vs Normal, and insertion rules for dialogs/widgets).

6. **Focus / raise if needed**  
   New top-level application windows are typically raised and become focused (subject to policy). Desktop widgets and minimized windows do not take focus.

7. **Lifetime operations**  
   - `EVENT_POLL` — client blocks or polls for input (mouse, keys, etc.).
   - `COMMIT_FRAME` — client signals that its SHM buffer is ready for composition.
   - `CONFIGURE_WINDOW` — title, state (min/max), flags, etc. are updated.
   - Drag / resize gestures are handled inside the compositor (no client SHM mutation required for geometry).

8. **CLOSE_WINDOW / DESTROY_WINDOW**  
   - Client explicitly requests close, or  
   - User action (title bar close button, Ctrl+W on focused window), or  
   - System policy (process exit cleanup, quota enforcement).

9. **Remove from z_order**  
   The window is removed from the compositor's stacking list.

10. **Release display-side references**  
    - Drop the SHM capability held by the compositor for this window.
    - Unmap / invalidate any internal buffer pointers.
    - Cancel any active drag/resize state associated with the window.

11. **Release or mark window slot reusable**  
    The `WindowEntry` (or slot) is returned to a free pool. With generation IDs this slot may be reused for a future window.

12. **Choose next focused window**  
    Focus policy selects a new focused window (typically the new topmost focusable window in the same z-group). If no focusable windows remain, focus becomes "none".

13. **Mark screen dirty / composite remaining windows**  
    The compositor redraws the desktop background plus all remaining windows and the cursor.

The lifecycle is intentionally synchronous and explicit. There is no garbage collector for windows.

---

## 3. Deallocation vs Compaction

**Deallocation is required.**

- Every `CLOSE_WINDOW` / `DESTROY_WINDOW` path must free the `WindowEntry` (or mark the slot free) and drop the associated SHM capability.
- Display-side buffer references must be released so the underlying physical pages can be reclaimed by the kernel when the last mapping/capability disappears.

**Slot reuse is required.**

- A naïve ever-growing `Vec<Window>` or monotonically increasing ID space will eventually exhaust practical limits even if total live memory is modest.
- Reusing slots (with proper safeguards) keeps the active set small.

**Display-side cleanup is required.**

- The compositor must not leak SHM objects it created or was given.
- Dropping the compositor's `CapabilityToken` for a window's buffer is the signal that it no longer needs the mapping.

**Compaction is not required at this phase.**

- Each window has its own independent SHM / capability-style buffer.
- There is no single contiguous "compositor heap" of window pixels that needs to be slid around.
- Moving live window buffers would be complex, racy with client rendering, and provides little benefit given the current design.

**Compaction may be revisited later** only if a future allocator design stores many window buffers inside one large contiguous pool where internal fragmentation or address-space pressure becomes a real problem. That is out of scope for the current SHM-per-window model.

---

## 4. Failure Policy

If `CREATE_WINDOW` cannot allocate resources (SHM object, WindowEntry slot, etc.):

- The call must **fail gracefully**.
- The compositor **must remain alive**.
- The client receives an error reply (no window id, no capability).
- The client can then decide how to react (show a dialog, fall back, exit, etc.).
- The desktop environment may later surface a visible memory / resource warning.
- **Restarting the compositor is not the resource management strategy.**

OOM or "too many windows" must be treated as an ordinary, recoverable condition for a long-running display service, just like a full filesystem or too many open files in a traditional OS.

---

## 5. Immediate Implementation Checklist

- [x] Add SgpMsg::CLOSE_WINDOW / DESTROY_WINDOW
- [x] Store owner_pid in WindowEntry
- [x] Remove window from z_order on close
- [x] Release SHM/cap references on display side
- [ ] Reuse WindowEntry slots with generation id
- [ ] Add per-process and global window memory quota
- [ ] Add process-exit cleanup hook
- [ ] Add displayctl windows/memory debug command
- [ ] Keep compositor restart only as supervisor fallback

### Day 21 Implementation Notes

- Implemented now:
  - Added `SgpMsg::CLOSE_WINDOW` as the client-facing alias for the existing `0xA104` close/destroy opcode.
  - `sunlight-ui` `Window::drop` now sends `CLOSE_WINDOW` before `shm_free(self.shm_cap)`, with best-effort failure handling.
  - `sunlight-display` now handles close through a shared idempotent cleanup path used by client close, title-bar close, and `Ctrl+W`.
  - Close removes the window from z-order, cancels active drag state for that window, updates focus to the next remaining focusable window by current stacking order, and redraws the scene.
  - `owner_pid` is now recorded from the IPC badge during `CREATE_WINDOW`.
  - The display server explicitly calls `shm_free` on its owned window SHM token during close.
- Intentionally deferred:
  - generation-id slot reuse
  - per-process and global quotas
  - process-exit cleanup hook
  - `displayctl` diagnostics
  - compositor restart policy changes
  - compaction
- SHM ownership assumption:
  - Current Day 21 cleanup assumes the normal client-driven path is `CLOSE_WINDOW` followed immediately by client-side `shm_free`.
  - Forced-close hardening for stale clients and process-exit coordination is still deferred.
- TODOs for hardening:
  - Use `owner_pid` for process-exit-driven window reap.
  - Revisit forced-close SHM revocation semantics once process-exit cleanup exists.

---

## 6. Phase Split

### Phase 1: Do now

Core correctness for normal close paths. The compositor must stop leaking windows on user close.

- Add `CLOSE_WINDOW` / `DESTROY_WINDOW` handling (ensure both client-driven and internal paths exist and are exercised).
- Store `owner_pid` in the window tracking structure if the caller pid is already available through the current IPC path (passed at CREATE time or obtainable from the endpoint).
- Remove the window from z_order on close.
- Mark / free the `WindowEntry` (or equivalent) on close.
- Update focus after close (choose the new topmost focusable window).
- Mark the screen dirty and composite the remaining scene.

### Phase 2: Hardening

Make reuse safe and bound resource consumption.

- Introduce generation id for reused `WindowEntry` slots so that a stale `win_id` cannot accidentally address a newer window that reuses the same slot.
- Add per-process and global window memory quotas. Enforce at CREATE time; return a specific error when a client or the system is over quota.
- Add a process-exit cleanup hook: when a process dies, the display service (or a broker that notifies it) walks that pid's windows and forcibly destroys them. `sunlight-gcd` process_exited notifications are a natural integration point.

### Phase 3: Diagnostics / supervisor integration

Observability and policy at the system level.

- Add `displayctl windows` and `displayctl memory` (or equivalent) subcommands that list live windows, their owners, sizes, and aggregate usage.
- Document and (where missing) implement supervisor restart policy for `sunlight-display` so that only true crashes, not resource pressure, cause a restart. Record the reason for any restart.

---

## 7. Suggested SGP Message Contract

Core messages (client ↔ display_server):

- `CREATE_WINDOW` — request a new window. Returns win_id + SHM capability + geometry info on success, or an error.
- `CONFIGURE_WINDOW` — update title, state (min/max/fullscreen), flags, etc. after creation.
- `EVENT_POLL` — blocking (or timeout) receive of input events for a specific window.
- `COMMIT_FRAME` — client declares that its SHM buffer now contains valid pixels for composition.
- `CLOSE_WINDOW` — client requests normal close / destroy of a window it owns.

`DESTROY_WINDOW` may be used as the internal opcode or as an alias. From the client's perspective, `CLOSE_WINDOW` expresses intent.

**CLOSE_WINDOW must be idempotent and robust:**

- Closing an already-closed window is harmless (no-op or benign error).
- An invalid `win_id` must not crash the compositor.
- Only the owner (matching pid recorded at creation) or a privileged system entity should be allowed to close a window. Unprivileged cross-process close attempts are rejected.

This contract keeps the protocol small and capability-oriented: possession of the win_id plus matching ownership is the right to operate on the window.

---

## 8. Safety Invariants

These invariants must hold at all times:

- z_order must never contain closed / destroyed windows.
- The focused window (if any) must be alive.
- The compositor must not dereference dead framebuffer pointers or use dropped SHM capabilities.
- The close path must be safe even if the client process has already died (the compositor may receive a close request or a process-exit notification after the client is gone).
- Out-of-memory on window creation must not kill the compositor.
- After generation-id phase: a window ID must not accidentally refer to a newer window that reused the same slot. The generation must be part of the checked identity on every operation.

Violations of these invariants are considered compositor bugs.

---

## 9. Testing Plan

Manual / interactive checks (run under the GUI session):

- Open the Run dialog, close it, open it again — window slot is reused cleanly.
- Launch `eyes` from Run, close the eyes window — resources are released.
- Open and close many small windows in succession — no growth in compositor memory or live window count.
- Alt+Tab after closing the focused window — focus moves to a remaining window without crashing.
- Drag a window, then close it while still dragging or after releasing — drag state is cancelled cleanly.
- The compositor keeps running after repeated create/close cycles.
- The task monitor (`sunlight-tasks`) still updates and shows correct process list while windows come and go.
- Stress (later): 1000 create/close cycles from a test client or script while watching memory and `next_win_id` behavior.

Automated expectations in `tools/test.sh` should continue to pass. If serial output changes due to new debug, update `EXPECTED` messages.

---

## 10. Answer to Reviewer

**"What happens if the display service resources fill up? Do we restart the compositor on failure, or do we have deallocation/compaction for closed windows?"**

We do **not** restart the compositor for normal resource pressure. Restart-on-failure is only a last-resort supervisor policy when the display process has genuinely crashed.

For ordinary operation we rely on explicit window lifecycle deallocation:

- `CLOSE_WINDOW` / `DESTROY_WINDOW` removes the window from z_order.
- Display-side SHM capabilities and buffer references are released.
- Window slots are marked reusable (with generation IDs in Phase 2 to prevent use-after-reuse).
- Focus is updated to the next live window.
- The screen is marked dirty and remaining windows are recomposited.

Because each window owns an independent SHM buffer (capability style), we do not use a single movable packed heap. Therefore compaction is unnecessary for the current architecture and is not part of the plan. It would only be reconsidered if future work placed many window pixels inside one shared contiguous pool.

`CREATE_WINDOW` fails gracefully on allocation failure or quota breach. The client receives an error. The compositor stays up. A higher-level desktop component can surface a friendly resource warning later.

Process exit cleanup (Phase 2) will forcibly reap a deceased process's windows using owner_pid tracking and process_exited notifications.

In short: we deallocate and reuse. We do not reboot the desktop to clean up after closed windows.

---

**End of document.**
