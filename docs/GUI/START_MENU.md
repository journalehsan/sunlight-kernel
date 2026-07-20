# SunlightOS Start Menu

**Status:** Implemented (dark theme only), MVP.
**Owner module:** `services/sunlight-vortex-shell/src/start_menu.rs`
**Driven by:** `services/sunlight-vortex-shell/src/main.rs` (`VortexShell`)

## Summary

The Vortex Shell's dock "grid" icon (bottom-center cluster, leftmost icon)
now opens a real Start Menu overlay instead of being a decorative no-op. The
menu is a self-contained, dark-themed panel anchored above the dock, with
search, a pinned-apps row, a full "All Apps" grid, a recent/suggested row,
and a footer with session/power actions.

This replaces the previous "flat launcher" mental model (a small fixed dock
with no search or grouping) with a structured, searchable app menu. The dock
remains the quick-launch strip for pinned apps — currently **Files, Terminal,
Calendar, Calculator, Edit, Writer, Rappid Rabbit** (plus the Start Menu grid
button) — while the Start Menu owns discovery of everything else.

## Architecture

- **`start_menu.rs` is UI-only.** It owns the Start Menu's view model
  (`StartMenuState`), pure layout computation (`compute_layout`), and
  drawing. It never touches IPC, process launching, or ACPI/power syscalls
  directly — it only *reports* what happened via `StartMenuAction`
  (`Launch(AppId)`, `Unavailable(name)`, `Power(action)`, `None`).
- **`main.rs` interprets actions.** `VortexShell::apply_start_menu_action`
  is the single place that turns a `StartMenuAction` into a real effect:
  - `Launch(app_id)` reuses the exact same `handle_app_click` /
    `launch_app` / app-registry machinery the bottom dock already uses
    (via `LaunchSource::Shell`), so launch semantics (duplicate-launch
    guarding, running/minimized activation, launch tracing) are shared,
    not duplicated.
  - `Unavailable(name)` shows an info notification ("Coming soon").
  - `Power(Sleep)` shows a notification (no kernel S3 support yet).
  - `Power(Restart)` / `Power(Shutdown)` call `sunlight_libc::power::reboot()`
    / `shutdown()`, which wrap the kernel's `PowerCtl` syscall (see
    `docs/ACPI_IMPLEMENTATION.md`). These never return on success.
- **Layout is a pure function** of `(screen_w, screen_h, search_query,
  recent_apps)` — `compute_layout()` is called fresh on every draw *and*
  every input event, so hit-testing is never stale (no cached rects from a
  previous frame to get out of sync).
- **Toggling:** `VortexShell.launcher_zone` is the dock grid icon's rect,
  captured each frame from `draw_bot_center`'s return value. Clicking it
  calls `StartMenuState::open_menu()` when closed, or closes the menu when
  already open. While open, `VortexShell::update()` routes all
  pointer/keyboard events straight into `StartMenuState::handle_event()`
  before any other shell input handling runs. Outside clicks/presses close
  the menu and are consumed so the same gesture does not leak through and
  accidentally activate the desktop, dock, or other shell chrome below.
  `Event::Tick` deliberately still falls through to the normal path so
  background app-registry polling keeps the menu's running-app badges
  fresh even while it's open.

## Layout (dark theme MVP)

Bottom-left-anchored panel, positioned directly above the dock (same 10px
gap convention as the desktop icon area), roughly 600px wide:

1. **Header** — "SunlightOS" title on the left, close ("x") button on the right.
2. **Search bar** — "Search apps, files, settings...", focused automatically
   on open.
