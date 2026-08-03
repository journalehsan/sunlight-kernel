//! Reusable editable or read-only selectable multiline text surface.

use alloc::string::String;

use crate::event::Event;
use crate::font::VecText;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::Theme;

use super::text_buffer::{TextBuffer, TextPosition, TextRange};
use super::text_context_menu::{TextCommand, TextContextMenu, TextMenuState, TextWidgetKind};

const KEY_A: u8 = 0x1E;
const KEY_C: u8 = 0x2E;
const KEY_V: u8 = 0x2F;
const KEY_X: u8 = 0x2D;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_DELETE: u8 = 0x53;
const KEY_ESC: u8 = 0x01;
const DOUBLE_CLICK_MS: u64 = 350;
const TRIPLE_CLICK_MS: u64 = 520;
const WHEEL_SCROLL_LINES: usize = 3;

#[derive(Debug)]
pub struct TextEditorState {
    pub anchor: Option<TextPosition>,
    pub drag_anchor: Option<TextPosition>,
    pub drag_active: bool,
    pub preferred_col: Option<usize>,
    pub scroll_line: usize,
    pub focused: bool,
    last_click_ms: u64,
    last_click_pos: Option<TextPosition>,
    click_count: u8,
    hovered: bool,
    menu: Option<TextContextMenu>,
}

impl TextEditorState {
    pub const fn new() -> Self {
        Self {
            anchor: None,
            drag_anchor: None,
            drag_active: false,
            preferred_col: None,
            scroll_line: 0,
            focused: false,
            last_click_ms: 0,
            last_click_pos: None,
            click_count: 0,
            hovered: false,
            menu: None,
        }
    }

    pub fn selection_range(&self, buffer: &TextBuffer) -> Option<TextRange> {
        let anchor = self.anchor?;
        let range = buffer.normalized_range(anchor, buffer.cursor());
        (range.start != range.end).then_some(range)
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
        self.drag_anchor = None;
        self.drag_active = false;
    }

    pub fn has_selection(&self, buffer: &TextBuffer) -> bool {
        self.selection_range(buffer).is_some()
    }

    pub fn selected_text(&self, buffer: &TextBuffer) -> Option<String> {
        self.selection_range(buffer)
            .map(|range| buffer.extract_range(range.start, range.end))
    }

    pub fn close_context_menu(&mut self) {
        self.menu = None;
    }

    pub fn context_menu_open(&self) -> bool {
        self.menu.is_some()
    }
}

impl Default for TextEditorState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextEditorResponse {
    pub consumed: bool,
    pub changed: bool,
    pub selection_changed: bool,
    pub command: Option<TextCommand>,
    #[cfg(feature = "app")]
    pub clipboard_error: Option<crate::clipboard::ClipboardError>,
}

impl TextEditorResponse {
    fn consumed() -> Self {
        Self {
            consumed: true,
            ..Self::default()
        }
    }

    fn changed() -> Self {
        Self {
            consumed: true,
            changed: true,
            ..Self::default()
        }
    }

    fn selection() -> Self {
        Self {
            consumed: true,
            selection_changed: true,
            ..Self::default()
        }
    }
}

pub struct TextEditor<'a> {
    pub rect: Rect,
    buffer: &'a mut TextBuffer,
    state: &'a mut TextEditorState,
    font: Option<&'a dyn VecText>,
    editable: bool,
    caret_visible: bool,
    gutter_width: u32,
    menu_bounds: Option<Rect>,
    clipboard_source: &'a [u8],
}

impl<'a> TextEditor<'a> {
    pub fn new(rect: Rect, buffer: &'a mut TextBuffer, state: &'a mut TextEditorState) -> Self {
        Self {
            rect,
            buffer,
            state,
            font: None,
            editable: true,
            caret_visible: true,
            gutter_width: 0,
            menu_bounds: None,
            clipboard_source: b"sunlight-ui",
        }
    }

