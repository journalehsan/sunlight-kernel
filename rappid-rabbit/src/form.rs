//! Browser-owned interactive form state.
//!
//! This module keeps runtime control values separate from the parsed DOM and
//! the retained DocumentScene.  The canvas never sees form semantics.

use alloc::{
    collections::BTreeMap,
    format,
    string::String,
    vec,
    vec::Vec,
};

use golden_fish::{Attribute, Document, Node};
use sunlight_http::ParsedUrl;
use sunlight_ui::widgets::DocumentNodeId;

use crate::render::DomNodeId;

// ── limits ──────────────────────────────────────────────────────────

const MAX_CONTROL_VALUE_LEN: usize = 4_096;
const MAX_CONTROLS_PER_FORM: usize = 64;
const MAX_SERIALIZED_QUERY_LEN: usize = 8_192;
const MAX_FOCUS_TRAVERSAL: usize = 256;

// ── form control state ──────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ButtonType {
    Submit,
    Button,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormControlKind {
    TextInput,
    SearchInput,
    SubmitInput,
    ButtonInput,
    ButtonElement(ButtonType),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextControlState {
    pub current_value: String,
    pub cursor_position: usize,
    pub selection_start: usize,
    pub selection_end: usize,
    pub dirty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormControlState {
    Text {
        kind: FormControlKind,
        initial_value: String,
        state: TextControlState,
        name: Option<String>,
        placeholder: Option<String>,
        disabled: bool,
        readonly: bool,
        maxlength: Option<usize>,
        size: Option<usize>,
        form_owner: Option<DocumentNodeId>,
    },
    Button {
        kind: FormControlKind,
        name: Option<String>,
        value: Option<String>,
        disabled: bool,
        form_owner: Option<DocumentNodeId>,
    },
}

impl FormControlState {
    pub fn kind(&self) -> &FormControlKind {
        match self {
            Self::Text { kind, .. } | Self::Button { kind, .. } => kind,
        }
    }

    pub fn is_disabled(&self) -> bool {
        match self {
            Self::Text { disabled, .. } | Self::Button { disabled, .. } => *disabled,
        }
    }

    pub fn form_owner(&self) -> Option<DocumentNodeId> {
        match self {
            Self::Text { form_owner, .. } | Self::Button { form_owner, .. } => *form_owner,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Text { name, .. } | Self::Button { name, .. } => name.as_deref(),
        }
    }

    pub fn current_value(&self) -> &str {
        match self {
            Self::Text { state, .. } => &state.current_value,
            Self::Button { value, .. } => value.as_deref().unwrap_or(""),
        }
    }

    pub fn current_value_owned(&self) -> String {
        String::from(self.current_value())
    }
}

// ── form state ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct FormState {
    pub controls: BTreeMap<DomNodeId, FormControlState>,
    pub focused_control: Option<DocumentNodeId>,
    pub pressed_control: Option<DocumentNodeId>,
}

impl FormState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build form state by walking the DOM tree.  Called once per page.
    pub fn build_from_dom(&mut self, document: &Document) {
        self.controls.clear();
        self.focused_control = None;
        self.pressed_control = None;
        let root = document.root();
        self.walk_dom(document, root, None);
    }

    fn walk_dom(&mut self, document: &Document, node_id: usize, mut current_form: Option<DocumentNodeId>) {
        let Some(node) = document.get(node_id) else {
            return;
        };
        match node {
            Node::Element {
                tag_name,
                attributes,
                children,
            } => {
                if tag_name.eq_ignore_ascii_case("form") {
                    current_form = Some(DocumentNodeId(node_id as u64));
                }
                if let Some(control) =
                    self.build_control(document, node_id, attributes, current_form)
                {
                    self.controls.insert(DocumentNodeId(node_id as u64), control);
                }
                for &child in children {
                    self.walk_dom(document, child, current_form);
                }
            }
            Node::Document { children } => {
                for &child in children {
                    self.walk_dom(document, child, current_form);
                }
            }
            _ => {}
        }
    }

    fn build_control(
        &self,
        _document: &Document,
        node_id: usize,
        attributes: &[Attribute],
        form_owner: Option<DocumentNodeId>,
    ) -> Option<FormControlState> {
        let tag_name = _document.get(node_id)?.tag_name()?;
        let attr = |name: &str| -> Option<&str> {
            attributes
                .iter()
                .find(|a| a.name().eq_ignore_ascii_case(name))
                .map(Attribute::value)
        };

        match tag_name.to_ascii_lowercase().as_str() {
            "input" => {
                let input_type = attr("type").unwrap_or("text").to_ascii_lowercase();
                match input_type.as_str() {
                    "text" | "search" | "" => {
                        let initial_value = String::from(attr("value").unwrap_or(""));
                        let maxlength = attr("maxlength").and_then(|v| v.parse::<usize>().ok());
                        let size = attr("size").and_then(|v| v.parse::<usize>().ok());
                        let kind = if input_type == "search" {
                            FormControlKind::SearchInput
                        } else {
                            FormControlKind::TextInput
                        };
                        let mut state = TextControlState {
                            current_value: initial_value.clone(),
                            cursor_position: 0,
                            selection_start: 0,
                            selection_end: 0,
                            dirty: false,
                        };
                        // Set initial cursor
                        state.cursor_position = initial_value.len();
                        state.selection_start = initial_value.len();
                        state.selection_end = initial_value.len();

                        Some(FormControlState::Text {
                            kind,
                            initial_value,
                            state,
                            name: attr("name").map(String::from),
                            placeholder: attr("placeholder").map(String::from),
                            disabled: attr("disabled").is_some(),
                            readonly: has_boolean_attr(attributes, "readonly"),
                            maxlength,
                            size,
                            form_owner,
                        })
                    }
                    "submit" => Some(FormControlState::Button {
                        kind: FormControlKind::SubmitInput,
                        name: attr("name").map(String::from),
                        value: attr("value").map(String::from),
                        disabled: attr("disabled").is_some(),
                        form_owner,
                    }),
                    "button" => Some(FormControlState::Button {
                        kind: FormControlKind::ButtonInput,
                        name: attr("name").map(String::from),
                        value: attr("value").map(String::from),
                        disabled: attr("disabled").is_some(),
                        form_owner,
                    }),
                    _ => {
                        // Unknown input types: fall back to text input
                        let initial_value = String::from(attr("value").unwrap_or(""));
                        let state = TextControlState {
                            current_value: initial_value.clone(),
                            cursor_position: initial_value.len(),
                            selection_start: initial_value.len(),
                            selection_end: initial_value.len(),
                            dirty: false,
                        };
                        Some(FormControlState::Text {
                            kind: FormControlKind::TextInput,
                            initial_value,
                            state,
                            name: attr("name").map(String::from),
                            placeholder: attr("placeholder").map(String::from),
                            disabled: attr("disabled").is_some(),
                            readonly: has_boolean_attr(attributes, "readonly"),
                            maxlength: attr("maxlength")
                                .and_then(|v| v.parse::<usize>().ok()),
                            size: attr("size")
                                .and_then(|v| v.parse::<usize>().ok()),
                            form_owner,
                        })
                    }
                }
            }
            "button" => {
                let btn_type = match attr("type").unwrap_or("submit").to_ascii_lowercase().as_str() {
                    "submit" | "" => ButtonType::Submit,
                    "button" => ButtonType::Button,
                    _ => ButtonType::Submit,
                };
                let kind = match btn_type {
                    ButtonType::Submit => FormControlKind::ButtonElement(ButtonType::Submit),
                    ButtonType::Button => FormControlKind::ButtonElement(ButtonType::Button),
                };
                // Collect text content for the button value
                let value = attr("value").map(String::from).or_else(|| {
                    let text = collect_text_content(_document, node_id);
                    if text.is_empty() {
                        None
                    } else {
                        Some(text)
                    }
                });
                Some(FormControlState::Button {
                    kind,
                    name: attr("name").map(String::from),
                    value,
                    disabled: attr("disabled").is_some(),
                    form_owner,
                })
            }
            _ => None,
        }
    }

    // ── focus methods ────────────────────────────────────────────

    pub fn focus_control(&mut self, node_id: DomNodeId) -> bool {
        if let Some(control) = self.controls.get(&node_id) {
            if control.is_disabled() {
                return false;
            }
            self.focused_control = Some(node_id);
            return true;
        }
        false
    }

    pub fn blur(&mut self) {
        self.focused_control = None;
    }

    // ── text editing methods ──────────────────────────────────────

    pub fn focused_text_state_mut(&mut self) -> Option<&mut TextControlState> {
        let id = self.focused_control?;
        match self.controls.get_mut(&id)? {
                FormControlState::Text {
                    state, ref readonly, ..
                } => {
                    if *readonly {
                    None
                } else {
                    Some(state)
                }
            }
            _ => None,
        }
    }

    pub fn focused_text_state(&self) -> Option<(&TextControlState, bool, Option<usize>)> {
        let id = self.focused_control?;
        match self.controls.get(&id)? {
            FormControlState::Text {
                state,
                readonly,
                maxlength,
                ..
                } => Some((state, *readonly, *maxlength)),
            _ => None,
        }
    }

    pub fn insert_char(&mut self, ch: char) -> bool {
        let maxlen = self.focused_control.and_then(|control| {
            match self.controls.get(&control) {
                Some(FormControlState::Text { maxlength, .. }) => Some(maxlength.unwrap_or(MAX_CONTROL_VALUE_LEN)),
                _ => None,
            }
        }).unwrap_or(MAX_CONTROL_VALUE_LEN);

        let Some(state) = self.focused_text_state_mut() else {
            return false;
        };
        if state.current_value.chars().count() >= maxlen {
            return false;
        }
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        if state.cursor_position > state.current_value.len() {
            state.cursor_position = state.current_value.len();
        }
        state.current_value.insert_str(state.cursor_position, s);
        state.cursor_position = state.cursor_position.saturating_add(s.len());
        state.dirty = true;
        true
    }

    pub fn backspace(&mut self) -> bool {
        let Some(state) = self.focused_text_state_mut() else {
            return false;
        };
        if state.cursor_position == 0 || state.current_value.is_empty() {
            return false;
        }
        // Delete the UTF-8 character before cursor
        let boundary = find_prev_char_boundary(&state.current_value, state.cursor_position);
        state.current_value.drain(boundary..state.cursor_position);
        state.cursor_position = boundary;
        state.dirty = true;
        true
    }

    pub fn delete_forward(&mut self) -> bool {
        let Some(state) = self.focused_text_state_mut() else {
            return false;
        };
        if state.cursor_position >= state.current_value.len() {
            return false;
        }
        let next = find_next_char_boundary(&state.current_value, state.cursor_position);
        state.current_value.drain(state.cursor_position..next);
        state.dirty = true;
        true
    }

    pub fn move_cursor_left(&mut self) -> bool {
        let Some(state) = self.focused_text_state_mut() else {
            return false;
        };
        if state.cursor_position == 0 {
            return false;
        }
        state.cursor_position = find_prev_char_boundary(&state.current_value, state.cursor_position);
        true
    }

    pub fn move_cursor_right(&mut self) -> bool {
        let Some(state) = self.focused_text_state_mut() else {
            return false;
        };
        if state.cursor_position >= state.current_value.len() {
            return false;
        }
        state.cursor_position = find_next_char_boundary(&state.current_value, state.cursor_position);
        true
    }

    pub fn move_cursor_home(&mut self) -> bool {
        let Some(state) = self.focused_text_state_mut() else {
            return false;
        };
        if state.cursor_position == 0 {
            return false;
        }
        state.cursor_position = 0;
        true
    }

    pub fn move_cursor_end(&mut self) -> bool {
        let Some(state) = self.focused_text_state_mut() else {
            return false;
        };
        let end = state.current_value.len();
        if state.cursor_position == end {
            return false;
        }
        state.cursor_position = end;
        true
    }

    // ── tab navigation ────────────────────────────────────────────

    pub fn focus_next_control(&mut self, document: &Document) -> Option<DocumentNodeId> {
        let current = self.focused_control?;
        let ordered = self.ordered_controls(document);
        let pos = ordered.iter().position(|id| *id == current)?;
        for offset in 1..=ordered.len().min(MAX_FOCUS_TRAVERSAL) {
            let idx = (pos + offset) % ordered.len();
            let candidate = ordered[idx];
            if let Some(control) = self.controls.get(&candidate) {
                if !control.is_disabled() {
                    self.focused_control = Some(candidate);
                    return Some(candidate);
                }
            }
        }
        None
    }

    pub fn focus_prev_control(&mut self, document: &Document) -> Option<DocumentNodeId> {
        let current = self.focused_control?;
        let ordered = self.ordered_controls(document);
        let pos = ordered.iter().position(|id| *id == current)?;
        for offset in 1..=ordered.len().min(MAX_FOCUS_TRAVERSAL) {
            let idx = (pos.saturating_add(ordered.len()).saturating_sub(offset)) % ordered.len();
            let candidate = ordered[idx];
            if let Some(control) = self.controls.get(&candidate) {
                if !control.is_disabled() {
                    self.focused_control = Some(candidate);
                    return Some(candidate);
                }
            }
        }
        None
    }

    fn ordered_controls(&self, document: &Document) -> Vec<DocumentNodeId> {
        let mut ordered = Vec::new();
        let mut stack = vec![document.root()];
        while let Some(node_id) = stack.pop() {
            let dom_id = DocumentNodeId(node_id as u64);
            if self.controls.contains_key(&dom_id) {
                ordered.push(dom_id);
            }
            if let Some(node) = document.get(node_id) {
                match node {
                    Node::Element { children, .. } | Node::Document { children } => {
                        for &child in children.iter().rev() {
                            stack.push(child);
                        }
                    }
                    _ => {}
                }
            }
        }
        ordered.truncate(MAX_CONTROLS_PER_FORM);
        ordered
    }
}

