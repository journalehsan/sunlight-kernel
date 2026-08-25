//! Writer document persistence.
//!
//! The rich document remains authoritative.  This module only translates
//! bounded external formats to and from `RichDocument`; it has no Canvas or
//! UI dependencies and is therefore host-testable.

use alloc::{string::String, vec::Vec};
use core::fmt;

use sunlight_ui::widgets::{RichDocument, RichTextStyle, StyleProperty};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentFormat {
    Markdown,
    PlainText,
}

impl DocumentFormat {
    pub const fn default_extension(self) -> &'static str {
        match self {
            Self::Markdown => ".md",
            Self::PlainText => ".txt",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Markdown => "Markdown",
            Self::PlainText => "Plain Text",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistenceError {
    UnsupportedFormat,
    InvalidUtf8,
    FileTooLarge,
    Io(&'static str),
}

impl PersistenceError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnsupportedFormat => "Unsupported document format",
            Self::InvalidUtf8 => "The file is not valid UTF-8",
            Self::FileTooLarge => "The document is too large to open",
            Self::Io(message) => message,
        }
    }
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

pub fn format_from_path(path: &str) -> Option<DocumentFormat> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let extension = name.rsplit_once('.').map(|(_, ext)| ext)?;
    if extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown") {
        Some(DocumentFormat::Markdown)
    } else if extension.eq_ignore_ascii_case("txt") {
        Some(DocumentFormat::PlainText)
    } else {
        None
    }
}

pub fn normalize_text(bytes: &[u8]) -> Result<&str, PersistenceError> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    core::str::from_utf8(bytes).map_err(|_| PersistenceError::InvalidUtf8)
}

pub fn normalize_newlines(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                out.push('\n');
                index += 2;
            }
            b'\r' => {
                out.push('\n');
                index += 1;
            }
            _ => {
                let ch = text[index..].chars().next().unwrap_or('\0');
                out.push(ch);
                index += ch.len_utf8();
            }
        }
    }
    out
}

pub fn import_plain_text(bytes: &[u8]) -> Result<RichDocument, PersistenceError> {
    let text = normalize_newlines(normalize_text(bytes)?);
    Ok(RichDocument::from_text(&text))
}

#[derive(Clone)]
struct StyledChunk {
    text: String,
    style: RichTextStyle,
}

fn push_chunk(chunks: &mut Vec<StyledChunk>, text: &str, style: RichTextStyle) {
    if text.is_empty() {
        return;
    }
    if let Some(previous) = chunks.last_mut() {
        if previous.style == style {
            previous.text.push_str(text);
            return;
        }
    }
    chunks.push(StyledChunk {
        text: String::from(text),
        style,
    });
}

fn marker_at(bytes: &[u8], index: usize) -> Option<(&'static [u8], RichTextStyle)> {
    if bytes.get(index..index + 3) == Some(b"***") {
        Some((
            b"***",
            RichTextStyle {
                bold: true,
                italic: true,
                underline: false,
            },
        ))
    } else if bytes.get(index..index + 2) == Some(b"**") {
        Some((
            b"**",
            RichTextStyle {
                bold: true,
                italic: false,
                underline: false,
            },
        ))
    } else if bytes.get(index) == Some(&b'*') {
        Some((
            b"*",
            RichTextStyle {
                bold: false,
                italic: true,
                underline: false,
            },
        ))
    } else {
        None
    }
}

fn parse_inline(text: &str, inherited: RichTextStyle, chunks: &mut Vec<StyledChunk>) {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut plain_start = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            if let Some(next) = bytes.get(index + 1) {
                if matches!(*next, b'*' | b'_' | b'\\' | b'`') {
                    push_chunk(chunks, &text[plain_start..index], inherited);
                    push_chunk(
                        chunks,
                        core::str::from_utf8(core::slice::from_ref(next)).unwrap_or(""),
                        inherited,
                    );
                    index += 2;
                    plain_start = index;
                    continue;
                }
            }
        }
        let Some((marker, marker_style)) = marker_at(bytes, index) else {
            index += text[index..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let marker_len = marker.len();
        let search_start = index + marker_len;
        let Some(relative_end) = bytes[search_start..]
            .windows(marker_len)
            .position(|window| window == marker)
        else {
            index += marker_len;
            continue;
        };
        let end = search_start + relative_end;
        if end == search_start {
            index += marker_len;
            continue;
        }
        push_chunk(chunks, &text[plain_start..index], inherited);
        let nested = RichTextStyle {
            bold: inherited.bold || marker_style.bold,
            italic: inherited.italic || marker_style.italic,
            underline: inherited.underline,
        };
        parse_inline(&text[search_start..end], nested, chunks);
        index = end + marker_len;
        plain_start = index;
    }
    push_chunk(chunks, &text[plain_start..], inherited);
}

