#![no_std]
#![no_main]

mod calculator_state;

use calculator_state::{ButtonAction, CalculatorState};
use sunlight_ipc::{debug_log, process_yield, ProcessExit};
use sunlight_ui::{
    request_close,
    widgets::Label,
    App, Event, Rect, Window, WindowConfig,
};
use sunlight_ui::paint::Canvas;
use sunlight_ui::theme::{Color, Theme};

// ── Layout constants ───────────────────────────────────────────────────────────

const WIN_W: u32 = 340;
const WIN_H: u32 = 350;
const HEADER_H: u32 = 50;
const DISPLAY_H: u32 = 46;

const GRID_X: i32 = 15;
const GRID_Y: i32 = 108;
const BTN_W: u32 = 58;
const BTN_H: u32 = 34;
const GAP: u32 = 5;
const BTN_ROWS: usize = 5;
const BTN_COLS: usize = 5;
const TOTAL_BTNS: usize = BTN_ROWS * BTN_COLS;

// ── Custom button colors ───────────────────────────────────────────────────────

const NUM_BG: Color = Color::rgb(0x42, 0x42, 0x68);
const NUM_HOVER: Color = Color::rgb(0x52, 0x52, 0x78);
const NUM_PRESSED: Color = Color::rgb(0x32, 0x32, 0x50);

const OP_BG: Color = Color::rgb(0x2A, 0x4A, 0x2A);
const OP_HOVER: Color = Color::rgb(0x3A, 0x5A, 0x3A);
const OP_PRESSED: Color = Color::rgb(0x1A, 0x3A, 0x1A);

const CE_BG: Color = Color::rgb(0x6A, 0x44, 0x2A);
const CE_HOVER: Color = Color::rgb(0x7A, 0x54, 0x3A);
const CE_PRESSED: Color = Color::rgb(0x5A, 0x34, 0x1A);

const EQUALS_BG: Color = Color::rgb(0xCC, 0x84, 0x00);
const EQUALS_HOVER: Color = Color::rgb(0xE0, 0x98, 0x20);
const EQUALS_PRESSED: Color = Color::rgb(0xAA, 0x6E, 0x00);

const DISPLAY_BG: Color = Color::rgb(0x28, 0x28, 0x30);
const BTN_TEXT: Color = Color::rgb(0xF0, 0xF0, 0xF0);
const BTN_TEXT_DARK: Color = Color::rgb(0x00, 0x00, 0x00);

// ── No-Alloc Stub ──────────────────────────────────────────────────────────────

struct NoAlloc;
unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}
#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[CALC] panic\n");
    loop {
        process_yield();
    }
}

// ── Button category ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BtnKind {
    Number,
    Operator,
    Memory,
    Clear,
    Func,
    Equals,
    Negate,
}

// ── Button data ────────────────────────────────────────────────────────────────

const BUTTON_LABELS: [&str; TOTAL_BTNS] = [
    "MC", "MR", "M+", "M-", "CE",
    "7",  "8",  "9",  "÷",  "√",
    "4",  "5",  "6",  "×",  "%",
    "1",  "2",  "3",  "−",  "1/x",
    "0",  ".",  "+/−", "+", "=",
];

const BUTTON_ACTIONS: [ButtonAction; TOTAL_BTNS] = [
    ButtonAction::MemoryClear,
    ButtonAction::MemoryRecall,
    ButtonAction::MemoryAdd,
    ButtonAction::MemorySubtract,
    ButtonAction::Clear,
    ButtonAction::Digit(7),
    ButtonAction::Digit(8),
    ButtonAction::Digit(9),
    ButtonAction::Divide,
    ButtonAction::Sqrt,
    ButtonAction::Digit(4),
    ButtonAction::Digit(5),
    ButtonAction::Digit(6),
    ButtonAction::Multiply,
    ButtonAction::Percent,
    ButtonAction::Digit(1),
    ButtonAction::Digit(2),
    ButtonAction::Digit(3),
    ButtonAction::Subtract,
    ButtonAction::Reciprocal,
    ButtonAction::Digit(0),
    ButtonAction::Decimal,
    ButtonAction::Negate,
    ButtonAction::Add,
    ButtonAction::Equals,
];

