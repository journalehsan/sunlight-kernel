#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::GlobalAlloc;

use sun_font::{
    draw_text, draw_text_vcenter, line_height, measure_text, FontRole, TextStyle, VecFont,
};
use sunlight_edit::args::extract_first_real_file_path;
use sunlight_edit::text_buffer::TextBuffer;
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, process_yield, ProcessExit,
};
use sunlight_libc::{self as libc, crt0};
use sunlight_ui::widgets::button::ButtonState;
use sunlight_ui::widgets::{StatusBar, Toolbar, ToolbarItem};
use sunlight_ui::{
    request_close, App, Canvas, Event, Point, Rect, Theme, Window, WindowConfig,
    WindowDecoration,
};

static FONT_MONO: VecFont = VecFont(FontRole::MonoRegular);

const WIN_W: u32 = 900;
const WIN_H: u32 = 640;
const HEADER_H: u32 = 34;
const TOOLBAR_H: u32 = 34;
const STATUS_H: u32 = 22;
const GUTTER_W: u32 = 56;
const PAD: i32 = 8;
const PATH_LEN: usize = 256;
const MSG_LEN: usize = 96;
const MAX_FILE_BYTES: usize = 512 * 1024;
const MAX_ARGC: usize = 8;
const DEFAULT_SAVE_PATH: &str = "/root/untitled.txt";
const UNTITLED_DISPLAY: &str = "Untitled";

const KEY_ESC: u8 = 0x01;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_DELETE: u8 = 0x53;
const KEY_S: u8 = 0x1F;
const KEY_ENTER: u8 = 0x1C;

const DIALOG_W: u32 = 520;
const DIALOG_BTN_W: u32 = 88;
const DIALOG_BTN_H: u32 = 28;
const DIALOG_BTN_GAP: u32 = 10;
const DIALOG_PAD: i32 = 16;
const DIALOG_FIELD_H: u32 = 30;

struct BumpAllocator;
unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 2 * 1024 * 1024] = [0; 2 * 1024 * 1024];
        static mut NEXT: usize = 0;
        let aligned = (NEXT + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[EDIT] panic\n");
    loop {
        process_yield();
    }
}

#[derive(Clone, Copy)]
struct PathBuf {
    buf: [u8; PATH_LEN],
    len: usize,
}

impl PathBuf {
    const fn empty() -> Self {
        Self {
            buf: [0; PATH_LEN],
            len: 0,
        }
    }

