# SunlightOS GUI Current State

**Status:** Implemented a stable graphical desktop stack with rounded window chrome, symbol glyphs, and flicker-free presentation.  
**Last updated:** 2026-06-27

## Overview

SunlightOS now has a working graphical desktop path that is distinct from the
early TTY/login path.

Current graphical pieces:

- `sunlight-display`
  - Ring-3 compositor / window manager.
  - Maps the physical framebuffer.
  - Owns window creation, composition, decorations, focus, dragging, and basic resizing.
- `eyes`
  - First graphical demo client.
  - Creates a window and tracks the mouse through display events.
- `sunlight-terminal`
  - First graphical terminal window.
  - Uses the real `pty_server` backend instead of fake built-in command handling.
- `sunlight-mouse`
  - Ring-3 mouse driver feeding the desktop path.
- `sunlight-kbd` + `tty_server`
  - Keyboard still enters through the existing TTY/session path.
  - `tty_server` remains the owner of global session switching and forwards desktop keys to the compositor when the desktop session is active.

## Session Model

The system still has two distinct session paths:

- `Ctrl+F1`
  - Switches to TTY/Login.
- `Ctrl+F2`
  - Switches to Desktop.

Important constraints that still hold:

- TTY/Login is not moved into `sunlightd`.
- The desktop does not replace `tty_server`.
- Global session switching remains outside individual GUI apps.

## Display / Windowing State

The display stack currently supports:

- Window creation through `SgpMsg::CREATE_WINDOW`
- Shared-memory client buffers
- Full-window commits through `SgpMsg::COMMIT_FRAME`
- Window title updates through `SgpMsg::CONFIGURE_WINDOW`
- Mouse position polling through `SgpMsg::EVENT_POLL`
- Focus, stacking, dragging, maximize/minimize, and close handling in the compositor
- Decorated windows with the current orange-accent theme

Recent extension:

- Focused keyboard delivery is now buffered by the compositor and returned via `EVENT_POLL`.
- This is enough for lightweight native GUI clients to receive text input without bypassing the existing global keyboard/session routing.

Day 22 polish checklist:

- [x] Rounded rectangle fill/stroke primitives added to `sunlight-ui`
- [x] Built-in UI symbol glyphs added for calculator and window controls
- [x] Calculator operator/function symbols fixed to use the new glyph set
- [x] Window controls updated to the improved rounded dark/orange style
- [x] Flicker/blinking fixed by staged client commits plus off-screen desktop composition

Current limitations:

- No dedicated resize event delivery to clients yet
- No damage tracking; redraws are effectively full-window
- No async pushed events; clients poll
- No richer widget toolkit yet

## Input Routing State

Keyboard routing currently works like this:

1. Keyboard driver sends events into the existing TTY/session path.
2. `tty_server` handles global shortcuts like `Ctrl+F1` / `Ctrl+F2`.
3. When the desktop session is active, non-global key events are forwarded to `sunlight-display`.
4. `sunlight-display` queues those events for the focused window.
5. GUI clients retrieve them through `EVENT_POLL`.

Mouse routing currently works like this:

1. `sunlight-mouse` sends raw motion/button information to the desktop stack.
2. `sunlight-display` updates global pointer state.
3. Clients poll the display service for mouse position and their current client origin.

Current PS/2 pointer behavior:

- Cursor movement uses fixed-point accumulation so subpixel motion is preserved across small deltas.
- Pointer acceleration is deterministic and threshold-based, with explicit sensitivity/gain constants in both `sunlight-mouse` and `sunlight-display`.
- Hardware cursor moves do not repaint the desktop back buffer when the VirtIO GPU cursor overlay is active.

This means:

- Global desktop/session shortcuts are still centralized.
- GUI apps do not consume `Ctrl+F1` / `Ctrl+F2`.
- Focused GUI clients can receive ordinary typing.

## Input Backend TODOs

- `VirtioInputMouse`
  - Feed the same relative-motion policy used by PS/2 without duplicating acceleration or clamp logic again.
