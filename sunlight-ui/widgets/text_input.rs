//! Editable, selectable single-line text input.

use crate::event::Event;
use crate::font::VecText;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::Theme;

use super::text_context_menu::{TextCommand, TextContextMenu, TextMenuState, TextWidgetKind};

const KEY_A: u8 = 0x1E;
const KEY_C: u8 = 0x2E;
const KEY_V: u8 = 0x2F;
const KEY_X: u8 = 0x2D;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_DELETE: u8 = 0x53;
const KEY_ESC: u8 = 0x01;

pub struct TextInput<'a, const N: usize> {
    pub rect: Rect,
    pub active: bool,
    len: usize,
    cursor: usize,
    selection_anchor: Option<usize>,
    drag_anchor: Option<usize>,
    hovered: bool,
    menu: Option<TextContextMenu>,
    menu_bounds: Option<Rect>,
    buf: [u8; N],
    font: Option<&'a dyn VecText>,
    placeholder: Option<&'a str>,
    clipboard_source: &'a [u8],
}

impl<'a, const N: usize> TextInput<'a, N> {
    pub const fn new(rect: Rect) -> Self {
        Self {
            rect,
            active: false,
            len: 0,
            cursor: 0,
            selection_anchor: None,
            drag_anchor: None,
            hovered: false,
            menu: None,
            menu_bounds: None,
            buf: [0; N],
            font: None,
            placeholder: None,
            clipboard_source: b"sunlight-ui",
        }
    }

    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
    }

    pub fn with_placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = Some(placeholder);
        self
    }

    pub fn with_clipboard_source(mut self, source: &'a [u8]) -> Self {
        self.clipboard_source = source;
        self
    }

    pub fn with_menu_bounds(mut self, bounds: Rect) -> Self {
        self.menu_bounds = Some(bounds);
        self
    }

    pub fn value(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    pub fn set_text(&mut self, text: &str) {
        self.len = floor_char_boundary(text, text.len().min(N));
        self.cursor = self.len;
        self.selection_anchor = None;
        self.buf[..self.len].copy_from_slice(&text.as_bytes()[..self.len]);
    }

    pub fn has_selection(&self) -> bool {
        self.selection_range().is_some()
    }

    pub fn context_menu_open(&self) -> bool {
        self.menu.is_some()
    }

    pub fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selection_range()?;
        self.value().get(start..end)
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(self.rect, theme.panel);
        canvas.draw_rect(
            self.rect,
            if self.active {
                theme.accent
            } else {
                theme.border
            },
        );

        let text_x = self.rect.x + 6;
        let show_placeholder = self.len == 0 && !self.active;
        let (visible, visible_start) = if show_placeholder {
            (self.placeholder.unwrap_or(""), 0)
        } else {
            self.visible_text()
        };
        let visible_cursor = self.cursor.saturating_sub(visible_start).min(visible.len());
        let text_color = if show_placeholder {
            theme.text_dim
        } else {
            theme.text
        };

        if !show_placeholder {
            self.draw_selection(canvas, theme, visible, visible_start, text_x);
        }
        if let Some(font) = self.font {
            font.draw_vcenter(
                canvas,
                visible,
                text_x,
                self.rect.y,
                self.rect.h,
                text_color,
            );
            if self.active {
                let cursor_x = text_x + font.measure_w(&visible[..visible_cursor]) as i32;
                canvas.vline(
                    cursor_x,
                    self.rect.y + 4,
                    self.rect.h.saturating_sub(8),
                    theme.accent,
                );
            }
        } else {
            let text_y = self.rect.y + (self.rect.h as i32 - 10) / 2;
            canvas.draw_text(text_x, text_y, visible, text_color);
            if self.active {
                let cursor_x = text_x + Canvas::measure_text(&visible[..visible_cursor]) as i32;
                canvas.vline(
                    cursor_x,
                    self.rect.y + 4,
                    self.rect.h.saturating_sub(8),
                    theme.accent,
                );
            }
        }

        if let Some(menu) = self.menu {
            menu.draw(canvas, theme, self.font);
        }
    }

    fn draw_selection(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        visible: &str,
        visible_start: usize,
        text_x: i32,
    ) {
        let Some((start, end)) = self.selection_range() else {
            return;
        };
        let visible_end = visible_start + visible.len();
        let start = start.max(visible_start).min(visible_end);
        let end = end.max(visible_start).min(visible_end);
        if start >= end {
            return;
        }
        let before = &visible[..start - visible_start];
        let selected = &visible[start - visible_start..end - visible_start];
        let x = text_x + self.measure(before) as i32;
        let width = self.measure(selected).max(2);
        canvas.fill_rect(
            Rect::new(x, self.rect.y + 3, width, self.rect.h.saturating_sub(6)),
            theme.chrome.selection,
        );
    }

    fn visible_text(&self) -> (&str, usize) {
        let value = self.value();
        let max_chars = self.max_visible_chars();
        if value.chars().count() <= max_chars {
            return (value, 0);
        }
        let cursor_chars = value[..self.cursor].chars().count();
        let total_chars = value.chars().count();
        let mut start_chars = cursor_chars.saturating_sub(max_chars.saturating_sub(1));
        if start_chars + max_chars > total_chars {
            start_chars = total_chars.saturating_sub(max_chars);
        }
        let start_byte = nth_char_byte(value, start_chars);
        let end_byte = nth_char_byte(value, (start_chars + max_chars).min(total_chars));
        (&value[start_byte..end_byte], start_byte)
    }

    fn max_visible_chars(&self) -> usize {
        let inner_w = self.rect.w.saturating_sub(12);
        let glyph_w = self
            .font
            .map(|font| font.measure_w("M").max(1) as usize)
            .unwrap_or(8);
        ((inner_w as usize) / glyph_w).max(1)
    }

    pub fn update(&mut self, event: Event) -> bool {
        if self.menu.is_some() {
            match event {
                // The release-side Click selects or dismisses the menu. Keep
                // the press from falling through to the underlying field.
                Event::MouseDown { .. } => return true,
                Event::MouseMove { .. } => {
                    #[cfg(feature = "app")]
                    crate::set_client_cursor(crate::CursorShape::Pointer);
                    self.hovered = false;
                    return false;
                }
                _ => {}
            }
        }
        match event {
            Event::Click { x, y } => self.handle_click(x, y),
            Event::MouseDown { x, y, button: 0 } => self.begin_selection(x, y),
            Event::MouseDown { x, y, button: 1 } => self.open_context_menu(x, y),
            Event::MouseUp { button: 0, .. } => {
                self.drag_anchor = None;
                false
            }
            Event::MouseMove { x, y } => {
                self.update_text_cursor(x, y);
                if self.drag_anchor.is_some() {
                    self.extend_selection(x, y)
                } else {
                    false
                }
            }
            Event::Key(ch) if self.active && self.menu.is_none() => match ch {
                '\u{8}' => self.backspace(),
                '\n' | '\r' => false,
                c if !c.is_control() => self.insert_char(c),
                _ => false,
            },
            Event::KeyPress {
                keycode,
                pressed: true,
                shift,
                ctrl,
                alt: false,
                super_key: false,
            } if self.active => {
                if keycode == KEY_ESC && self.menu.take().is_some() {
                    true
                } else if ctrl {
                    self.handle_shortcut(keycode)
                } else if self.menu.is_none() {
                    self.handle_navigation(keycode, shift)
                } else {
                    false
                }
            }
            Event::FocusChanged { focused: false } => {
                let changed = self.active || self.menu.is_some();
                self.active = false;
                self.drag_anchor = None;
                self.menu = None;
                changed
            }
            Event::PointerOwnership { owned: false, .. } => {
                self.drag_anchor = None;
                self.hovered = false;
                false
            }
            _ => false,
        }
    }

    fn handle_click(&mut self, x: i32, y: i32) -> bool {
        let point = Point::new(x, y);
        if let Some(menu) = self.menu.take() {
            if let Some(command) = menu.command_at(point) {
                return self.execute(command) || true;
            }
            return true;
        }
        let inside = self.rect.contains(point);
        let changed = self.active != inside;
        self.active = inside;
        if inside {
            if self.drag_anchor.take().is_none() {
                self.cursor = self.byte_at_x(x);
                self.selection_anchor = None;
            }
            true
        } else {
            self.drag_anchor = None;
            changed
        }
    }

    fn begin_selection(&mut self, x: i32, y: i32) -> bool {
        if !self.rect.contains(Point::new(x, y)) {
            return false;
        }
        self.active = true;
        self.cursor = self.byte_at_x(x);
        self.selection_anchor = None;
        self.drag_anchor = Some(self.cursor);
        self.menu = None;
        true
    }

    fn extend_selection(&mut self, x: i32, y: i32) -> bool {
        if !self.rect.contains(Point::new(x, y)) {
            return false;
        }
        let anchor = self.drag_anchor.unwrap_or(self.cursor);
        let cursor = self.byte_at_x(x);
        let changed = self.cursor != cursor || self.selection_anchor != Some(anchor);
        self.cursor = cursor;
        self.selection_anchor = Some(anchor);
        changed
    }

    fn open_context_menu(&mut self, x: i32, y: i32) -> bool {
        if !self.rect.contains(Point::new(x, y)) {
            return false;
        }
        self.active = true;
        let clicked = self.byte_at_x(x);
        if !self.selection_contains(clicked) {
            self.cursor = clicked;
            self.selection_anchor = None;
        }
        let state = TextMenuState {
            kind: TextWidgetKind::EditableSingleLine,
            has_selection: self.has_selection(),
            has_text: self.len != 0,
            can_paste: clipboard_text_available(),
            can_delete: self.has_selection() || self.cursor < self.len,
            can_undo: false,
            can_redo: false,
        };
        self.menu = Some(TextContextMenu::open_at(
            x,
            y,
            self.resolved_menu_bounds(),
            state,
        ));
        true
    }

    fn handle_shortcut(&mut self, keycode: u8) -> bool {
        let command = match keycode {
            KEY_A => Some(TextCommand::SelectAll),
            KEY_C => Some(TextCommand::Copy),
            KEY_X => Some(TextCommand::Cut),
            KEY_V => Some(TextCommand::Paste),
            _ => None,
        };
        command
            .map(|command| self.execute(command) || true)
            .unwrap_or(false)
    }

    fn handle_navigation(&mut self, keycode: u8, shift: bool) -> bool {
        if keycode == KEY_DELETE {
            return self.delete_forward();
        }
        let before = self.cursor;
        let moved = match keycode {
            KEY_LEFT => self.move_left(),
            KEY_RIGHT => self.move_right(),
            KEY_HOME => self.move_home(),
            KEY_END => self.move_end(),
            _ => false,
        };
        if moved {
            if shift {
                self.selection_anchor.get_or_insert(before);
            } else {
                self.selection_anchor = None;
            }
        }
        moved
    }

    fn execute(&mut self, command: TextCommand) -> bool {
        match command {
            TextCommand::Cut => self.cut(),
            TextCommand::Copy => self.copy(),
            TextCommand::Paste => self.paste(),
            TextCommand::Delete => self.delete_forward(),
            TextCommand::SelectAll => self.select_all(),
            TextCommand::Undo | TextCommand::Redo => false,
        }
    }

    fn copy(&self) -> bool {
        let Some(text) = self.selected_text() else {
            return false;
        };
        #[cfg(feature = "app")]
        {
            return crate::clipboard::set_text_from(self.clipboard_source, text).is_ok();
        }
        #[cfg(not(feature = "app"))]
        {
            let _ = text;
            false
        }
    }

    fn cut(&mut self) -> bool {
        if !self.copy() {
            return false;
        }
        self.delete_selection()
    }

    fn paste(&mut self) -> bool {
        #[cfg(feature = "app")]
        {
            if let Ok(text) = crate::clipboard::get_text() {
                return self.replace_selection(&text);
            }
        }
        false
    }

    fn select_all(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }
        self.selection_anchor = Some(0);
        self.cursor = self.len;
        true
    }

    fn insert_char(&mut self, ch: char) -> bool {
        let mut encoded = [0u8; 4];
        self.replace_selection(ch.encode_utf8(&mut encoded))
    }

    fn replace_selection(&mut self, text: &str) -> bool {
        let (start, end) = self.selection_range().unwrap_or((self.cursor, self.cursor));
        let retained = self.len - (end - start);
        let insert_len = floor_char_boundary(text, text.len().min(N.saturating_sub(retained)));
        if start == end && insert_len == 0 {
            return false;
        }
        self.buf.copy_within(end..self.len, start + insert_len);
        self.buf[start..start + insert_len].copy_from_slice(&text.as_bytes()[..insert_len]);
        self.len = retained + insert_len;
        self.cursor = start + insert_len;
        self.selection_anchor = None;
        true
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection_range() else {
            return false;
        };
        self.buf.copy_within(end..self.len, start);
        self.len -= end - start;
        self.cursor = start;
        self.selection_anchor = None;
        true
    }

    fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor == 0 {
            return false;
        }
        let start = previous_char_boundary(self.value(), self.cursor);
        self.buf.copy_within(self.cursor..self.len, start);
        self.len -= self.cursor - start;
        self.cursor = start;
        true
    }

    fn delete_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor >= self.len {
            return false;
        }
        let end = next_char_boundary(self.value(), self.cursor);
        self.buf.copy_within(end..self.len, self.cursor);
        self.len -= end - self.cursor;
        true
    }

    fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = previous_char_boundary(self.value(), self.cursor);
        true
    }

    fn move_right(&mut self) -> bool {
        if self.cursor >= self.len {
            return false;
        }
        self.cursor = next_char_boundary(self.value(), self.cursor);
        true
    }

    fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    fn move_end(&mut self) -> bool {
        if self.cursor == self.len {
            return false;
        }
        self.cursor = self.len;
        true
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.selection_anchor?;
        if anchor == self.cursor {
            None
        } else {
            Some((anchor.min(self.cursor), anchor.max(self.cursor)))
        }
    }

    fn selection_contains(&self, offset: usize) -> bool {
        self.selection_range()
            .is_some_and(|(start, end)| offset >= start && offset <= end)
    }

    fn byte_at_x(&self, x: i32) -> usize {
        let (visible, start) = self.visible_text();
        let relative = (x - self.rect.x - 6).max(0) as u32;
        let mut previous = 0usize;
        for (index, ch) in visible.char_indices() {
            let next = index + ch.len_utf8();
            let left = self.measure(&visible[..index]);
            let right = self.measure(&visible[..next]);
            if relative < left + (right.saturating_sub(left) / 2) {
                return start + previous;
            }
            if relative < right {
                return start + next;
            }
            previous = next;
        }
        start + visible.len()
    }

    fn measure(&self, text: &str) -> u32 {
        self.font
            .map(|font| font.measure_w(text))
            .unwrap_or_else(|| Canvas::measure_text(text))
    }

    fn update_text_cursor(&mut self, x: i32, y: i32) {
        let hovered = self.rect.contains(Point::new(x, y));
        #[cfg(feature = "app")]
        if hovered != self.hovered {
            crate::set_client_cursor(if hovered {
                crate::CursorShape::Text
            } else {
                crate::CursorShape::Pointer
            });
        }
        self.hovered = hovered;
    }

    fn resolved_menu_bounds(&self) -> Rect {
        if let Some(bounds) = self.menu_bounds {
            return bounds;
        }
        #[cfg(feature = "app")]
        if let Some(bounds) = crate::app::active_client_bounds() {
            return bounds;
        }
        Rect::new(0, 0, 4096, 4096)
    }
}