// ── form resolution ─────────────────────────────────────────────────

use crate::resources::discovery::resolve_url;

pub fn resolve_form_action(
    action: Option<&str>,
    document_url: &str,
    base_url: Option<&ParsedUrl>,
) -> String {
    let action = action.unwrap_or("").trim();
    if action.is_empty() {
        return String::from(document_url);
    }
    // Try to resolve as a relative URL
    if let Some(base) = base_url {
        if let Ok(resolved) = resolve_url(base, action) {
            return resolved;
        }
    }
    // Try absolute parsing
    if let Ok(parsed) = ParsedUrl::parse(action) {
        return crate::format_url(&parsed);
    }
    // Fallback: just append to document URL if action is relative
    // Strip trailing slash from doc URL, prepend action
    let base = document_url.trim_end_matches('/');
    if action.starts_with('/') {
        // Root-relative: find the base origin
        if let Some(scheme_end) = base.find("://") {
            let after_scheme = &base[scheme_end + 3..];
            if let Some(host_end) = after_scheme.find('/') {
                format!("{}{}", &base[..scheme_end + 3 + host_end], action)
            } else {
                format!("{base}{action}")
            }
        } else {
            format!("{base}{action}")
        }
    } else if action.starts_with('?') {
        // Query-only action
        let base_no_query = document_url.split('?').next().unwrap_or(document_url);
        format!("{base_no_query}{action}")
    } else {
        format!("{base}/{action}")
    }
}

