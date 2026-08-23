# Dynamic terminal winsize contract

The drawable client dimensions reported by `sunlight-ui::WindowEvent::Resized`
are the frontend input. They already exclude compositor borders and titlebar.
`sunlight-terminal` subtracts its tab strip, footer, padding, and the terminal
viewport's one-pixel border. It divides the remaining width and height by the
renderer-owned 8×16 cell metrics. Incomplete cells at the right and bottom are
intentionally ignored and painted as terminal background.

The resulting `TerminalWinsize` is sent with `PtyMsg::CREATE` and on each real
grid change with `PtyMsg::SET_WINDOW_SIZE`. `pty_server` is the authoritative
per-session owner. It validates the value, suppresses unchanged updates, and
publishes a generation-qualified copy into the kernel TTY cache. The cache is
necessary because fd syscalls cannot synchronously call a userspace service.
It is derived state, not a second owner.

When the PTY shell is spawned, the terminal registers it as the foreground
process. The trusted broker attaches `(session id, generation)` in the kernel;
children inherit it. Native applications query `sunlight_libc::terminal_winsize`.
Helios validates that the requested fd is a TTY and translates the same value
field-by-field into Linux x86_64 `struct winsize` for `TIOCGWINSZ`.

The broker fallback is 100×30 only when `CREATE` carries no size. The graphical
terminal always supplies real initial geometry, so that fallback never
overwrites frontend state. The legacy framebuffer TTY publishes its existing
`sunlight-tui` grid calculation under generation zero; its documented 80×25
kernel fallback is visible only before that first renderer publication.

Kernel routing IDs reserve the low range for legacy framebuffer tabs and place
brokered PTYs at `PTY_TTY_TAB_BASE + session_id`. This prevents a graphical PTY
and a legacy tab with the same small numeric ID from sharing byte rings or
winsize cache slots.

`TIOCSWINSZ` does not resize the graphical window and returns `EPERM` on a TTY.
SunlightOS currently has no distinct nested-PTY logical resize owner.

Linux signal frames and `rt_sigreturn` remain incomplete. Consequently this
phase does not claim `SIGWINCH` delivery. Winsize state changes immediately;
applications that re-query after their existing input/poll wake see it. No
geometry-only poll readiness or periodic wake hack is introduced.

```text
compositor drawable client surface
  -> terminal chrome/padding subtraction
  -> renderer 8x16 complete-cell grid
  -> PTY session TerminalWinsize (authoritative)
  -> generation-qualified kernel TTY cache (derived)
     -> native terminal_winsize()
     -> Helios TIOCGWINSZ -> Linux struct winsize
```
