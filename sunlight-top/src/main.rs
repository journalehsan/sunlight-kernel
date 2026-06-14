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

    let mut view = ViewState::new();
    view.term_cols = cols;
    view.term_rows = rows;

    sunlight_ipc::debug_log("[TOP] rendering");

    let mut iterations = 0;
    const MAX_ITERATIONS: u32 = 100; // ~10 seconds at 100ms per iteration

    loop {
        iterations += 1;
        if iterations >= MAX_ITERATIONS {
            canvas.show_cursor();
            canvas.exit_alt_screen();
            canvas.flush();
            sunlight_ipc::ProcessExit::exit(0);
        }

        if let Some(key) = read_key_nonblocking() {
            match key {
                b'q' | b'Q' | 0x1b => {
                    canvas.show_cursor();
                    canvas.exit_alt_screen();
                    canvas.flush();
                    sunlight_ipc::ProcessExit::exit(0);
                }
                b's' | b'S' => {
                    view.sort = SortKey {
                        column: SortColumn::Cpu,
                        descending: true,
                    }
                }
                b'm' | b'M' => {
                    view.sort = SortKey {
                        column: SortColumn::Mem,
                        descending: true,
                    }
                }
                b'p' | b'P' => {
                    view.sort = SortKey {
                        column: SortColumn::Pid,
                        descending: false,
                    }
                }
                b'n' | b'N' => {
                    view.sort = SortKey {
                        column: SortColumn::Name,
                        descending: false,
                    }
                }
                _ => {}
            }
        }

        if telem.poll() {
            view.render(&mut canvas, telem.snapshot(), MY_PID.load(Ordering::Relaxed));
        }

        sleep_ms(100);
    }
}

fn read_key_nonblocking() -> Option<u8> {
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

    if ret == 1 {
        Some(buf[0])
    } else {
        None
    }
}

fn get_terminal_size() -> Option<(u16, u16)> {
    None
}

fn sleep_ms(ms: u64) {
    for _ in 0..(ms.saturating_mul(5000)) {
        core::hint::spin_loop();
    }
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
