# `sunlight-terminal` Multi-Tab Support

This documents the tab architecture added on top of the terminal-rendering
work in `docs/terminal/graphical-terminal-audit.md`. That audit fixed *what*
gets drawn for a single session (`sunlight_tty::TerminalGrid` instead of the
old `Console`); this patch is about *how many* sessions a terminal window can
run at once and how the UI/input/PTY plumbing is shared between them.

All code referenced below lives in `services/sunlight-terminal/src/main.rs`
unless noted otherwise.

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
  `TerminalApp`, and calls `spawn_tab()` to create the first tab ("Tab 1")
  before the window is even created. If either step fails the process yields
  forever rather than opening a broken window (same failure style as before
  this patch).
- **New tab** (`TerminalApp::spawn_tab`): opens a fresh `PtySession`
  (`PtySession::open_with`), spawns `/bin/sshl<pty_id>` against it
  (`spawn_shell`), and inserts the resulting `TerminalTab` via
  `insert_tab`. The per-tab shell id is derived from the *PTY session id*
  (not the terminal's own pid), so concurrent tabs never collide on
  `/bin/sshl<id>`. Failure at any step (no PTY capacity, spawn failure) is a
  no-op — existing tabs are left untouched.
- **Close tab** (`TerminalApp::close_tab`): removes the tab from the array
  (`remove_tab`) and then calls `TerminalTab::close`, which signals the
  shell (`libc::kill(pid, SIGTERM)`, only if it's still `Running`) and
  releases the PTY session (`PtySession::close`, i.e. `PtyMsg::CLOSE`). This
  is the *same* teardown a single-session terminal would use — no second
  shutdown path was invented for tabs.
- **Last tab is un-closeable**: `close_tab` is a no-op when `tab_count == 1`
  (it returns `false` before touching the tab at all, so no PTY/process
  syscalls happen). There is therefore always at least one tab open; closing
  the terminal requires closing the *window* (title bar or `Ctrl+W`), not a
  tab. `handle_click`/`draw_tab_bar` hide the `x` button entirely on a lone
  tab so this is visible rather than a silent no-op.
- **Switch tab** (`switch_tab`/`next_tab`/`prev_tab`): changes `active` and
  clears the new active tab's `dirty` flag. Input and rendering are always
  scoped to `tabs[active]` only (see `App::update`/`App::view`).

`insert_tab`/`remove_tab` are intentionally pure array bookkeeping (no
syscalls) split out from `spawn_tab`/`close_tab` specifically so they can be
unit tested — see [Testing](#testing).

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

To make these possible, `#![no_main]`, the `#[global_allocator]`, and
`#[panic_handler]`/`_start` are all gated with `#[cfg(not(test))]` (the crate
stays `#![no_std]` unconditionally), mirroring the existing pattern in
`services/sunlight-display/src/main.rs`. `TerminalTab::poll_pty` was split
into a thin IPC wrapper plus a pure `ingest(bytes, ...)` step, and
`spawn_tab`/`close_tab` were split into pure `insert_tab`/`remove_tab`
bookkeeping plus the real PTY/process calls, specifically so the array
bookkeeping and byte-routing logic could be tested without a live PTY or
process (`PtySession::open_with/read/write/close`, `spawn_shell`,
`libc::kill`, `libc::try_waitpid` all perform real syscalls that don't exist
on a host target and are not exercised by these tests).

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

1. Launch `sunlight-terminal` — confirm exactly one tab ("Tab 1") appears.
2. Click `+` to open a second tab — confirm it becomes active immediately.
3. Run a different command in each tab (e.g. `echo one` in Tab 1, `echo two`
   in Tab 2) and switch between them — confirm each tab shows only its own
   output.
4. Run `top` (or `sunlight-top`) in one tab and a normal shell command in the
   other — confirm `top` keeps updating only while/after its own tab is
   selected, and the other tab's content is never overwritten by `top`'s
   output.
5. Click a tab's `x` — confirm the other tab remains alive and its shell
   keeps running.
6. Close tabs down to the last one — confirm the last tab's `x` button
   disappears entirely and neither clicking where it was nor pressing
   `Ctrl+Shift+W` closes it or the window.
7. With 2+ tabs open, press `Ctrl+Shift+W` — confirm it closes the *active*
   tab (same teardown as the `x` button) and switches focus sensibly, and
   confirm plain `Ctrl+W` still closes the whole window immediately
   regardless of how many tabs are open.
8. Close the terminal window directly (title-bar close) with multiple tabs
   open — confirm all tabs' PTYs/processes are cleaned up
   (`shutdown_all_tabs`).
9. Confirm minimize/restore still works (this patch does not touch window
   chrome handling).
10. Confirm no regression in terminal rendering, colors, or cursor handling
    versus the state described in `graphical-terminal-audit.md` (run `top`,
    `ls --color`, etc. and compare).
11. Open a second `sunlight-terminal` window (or any other app) and confirm
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

## Future work

- Tab rename (manual or via OSC title escapes).
- OSC `0`/`2` title escape support.
- Drag-to-reorder tabs.
- Split panes.
- Session restore across terminal restarts.
- Full resize correctness (`TIOCGWINSZ`/`TIOCSWINSZ`/`SIGWINCH`), tracked
  separately per `graphical-terminal-audit.md`'s recommended next work.
- Either fix host `cargo test` for bin-only crates, or extract the pure tab
  logic (`TerminalTab`/`TerminalApp` bookkeeping) into a small `[lib]`
  target so its tests can run the same way `sunlight-fs`/`sun-font`'s do.