    /// Construct a read-only multiline surface that still supports selection,
    /// copy, select-all, a text context menu, and the I-beam cursor.
    pub fn selectable(
        rect: Rect,
        buffer: &'a mut TextBuffer,
        state: &'a mut TextEditorState,
    ) -> Self {
        Self::new(rect, buffer, state).read_only()
    }

    pub fn read_only(mut self) -> Self {
        self.editable = false;
        self
    }

    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
    }

    pub fn with_caret_visible(mut self, visible: bool) -> Self {
        self.caret_visible = visible;
        self
    }

    pub fn with_gutter_width(mut self, width: u32) -> Self {
        self.gutter_width = width.min(self.rect.w);
        self
    }

    pub fn with_menu_bounds(mut self, bounds: Rect) -> Self {
        self.menu_bounds = Some(bounds);
        self
    }

    pub fn with_clipboard_source(mut self, source: &'a [u8]) -> Self {
        self.clipboard_source = source;
        self
    }

    pub fn visible_line_count(&self) -> usize {
        (self.rect.h / self.line_height().max(1)).max(1) as usize
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        self.draw_surface(canvas, theme);
        self.draw_context_menu(canvas, theme);
    }

    pub fn draw_surface(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(self.rect, theme.bg);
        if self.gutter_width > 0 {
            let gutter = Rect::new(self.rect.x, self.rect.y, self.gutter_width, self.rect.h);
            canvas.fill_rect(gutter, theme.panel_alt);
            canvas.vline(gutter.right() - 1, gutter.y, gutter.h, theme.border);
        }
        if let Some(range) = self.state.selection_range(self.buffer) {
            self.draw_selection(canvas, theme, range);
        }

        let line_h = self.line_height() as i32;
        let text_x = self.text_x();
        for row in 0..self.visible_line_count() {
            let line_index = self.state.scroll_line + row;
            let y = self.rect.y + row as i32 * line_h;
            if y + line_h > self.rect.bottom() {
                break;
            }
            let Some(line) = self.buffer.line(line_index) else {
                break;
            };
            if self.gutter_width > 0 {
                let mut number = [0u8; 20];
                let label = decimal(line_index + 1, &mut number);
                let width = self.measure(label) as i32;
                self.draw_text(
                    canvas,
                    label,
                    self.rect.x + self.gutter_width as i32 - width - 6,
                    y,
                    theme.text_dim,
                );
            }
            self.draw_text(canvas, line, text_x, y, theme.text);
            if self.state.focused
                && self.editable
                && self.caret_visible
                && line_index == self.buffer.cursor_line
            {
                let prefix = prefix_chars(line, self.buffer.cursor_col);
                let x = text_x + self.measure(prefix) as i32;
                canvas.vline(x, y, self.line_height().saturating_sub(2), theme.accent);
            }
        }
    }

    pub fn draw_context_menu(&self, canvas: &mut Canvas, theme: &Theme) {
        if let Some(menu) = self.state.menu {
            menu.draw(canvas, theme, self.font);
        }
    }

    fn draw_selection(&self, canvas: &mut Canvas, theme: &Theme, range: TextRange) {
        let line_h = self.line_height() as i32;
        let text_x = self.text_x();
        for line_index in range.start.line..=range.end.line {
            if line_index < self.state.scroll_line {
                continue;
            }
            let row = line_index - self.state.scroll_line;
            let y = self.rect.y + row as i32 * line_h;
            if y >= self.rect.bottom() {
                break;
            }
            let line = self.buffer.line(line_index).unwrap_or("");
            let start_col = if line_index == range.start.line {
                range.start.col
            } else {
                0
            };
            let end_col = if line_index == range.end.line {
                range.end.col
            } else {
                self.buffer.line_len_chars(line_index)
            };
            let before = prefix_chars(line, start_col);
            let selected = char_slice(line, start_col, end_col);
            let x = text_x + self.measure(before) as i32;
            let width = self.measure(selected).max(4);
            canvas.fill_rect(
                Rect::new(
                    x,
                    y,
                    width,
                    self.line_height()
                        .saturating_sub(1)
                        .min(self.rect.bottom().saturating_sub(y) as u32),
                ),
                theme.chrome.selection,
            );
        }
    }

    pub fn update(&mut self, event: Event) -> TextEditorResponse {
        if self.state.menu.is_some() {
            match event {
                // Commands are selected by the release-side Click. Retain
                // menu ownership through the preceding button press.
                Event::MouseDown { .. } => return TextEditorResponse::consumed(),
                Event::MouseMove { .. } => {
                    #[cfg(feature = "app")]
                    crate::set_client_cursor(crate::CursorShape::Pointer);
                    self.state.hovered = false;
                    return TextEditorResponse::default();
                }
                _ => {}
            }
        }
        let response = match event {
            Event::Click { x, y } => self.handle_click(x, y),
            Event::MouseDown { x, y, button: 0 } => self.handle_left_down(x, y),
            Event::MouseDown { x, y, button: 1 } => self.handle_right_down(x, y),
            Event::MouseUp { button: 0, .. } => {
                self.state.drag_anchor = None;
                self.state.drag_active = false;
                TextEditorResponse::default()
            }
            Event::MouseMove { x, y } => {
                self.update_text_cursor(x, y);
                self.handle_drag(x, y)
            }
            Event::MouseWheel { x, y, delta } => self.handle_mouse_wheel(x, y, delta),
            Event::Key(ch) if self.state.focused && self.editable && self.state.menu.is_none() => {
                self.handle_text(ch)
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                shift,
                ctrl,
                alt: false,
                super_key: false,
            } if self.state.focused => {
                if keycode == KEY_ESC && self.state.menu.take().is_some() {
                    TextEditorResponse::consumed()
                } else if ctrl {
                    self.handle_shortcut(keycode)
                } else if self.state.menu.is_none() {
                    self.handle_navigation_or_delete(keycode, shift)
                } else {
                    TextEditorResponse::default()
                }
            }
            Event::FocusChanged { focused: false } => {
                self.state.focused = false;
                self.state.drag_anchor = None;
                self.state.drag_active = false;
                self.state.menu = None;
                TextEditorResponse::consumed()
            }
            Event::PointerOwnership { owned: false, .. } => {
                self.state.drag_anchor = None;
                self.state.drag_active = false;
                self.state.hovered = false;
                TextEditorResponse::default()
            }
            _ => TextEditorResponse::default(),
        };
        if response.changed || response.selection_changed {
            self.ensure_cursor_visible();
        }
        response
    }

    fn handle_mouse_wheel(&mut self, x: i32, y: i32, delta: i16) -> TextEditorResponse {
        if delta == 0 || !self.rect.contains(Point::new(x, y)) {
            return TextEditorResponse::default();
        }

        let detents = if delta.unsigned_abs() >= 120 {
            delta as i32 / 120
        } else {
            delta.signum() as i32
        };
        let line_delta = detents.saturating_mul(WHEEL_SCROLL_LINES as i32);
        let old_scroll = self.state.scroll_line;
        let max_scroll = self
            .buffer
            .line_count()
            .saturating_sub(self.visible_line_count());
        self.state.scroll_line = if line_delta > 0 {
            old_scroll
                .saturating_add(line_delta as usize)
                .min(max_scroll)
        } else {
            old_scroll.saturating_sub(line_delta.unsigned_abs() as usize)
        };

        if self.state.scroll_line != old_scroll {
            TextEditorResponse::consumed()
        } else {
            TextEditorResponse::default()
        }
    }

    /// Execute a widget-local editing command (for example from an application
    /// toolbar) through the same path used by shortcuts and the context menu.
    pub fn command(&mut self, command: TextCommand) -> TextEditorResponse {
        let response = self.execute(command);
        if response.changed || response.selection_changed {
            self.ensure_cursor_visible();
        }
        response
    }

    fn handle_click(&mut self, x: i32, y: i32) -> TextEditorResponse {
        let point = Point::new(x, y);
        if let Some(menu) = self.state.menu.take() {
            if let Some(command) = menu.command_at(point) {
                return self.execute(command);
            }
            return TextEditorResponse::consumed();
        }
        if self.state.drag_anchor.take().is_some() {
            self.state.drag_active = false;
            return TextEditorResponse::selection();
        }
        if !self.rect.contains(point) {
            let consumed = self.state.focused;
            self.state.focused = false;
            return TextEditorResponse {
                consumed,
                ..TextEditorResponse::default()
            };
        }
        self.state.focused = true;
        self.state.clear_selection();
        self.buffer.set_cursor(self.position_at(x, y));
        TextEditorResponse::selection()
    }

    fn handle_left_down(&mut self, x: i32, y: i32) -> TextEditorResponse {
        if !self.rect.contains(Point::new(x, y)) {
            return TextEditorResponse::default();
        }
        self.state.focused = true;
        self.state.menu = None;
        let position = self.position_at(x, y);
        let now = current_millis();
        let same_spot = self.state.last_click_pos == Some(position);
        let elapsed = now.saturating_sub(self.state.last_click_ms);
        if same_spot && elapsed <= TRIPLE_CLICK_MS && self.state.click_count >= 2 {
            if let Some(range) = self.buffer.line_range_at(position.line) {
                self.buffer.set_cursor(range.end);
                self.state.anchor = Some(range.start);
            }
            self.state.click_count = 3;
        } else if same_spot && elapsed <= DOUBLE_CLICK_MS {
            if let Some(range) = self.buffer.word_range_at(position) {
                self.buffer.set_cursor(range.end);
                self.state.anchor = Some(range.start);
            } else {
                self.buffer.set_cursor(position);
                self.state.clear_selection();
            }
            self.state.click_count = 2;
        } else {
            self.buffer.set_cursor(position);
            self.state.clear_selection();
            self.state.drag_anchor = Some(position);
            self.state.click_count = 1;
        }
        self.state.last_click_ms = now;
        self.state.last_click_pos = Some(position);
        TextEditorResponse::selection()
    }

    fn handle_right_down(&mut self, x: i32, y: i32) -> TextEditorResponse {
        if !self.rect.contains(Point::new(x, y)) {
            return TextEditorResponse::default();
        }
        self.state.focused = true;
        let position = self.position_at(x, y);
        let inside_selection = self
            .state
            .selection_range(self.buffer)
            .is_some_and(|range| position >= range.start && position <= range.end);
        if !inside_selection {
            self.buffer.set_cursor(position);
            self.state.clear_selection();
        }
        let kind = if self.editable {
            TextWidgetKind::EditableMultiline
        } else {
            TextWidgetKind::SelectableReadOnly
        };
        let state = TextMenuState {
            kind,
            has_selection: self.state.has_selection(self.buffer),
            has_text: !self.buffer.is_content_empty(),
            can_paste: self.editable && clipboard_text_available(),
            can_delete: self.editable
                && (self.state.has_selection(self.buffer)
                    || self.buffer.cursor() != self.buffer.document_end()),
            can_undo: false,
            can_redo: false,
        };
        self.state.menu = Some(TextContextMenu::open_at(
            x,
            y,
            self.resolved_menu_bounds(),
            state,
        ));
        TextEditorResponse::consumed()
    }

    fn handle_drag(&mut self, x: i32, y: i32) -> TextEditorResponse {
        let Some(anchor) = self.state.drag_anchor else {
            return TextEditorResponse::default();
        };
        if !self.rect.contains(Point::new(x, y)) {
            return TextEditorResponse::default();
        }
        let position = self.position_at(x, y);
        self.state.drag_active = true;
        self.state.anchor = Some(anchor);
        self.buffer.set_cursor(position);
        TextEditorResponse::selection()
    }

    fn handle_shortcut(&mut self, keycode: u8) -> TextEditorResponse {
        let command = match keycode {
            KEY_A => Some(TextCommand::SelectAll),
            KEY_C => Some(TextCommand::Copy),
            KEY_X if self.editable => Some(TextCommand::Cut),
            KEY_V if self.editable => Some(TextCommand::Paste),
            _ => None,
        };
        command
            .map(|command| self.execute(command))
            .unwrap_or_default()
    }

    fn handle_navigation_or_delete(&mut self, keycode: u8, shift: bool) -> TextEditorResponse {
        if keycode == KEY_DELETE && self.editable {
            let changed = self.delete_selection() || self.buffer.delete_forward();
            return if changed {
                TextEditorResponse::changed()
            } else {
                TextEditorResponse::consumed()
            };
        }
        let before = self.buffer.cursor();
        let moved = match keycode {
            KEY_LEFT => self.buffer.move_left(),
            KEY_RIGHT => self.buffer.move_right(),
            KEY_UP => {
                let preferred = self.state.preferred_col.unwrap_or(self.buffer.cursor_col);
                let moved = self.buffer.move_up();
                if moved {
                    self.buffer.cursor_col =
                        preferred.min(self.buffer.line_len_chars(self.buffer.cursor_line));
                    self.state.preferred_col = Some(preferred);
                }
                moved
            }
            KEY_DOWN => {
                let preferred = self.state.preferred_col.unwrap_or(self.buffer.cursor_col);
                let moved = self.buffer.move_down();
                if moved {
                    self.buffer.cursor_col =
                        preferred.min(self.buffer.line_len_chars(self.buffer.cursor_line));
                    self.state.preferred_col = Some(preferred);
                }
                moved
            }
            KEY_HOME => self.buffer.move_home(),
            KEY_END => self.buffer.move_end(),
            _ => false,
        };
        if !moved {
            return TextEditorResponse::default();
        }
        if !matches!(keycode, KEY_UP | KEY_DOWN) {
            self.state.preferred_col = None;
        }
        if shift {
            self.state.anchor.get_or_insert(before);
        } else {
            self.state.clear_selection();
        }
        TextEditorResponse::selection()
    }

    fn handle_text(&mut self, ch: char) -> TextEditorResponse {
        let changed = match ch {
            '\u{8}' => self.delete_selection() || self.buffer.backspace(),
            '\n' => self.replace_selection_with("\n"),
            '\r' => false,
            value if !value.is_control() => {
                let mut bytes = [0u8; 4];
                self.replace_selection_with(value.encode_utf8(&mut bytes))
            }
            _ => false,
        };
        if changed {
            self.state.preferred_col = None;
            TextEditorResponse::changed()
        } else {
            TextEditorResponse::default()
        }
    }

    fn execute(&mut self, command: TextCommand) -> TextEditorResponse {
        let mut response = TextEditorResponse {
            consumed: true,
            command: Some(command),
            ..TextEditorResponse::default()
        };
        match command {
            TextCommand::Copy => {
                let Some(text) = self.state.selected_text(self.buffer) else {
                    response.command = None;
                    return response;
                };
                #[cfg(not(feature = "app"))]
                let _ = &text;
                #[cfg(feature = "app")]
                if let Err(error) = crate::clipboard::set_text_from(self.clipboard_source, &text) {
                    response.clipboard_error = Some(error);
                }
            }
            TextCommand::Cut if self.editable => {
                let Some(text) = self.state.selected_text(self.buffer) else {
                    response.command = None;
                    return response;
                };
                #[cfg(not(feature = "app"))]
                let _ = &text;
                #[cfg(feature = "app")]
                match crate::clipboard::set_text_from(self.clipboard_source, &text) {
                    Ok(()) => response.changed = self.delete_selection(),
                    Err(error) => response.clipboard_error = Some(error),
                }
            }
            TextCommand::Paste if self.editable =>
            {
                #[cfg(feature = "app")]
                match crate::clipboard::get_text() {
                    Ok(text) => response.changed = self.replace_selection_with(&text),
                    Err(error) => response.clipboard_error = Some(error),
                }
            }
            TextCommand::Delete if self.editable => {
                response.changed = self.delete_selection() || self.buffer.delete_forward();
                if !response.changed {
                    response.command = None;
                }
            }
            TextCommand::SelectAll => {
                if self.buffer.is_content_empty() {
                    response.command = None;
                    return response;
                }
                let range = self.buffer.select_all_range();
                self.buffer.set_cursor(range.end);
                self.state.anchor = Some(range.start);
                response.selection_changed = true;
            }
            TextCommand::Undo
            | TextCommand::Redo
            | TextCommand::Cut
            | TextCommand::Paste
            | TextCommand::Delete => {}
        }
        response
    }

    fn delete_selection(&mut self) -> bool {
        let Some(range) = self.state.selection_range(self.buffer) else {
            return false;
        };
        let changed = self.buffer.delete_range(range.start, range.end);
        if changed {
            self.state.clear_selection();
        }
        changed
    }

    fn replace_selection_with(&mut self, text: &str) -> bool {
        if let Some(range) = self.state.selection_range(self.buffer) {
            let changed = self.buffer.replace_range(range.start, range.end, text);
            self.state.clear_selection();
            changed
        } else {
            self.buffer.insert_text(text)
        }
    }

    fn ensure_cursor_visible(&mut self) {
        let visible = self.visible_line_count();
        if self.buffer.cursor_line < self.state.scroll_line {
            self.state.scroll_line = self.buffer.cursor_line;
        } else if self.buffer.cursor_line >= self.state.scroll_line + visible {
            self.state.scroll_line = self.buffer.cursor_line + 1 - visible;
        }
    }

    fn position_at(&self, x: i32, y: i32) -> TextPosition {
        let line_h = self.line_height().max(1) as i32;
        let row = ((y - self.rect.y).max(0) / line_h) as usize;
        let line = (self.state.scroll_line + row).min(self.buffer.line_count().saturating_sub(1));
        let relative_x = (x - self.text_x()).max(0) as u32;
        let text = self.buffer.line(line).unwrap_or("");
        let mut col = 0usize;
        for (index, ch) in text.char_indices() {
            let end = index + ch.len_utf8();
            let left = self.measure(&text[..index]);
            let right = self.measure(&text[..end]);
            if relative_x < left + (right.saturating_sub(left) / 2) {
                return TextPosition { line, col };
            }
            col += 1;
            if relative_x < right {
                return TextPosition { line, col };
            }
        }
        TextPosition { line, col }
    }

    fn text_x(&self) -> i32 {
        self.rect.x + self.gutter_width as i32 + if self.gutter_width > 0 { 8 } else { 6 }
    }

    fn line_height(&self) -> u32 {
        self.font
            .map(|font| font.line_height().max(1))
            .unwrap_or(crate::paint::font::GLYPH_H.max(1))
    }

    fn measure(&self, text: &str) -> u32 {
        self.font
            .map(|font| font.measure_w(text))
            .unwrap_or_else(|| Canvas::measure_text(text))
    }

    fn draw_text(
        &self,
        canvas: &mut Canvas,
        text: &str,
        x: i32,
        y: i32,
        color: crate::theme::Color,
    ) {
        if let Some(font) = self.font {
            font.draw(canvas, text, x, y, color);
        } else {
            canvas.draw_text(x, y, text, color);
        }
    }

    fn update_text_cursor(&mut self, x: i32, y: i32) {
        let hovered = self.rect.contains(Point::new(x, y));
        #[cfg(feature = "app")]
        if hovered != self.state.hovered {
            crate::set_client_cursor(if hovered {
                crate::CursorShape::Text
            } else {
                crate::CursorShape::Pointer
            });
        }
        self.state.hovered = hovered;
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

fn prefix_chars(text: &str, count: usize) -> &str {
    let end = text
        .char_indices()
        .nth(count)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    &text[..end]
}

fn char_slice(text: &str, start: usize, end: usize) -> &str {
    let start = text
        .char_indices()
        .nth(start)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let end = text
        .char_indices()
        .nth(end)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    &text[start..end.max(start)]
}

fn decimal(mut value: usize, buffer: &mut [u8; 20]) -> &str {
    if value == 0 {
        buffer[19] = b'0';
        return core::str::from_utf8(&buffer[19..]).unwrap_or("0");
    }
    let mut index = buffer.len();
    while value > 0 && index > 0 {
        index -= 1;
        buffer[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    core::str::from_utf8(&buffer[index..]).unwrap_or("")
}

#[cfg(feature = "app")]
fn current_millis() -> u64 {
    sunlight_ipc::monotonic_millis()
}

#[cfg(not(feature = "app"))]
fn current_millis() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::{TextEditor, TextEditorState, KEY_A, KEY_DELETE, KEY_LEFT};
    use crate::widgets::TextBuffer;
    use crate::{Event, Rect};

    #[test]
    fn editable_surface_handles_selection_and_deletion() {
        let mut buffer = TextBuffer::from_utf8("hello\nworld");
        let mut state = TextEditorState::new();
        state.focused = true;
        buffer.move_end();
        let mut editor = TextEditor::new(Rect::new(0, 0, 200, 100), &mut buffer, &mut state);
        assert!(
            editor
                .update(Event::key_press(KEY_LEFT, true, true, false, false, false))
                .selection_changed
        );
        assert!(
            editor
                .update(Event::key_press(
                    KEY_DELETE, true, false, false, false, false
                ))
                .changed
        );
        assert_eq!(buffer.line(0), Some("hell"));
    }

    #[test]
    fn read_only_surface_selects_but_does_not_delete() {
        let mut buffer = TextBuffer::from_utf8("hello");
        let mut state = TextEditorState::new();
        state.focused = true;
        let mut editor =
            TextEditor::new(Rect::new(0, 0, 200, 100), &mut buffer, &mut state).read_only();
        assert!(
            editor
                .update(Event::key_press(KEY_A, true, false, true, false, false))
                .selection_changed
        );
        assert!(
            !editor
                .update(Event::key_press(
                    KEY_DELETE, true, false, false, false, false
                ))
                .changed
        );
        assert_eq!(buffer.to_utf8_string(), "hello");
    }

    #[test]
    fn context_menu_keeps_mouse_press_until_click() {
        let mut buffer = TextBuffer::from_utf8("hello");
        let mut state = TextEditorState::new();
        let rect = Rect::new(0, 0, 200, 100);
        let mut editor = TextEditor::new(rect, &mut buffer, &mut state);
        assert!(editor.update(Event::mouse_down(10, 10, 1)).consumed);
        assert!(editor.state.context_menu_open());
        assert!(editor.update(Event::mouse_down(12, 12, 0)).consumed);
        assert!(editor.state.context_menu_open());
    }

    #[test]
    fn mouse_wheel_scrolls_and_clamps_inside_editor() {
        let mut buffer = TextBuffer::from_utf8("0\n1\n2\n3\n4\n5\n6\n7\n8\n9");
        let mut state = TextEditorState::new();
        let rect = Rect::new(0, 0, 200, 1);
        let mut editor = TextEditor::new(rect, &mut buffer, &mut state);

        assert!(editor.update(Event::mouse_wheel(10, 0, 1)).consumed);
        assert_eq!(editor.state.scroll_line, 3);

        assert!(editor.update(Event::mouse_wheel(10, 0, 120)).consumed);
        assert_eq!(editor.state.scroll_line, 6);

        assert!(editor.update(Event::mouse_wheel(10, 0, -1)).consumed);
        assert_eq!(editor.state.scroll_line, 3);

        assert!(!editor.update(Event::mouse_wheel(250, 0, 1)).consumed);
        assert_eq!(editor.state.scroll_line, 3);

        assert!(editor.update(Event::mouse_wheel(10, 0, 1200)).consumed);
        assert_eq!(editor.state.scroll_line, 9);
        assert!(!editor.update(Event::mouse_wheel(10, 0, 1)).consumed);
    }
}
