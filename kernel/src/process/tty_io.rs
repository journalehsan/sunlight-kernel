//! Per-tab TTY byte rings — the kernel-side mux that lets a foreground process
//! and the tty_server exchange stdin/stdout without a synchronous IPC round-trip
//! inside `sys_read`/`sys_write` (which run while holding the scheduler lock).
//!
//! Data flow:
//! - keyboard → tty_server → `TtyStdinPush` syscall → `stdin` ring → process `read(fd0)`
//! - process `write(fd1)` → `stdout` ring → `TtyStdoutPull` syscall → tty_server renders
//!
//! Rings are keyed by tab index (the slot the shell/app belongs to). They have
//! their own lock and never touch the scheduler, so they cannot deadlock against
//! `sys_read`/`sys_write`.

use spin::Mutex;

/// Maximum number of TTY tabs the kernel will route for. Matches (and bounds)
/// the tab count tty_server manages.
pub const MAX_TTY_TABS: usize = 16;

const STDIN_CAP: usize = 1024;
const STDOUT_CAP: usize = 8192;

/// Fixed-capacity byte FIFO. On overflow the oldest behaviour is to drop the
/// incoming byte (return `false`) rather than clobber unread data.
struct Ring<const N: usize> {
    buf: [u8; N],
    head: usize,
    len: usize,
}

impl<const N: usize> Ring<N> {
    const fn new() -> Self {
        Self {
            buf: [0u8; N],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, b: u8) -> bool {
        if self.len == N {
            return false;
        }
        let tail = (self.head + self.len) % N;
        self.buf[tail] = b;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let b = self.buf[self.head];
        self.head = (self.head + 1) % N;
        self.len -= 1;
        Some(b)
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
}

static STDIN: Mutex<[Ring<STDIN_CAP>; MAX_TTY_TABS]> =
    Mutex::new([const { Ring::<STDIN_CAP>::new() }; MAX_TTY_TABS]);
static STDOUT: Mutex<[Ring<STDOUT_CAP>; MAX_TTY_TABS]> =
    Mutex::new([const { Ring::<STDOUT_CAP>::new() }; MAX_TTY_TABS]);

/// Push keyboard bytes into a tab's stdin ring (called via `TtyStdinPush`).
/// Returns the number of bytes accepted (drops the tail on overflow).
pub fn push_stdin(tab: usize, bytes: &[u8]) -> usize {
    if tab >= MAX_TTY_TABS {
        return 0;
    }
    let mut rings = STDIN.lock();
    let ring = &mut rings[tab];
    let mut n = 0;
    for &b in bytes {
        if !ring.push(b) {
            break;
        }
        n += 1;
    }
    n
}

/// Drain a tab's stdin ring into `out` (called from `sys_read` on fd0).
/// Returns the number of bytes read; 0 means "empty → EAGAIN".
pub fn read_stdin(tab: usize, out: &mut [u8]) -> usize {
    if tab >= MAX_TTY_TABS {
        return 0;
    }
    let mut rings = STDIN.lock();
    let ring = &mut rings[tab];
    let mut n = 0;
    while n < out.len() {
        match ring.pop() {
            Some(b) => {
                out[n] = b;
                n += 1;
            }
            None => break,
        }
    }
    n
}

/// Push process output into a tab's stdout ring (called from `sys_write` on fd1).
/// Returns the number of bytes accepted.
pub fn write_stdout(tab: usize, bytes: &[u8]) -> usize {
    if tab >= MAX_TTY_TABS {
        return 0;
    }
    let mut rings = STDOUT.lock();
    let ring = &mut rings[tab];
    let mut n = 0;
    for &b in bytes {
        if !ring.push(b) {
            break;
        }
        n += 1;
    }
    n
}

/// Drain a tab's stdout ring into `out` (called via `TtyStdoutPull`).
/// Returns the number of bytes pulled.
pub fn pull_stdout(tab: usize, out: &mut [u8]) -> usize {
    if tab >= MAX_TTY_TABS {
        return 0;
    }
    let mut rings = STDOUT.lock();
    let ring = &mut rings[tab];
    let mut n = 0;
    while n < out.len() {
        match ring.pop() {
            Some(b) => {
                out[n] = b;
                n += 1;
            }
            None => break,
        }
    }
    n
}

/// Drop any buffered bytes for a tab. Called when a new foreground command
/// starts so stale keystrokes/output from the previous command don't bleed in.
pub fn clear_tab(tab: usize) {
    if tab >= MAX_TTY_TABS {
        return;
    }
    STDIN.lock()[tab].clear();
    STDOUT.lock()[tab].clear();
}
