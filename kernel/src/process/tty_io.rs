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
use sunlight_ipc::TerminalWinsize;

/// Maximum number of TTY tabs the kernel will route for. Must cover the full
/// u8 range since tty_tab is stored as u8 (shell_id cast via `as u8`).
pub const MAX_TTY_TABS: usize = 256;

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

#[derive(Clone, Copy)]
struct WinsizeSlot {
    generation: u64,
    size: Option<TerminalWinsize>,
}

impl WinsizeSlot {
    const fn empty() -> Self {
        Self {
            generation: 0,
            size: None,
        }
    }
}

/// Generation-qualified kernel cache of the authoritative PTY broker state.
/// The broker remains the owner; this cache lets native fd operations and the
/// Helios ioctl translator read geometry without attempting user IPC while in
/// a syscall. Generation matching prevents a closed/reused session slot from
/// leaking a later terminal's geometry to an old process.
static WINSIZES: Mutex<[WinsizeSlot; MAX_TTY_TABS]> =
    Mutex::new([WinsizeSlot::empty(); MAX_TTY_TABS]);

pub fn publish_winsize(tab: usize, generation: u64, size: Option<TerminalWinsize>) -> bool {
    if tab >= MAX_TTY_TABS || size.is_some_and(|value| !value.is_valid()) {
        return false;
    }
    let mut slots = WINSIZES.lock();
    let slot = &mut slots[tab];
    if size.is_none() && slot.generation != generation {
        return false;
    }
    slot.generation = generation;
    slot.size = size;
    true
}

pub fn winsize(tab: usize, generation: u64) -> Option<TerminalWinsize> {
    if tab >= MAX_TTY_TABS {
        return None;
    }
    let slot = WINSIZES.lock()[tab];
    (slot.generation == generation)
        .then_some(slot.size)
        .flatten()
}

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

/// Check if a tab's stdin ring has any bytes available without draining.
pub fn has_stdin(tab: usize) -> bool {
    if tab >= MAX_TTY_TABS {
        return false;
    }
    STDIN.lock()[tab].len > 0
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

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn winsizes_are_per_terminal_and_generation_qualified() {
        let a = TerminalWinsize::new(120, 40, 960, 640);
        let b = TerminalWinsize::new(80, 25, 640, 400);
        assert!(publish_winsize(10, 7, Some(a)));
        assert!(publish_winsize(11, 3, Some(b)));
        assert_eq!(winsize(10, 7), Some(a));
        assert_eq!(winsize(11, 3), Some(b));

        let resized = TerminalWinsize::new(170, 50, 1360, 800);
        assert!(publish_winsize(10, 7, Some(resized)));
        assert_eq!(winsize(10, 7), Some(resized));
        assert_eq!(winsize(11, 3), Some(b));
        assert_eq!(winsize(10, 6), None);
    }

    #[test]
    fn stale_teardown_cannot_clear_reused_slot() {
        let current = TerminalWinsize::new(132, 44, 1056, 704);
        assert!(publish_winsize(12, 9, Some(current)));
        assert!(!publish_winsize(12, 8, None));
        assert_eq!(winsize(12, 9), Some(current));
        assert!(publish_winsize(12, 9, None));
        assert_eq!(winsize(12, 9), None);
    }
}
