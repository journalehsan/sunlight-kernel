//! UTF-8 rich-document editing state for [`DocumentCanvas`].
//!
//! Logical positions are byte offsets into `text`; cached [`TextLineLayout`]
//! entries map them to visual wrapped lines and pixels. Soft wrapping never
//! changes `text`.

use alloc::vec::Vec;

use crate::font::VecText;

use super::document_canvas::{
    byte_at_x_on_line, caret_x_on_line, find_line_index, layout_rich_text_lines, layout_text_lines,
    line_end_byte, line_home_byte, rich_byte_at_x_on_line, rich_caret_x_on_line, RichTextFonts,
    TextLineLayout,
};
use super::rich_document::{RichDocument, StyleProperty, TextStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormattingState {
    Off,
    On,
    Mixed,
}

#[derive(Clone, Debug)]
pub struct DocumentEditor {
    document: RichDocument,
    caret_byte: usize,
    selection_anchor_byte: Option<usize>,
    preferred_caret_x: Option<u32>,
    scroll_y: u32,
    viewport_h: u32,
    wrap_width: u32,
    line_height: u32,
    lines: Vec<TextLineLayout>,
    layout_dirty: bool,
    typing_style: TextStyle,
}

impl Default for DocumentEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentEditor {
    pub fn new() -> Self {
        Self::from_text("")
    }

    pub fn from_text(text: &str) -> Self {
        Self {
            document: RichDocument::from_text(text),
            caret_byte: 0,
            selection_anchor_byte: None,
            preferred_caret_x: None,
            scroll_y: 0,
            viewport_h: 0,
            wrap_width: 1,
            line_height: 1,
            lines: Vec::new(),
            layout_dirty: true,
            typing_style: TextStyle::default(),
        }
    }

    /// Create an editor around an already imported rich document.
    ///
    /// Persistence belongs to the owning application; this constructor keeps
    /// the canvas/editor layer format-neutral while allowing Writer to replace
    /// its document only after an import has succeeded.
    pub fn from_document(document: RichDocument) -> Self {
        Self {
            document,
            caret_byte: 0,
            selection_anchor_byte: None,
            preferred_caret_x: None,
            scroll_y: 0,
            viewport_h: 0,
            wrap_width: 1,
            line_height: 1,
            lines: Vec::new(),
            layout_dirty: true,
            typing_style: TextStyle::default(),
        }
    }

    /// Replace the edited document and reset transient navigation state.
    pub fn set_document(&mut self, document: RichDocument) {
        *self = Self::from_document(document);
    }

    pub fn text(&self) -> &str {
        self.document.text()
    }

    pub fn document(&self) -> &RichDocument {
        &self.document
    }

    pub fn typing_style(&self) -> TextStyle {
        self.typing_style
    }

    pub fn caret_byte(&self) -> usize {
        self.caret_byte
    }

    pub fn selection_anchor_byte(&self) -> Option<usize> {
        self.selection_anchor_byte
    }

    pub fn preferred_caret_x(&self) -> Option<u32> {
        self.preferred_caret_x
    }

    pub fn scroll_y(&self) -> u32 {
        self.scroll_y
    }

    pub fn lines(&self) -> &[TextLineLayout] {
        &self.lines
    }

    pub fn line_height(&self) -> u32 {
        self.line_height.max(1)
    }

    pub fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.selection_anchor_byte?;
        if anchor == self.caret_byte {
            None
        } else {
            Some(if anchor < self.caret_byte {
                (anchor, self.caret_byte)
            } else {
                (self.caret_byte, anchor)
            })
        }
    }

    pub fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selection_range()?;
        self.text().get(start..end)
    }

    pub fn set_caret(&mut self, byte: usize, extend_selection: bool) -> bool {
        let byte = self.clamp_boundary(byte);
        let old = (self.caret_byte, self.selection_anchor_byte);
        if extend_selection {
            self.selection_anchor_byte.get_or_insert(self.caret_byte);
        } else {
            self.selection_anchor_byte = None;
        }
        self.caret_byte = byte;
        self.preferred_caret_x = None;
        self.sync_typing_style();
        self.ensure_caret_visible();
        old != (self.caret_byte, self.selection_anchor_byte)
    }

    pub fn configure_layout(
        &mut self,
        font: Option<&dyn VecText>,
        wrap_width: u32,
        viewport_h: u32,
        line_height: u32,
    ) -> bool {
        let wrap_width = wrap_width.max(1);
        let line_height = line_height.max(1);
        let geometry_changed = self.wrap_width != wrap_width
            || self.viewport_h != viewport_h
            || self.line_height != line_height;
        self.wrap_width = wrap_width;
        self.viewport_h = viewport_h;
        self.line_height = line_height;
        if geometry_changed || self.layout_dirty {
            self.lines = layout_text_lines(font, self.text(), wrap_width, line_height);
            self.layout_dirty = false;
            self.clamp_invariants();
            self.ensure_caret_visible();
            true
        } else {
            self.clamp_scroll();
            false
        }
    }

    pub fn configure_rich_layout(
        &mut self,
        fonts: RichTextFonts<'_>,
        wrap_width: u32,
        viewport_h: u32,
        line_height: u32,
    ) -> bool {
        let wrap_width = wrap_width.max(1);
        let line_height = line_height.max(1);
        let geometry_changed = self.wrap_width != wrap_width
            || self.viewport_h != viewport_h
            || self.line_height != line_height;
        self.wrap_width = wrap_width;
        self.viewport_h = viewport_h;
        self.line_height = line_height;
        if geometry_changed || self.layout_dirty {
            self.lines = layout_rich_text_lines(fonts, &self.document, wrap_width, line_height);
            self.layout_dirty = false;
            self.clamp_invariants();
            self.ensure_caret_visible();
            true
        } else {
            self.clamp_scroll();
            false
        }
    }

    pub fn content_height(&self) -> u32 {
        (self.lines.len() as u32).saturating_mul(self.line_height())
    }

    pub fn max_scroll_y(&self) -> u32 {
        self.content_height().saturating_sub(self.viewport_h)
    }

    pub fn scroll_by(&mut self, delta_y: i32) -> bool {
        let old = self.scroll_y;
        self.scroll_y = if delta_y < 0 {
            self.scroll_y.saturating_sub(delta_y.unsigned_abs())
        } else {
            self.scroll_y
                .saturating_add(delta_y as u32)
                .min(self.max_scroll_y())
        };
        old != self.scroll_y
    }

    pub fn hit_test(&self, font: Option<&dyn VecText>, x: i32, y: i32) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let document_y = y.max(0).saturating_add(self.scroll_y as i32);
        let line_index = (document_y / self.line_height() as i32).max(0) as usize;
        let line = &self.lines[line_index.min(self.lines.len() - 1)];
        byte_at_x_on_line(font, self.text(), line, x)
    }

    pub fn rich_hit_test(&self, fonts: RichTextFonts<'_>, x: i32, y: i32) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let document_y = y.max(0).saturating_add(self.scroll_y as i32);
        let line_index = (document_y / self.line_height() as i32).max(0) as usize;
        let line = &self.lines[line_index.min(self.lines.len() - 1)];
        rich_byte_at_x_on_line(fonts, &self.document, line, x)
    }

    pub fn pointer_select(
        &mut self,
        font: Option<&dyn VecText>,
        x: i32,
        y: i32,
        anchor: Option<usize>,
    ) -> bool {
        let byte = self.hit_test(font, x, y);
        let old = (self.caret_byte, self.selection_anchor_byte);
        self.caret_byte = byte;
        self.selection_anchor_byte = anchor.map(|value| self.clamp_boundary(value));
        self.preferred_caret_x = None;
        self.sync_typing_style();
        self.ensure_caret_visible();
        old != (self.caret_byte, self.selection_anchor_byte)
    }

    pub fn rich_pointer_select(
        &mut self,
        fonts: RichTextFonts<'_>,
        x: i32,
        y: i32,
        anchor: Option<usize>,
    ) -> bool {
        let byte = self.rich_hit_test(fonts, x, y);
        let old = (self.caret_byte, self.selection_anchor_byte);
        self.caret_byte = byte;
        self.selection_anchor_byte = anchor.map(|value| self.clamp_boundary(value));
        self.preferred_caret_x = None;
        self.sync_typing_style();
        self.ensure_caret_visible();
        old != (self.caret_byte, self.selection_anchor_byte)
    }

    pub fn select_all(&mut self) -> bool {
        let old = (self.caret_byte, self.selection_anchor_byte);
        self.selection_anchor_byte = Some(0);
        self.caret_byte = self.text().len();
        self.preferred_caret_x = None;
        self.ensure_caret_visible();
        old != (self.caret_byte, self.selection_anchor_byte)
    }

    pub fn insert_str(&mut self, value: &str) -> bool {
        if value.is_empty() && self.selection_range().is_none() {
            return false;
        }
        let (start, end) = self
            .selection_range()
            .unwrap_or((self.caret_byte, self.caret_byte));
        self.document.replace(start..end, value, self.typing_style);
        self.caret_byte = start + value.len();
        self.after_mutation();
        true
    }

    pub fn insert_char(&mut self, value: char) -> bool {
        let mut bytes = [0u8; 4];
        self.insert_str(value.encode_utf8(&mut bytes))
    }

    pub fn insert_newline(&mut self) -> bool {
        self.insert_str("\n")
    }

    pub fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection_range() else {
            return false;
        };
        self.document.replace(start..end, "", self.typing_style);
        self.caret_byte = start;
        self.after_mutation();
        self.sync_typing_style();
        true
    }

    pub fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let Some(start) = previous_boundary(self.text(), self.caret_byte) else {
            return false;
        };
        self.document
            .replace(start..self.caret_byte, "", self.typing_style);
        self.caret_byte = start;
        self.after_mutation();
        self.sync_typing_style();
        true
    }

    pub fn delete_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let Some(end) = next_boundary(self.text(), self.caret_byte) else {
            return false;
        };
        self.document
            .replace(self.caret_byte..end, "", self.typing_style);
        self.after_mutation();
        self.sync_typing_style();
        true
    }

    pub fn move_left(&mut self, extend: bool) -> bool {
        if !extend {
            if let Some((start, _)) = self.selection_range() {
                return self.set_caret(start, false);
            }
        }
        let target = previous_boundary(self.text(), self.caret_byte).unwrap_or(0);
        self.move_to(target, extend, false)
    }

    pub fn move_right(&mut self, extend: bool) -> bool {
        if !extend {
            if let Some((_, end)) = self.selection_range() {
                return self.set_caret(end, false);
            }
        }
        let target = next_boundary(self.text(), self.caret_byte).unwrap_or(self.text().len());
        self.move_to(target, extend, false)
    }

    pub fn move_home(&mut self, extend: bool) -> bool {
        let line = self.caret_line_index();
        self.move_to(line_home_byte(&self.lines, line), extend, false)
    }

    pub fn move_end(&mut self, extend: bool) -> bool {
        let line = self.caret_line_index();
        self.move_to(line_end_byte(&self.lines, line), extend, false)
    }

    pub fn move_up(&mut self, font: Option<&dyn VecText>, extend: bool) -> bool {
        self.move_vertical(font, -1, extend)
    }

    pub fn move_down(&mut self, font: Option<&dyn VecText>, extend: bool) -> bool {
        self.move_vertical(font, 1, extend)
    }

    pub fn page_up(&mut self, font: Option<&dyn VecText>, extend: bool) -> bool {
        let count = (self.viewport_h / self.line_height()).max(1) as i32;
        self.move_vertical(font, -count, extend)
    }

    pub fn page_down(&mut self, font: Option<&dyn VecText>, extend: bool) -> bool {
        let count = (self.viewport_h / self.line_height()).max(1) as i32;
        self.move_vertical(font, count, extend)
    }

    pub fn move_up_rich(&mut self, fonts: RichTextFonts<'_>, extend: bool) -> bool {
        self.move_vertical_rich(fonts, -1, extend)
    }

    pub fn move_down_rich(&mut self, fonts: RichTextFonts<'_>, extend: bool) -> bool {
        self.move_vertical_rich(fonts, 1, extend)
    }

    pub fn page_up_rich(&mut self, fonts: RichTextFonts<'_>, extend: bool) -> bool {
        let count = (self.viewport_h / self.line_height()).max(1) as i32;
        self.move_vertical_rich(fonts, -count, extend)
    }

    pub fn page_down_rich(&mut self, fonts: RichTextFonts<'_>, extend: bool) -> bool {
        let count = (self.viewport_h / self.line_height()).max(1) as i32;
        self.move_vertical_rich(fonts, count, extend)
    }

    pub fn ensure_caret_visible(&mut self) {
        let line = self.caret_line_index() as u32;
        let top = line.saturating_mul(self.line_height());
        let bottom = top.saturating_add(self.line_height());
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if self.viewport_h > 0 && bottom > self.scroll_y.saturating_add(self.viewport_h) {
            self.scroll_y = bottom.saturating_sub(self.viewport_h);
        }
        self.clamp_scroll();
    }

    fn move_vertical(
        &mut self,
        font: Option<&dyn VecText>,
        delta_lines: i32,
        extend: bool,
    ) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        let current = self.caret_line_index();
        let desired_x = self.preferred_caret_x.unwrap_or_else(|| {
            caret_x_on_line(font, self.text(), &self.lines[current], self.caret_byte)
        });
        let last = self.lines.len().saturating_sub(1) as i32;
        let target_line = (current as i32).saturating_add(delta_lines).clamp(0, last) as usize;
        let target = byte_at_x_on_line(
            font,
            self.text(),
            &self.lines[target_line],
            desired_x as i32,
        );
        let changed = self.move_to(target, extend, true);
        self.preferred_caret_x = Some(desired_x);
        changed
    }

    fn move_vertical_rich(
        &mut self,
        fonts: RichTextFonts<'_>,
        delta_lines: i32,
        extend: bool,
    ) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        let current = self.caret_line_index();
        let desired_x = self.preferred_caret_x.unwrap_or_else(|| {
            rich_caret_x_on_line(fonts, &self.document, &self.lines[current], self.caret_byte)
        });
        let last = self.lines.len().saturating_sub(1) as i32;
        let target_line = (current as i32).saturating_add(delta_lines).clamp(0, last) as usize;
        let target = rich_byte_at_x_on_line(
            fonts,
            &self.document,
            &self.lines[target_line],
            desired_x as i32,
        );
        let changed = self.move_to(target, extend, true);
        self.preferred_caret_x = Some(desired_x);
        changed
    }

    fn move_to(&mut self, target: usize, extend: bool, vertical: bool) -> bool {
        let old = (self.caret_byte, self.selection_anchor_byte);
        if extend {
            self.selection_anchor_byte.get_or_insert(self.caret_byte);
        } else {
            self.selection_anchor_byte = None;
        }
        self.caret_byte = self.clamp_boundary(target);
        self.sync_typing_style();
        if !vertical {
            self.preferred_caret_x = None;
        }
        self.ensure_caret_visible();
        old != (self.caret_byte, self.selection_anchor_byte)
    }

    fn caret_line_index(&self) -> usize {
        find_line_index(&self.lines, self.caret_byte).unwrap_or(0)
    }

    fn after_mutation(&mut self) {
        self.selection_anchor_byte = None;
        self.preferred_caret_x = None;
        self.layout_dirty = true;
        self.clamp_invariants();
    }

    pub fn formatting_state(&self, property: StyleProperty) -> FormattingState {
        if let Some((start, end)) = self.selection_range() {
            return match self.document.range_uniform(start..end, property) {
                Some(true) => FormattingState::On,
                Some(false) => FormattingState::Off,
                None => FormattingState::Mixed,
            };
        }
        if self.typing_style.has(property) {
            FormattingState::On
        } else {
            FormattingState::Off
        }
    }

    /// Toggle a property predictably: a mixed selection becomes fully styled,
    /// while a uniformly styled selection has that property removed.
    pub fn toggle_format(&mut self, property: StyleProperty) -> bool {
        if let Some((start, end)) = self.selection_range() {
            let enabled = self.document.range_uniform(start..end, property) != Some(true);
            let changed = self.document.format(start..end, property, enabled);
            self.typing_style = self.typing_style.with(property, enabled);
            if changed {
                self.layout_dirty = true;
            }
            return changed;
        }
        self.typing_style = self
            .typing_style
            .with(property, !self.typing_style.has(property));
        true
    }

    fn sync_typing_style(&mut self) {
        if self.selection_range().is_none() {
            self.typing_style = self.document.insertion_style(self.caret_byte);
        }
    }

    fn clamp_boundary(&self, requested: usize) -> usize {
        let mut value = requested.min(self.text().len());
        while value > 0 && !self.text().is_char_boundary(value) {
            value -= 1;
        }
        value
    }

    fn clamp_invariants(&mut self) {
        self.caret_byte = self.clamp_boundary(self.caret_byte);
        self.selection_anchor_byte = self
            .selection_anchor_byte
            .map(|anchor| self.clamp_boundary(anchor));
        self.clamp_scroll();
        debug_assert!(self.text().is_char_boundary(self.caret_byte));
        debug_assert!(self
            .selection_anchor_byte
            .is_none_or(|anchor| self.text().is_char_boundary(anchor)));
        debug_assert!(!self.lines.is_empty() || self.layout_dirty);
    }

    fn clamp_scroll(&mut self) {
        self.scroll_y = self.scroll_y.min(self.max_scroll_y());
    }
}

