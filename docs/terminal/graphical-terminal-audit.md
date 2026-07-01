# Graphical Terminal Audit

## Scope

This audit compares:

- `services/tty_server`
- `services/sunlight-terminal`
- `services/pty_server`
- kernel launch/env/ioctl paths relevant to terminal apps

The goal was to explain why full-screen apps behaved better in the TTY than in
the graphical terminal, then fix the graphical terminal without turning `ptty`
into a renderer.

## Findings

### 1. Why `top` worked better in TTY

`tty_server` does not render raw bytes directly. It feeds terminal output
through `sunlight_tty::TerminalGrid`, which consumes VT/ANSI control sequences
before rendering cells to the framebuffer.

`sunlight-terminal` previously used `sunlight_tty::console::Console`, which only
implemented a very small CSI subset. It did not correctly handle the control
flow used by full-screen apps such as:

- cursor-relative movement
- `CSI K` clear line variants
- multi-parameter SGR
- alternate-screen enter/exit
- cursor save/restore
- cursor visibility toggles

That mismatch explains the repeated appended output behind itself: the PTY byte
stream was fine enough to drive the TTY path, but the graphical terminal was not
emulating the terminal state machine with similar fidelity.

### 2. Does TTY already parse ANSI/VT sequences?

Yes.

`tty_server` renders via `sunlight_tty::TerminalGrid`, which uses
`sunlight_tty::vt100::Vt100Parser`.

It was not a complete xterm implementation, but it already consumed enough VT
control flow to behave materially better than the graphical terminal.

### 3. Did `sunlight-terminal` render raw PTY bytes directly?

Not literally raw bytes, but close enough in effect.

It fed PTY output into `sunlight_tty::console::Console`, which had only a very
limited parser/screen model. For `top`-style streams that is functionally
similar to rendering raw output because key state changes were ignored.

### 4. Did `sunlight-terminal` have a real screen grid/cell model?

Before this patch: only a minimal cell buffer with weak ANSI handling.

After this patch: it uses the shared `sunlight_tty::TerminalGrid` terminal
model, so the pipeline is now:

`PTY bytes -> VT parser -> terminal grid -> graphical cell renderer`

### 5. Were cursor movement, clear screen, clear line, colors, and alternate screen implemented?

Before:

- cursor movement: partial
- clear screen: partial
- clear line: effectively incomplete
- colors: only a tiny SGR subset
- alternate screen: not implemented

After:

- cursor movement: absolute and relative cursor movement supported
- clear screen: `CSI J` modes handled
- clear line: `CSI K` modes handled
- colors/styles: basic SGR, bright colors, reset, inverse, underline, bold
- alt screen: DEC private mode enter/exit handled for `?47/?1047/?1049`
- cursor save/restore: handled
- cursor hide/show: handled

Extended color sequences (`38;5`, `38;2`, `48;5`, `48;2`) are reduced to the
existing 16-color renderer palette rather than rendered as truecolor.

### 6. Is `ptty` sending the correct byte stream?

Evidence points to yes.

Reasons:

- `tty_server` already handled the same PTY-backed app output materially better.
- `sunlight-top` emits proper full-screen VT sequences such as `ESC[?1049h`,
  cursor moves, and line clears.
- `pty_server` remains a byte-ring plus line-editor layer; it does not do any
  rendering.

This patch therefore keeps `ptty` rendering-free.

To help prove this interactively, `sunlight-terminal` now accepts:

- `--debug-pty-stream`

That flag logs PTY output in escaped form such as `\x1b[?1049h` without turning
normal logs into noise.

### 7. Is `TERM` useful?

Before this patch: no default `TERM` was set in the kernel process environment.

After this patch: default process environments include:

- `TERM=vt100`

This is intentionally conservative and matches current emulator capabilities far
better than leaving `TERM` unset.

### 8. Are rows/cols/winsize communicated correctly?

Not fully.

Current state:

- kernel `TIOCGWINSZ` still returns a fixed `80x25`
- kernel `TIOCSWINSZ` is accepted but ignored
- `sunlight-terminal` window content was resized in this patch to match `80x25`
  so the graphical terminal is at least internally consistent with the current
  ioctl result
- live resize / `SIGWINCH` is still future work

This means the graphical terminal is now much closer to correct behavior for the
current fixed-size world, but dynamic winsize negotiation is still incomplete.

## Implemented Changes

- `sunlight-terminal` now uses `sunlight_tty::TerminalGrid` instead of the old
  `Console` path.
- The shared VT parser/grid was extended for full-screen app basics.
- Graphical rendering now draws from the terminal cell model rather than the old
  partial console abstraction.
- A visible cursor outline is rendered from terminal state.
- `TERM=vt100` is now present in default spawned process environments.
- A PTY diagnostic mode was added behind `--debug-pty-stream`.

## Architecture After Fix

- `pty_server`: PTY/session byte-stream broker, no rendering
- `sunlight_tty::vt100`: escape-sequence parser
- `sunlight_tty::TerminalGrid`: screen model, cursor state, alt-screen state,
  scrollback, cell attributes
- `sunlight-terminal`: graphical renderer for the terminal model
- `tty_server`: framebuffer renderer for the same model family

## Intentionally Not Fixed Yet

- dynamic winsize propagation and `SIGWINCH`
- truecolor/256-color fidelity beyond palette reduction
- full xterm compatibility
- scroll regions / full DEC private mode coverage
- graphical scrollback UI
- Mousefood/Ratatui integration

## Mousefood Status

Mousefood was not introduced here.

Reason:

- the bug was in terminal emulation correctness, not in the choice of drawing
  toolkit
- fixing the parser/model boundary first keeps the patch incremental and keeps
  `ptty` clean

## Recommended Next Work

1. Wire real terminal geometry into `TIOCGWINSZ` / `TIOCSWINSZ`.
2. Propagate resize events to foreground apps with `SIGWINCH`.
3. Improve renderer fidelity for underline/inverse and optional truecolor.
4. Add manual validation passes for `top`, `less`, `nano`, `clear`, and color
   demos on both TTY and graphical terminal.
