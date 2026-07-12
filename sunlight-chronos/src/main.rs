#![no_std]
#![cfg_attr(not(test), no_main)]

extern crate alloc;

#[cfg(not(test))]
use core::alloc::{GlobalAlloc, Layout};

use chronos_core::{display_char, GuestState, Runtime, TextModeSurface, HELLO_CHRONOS_COM};
use sunlight_ipc::debug_log;
#[cfg(not(test))]
use sunlight_ipc::{process_yield, ProcessExit};
use sunlight_ui::{
    widgets::{Panel, StatusBar},
    App, Canvas, Event, Rect, Theme,
};
#[cfg(not(test))]
use sunlight_ui::{Window, WindowConfig, WindowDecoration};

const WIN_W: u32 = 672;
const WIN_H: u32 = 390;
const HEADER_H: u32 = 34;
const FOOTER_H: u32 = StatusBar::HEIGHT;
const PAD: i32 = 12;
const TEXT_CELL_W: i32 = 7;
const TEXT_CELL_H: i32 = 10;
const INSTRUCTIONS_PER_TICK: usize = 128;

#[cfg(not(test))]
struct BumpAllocator;

#[cfg(not(test))]
unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        static mut HEAP: [u8; 2 * 1024 * 1024] = [0; 2 * 1024 * 1024];
        static mut NEXT: usize = 0;

        let aligned = (NEXT + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned.saturating_add(layout.size());
        if end > 2 * 1024 * 1024 {
            return core::ptr::null_mut();
        }
        NEXT = end;
        core::ptr::addr_of_mut!(HEAP).cast::<u8>().add(aligned)
    }

    unsafe fn dealloc(&self, _pointer: *mut u8, _layout: Layout) {}
}

#[cfg(not(test))]
#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[CHRONOS] panic\n");
    loop {
        process_yield();
    }
}

struct ChronosApp {
    runtime: Runtime,
    status: [u8; 32],
    status_len: usize,
    trap_logged: bool,
}

impl ChronosApp {
    fn new() -> Self {
        let runtime = Runtime::from_com(HELLO_CHRONOS_COM)
            .expect("bundled Chronos COM program must fit the guest process");
        let mut app = Self {
            runtime,
            status: [0; 32],
            status_len: 0,
            trap_logged: false,
        };
        app.set_status(b"Ready");
        app
    }

    fn set_status(&mut self, value: &[u8]) -> bool {
        let length = value.len().min(self.status.len());
        if self.status_len == length && self.status[..length] == value[..length] {
            return false;
        }
        self.status_len = length;
        self.status[..length].copy_from_slice(&value[..length]);
        true
    }

    fn status_text(&self) -> &str {
        core::str::from_utf8(&self.status[..self.status_len]).unwrap_or("Guest trapped")
    }

    fn update_status_from_runtime(&mut self) -> bool {
        match self.runtime.state().clone() {
            GuestState::Runnable => self.set_status(b"Running"),
            GuestState::Exited { code } => {
                let mut status = [0; 32];
                let mut length = copy_bytes(b"Exited with code ", &mut status);
                length += write_decimal_u8(code, &mut status[length..]);
                self.set_status(&status[..length])
            }
            GuestState::Halted => self.set_status(b"Guest halted"),
            GuestState::Trapped(trap) => {
                let changed = self.set_status(b"Guest trapped");
                if !self.trap_logged {
                    log_trap(&trap);
                    self.trap_logged = true;
                }
                changed
            }
        }
    }

    fn draw_text_surface(&self, canvas: &mut Canvas, rect: Rect, theme: &Theme) {
        let background = sunlight_ui::Color::rgb(0x08, 0x16, 0x12);
        let foreground = sunlight_ui::Color::rgb(0xB9, 0xF6, 0xCA);
        canvas.fill_rect(rect, background);
        canvas.draw_rect(rect, theme.border);

        let x0 = rect.x + ((rect.w as i32 - 80 * TEXT_CELL_W) / 2).max(0);
        let y0 = rect.y + ((rect.h as i32 - 25 * TEXT_CELL_H) / 2).max(0);
        draw_surface_cells(canvas, &self.runtime.text, x0, y0, foreground);
    }
}

impl App for ChronosApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        Panel::with_title(
            Rect::new(0, 0, WIN_W, HEADER_H),
            "Chronos · DOS Compatibility",
        )
        .draw(canvas, theme);
        canvas.draw_text(16, 12, "16-bit real-mode guest", theme.text_dim);

        let text_rect = Rect::new(
            PAD,
            HEADER_H as i32 + PAD,
            WIN_W - (PAD as u32 * 2),
            WIN_H - HEADER_H - FOOTER_H - PAD as u32 * 2,
        );
        self.draw_text_surface(canvas, text_rect, theme);
        StatusBar::new(
            Rect::new(0, WIN_H as i32 - FOOTER_H as i32, WIN_W, FOOTER_H),
            "DOS .COM",
            self.status_text(),
            "INT 21h / INT 10h",
        )
        .draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        if !matches!(event, Event::Tick) || !matches!(self.runtime.state(), GuestState::Runnable) {
            return false;
        }

        let text_or_state_changed = self.runtime.run_slice(INSTRUCTIONS_PER_TICK);
        text_or_state_changed || self.update_status_from_runtime()
    }
}