fn previous_boundary(text: &str, byte: usize) -> Option<usize> {
    let byte = byte.min(text.len());
    text[..byte]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
}

fn next_boundary(text: &str, byte: usize) -> Option<usize> {
    let byte = byte.min(text.len());
    text[byte..]
        .chars()
        .next()
        .map(|value| byte + value.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::{DocumentEditor, FormattingState};
    use crate::widgets::rich_document::{StyleProperty, TextStyle};

    fn editor(text: &str, width: u32, height: u32) -> DocumentEditor {
        let mut editor = DocumentEditor::from_text(text);
        editor.configure_layout(None, width, height, 7);
        editor
    }

    #[test]
    fn insertion_newlines_and_cross_line_deletion_are_utf8_safe() {
        let mut value = editor("", 600, 70);
        assert!(value.insert_str("ab🐇cd"));
        value.configure_layout(None, 600, 70, 7);
        value.set_caret(2, false);
        assert!(value.insert_newline());
        assert_eq!(value.text(), "ab\n🐇cd");
        assert!(value.backspace());
        assert_eq!(value.text(), "ab🐇cd");
        value.set_caret(2, false);
        assert!(value.delete_forward());
        assert_eq!(value.text(), "abcd");
        assert!(value.text().is_char_boundary(value.caret_byte()));
    }

    #[test]
    fn selection_delete_replace_and_multiline_paste() {
        let mut value = editor("one\ntwo\nthree", 600, 70);
        value.set_caret(4, false);
        value.set_caret(8, true);
        assert_eq!(value.selected_text(), Some("two\n"));
        assert!(value.insert_str("A\nB\n"));
        assert_eq!(value.text(), "one\nA\nB\nthree");
        value.select_all();
        assert!(value.delete_selection());
        assert_eq!(value.text(), "");
        assert_eq!(value.caret_byte(), 0);
    }

    #[test]
    fn navigation_crosses_lines_and_preserves_preferred_x() {
        let mut value = editor("abcdef\nx\nabcdef", 600, 70);
        value.set_caret(5, false);
        assert!(value.move_down(None, false));
        assert_eq!(value.caret_byte(), 8);
        assert!(value.move_down(None, false));
        assert_eq!(value.caret_byte(), 14);
        assert!(value.move_left(false));
        assert!(value.move_right(false));
        assert!(value.move_home(false));
        assert_eq!(value.caret_byte(), 9);
        assert!(value.move_end(false));
        assert_eq!(value.caret_byte(), value.text().len());
    }

    #[test]
    fn wrapping_resize_and_hit_testing_use_visual_lines() {
        let mut value = editor("hello world abcdef", 36, 14);
        let narrow = value.lines().len();
        assert!(narrow >= 3);
        let wrapped_byte = value.hit_test(None, 0, 8);
        assert!(wrapped_byte > 0);
        assert_eq!(value.hit_test(None, 999, 999), value.text().len());
        value.configure_layout(None, 300, 14, 7);
        assert!(value.lines().len() < narrow);
    }

    #[test]
    fn blank_lines_trailing_newline_and_empty_document_have_caret_rows() {
        for text in ["", "\n", "a\n", "a\n\n"] {
            let value = editor(text, 600, 70);
            assert!(!value.lines().is_empty());
            assert_eq!(
                value.lines().len(),
                text.bytes().filter(|b| *b == b'\n').count() + 1
            );
        }
    }

    #[test]
    fn scrolling_follows_caret_and_clamps_after_delete_and_resize() {
        let mut value = editor("0\n1\n2\n3\n4\n5", 600, 14);
        value.set_caret(value.text().len(), false);
        assert!(value.scroll_y() > 0);
        value.select_all();
        value.delete_selection();
        value.configure_layout(None, 600, 14, 7);
        assert_eq!(value.scroll_y(), 0);
        value.insert_str("0\n1\n2\n3");
        value.configure_layout(None, 600, 14, 7);
        value.set_caret(value.text().len(), false);
        value.configure_layout(None, 600, 100, 7);
        assert_eq!(value.scroll_y(), 0);
    }

    #[test]
    fn shift_selection_spans_wrapped_and_logical_lines() {
        let mut value = editor("abcdef\ngh", 18, 70);
        assert!(value.move_right(true));
        assert!(value.move_down(None, true));
        assert!(value.selection_range().is_some());
        value.select_all();
        assert_eq!(value.selected_text(), Some("abcdef\ngh"));
    }

    #[test]
    fn enter_at_start_middle_and_end_preserves_trailing_lines() {
        let mut start = editor("abc", 600, 70);
        start.insert_newline();
        assert_eq!(start.text(), "\nabc");
        assert_eq!(start.caret_byte(), 1);

        let mut middle = editor("abc", 600, 70);
        middle.set_caret(1, false);
        middle.insert_newline();
        assert_eq!(middle.text(), "a\nbc");

        let mut end = editor("abc", 600, 70);
        end.set_caret(3, false);
        end.insert_newline();
        end.configure_layout(None, 600, 70, 7);
        assert_eq!(end.text(), "abc\n");
        assert_eq!(end.lines().len(), 2);
    }

    #[test]
    fn delete_forward_at_line_end_merges_logical_lines() {
        let mut value = editor("ab\ncd", 600, 70);
        value.set_caret(2, false);
        assert!(value.delete_forward());
        assert_eq!(value.text(), "abcd");
        assert_eq!(value.caret_byte(), 2);
    }

    #[test]
    fn horizontal_navigation_crosses_newlines_and_empty_document_is_stable() {
        let mut value = editor("a\nb", 600, 70);
        value.set_caret(2, false);
        assert!(value.move_left(false));
        assert_eq!(value.caret_byte(), 1);
        assert!(value.move_right(false));
        assert_eq!(value.caret_byte(), 2);

        let mut empty = editor("", 600, 70);
        assert!(!empty.move_left(false));
        assert!(!empty.move_right(false));
        assert_eq!(empty.hit_test(None, 100, 100), 0);
    }

    #[test]
    fn page_navigation_uses_viewport_geometry_and_keeps_caret_visible() {
        let mut value = editor("0\n1\n2\n3\n4\n5", 600, 14);
        assert!(value.page_down(None, false));
        assert_eq!(value.caret_byte(), 4);
        assert!(value.scroll_y() > 0);
        assert!(value.page_up(None, false));
        assert_eq!(value.caret_byte(), 0);
        assert_eq!(value.scroll_y(), 0);
    }

    #[test]
    fn ordinary_navigation_collapses_selection_to_nearest_edge() {
        let mut value = editor("abcdef", 600, 70);
        value.set_caret(1, false);
        value.set_caret(5, true);
        assert!(value.move_left(false));
        assert_eq!(value.caret_byte(), 1);
        assert!(value.selection_range().is_none());
        value.set_caret(5, true);
        assert!(value.move_right(false));
        assert_eq!(value.caret_byte(), 5);
        assert!(value.selection_range().is_none());
    }

    #[test]
    fn active_typing_style_creates_and_combines_normalized_runs() {
        let mut value = editor("", 600, 70);
        value.toggle_format(StyleProperty::Bold);
        value.insert_str("bold");
        assert_eq!(
            value.document().runs()[0].style,
            TextStyle {
                bold: true,
                italic: false,
                underline: false
            }
        );
        value.toggle_format(StyleProperty::Italic);
        value.toggle_format(StyleProperty::Underline);
        value.insert_str(" all");
        assert_eq!(
            value.document().runs()[1].style,
            TextStyle {
                bold: true,
                italic: true,
                underline: true
            }
        );
        value.toggle_format(StyleProperty::Bold);
        value.insert_str(" iu");
        assert_eq!(
            value.document().runs()[2].style,
            TextStyle {
                bold: false,
                italic: true,
                underline: true
            }
        );
    }

    #[test]
    fn selection_toggle_uses_uniform_and_mixed_semantics() {
        let mut value = editor("hello world", 600, 70);
        value.set_caret(0, false);
        value.set_caret(5, true);
        value.toggle_format(StyleProperty::Bold);
        assert_eq!(
            value.formatting_state(StyleProperty::Bold),
            FormattingState::On
        );
        value.select_all();
        assert_eq!(
            value.formatting_state(StyleProperty::Bold),
            FormattingState::Mixed
        );
        value.toggle_format(StyleProperty::Bold);
        assert_eq!(
            value.formatting_state(StyleProperty::Bold),
            FormattingState::On
        );
        value.toggle_format(StyleProperty::Bold);
        assert_eq!(
            value.formatting_state(StyleProperty::Bold),
            FormattingState::Off
        );
    }

    #[test]
    fn insert_delete_replace_and_paragraph_merge_preserve_styles() {
        let mut value = editor("ab\ncd", 600, 70);
        value.set_caret(0, false);
        value.set_caret(2, true);
        value.toggle_format(StyleProperty::Bold);
        value.set_caret(1, false);
        value.insert_str("🐇");
        assert!(value.document().style_at(1).bold);
        value.set_caret("a🐇b".len(), false);
        value.delete_forward();
        assert_eq!(value.text(), "a🐇bcd");
        assert!(value.document().style_at(0).bold);
        assert!(!value.document().style_at(value.text().len() - 1).bold);
        value.set_caret(1, false);
        value.set_caret("a🐇".len(), true);
        value.insert_str("Z");
        assert_eq!(value.text(), "aZbcd");
        assert!(value.document().style_at(1).bold);
    }

    #[test]
    fn enter_and_boundary_affinity_inherit_left_style() {
        let mut value = editor("Hello world", 600, 70);
        value.set_caret(0, false);
        value.set_caret(5, true);
        value.toggle_format(StyleProperty::Italic);
        value.set_caret(5, false);
        assert!(value.typing_style().italic);
        value.insert_newline();
        value.insert_str("new");
        assert_eq!(value.text(), "Hello\nnew world");
        assert!(value.document().style_at(6).italic);
    }

    #[test]
    fn unicode_selection_formats_only_valid_byte_range() {
        let mut value = editor("a🐇بz", 600, 70);
        value.set_caret(1, false);
        value.set_caret("a🐇ب".len(), true);
        value.toggle_format(StyleProperty::Underline);
        assert!(!value.document().style_at(0).underline);
        assert!(value.document().style_at(1).underline);
        assert!(!value.document().style_at(value.text().len() - 1).underline);
        assert!(value
            .document()
            .runs()
            .iter()
            .all(|run| value.text().is_char_boundary(run.range.start)
                && value.text().is_char_boundary(run.range.end)));
    }
}