- `VirtioInputTablet`
  - Add an absolute-pointer path that bypasses relative acceleration and maps directly into compositor coordinates.
- `UsbHidMouse`
  - Land later behind the same backend abstraction as PS/2 and virtio-input so diagnostics and button semantics stay aligned.

## PTY State

The PTY backend is provided by `pty_server`.

Current PTY capabilities:

- Create PTY sessions with `PtyMsg::CREATE`
- Read/write master side
- Read/write slave side
- Switch mode flags with `PtyMsg::SET_MODE`
- Close sessions

Current PTY properties:

- Up to 8 sessions
- Byte-ring transport
- Register-IPC chunking at 16 bytes per transfer
- Canonical/echo flags exist
- No resize API
- No shell spawning API inside the PTY server

This is important:

- `pty_server` is currently a PTY transport and line-discipline service.
- It is not yet a session launcher by itself.

## Graphical Terminal State

`sunlight-terminal` is the first native desktop terminal client.

What it does now:

- Opens a decorated graphical window
- Connects to `display_server`
- Creates and maps its window surface
- Opens a real PTY session from `pty_server`
- Spawns `sunshell` in a PTY-slave startup mode
- Sends keyboard input to the PTY master
- Reads PTY output from the PTY master
- Renders terminal cells inside the window
- Shows a visible tab bar with `Tab 1` and a visual `+` button

Rendering model:

- Dark background
- Simple top tab strip
- Terminal text grid using existing `sunlight-tui` font/framebuffer code
- Full-window redraws

Terminal behavior:

- PTY output is the source of truth
- The client does not fake shell commands
- The shell prompt and shell editing echo come back through the PTY path

Current tab state:

- One functional tab
- `+` is visual-only for now

Current scrollback state:

- Bounded through the existing `sunlight_tty::console::Console`
- Current bound is 256 lines

## `sunshell` PTY Mode

Because `pty_server` does not spawn shells directly, `sunshell` now supports
two startup modes:

- Existing tty_server IPC shell mode
- New PTY-slave argv-driven mode

In PTY mode:

- `sunshell` receives the PTY session id and slave capability through argv
- It writes prompt/output back to the PTY slave
- It reads input from the PTY slave
- For foreground child processes, it reuses the hidden TTY stdin/stdout ring path so spawned commands still behave like normal shell children

This keeps the PTY server as the real terminal/session backend without moving
the global TTY architecture.

## Build / Boot Integration State

The GUI terminal is now integrated into the normal build/embed path:

- Workspace member added
- Built by `tools/build.sh`
- Built by `tools/test.sh`
- Embedded in the kernel binary table
- Spawn path added in kernel process path resolution

Desktop launch behavior currently:

- Desktop session still launches `eyes`
- Desktop session also launches `sunlight-terminal`

This keeps the existing demo alive while adding the first useful GUI app.

## What Is Working

- Graphical desktop session activation
- Window creation and composition
- Mouse-driven window interaction
- Focused keyboard delivery to GUI clients
- Real PTY-backed graphical terminal path
- Shell prompt/output appearing inside a graphical window
- Keyboard input reaching the shell through PTY
- Existing TTY/Desktop split preserved

## Known Limitations

- No PTY resize support yet
- No client-visible resize event handling yet
- No multi-tab PTY management yet
- `+` button is visual-only
- Scrollback is bounded but still smaller than the long-term target
- PTY transport is polling-based, not event-driven
- PTY chunks are small, so bulk output is correct but not optimized
- The compositor keyboard path is still intentionally minimal and poll-based

## Suggested Next Steps

1. Add real multi-tab support with one PTY session per tab.
2. Add resize propagation from window manager to terminal client.
3. Add PTY resize protocol support.
4. Improve terminal input coverage for arrows/page keys and more ANSI handling.
5. Add better desktop app-launch policy instead of spawning demo apps directly from `tty_server`.