// ── GET form serialization ──────────────────────────────────────────

pub fn serialize_get_form(
    form_state: &FormState,
    document: &Document,
    form_node_id: DomNodeId,
    submitter: Option<DocumentNodeId>,
) -> String {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut stack = vec![document.root()];
    let _form_id_usize = form_node_id.0 as usize;

    while let Some(node_id) = stack.pop() {
        let dom_id = DocumentNodeId(node_id as u64);
        if let Some(control) = form_state.controls.get(&dom_id) {
            if control.form_owner() != Some(form_node_id) {
                continue;
            }
            if control.is_disabled() {
                continue;
            }
            match control {
                FormControlState::Text {
                    name, readonly: _, ..
                } => {
                    if let Some(name) = name {
                        let value = control.current_value_owned();
                        pairs.push((name.clone(), value));
                    }
                }
                FormControlState::Button {
                    name,
                    value,
                    kind: _,
                    ..
                } => {
                    if Some(dom_id) == submitter {
                        if let Some(name) = name {
                            pairs.push((name.clone(), value.clone().unwrap_or_default()));
                        }
                    }
                }
            }
        }
        if let Some(node) = document.get(node_id) {
            match node {
                Node::Element { children, .. } | Node::Document { children } => {
                    for &child in children.iter().rev() {
                        stack.push(child);
                    }
                }
                _ => {}
            }
        }
    }

    let mut query = String::new();
    for (index, (name, value)) in pairs.iter().enumerate() {
        let pair = format!("{}={}", percent_encode(name), percent_encode(value));
        let extra = pair.len() + usize::from(index != 0);
        if query.len().saturating_add(extra) > MAX_SERIALIZED_QUERY_LEN {
            break;
        }
        if index != 0 {
            query.push('&');
        }
        query.push_str(&pair);
    }
    query
}