const BUTTON_KINDS: [BtnKind; TOTAL_BTNS] = [
    BtnKind::Memory, BtnKind::Memory, BtnKind::Memory, BtnKind::Memory, BtnKind::Clear,
    BtnKind::Number, BtnKind::Number, BtnKind::Number, BtnKind::Operator, BtnKind::Func,
    BtnKind::Number, BtnKind::Number, BtnKind::Number, BtnKind::Operator, BtnKind::Func,
    BtnKind::Number, BtnKind::Number, BtnKind::Number, BtnKind::Operator, BtnKind::Func,
    BtnKind::Number, BtnKind::Number, BtnKind::Negate, BtnKind::Operator, BtnKind::Equals,
];

const KEY_ESC: u8 = 0x01;

// ── Application state ──────────────────────────────────────────────────────────

struct CalcApp {
    state: CalculatorState,
    hovered_idx: Option<usize>,
    pressed_idx: Option<usize>,
}

impl CalcApp {
    fn new() -> Self {
        Self {
            state: CalculatorState::new(),
            hovered_idx: None,
            pressed_idx: None,
        }
    }

    fn btn_rect(idx: usize) -> Rect {
        let row = idx / BTN_COLS;
        let col = idx % BTN_COLS;
        Rect::new(
            GRID_X + (col as i32) * (BTN_W + GAP) as i32,
            GRID_Y as i32 + (row as i32) * (BTN_H + GAP) as i32,
            BTN_W,
            BTN_H,
        )
    }

    fn hit_test(&self, x: i32, y: i32) -> Option<usize> {
        for idx in 0..TOTAL_BTNS {
            let rect = Self::btn_rect(idx);
            if rect.contains(sunlight_ui::Point::new(x, y)) {
                return Some(idx);
            }
        }
        None
    }

    fn draw_button(canvas: &mut Canvas, idx: usize, hovered: bool, pressed: bool, theme: &Theme) {
        let rect = Self::btn_rect(idx);
        let kind = BUTTON_KINDS[idx];
        let label = BUTTON_LABELS[idx];

        let (bg, text_color) = match (kind, hovered || pressed, pressed) {
            (BtnKind::Number, false, _) => (NUM_BG, BTN_TEXT),
            (BtnKind::Number, true, false) => (NUM_HOVER, BTN_TEXT),
            (BtnKind::Number, _, true) => (NUM_PRESSED, BTN_TEXT),

            (BtnKind::Operator, false, _) => (OP_BG, BTN_TEXT),
            (BtnKind::Operator, true, false) => (OP_HOVER, BTN_TEXT),
            (BtnKind::Operator, _, true) => (OP_PRESSED, BTN_TEXT),

            (BtnKind::Memory, false, _) => (OP_BG, BTN_TEXT),
            (BtnKind::Memory, true, false) => (OP_HOVER, BTN_TEXT),
            (BtnKind::Memory, _, true) => (OP_PRESSED, BTN_TEXT),

            (BtnKind::Func, false, _) => (OP_BG, BTN_TEXT),
            (BtnKind::Func, true, false) => (OP_HOVER, BTN_TEXT),
            (BtnKind::Func, _, true) => (OP_PRESSED, BTN_TEXT),

            (BtnKind::Negate, false, _) => (OP_BG, BTN_TEXT),
            (BtnKind::Negate, true, false) => (OP_HOVER, BTN_TEXT),
            (BtnKind::Negate, _, true) => (OP_PRESSED, BTN_TEXT),

            (BtnKind::Clear, false, _) => (CE_BG, BTN_TEXT),
            (BtnKind::Clear, true, false) => (CE_HOVER, BTN_TEXT),
            (BtnKind::Clear, _, true) => (CE_PRESSED, BTN_TEXT),

            (BtnKind::Equals, false, _) => (EQUALS_BG, BTN_TEXT_DARK),
            (BtnKind::Equals, true, false) => (EQUALS_HOVER, BTN_TEXT_DARK),
            (BtnKind::Equals, _, true) => (EQUALS_PRESSED, BTN_TEXT_DARK),
        };

        canvas.fill_rect(rect, bg);
        canvas.draw_rect(rect, theme.accent);
        canvas.draw_text_centered(rect, label, text_color);
    }