    fn from_str(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.is_empty() || bytes.len() >= PATH_LEN {
            return None;
        }
        let mut out = Self::empty();
        out.buf[..bytes.len()].copy_from_slice(bytes);
        out.len = bytes.len();
        Some(out)
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    fn file_name(&self) -> &str {
        let path = self.as_str();
        path.rsplit('/').next().unwrap_or(path)
    }

    fn push_char(&mut self, ch: char) -> bool {
        let mut encoded = [0u8; 4];
        let bytes = ch.encode_utf8(&mut encoded).as_bytes();
        if self.len + bytes.len() >= PATH_LEN {
            return false;
        }
        self.buf[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        true
    }

    fn pop_char(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }
        let text = self.as_str();
        let Some(ch) = text.chars().last() else {
            self.len = 0;
            return true;
        };
        self.len -= ch.len_utf8();
        true
    }
}

#[derive(Clone, Copy)]
struct TextSlot {
    buf: [u8; MSG_LEN],
    len: usize,
}

impl TextSlot {
    const fn empty() -> Self {
        Self {
            buf: [0; MSG_LEN],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn set(&mut self, text: &str) {
        let bytes = text.as_bytes();
        self.len = bytes.len().min(MSG_LEN);
        self.buf[..self.len].copy_from_slice(&bytes[..self.len]);
    }

    fn as_str(&self) -> &str {
        if self.len == 0 {
            return "";
        }
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseDialog {
    None,
    SaveBeforeClose,
    SaveToPath,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DialogAction {
    Save,
    Discard,
    Cancel,
}

#[derive(Clone, Copy)]
struct DialogButton {
    action: DialogAction,
    rect: Rect,
    label: &'static str,
}

struct EditApp {
    buffer: TextBuffer,
    backing_path: Option<PathBuf>,
    user_path: Option<PathBuf>,
    is_temporary: bool,
    startup_error: TextSlot,
    status_left: TextSlot,
    status_center: TextSlot,
    status_right: TextSlot,
    header_title: TextSlot,
    scroll_line: usize,
    caret_visible: bool,
    caret_ticks: u8,
    status_msg_ticks: u16,
    toolbar_hover: Option<usize>,
    toolbar_pressed: Option<usize>,
    pending_user_path: Option<PathBuf>,
    document_ready: bool,
    close_dialog: CloseDialog,
    dialog_path_input: PathBuf,
    dialog_buttons: [DialogButton; 3],
    dialog_button_count: usize,
}

impl EditApp {
    fn new(pending_user_path: Option<PathBuf>) -> Self {
        let mut app = Self {
            buffer: TextBuffer::new(),
            backing_path: None,
            user_path: None,
            is_temporary: false,
            startup_error: TextSlot::empty(),
            status_left: TextSlot::empty(),
            status_center: TextSlot::empty(),
            status_right: TextSlot::empty(),
            header_title: TextSlot::empty(),
            scroll_line: 0,
            caret_visible: true,
            caret_ticks: 0,
            status_msg_ticks: 0,
            toolbar_hover: None,
            toolbar_pressed: None,
            pending_user_path,
            document_ready: false,
            close_dialog: CloseDialog::None,
            dialog_path_input: PathBuf::empty(),
            dialog_buttons: [DialogButton {
                action: DialogAction::Cancel,
                rect: Rect::new(0, 0, 0, 0),
                label: "",
            }; 3],
            dialog_button_count: 0,
        };
        app.refresh_header_title();
        app.refresh_status_right();
        app.refresh_status_left();
        app
    }

    fn display_name(&self) -> &str {
        if let Some(path) = &self.user_path {
            return path.file_name();
        }
        if self.is_temporary {
            return UNTITLED_DISPLAY;
        }
        self.backing_path
            .as_ref()
            .map(PathBuf::file_name)
            .unwrap_or(UNTITLED_DISPLAY)
    }

    fn refresh_header_title(&mut self) {
        let mut title = String::from("\u{1F31E} ");
        title.push_str(self.display_name());
        if self.buffer.is_dirty() {
            title.push('*');
        }
        self.header_title.set(&title);
    }

    fn refresh_status_left(&mut self) {
        let mut text = String::from("Ln ");
        push_usize(&mut text, self.buffer.cursor_line + 1);
        text.push_str(", Col ");
        push_usize(&mut text, self.buffer.cursor_col + 1);
        self.status_left.set(&text);
    }

    fn refresh_status_right(&mut self) {
        let mut text = String::new();
        push_usize(&mut text, self.buffer.char_count());
        text.push_str(" chars | ");
        push_usize(&mut text, self.buffer.word_count());
        text.push_str(" words | ");
        push_usize(&mut text, self.buffer.line_count());
        text.push_str(" lines | UTF-8 | ");
        if self.is_temporary {
            text.push_str("Temporary · ");
        }
        if self.buffer.is_dirty() {
            text.push_str("Modified via VFS");
        } else {
            text.push_str("Saved via VFS");
        }
        text.push_str(" | SunlightOS");
        self.status_right.set(&text);
    }

    fn refresh_status_bars(&mut self) {
        self.refresh_status_left();
        self.refresh_status_right();
        self.refresh_header_title();
    }

    fn set_status_message(&mut self, text: &str) {
        self.status_center.set(text);
        self.status_msg_ticks = 30;
    }

    fn open_real_file(&mut self, path: PathBuf) {
        self.startup_error.clear();
        match read_utf8_file(path.as_bytes()) {
            Ok(content) => {
                self.buffer = TextBuffer::from_utf8(&content);
                self.backing_path = Some(path);
                self.user_path = Some(path);
                self.is_temporary = false;
                self.scroll_line = 0;
                self.buffer.mark_saved();
                self.set_status_message("Opened");
            }
            Err(msg) => {
                self.startup_error.set(msg);
                self.set_status_message(msg);
            }
        }
        self.refresh_status_bars();
    }

    fn create_temp_document(&mut self) {
        self.startup_error.clear();
        match create_temp_backing_path() {
            Ok(path) => {
                self.buffer = TextBuffer::new();
                self.backing_path = Some(path);
                self.user_path = None;
                self.is_temporary = true;
                self.scroll_line = 0;
                self.buffer.mark_saved();
            }
            Err(msg) => {
                self.startup_error.set(msg);
                self.set_status_message(msg);
            }
        }
        self.refresh_status_bars();
    }

    fn initialize_document(&mut self) {
        if self.document_ready {
            return;
        }
        self.document_ready = true;
        if let Some(path) = self.pending_user_path.take() {
            self.open_real_file(path);
        } else {
            self.create_temp_document();
        }
    }

    fn save_to_backing(&mut self) -> Result<(), &'static str> {
        let backing = self
            .backing_path
            .ok_or("No save path")?;
        let content = self.buffer.to_utf8_string();
        write_utf8_file(backing.as_bytes(), content.as_bytes())
    }

    fn save(&mut self) {
        match self.save_to_backing() {
            Ok(()) => {
                self.buffer.mark_saved();
                if self.is_temporary {
                    self.set_status_message("Saved to temporary file");
                } else {
                    self.set_status_message("Saved");
                }
            }
            Err(msg) => self.set_status_message(msg),
        }
        self.refresh_status_bars();
    }

    fn save_to_final_path(&mut self, path: PathBuf) -> Result<(), &'static str> {
        let content = self.buffer.to_utf8_string();
        write_utf8_file(path.as_bytes(), content.as_bytes())?;
        self.backing_path = Some(path);
        self.user_path = Some(path);
        self.is_temporary = false;
        self.buffer.mark_saved();
        Ok(())
    }

    fn needs_close_prompt(&self) -> bool {
        if self.is_temporary {
            !self.buffer.is_content_empty() || self.buffer.is_dirty()
        } else {
            self.buffer.is_dirty()
        }
    }

    fn try_close(&mut self) {
        if self.close_dialog != CloseDialog::None {
            return;
        }
        if !self.needs_close_prompt() {
            request_close();
            return;
        }
        if self.is_temporary {
            self.dialog_path_input =
                PathBuf::from_str(DEFAULT_SAVE_PATH).unwrap_or(PathBuf::empty());
            self.close_dialog = CloseDialog::SaveToPath;
        } else {
            self.close_dialog = CloseDialog::SaveBeforeClose;
        }
        self.layout_dialog_buttons();
    }

    fn dismiss_dialog(&mut self) {
        self.close_dialog = CloseDialog::None;
        self.dialog_button_count = 0;
    }

    fn dialog_action(&mut self, action: DialogAction) -> bool {
        match (self.close_dialog, action) {
            (CloseDialog::SaveBeforeClose, DialogAction::Save) => {
                if self.save_to_backing().is_ok() {
                    self.buffer.mark_saved();
                    self.dismiss_dialog();
                    request_close();
                    return true;
                }
                self.set_status_message("Save failed");
                false
            }
            (CloseDialog::SaveBeforeClose, DialogAction::Discard) => {
                self.dismiss_dialog();
                request_close();
                true
            }
            (CloseDialog::SaveBeforeClose, DialogAction::Cancel) => {
                self.dismiss_dialog();
                true
            }
            (CloseDialog::SaveToPath, DialogAction::Save) => {
                let Some(path) = PathBuf::from_str(self.dialog_path_input.as_str()) else {
                    self.set_status_message("Invalid path");
                    return false;
                };
                match self.save_to_final_path(path) {
                    Ok(()) => {
                        self.dismiss_dialog();
                        request_close();
                        true
                    }
                    Err(msg) => {
                        self.set_status_message(msg);
                        false
                    }
                }
            }
            (CloseDialog::SaveToPath, DialogAction::Discard) => {
                self.dismiss_dialog();
                request_close();
                true
            }
            (CloseDialog::SaveToPath, DialogAction::Cancel) => {
                self.dismiss_dialog();
                true
            }
            _ => false,
        }
    }

    fn dialog_panel_rect(&self) -> Rect {
        let h = match self.close_dialog {
            CloseDialog::SaveToPath => 188,
            CloseDialog::SaveBeforeClose => 132,
            CloseDialog::None => 0,
        };
        Rect::new(
            ((WIN_W - DIALOG_W) / 2) as i32,
            ((WIN_H - h) / 2) as i32,
            DIALOG_W,
            h,
        )
    }

    fn layout_dialog_buttons(&mut self) {
        let panel = self.dialog_panel_rect();
        let labels = match self.close_dialog {
            CloseDialog::SaveBeforeClose => ["Save", "Discard", "Cancel"],
            CloseDialog::SaveToPath => ["Save", "Discard", "Cancel"],
            CloseDialog::None => ["", "", ""],
        };
        let count = 3usize;
        let total_w = count as u32 * DIALOG_BTN_W + (count as u32 - 1) * DIALOG_BTN_GAP;
        let mut x = panel.x + ((panel.w as i32 - total_w as i32) / 2);
        let y = panel.bottom() - DIALOG_PAD - DIALOG_BTN_H as i32;
        self.dialog_button_count = count;
        for (i, label) in labels.iter().enumerate() {
            self.dialog_buttons[i] = DialogButton {
                action: match i {
                    0 => DialogAction::Save,
                    1 => DialogAction::Discard,
                    _ => DialogAction::Cancel,
                },
                rect: Rect::new(x, y, DIALOG_BTN_W, DIALOG_BTN_H),
                label,
            };
            x += DIALOG_BTN_W as i32 + DIALOG_BTN_GAP as i32;
        }
    }

    fn dialog_button_hit(&self, x: i32, y: i32) -> Option<DialogAction> {
        for btn in &self.dialog_buttons[..self.dialog_button_count] {
            if btn.rect.contains(Point::new(x, y)) {
                return Some(btn.action);
            }
        }
        None
    }

    fn handle_dialog_key(&mut self, keycode: u8, pressed: bool) -> bool {
        if !pressed {
            return false;
        }
        match keycode {
            KEY_ESC => self.dialog_action(DialogAction::Cancel),
            KEY_ENTER => self.dialog_action(DialogAction::Save),
            _ => false,
        }
    }

    fn handle_dialog_text(&mut self, ch: char) -> bool {
        if self.close_dialog != CloseDialog::SaveToPath {
            return false;
        }
        match ch {
            '\u{8}' => self.dialog_path_input.pop_char(),
            '\r' | '\n' => return self.dialog_action(DialogAction::Save),
            c if !c.is_control() => self.dialog_path_input.push_char(c),
            _ => false,
        }
    }

    fn editor_rect(&self) -> Rect {
        let top = HEADER_H + TOOLBAR_H;
        let bottom = WIN_H.saturating_sub(STATUS_H);
        Rect::new(0, top as i32, WIN_W, bottom.saturating_sub(top))
    }

    fn visible_line_count(&self) -> usize {
        let rect = self.editor_rect();
        let lh = line_height(FontRole::MonoRegular).max(1) as u32;
        (rect.h / lh).max(1) as usize
    }

    fn ensure_cursor_visible(&mut self) {
        let visible = self.visible_line_count();
        if self.buffer.cursor_line < self.scroll_line {
            self.scroll_line = self.buffer.cursor_line;
        } else if self.buffer.cursor_line >= self.scroll_line + visible {
            self.scroll_line = self.buffer.cursor_line + 1 - visible;
        }
    }

    fn toolbar_rect(&self) -> Rect {
        Rect::new(0, HEADER_H as i32, WIN_W, TOOLBAR_H)
    }

    fn toolbar_hit(&self, x: i32, y: i32) -> Option<usize> {
        const ITEMS: usize = 2;
        const ITEM_W: u32 = 72;
        let rect = self.toolbar_rect();
        if !rect.contains(Point::new(x, y)) {
            return None;
        }
        let rel = (x - rect.x).max(0) as u32;
        let idx = (rel / ITEM_W) as usize;
        if idx < ITEMS { Some(idx) } else { None }
    }

    fn handle_toolbar_click(&mut self, idx: usize) -> bool {
        match idx {
            0 => {
                self.save();
                true
            }
            _ => false,
        }
    }

    fn handle_key_press(&mut self, keycode: u8, pressed: bool, ctrl: bool) -> bool {
        if self.close_dialog != CloseDialog::None {
            return self.handle_dialog_key(keycode, pressed);
        }
        if !pressed {
            return false;
        }
        if keycode == KEY_ESC {
            self.try_close();
            return true;
        }
        if ctrl && keycode == KEY_S {
            self.save();
            return true;
        }
        let changed = match keycode {
            KEY_LEFT => self.buffer.move_left(),
            KEY_RIGHT => self.buffer.move_right(),
            KEY_UP => self.buffer.move_up(),
            KEY_DOWN => self.buffer.move_down(),
            KEY_HOME => self.buffer.move_home(),
            KEY_END => self.buffer.move_end(),
            KEY_DELETE => self.buffer.delete_forward(),
            _ => false,
        };
        if changed {
            self.ensure_cursor_visible();
            self.refresh_status_bars();
        }
        changed
    }

    fn handle_text_key(&mut self, ch: char) -> bool {
        if self.close_dialog != CloseDialog::None {
            return self.handle_dialog_text(ch);
        }
        let changed = match ch {
            '\u{8}' => self.buffer.backspace(),
            '\n' => self.buffer.insert_newline(),
            '\r' => false,
            c if !c.is_control() => self.buffer.insert_char(c),
            _ => false,
        };
        if changed {
            self.ensure_cursor_visible();
            self.refresh_status_bars();
        }
        changed
    }

    fn draw_header(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = Rect::new(0, 0, WIN_W, HEADER_H);
        canvas.fill_rect(rect, theme.panel);
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);
        draw_text_vcenter(
            canvas,
            self.header_title.as_str(),
            PAD,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        draw_text_vcenter(
            canvas,
            "Sunlight Edit",
            rect.right() - 120,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
    }

    fn draw_toolbar(&self, canvas: &mut Canvas, theme: &Theme) {
        let items = [
            ToolbarItem {
                label: "Save",
                state: if self.toolbar_pressed == Some(0) {
                    ButtonState::Pressed
                } else if self.toolbar_hover == Some(0) {
                    ButtonState::Hovered
                } else {
                    ButtonState::Normal
                },
                active: false,
            },
            ToolbarItem {
                label: "Redo",
                state: ButtonState::Disabled,
                active: false,
            },
        ];
        Toolbar::new(self.toolbar_rect(), &items, 72).draw(canvas, theme);
    }

    fn draw_editor(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.editor_rect();
        canvas.fill_rect(rect, theme.bg);

        let gutter = Rect::new(rect.x, rect.y, GUTTER_W, rect.h);
        canvas.fill_rect(gutter, theme.panel_alt);
        canvas.vline(gutter.right() - 1, gutter.y, gutter.h, theme.border);

        let text_x = gutter.right() + PAD;
        let lh = line_height(FontRole::MonoRegular).max(1) as i32;
        let visible = self.visible_line_count();
        let mono = TextStyle::new(FontRole::MonoRegular, theme.text);
        let gutter_style = TextStyle::new(FontRole::MonoRegular, theme.text_dim);

        for row in 0..visible {
            let line_idx = self.scroll_line + row;
            let y = rect.y + (row as i32) * lh;
            if y + lh > rect.bottom() {
                break;
            }
            let line_no = line_idx + 1;
            let mut num = String::new();
            push_usize(&mut num, line_no);
            let nw = measure_text(&num, FontRole::MonoRegular).w;
            draw_text(
                canvas,
                &num,
                gutter.right() - (nw as i32) - 6,
                y,
                &gutter_style,
            );

            let Some(line) = self.buffer.line(line_idx) else {
                continue;
            };
            draw_text(canvas, line, text_x, y, &mono);

            if line_idx == self.buffer.cursor_line && self.caret_visible {
                let prefix: String = line.chars().take(self.buffer.cursor_col).collect();
                let cx = text_x + measure_text(&prefix, FontRole::MonoRegular).w as i32;
                canvas.vline(cx, y, lh.saturating_sub(2) as u32, theme.accent);
            }
        }
    }

    fn draw_startup_error(&self, canvas: &mut Canvas, theme: &Theme) {
        if self.startup_error.len == 0 {
            return;
        }
        let rect = Rect::new(PAD, (HEADER_H + TOOLBAR_H + 4) as i32, WIN_W - 16, 20);
        draw_text(
            canvas,
            self.startup_error.as_str(),
            rect.x,
            rect.y,
            &TextStyle::new(FontRole::UiSmall, theme.danger),
        );
    }

    fn draw_dialog_button(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        btn: DialogButton,
        hovered: bool,
    ) {
        let fill = if hovered {
            theme.panel_alt
        } else {
            theme.panel
        };
        canvas.fill_rounded_rect(btn.rect, 5, fill);
        canvas.stroke_rounded_rect(btn.rect, 5, 1, theme.border);
        draw_text_vcenter(
            canvas,
            btn.label,
            btn.rect.x + 8,
            btn.rect.y,
            btn.rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );
    }

    fn draw_close_dialog(&self, canvas: &mut Canvas, theme: &Theme) {
        if self.close_dialog == CloseDialog::None {
            return;
        }
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg.darken(70));
        let panel = self.dialog_panel_rect();
        canvas.fill_rounded_rect_with_border(panel, 8, theme.panel, theme.border, 1);

        let title = match self.close_dialog {
            CloseDialog::SaveBeforeClose => "Save changes before closing?",
            CloseDialog::SaveToPath => {
                "This document is only saved in a temporary file. Enter a final path to save it:"
            }
            CloseDialog::None => "",
        };
        draw_text(
            canvas,
            title,
            panel.x + DIALOG_PAD,
            panel.y + DIALOG_PAD,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );

        if self.close_dialog == CloseDialog::SaveToPath {
            let field = Rect::new(
                panel.x + DIALOG_PAD,
                panel.y + 44,
                panel.w - (DIALOG_PAD as u32 * 2),
                DIALOG_FIELD_H,
            );
            canvas.fill_rounded_rect(field, 5, theme.bg);
            canvas.stroke_rounded_rect(field, 5, 1, theme.border);
            draw_text_vcenter(
                canvas,
                self.dialog_path_input.as_str(),
                field.x + 8,
                field.y,
                field.h,
                &TextStyle::new(FontRole::MonoRegular, theme.text),
            );
        }

        for btn in &self.dialog_buttons[..self.dialog_button_count] {
            self.draw_dialog_button(canvas, theme, *btn, false);
        }
    }

    fn draw_status(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = Rect::new(0, (WIN_H - STATUS_H) as i32, WIN_W, STATUS_H);
        StatusBar::new(
            rect,
            self.status_left.as_str(),
            self.status_center.as_str(),
            self.status_right.as_str(),
        )
        .draw(canvas, theme);
    }
}

impl App for EditApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        self.draw_header(canvas, theme);
        self.draw_toolbar(canvas, theme);
        self.draw_editor(canvas, theme);
        self.draw_startup_error(canvas, theme);
        self.draw_status(canvas, theme);
        self.draw_close_dialog(canvas, theme);
    }

    fn on_ready(&mut self) -> bool {
        self.initialize_document();
        true
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => {
                let mut redraw = false;
                self.caret_ticks = self.caret_ticks.wrapping_add(1);
                if self.caret_ticks % 5 == 0 {
                    self.caret_visible = !self.caret_visible;
                    redraw = true;
                }
                if self.status_msg_ticks > 0 {
                    self.status_msg_ticks -= 1;
                    if self.status_msg_ticks == 0 {
                        self.status_center.clear();
                        redraw = true;
                    }
                }
                redraw
            }
            Event::Click { x, y } => {
                self.toolbar_pressed = None;
                if self.close_dialog != CloseDialog::None {
                    if let Some(action) = self.dialog_button_hit(x, y) {
                        return self.dialog_action(action);
                    }
                    return false;
                }
                if let Some(idx) = self.toolbar_hit(x, y) {
                    return self.handle_toolbar_click(idx);
                }
                false
            }
            Event::MouseDown { x, y, button: 0 } => {
                if self.close_dialog != CloseDialog::None {
                    return false;
                }
                self.toolbar_pressed = self.toolbar_hit(x, y);
                true
            }
            Event::MouseMove { x, y } => {
                if self.close_dialog != CloseDialog::None {
                    return false;
                }
                let hover = self.toolbar_hit(x, y);
                if hover != self.toolbar_hover {
                    self.toolbar_hover = hover;
                    return true;
                }
                false
            }
            Event::Key(ch) => self.handle_text_key(ch),
            Event::KeyPress {
                keycode,
                pressed,
                ctrl,
                ..
            } => self.handle_key_press(keycode, pressed, ctrl),
            _ => false,
        }
    }
}

fn push_usize(out: &mut String, mut n: usize) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut len = 0usize;
    while n > 0 {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
    }
    while len > 0 {
        len -= 1;
        out.push(digits[len] as char);
    }
}