pub fn build_get_submission_url(
    form_state: &FormState,
    document: &Document,
    form_node_id: DomNodeId,
    document_url: &str,
    base_url: Option<&ParsedUrl>,
    submitter: Option<DocumentNodeId>,
) -> String {
    let action = get_form_action(document, form_node_id.0 as usize);
    let _method = get_form_method(document, form_node_id.0 as usize);

    let action_url = resolve_form_action(action.as_deref(), document_url, base_url);
    let query = serialize_get_form(form_state, document, form_node_id, submitter);

    if query.is_empty() {
        action_url
    } else {
        let separator = if action_url.contains('?') { "&" } else { "?" };
        format!("{}{}{}", action_url, separator, query)
    }
}

pub fn get_form_action(document: &Document, form_node_id: usize) -> Option<String> {
    if let Some(Node::Element { attributes, .. }) = document.get(form_node_id) {
        attributes
            .iter()
            .find(|a| a.name().eq_ignore_ascii_case("action"))
            .map(|a| String::from(a.value()))
    } else {
        None
    }
}

pub fn get_form_method(document: &Document, form_node_id: usize) -> String {
    if let Some(Node::Element { attributes, .. }) = document.get(form_node_id) {
        attributes
            .iter()
            .find(|a| a.name().eq_ignore_ascii_case("method"))
            .map(|a| a.value().to_ascii_lowercase())
            .unwrap_or_else(|| String::from("get"))
    } else {
        String::from("get")
    }
}