pub fn import_markdown(bytes: &[u8]) -> Result<RichDocument, PersistenceError> {
    let source = normalize_newlines(normalize_text(bytes)?);
    let mut chunks = Vec::new();
    parse_inline(&source, RichTextStyle::default(), &mut chunks);
    let mut text = String::new();
    let mut ranges = Vec::new();
    for chunk in chunks {
        let start = text.len();
        text.push_str(&chunk.text);
        ranges.push((start..text.len(), chunk.style));
    }
    let mut document = RichDocument::from_text(&text);
    for (range, style) in ranges {
        for (property, enabled) in [
            (StyleProperty::Bold, style.bold),
            (StyleProperty::Italic, style.italic),
            (StyleProperty::Underline, style.underline),
        ] {
            if enabled {
                let _ = document.format(range.clone(), property, true);
            }
        }
    }
    Ok(document)
}

pub fn import(format: DocumentFormat, bytes: &[u8]) -> Result<RichDocument, PersistenceError> {
    match format {
        DocumentFormat::Markdown => import_markdown(bytes),
        DocumentFormat::PlainText => import_plain_text(bytes),
    }
}

fn escape_markdown(text: &str, out: &mut String) {
    for ch in text.chars() {
        if matches!(ch, '*' | '_' | '\\' | '`') {
            out.push('\\');
        }
        out.push(ch);
    }
}

pub fn export_plain_text(document: &RichDocument) -> String {
    String::from(document.text())
}

pub fn export_markdown(document: &RichDocument) -> String {
    let mut out = String::new();
    for run in document.runs() {
        let marker = match (run.style.bold, run.style.italic) {
            (true, true) => "***",
            (true, false) => "**",
            (false, true) => "*",
            (false, false) => "",
        };
        let mut segment_start = run.range.start;
        while segment_start < run.range.end {
            let segment_end = document.text()[segment_start..run.range.end]
                .find('\n')
                .map(|offset| segment_start + offset)
                .unwrap_or(run.range.end);
            if !marker.is_empty() && segment_end > segment_start {
                out.push_str(marker);
            }
            escape_markdown(&document.text()[segment_start..segment_end], &mut out);
            if !marker.is_empty() && segment_end > segment_start {
                out.push_str(marker);
            }
            if segment_end < run.range.end {
                out.push('\n');
                segment_start = segment_end + 1;
            } else {
                segment_start = run.range.end;
            }
        }
    }
    out
}

pub fn export(format: DocumentFormat, document: &RichDocument) -> String {
    match format {
        DocumentFormat::Markdown => export_markdown(document),
        DocumentFormat::PlainText => export_plain_text(document),
    }
}

