//! Freestanding fallback HTML parser used on `target_os = "none"`.
//!
//! The std-backed `tl` parser remains the primary backend on host targets.
//! This fallback keeps Golden Fish usable inside SunlightOS userland where
//! `std` is unavailable.

use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::attributes::Attribute;
use crate::document::Document;
use crate::error::ParseError;
use crate::node::{Node, NodeId};

pub(crate) fn parse_with_fallback(source: &str) -> Result<Document, ParseError> {
    if source.len() > u32::MAX as usize {
        return Err(ParseError::new("parse error: input is too large"));
    }

    let mut parser = FallbackParser::new(source);
    parser.parse();
    Ok(parser.finish())
}

struct FallbackParser<'a> {
    source: &'a str,
    pos: usize,
    document: Document,
    stack: Vec<NodeId>,
}

impl<'a> FallbackParser<'a> {
    fn new(source: &'a str) -> Self {
        let document = Document::new();
        let root = document.root();
        Self {
            source,
            pos: 0,
            document,
            stack: vec![root],
        }
    }

    fn finish(self) -> Document {
        self.document
    }

    fn parse(&mut self) {
        while self.pos < self.source.len() {
            if let Some(tag_name) = self.current_raw_text_tag() {
                self.parse_raw_text(&tag_name);
                continue;
            }

            if self.starts_with("<!--") {
                self.parse_comment();
                continue;
            }
            if self.starts_with_ci("</") {
                self.parse_end_tag();
                continue;
            }
            if self.starts_with("<!") || self.starts_with("<?") {
                self.skip_declaration();
                continue;
            }
            if self.current_byte() == Some(b'<') {
                if self.parse_start_tag() {
                    continue;
                }
                self.append_text_slice(self.pos, self.pos + 1);
                self.pos += 1;
                continue;
            }

            self.parse_text();
        }
    }

    fn parse_text(&mut self) {
        let start = self.pos;
        while self.pos < self.source.len() && self.current_byte() != Some(b'<') {
            self.pos += 1;
        }
        self.append_text_slice(start, self.pos);
    }

    fn parse_comment(&mut self) {
        let start = self.pos + 4;
        let end = find_substring(&self.source[start..], "-->")
            .map_or(self.source.len(), |offset| start + offset);
        let content = &self.source[start..end];
        self.append_leaf(Node::Comment {
            content: content.to_string(),
        });
        self.pos = end;
        if self.starts_with("-->") {
            self.pos += 3;
        }
    }

    fn parse_end_tag(&mut self) {
        self.pos += 2;
        self.skip_whitespace();
        let tag_start = self.pos;
        self.consume_name();
        let tag_name = self.source[tag_start..self.pos].to_ascii_lowercase();
        self.skip_until_gt();
        if self.current_byte() == Some(b'>') {
            self.pos += 1;
        }

        if tag_name.is_empty() {
            return;
        }

        let mut found_index = None;
        for index in (1..self.stack.len()).rev() {
            let Some(node) = self.document.get(self.stack[index]) else {
                continue;
            };
            if node
                .tag_name()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(&tag_name))
            {
                found_index = Some(index);
                break;
            }
        }

