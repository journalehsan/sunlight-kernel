#![no_std]
#![cfg_attr(not(test), no_main)]

extern crate alloc;

#[cfg(not(test))]
use core::alloc::{GlobalAlloc, Layout};

use chronos_core::{
    display_char, translate_key_press, BiosKey, GuestState, HostKeyEvent, Runtime,
    CHRONOS_INTERACTIVE_COM,
};
use sun_font::{draw_text, measure_text, FontRole, TextStyle};
use sunlight_ipc::{debug_log, monotonic_millis};
#[cfg(not(test))]
use sunlight_ipc::{process_yield, ProcessExit};
use sunlight_ui::{widgets::Panel, App, Canvas, Color, Event, Rect, Theme};
#[cfg(not(test))]
use sunlight_ui::{Window, WindowConfig, WindowDecoration};

const WIN_W: u32 = 840;
const WIN_H: u32 = 592;
const HEADER_H: u32 = 48;
const FOOTER_H: u32 = 24;
const PAD: i32 = 18;
const SURFACE_INSET: i32 = 6;
const DOS_CELL_W: i32 = 9;
const DOS_CELL_H: i32 = 16;
const INSTRUCTIONS_PER_TICK: usize = 128;
const DOS_SURFACE: Color = Color::rgb(12, 20, 37);
const DOS_CURSOR: Color = Color::rgb(255, 181, 71);

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
    cursor_visible: bool,
}