    fn handle_keyboard(&mut self, ch: char) -> bool {
        let action = match ch {
            '0'..='9' => Some(ButtonAction::Digit(ch as u8 - b'0')),
            '.' => Some(ButtonAction::Decimal),
            '+' => Some(ButtonAction::Add),
            '-' => Some(ButtonAction::Subtract),
            '*' => Some(ButtonAction::Multiply),
            '/' => Some(ButtonAction::Divide),
            '%' => Some(ButtonAction::Percent),
            '\n' | '\r' | '=' => Some(ButtonAction::Equals),
            _ => None,
        };
        if let Some(a) = action {
            self.state.handle_action(a);
            true
        } else {
            false
        }
    }

    fn draw_header(canvas: &mut Canvas, theme: &Theme) {
        let header = Rect::new(0, 0, WIN_W, HEADER_H);
        canvas.fill_rect(header, theme.panel);

        Label::new(Rect::new(10, 6, 200, 14), "Sunlight Calculator")
            .draw(canvas, theme);
        Label::new(Rect::new(10, 22, 280, 12), "Simple arithmetic with memory and chained operations")
            .dim()
            .draw(canvas, theme);

        let sunlight_label = "SunlightOS";
        let tw = Canvas::measure_text(sunlight_label);
        let sx = (WIN_W as i32) - (tw as i32) - 10;
        canvas.draw_text(sx, 6, sunlight_label, theme.accent);

        canvas.hbar(0, 48, WIN_W, 2, theme.accent);
    }

    fn draw_display(canvas: &mut Canvas, app: &Self, theme: &Theme) {
        let display_rect = Rect::new(10, 54, WIN_W - 20, DISPLAY_H);
        canvas.fill_rect(display_rect, DISPLAY_BG);
        canvas.draw_rect(display_rect, theme.accent);

        let text = app.state.display_str();
        let text_color = if app.state.error || text == "Error" {
            theme.danger
        } else {
            theme.text
        };

        let tw = Canvas::measure_text(text);
        let pad = 8;
        let tx = display_rect.right() - (tw as i32) - pad;
        let ty = display_rect.y + (display_rect.h as i32 - 7) / 2;
        canvas.draw_text(tx, ty, text, text_color);

        if app.state.memory_value() != 0.0 {
            canvas.draw_text(display_rect.x + 4, ty, "M", theme.accent);
        }
    }
}

impl App for CalcApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

        Self::draw_header(canvas, theme);
        Self::draw_display(canvas, self, theme);

        for idx in 0..TOTAL_BTNS {
            let hovered = self.hovered_idx == Some(idx);
            let pressed = self.pressed_idx == Some(idx);
            Self::draw_button(canvas, idx, hovered, pressed, theme);
        }
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => {
                self.pressed_idx = None;
                if let Some(idx) = self.hit_test(x, y) {
                    if let Some(&action) = BUTTON_ACTIONS.get(idx) {
                        self.state.handle_action(action);
                        return true;
                    }
                }
                false
            }
            Event::MouseDown { x, y, button: 0 } => {
                self.pressed_idx = self.hit_test(x, y);
                true
            }
            Event::MouseMove { x, y } => {
                let new_hover = self.hit_test(x, y);
                if new_hover != self.hovered_idx {
                    self.hovered_idx = new_hover;
                    return true;
                }
                false
            }
            Event::Key(ch) => self.handle_keyboard(ch),
            Event::KeyPress {
                keycode, pressed: true, ..
            } => {
                if keycode == KEY_ESC {
                    request_close();
                    return true;
                }
                false
            }
            _ => false,
        }
    }
}

// ── Entry point ────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut app = CalcApp::new();

    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Calculator",
    }) {
        Some(w) => w,
        None => {
            debug_log("[CALC] failed to connect window\n");
            loop {
                process_yield();
            }
        }
    };

    window.run(&mut app);
    ProcessExit::exit(0);
}