pub fn loses_formatting(format: DocumentFormat, document: &RichDocument) -> bool {
    document.runs().iter().any(|run| match format {
        DocumentFormat::PlainText => run.style.bold || run.style.italic || run.style.underline,
        DocumentFormat::Markdown => run.style.underline,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterDocumentSession {
    pub document: RichDocument,
    pub path: Option<String>,
    pub format: Option<DocumentFormat>,
    revision: u64,
    saved_revision: u64,
}

impl WriterDocumentSession {
    pub fn new() -> Self {
        Self {
            document: RichDocument::new(),
            path: None,
            format: None,
            revision: 0,
            saved_revision: 0,
        }
    }

    pub fn from_document(
        document: RichDocument,
        path: Option<String>,
        format: Option<DocumentFormat>,
    ) -> Self {
        Self {
            document,
            path,
            format,
            revision: 0,
            saved_revision: 0,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_changed(&mut self, document: &RichDocument) {
        self.document = document.clone();
        self.revision = self.revision.saturating_add(1);
    }

    pub fn mark_saved(&mut self, document: &RichDocument) {
        self.document = document.clone();
        self.saved_revision = self.revision;
    }

    pub fn replace_loaded(&mut self, document: RichDocument, path: String, format: DocumentFormat) {
        self.document = document;
        self.path = Some(path);
        self.format = Some(format);
        self.revision = self.revision.saturating_add(1);
        self.saved_revision = self.revision;
    }

    pub fn set_saved_target(&mut self, path: String, format: DocumentFormat) {
        self.path = Some(path);
        self.format = Some(format);
        self.saved_revision = self.revision;
    }
}

impl Default for WriterDocumentSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style_at(document: &RichDocument, needle: &str) -> RichTextStyle {
        let start = document.text().find(needle).unwrap();
        document.style_at(start)
    }

    #[test]
    fn detects_supported_extensions_case_insensitively() {
        assert_eq!(
            format_from_path("notes.MARKDOWN"),
            Some(DocumentFormat::Markdown)
        );
        assert_eq!(
            format_from_path("/tmp/a.TXT"),
            Some(DocumentFormat::PlainText)
        );
        assert_eq!(format_from_path("a.rtf"), None);
    }

    #[test]
    fn markdown_imports_styles_and_preserves_unsupported_text() {
        let document = import_markdown(b"one **bold** *italic* ***both***\n> quote").unwrap();
        assert_eq!(document.text(), "one bold italic both\n> quote");
        assert!(style_at(&document, "bold").bold);
        assert!(style_at(&document, "italic").italic);
        let both = style_at(&document, "both");
        assert!(both.bold && both.italic);
        assert!(document.text().contains("> quote"));
    }

    #[test]
    fn markdown_import_handles_escaped_and_incomplete_markers() {
        let document = import_markdown(br"\*literal\* **unfinished").unwrap();
        assert_eq!(document.text(), "*literal* **unfinished");
    }

    #[test]
    fn text_import_strips_bom_and_normalizes_newlines() {
        let document = import_plain_text(b"\xEF\xBB\xBFone\r\ntwo\rthree").unwrap();
        assert_eq!(document.text(), "one\ntwo\nthree");
        assert_eq!(document.runs().len(), 1);
    }

    #[test]
    fn markdown_export_escapes_literals_and_drops_underline_only() {
        let mut document = RichDocument::from_text("*_\\`");
        document.format(0..1, StyleProperty::Underline, true);
        assert_eq!(export_markdown(&document), r"\*\_\\\`");
        assert_eq!(export_plain_text(&document), "*_\\`");
    }

    #[test]
    fn markdown_export_handles_adjacent_style_runs() {
        let mut document = RichDocument::from_text("hellosunlight");
        document.format(5..8, StyleProperty::Bold, true);
        document.format(8..11, StyleProperty::Italic, true);
        let markdown = export_markdown(&document);
        assert_eq!(markdown, "hello**sun***lig*ht");
        let round_trip = import_markdown(markdown.as_bytes()).unwrap();
        assert_eq!(round_trip.text(), document.text());
        assert_eq!(style_at(&round_trip, "sun").bold, true);
        assert_eq!(style_at(&round_trip, "lig").italic, true);
    }

    #[test]
    fn session_revision_state_preserves_failed_save_semantics() {
        let mut session = WriterDocumentSession::new();
        assert!(!session.is_dirty());
        let document = RichDocument::from_text("draft");
        session.mark_changed(&document);
        assert!(session.is_dirty());
        session.set_saved_target(String::from("/tmp/notes.md"), DocumentFormat::Markdown);
        assert!(!session.is_dirty());
        session.mark_changed(&RichDocument::from_text("changed"));
        assert!(session.is_dirty());
        assert_eq!(session.path.as_deref(), Some("/tmp/notes.md"));
    }

    #[test]
    fn failed_import_does_not_require_mutating_existing_session() {
        let session = WriterDocumentSession::from_document(
            RichDocument::from_text("existing"),
            Some(String::from("/tmp/existing.txt")),
            Some(DocumentFormat::PlainText),
        );
        let before = session.clone();
        assert!(import(DocumentFormat::PlainText, b"\xFF").is_err());
        assert_eq!(session, before);
    }
}