fn push_u64(out: &mut String, mut n: u64) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut digits = [0u8; 24];
    let mut len = 0usize;
    while n > 0 {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
    }
    while len > 0 {
        len -= 1;
        out.push(digits[len] as char);
    }
}

fn read_utf8_file(path: &[u8]) -> Result<String, &'static str> {
    let fd = libc::open(path).map_err(|_| "Could not open file")?;
    let mut out = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        let n = libc::read(fd, &mut chunk).map_err(|_| "Read failed")?;
        if n == 0 {
            break;
        }
        if out.len() + n > MAX_FILE_BYTES {
            let _ = libc::close(fd);
            return Err("File too large");
        }
        out.extend_from_slice(&chunk[..n]);
    }
    let _ = libc::close(fd);
    core::str::from_utf8(&out)
        .map(String::from)
        .map_err(|_| "Invalid UTF-8")
}

fn write_utf8_file(path: &[u8], data: &[u8]) -> Result<(), &'static str> {
    let fd = libc::open_with_flags(path, libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC)
        .map_err(|_| "Could not open for writing")?;
    let mut offset = 0usize;
    while offset < data.len() {
        let n = libc::write(fd, &data[offset..]).map_err(|_| "Write failed")?;
        if n == 0 {
            let _ = libc::close(fd);
            return Err("Write stalled");
        }
        offset += n;
    }
    let _ = libc::close(fd);
    Ok(())
}

