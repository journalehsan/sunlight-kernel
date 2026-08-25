//! Format-neutral rich-text storage for document editors.
//!
//! Text remains one UTF-8 buffer so logical positions stay compatible with
//! existing editors.  Character formatting is represented by a compact,
//! normalized partition of that buffer.  Newlines are ordinary styled bytes
//! and are also the explicit separators between logical paragraphs.

use alloc::{string::String, vec::Vec};
use core::ops::Range;

pub type FontSize = u16;
pub const DEFAULT_FONT_SIZE: FontSize = 13;
pub const MIN_FONT_SIZE: FontSize = 8;
pub const MAX_FONT_SIZE: FontSize = 48;

pub const fn clamp_font_size(size: FontSize) -> FontSize {
    if size < MIN_FONT_SIZE {
        MIN_FONT_SIZE
    } else if size > MAX_FONT_SIZE {
        MAX_FONT_SIZE
    } else {
        size
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Font size in points. Values are always bounded by `clamp_font_size`.
    pub font_size: FontSize,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            bold: false,
            italic: false,
            underline: false,
            font_size: DEFAULT_FONT_SIZE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleProperty {
    Bold,
    Italic,
    Underline,
}

impl TextStyle {
    pub const fn with_font_size(self, size: FontSize) -> Self {
        Self {
            font_size: clamp_font_size(size),
            ..self
        }
    }

    pub const fn with(self, property: StyleProperty, enabled: bool) -> Self {
        match property {
            StyleProperty::Bold => Self {
                bold: enabled,
                ..self
            },
            StyleProperty::Italic => Self {
                italic: enabled,
                ..self
            },
            StyleProperty::Underline => Self {
                underline: enabled,
                ..self
            },
        }
    }

    pub const fn has(self, property: StyleProperty) -> bool {
        match property {
            StyleProperty::Bold => self.bold,
            StyleProperty::Italic => self.italic,
            StyleProperty::Underline => self.underline,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParagraphAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParagraphKind {
    #[default]
    Normal,
    Heading1,
    Heading2,
    Heading3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParagraphStyle {
    pub alignment: ParagraphAlignment,
    pub kind: ParagraphKind,
}

impl ParagraphStyle {
    pub const fn resolved(self, inline: TextStyle) -> TextStyle {
        let (heading_size, heading_bold) = match self.kind {
            ParagraphKind::Normal => (DEFAULT_FONT_SIZE, false),
            ParagraphKind::Heading1 => (24, true),
            ParagraphKind::Heading2 => (20, true),
            ParagraphKind::Heading3 => (16, true),
        };
        TextStyle {
            bold: inline.bold || heading_bold,
            font_size: if inline.font_size == DEFAULT_FONT_SIZE {
                heading_size
            } else {
                inline.font_size
            },
            ..inline
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleRun {
    pub range: Range<usize>,
    pub style: TextStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RichDocument {
    text: String,
    runs: Vec<StyleRun>,
    paragraph_styles: Vec<ParagraphStyle>,
}

impl Default for RichDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl RichDocument {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            runs: Vec::new(),
            paragraph_styles: Vec::from([ParagraphStyle::default()]),
        }
    }

    pub fn from_text(text: &str) -> Self {
        Self::from_styled_text(text, TextStyle::default())
    }

    pub fn from_styled_text(text: &str, style: TextStyle) -> Self {
        let style = style.with_font_size(style.font_size);
        let runs = if text.is_empty() {
            Vec::new()
        } else {
            Vec::from([StyleRun {
                range: 0..text.len(),
                style,
            }])
        };
        Self {
            text: String::from(text),
            runs,
            paragraph_styles: core::iter::repeat(ParagraphStyle::default())
                .take(text.bytes().filter(|b| *b == b'\n').count() + 1)
                .collect(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn len(&self) -> usize {
        self.text.len()
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn runs(&self) -> &[StyleRun] {
        &self.runs
    }

    pub fn paragraph_styles(&self) -> &[ParagraphStyle] {
        &self.paragraph_styles
    }

    /// Number of explicit logical paragraphs.  Even an empty document has one.
    pub fn paragraph_count(&self) -> usize {
        self.text.bytes().filter(|byte| *byte == b'\n').count() + 1
    }

    /// Byte range of a logical paragraph, excluding its newline separator.
    pub fn paragraph_range(&self, index: usize) -> Option<Range<usize>> {
        let mut start = 0;
        let mut current = 0;
        for (offset, byte) in self.text.bytes().enumerate() {
            if byte == b'\n' {
                if current == index {
                    return Some(start..offset);
                }
                current += 1;
                start = offset + 1;
            }
        }
        (current == index).then_some(start..self.text.len())
    }

    pub fn paragraph_index_at(&self, byte: usize) -> usize {
        self.text
            .as_bytes()
            .get(..byte.min(self.text.len()))
            .unwrap_or(self.text.as_bytes())
            .iter()
            .filter(|b| **b == b'\n')
            .count()
    }

    pub fn paragraph_style(&self, index: usize) -> ParagraphStyle {
        self.paragraph_styles
            .get(index)
            .copied()
            .unwrap_or_default()
    }

    pub fn paragraph_style_at(&self, byte: usize) -> ParagraphStyle {
        self.paragraph_style(self.paragraph_index_at(byte))
    }

    pub fn resolved_style_at(&self, byte: usize) -> TextStyle {
        self.paragraph_style_at(byte).resolved(self.style_at(byte))
    }

    pub fn set_paragraph_style(&mut self, index: usize, style: ParagraphStyle) -> bool {
        let Some(previous) = self.paragraph_styles.get_mut(index) else {
            return false;
        };
        if *previous == style {
            return false;
        }
        *previous = style;
        true
    }

    pub fn set_paragraph_styles(&mut self, range: Range<usize>, style: ParagraphStyle) -> bool {
        let Some(range) = self.valid_range(range) else {
            return false;
        };
        let first = self.paragraph_index_at(range.start);
        let last = self.paragraph_index_at(range.end.saturating_sub(1).max(range.start));
        let mut changed = false;
        for index in first..=last.min(self.paragraph_styles.len().saturating_sub(1)) {
            changed |= self.set_paragraph_style(index, style);
        }
        changed
    }

    pub fn style_at(&self, byte: usize) -> TextStyle {
        if self.text.is_empty() {
            return TextStyle::default();
        }
        let byte = byte.min(self.text.len().saturating_sub(1));
        self.runs
            .iter()
            .find(|run| run.range.contains(&byte))
            .map_or(TextStyle::default(), |run| run.style)
    }

    /// Style for insertion at a caret. Boundaries prefer the character on the
    /// left; document start uses the right character. This affinity is stable
    /// across wrapping and paragraph reflow.
    pub fn insertion_style(&self, caret: usize) -> TextStyle {
        if self.text.is_empty() {
            return TextStyle::default();
        }
        if caret == 0 {
            self.style_at(0)
        } else {
            let left = self.text[..caret.min(self.text.len())]
                .char_indices()
                .next_back()
                .map_or(0, |(offset, _)| offset);
            self.style_at(left)
        }
    }

    pub fn range_uniform(&self, range: Range<usize>, property: StyleProperty) -> Option<bool> {
        let range = self.valid_range(range)?;
        if range.is_empty() {
            return None;
        }
        let mut value = None;
        for run in self.runs.iter().filter(|run| overlaps(&run.range, &range)) {
            let current = run.style.has(property);
            match value {
                None => value = Some(current),
                Some(previous) if previous != current => return None,
                _ => {}
            }
        }
        value
    }

    pub fn range_uniform_font_size(&self, range: Range<usize>) -> Option<FontSize> {
        let range = self.valid_range(range)?;
        if range.is_empty() {
            return None;
        }
        let mut value = None;
        for run in self.runs.iter().filter(|run| overlaps(&run.range, &range)) {
            match value {
                None => value = Some(run.style.font_size),
                Some(previous) if previous != run.style.font_size => return None,
                _ => {}
            }
        }
        value
    }

    pub fn set_font_size(&mut self, range: Range<usize>, size: FontSize) -> bool {
        let Some(range) = self.valid_range(range) else {
            return false;
        };
        if range.is_empty() {
            return false;
        }
        let size = clamp_font_size(size);
        let mut changed = false;
        let mut next = Vec::with_capacity(self.runs.len() + 2);
        for run in &self.runs {
            if !overlaps(&run.range, &range) {
                push_run(&mut next, run.range.clone(), run.style);
                continue;
            }
            if run.range.start < range.start {
                push_run(&mut next, run.range.start..range.start, run.style);
            }
            let middle = run.range.start.max(range.start)..run.range.end.min(range.end);
            let style = run.style.with_font_size(size);
            changed |= style != run.style;
            push_run(&mut next, middle, style);
            if run.range.end > range.end {
                push_run(&mut next, range.end..run.range.end, run.style);
            }
        }
        self.runs = next;
        self.assert_invariants();
        changed
    }

    pub fn format(&mut self, range: Range<usize>, property: StyleProperty, enabled: bool) -> bool {
        let Some(range) = self.valid_range(range) else {
            return false;
        };
        if range.is_empty() {
            return false;
        }
        let mut changed = false;
        let mut next = Vec::with_capacity(self.runs.len() + 2);
        for run in &self.runs {
            if !overlaps(&run.range, &range) {
                push_run(&mut next, run.range.clone(), run.style);
                continue;
            }
            if run.range.start < range.start {
                push_run(&mut next, run.range.start..range.start, run.style);
            }
            let middle = run.range.start.max(range.start)..run.range.end.min(range.end);
            let style = run.style.with(property, enabled);
            changed |= style != run.style;
            push_run(&mut next, middle, style);
            if run.range.end > range.end {
                push_run(&mut next, range.end..run.range.end, run.style);
            }
        }
        self.runs = next;
        self.assert_invariants();
        changed
    }

    pub fn replace(&mut self, range: Range<usize>, value: &str, style: TextStyle) -> bool {
        let Some(range) = self.valid_range(range) else {
            return false;
        };
        if range.is_empty() && value.is_empty() {
            return false;
        }
        let style = style.with_font_size(style.font_size);
        let removed = range.end - range.start;
        let inserted = value.len();
        let start_paragraph = self.paragraph_index_at(range.start);
        let end_paragraph = if range.end > range.start {
            let end_includes_newline =
                self.text.as_bytes()[range.start..range.end].contains(&b'\n');
            if end_includes_newline {
                self.paragraph_index_at(range.end)
            } else {
                self.paragraph_index_at(range.end - 1)
            }
        } else {
            start_paragraph
        };
        let inherited_paragraph_style = self.paragraph_style(start_paragraph);
        let old_paragraph_styles = self.paragraph_styles.clone();
        let mut next = Vec::with_capacity(self.runs.len() + 1);

        for run in &self.runs {
            let end = run.range.end.min(range.start);
            if end > run.range.start {
                push_run(&mut next, run.range.start..end, run.style);
            }
        }
        push_run(&mut next, range.start..range.start + inserted, style);
        for run in &self.runs {
            let start = run.range.start.max(range.end);
            if run.range.end > start {
                let shifted_start = start - removed + inserted;
                let shifted_end = run.range.end - removed + inserted;
                push_run(&mut next, shifted_start..shifted_end, run.style);
            }
        }
        self.text.replace_range(range, value);
        self.runs = next;
        let inserted_paragraphs = value.bytes().filter(|b| *b == b'\n').count();
        let mut paragraph_styles = Vec::with_capacity(
            old_paragraph_styles
                .len()
                .saturating_sub(end_paragraph.saturating_sub(start_paragraph))
                .saturating_add(inserted_paragraphs),
        );
        paragraph_styles.extend_from_slice(&old_paragraph_styles[..start_paragraph]);
        paragraph_styles
            .extend(core::iter::repeat(inherited_paragraph_style).take(inserted_paragraphs + 1));
        if end_paragraph + 1 < old_paragraph_styles.len() {
            paragraph_styles.extend_from_slice(&old_paragraph_styles[end_paragraph + 1..]);
        }
        self.paragraph_styles = paragraph_styles;
        self.assert_invariants();
        true
    }

    fn valid_range(&self, range: Range<usize>) -> Option<Range<usize>> {
        (range.start <= range.end
            && range.end <= self.text.len()
            && self.text.is_char_boundary(range.start)
            && self.text.is_char_boundary(range.end))
        .then_some(range)
    }

    fn assert_invariants(&self) {
        debug_assert_eq!(self.runs.first().map(|run| run.range.start).unwrap_or(0), 0);
        debug_assert_eq!(
            self.runs.last().map(|run| run.range.end).unwrap_or(0),
            self.text.len()
        );
        debug_assert_eq!(self.paragraph_styles.len(), self.paragraph_count());
        for (index, run) in self.runs.iter().enumerate() {
            debug_assert!(run.range.start < run.range.end);
            debug_assert!(self.text.is_char_boundary(run.range.start));
            debug_assert!(self.text.is_char_boundary(run.range.end));
            debug_assert!((MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&run.style.font_size));
            if let Some(previous) = index.checked_sub(1).and_then(|i| self.runs.get(i)) {
                debug_assert_eq!(previous.range.end, run.range.start);
                debug_assert_ne!(previous.style, run.style);
            }
        }
    }
}

fn overlaps(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn push_run(runs: &mut Vec<StyleRun>, range: Range<usize>, style: TextStyle) {
    if range.is_empty() {
        return;
    }
    if let Some(previous) = runs.last_mut() {
        if previous.range.end == range.start && previous.style == style {
            previous.range.end = range.end;
            return;
        }
    }
    runs.push(StyleRun { range, style });
}

#[cfg(test)]
mod tests {
    use super::{
        ParagraphAlignment, ParagraphKind, ParagraphStyle, RichDocument, StyleProperty, TextStyle,
        DEFAULT_FONT_SIZE,
    };

    fn style(bold: bool, italic: bool, underline: bool) -> TextStyle {
        TextStyle {
            bold,
            italic,
            underline,
            font_size: DEFAULT_FONT_SIZE,
        }
    }

    #[test]
    fn creates_empty_regular_and_combined_documents() {
        assert!(RichDocument::new().runs().is_empty());
        let regular = RichDocument::from_text("hello");
        assert_eq!(regular.runs().len(), 1);
        for combined in [
            style(true, false, false),
            style(false, true, false),
            style(false, false, true),
            style(true, true, true),
        ] {
            let value = RichDocument::from_styled_text("text", combined);
            assert_eq!(value.runs()[0].style, combined);
        }
    }

    #[test]
    fn formats_partial_full_and_multiple_runs_with_normalization() {
        let mut doc = RichDocument::from_text("Hello Sunlight World");
        assert!(doc.format(6..14, StyleProperty::Bold, true));
        assert_eq!(doc.runs().len(), 3);
        assert!(doc.format(0..doc.len(), StyleProperty::Bold, true));
        assert_eq!(doc.runs().len(), 1);
        assert!(doc.format(0..doc.len(), StyleProperty::Bold, false));
        assert_eq!(doc.runs().len(), 1);
        assert_eq!(doc.runs()[0].style, TextStyle::default());
    }

    #[test]
    fn formats_across_paragraphs_and_composes_styles() {
        let mut doc = RichDocument::from_text("one\ntwo\nthree");
        doc.format(2..9, StyleProperty::Italic, true);
        doc.format(4..7, StyleProperty::Underline, true);
        assert_eq!(doc.paragraph_count(), 3);
        assert_eq!(doc.paragraph_range(1), Some(4..7));
        assert_eq!(doc.style_at(5), style(false, true, true));
    }

    #[test]
    fn replacement_preserves_surrounding_styles_and_removes_empty_runs() {
        let mut doc = RichDocument::from_text("abcdef");
        doc.format(0..3, StyleProperty::Bold, true);
        doc.replace(2..4, "🐇", style(false, true, false));
        assert_eq!(doc.text(), "ab🐇ef");
        assert_eq!(doc.runs().len(), 3);
        doc.replace(2..6, "", TextStyle::default());
        assert_eq!(doc.text(), "abef");
        assert_eq!(doc.runs().len(), 2);
        assert!(doc.runs().iter().all(|run| !run.range.is_empty()));
    }

    #[test]
    fn uniform_query_reports_mixed_selection() {
        let mut doc = RichDocument::from_text("hello world");
        doc.format(0..5, StyleProperty::Bold, true);
        assert_eq!(doc.range_uniform(0..5, StyleProperty::Bold), Some(true));
        assert_eq!(doc.range_uniform(6..11, StyleProperty::Bold), Some(false));
        assert_eq!(doc.range_uniform(0..11, StyleProperty::Bold), None);
    }

    #[test]
    fn paragraph_styles_and_font_sizes_are_structural_and_normalized() {
        let mut doc = RichDocument::from_text("Title\nbody");
        assert_eq!(doc.paragraph_count(), 2);
        assert!(doc.set_paragraph_style(
            0,
            ParagraphStyle {
                alignment: ParagraphAlignment::Center,
                kind: ParagraphKind::Heading1,
            }
        ));
        assert_eq!(doc.paragraph_style_at(0).kind, ParagraphKind::Heading1);
        assert_eq!(doc.resolved_style_at(0).font_size, 24);
        assert!(doc.set_font_size(0..5, 18));
        assert_eq!(doc.runs()[0].style.font_size, 18);
        doc.replace(5..5, "\n", TextStyle::default());
        assert_eq!(doc.paragraph_count(), 3);
        assert_eq!(doc.paragraph_style(0).kind, ParagraphKind::Heading1);
        assert_eq!(doc.paragraph_style(1).kind, ParagraphKind::Heading1);
    }
}