        if let Some(index) = found_index {
            self.stack.truncate(index);
        }
    }

    fn parse_raw_text(&mut self, tag_name: &str) {
        let Some(end) = find_end_tag_case_insensitive(self.source, self.pos, tag_name) else {
            self.append_text_slice(self.pos, self.source.len());
            self.pos = self.source.len();
            return;
        };

        self.append_text_slice(self.pos, end);
        self.pos = end;
    }

    fn skip_declaration(&mut self) {
        self.pos += 2;
        self.skip_until_gt();
        if self.current_byte() == Some(b'>') {
            self.pos += 1;
        }
    }

    fn parse_start_tag(&mut self) -> bool {
        let tag_open = self.pos;
        self.pos += 1;
        let tag_name_start = self.pos;
        self.consume_name();
        if tag_name_start == self.pos {
            self.pos = tag_open;
            return false;
        }

        let tag_name = self.source[tag_name_start..self.pos].to_ascii_lowercase();
        let mut attributes = Vec::new();
        let mut self_closing = false;

        loop {
            self.skip_whitespace();
            if self.pos >= self.source.len() {
                break;
            }

            if self.starts_with("/>") {
                self.pos += 2;
                self_closing = true;
                break;
            }

            if self.current_byte() == Some(b'>') {
                self.pos += 1;
                break;
            }

            let attr_name_start = self.pos;
            self.consume_attr_name();
            if attr_name_start == self.pos {
                self.pos += 1;
                continue;
            }

            let name = self.source[attr_name_start..self.pos].to_string();
            self.skip_whitespace();
            let value = if self.current_byte() == Some(b'=') {
                self.pos += 1;
                self.skip_whitespace();
                self.parse_attr_value()
            } else {
                String::new()
            };
            attributes.push(Attribute::new(name, value));
        }

        let element_id = self.append_element(&tag_name, attributes);
        if !self_closing && !is_void_element(&tag_name) {
            self.stack.push(element_id);
        }
        true
    }

    fn parse_attr_value(&mut self) -> String {
        match self.current_byte() {
            Some(b'"') | Some(b'\'') => {
                let quote = self.current_byte().unwrap_or_default();
                self.pos += 1;
                let start = self.pos;
                while self.pos < self.source.len() && self.current_byte() != Some(quote) {
                    self.pos += 1;
                }
                let value = self.source[start..self.pos].to_string();
                if self.current_byte() == Some(quote) {
                    self.pos += 1;
                }
                value
            }
            _ => {
                let start = self.pos;
                while self.pos < self.source.len() {
                    match self.current_byte() {
                        Some(byte) if is_space(byte) || byte == b'>' => break,
                        Some(byte) if byte == b'/' && self.peek_byte(1) == Some(b'>') => break,
                        Some(_) => self.pos += 1,
                        None => break,
                    }
                }
                self.source[start..self.pos].to_string()
            }
        }
    }

    fn append_element(&mut self, tag_name: &str, attributes: Vec<Attribute>) -> NodeId {
        let node_id = self.document.alloc_node(Node::Element {
            tag_name: tag_name.to_string(),
            attributes,
            children: Vec::new(),
        });
        self.append_to_current_parent(node_id);
        node_id
    }

    fn append_leaf(&mut self, node: Node) {
        let node_id = self.document.alloc_node(node);
        self.append_to_current_parent(node_id);
    }

    fn append_to_current_parent(&mut self, node_id: NodeId) {
        let parent_id = self
            .stack
            .last()
            .copied()
            .unwrap_or_else(|| self.document.root());
        self.document.append_child(parent_id, node_id);
        self.document.set_parent(node_id, parent_id);
    }

    fn append_text_slice(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        self.append_leaf(Node::Text {
            content: self.source[start..end].to_string(),
        });
    }

    fn current_raw_text_tag(&self) -> Option<String> {
        let current_id = *self.stack.last()?;
        if current_id == self.document.root() {
            return None;
        }

        let tag_name = self.document.tag_name(current_id)?;
        if matches!(tag_name, "script" | "style") {
            Some(tag_name.to_string())
        } else {
            None
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.source.len() && self.current_byte().is_some_and(is_space) {
            self.pos += 1;
        }
    }

    fn skip_until_gt(&mut self) {
        while self.pos < self.source.len() && self.current_byte() != Some(b'>') {
            self.pos += 1;
        }
    }

    fn consume_name(&mut self) {
        while self.pos < self.source.len() && self.current_byte().is_some_and(is_name_char) {
            self.pos += 1;
        }
    }

    fn consume_attr_name(&mut self) {
        while self.pos < self.source.len() && self.current_byte().is_some_and(is_attr_name_char) {
            self.pos += 1;
        }
    }

    fn current_byte(&self) -> Option<u8> {
        self.source.as_bytes().get(self.pos).copied()
    }

    fn peek_byte(&self, offset: usize) -> Option<u8> {
        self.source.as_bytes().get(self.pos + offset).copied()
    }

    fn starts_with(&self, pattern: &str) -> bool {
        self.source[self.pos..].starts_with(pattern)
    }

    fn starts_with_ci(&self, pattern: &str) -> bool {
        starts_with_case_insensitive_at(self.source, self.pos, pattern)
    }
}

