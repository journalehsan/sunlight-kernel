use super::{
    constants,
    handle::NodeHandle,
    tag::{Attributes, HTMLTag, Node},
};
use crate::InnerNodeHandle;
use crate::{bytes::Bytes, inline::vec::InlineVec, simd, ParseError};
use crate::{stream::Stream, ParserOptions};
use std::collections::HashMap;

/// A list of HTML nodes
pub type Tree<'a> = Vec<Node<'a>>;

/// Inline class vector
pub type ClassVec = InlineVec<NodeHandle, 2>;

/// HTML Version (<!DOCTYPE>)
#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(C)]
pub enum HTMLVersion {
    /// HTML Version 5
    HTML5,
    /// Strict HTML 4.01
    StrictHTML401,
    /// Transitional HTML 4.01
    TransitionalHTML401,
    /// Frameset HTML 4.01:
    FramesetHTML401,
}
/// The main HTML parser
///
/// Users of this library are not supposed to directly construct this struct.
/// Instead, users must call `tl::parse()` and use the returned `VDom`.
#[derive(Debug)]
pub struct Parser<'a> {
    /// The inner stream that is used to iterate through the HTML source
    pub(crate) stream: Stream<'a, u8>,
    pub(crate) stack: Vec<NodeHandle>,
    /// Specified options for this HTML parser
    pub(crate) options: ParserOptions,
    /// A global collection of all HTML tags that appear in the source code
    ///
    /// HTML Nodes contain indicies into this vector
    pub(crate) tags: Tree<'a>,
    /// The topmost HTML nodes
    pub(crate) ast: Vec<NodeHandle>,
    /// A HashMap that maps Tag ID to a Node ID
    pub(crate) ids: HashMap<Bytes<'a>, NodeHandle>,
    /// A HashMap that maps Tag Class to a Node ID
    pub(crate) classes: HashMap<Bytes<'a>, ClassVec>,
    /// The current HTML version, if set
    pub(crate) version: Option<HTMLVersion>,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(input: &str, options: ParserOptions) -> Parser {
        Parser {
            stack: Vec::with_capacity(4),
            options,
            tags: Vec::new(),
            stream: Stream::new(input.as_bytes()),
            ast: Vec::new(),
            ids: HashMap::new(),
            classes: HashMap::new(),
            version: None,
        }
    }

    #[inline(always)]
    fn register_tag(&mut self, node: Node<'a>) -> NodeHandle {
        self.tags.push(node);
        NodeHandle::new((self.tags.len() - 1) as u32)
    }

    #[inline(always)]
    fn skip_whitespaces(&mut self) {
        self.read_while2(b' ', b'\n');
    }

    fn read_to(&mut self, needle: u8) -> &'a [u8] {
        let start = self.stream.idx;
        let bytes = &self.stream.data()[start..];

        let end = simd::find(bytes, needle).unwrap_or_else(|| self.stream.len() - start);

        self.stream.idx += end;
        self.stream.slice(start, start + end)
    }

    fn read_while2(&mut self, needle1: u8, needle2: u8) -> Option<()> {
        loop {
            let ch = self.stream.current_cpy()?;

            let eq1 = ch == needle1;
            let eq2 = ch == needle2;

            if !eq1 & !eq2 {
                return Some(());
            }

            self.stream.advance();
        }
    }

    fn read_ident(&mut self) -> Option<&'a [u8]> {
        let start = self.stream.idx;
        let bytes = &self.stream.data()[start..];

        // If we do not find any characters that are not identifiers
        // then we are probably at the end of the stream
        let end = simd::search_non_ident(bytes).unwrap_or_else(|| self.stream.len() - start);

        self.stream.idx += end;
        Some(self.stream.slice(start, start + end))
    }

    fn skip_comment_with_start(&mut self, start: usize) -> &'a [u8] {
        while !self.stream.is_eof() {
            let idx = self.stream.idx;

            if self
                .stream
                .slice_len(idx, constants::COMMENT.len())
                .eq(constants::COMMENT)
            {
                self.stream.advance_by(constants::COMMENT.len());

                let is_end_of_comment = self.stream.expect_and_skip_cond(b'>');

                if is_end_of_comment {
                    return self.stream.slice(start, self.stream.idx);
                }
            }

            self.stream.advance();
        }

        &[]
    }

    fn parse_attribute(&mut self) -> Option<(&'a [u8], Option<&'a [u8]>)> {
        let name = self.read_ident()?;
        self.skip_whitespaces();

        let has_value = self.stream.expect_and_skip_cond(b'=');
        if !has_value {
            return Some((name, None));
        }

        self.skip_whitespaces();

        let value = if let Some(quote) = self.stream.expect_oneof_and_skip(&[b'"', b'\'']) {
            self.read_to(quote)
        } else {
            // A slash is legal in an unquoted HTML attribute value, including
            // the leading slash of a relative URL. Only HTML whitespace and
            // `>` terminate this state (even a trailing slash belongs to it).
            let start = self.stream.idx;
            while let Some(ch) = self.stream.current_cpy() {
                if ch == b'>' || matches!(ch, b' ' | b'\t' | b'\n' | b'\r' | 0x0c) {
                    break;
                }
                self.stream.advance();
            }
            self.stream.slice(start, self.stream.idx)
        };

        Some((name, Some(value)))
    }

    fn parse_attributes(&mut self) -> Option<Attributes<'a>> {
        let mut attributes = Attributes::new();

        loop {
            self.skip_whitespaces();

            let cur = self.stream.current_cpy()?;

            if simd::is_closing(cur) {
                break;
            }

            if let Some((key, value)) = self.parse_attribute() {
                let value: Option<Bytes<'a>> = value.map(Into::into);

                match key {
                    b"id" => attributes.id = value,
                    b"class" => attributes.class = value,
                    _ => attributes.raw.insert(key.into(), value),
                };
            }

            if !simd::is_closing(self.stream.current_cpy()?) {
                self.stream.advance();
            }
        }

        Some(attributes)
    }

    #[inline]
    fn add_to_parent(&mut self, handle: NodeHandle) {
        if let Some(last) = self.stack.last() {
            let last = self
                .tags
                .get_mut(last.get_inner() as usize)
                .unwrap()
                .as_tag_mut()
                .unwrap();

            last._children.push(handle);
        } else {
            self.ast.push(handle);
        }
    }

    fn read_end(&mut self) {
        self.stream.advance();

        let closing_tag_name = self
            .read_to(b'>')
            .split(|b| b.is_ascii_whitespace() || *b == b'/')
            .next()
            .unwrap_or_default();

        self.stream.expect_and_skip_cond(b'>');

        // End tags close intervening elements with omitted end tags. This
        // also handles </tr> while a td/th is still open in classic tables.
        let position = self.stack.iter().rposition(|handle| {
            handle
                .get(self)
                .and_then(|node| node.as_tag())
                .is_some_and(|tag| tag.name().as_bytes().eq_ignore_ascii_case(closing_tag_name))
        });
        let Some(position) = position else { return };
        while self.stack.len() > position {
            self.finish_open_tag();
        }
    }

    fn finish_open_tag(&mut self) {
        if let Some(handle) = self.stack.pop() {
            let tag = self
                .tags
                .get_mut(handle.get_inner() as usize)
                .unwrap()
                .as_tag_mut()
                .unwrap();

            let ptr = self.stream.data().as_ptr() as usize;
            let offset = tag._raw.as_ptr() as usize;
            let offset = offset - ptr;

            tag._raw = self.stream.slice(offset, self.stream.idx).into();

            let (track_classes, track_ids) = (
                self.options.is_tracking_classes(),
                self.options.is_tracking_ids(),
            );

            if let (true, Some(bytes)) = (track_classes, &tag._attributes.class) {
                let s = bytes
                    .as_bytes_borrowed()
                    .and_then(|x| std::str::from_utf8(x).ok())
                    .map(|x| x.split_ascii_whitespace());

                if let Some(s) = s {
                    for class in s {
                        self.classes
                            .entry(class.into())
                            .or_insert_with(InlineVec::new)
                            .push(handle);
                    }
                }
            }

            if let (true, Some(bytes)) = (track_ids, &tag._attributes.id) {
                self.ids.insert(bytes.clone(), handle);
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn read_markdown(&mut self) -> Option<()> {
        let start = self.stream.idx - 1; // position of the < which is needed when registering the comment

        self.stream.advance(); // skip !

        let is_comment = self
            .stream
            .slice_len(self.stream.idx, 2)
            .eq(constants::COMMENT);

        if is_comment {
            let comment = self.skip_comment_with_start(start);
            let comment = self.register_tag(Node::Comment(comment.into()));
            self.add_to_parent(comment);
        } else {
            let tag = self.read_ident()?;

            self.skip_whitespaces();

            if simd::matches_case_insensitive(tag, *b"doctype") {
                let doctype = self.read_ident()?;

                let html5 = simd::matches_case_insensitive(doctype, *b"html");

                if html5 {
                    self.version = Some(HTMLVersion::HTML5);
                }

                self.skip_whitespaces();
                self.stream.advance(); // skip >
            }
        }

        Some(())
    }

    /// Script and style contents are text, even when they contain `<`, HTML
    /// strings, or comments. Only an appropriate end tag returns to markup.
    fn read_raw_text(&mut self, name: &[u8]) {
        let start = self.stream.idx;
        let data = self.stream.data();
        let end = (start..data.len())
            .find(|&i| {
                data[i..].starts_with(b"</")
                    && data
                        .get(i + 2..i + 2 + name.len())
                        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
                    && data
                        .get(i + 2 + name.len())
                        .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'>' | b'/'))
            })
            .unwrap_or(data.len());
        self.stream.idx = end;
        if end > start {
            let raw = self.register_tag(Node::Raw(self.stream.slice(start, end).into()));
            self.add_to_parent(raw);
        }
        if !self.stream.is_eof() {
            self.stream.advance(); // `<`; read_end consumes the slash
            self.read_end();
        }
    }

    /// Apply the optional-end rules within the nearest table/list scope.
    /// An inner table must never close a cell in its enclosing table.
    fn close_optional_tags(&mut self, name: &[u8]) {
        let names: &[&[u8]] =
            if name.eq_ignore_ascii_case(b"td") || name.eq_ignore_ascii_case(b"th") {
                &[b"td", b"th"]
            } else if name.eq_ignore_ascii_case(b"tr") {
                &[b"tr"]
            } else if [b"thead".as_slice(), b"tbody", b"tfoot"]
                .iter()
                .any(|s| name.eq_ignore_ascii_case(s))
            {
                &[b"thead", b"tbody", b"tfoot"]
            } else if name.eq_ignore_ascii_case(b"li") {
                &[b"li"]
            } else if name.eq_ignore_ascii_case(b"dt") || name.eq_ignore_ascii_case(b"dd") {
                &[b"dt", b"dd"]
            } else {
                return;
            };
        let mut position = None;
        for (index, handle) in self.stack.iter().enumerate().rev() {
            let tag = handle.get(self).and_then(|node| node.as_tag()).unwrap();
            let open = tag.name().as_bytes();
            if names.iter().any(|s| open.eq_ignore_ascii_case(s)) {
                position = Some(index);
                break;
            }
            if [b"table".as_slice(), b"ul", b"ol", b"dl"]
                .iter()
                .any(|s| open.eq_ignore_ascii_case(s))
            {
                break;
            }
        }
        if let Some(position) = position {
            while self.stack.len() > position {
                self.finish_open_tag();
            }
        }
    }

    fn parse_tag(&mut self) -> Option<()> {
        let start = self.stream.idx;

        self.stream.advance();
        self.skip_whitespaces();
        let cur = self.stream.current_cpy()?;

        match cur {
            b'/' => self.read_end(),
            b'!' => {
                self.read_markdown();
            }
            _ => {
                let name = self.read_ident()?;
                self.skip_whitespaces();

                self.close_optional_tags(name);

                let attr = self.parse_attributes()?;

                let is_self_closing = self.stream.expect_and_skip_cond(b'/');

                self.stream.expect_and_skip(b'>')?;

                let this = self.register_tag(Node::Tag(HTMLTag::new(
                    name.into(),
                    attr,
                    InlineVec::new(),
                    self.stream.slice(start, self.stream.idx).into(),
                )));

                self.add_to_parent(this);

                // some tags are self closing, so even though there might not be a /,
                // we don't always want to push them to the stack
                // e.g. <br><p>Hello</p>
                // <p> should not be a subtag of <br>
                if !is_self_closing
                    && !constants::VOID_TAGS
                        .iter()
                        .any(|tag| name.eq_ignore_ascii_case(tag))
                {
                    self.stack.push(this);
                    if name.eq_ignore_ascii_case(b"script") || name.eq_ignore_ascii_case(b"style") {
                        self.read_raw_text(name);
                    }
                }
            }
        };

        Some(())
    }

    pub(crate) fn parse_single(&mut self) -> Option<()> {
        loop {
            let cur = self.stream.current()?;

            if *cur == b'<' {
                self.parse_tag();
            } else {
                let raw = Node::Raw(self.read_to(b'<').into());
                let handle = self.register_tag(raw);
                self.add_to_parent(handle);
            }
        }
    }

    /// Resolves an internal Node ID obtained from a NodeHandle to a Node
    #[inline]
    pub fn resolve_node_id(&self, id: InnerNodeHandle) -> Option<&Node<'a>> {
        self.tags.get(id as usize)
    }

    /// Resolves an internal Node ID obtained from a NodeHandle to a Node
    #[inline]
    pub fn resolve_node_id_mut(&mut self, id: InnerNodeHandle) -> Option<&mut Node<'a>> {
        self.tags.get_mut(id as usize)
    }

    pub(crate) fn parse(&mut self) -> Result<(), ParseError> {
        if self.stream.len() > u32::MAX as usize {
            return Err(ParseError::InvalidLength);
        }

        while !self.stream.is_eof() {
            self.parse_single();
        }

        Ok(())
    }
}