fn log_trap(trap: &chronos_core::Trap) {
    let mut line = [0u8; 160];
    let mut length = copy_bytes(b"[CHRONOS] guest trap: ", &mut line);
    length += copy_bytes(trap.summary().as_bytes(), &mut line[length..]);

    if let chronos_core::Trap::UnsupportedOpcode { cs, ip, bytes, cpu } = trap {
        length += copy_bytes(b" cs:ip=", &mut line[length..]);
        length += write_hex_u16(*cs, &mut line[length..]);
        line[length] = b':';
        length += 1;
        length += write_hex_u16(*ip, &mut line[length..]);
        length += copy_bytes(b" opcode=", &mut line[length..]);
        length += write_hex_u8(bytes[0], &mut line[length..]);
        length += copy_bytes(b" ax=", &mut line[length..]);
        length += write_hex_u16(cpu.ax, &mut line[length..]);
        length += copy_bytes(b" bx=", &mut line[length..]);
        length += write_hex_u16(cpu.bx, &mut line[length..]);
        length += copy_bytes(b" cx=", &mut line[length..]);
        length += write_hex_u16(cpu.cx, &mut line[length..]);
        length += copy_bytes(b" dx=", &mut line[length..]);
        length += write_hex_u16(cpu.dx, &mut line[length..]);
        length += copy_bytes(b" ds=", &mut line[length..]);
        length += write_hex_u16(cpu.ds, &mut line[length..]);
        length += copy_bytes(b" es=", &mut line[length..]);
        length += write_hex_u16(cpu.es, &mut line[length..]);
        length += copy_bytes(b" ss=", &mut line[length..]);
        length += write_hex_u16(cpu.ss, &mut line[length..]);
    }
    if let chronos_core::Trap::UnsupportedInterrupt {
        interrupt,
        function,
    } = trap
    {
        length += copy_bytes(b" int=", &mut line[length..]);
        length += write_hex_u8(*interrupt, &mut line[length..]);
        length += copy_bytes(b" ah=", &mut line[length..]);
        length += write_hex_u8(*function, &mut line[length..]);
    }
    if let chronos_core::Trap::UnterminatedDosString {
        segment, offset, ..
    } = trap
    {
        length += copy_bytes(b" ds:dx=", &mut line[length..]);
        length += write_hex_u16(*segment, &mut line[length..]);
        line[length] = b':';
        length += 1;
        length += write_hex_u16(*offset, &mut line[length..]);
    }
    line[length] = b'\n';
    length += 1;

    if let Ok(text) = core::str::from_utf8(&line[..length]) {
        debug_log(text);
    }
}

fn copy_bytes(source: &[u8], destination: &mut [u8]) -> usize {
    let length = source.len().min(destination.len());
    destination[..length].copy_from_slice(&source[..length]);
    length
}

fn write_hex_u16(value: u16, output: &mut [u8]) -> usize {
    for (index, shift) in [12, 8, 4, 0].iter().enumerate() {
        output[index] = hex_digit((value >> shift) as u8);
    }
    4
}

fn write_hex_u8(value: u8, output: &mut [u8]) -> usize {
    output[0] = hex_digit(value >> 4);
    output[1] = hex_digit(value);
    2
}

fn write_decimal_u8(value: u8, output: &mut [u8]) -> usize {
    if value >= 100 {
        output[0] = b'0' + value / 100;
        output[1] = b'0' + (value / 10) % 10;
        output[2] = b'0' + value % 10;
        3
    } else if value >= 10 {
        output[0] = b'0' + value / 10;
        output[1] = b'0' + value % 10;
        2
    } else {
        output[0] = b'0' + value;
        1
    }
}

fn hex_digit(value: u8) -> u8 {
    match value & 0x0f {
        0..=9 => b'0' + (value & 0x0f),
        digit => b'A' + (digit - 10),
    }
}

fn draw_surface_cells(
    canvas: &mut Canvas,
    surface: &TextModeSurface,
    x0: i32,
    y0: i32,
    color: sunlight_ui::Color,
) {
    for row in 0..25 {
        for column in 0..80 {
            let cell = surface.cell(column, row);
            if cell.character != b' ' {
                canvas.draw_char(
                    x0 + column as i32 * TEXT_CELL_W,
                    y0 + row as i32 * TEXT_CELL_H,
                    display_char(cell.character),
                    color,
                );
            }
        }
    }
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8, _envp: *const *const u8) -> ! {
    let mut app = ChronosApp::new();
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Chronos",
        decoration: WindowDecoration::Normal,
    }) {
        Some(window) => window,
        None => loop {
            process_yield();
        },
    };
    window.run(&mut app);
    ProcessExit::exit(0);
}

#[cfg(test)]
mod tests {
    use super::ChronosApp;
    use chronos_core::GuestState;

    #[test]
    fn bundled_guest_reaches_a_clean_exit_after_a_bounded_slice() {
        let mut app = ChronosApp::new();
        app.runtime.run_slice(128);
        assert!(app.update_status_from_runtime());

        assert_eq!(app.runtime.state(), &GuestState::Exited { code: 0 });
        assert_eq!(app.status_text(), "Exited with code 0");
    }
}
