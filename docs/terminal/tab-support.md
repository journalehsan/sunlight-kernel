# `sunlight-terminal` Multi-Tab Support

This documents the tab architecture added on top of the terminal-rendering
work in `docs/terminal/graphical-terminal-audit.md`. That audit fixed *what*
gets drawn for a single session (`sunlight_tty::TerminalGrid` instead of the
old `Console`); this patch is about *how many* sessions a terminal window can
run at once and how the UI/input/PTY plumbing is shared between them.

All code referenced below lives in `services/sunlight-terminal/src/main.rs`
unless noted otherwise.

> **Update:** the first version of this patch created a tab by making the
> `pty` `CREATE`/`SET_MODE` calls (and the shell spawn) synchronously, inline,
> inside the click/shortcut handler, using the plain unbounded
> `sunlight_ipc::ipc_call`. That version hung the *entire* terminal window
> (not just the new tab) whenever the `pty` service didn't answer instantly.
> See [Root cause: the new-tab hang](#root-cause-the-new-tab-hang) below for
> the analysis and the fix that shipped instead (a small tick-driven state
> machine using bounded `ipc_call_timeout`). Everything else in this document
> (ownership model, keyboard shortcuts, PTY routing, titles) is unchanged by
> that fix.

## Root cause: the new-tab hang

**Symptom:** launching the terminal and using its single starting tab was
fine (window visible in well under a second). Clicking `+` (or `Ctrl+T`) to
open a *second* tab froze the whole window — no redraws, no keyboard input,
not even to the already-running first tab — until the process was killed.

**Why:** `sunlight-ui`'s event loop (`Window::run_with` in
`sunlight-ui/app.rs`) is a single synchronous loop per window:
`poll_event → App::update → (redraw)`. There is no background thread. Any
call made from inside `App::update` that never returns therefore freezes
*that entire loop* — polling, input, and rendering for every tab, not just
the one being created.

The original tab-creation path (`TerminalApp::spawn_tab`, called directly
from the tab-bar click handler) did exactly that: it opened a new
`PtySession` with `PtySession::open_with`, which issued `PtyMsg::CREATE` and
`PtyMsg::SET_MODE` via `sunlight_ipc::ipc_call` — the **unbounded** IPC
primitive that retries forever on `WouldBlock` and has no timeout. Contrast
this with `Window::poll_event` and `Window::connect`'s launch-trace call in
the very same crate, which both correctly use `ipc_call_timeout`. `pty`
session creation was the one blocking, multi-round-trip lifecycle operation
in this app that had *not* been given a bound.

In other words: this was a **lifecycle/state-machine bug**, not a resource
leak or a data race. Tab creation was modeled as "do three blocking things in
a row on the UI thread and hope they all return quickly" instead of as an
explicit state machine with bounded steps and a failure state. The first
tab's bootstrap (in `_start`, before the window/event loop even exists)
happened to mask this because there is nothing else competing for `pty`
attention that early — which is exactly why the launch path measured as
healthy while the *second* tab was the one that could wedge the process.

**Fix:** tab creation is now a small explicit state machine
(`SpawnStep`/`PendingSpawn`), advanced one bounded step per `Event::Tick`
by `TerminalApp::advance_pending_spawn`:

1. `TerminalApp::spawn_tab` (the click/shortcut entry point) performs **no
   IPC and no process spawn at all**. It only allocates an in-memory
   `TabStatus::Connecting` placeholder tab, activates it, and queues a
   `PendingSpawn`. This is what makes tab creation non-blocking from the
   UI's perspective — the tab visibly appears in the tab bar on the very
   next frame regardless of how the `pty` service behaves afterward.
2. Each tick, `advance_pending_spawn` attempts exactly one step (`CREATE`,
   then `SET_MODE`, then spawn the shell) using
   `sunlight_ipc::ipc_call_timeout` with a `SPAWN_STEP_TIMEOUT_MS` (40 ms)
   budget instead of the unbounded `ipc_call`. A timeout just retries next
   tick; the render/input loop keeps running in between — the worst case is
   a ~40 ms stall on a single frame, not an unbounded freeze.