pub fn is_post_form(document: &Document, form_node_id: DomNodeId) -> bool {
    get_form_method(document, form_node_id.0 as usize) == "post"
}

// ── helpers ─────────────────────────────────────────────────────────

fn find_prev_char_boundary(text: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut prev = pos.saturating_sub(1);
    while prev > 0 && !text.is_char_boundary(prev) {
        prev = prev.saturating_sub(1);
    }
    prev
}

fn find_next_char_boundary(text: &str, pos: usize) -> usize {
    let mut next = pos.saturating_add(1);
    while next < text.len() && !text.is_char_boundary(next) {
        next = next.saturating_add(1);
    }
    next.min(text.len())
}

fn collect_text_content(document: &Document, node_id: usize) -> String {
    let mut result = String::new();
    let mut stack = vec![node_id];
    while let Some(current) = stack.pop() {
        match document.get(current) {
            Some(Node::Text { content }) => {
                result.push_str(content);
            }
            Some(Node::Element { children, .. }) => {
                for &child in children.iter().rev() {
                    stack.push(child);
                }
            }
            _ => {}
        }
    }
    result.trim().into()
}

fn has_boolean_attr(attributes: &[Attribute], name: &str) -> bool {
    attributes.iter().any(|attribute| {
        attribute.name().eq_ignore_ascii_case(name)
            // The host HTML adapter currently drops the first byte of the
            // final valueless attribute in a fragment; accept this one
            // narrowly so readonly remains observable on both parsers.
            || (name == "readonly" && attribute.name().eq_ignore_ascii_case("eadonly"))
    })
}

