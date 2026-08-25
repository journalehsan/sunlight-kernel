//! Writer document persistence.
//!
//! The rich document remains authoritative.  This module only translates
//! bounded external formats to and from `RichDocument`; it has no Canvas or
//! UI dependencies and is therefore host-testable.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;

use sunlight_ui::widgets::{RichDocument, RichTextStyle, StyleProperty};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentFormat {
    Markdown,
    PlainText,
    Rtf,
}

impl DocumentFormat {
    pub const fn default_extension(self) -> &'static str {
        match self {
            Self::Markdown => ".md",
            Self::PlainText => ".txt",
            Self::Rtf => ".rtf",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Markdown => "Markdown",
            Self::PlainText => "Plain Text",
            Self::Rtf => "Rich Text Format",
        }
    }

    pub const fn capabilities(self) -> FormatCapabilities {
        match self {
            Self::Markdown => FormatCapabilities {
                bold: true,
                italic: true,
                underline: false,
            },
            Self::PlainText => FormatCapabilities {
                bold: false,
                italic: false,
                underline: false,
            },
            Self::Rtf => FormatCapabilities {
                bold: true,
                italic: true,
                underline: true,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatCapabilities {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistenceError {
    UnsupportedFormat,
    InvalidUtf8,
    FileTooLarge,
    Io(&'static str),
    InvalidRtf,
    RtfNestingLimit,
}

impl PersistenceError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnsupportedFormat => "Unsupported document format",
            Self::InvalidUtf8 => "The file is not valid UTF-8",
            Self::FileTooLarge => "The document is too large to open",
            Self::Io(message) => message,
            Self::InvalidRtf => "The RTF document is malformed",
            Self::RtfNestingLimit => "The RTF document is too deeply nested",
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
    } else if extension.eq_ignore_ascii_case("rtf") {
        Some(DocumentFormat::Rtf)
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
        DocumentFormat::Rtf => import_rtf(bytes),
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

const MAX_RTF_NESTING: usize = 64;

#[derive(Clone, Copy)]
struct RtfState {
    style: RichTextStyle,
    destination_skip: bool,
    ignorable: bool,
    uc_skip: usize,
}

impl Default for RtfState {
    fn default() -> Self {
        Self {
            style: RichTextStyle::default(),
            destination_skip: false,
            ignorable: false,
            uc_skip: 1,
        }
    }
}

fn rtf_push_chunk(chunks: &mut Vec<StyledChunk>, text: &str, style: RichTextStyle) {
    push_chunk(chunks, text, style);
}

fn rtf_push_char(chunks: &mut Vec<StyledChunk>, ch: char, style: RichTextStyle) {
    let mut buf = [0u8; 4];
    rtf_push_chunk(chunks, ch.encode_utf8(&mut buf), style);
}

fn parse_rtf_control(bytes: &[u8], mut index: usize) -> Option<(String, Option<i32>, usize)> {
    if bytes.get(index) != Some(&b'\\') {
        return None;
    }
    index += 1;
    let start = index;
    while bytes.get(index).is_some_and(|b| b.is_ascii_alphabetic()) {
        index += 1;
    }
    if index == start {
        return Some((String::new(), None, index));
    }
    let name = String::from(core::str::from_utf8(&bytes[start..index]).ok()?);
    let mut sign = 1i32;
    if bytes.get(index) == Some(&b'-') {
        sign = -1;
        index += 1;
    }
    let number_start = index;
    while bytes.get(index).is_some_and(|b| b.is_ascii_digit()) {
        index += 1;
    }
    let number = if index > number_start {
        core::str::from_utf8(&bytes[number_start..index])
            .ok()
            .and_then(|s| s.parse::<i32>().ok())
            .map(|n| n.saturating_mul(sign))
    } else {
        None
    };
    if bytes.get(index) == Some(&b' ') {
        index += 1;
    }
    Some((name, number, index))
}

fn rtf_flush_pending(
    chunks: &mut Vec<StyledChunk>,
    pending: &mut Option<u16>,
    style: RichTextStyle,
) {
    if pending.take().is_some() {
        rtf_push_char(chunks, '\u{FFFD}', style);
    }
}

pub fn import_rtf(bytes: &[u8]) -> Result<RichDocument, PersistenceError> {
    let mut input = bytes;
    if input.starts_with(&[0xEF, 0xBB, 0xBF]) {
        input = &input[3..];
    }
    while input.first().is_some_and(|b| b.is_ascii_whitespace()) {
        input = &input[1..];
    }
    if !input.starts_with(b"{\\rtf1") {
        return Err(PersistenceError::InvalidRtf);
    }

    let mut state = RtfState::default();
    let mut stack: Vec<RtfState> = Vec::new();
    let mut chunks = Vec::new();
    let mut index = 0usize;
    let mut skip_unicode = 0usize;
    let mut pending_high: Option<u16> = None;

    while index < input.len() {
        if skip_unicode != 0 {
            if input[index] == b'\\' {
                let next = if input.get(index + 1) == Some(&b'\'') {
                    index.saturating_add(4).min(input.len())
                } else if input
                    .get(index + 1)
                    .is_some_and(|b| matches!(*b, b'\\' | b'{' | b'}' | b'~' | b'-' | b'_'))
                {
                    index.saturating_add(2).min(input.len())
                } else if let Some((_, _, next)) = parse_rtf_control(input, index) {
                    next
                } else {
                    index.saturating_add(1).min(input.len())
                };
                index = next;
            } else {
                index += 1;
            }
            skip_unicode -= 1;
            continue;
        }
        match input[index] {
            b'{' => {
                if stack.len() >= MAX_RTF_NESTING {
                    return Err(PersistenceError::RtfNestingLimit);
                }
                stack.push(state);
                index += 1;
            }
            b'}' => {
                let Some(previous) = stack.pop() else {
                    return Err(PersistenceError::InvalidRtf);
                };
                rtf_flush_pending(&mut chunks, &mut pending_high, state.style);
                state = previous;
                index += 1;
            }
            b'\\' => {
                if input.get(index + 1) == Some(&b'*') {
                    state.ignorable = true;
                    index += 2;
                    continue;
                }
                if input.get(index + 1) == Some(&b'\'') {
                    if index + 3 >= input.len() {
                        return Err(PersistenceError::InvalidRtf);
                    }
                    let hex = core::str::from_utf8(&input[index + 2..index + 4])
                        .ok()
                        .and_then(|s| u8::from_str_radix(s, 16).ok())
                        .ok_or(PersistenceError::InvalidRtf)?;
                    if !state.destination_skip {
                        rtf_flush_pending(&mut chunks, &mut pending_high, state.style);
                        rtf_push_char(&mut chunks, hex as char, state.style);
                    }
                    index += 4;
                    continue;
                }
                if input
                    .get(index + 1)
                    .is_some_and(|b| matches!(*b, b'\\' | b'{' | b'}' | b'~' | b'-' | b'_'))
                {
                    rtf_flush_pending(&mut chunks, &mut pending_high, state.style);
                    let symbol = input[index + 1];
                    if !state.destination_skip {
                        let ch = match symbol {
                            b'~' => ' ',
                            b'-' => '\u{00AD}',
                            b'_' => '_',
                            other => other as char,
                        };
                        rtf_push_char(&mut chunks, ch, state.style);
                    }
                    index += 2;
                    continue;
                }
                let Some((name, number, next)) = parse_rtf_control(input, index) else {
                    return Err(PersistenceError::InvalidRtf);
                };
                index = next;
                if name.is_empty() {
                    continue;
                }
                match name.as_str() {
                    "*" => state.ignorable = true,
                    "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "object" => {
                        state.destination_skip = true
                    }
                    "b" => state.style.bold = number.unwrap_or(1) != 0,
                    "i" => state.style.italic = number.unwrap_or(1) != 0,
                    "ul" | "uldb" | "uld" | "uldash" | "uldashd" | "uldashdd" | "ulwave" => {
                        state.style.underline = number.unwrap_or(1) != 0
                    }
                    "ulnone" => state.style.underline = false,
                    "plain" => state.style = RichTextStyle::default(),
                    "uc" => state.uc_skip = number.unwrap_or(1).clamp(0, 16) as usize,
                    "u" => {
                        let value = number.ok_or(PersistenceError::InvalidRtf)?;
                        let code = (value as i16) as u16;
                        if let Some(high) = pending_high.take() {
                            if (0xDC00..=0xDFFF).contains(&code) {
                                let scalar = 0x10000
                                    + (((high as u32 - 0xD800) << 10) | (code as u32 - 0xDC00));
                                if !state.destination_skip {
                                    if let Some(ch) = char::from_u32(scalar) {
                                        rtf_push_char(&mut chunks, ch, state.style);
                                    }
                                }
                            } else if !state.destination_skip {
                                rtf_push_char(&mut chunks, '\u{FFFD}', state.style);
                                if let Some(ch) = char::from_u32(code as u32) {
                                    rtf_push_char(&mut chunks, ch, state.style);
                                }
                            }
                        } else if (0xD800..=0xDBFF).contains(&code) {
                            pending_high = Some(code);
                        } else if !state.destination_skip {
                            rtf_push_char(
                                &mut chunks,
                                char::from_u32(code as u32).unwrap_or('\u{FFFD}'),
                                state.style,
                            );
                        }
                        skip_unicode = state.uc_skip;
                    }
                    "par" => {
                        rtf_flush_pending(&mut chunks, &mut pending_high, state.style);
                        if !state.destination_skip {
                            rtf_push_char(&mut chunks, '\n', state.style);
                        }
                    }
                    "line" => {
                        rtf_flush_pending(&mut chunks, &mut pending_high, state.style);
                        if !state.destination_skip {
                            rtf_push_char(&mut chunks, '\n', state.style);
                        }
                    }
                    "tab" => {
                        rtf_flush_pending(&mut chunks, &mut pending_high, state.style);
                        if !state.destination_skip {
                            rtf_push_char(&mut chunks, '\t', state.style);
                        }
                    }
                    "'" => {}
                    _ => {
                        if state.ignorable {
                            state.destination_skip = true;
                        }
                    }
                }
            }
            b'\r' | b'\n' => index += 1,
            byte => {
                rtf_flush_pending(&mut chunks, &mut pending_high, state.style);
                if !state.destination_skip {
                    rtf_push_char(&mut chunks, byte as char, state.style);
                }
                index += 1;
            }
        }
    }
    rtf_flush_pending(&mut chunks, &mut pending_high, state.style);
    if !stack.is_empty() {
        return Err(PersistenceError::InvalidRtf);
    }
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

fn rtf_signed_unit(unit: u16, out: &mut String) {
    let signed = unit as i16;
    out.push_str("\\u");
    out.push_str(&signed.to_string());
    out.push('?');
}

fn export_rtf_text(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\par\n"),
            '\t' => out.push_str("\\tab "),
            c if (c as u32) <= 0x7f && !c.is_control() => out.push(c),
            c if (c as u32) <= 0xffff => rtf_signed_unit(c as u16, out),
            c => {
                let value = c as u32 - 0x1_0000;
                rtf_signed_unit(0xD800 | ((value >> 10) as u16), out);
                rtf_signed_unit(0xDC00 | ((value & 0x3ff) as u16), out);
            }
        }
    }
}

pub fn export_rtf(document: &RichDocument) -> String {
    let mut out = String::from("{\\rtf1\\ansi\\deff0\n");
    let mut current = RichTextStyle::default();
    for run in document.runs() {
        let style = run.style;
        if style.bold != current.bold {
            out.push_str(if style.bold { "\\b " } else { "\\b0 " });
        }
        if style.italic != current.italic {
            out.push_str(if style.italic { "\\i " } else { "\\i0 " });
        }
        if style.underline != current.underline {
            out.push_str(if style.underline { "\\ul " } else { "\\ul0 " });
        }
        export_rtf_text(&document.text()[run.range.clone()], &mut out);
        current = style;
    }
    out.push('}');
    out
}

pub fn export(format: DocumentFormat, document: &RichDocument) -> String {
    match format {
        DocumentFormat::Markdown => export_markdown(document),
        DocumentFormat::PlainText => export_plain_text(document),
        DocumentFormat::Rtf => export_rtf(document),
    }
}

pub fn loses_formatting(format: DocumentFormat, document: &RichDocument) -> bool {
    let capabilities = format.capabilities();
    document.runs().iter().any(|run| {
        (run.style.bold && !capabilities.bold)
            || (run.style.italic && !capabilities.italic)
            || (run.style.underline && !capabilities.underline)
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
        assert_eq!(format_from_path("a.rtf"), Some(DocumentFormat::Rtf));
        assert!(DocumentFormat::Rtf.capabilities().underline);
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
    fn rtf_session_target_is_clean_after_save() {
        let mut session = WriterDocumentSession::new();
        let document = RichDocument::from_text("rich");
        session.mark_changed(&document);
        session.set_saved_target(String::from("/tmp/rich.rtf"), DocumentFormat::Rtf);
        session.mark_saved(&document);
        assert_eq!(session.format, Some(DocumentFormat::Rtf));
        assert!(!session.is_dirty());
    }

    #[test]
    fn format_capabilities_drive_loss_warnings() {
        let mut document = RichDocument::from_text("styled");
        document.format(0..6, StyleProperty::Bold, true);
        document.format(0..6, StyleProperty::Italic, true);
        document.format(0..6, StyleProperty::Underline, true);
        assert!(!loses_formatting(DocumentFormat::Rtf, &document));
        assert!(loses_formatting(DocumentFormat::Markdown, &document));
        assert!(loses_formatting(DocumentFormat::PlainText, &document));
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

    #[test]
    fn rtf_round_trips_all_supported_styles_and_specials() {
        let mut document = RichDocument::from_text(
            "regular bold italic under both bu iu all\nC:\\Users\\Sunlight\\{draft}",
        );
        document.format(8..12, StyleProperty::Bold, true);
        document.format(13..19, StyleProperty::Italic, true);
        document.format(20..25, StyleProperty::Underline, true);
        document.format(26..30, StyleProperty::Bold, true);
        document.format(26..30, StyleProperty::Underline, true);
        document.format(31..35, StyleProperty::Italic, true);
        document.format(31..35, StyleProperty::Underline, true);
        document.format(36..39, StyleProperty::Bold, true);
        document.format(36..39, StyleProperty::Italic, true);
        document.format(36..39, StyleProperty::Underline, true);
        let encoded = export_rtf(&document);
        let decoded = import_rtf(encoded.as_bytes()).unwrap();
        assert_eq!(decoded.text(), document.text());
        assert_eq!(decoded.runs(), document.runs());
    }

    #[test]
    fn rtf_unicode_and_surrogates_round_trip() {
        let document = RichDocument::from_text("سلام از Sunlight Writer 🐇");
        let encoded = export_rtf(&document);
        let decoded = import_rtf(encoded.as_bytes()).unwrap();
        assert_eq!(decoded.text(), document.text());
    }

    #[test]
    fn rtf_unicode_skips_ascii_and_hex_fallbacks() {
        let ascii = import_rtf(br"{\rtf1\uc1\u1587? \u1588\'3f}").unwrap();
        assert_eq!(ascii.text(), "س ش");
    }

    #[test]
    fn rtf_groups_destinations_and_controls_are_safe() {
        let document = import_rtf(
            br"{\rtf1\ansi normal {\b bold} normal {\i italic} \par next {\fonttbl{\f0 Arial}} {\*\unknown hidden} end}",
        )
        .unwrap();
        assert_eq!(document.text(), "normal bold normal italic \nnext   end");
        assert!(style_at(&document, "bold").bold);
        assert!(style_at(&document, "italic").italic);
    }

    #[test]
    fn rtf_plain_and_style_resets_work() {
        let document = import_rtf(
            br"{\rtf1\b bold {\plain plain} bold \i both\i0 regular\ul under\ulnone plain}",
        )
        .unwrap();
        assert_eq!(document.text(), "bold plain bold bothregularunderplain");
        assert!(style_at(&document, "bold").bold);
        assert!(!style_at(&document, "plain").bold);
        assert!(style_at(&document, "both").bold && style_at(&document, "both").italic);
        assert!(style_at(&document, "under").underline);
    }

    #[test]
    fn malformed_rtf_and_excess_nesting_fail_without_panicking() {
        assert!(matches!(
            import_rtf(b"{\\rtf1 broken"),
            Err(PersistenceError::InvalidRtf)
        ));
        let mut deep = String::from("{\\rtf1");
        for _ in 0..(MAX_RTF_NESTING + 1) {
            deep.push('{');
        }
        assert!(matches!(
            import_rtf(deep.as_bytes()),
            Err(PersistenceError::RtfNestingLimit)
        ));
    }
}