3. **Content** (one of two modes):
   - **Not searching:** three sections, top to bottom:
     - **Pinned** — a fixed row of 6 core apps (Terminal, Files, Calculator,
       Settings, Task Manager, Sunlight Mines).
     - **All Apps** — a 4-column grid covering the full catalog (12 tiles:
       11 real apps + 1 placeholder).
     - **Recent** (or **Suggested** when there's no session history yet) —
       up to 6 tiles.
   - **Searching:** a single "Results" grid (same 4-column tile layout)
     filtered live from the full catalog; "No matches" is shown when empty.
4. **Footer** — a placeholder user row (avatar + "User" / "SunlightOS") on
   the left, three power buttons (Sleep / Restart / Shut Down) on the right.

Tiles show a subtle accent-bordered highlight for the keyboard-selected
item and a background tint on mouse hover. Tiles for apps that are
currently Running/Minimized get a small accent bar under the icon, matching
the dock's own running-indicator convention.

## App catalog & grouping

Defined in `start_menu.rs` as `APP_CATALOG` (12 entries) plus
`DEFAULT_PINNED` (6 `AppId`s) and `SUGGESTED_RECENT` (3 `AppId`s, the
no-history fallback).

Real, launchable apps (share `AppId` with the dock's existing registry):

| App           | `AppId`      | Icon                              |
|---------------|--------------|------------------------------------|
| Terminal      | `Terminal`   | `apps/48/utilities-terminal.tga`   |
| Files         | `Files`      | `apps/48/org.kde.dolphin.tga`      |
| Calculator    | `Calculator` | `apps/48/galculator.tga`           |
| Settings      | `Settings`   | `apps/48/preferences-system.tga`   |
| Task Manager  | `Tasks`      | `apps/48/ksysguard.tga`            |
| Sunlight Bench| `Bench`      | `apps/48/cpu-x.tga`                |
| Sunlight Mines | `Mines`     | `apps/48/bomber.tga`               |
| Sunlight Writer | `Writer`   | `apps/48/libreoffice-writer.tga`   |
| Text Editor   | `TextEditor` | `apps/48/kate.tga`                 |
| Sunlight Calendar | `Calendar` | `apps/48/office-calendar.tga`    |
| Rappid Rabbit | `RappidRabbit` | `apps/48/internet-web-browser.tga` |
| Sunlight Mines | `Mines`     | `apps/48/bomber.tga` (or bundle icon) |

Placeholder tiles (no backing binary yet; `CatalogId::Placeholder(slug)`,
`available: false`). Clicking one shows a "Coming soon" notification
instead of launching, and the tile renders dimmed with a small "Soon" tag:

| App           | Slug            | Icon                                   |
|---------------|-----------------|------------------------------------------|
| Photo Viewer  | `photo-viewer`  | `apps/48/accessories-image-viewer.tga`   |

`TextEditor`, `Writer`, and `RappidRabbit` are also pinned on the bottom dock
(see `DOCK_PINNED` in `main.rs`). `Tasks`, `Bench`, `ApiLab`, and `Mines`
remain Start-Menu / running-strip only. All share the same launch/state-sync
logic (`sync_app_registry`, duplicate-launch guarding, window activation).

`DEFAULT_PINNED` is a static list today (no persistent user pinning yet —
see Future Ideas).

## Recent / Suggested behavior

`VortexShell.recent_apps: Vec<AppId>` is a session-only, in-memory MRU list
(newest first, capped at `MAX_RECENT_APPS = 6`), updated by
`open_app_from_ui()` (which calls `note_recent_app`) for **every** UI
launch surface. It is:

- **Not persisted** across shell restarts.
- **Real data**, not mocked — it reflects launches from the Start Menu,
  dock pins, desktop icons, and context-menu opens. Dock Terminal and
  Start Menu Terminal therefore share the same Recent list and the same
  launch/focus policy (`handle_app_click` / `launch_app`).
- **Falls back to `SUGGESTED_RECENT`** (Files, Terminal, Settings) when
  empty, and the section label changes from "Recent" to "Suggested"
  accordingly (`StartMenuLayout::recent_is_real`).

## Search behavior

- **Scope today:** app name + category, case-insensitive substring match
  over the full `APP_CATALOG` (including placeholders, so users can find
  out a video player is "coming soon"). No file or settings-entry search
  yet.
- Typing filters results live (`compute_layout` re-filters on every
  keystroke).
- `Enter` launches the keyboard-selected result (or the first result if
  none is explicitly selected).
- Arrow keys `Up`/`Down` always move the selection; `Left`/`Right` move the
  text cursor while the search field is focused, or move the selection
  otherwise.
- `Escape` clears the search query first; a second `Escape` (or one press
  with an empty query) closes the menu.
- **Future:** settings entries, file/document search (see Future Ideas).

## Keyboard & mouse UX

- Opening the menu focuses the search field automatically.
- Clicking outside the panel, the header close button, or pressing `Escape`
  (with an empty search query) closes the menu. Outside dismissal is
  intentionally consumed to avoid click-through bugs; clicking the dock
  grid icon while the menu is open only closes it (toggle-off).
- Clicking a tile launches it (or shows "coming soon" for placeholders);
  clicking a power button arms/executes it (see below).
- Mouse hover tints a tile's background; keyboard selection is tracked
  separately and only draws an accent border after explicit keyboard
  navigation, so the menu does not open with a permanent highlight stuck
  on the first tile.
- `Tab`-based focus traversal is not implemented (not needed given the
  search-first UX); documented here as a possible future addition.

## Interaction notes for this patch

- The close button and outside-click dismissal both use the existing
  `StartMenuState::close()` path; no second shell-only close path was added.
- App tiles map directly to shared `AppId` values (`CatalogId::App(AppId)`),
  and `main.rs` still launches them exclusively through
  `VortexShell::handle_app_click()` / `launch_app()`.
- Hover state is stored as `StartMenuState.hover`; keyboard selection is
  stored separately as `StartMenuState.selected: Option<usize>`.
- Search behavior itself is intentionally unchanged in this patch.

## Power / session actions

Footer has three buttons: **Sleep**, **Restart**, **Shut Down**.

- **Sleep** is inert today (no kernel S3/suspend support — see
  `docs/ACPI_IMPLEMENTATION.md`); clicking it shows a notification instead
  of doing anything destructive, so it never needs confirmation.
- **Restart** / **Shut Down** are destructive and never return once issued,
  so they require a *confirm click*: the first click arms a 3-second
  confirmation window (`CONFIRM_WINDOW_MS`) and the button's label swaps to
  "Confirm?" with a danger-colored border; a second click on the same
  button within that window actually issues the call
  (`sunlight_libc::power::reboot()` / `shutdown()`, i.e. the kernel
  `PowerCtl` syscall). Letting the window expire, clicking elsewhere, or
  closing the menu cancels the pending confirmation.
- Lock / Log out / Switch user are not implemented (no session-manager
  concept exists yet in SunlightOS) — see Future Ideas.

## Performance & stability notes

- `compute_layout()` is cheap (a few dozen `Rect` computations and small
  `Vec` allocations over a 12-item catalog) and is intentionally
  recomputed on every draw/event rather than cached, to avoid any
  stale-geometry class of bugs — this was measured as negligible next to
  the shell's existing per-frame work (icon theme lookups, dock button
  drawing).
- Icon bytes are embedded via `include_bytes!` and parsed
  (`TgaImage::parse`) on demand per tile per draw; parsing is header-only
  and cheap (no decode until a pixel is actually blitted), so no icon
  cache was added for this MVP.
- No IPC calls happen while the menu is open except the ones the shell
  already made before it opened (`sync_app_registry` on `Tick`, unrelated
  to the menu itself) and the terminal ones triggered by launching an app
  or a power action.
- Binary size impact: ~12 additional embedded 48px/16/32px TGA icons
  (a few hundred KB total); acceptable for this feature's scope.

## Known limitations

- **No scrolling.** The MVP layout is sized to fit its fixed content (12
  catalog tiles, 6 pinned, 6 recent) without scrolling. On very short
  screens the panel's top edge is clamped to stay below the top bar, which
  means the *bottom* of the panel may slightly crowd the dock instead of
  scrolling — this is a deliberate simplicity trade-off, not a crash risk.
  A future version should add `Canvas::sub_canvas`-based clipping plus a
  scroll offset if the catalog grows meaningfully.
- **No full-screen dim/scrim behind the panel.** Skipped for simplicity and
  per-frame cost (would require per-pixel `blend_pixel` over the whole
  screen); the panel's own border/shadow-less flat style was judged
  sufficient for a first version.
- **Recent apps are session-only** (see above) — no persistence.
- **Pinning is static** — there is no UI to pin/unpin apps yet.
- **Search is app-only** — no settings/file indexing yet.
- **User row is a placeholder** ("U" / "User" / "SunlightOS") — SunlightOS
  has no multi-user/session-identity concept yet.
- **No light theme** — dark only, as scoped for this iteration.

## Future ideas (not implemented)

- Settings search (once a settings-entry catalog exists).
- Document/file search (e.g. indexing `~/Desktop`, `~/Documents`).
- Category navigation / filtering within "All Apps".
- Persistent, user-editable pinning (drag-to-pin/unpin).
- Recommended items / contextual cards.
- Recent *documents*, not just recent apps.
- Lock / Log out / Switch user (needs a session-manager concept first).
- Light theme variant.
- Scroll support if the catalog grows beyond the fixed-grid MVP size.
- Full-screen dim/scrim behind the panel for extra visual focus.
