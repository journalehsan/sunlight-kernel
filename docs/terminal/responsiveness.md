# Terminal rendering and transport

The graphical terminal uses 14 px Sun Mono in 9×20 cells, with a dark palette,
an adaptive tab strip, and a horizontally scrolling monospace command field.
Foreground application names appear in the tabs. Bold, inverse, underline,
cursor visibility, alternate screens, and scrollback come from the shared
`TerminalGrid` model. Only changed cells, including the old/new cursor cells,
are rasterized. Resizing, changing tabs, and changing session status invalidate
the retained viewport; the compositor still receives complete frames.

`sunlight_tty::console::Console` now wraps `TerminalGrid`, so there is one VT
parser and screen implementation. Each framebuffer TTY tab retains its own
console, feeding output once as it arrives. It no longer replays a 64 KiB byte
log or discards screen state on `CSI 2 J` and alternate-screen transitions.
Wrapping at the right margin is deferred until the next printable character,
preventing a full-width bottom row from scrolling the screen prematurely.

## Bulk PTY protocol

The existing eight-byte register operations remain available. Two additional
operations carry up to one shared-memory page (4096 bytes):

| Operation | Endpoint authority | Data direction |
| --- | --- | --- |
| `READ_MASTER_BULK` (`0x7310`) | Master | Output ring to shared page |
| `WRITE_SLAVE_BULK` (`0x7311`) | Slave | Shared page to output ring |

Requests carry the session ID, generation, and length in words 0, 1, and 2.
Capability 0 is the endpoint authority; capability 1 is the shared page.
Successful replies retain ID/generation in words 0/1 and report the transferred
count in word 2. The broker validates the authenticated caller and generation
before mapping the page, rejects lengths over 4096, and releases its temporary
mapping before replying. Empty live reads return `ERR_WOULD_BLOCK`; buffered
output remains readable after the slave closes. A full output ring returns
`ERR_WOULD_BLOCK` without overwriting queued bytes.

The graphical frontend and PTY shell each reuse a page. Allocation failure
falls back to the register operations. The shell retries full-ring writes and
advances only by the accepted count. It also drains the child's remaining
stdout before sending the application-completion notification.

Graphical output draining is capped at four reads per tab per tick, with a
4 ms elapsed-time budget checked between reads. Each individual IPC retains
the 100 ms service timeout. The active tab and one background tab are serviced
per tick. Streaming uses a 1 ms display poll; idle polling uses 16 ms. Cursor
blink requests no repaint when a foreground application hides its cursor.

Rebuild the terminal, PTY broker, TTY server, and `sshl` together before booting.
The protocol supports existing register clients, but new bulk clients require
the updated broker.

## Verification

```sh
cargo test -p sunlight-tty -p sunlight-terminal -p pty_server --target x86_64-unknown-linux-gnu
cargo check -p sunlight-terminal -p pty_server -p sunlight-tty-server --target x86_64-unknown-none
cargo check -p sunshell --target x86_64-unknown-none
```

Check Sunshell separately: combining its custom allocator with GUI packages
unifies `sunlight-libc/global-alloc` and produces a duplicate allocator error.

Host tests cover page transfers through partial writes/ring wraparound,
authorization rejection, split VT sequences, long-running alternate screens,
bottom-row wrapping, scrollback joins, incremental/full repaint equivalence,
font metrics, and narrow-window clipping/tab hit targets.

To export actual renderer previews with synthetic shell and `top` content:

```sh
SUNLIGHT_TERMINAL_PREVIEW=/tmp/terminal cargo test -p sunlight-terminal --target x86_64-unknown-linux-gnu terminal_preview_and_small_window_clipping
```

This writes `/tmp/terminal-shell.ppm` and `/tmp/terminal-top.ppm`. Host tests and
previews do not measure live IPC latency or prove QEMU interaction. For runtime
validation, run `top`, switch/resize tabs during output, generate more than
8 KiB of output, and verify a short-lived command's final lines. Compare the
framebuffer TTY and graphical terminal, including leaving an alternate-screen
application and returning to shell history. Linux `SIGWINCH`, UTF-8 cell
decoding, and truecolor remain outside this change.