fn find_substring(haystack: &str, needle: &str) -> Option<usize> {
    haystack.find(needle)
}

fn find_end_tag_case_insensitive(source: &str, start: usize, tag_name: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let pattern = tag_name.as_bytes();
    let mut index = start;
    while index < bytes.len() {
        if bytes[index] != b'<' || index + 2 >= bytes.len() {
            index += 1;
            continue;
        }
        if bytes[index + 1] != b'/' {
            index += 1;
            continue;
        }

        let mut cursor = index + 2;
        while cursor < bytes.len() && is_space(bytes[cursor]) {
            cursor += 1;
        }
        if cursor + pattern.len() > bytes.len() {
            return None;
        }

        if bytes[cursor..cursor + pattern.len()]
            .iter()
            .zip(pattern.iter())
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
        {
            let next = bytes.get(cursor + pattern.len()).copied();
            if next.is_none() || next.is_some_and(|byte| is_space(byte) || byte == b'>') {
                return Some(index);
            }
        }

        index += 1;
    }
    None
}

fn starts_with_case_insensitive_at(source: &str, start: usize, pattern: &str) -> bool {
    let haystack = source.as_bytes();
    let needle = pattern.as_bytes();
    haystack
        .get(start..start + needle.len())
        .is_some_and(|slice| {
            slice
                .iter()
                .zip(needle.iter())
                .all(|(left, right)| left.eq_ignore_ascii_case(right))
        })
}

fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b':' | b'_')
}

fn is_attr_name_char(byte: u8) -> bool {
    is_name_char(byte) || byte == b'.'
}

fn is_space(byte: u8) -> bool {
    byte.is_ascii_whitespace()
}

fn is_void_element(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

#[cfg(test)]
mod tests {
    use super::parse_with_fallback;
    use crate::{Document, Node, NodeId};

    fn parse(html: &str) -> Document {
        parse_with_fallback(html).expect("fallback parse should succeed")
    }

    #[test]
    fn parses_basic_document_structure() {
        let document = parse("<html><head></head><body><p>Hello</p></body></html>");
        assert_eq!(
            document.tag_name(document.children(document.root())[0]),
            Some("html")
        );
        assert!(document.find_first_element("head").is_some());
        assert!(document.find_first_element("body").is_some());
        assert_eq!(
            document.text_content(document.children(document.find_first_element("p").unwrap())[0]),
            Some("Hello")
        );
    }

    #[test]
    fn preserves_fragments_and_comments() {
        let document = parse("<!-- hello --><p>First</p><p>Second</p>");
        assert!(matches!(
            document.get(document.children(document.root())[0]),
            Some(Node::Comment { .. })
        ));
        let mut p_count = 0;
        fn count_p(document: &Document, id: NodeId, count: &mut usize) {
            if document.tag_name(id) == Some("p") {
                *count += 1;
            }
            for &child in document.children(id) {
                count_p(document, child, count);
            }
        }
        count_p(&document, document.root(), &mut p_count);
        assert_eq!(p_count, 2);
    }

    #[test]
    fn tolerates_malformed_markup_without_panicking() {
        let document = parse("<div><p>text<div></p>");
        assert!(document.find_first_element("div").is_some());
        assert!(document.find_first_element("p").is_some());
    }

    #[test]
    fn preserves_script_text_as_text_node() {
        let document = parse("<script>if (a < b) { ok(); }</script>");
        let script_id = document.find_first_element("script").unwrap();
        let children = document.children(script_id);
        assert_eq!(children.len(), 1);
        assert_eq!(
            document.text_content(children[0]),
            Some("if (a < b) { ok(); }")
        );
    }
}