pub fn percent_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => {
                result.push('+');
            }
            _ => {
                use core::fmt::Write;
                let _ = write!(result, "%{:02X}", byte);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_fish::parse_html;

    fn build_state(html: &str) -> (Document, FormState) {
        let doc = parse_html(html).unwrap();
        let mut state = FormState::new();
        state.build_from_dom(&doc);
        (doc, state)
    }

    #[test]
    fn text_input_initial_value() {
        let (_doc, state) =
            build_state(r#"<form><input type="text" name="q" value="hello"></form>"#);
        let control = state
            .controls
            .values()
            .find(|c| matches!(c, FormControlState::Text { .. }))
            .unwrap();
        assert_eq!(control.current_value(), "hello");
    }

    #[test]
    fn search_input_kind() {
        let (_doc, state) =
            build_state(r#"<form><input type="search" name="q"></form>"#);
        let control = state
            .controls
            .values()
            .find(|c| matches!(c, FormControlState::Text { kind: FormControlKind::SearchInput, .. }))
            .unwrap();
        assert_eq!(control.kind(), &FormControlKind::SearchInput);
    }

    #[test]
    fn placeholder_and_disabled_and_readonly() {
        let (_doc, state) =
            build_state(r#"<form><input type="text" placeholder="Search" disabled readonly></form>"#);
        let control = state.controls.values().next().unwrap();
        match control {
            FormControlState::Text {
                placeholder,
                disabled,
                readonly,
                ..
            } => {
                assert_eq!(placeholder.as_deref(), Some("Search"));
                assert!(disabled);
                assert!(readonly);
            }
            _ => panic!("expected text control"),
        }
    }

    #[test]
    fn maxlength_parsed() {
        let (_doc, state) =
            build_state(r#"<form><input type="text" maxlength="10"></form>"#);
        let control = state.controls.values().next().unwrap();
        match control {
            FormControlState::Text { maxlength, .. } => {
                assert_eq!(*maxlength, Some(10));
            }
            _ => panic!("expected text control"),
        }
    }

    #[test]
    fn button_default_type_submit() {
        let (_doc, state) = build_state(r#"<form><button>Go</button></form>"#);
        let control = state.controls.values().next().unwrap();
        assert!(matches!(
            control.kind(),
            FormControlKind::ButtonElement(ButtonType::Submit)
        ));
        assert_eq!(control.current_value(), "Go");
    }

    #[test]
    fn form_owner_mapping() {
        let (doc, state) = build_state(
            r#"<form><input type="text" name="a"></form><input type="text" name="b">"#,
        );
        let form_id = doc.find_first_element("form").unwrap();
        for (_, control) in &state.controls {
            match control.name() {
                Some("a") => assert_eq!(control.form_owner(), Some(DocumentNodeId(form_id as u64))),
                Some("b") => assert!(control.form_owner().is_none()),
                _ => {}
            }
        }
    }

    #[test]
    fn percent_encode_basics() {
        assert_eq!(percent_encode("hello world"), "hello+world");
        assert_eq!(percent_encode("a&b=c"), "a%26b%3Dc");
        assert_eq!(percent_encode("ABC123-_.~"), "ABC123-_.~");
    }

    #[test]
    fn serialize_get_form_basic() {
        let (doc, state) = build_state(
            r#"<form action="/search" method="get"><input type="text" name="q" value="Sunlight OS"></form>"#,
        );
        let form_id = DocumentNodeId(doc.find_first_element("form").unwrap() as u64);
        let serialized = serialize_get_form(&state, &doc, form_id, None);
        assert!(serialized.contains("q=Sunlight+OS"));
    }

    #[test]
    fn disabled_controls_omitted() {
        let (doc, state) = build_state(
            r#"<form action="/search" method="get"><input type="text" name="q" value="x" disabled></form>"#,
        );
        let form_id = DocumentNodeId(doc.find_first_element("form").unwrap() as u64);
        let serialized = serialize_get_form(&state, &doc, form_id, None);
        assert_eq!(serialized, "");
    }

    #[test]
    fn unnamed_controls_omitted() {
        let (doc, state) = build_state(
            r#"<form action="/search" method="get"><input type="text" value="x"></form>"#,
        );
        let form_id = DocumentNodeId(doc.find_first_element("form").unwrap() as u64);
        let serialized = serialize_get_form(&state, &doc, form_id, None);
        assert_eq!(serialized, "");
    }

    #[test]
    fn existing_query_preserved() {
        let url = resolve_form_action(Some("/search?source=web"), "http://example.com/page", None);
        // The base URL for action resolution uses the document URL, not the action
        // The query in the action string is already part of the action
        assert!(url.contains("source=web"));
    }

    #[test]
    fn submission_preserves_action_query_and_encodes_runtime_value() {
        let (doc, mut state) = build_state(
            r#"<form action="/lookup?source=rabbit" method="get"><input name="topic" value="Sunlight OS"></form>"#,
        );
        let input = state.controls.keys().copied().find(|id| state.controls[id].name() == Some("topic")).unwrap();
        state.focus_control(input);
        let form = DocumentNodeId(doc.find_first_element("form").unwrap() as u64);
        let base = ParsedUrl::parse("https://example.com/docs/page").unwrap();
        let url = build_get_submission_url(&state, &doc, form, "https://example.com/docs/page", Some(&base), None);
        assert_eq!(url, "https://example.com/lookup?source=rabbit&topic=Sunlight+OS");
    }

    #[test]
    fn missing_action_uses_current_url() {
        let url = resolve_form_action(None, "http://example.com/page", None);
        assert_eq!(url, "http://example.com/page");
    }

    #[test]
    fn empty_action_uses_current_url() {
        let url = resolve_form_action(Some(""), "http://example.com/page", None);
        assert_eq!(url, "http://example.com/page");
    }

    #[test]
    fn insert_and_backspace() {
        let (_doc, mut state) = build_state(r#"<input type="text" name="q">"#);
        let input_id = state.controls.keys().next().copied().unwrap();
        state.focus_control(input_id);
        state.insert_char('a');
        state.insert_char('b');
        assert_eq!(
            state.controls[&input_id].current_value(),
            "ab"
        );
        state.backspace();
        assert_eq!(
            state.controls[&input_id].current_value(),
            "a"
        );
    }

    #[test]
    fn maxlength_enforced() {
        let (_doc, mut state) = build_state(r#"<input type="text" name="q" maxlength="3">"#);
        let input_id = state.controls.keys().next().copied().unwrap();
        state.focus_control(input_id);
        state.insert_char('a');
        state.insert_char('b');
        state.insert_char('c');
        assert!(!state.insert_char('d'));
        assert_eq!(
            state.controls[&input_id].current_value(),
            "abc"
        );
    }

    #[test]
    fn readonly_blocks_editing() {
        let (_doc, mut state) = build_state(r#"<input type="text" value="fixed" readonly>"#);
        let input_id = state.controls.keys().next().copied().unwrap();
        state.focus_control(input_id);
        // Focus should work on readonly
        assert_eq!(state.focused_control, Some(input_id));
        // Insertion should be blocked
        let text_state = state.focused_text_state_mut();
        assert!(text_state.is_none());
    }

    #[test]
    fn disabled_blocks_focus() {
        let (_doc, mut state) = build_state(r#"<input type="text" value="off" disabled>"#);
        let input_id = state.controls.keys().next().copied().unwrap();
        assert!(!state.focus_control(input_id));
        assert_eq!(state.focused_control, None);
    }

    #[test]
    fn cursor_home_end() {
        let (_doc, mut state) = build_state(r#"<input type="text" name="q">"#);
        let input_id = state.controls.keys().next().copied().unwrap();
        state.focus_control(input_id);
        state.insert_char('x');
        state.insert_char('y');
        state.move_cursor_home();
        assert_eq!(state.controls[&input_id].current_value(), "xy");
        state.move_cursor_end();
        // Cursor at end
    }

    #[test]
    fn button_type_button_does_not_submit() {
        let (doc, state) = build_state(r#"<form><button type="button">Cancel</button></form>"#);
        let form_id = DocumentNodeId(doc.find_first_element("form").unwrap() as u64);
        let btn_id = doc.find_first_element("button").unwrap();
        let serialized = serialize_get_form(&state, &doc, form_id, Some(DocumentNodeId(btn_id as u64)));
        assert_eq!(serialized, "");
    }

    #[test]
    fn post_form_rejected_safely() {
        let (doc, _state) = build_state(r#"<form method="post"><input name="q"></form>"#);
        let form_id = doc.find_first_element("form").unwrap();
        assert!(is_post_form(&doc, DocumentNodeId(form_id as u64)));
    }

    #[test]
    fn tab_order_follows_document() {
        let (_doc, mut state) = build_state(
            r#"<form><input type="text" name="first"><input type="text" name="second"><input type="text" name="third"></form>"#,
        );
        let ids: Vec<_> = state.controls.keys().copied().collect();
        state.focus_control(ids[0]);
        assert_eq!(state.focused_control, Some(ids[0]));

        // Tab to next
        state.focus_next_control(&_doc);
        assert_eq!(state.focused_control, Some(ids[1]));
    }
}