3. An overall `SPAWN_DEADLINE_MS` (1.5 s) wall-clock budget bounds the whole
   sequence. Exceeding it — or an outright rejection from the `pty` service
   — flips the tab to `TabStatus::Failed`, a normal, visible, closable tab
   state, instead of hanging.
4. Closing a tab whose spawn is still in flight (`TerminalApp::close_tab`)
   cancels the `PendingSpawn` and releases any partially-created PTY session
   so it isn't leaked in `pty_server`.

The very first tab (`_start`, pre-window) drives the *same* state machine in
a tight bounded loop before the window is created, so the happy-path timing
is unchanged, but a stuck `pty` service can no longer prevent the window
from ever appearing either.

### Phase log

Every step above is logged through `log_tab_phase` (monotonic timestamp via
`sunlight_ipc::monotonic_millis`, tagged with the numeric tab id) to the
serial debug log:

```
tab_create_clicked → tab_state_allocated → tab_focused → pty_request_sent
  → pty_created → shell_spawn_requested → shell_spawned
  → tab_attached_to_pty
(and, from the render side) first_tab_frame
(on failure, instead of the remaining steps) tab_create_failed
```

Format: `[TERM][TAB] tab=<id> phase=<phase> t=<monotonic_ms>ms`.

#### Before (synchronous, unbounded — hangs)

With the old `ipc_call`-based `spawn_tab`, a slow/unresponsive `pty` service
produced logs like this (note there is no way to observe *where* it got
stuck beyond "system log stops"; the process never returns to any logging
point after `pty_request_sent`, since `ipc_call` retries silently forever):

```
[TERM][TAB] tab=2 phase=tab_create_clicked t=812ms
[TERM][TAB] tab=2 phase=tab_state_allocated t=812ms
[TERM][TAB] tab=2 phase=pty_request_sent t=813ms
<window frozen here forever — no further output, no redraws, no input>
```

#### After (state machine, bounded — recovers)

Same scenario with the fix in place — the deadline trips and the tab is
marked `Failed` instead of hanging the window; input and rendering never
stop:

```
[TERM][TAB] tab=2 phase=tab_create_clicked t=812ms
[TERM][TAB] tab=2 phase=tab_state_allocated t=812ms
[TERM][TAB] tab=2 phase=tab_focused t=812ms
[TERM][TAB] tab=2 phase=pty_request_sent t=812ms
[TERM][TAB] tab=2 phase=first_tab_frame t=814ms
[TERM][TAB] tab=2 phase=tab_create_failed t=2314ms
```

And the healthy/normal case (what you'll actually see in practice —
everything resolves within a tick or two, not the full deadline):

```
[TERM][TAB] tab=2 phase=tab_create_clicked t=4021ms
[TERM][TAB] tab=2 phase=tab_state_allocated t=4021ms
[TERM][TAB] tab=2 phase=tab_focused t=4021ms
[TERM][TAB] tab=2 phase=pty_request_sent t=4021ms
[TERM][TAB] tab=2 phase=first_tab_frame t=4023ms
[TERM][TAB] tab=2 phase=pty_created t=4023ms
[TERM][TAB] tab=2 phase=shell_spawn_requested t=4023ms
[TERM][TAB] tab=2 phase=shell_spawned t=4024ms
[TERM][TAB] tab=2 phase=tab_attached_to_pty t=4024ms
```

Grep serial output for `tab=<id>` to reconstruct one tab's full lifecycle
timeline, or for `phase=tab_create_failed` to find any tab that hit the
deadline.

## Phase 1 — audit of the pre-tab code

Before this patch:

- The tab strip drew a single hard-coded label ("Tab 1"). It was purely
  decorative — there was no `TerminalTab` type, no array of sessions, and no
  click handling on the strip at all.
- There was exactly one `PtySession`, one `ModelGrid`, one `Footer`, and one
  `OscParser`, all held directly as fields of `TerminalApp`.
- Keyboard input (`Event::KeyPress`/`Event::Key`) went straight to that one
  session via `handle_raw_key`/`handle_char` (the same methods `TerminalTab`
  has today — they were simply inherent methods on `TerminalApp` before).
- Mouse input was not routed anywhere inside the window content area (the
  window frame's close/minimize buttons are handled entirely by
  `sunlight-display`, outside this app).
- Rendering (`App::view`) drew the fixed tab-bar label, then the single
  session's `TerminalViewport` and footer.
- Shutdown was whatever fell out of `window.run()` returning — the process
  did not call `ProcessExit::exit`, so it looped rather than exiting
  cleanly, and there was no explicit PTY/process cleanup at all.

This is the baseline the tab model below replaces.

## Ownership model

```
TerminalApp
├── tabs: [Option<TerminalTab>; MAX_TABS]   // compacted at the front
├── tab_count: usize
├── active: usize                           // index into `tabs`
├── next_tab_id: TabId
├── pty_cap: CapabilityToken                 // shared `pty` service capability
├── read_buf / console_buf                   // shared scratch I/O buffers
├── debug: DebugFlags
└── mods: Mods                               // tracked Ctrl/Alt state

TerminalTab (one per open tab)
├── id: TabId
├── title: [u8; TAB_TITLE_MAX]
├── pty: PtySession           // this tab's own PTY session
├── shell_pid: u64            // this tab's own shell process
├── grid: ModelGrid           // this tab's own sunlight_tty::TerminalGrid
├── footer: Footer            // this tab's own prompt/line-editor/history
├── osc: OscParser            // this tab's own OSC parser state
├── status: TabStatus         // Running | Exited
└── dirty: bool               // background output not yet seen
```

`pty_cap` and the scratch I/O buffers (`read_buf`/`console_buf`) are the only
state shared across tabs, and neither carries session identity: `pty_cap` is
just the resolved `pty` service endpoint (sessions are distinguished
server-side by `PtySession::id`), and the buffers are reused sequentially,
one tab at a time, inside `poll_all_tabs`/`poll_pty` — nothing persists
between tabs in them. There is no shared/global terminal or PTY singleton.

## Tab lifecycle (Phase 3)