impl ChronosApp {
    fn new() -> Self {
        let runtime = Runtime::from_com(CHRONOS_INTERACTIVE_COM)
            .expect("bundled Chronos COM program must fit the guest process");
        let mut app = Self {
            runtime,
            status: [0; 32],
            status_len: 0,
            trap_logged: false,
            cursor_visible: true,
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
        core::str::from_utf8(&self.status[..self.status_len]).unwrap_or("Guest Trapped")
    }

    fn update_status_from_runtime(&mut self) -> bool {
        match self.runtime.state().clone() {
            GuestState::Ready => self.set_status(b"Ready"),
            GuestState::Running => self.set_status(b"Running"),
            GuestState::WaitingForInput => self.set_status(b"Waiting for Input"),
            GuestState::Exited { code } => {
                let mut status = [0; 32];
                let mut length = copy_bytes(b"Exited with code ", &mut status);
                length += write_decimal_u8(code, &mut status[length..]);
                self.set_status(&status[..length])
            }
            GuestState::Halted => self.set_status(b"Guest Halted"),
            GuestState::Trapped(trap) => {
                let changed = self.set_status(b"Guest Trapped");
                if !self.trap_logged {
                    log_trap(&trap);
                    self.trap_logged = true;
                }
                changed
            }
        }
    }

    fn draw_text_surface(&self, canvas: &mut Canvas, rect: Rect, theme: &Theme) {
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.draw_rect(rect, theme.border);
        let surface = rect.inset(SURFACE_INSET);
        canvas.fill_rect(surface, DOS_SURFACE);
        canvas.draw_rect(surface, dos_color(8));

        let x0 = surface.x + ((surface.w as i32 - 80 * DOS_CELL_W) / 2).max(0);
        let y0 = surface.y + ((surface.h as i32 - 25 * DOS_CELL_H) / 2).max(0);
        draw_surface_cells(canvas, &self.runtime, x0, y0);
        if self.cursor_visible
            && matches!(
                self.runtime.state(),
                GuestState::WaitingForInput | GuestState::Running
            )
        {
            let cursor_x = x0 + self.runtime.cursor_column() as i32 * DOS_CELL_W;
            let cursor_y = y0 + self.runtime.cursor_row() as i32 * DOS_CELL_H + DOS_CELL_H - 2;
            canvas.fill_rect(
                Rect::new(cursor_x, cursor_y, DOS_CELL_W as u32, 2),
                DOS_CURSOR,
            );
        }
    }

    fn draw_status_bar(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = Rect::new(0, WIN_H as i32 - FOOTER_H as i32, WIN_W, FOOTER_H);
        let small = FontRole::UiSmall;
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.hbar(rect.x, rect.y, rect.w, 1, theme.border);

        let text_y = rect.y + (rect.h as i32 - measure_text("Ag", small).h as i32) / 2;
        draw_text(
            canvas,
            "DOS .COM",
            rect.x + 10,
            text_y,
            &TextStyle::new(small, theme.text_dim),
        );

        let status_size = measure_text(self.status_text(), small);
        draw_text(
            canvas,
            self.status_text(),
            rect.x + (rect.w as i32 - status_size.w as i32) / 2,
            text_y,
            &TextStyle::new(small, theme.text),
        );

        let runtime_hint = "INT 16h / B8000";
        let hint_size = measure_text(runtime_hint, small);
        draw_text(
            canvas,
            runtime_hint,
            rect.right() - hint_size.w as i32 - 10,
            text_y,
            &TextStyle::new(small, theme.text_dim),
        );
    }
}

impl App for ChronosApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        Panel::new(Rect::new(0, 0, WIN_W, HEADER_H)).draw(canvas, theme);
        draw_text(
            canvas,
            "Chronos - Sunlight DOS Terminal",
            PAD,
            7,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        draw_text(
            canvas,
            "16-bit real-mode guest",
            PAD,
            25,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );

        let text_rect = Rect::new(
            PAD,
            HEADER_H as i32 + PAD,
            WIN_W - (PAD as u32 * 2),
            WIN_H - HEADER_H - FOOTER_H - PAD as u32 * 2,
        );
        self.draw_text_surface(canvas, text_rect, theme);
        self.draw_status_bar(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => {
                let cursor_active = matches!(
                    self.runtime.state(),
                    GuestState::WaitingForInput | GuestState::Running
                );
                let cursor_visible = cursor_active && (monotonic_millis() / 500) % 2 == 0;
                let cursor_changed = self.cursor_visible != cursor_visible;
                self.cursor_visible = cursor_visible;
                if matches!(
                    self.runtime.state(),
                    GuestState::Ready | GuestState::Running
                ) {
                    let text_or_state_changed = self.runtime.run_slice(INSTRUCTIONS_PER_TICK);
                    cursor_changed || text_or_state_changed || self.update_status_from_runtime()
                } else {
                    cursor_changed
                }
            }
            Event::Key(ch) if ch.is_ascii_graphic() || ch == ' ' => {
                self.runtime.inject_key(BiosKey {
                    ascii: ch as u8,
                    scan_code: 0,
                });
                self.update_status_from_runtime()
            }
            Event::KeyPress {
                keycode,
                pressed,
                shift,
                ctrl,
                alt,
                ..
            } => {
                self.runtime.update_modifiers(shift, ctrl, alt);
                let key = translate_key_press(HostKeyEvent {
                    keycode,
                    pressed,
                    shift,
                    ctrl,
                    alt,
                });
                if let Some(key) = key.filter(|key| key.ascii == 0 || key.ascii < 0x20) {
                    self.runtime.inject_key(key);
                    self.update_status_from_runtime()
                } else {
                    false
                }
            }
            _ => false,
        }
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

fn draw_surface_cells(canvas: &mut Canvas, runtime: &Runtime, x0: i32, y0: i32) {
    for row in 0..25 {
        for column in 0..80 {
            let cell = runtime.cell(column, row);
            let background = dos_color((cell.attribute >> 4) & 0x07);
            let foreground = dos_color(cell.attribute & 0x0f);
            canvas.fill_rect(
                Rect::new(
                    x0 + column as i32 * DOS_CELL_W,
                    y0 + row as i32 * DOS_CELL_H,
                    DOS_CELL_W as u32,
                    DOS_CELL_H as u32,
                ),
                background,
            );
            if cell.character != b' ' {
                let mut encoded = [0; 4];
                let text = display_char(cell.character).encode_utf8(&mut encoded);
                draw_text(
                    canvas,
                    text,
                    x0 + column as i32 * DOS_CELL_W,
                    y0 + row as i32 * DOS_CELL_H,
                    &TextStyle::new(FontRole::MonoRegular, foreground),
                );
            }
        }
    }
}

fn dos_color(index: u8) -> sunlight_ui::Color {
    const COLORS: [(u8, u8, u8); 16] = [
        (12, 20, 37),
        (71, 108, 174),
        (70, 151, 122),
        (72, 158, 181),
        (188, 91, 108),
        (175, 105, 168),
        (193, 137, 74),
        (203, 214, 229),
        (76, 95, 121),
        (111, 162, 230),
        (107, 205, 155),
        (106, 198, 222),
        (242, 125, 140),
        (212, 136, 207),
        (248, 190, 102),
        (244, 248, 255),
    ];
    let (red, green, blue) = COLORS[index as usize & 0x0f];
    sunlight_ui::Color::rgb(red, green, blue)
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8, _envp: *const *const u8) -> ! {
    let mut app = ChronosApp::new();
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Chronos - Sunlight DOS Terminal",
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
    use super::{dos_color, ChronosApp, DOS_CELL_W, DOS_SURFACE};
    use chronos_core::{BiosKey, GuestState};
    use sun_font::{measure_text, FontRole};
    use sunlight_ui::{App, Event};

    #[test]
    fn status_text_uses_the_precise_waiting_state() {
        let mut app = ChronosApp::new();
        assert_eq!(app.status_text(), "Ready");

        app.runtime.run_slice(1024);
        assert!(app.update_status_from_runtime());
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
        assert_eq!(app.status_text(), "Waiting for Input");
    }

    #[test]
    fn printable_text_and_enter_use_separate_host_event_paths() {
        let mut app = ChronosApp::new();
        app.runtime.run_slice(1024);
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);

        assert!(!app.update(Event::Key('\n')));
        assert_eq!(app.runtime.cursor_column(), 0);
        assert!(app.update(Event::KeyPress {
            keycode: 0x1c,
            pressed: true,
            shift: false,
            ctrl: false,
            alt: false,
            super_key: false,
        }));
        app.update(Event::Tick);
        assert_eq!(
            (app.runtime.cursor_column(), app.runtime.cursor_row()),
            (0, 7)
        );
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
    }

    #[test]
    fn raw_printable_key_events_do_not_duplicate_text_input() {
        let mut app = ChronosApp::new();
        app.runtime.run_slice(1024);

        assert!(!app.update(Event::KeyPress {
            keycode: 0x39,
            pressed: true,
            shift: false,
            ctrl: false,
            alt: false,
            super_key: false,
        }));
        assert!(app.update(Event::Key('A')));
        app.update(Event::Tick);

        assert_eq!(app.runtime.cell(0, 6).character, b'A');
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
        assert_eq!(
            (app.runtime.cursor_column(), app.runtime.cursor_row()),
            (1, 6)
        );
    }

    #[test]
    fn dos_grid_uses_fixed_fira_code_cell_widths() {
        assert_eq!(
            measure_text("MM", FontRole::MonoRegular).w,
            (DOS_CELL_W * 2) as u32
        );
    }

    #[test]
    fn dos_palette_keeps_a_navy_surface_and_distinct_dos_colors() {
        assert_eq!(dos_color(0), DOS_SURFACE);
        assert_ne!(dos_color(0), dos_color(1));
        assert_ne!(dos_color(7), dos_color(15));
        assert!(dos_color(15).r() > dos_color(0).r());
    }

    #[test]
    fn idle_cursor_ticks_do_not_wake_a_blocked_guest() {
        let mut app = ChronosApp::new();
        app.runtime.run_slice(1024);
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);

        app.update(Event::Tick);
        app.update(Event::Tick);
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
    }

    #[test]
    fn bundled_interactive_guest_waits_and_exits_from_guest_keyboard_input() {
        let mut app = ChronosApp::new();
        app.runtime.run_slice(1024);
        assert!(app.update_status_from_runtime());
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);

        app.runtime.inject_key(BiosKey {
            ascii: 0x1b,
            scan_code: 0x01,
        });
        app.runtime.run_slice(64);
        assert!(app.update_status_from_runtime());
        assert_eq!(app.runtime.state(), &GuestState::Exited { code: 0 });
        assert_eq!(app.status_text(), "Exited with code 0");
    }
}
