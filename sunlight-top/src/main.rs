#![no_std]
#![no_main]

extern crate alloc;

mod telemetry;
mod terminal;
mod ui;

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicU32, Ordering};
use telemetry::Telemetry;
use terminal::Canvas;
use ui::table::{SortColumn, SortKey};
use ui::ViewState;

pub static MY_PID: AtomicU32 = AtomicU32::new(0);

const REFRESH_INTERVAL_MS: u64 = 1_000;

struct BumpAllocator;

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        static mut HEAP: [u8; 256 * 1024] = [0; 256 * 1024];
        static mut NEXT: usize = 0;

        // SAFETY: single-threaded process-local bump allocator state.
        let start = unsafe { NEXT };
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }

        // SAFETY: bounds checked; monotonic NEXT update in single-threaded process.
        unsafe {
            NEXT = end;
            HEAP.as_mut_ptr().add(aligned)
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let my_pid = getpid();
    MY_PID.store(my_pid, Ordering::Relaxed);

    let (cols, rows) = get_terminal_size().unwrap_or((80, 24));

    let mut telem = match Telemetry::init() {
        Ok(t) => {
            sunlight_ipc::debug_log("[TOP] telemetry page mapped");
            sunlight_ipc::debug_log("[TOP] magic OK");
            t
        }
        Err(_) => {
            terminal::write_stdout(b"sunlight-top: telemetry unavailable\n");
            sunlight_ipc::ProcessExit::exit(1);
        }
    };

    let mut canvas = Canvas::new();
    canvas.enter_alt_screen();
    canvas.hide_cursor();
    canvas.flush();

    let mut view = ViewState::new(cols, rows);

    sunlight_ipc::debug_log("[TOP] rendering");

    // Paint the first frame immediately.  `Telemetry::poll()` reports whether
    // a newer publication arrived; it may legitimately be false for the
    // initial snapshot, which previously left top's alternate screen blank
    // until a key press or telemetry update occurred.
    view.render(
        &mut canvas,
        telem.snapshot(),
        MY_PID.load(Ordering::Relaxed),
    );

    let mut next_refresh = sunlight_ipc::monotonic_millis().saturating_add(REFRESH_INTERVAL_MS);

    loop {
        let now = sunlight_ipc::monotonic_millis();
        if now >= next_refresh {
            refresh_view(&mut view, &mut canvas, &mut telem);
            // Rebase after a late wake instead of issuing a burst of catch-up
            // renders. The next sample is always one interval from now.
            next_refresh = now.saturating_add(REFRESH_INTERVAL_MS);
            continue;
        }

        let wait_ms = next_refresh.saturating_sub(now).min(i32::MAX as u64) as i32;
        match wait_for_key(wait_ms) {
            Err(()) => {
                // Foreground cancellation, EOF, or a closed PTY exits through
                // the same terminal cleanup path as `q`.
                canvas.show_cursor();
                canvas.exit_alt_screen();
                canvas.flush();
                sunlight_ipc::ProcessExit::exit(0);
            }
            Ok(None) => continue,
            Ok(Some(key)) => {
                let mut redraw = false;
                if let Some((next_cols, next_rows)) = get_terminal_size() {
                    redraw |= view.set_terminal_size(next_cols, next_rows);
                }

                match key {
                    b'q' | b'Q' | 0x03 | 0x1b => {
                        canvas.show_cursor();
                        canvas.exit_alt_screen();
                        canvas.flush();
                        sunlight_ipc::ProcessExit::exit(0);
                    }
                    b's' | b'S' => {
                        view.sort = SortKey {
                            column: SortColumn::Cpu,
                            descending: true,
                        };
                        view.scroll_to_top();
                        redraw = true;
                    }
                    b'm' | b'M' => {
                        view.sort = SortKey {
                            column: SortColumn::Mem,
                            descending: true,
                        };
                        view.scroll_to_top();
                        redraw = true;
                    }
                    b'p' | b'P' => {
                        view.sort = SortKey {
                            column: SortColumn::Pid,
                            descending: false,
                        };
                        view.scroll_to_top();
                        redraw = true;
                    }
                    b'n' | b'N' => {
                        view.sort = SortKey {
                            column: SortColumn::Name,
                            descending: false,
                        };
                        view.scroll_to_top();
                        redraw = true;
                    }
                    b'j' | b'J' => {
                        view.scroll_down(telem.snapshot().proc_count);
                        redraw = true;
                    }
                    b'k' | b'K' => {
                        view.scroll_up();
                        redraw = true;
                    }
                    b'g' => {
                        view.scroll_to_top();
                        redraw = true;
                    }
                    b'G' => {
                        view.scroll_to_bottom(telem.snapshot().proc_count);
                        redraw = true;
                    }
                    _ => {}
                }

                if redraw {
                    view.render(
                        &mut canvas,
                        telem.snapshot(),
                        MY_PID.load(Ordering::Relaxed),
                    );
                }
            }
        }
    }
}

fn refresh_view(view: &mut ViewState, canvas: &mut Canvas, telem: &mut Telemetry) {
    // SIGWINCH delivery is not available yet, so sample the authoritative
    // winsize cache at the same cadence as statistics.
    if let Some((cols, rows)) = get_terminal_size() {
        view.set_terminal_size(cols, rows);
    }
    telem.poll();
    view.render(canvas, telem.snapshot(), MY_PID.load(Ordering::Relaxed));
}

fn wait_for_key(timeout_ms: i32) -> Result<Option<u8>, ()> {
    let mut pollfd = sunlight_ipc::NativePollFd {
        fd: 0,
        events: 1,
        revents: 0,
    };
    let ready = sunlight_ipc::native_poll(core::slice::from_mut(&mut pollfd), timeout_ms)?;
    if ready == 0 {
        return Ok(None);
    }

    let mut buf = [0u8; 1];
    // SAFETY: read syscall arguments are valid userspace pointers and lengths.
    let ret = unsafe {
        let mut out: u64;
        core::arch::asm!(
            "syscall",
            in("rax") 42u64,
            in("rdi") 0u64,
            in("rsi") buf.as_mut_ptr() as u64,
            in("rdx") 1u64,
            lateout("rax") out,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
        out
    };

    if ret == u64::MAX {
        Err(())
    } else if ret == 1 {
        Ok(Some(buf[0]))
    } else {
        Ok(None)
    }
}

fn get_terminal_size() -> Option<(u16, u16)> {
    let size = sunlight_ipc::tty_winsize()?;
    Some((size.columns, size.rows))
}

fn getpid() -> u32 {
    let pid: u64;
    // SAFETY: getpid syscall takes no pointers and returns PID in rax.
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 33u64,
            lateout("rax") pid,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    pid as u32
}