- **Startup**: `_start` resolves the `pty` capability once, constructs
  `TerminalApp`, calls `spawn_tab()` to allocate the first tab ("Tab 1"),
  then drives `advance_pending_spawn()` in a bounded loop until it resolves
  (`Running` or `Failed`) before the window is created. If the `pty`
  capability itself can't be resolved at all, the process yields forever
  rather than opening a broken window (same failure style as before this
  patch) — but a *slow* `pty` service can no longer do that (see
  [Root cause](#root-cause-the-new-tab-hang)).
- **New tab** (`TerminalApp::spawn_tab` + `TerminalApp::advance_pending_spawn`):
  `spawn_tab` synchronously allocates a `TabStatus::Connecting`
  `TerminalTab` (no PTY/shell yet) and activates it — no IPC, so it can never
  block. `advance_pending_spawn`, called once per `Event::Tick`, then opens a
  fresh `PtySession` (`PtySession::create_timeout` + `set_mode_timeout`,
  bounded by `SPAWN_STEP_TIMEOUT_MS` each) and spawns `/bin/sshl<pty_id>`
  against it (`spawn_shell`), one bounded step at a time, attaching the
  result to the tab via `attach_tab` once the shell is actually running. The
  per-tab shell id is derived from the *PTY session id* (not the terminal's
  own pid), so concurrent tabs never collide on `/bin/sshl<id>`. Failure or
  timeout at any step flips the tab to `TabStatus::Failed` (see
  [Root cause](#root-cause-the-new-tab-hang)) rather than leaving existing
  tabs — or the window — in a bad state.
- **Close tab** (`TerminalApp::close_tab`): removes the tab from the array
  (`remove_tab`) and then calls `TerminalTab::close`, which signals the
  shell (`libc::kill(pid, SIGTERM)`, only if it's still `Running`) and
  releases the PTY session (`PtySession::close`, i.e. `PtyMsg::CLOSE`,
  bounded by `CLOSE_TIMEOUT_MS`) if one was ever created. If the closed tab
  had a spawn still in flight, `close_tab` also cancels the `PendingSpawn`
  and releases any partially-created PTY session. This is the *same*
  teardown a single-session terminal would use — no second shutdown path was
  invented for tabs.
- **Last tab is un-closeable**: `close_tab` is a no-op when `tab_count == 1`
  (it returns `false` before touching the tab at all, so no PTY/process
  syscalls happen). There is therefore always at least one tab open; closing
  the terminal requires closing the *window* (title bar or `Ctrl+W`), not a
  tab. `handle_click`/`draw_tab_bar` hide the `x` button entirely on a lone
  tab so this is visible rather than a silent no-op.
- **Switch tab** (`switch_tab`/`next_tab`/`prev_tab`): changes `active` and
  clears the new active tab's `dirty` flag. Input and rendering are always
  scoped to `tabs[active]` only (see `App::update`/`App::view`) — a
  `Connecting`/`Failed` tab simply has no `PtySession` to route input to
  (`TerminalTab::handle_char`/`handle_raw_key` no-op when `pty` is `None`),
  so keystrokes typed while a background tab is still connecting never leak
  anywhere.

`insert_tab`/`remove_tab` are intentionally pure array bookkeeping (no
syscalls) split out from `spawn_tab`/`close_tab` specifically so they can be
unit tested — see [Testing](#testing). `spawn_tab` itself is also pure/
syscall-free by construction now (see above), which is what lets its
regression tests run directly on the host.

## UI behavior (Phase 4)

The tab bar is a small custom renderer (`TerminalApp::draw_tab_bar`) rather
than the existing `sunlight-ui::widgets::TabBar`, because that widget has no
notion of per-tab close buttons or a dirty/activity indicator. It draws, left
to right:

- One fixed-width (`TAB_W = 92px`) slot per open tab, showing the title
  (`F_SMALL`), an accent underline when active, a small accent dot when
  inactive-and-dirty, danger-colored text when the tab's shell has exited,
  and an `x` close button — hidden while it's the only open tab (see
  "Last tab is un-closeable" above).
- A trailing `+` new-tab button (`NEW_TAB_W = 34px`, centered text), hidden
  once `MAX_TABS` is reached. Widened from an earlier 26px so it's a more
  comfortable click target.

`TerminalApp::handle_click` hit-tests these in order (close buttons, then
tab bodies, then the new-tab button) against `Event::Click`; clicking a tab
body switches to it (`switch_tab`). Everything else (drag-reorder,
detachable tabs, split panes, animations, session restore) was intentionally
left out per the task's non-goals.

## Keyboard shortcuts (Phase 5)

| Shortcut | Action |
|---|---|
| `Ctrl+Tab` | next tab |
| `Ctrl+Shift+Tab` | previous tab |
| `Ctrl+T` / `Ctrl+Shift+T` | new tab |
| `Ctrl+Shift+W` | close the active tab (no-op if it's the last tab) |
| `Alt+1`..`Alt+9` | jump to tab N (best-effort) |

`Ctrl+Shift+W` required a small, deliberately narrow `sunlight-display`
change: that compositor globally intercepts `Ctrl+W` to close the focused
window, and previously did so *regardless* of Shift, so `Ctrl+Shift+W` never
reached this app either. The interceptor's `KEY_W` branch now also requires
`!shift`, so:

- Plain `Ctrl+W` still force-closes the focused window immediately,
  everywhere, exactly as before this patch.
- `Ctrl+Shift+W` is left unconsumed and is forwarded to the focused app like
  any other keypress. `sunlight-terminal` is the first (and currently only)
  app that binds it, to close the active tab.

This is the one exception to "no `sunlight-display` changes" from the
original tab-support patch — it was accepted as a follow-up specifically
because it's additive (nothing that worked before stops working) and scoped
to a single modifier check.

### Input-stack limitation

`sunlight-ui`'s `Window::poll_event` resolves a pressed key to `Event::Key(char)`
whenever the keyboard driver produced an ASCII value, and that ASCII value is
computed independently of the Ctrl modifier (only Shift is factored in). So
`Ctrl+T`/`Ctrl+1..9` cannot be observed as an accurately-tagged event the way
`Ctrl+Tab` can (Tab has no ASCII mapping, so it always falls through to
`Event::KeyPress`, which *does* carry full modifier bits).

To still offer `Ctrl+T` and `Alt+1..9`, `TerminalApp` tracks `ctrl`/`alt`
state itself (the `Mods` struct) from every `Event::KeyPress` it observes
(including presses of the modifier keys themselves) and consults that
tracked state when a plain `Event::Key(ch)` arrives. This is best-effort: if
a modifier key-up event is ever dropped by the input stack, the tracked state
can desync until the next `KeyPress` resynchronizes it. A correct fix would
mean changing the shared `sunlight-ui` event translation to always tag
`Event::Key` with modifiers, which affects every GUI app, not just the
terminal, and was out of scope here.

Regular input (including `Ctrl+C`) is unaffected by any of this: it is only
consulted for the exact character/keycode combinations listed in the table
above; everything else still reaches `TerminalTab::handle_char`/
`handle_raw_key` → the active tab's PTY, exactly as before.

## PTY/output routing (Phase 6)

`TerminalApp::poll_all_tabs` iterates every open tab once per `Event::Tick`:

1. `TerminalTab::refresh_status()` — non-blocking `libc::try_waitpid` to flip
   `status` to `Exited` if the shell died.
2. `TerminalTab::poll_pty()` — reads one round from *that tab's own*
   `PtySession` into the shared scratch buffer, then calls
   `TerminalTab::ingest()`, which feeds the bytes through *that tab's own*
   `OscParser` and `ModelGrid`. No other tab's state is ever touched by this
   call.
3. If the tab produced output: the active tab causes a redraw; a background
   tab instead sets its own `dirty = true` (and still causes a redraw, so the
   tab-bar's activity dot appears promptly).

`App::view` only ever draws `tabs[active]`'s grid/footer — inactive tabs keep
accumulating state in their own `ModelGrid` in the background, and that state
is simply what gets rendered the moment the user switches to them
(`switch_tab` does not need to do anything special to "catch up" the
rendering — the grid is already current).

### `pty_server` capacity

`pty_server` enforces a system-wide `MAX_SESSIONS = 8` (see
`services/pty_server/src/main.rs`). `MAX_TABS` here is capped at **6** to
leave headroom for other PTY consumers (e.g. a second terminal window, `tty`
sessions) rather than letting one terminal window exhaust the whole system's
PTY budget. `PtyMsg::CLOSE` fully releases a session slot for reuse
server-side, so closing tabs (rather than just leaking them) is what keeps
this budget usable across a long-running session.

## Titles (Phase 7)

Tabs default to `Tab N` where `N` is the tab's `TabId` (a monotonically
increasing counter, not a position — closing "Tab 2" does not cause "Tab 3"
to be renamed). Exited tabs are visually distinguished by drawing their title
in the theme's danger color rather than by changing the text itself (no
`" exited"` suffix), to keep `TAB_TITLE_MAX` (20 bytes) headroom for future
OSC-derived titles.

OSC window-title (`OSC 0`/`OSC 2`) support is **not implemented** — this is a
deliberate follow-up, not an oversight. `OscParser`/`parse_osc` already exist
per tab (used today for prompt/app-mode detection) and would be the natural
place to add it.

## Rendering (Phase 8)

`App::view` draws, top to bottom: the tab bar, the active tab's
`TerminalViewport` (using the same fixed `CONTENT_COLS`/`CONTENT_ROWS` as
before — no per-tab or dynamic geometry), then the footer/status area. This
is unchanged from the pre-tab rendering pipeline other than being scoped to
`tabs[active]` instead of a single field. Window resize behavior was not
touched: geometry is still the same fixed `WIN_W`/`WIN_H`/`CONTENT_COLS`/
`CONTENT_ROWS` as before this patch.

## Testing (Phase 9)

### Unit tests

`services/sunlight-terminal/src/main.rs` has a `#[cfg(test)] mod tests` at
the bottom covering the state that doesn't require a live PTY/process:

- `insert_tab_creates_first_tab`
- `insert_tab_adds_additional_tabs_and_activates_newest`
- `insert_tab_respects_max_tabs_capacity`
- `switch_tab_changes_active_and_clears_dirty_flag` (also exercises
  `next_tab`/`prev_tab`)
- `remove_tab_shifts_later_tabs_and_reindexes_active`
- `remove_tab_out_of_range_is_a_no_op`
- `removing_last_tab_reaches_zero_tabs`
- `close_tab_is_a_no_op_on_the_last_remaining_tab`
- `ingest_routes_pty_output_into_its_own_grid`
- `inactive_tab_output_does_not_leak_into_other_tabs`
- `translate_special_key_maps_tab_to_tab_byte`

Regression coverage added for the new-tab hang fix (see
[Root cause](#root-cause-the-new-tab-hang)) — these are the tests that would
have caught the bug, because they assert `spawn_tab` never touches IPC/PTY/
process syscalls directly:

- `spawn_tab_allocates_connecting_placeholder_without_any_blocking_call` —
  asserts `spawn_tab` synchronously produces a `TabStatus::Connecting` tab
  with `pty`/`shell_pid` still `None` and a queued `PendingSpawn` in the
  `RequestPty` step, all without any syscall.
- `spawn_tab_is_a_no_op_while_a_spawn_is_already_pending` — a second
  click/shortcut before the first tab finishes connecting must not start a
  second concurrent spawn.
- `spawn_tab_respects_max_tabs_capacity` — unchanged behavior, re-verified
  against the new allocate-then-advance split.
- `closing_a_tab_with_a_pending_spawn_cancels_it` — closing the tab a spawn
  was targeting clears `pending_spawn` (this is what prevents a leaked
  `pty_server` session / stuck state if the user closes a still-connecting
  tab).
- `closing_an_unrelated_tab_leaves_a_pending_spawn_intact` — closing a
  *different* tab while one spawn is in flight does not disturb it, even
  though `remove_tab` shifts array indices.
- `advance_pending_spawn_is_a_cheap_no_op_when_nothing_is_pending` — the
  per-tick check when idle costs nothing and touches no syscalls.

To make these possible, `#![no_main]`, the `#[global_allocator]`, and
`#[panic_handler]`/`_start` are all gated with `#[cfg(not(test))]` (the crate
stays `#![no_std]` unconditionally), mirroring the existing pattern in
`services/sunlight-display/src/main.rs`. `TerminalTab::poll_pty` was split
into a thin IPC wrapper plus a pure `ingest(bytes, ...)` step, and
`spawn_tab`/`close_tab` were split into pure `insert_tab`/`remove_tab`
bookkeeping plus the real PTY/process calls, specifically so the array
bookkeeping and byte-routing logic could be tested without a live PTY or
process (`PtySession::create_timeout`/`set_mode_timeout`/`read`/`write`/
`close`, `spawn_shell`, `libc::kill`, `libc::try_waitpid` all perform real
syscalls that don't exist on a host target and are not exercised by these
tests). `spawn_tab` itself went further: it was restructured so the entire
click/shortcut-handler path is syscall-free (see
[Root cause](#root-cause-the-new-tab-hang)), which is precisely what lets
its regression tests assert on real (not simulated) production behavior.

Run with:

```
cargo test --target x86_64-unknown-linux-gnu -p sunlight-terminal
```

**Known environment issue:** in this workspace, that command currently
compiles cleanly but the resulting test *binary* panics before running any
test (`this long-format option was given no name`, inside the vendored
`getopts` crate used by the test harness). This reproduces identically for
`services/sunlight-display`'s pre-existing bin-crate tests (`cargo test
--target x86_64-unknown-linux-gnu -p sunlight-display`), and does **not**
reproduce for library-crate tests in this workspace (e.g. `cargo test -p
sun-font` passes normally) — so it is a pre-existing limitation of running
`cargo test` against `[[bin]]`-only crates on this host toolchain, not
something introduced by this patch. Fixing it (e.g. by upgrading the host
toolchain, or by moving the testable logic into a `[lib]` target the way
`sunlight-fs`/`sunlight-net` do) is out of scope here and is listed under
Future Work below.

### Manual test plan

**Core hang regression (this is the scenario reported as broken):**

1. Launch `sunlight-terminal` from the dock — confirm the window appears in
   well under a second, with exactly one tab ("Tab 1"), and it is fully
   interactive immediately (typing works).
2. Click `+` three times in a row (or press `Ctrl+T` three times) to create
   three more tabs — confirm **each** click:
   - shows the new tab in the tab bar on the very next frame (briefly as
     "Connecting...", per `TabStatus::Connecting`, then live), and
   - never freezes the window — you should be able to keep clicking/typing
     immediately after each click, with no perceptible stall.
3. Switch between all four tabs (click each tab, and via `Ctrl+Tab`/
   `Ctrl+Shift+Tab`) — confirm each tab shows only its own output and the
   active tab visibly changes every time.
4. Type a distinct command into each tab (e.g. `echo one` .. `echo four`)
   and confirm keystrokes always land in the *currently focused* tab only —
   switching tabs mid-typing must never cause a keystroke to appear in the
   wrong tab's line editor.
5. Close the middle tab (tab 2 or 3) via its `x` button — confirm the
   remaining tabs (including the one to its right) shift left by one, stay
   alive with their shells still running, and focus lands on a sensible
   remaining tab.
6. Immediately create another new tab (`+` or `Ctrl+T`) — confirm this
   succeeds exactly like step 2 (instant appearance, no hang), which is the
   specific "close then create again" sequence that would surface any
   leaked/stale state from the previous close.
7. Check the serial debug log (`debug_log`/QEMU serial output) for the
   `[TERM][TAB] tab=<id> phase=...` lines described in
   [Root cause](#root-cause-the-new-tab-hang) — confirm every tab created in
   steps 2 and 6 reaches `tab_attached_to_pty` (not stuck at
   `pty_request_sent`, and not `tab_create_failed`) within a couple of
   ticks.

**Failure-path check (optional, requires faking a stuck/absent `pty`
service, e.g. by not starting `pty_server` or killing it mid-session):**

8. Attempt to open a new tab with no working `pty` service — confirm the tab
   reaches `TabStatus::Failed` (danger-colored title, "Failed to start
   shell" message) within `SPAWN_DEADLINE_MS` (1.5 s) instead of hanging,
   and confirm the rest of the window (existing tabs, input, redraws) stays
   fully responsive throughout and afterward. Confirm the failed tab can
   still be closed with `x` like any other tab.

**Existing coverage (unchanged by this fix, still worth re-checking):**

9. Run `top` (or `sunlight-top`) in one tab and a normal shell command in
   another — confirm `top` keeps updating only while/after its own tab is
   selected, and the other tab's content is never overwritten by `top`'s
   output.
10. Close tabs down to the last one — confirm the last tab's `x` button
    disappears entirely and neither clicking where it was nor pressing
    `Ctrl+Shift+W` closes it or the window.
11. With 2+ tabs open, press `Ctrl+Shift+W` — confirm it closes the *active*
    tab (same teardown as the `x` button) and switches focus sensibly, and
    confirm plain `Ctrl+W` still closes the whole window immediately
    regardless of how many tabs are open.
12. Close the terminal window directly (title-bar close) with multiple tabs
    open — confirm all tabs' PTYs/processes are cleaned up
    (`shutdown_all_tabs`).
13. Confirm minimize/restore still works (this patch does not touch window
    chrome handling).
14. Confirm no regression in terminal rendering, colors, or cursor handling
    versus the state described in `graphical-terminal-audit.md` (run `top`,
    `ls --color`, etc. and compare).
15. Open a second `sunlight-terminal` window (or any other app) and confirm
    plain `Ctrl+W` still force-closes whichever window is focused — the
    `sunlight-display` change only added a `Shift` exception, it did not
    change the un-shifted binding.

## Known limitations

- Window/content resize is still fixed-geometry; this was explicitly
  deferred (see module doc comment and non-goals below).
- `Ctrl+T`/`Ctrl+Shift+W`/`Alt+1..9` detection relies on locally-tracked modifier state and
  can theoretically desync from real key state if a key-up event is dropped
  by the input stack (see the "Input-stack limitation" section above).
- `MAX_TABS = 6`, bounded by both `pty_server`'s system-wide `MAX_SESSIONS =
  8` and this app's bump allocator (no per-tab deallocation of grid/footer
  scratch memory across the app's lifetime — closing a tab frees its *slot*
  in the `tabs` array, but the bump allocator itself never reclaims the
  heap bytes that tab's `ModelGrid`/`Footer` used).
- No OSC window-title support yet — titles are always `Tab N`.
- `cargo test` for this crate's unit tests currently cannot execute in this
  environment due to a host-toolchain/test-harness issue affecting all
  bin-only crates (see Testing above); the tests do compile and are
  reviewable as-is.
- `PtySession::read`/`write` (the per-keystroke/output hot path, used every
  `Event::Tick` and on every keypress) still use the unbounded `ipc_call`,
  matching the pre-existing single-tab behavior. Only the tab *lifecycle*
  (create/close, i.e. the reported hang) was moved onto bounded,
  tick-driven primitives — broadening the same treatment to steady-state
  read/write was judged a larger change than this fix required (see
  [Root cause](#root-cause-the-new-tab-hang)) and is listed here as
  follow-up work rather than folded into this patch.
- Only one tab creation can be in flight at a time (`spawn_tab` is a no-op
  while `pending_spawn` is `Some`); rapid repeated clicks/`Ctrl+T` presses
  are simply ignored until the current one resolves, rather than being
  queued.

## Future work

- Tab rename (manual or via OSC title escapes).
- OSC `0`/`2` title escape support.
- Drag-to-reorder tabs.
- Split panes.
- Session restore across terminal restarts.
- Extend the bounded-timeout treatment from tab creation/close to
  `PtySession::read`/`write` for full protection against a stuck `pty`
  service mid-session, not just during tab lifecycle events.
- Real Linux signal-frame delivery for `SIGWINCH`; renderer/PTTY geometry and
  `TIOCGWINSZ` are now dynamic, while `TIOCSWINSZ` is explicitly frontend-owned.
- Either fix host `cargo test` for bin-only crates, or extract the pure tab
  logic (`TerminalTab`/`TerminalApp` bookkeeping) into a small `[lib]`
  target so its tests can run the same way `sunlight-fs`/`sun-font`'s do.