fn touch_empty_file(path: &[u8]) -> Result<(), &'static str> {
    write_utf8_file(path, &[])
}

fn create_temp_backing_path() -> Result<PathBuf, &'static str> {
    let pid = libc::getpid();
    let ticks = monotonic_millis();
    let mut candidates = [String::new(), String::new()];
    candidates[0].push_str("/tmp/sunedit-");
    push_u64(&mut candidates[0], pid);
    candidates[0].push('-');
    push_u64(&mut candidates[0], ticks);
    candidates[0].push_str(".txt");

    candidates[1].push_str("/root/sunedit-");
    push_u64(&mut candidates[1], pid);
    candidates[1].push('-');
    push_u64(&mut candidates[1], ticks);
    candidates[1].push_str(".txt");

    let _ = libc::mkdir_recursive(b"/tmp");
    for candidate in &candidates {
        let Some(path) = PathBuf::from_str(candidate) else {
            continue;
        };
        if touch_empty_file(path.as_bytes()).is_ok() {
            return Ok(path);
        }
    }
    Err("Could not create temporary file")
}

fn collect_argv(argc: u64, argv: *const *const u8) -> ([String; MAX_ARGC], usize) {
    let mut out = core::array::from_fn(|_| String::new());
    let mut raw = [core::ptr::null::<u8>(); MAX_ARGC];
    let count = unsafe { crt0::collect_raw_args(argc, argv, &mut raw) };
    let mut kept = 0usize;
    for i in 0..count.min(MAX_ARGC) {
        let len = unsafe { crt0::cstr_len(raw[i], PATH_LEN) };
        if len == 0 {
            continue;
        }
        let bytes = unsafe { core::slice::from_raw_parts(raw[i], len) };
        if let Ok(text) = core::str::from_utf8(bytes) {
            out[kept] = String::from(text);
            kept += 1;
        }
    }
    (out, kept)
}

fn parse_user_path_arg(argc: u64, argv: *const *const u8) -> Option<PathBuf> {
    let (args, count) = collect_argv(argc, argv);
    let slice: Vec<&str> = args[..count].iter().map(String::as_str).collect();
    // argv[0] is the executable path; user paths start after it.
    let user_args = if count > 0 { &slice[1..] } else { &[][..] };
    extract_first_real_file_path(user_args).and_then(PathBuf::from_str)
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=sunlight-edit",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );

    let pending_user_path = parse_user_path_arg(argc, argv);
    let mut app = EditApp::new(pending_user_path);

    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Edit",
        decoration: WindowDecoration::Normal,
    }) {
        Some(w) => w,
        None => {
            debug_log("[EDIT] failed to connect window\n");
            loop {
                process_yield();
            }
        }
    };

    window.run(&mut app);
    ProcessExit::exit(0);
}