fn clipboard_text_available() -> bool {
    #[cfg(feature = "app")]
    {
        crate::clipboard::text_available()
    }
    #[cfg(not(feature = "app"))]
    {
        false
    }
}

fn nth_char_byte(text: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }
    text.char_indices()
        .nth(char_index)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len())
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn previous_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.saturating_sub(1);
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn next_char_boundary(text: &str, index: usize) -> usize {
    let mut index = (index + 1).min(text.len());
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::TextInput;
    use crate::{Event, Rect};

    #[test]
    fn utf8_editing_keeps_boundaries_valid() {
        let mut input = TextInput::<12>::new(Rect::new(0, 0, 100, 24));
        input.set_text("سلام");
        assert!(input.update(Event::key('\u{8}')) == false);
        input.active = true;
        assert!(input.update(Event::key('\u{8}')));
        assert_eq!(input.value(), "سلا");
        assert!(input.update(Event::key('م')));
        assert_eq!(input.value(), "سلام");
    }

    #[test]
    fn shift_navigation_creates_selection() {
        let mut input = TextInput::<16>::new(Rect::new(0, 0, 100, 24));
        input.set_text("abc");
        input.active = true;
        assert!(input.update(Event::key_press(
            super::KEY_LEFT,
            true,
            true,
            false,
            false,
            false,
        )));
        assert_eq!(input.selected_text(), Some("c"));
    }

    #[test]
    fn select_all_and_delete_use_local_shortcuts() {
        let mut input = TextInput::<16>::new(Rect::new(0, 0, 100, 24));
        input.set_text("abc");
        input.active = true;
        assert!(input.update(Event::key_press(
            super::KEY_A,
            true,
            false,
            true,
            false,
            false,
        )));
        assert_eq!(input.selected_text(), Some("abc"));
        assert!(input.update(Event::key_press(
            super::KEY_DELETE,
            true,
            false,
            false,
            false,
            false,
        )));
        assert_eq!(input.value(), "");
    }

    #[test]
    fn context_menu_keeps_mouse_press_until_click() {
        let mut input = TextInput::<16>::new(Rect::new(0, 0, 100, 24));
        input.set_text("abc");
        assert!(input.update(Event::mouse_down(10, 10, 1)));
        assert!(input.context_menu_open());
        assert!(input.update(Event::mouse_down(12, 12, 0)));
        assert!(input.context_menu_open());
    }
}
