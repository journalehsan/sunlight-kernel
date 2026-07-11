//! A deliberately small, defensive CSS pipeline for the DOM inspector.
//!
//! This module intentionally models only the first useful slice of CSS.  It
//! does not know about painting or layout; it turns stylesheet input and a DOM
//! into per-element computed values that other subsystems can query.

use alloc::{format, string::String, vec, vec::Vec};

pub const MAX_STYLESHEET_BYTES: usize = 128 * 1024;
pub const MAX_RULES: usize = 1_024;
pub const MAX_SELECTOR_LENGTH: usize = 256;
pub const MAX_DECLARATIONS_PER_RULE: usize = 128;
pub const MAX_DESCENDANT_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stylesheet {
    pub source: StylesheetSource,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StylesheetSource {
    UserAgent,
    Embedded,
    External(String),
}

impl StylesheetSource {
    pub fn label(&self) -> String {
        match self {
            Self::UserAgent => String::from("user-agent"),
            Self::Embedded => String::from("style element"),
            Self::External(url) => format!("stylesheet {url}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    /// Approximate location of the rule start in the stylesheet source (if tracked).
    pub location: Option<SourceLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// Parts are ordered from the outermost ancestor to the target element.
    pub parts: Vec<SimpleSelector>,
    /// The selector text as used for matching (expanded form for nested rules).
    pub text: String,
    /// The original source form when this selector came from CSS nesting (e.g. "& ul").
    /// None for top-level selectors.
    pub original_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SimpleSelector {
    pub tag_name: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub universal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub property: Property,
    pub value: PropertyValue,
    pub raw_value: String,
    pub order: usize,
    pub important: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Property {
    Display,
    FlexDirection,
    FlexWrap,
    JustifyContent,
    AlignItems,
    AlignContent,
    Gap,
    RowGap,
    ColumnGap,
    Color,
    BackgroundColor,
    BackgroundImage,
    BackgroundRepeat,
    BackgroundAttachment,
    BackgroundPositionX,
    BackgroundPositionY,
    FontSize,
    FontFamily,
    FontWeight,
    FontStyle,
    TextAlign,
    TextDecoration,
    WhiteSpace,
    ListStyleType,
    ListStyle,
    ListStylePosition,
    LineHeight,
    Width,
    Height,
    MinHeight,
    BoxSizing,
    LetterSpacing,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    Border,
    BorderWidth,
    BorderStyle,
    BorderColor,
    BorderBottom,
    Custom(String),
    Unknown(String),
}

impl Property {
    pub fn parse(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "display" => Self::Display,
            "flex-direction" => Self::FlexDirection,
            "flex-wrap" => Self::FlexWrap,
            "justify-content" => Self::JustifyContent,
            "align-items" => Self::AlignItems,
            "align-content" => Self::AlignContent,
            "gap" => Self::Gap,
            "row-gap" => Self::RowGap,
            "column-gap" => Self::ColumnGap,
            "color" => Self::Color,
            "background-color" => Self::BackgroundColor,
            "background-image" => Self::BackgroundImage,
            "background-repeat" => Self::BackgroundRepeat,
            "background-attachment" => Self::BackgroundAttachment,
            "background-position-x" => Self::BackgroundPositionX,
            "background-position-y" => Self::BackgroundPositionY,
            "font-size" => Self::FontSize,
            "font-family" => Self::FontFamily,
            "font-weight" => Self::FontWeight,
            "font-style" => Self::FontStyle,
            "text-align" => Self::TextAlign,
            "text-decoration" => Self::TextDecoration,
            "white-space" => Self::WhiteSpace,
            "list-style-type" => Self::ListStyleType,
            "list-style" => Self::ListStyle,
            "list-style-position" => Self::ListStylePosition,
            "line-height" => Self::LineHeight,
            "width" => Self::Width,
            "height" => Self::Height,
            "min-height" => Self::MinHeight,
            "box-sizing" => Self::BoxSizing,
            "letter-spacing" => Self::LetterSpacing,
            "margin-top" => Self::MarginTop,
            "margin-right" => Self::MarginRight,
            "margin-bottom" => Self::MarginBottom,
            "margin-left" => Self::MarginLeft,
            "padding-top" => Self::PaddingTop,
            "padding-right" => Self::PaddingRight,
            "padding-bottom" => Self::PaddingBottom,
            "padding-left" => Self::PaddingLeft,
            "border" => Self::Border,
            "border-width" => Self::BorderWidth,
            "border-style" => Self::BorderStyle,
            "border-color" => Self::BorderColor,
            "border-bottom" => Self::BorderBottom,
            other if other.starts_with("--") => Self::Custom(String::from(other)),
            other => Self::Unknown(String::from(other)),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Display => "display",
            Self::FlexDirection => "flex-direction",
            Self::FlexWrap => "flex-wrap",
            Self::JustifyContent => "justify-content",
            Self::AlignItems => "align-items",
            Self::AlignContent => "align-content",
            Self::Gap => "gap",
            Self::RowGap => "row-gap",
            Self::ColumnGap => "column-gap",
            Self::Color => "color",
            Self::BackgroundColor => "background-color",
            Self::BackgroundImage => "background-image",
            Self::BackgroundRepeat => "background-repeat",
            Self::BackgroundAttachment => "background-attachment",
            Self::BackgroundPositionX => "background-position-x",
            Self::BackgroundPositionY => "background-position-y",
            Self::FontSize => "font-size",
            Self::FontFamily => "font-family",
            Self::FontWeight => "font-weight",
            Self::FontStyle => "font-style",
            Self::TextAlign => "text-align",
            Self::TextDecoration => "text-decoration",
            Self::WhiteSpace => "white-space",
            Self::ListStyleType => "list-style-type",
            Self::ListStyle => "list-style",
            Self::ListStylePosition => "list-style-position",
            Self::LineHeight => "line-height",
            Self::Width => "width",
            Self::Height => "height",
            Self::MinHeight => "min-height",
            Self::BoxSizing => "box-sizing",
            Self::LetterSpacing => "letter-spacing",
            Self::MarginTop => "margin-top",
            Self::MarginRight => "margin-right",
            Self::MarginBottom => "margin-bottom",
            Self::MarginLeft => "margin-left",
            Self::PaddingTop => "padding-top",
            Self::PaddingRight => "padding-right",
            Self::PaddingBottom => "padding-bottom",
            Self::PaddingLeft => "padding-left",
            Self::Border => "border",
            Self::BorderWidth => "border-width",
            Self::BorderStyle => "border-style",
            Self::BorderColor => "border-color",
            Self::BorderBottom => "border-bottom",
            Self::Custom(name) => name,
            Self::Unknown(name) => name,
        }
    }

    fn is_inherited(&self) -> bool {
        matches!(
            self,
            Self::Color
                | Self::FontSize
                | Self::FontFamily
                | Self::FontWeight
                | Self::FontStyle
                | Self::TextAlign
                | Self::TextDecoration
                | Self::WhiteSpace
                | Self::ListStyleType
                | Self::ListStylePosition
                | Self::LetterSpacing
        )
    }

    /// Exposed for the inspector's text-node inherited-style projection.
    pub fn is_inherited_for_inspector(&self) -> bool {
        self.is_inherited()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyValue {
    Color(Color),
    LengthPx(i32),
    Auto,
    Normal,
    Keyword(String),
    Raw(String),
}

impl PropertyValue {
    pub fn display(&self) -> String {
        match self {
            Self::Color(color) => color.display(),
            Self::LengthPx(value) => format!("{value}px"),
            Self::Auto => String::from("auto"),
            Self::Normal => String::from("normal"),
            Self::Keyword(value) | Self::Raw(value) => value.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Rgb(u8, u8, u8),
    Transparent,
}

impl Color {
    pub fn display(self) -> String {
        match self {
            Self::Rgb(red, green, blue) => format!("#{red:02x}{green:02x}{blue:02x}"),
            Self::Transparent => String::from("transparent"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Specificity {
    pub ids: u16,
    pub classes: u16,
    pub tags: u16,
}

/// Lightweight source location for DevTools. Lines and columns are 1-based.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedDeclaration {
    pub property: Property,
    pub value: PropertyValue,
    pub selector: String,
    /// For nested rules, the original written form (e.g. "& ul"). Same as selector otherwise.
    pub original_selector: Option<String>,
    pub source: String,
    pub specificity: Specificity,
    pub inherited: bool,
    pub important: bool,
    /// Source order within the element's cascade (for explaining "earlier rule lost").
    pub source_order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputedProperty {
    pub property: Property,
    pub value: PropertyValue,
    pub matched: Option<MatchedDeclaration>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputedStyle {
    pub properties: Vec<ComputedProperty>,
    pub custom_properties: Vec<(String, PropertyValue, Option<MatchedDeclaration>)>,
    /// All declarations (from rules whose selector matched this element) that
    /// participated in the cascade for this element, winners and overridden.
    /// This is the primary data for the DevTools Rules view. The engine
    /// decides winners; DevTools only presents them.
    pub matched_declarations: Vec<MatchedDeclaration>,
}

impl ComputedStyle {
    pub fn value(&self, property: &Property) -> Option<&PropertyValue> {
        self.properties
            .iter()
            .find(|entry| entry.property == *property)
            .map(|entry| &entry.value)
    }
}

#[cfg(feature = "dom")]
const PROPERTY_ORDER: &[Property] = &[
    Property::Display,
    Property::FlexDirection,
    Property::FlexWrap,
    Property::JustifyContent,
    Property::AlignItems,
    Property::AlignContent,
    Property::Gap,
    Property::RowGap,
    Property::ColumnGap,
    Property::Color,
    Property::BackgroundColor,
    Property::BackgroundImage,
    Property::BackgroundRepeat,
    Property::BackgroundAttachment,
    Property::BackgroundPositionX,
    Property::BackgroundPositionY,
    Property::FontSize,
    Property::FontFamily,
    Property::FontWeight,
    Property::FontStyle,
    Property::TextAlign,
    Property::TextDecoration,
    Property::WhiteSpace,
    Property::ListStyleType,
    Property::ListStyle,
    Property::ListStylePosition,
    Property::LineHeight,
    Property::Width,
    Property::Height,
    Property::MinHeight,
    Property::BoxSizing,
    Property::LetterSpacing,
    Property::MarginTop,
    Property::MarginRight,
    Property::MarginBottom,
    Property::MarginLeft,
    Property::PaddingTop,
    Property::PaddingRight,
    Property::PaddingBottom,
    Property::PaddingLeft,
    Property::BorderWidth,
    Property::BorderStyle,
    Property::BorderColor,
    Property::BorderBottom,
];

/// Parses an ordinary stylesheet without panicking on malformed website CSS.
pub fn parse_stylesheet(css: &str, source: StylesheetSource) -> Stylesheet {
    let css = bounded_css(css);
    let clean = strip_comments(css);
    let mut rules = Vec::new();
    parse_rule_block(&clean, None, &mut rules, &css);
    Stylesheet { source, rules }
}

fn parse_rule_block(
    input: &str,
    parent: Option<&str>,
    rules: &mut Vec<Rule>,
    original_for_lines: &str,
) {
    let mut cursor = 0usize;
    while cursor < input.len() && rules.len() < MAX_RULES {
        let Some(open_rel) = input[cursor..].find('{') else {
            break;
        };
        let open = cursor + open_rel;
        let selector_text_raw = input[cursor..open].trim();
        let loc = location_from_offset(original_for_lines, cursor);
        let Some(close) = matching_brace(input, open) else {
            break;
        };
        let selector_text = if let Some(parent) = parent {
            selector_text_raw.replace('&', parent)
        } else {
            String::from(selector_text_raw)
        };
        let body = &input[open + 1..close];
        let (declaration_text, nested) = split_nested_rules(body);
        if !selector_text.is_empty() && !selector_text.starts_with('@') {
            let selectors: Vec<Selector> = selector_text
                .split(',')
                .filter_map(|s| {
                    parse_selector_with_original(
                        s,
                        if parent.is_some() {
                            Some(String::from(selector_text_raw))
                        } else {
                            None
                        },
                    )
                })
                .collect();
            let declarations = parse_declarations(&declaration_text);
            if !selectors.is_empty() && !declarations.is_empty() {
                rules.push(Rule {
                    selectors,
                    declarations,
                    location: Some(loc),
                });
            }
            for (nested_selector, nested_body) in nested {
                let original_nested = nested_selector.clone();
                let combined = if nested_selector.contains('&') {
                    nested_selector.replace('&', &selector_text)
                } else {
                    format!("{selector_text} {nested_selector}")
                };
                // Parse the inner declarations directly and construct rule so we can attach original_text.
                let inner_decl_text = &nested_body; // declarations part; nested inside nested rare, ignore deeper for location
                let (inner_decls_text, _deeper_nested) = split_nested_rules(inner_decl_text);
                let inner_decls = parse_declarations(&inner_decls_text);
                if !inner_decls.is_empty() {
                    if let Some(sel) =
                        parse_selector_with_original(&combined, Some(original_nested))
                    {
                        let inner_selectors = vec![sel];
                        if !inner_decls.is_empty() {
                            rules.push(Rule {
                                selectors: inner_selectors,
                                declarations: inner_decls,
                                location: Some(loc), // approximate: location of parent rule
                            });
                        }
                    }
                }
                // Also support deeper nesting by falling back to string form (originals may be lost for depth>1)
                if !_deeper_nested.is_empty() {
                    parse_rule_block(
                        &format!("{combined}{{{nested_body}}}"),
                        None,
                        rules,
                        original_for_lines,
                    );
                }
            }
        }
        cursor = close + 1;
    }
}

fn matching_brace(input: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in input.as_bytes().iter().enumerate().skip(open) {
        if *byte == b'{' {
            depth = depth.saturating_add(1);
        }
        if *byte == b'}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(offset);
            }
        }
        if depth > 8 {
            return None;
        }
    }
    None
}

fn split_nested_rules(body: &str) -> (String, Vec<(String, String)>) {
    let mut declarations = String::new();
    let mut nested = Vec::new();
    let mut cursor = 0usize;
    while cursor < body.len() {
        let Some(open_rel) = body[cursor..].find('{') else {
            declarations.push_str(&body[cursor..]);
            break;
        };
        let open = cursor + open_rel;
        let Some(close) = matching_brace(body, open) else {
            break;
        };
        let prefix = body[cursor..open].trim();
        if let Some((before, selector)) = prefix.rsplit_once(';') {
            declarations.push_str(before);
            declarations.push(';');
            nested.push((
                String::from(selector.trim()),
                String::from(&body[open + 1..close]),
            ));
        } else if prefix.starts_with('&') || prefix.contains(':') || !prefix.contains(':') {
            nested.push((String::from(prefix), String::from(&body[open + 1..close])));
        }
        cursor = close + 1;
    }
    (declarations, nested)
}

fn bounded_css(css: &str) -> &str {
    if css.len() <= MAX_STYLESHEET_BYTES {
        return css;
    }
    let mut end = MAX_STYLESHEET_BYTES;
    while end > 0 && !css.is_char_boundary(end) {
        end -= 1;
    }
    &css[..end]
}

pub fn parse_inline_style(css: &str) -> Vec<Declaration> {
    parse_declarations(css)
}

fn strip_comments(css: &str) -> String {
    let mut result = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        result.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("*/") else {
            rest = "";
            break;
        };
        rest = &after[end + 2..];
    }
    result.push_str(rest);
    result
}

fn parse_selector(input: &str) -> Option<Selector> {
    parse_selector_with_original(input, None)
}

fn parse_selector_with_original(input: &str, original: Option<String>) -> Option<Selector> {
    let text = input.trim();
    if text.is_empty() || text.len() > MAX_SELECTOR_LENGTH {
        return None;
    }
    let mut parts = Vec::new();
    for raw_part in text.split_ascii_whitespace() {
        let simple = parse_simple_selector(raw_part)?;
        parts.push(simple);
        if parts.len() > MAX_DESCENDANT_DEPTH {
            return None;
        }
    }
    (!parts.is_empty()).then(|| Selector {
        parts,
        text: String::from(text),
        original_text: original,
    })
}

fn location_from_offset(source: &str, byte_offset: usize) -> SourceLocation {
    if byte_offset > source.len() {
        return SourceLocation { line: 1, column: 1 };
    }
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, b) in source.as_bytes().iter().enumerate().take(byte_offset) {
        if *b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        // crude but sufficient; tabs count as 1 for inspector purposes
    }
    SourceLocation { line, column: col }
}

fn parse_simple_selector(input: &str) -> Option<SimpleSelector> {
    let mut out = SimpleSelector::default();
    let bytes = input.as_bytes();
    let mut index = 0usize;
    if bytes.get(0) == Some(&b'*') {
        out.universal = true;
        index = 1;
    } else if bytes.get(0).is_some_and(|byte| byte.is_ascii_alphabetic()) {
        let start = index;
        while bytes
            .get(index)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_')
        {
            index += 1;
        }
        out.tag_name = Some(input[start..index].to_ascii_lowercase());
    }
    while index < input.len() {
        let marker = *bytes.get(index)?;
        if marker != b'.' && marker != b'#' {
            return None;
        }
        index += 1;
        let start = index;
        while bytes
            .get(index)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_')
        {
            index += 1;
        }
        if start == index {
            return None;
        }
        let name = String::from(&input[start..index]);
        if marker == b'#' {
            if out.id.is_some() {
                return None;
            }
            out.id = Some(name);
        } else {
            out.classes.push(name);
        }
    }
    (out.universal || out.tag_name.is_some() || out.id.is_some() || !out.classes.is_empty())
        .then_some(out)
}

fn parse_declarations(body: &str) -> Vec<Declaration> {
    let mut out = Vec::new();
    let mut order = 0usize;
    for segment in body.split(';') {
        if out.len() >= MAX_DECLARATIONS_PER_RULE {
            break;
        }
        let Some((name, raw_value)) = segment.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let (raw_value, important) = strip_important(raw_value.trim());
        if name == "background" {
            for declaration in expand_background(raw_value, order, important) {
                if out.len() < MAX_DECLARATIONS_PER_RULE {
                    out.push(declaration);
                }
            }
            order = order.saturating_add(1);
            continue;
        }
        if name == "margin" || name == "padding" {
            if raw_value.is_empty() {
                continue;
            }
            let declarations = expand_box_shorthand(&name, raw_value, order, important);
            order = order.saturating_add(declarations.len().max(1));
            for declaration in declarations {
                if out.len() < MAX_DECLARATIONS_PER_RULE {
                    out.push(declaration);
                }
            }
            continue;
        }
        let property = Property::parse(&name);
        if matches!(property, Property::Unknown(_)) {
            continue;
        }
        if raw_value.is_empty() {
            continue;
        }
        let declarations = expand_declaration(property, raw_value, order, important);
        order = order.saturating_add(declarations.len().max(1));
        for declaration in declarations {
            if out.len() < MAX_DECLARATIONS_PER_RULE {
                out.push(declaration);
            }
        }
    }
    out
}

fn expand_background(raw: &str, order: usize, important: bool) -> Vec<Declaration> {
    let mut out = Vec::new();
    let tokens = raw.split_ascii_whitespace().collect::<Vec<_>>();
    for token in tokens {
        let property = if parse_color(token).is_some() || token.starts_with("var(") {
            Property::BackgroundColor
        } else if token.eq_ignore_ascii_case("none") || token.starts_with("url(") {
            Property::BackgroundImage
        } else if matches!(
            token.to_ascii_lowercase().as_str(),
            "repeat" | "no-repeat" | "repeat-x" | "repeat-y"
        ) {
            Property::BackgroundRepeat
        } else if token.eq_ignore_ascii_case("scroll") {
            Property::BackgroundAttachment
        } else {
            continue;
        };
        out.push(Declaration {
            property: property.clone(),
            value: parse_value(&property, token),
            raw_value: String::from(token),
            order,
            important,
        });
    }
    out
}

fn strip_important(value: &str) -> (&str, bool) {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with("!important") {
        (
            &trimmed[..trimmed.len().saturating_sub("!important".len())].trim_end(),
            true,
        )
    } else {
        (trimmed, false)
    }
}

fn expand_box_shorthand(name: &str, raw: &str, order: usize, important: bool) -> Vec<Declaration> {
    let values = raw.split_ascii_whitespace().collect::<Vec<_>>();
    if values.is_empty() || values.len() > 4 {
        return Vec::new();
    }
    let (top, right, bottom, left) = match values.len() {
        1 => (values[0], values[0], values[0], values[0]),
        2 => (values[0], values[1], values[0], values[1]),
        3 => (values[0], values[1], values[2], values[1]),
        4 => (values[0], values[1], values[2], values[3]),
        _ => return Vec::new(),
    };
    let properties = if name == "margin" {
        [
            Property::MarginTop,
            Property::MarginRight,
            Property::MarginBottom,
            Property::MarginLeft,
        ]
    } else {
        [
            Property::PaddingTop,
            Property::PaddingRight,
            Property::PaddingBottom,
            Property::PaddingLeft,
        ]
    };
    [top, right, bottom, left]
        .into_iter()
        .zip(properties)
        .map(|(value, property)| Declaration {
            value: parse_value(&property, value),
            property,
            raw_value: String::from(value),
            order,
            important,
        })
        .collect()
}

fn expand_declaration(
    property: Property,
    raw: &str,
    order: usize,
    important: bool,
) -> Vec<Declaration> {
    let make = |property: Property, value: &str, order| Declaration {
        property: property.clone(),
        value: parse_value(&property, value),
        raw_value: String::from(value.trim()),
        order,
        important,
    };
    match property {
        Property::Unknown(_) => Vec::new(),
        Property::Gap => {
            let values = raw.split_ascii_whitespace().collect::<Vec<_>>();
            if values.len() == 1 || values.len() == 2 {
                let row = values[0];
                let column = values.get(1).copied().unwrap_or(row);
                vec![
                    make(Property::Gap, raw, order),
                    make(Property::RowGap, row, order),
                    make(Property::ColumnGap, column, order),
                ]
            } else {
                Vec::new()
            }
        }
        Property::ListStyle => {
            let keyword = raw
                .split_ascii_whitespace()
                .find(|part| {
                    matches!(
                        part.to_ascii_lowercase().as_str(),
                        "none" | "disc" | "circle" | "square" | "decimal"
                    )
                })
                .unwrap_or(raw.trim());
            vec![
                make(Property::ListStyle, raw, order),
                make(Property::ListStyleType, keyword, order),
            ]
        }
        Property::Border => {
            let mut output = vec![make(Property::Border, raw, order)];
            for part in raw.split_ascii_whitespace() {
                if let Some(color) = parse_color(part) {
                    output.push(Declaration {
                        property: Property::BorderColor,
                        value: PropertyValue::Color(color),
                        raw_value: String::from(part),
                        order,
                        important,
                    });
                } else if let Some(length) = parse_length(part) {
                    output.push(Declaration {
                        property: Property::BorderWidth,
                        value: length,
                        raw_value: String::from(part),
                        order,
                        important,
                    });
                } else if is_border_style(part) {
                    output.push(Declaration {
                        property: Property::BorderStyle,
                        value: PropertyValue::Keyword(part.to_ascii_lowercase()),
                        raw_value: String::from(part),
                        order,
                        important,
                    });
                }
            }
            output
        }
        Property::BorderBottom => {
            let mut output = vec![make(Property::BorderBottom, raw, order)];
            for part in raw.split_ascii_whitespace() {
                if parse_color(part).is_some() || part.starts_with("var(") {
                    output.push(make(Property::BorderColor, part, order));
                } else if parse_length(part).is_some() {
                    output.push(make(Property::BorderWidth, part, order));
                } else if is_border_style(part) {
                    output.push(make(Property::BorderStyle, part, order));
                }
            }
            output
        }
        Property::BackgroundColor
        | Property::BackgroundImage
        | Property::BackgroundRepeat
        | Property::BackgroundAttachment
        | Property::BackgroundPositionX
        | Property::BackgroundPositionY
        | Property::BoxSizing
        | Property::LetterSpacing
        | Property::MinHeight
        | Property::Custom(_) => vec![make(property, raw, order)],
        _ => vec![make(property, raw, order)],
    }
}

fn parse_value(property: &Property, raw: &str) -> PropertyValue {
    let value = raw.trim();
    if matches!(
        property,
        Property::Color | Property::BackgroundColor | Property::BorderColor
    ) {
        return parse_color(value).map_or_else(
            || PropertyValue::Raw(String::from(value)),
            PropertyValue::Color,
        );
    }
    if matches!(
        property,
        Property::MarginTop
            | Property::MarginRight
            | Property::MarginBottom
            | Property::MarginLeft
            | Property::PaddingTop
            | Property::PaddingRight
            | Property::PaddingBottom
            | Property::PaddingLeft
            | Property::Width
            | Property::Height
            | Property::FontSize
            | Property::LineHeight
            | Property::BorderWidth
            | Property::RowGap
            | Property::ColumnGap
            | Property::Gap
    ) {
        return parse_length(value).unwrap_or_else(|| {
            if value.eq_ignore_ascii_case("auto") {
                PropertyValue::Auto
            } else if value.eq_ignore_ascii_case("normal") {
                PropertyValue::Normal
            } else {
                PropertyValue::Raw(String::from(value))
            }
        });
    }
    if value.eq_ignore_ascii_case("normal") {
        PropertyValue::Normal
    } else {
        PropertyValue::Keyword(value.to_ascii_lowercase())
    }
}

fn parse_length(value: &str) -> Option<PropertyValue> {
    if value == "0" || value == "+0" || value == "-0" {
        return Some(PropertyValue::LengthPx(0));
    }
    let normalized = value.to_ascii_lowercase();
    let number = normalized.strip_suffix("px")?.trim();
    number.parse::<i32>().ok().map(PropertyValue::LengthPx)
}

fn parse_color(value: &str) -> Option<Color> {
    let lower = value.trim().to_ascii_lowercase();
    let named = match lower.as_str() {
        "transparent" => return Some(Color::Transparent),
        "black" => Some((0, 0, 0)),
        "white" => Some((255, 255, 255)),
        "red" => Some((255, 0, 0)),
        "green" => Some((0, 128, 0)),
        "blue" => Some((0, 0, 255)),
        "yellow" => Some((255, 255, 0)),
        "gray" | "grey" => Some((128, 128, 128)),
        "purple" => Some((128, 0, 128)),
        "orange" => Some((255, 165, 0)),
        _ => None,
    };
    if let Some((r, g, b)) = named {
        return Some(Color::Rgb(r, g, b));
    }
    if let Some(hex) = lower.strip_prefix('#') {
        if hex.len() == 3 {
            let bytes = hex.as_bytes();
            return Some(Color::Rgb(
                hex_digit(bytes[0])? * 17,
                hex_digit(bytes[1])? * 17,
                hex_digit(bytes[2])? * 17,
            ));
        }
        if hex.len() == 6 {
            return Some(Color::Rgb(
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            ));
        }
    }
    let values = lower
        .strip_prefix("rgb(")?
        .strip_suffix(')')?
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    if values.len() != 3 {
        return None;
    }
    Some(Color::Rgb(
        values[0].parse().ok()?,
        values[1].parse().ok()?,
        values[2].parse().ok()?,
    ))
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
fn is_border_style(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "none" | "solid" | "dashed" | "dotted" | "double"
    )
}

/// Small, inspectable defaults; these are normal rules and therefore use the
/// same cascade machinery as website CSS.
pub fn user_agent_stylesheet() -> Stylesheet {
    parse_stylesheet(
        r#"
        html, body, div, header, main, section, article, nav, footer, p, pre, dl, dt, dd, blockquote, hr { display: block; }
        span, a, strong, b, em, i, code, small, mark, del, ins, sub, sup, br { display: inline; }
        img { display: inline-block; }
        input, button { display: inline-block; font-family: sans-serif; font-size: 14px; color: #111111; background-color: white; border: 1px solid #888888; padding: 5px 7px; }
        button { background-color: #e9e9e9; color: #111111; }
        ul, ol { display: block; margin: 16px 0; padding-left: 40px; }
        ul { list-style-type: disc; }
        ol { list-style-type: decimal; }
        li { display: list-item; }
        dl { margin: 16px 0; }
        dt { display: block; font-weight: bold; }
        dd { display: block; margin-left: 40px; }
        blockquote { margin: 16px 40px; }
        pre { white-space: pre; font-family: monospace; }
        code { font-family: monospace; }
        script, style, head, meta, link, title { display: none; }
        h1 { display: block; font-weight: bold; font-size: 32px; }
        h2 { display: block; font-weight: bold; font-size: 28px; }
        h3 { display: block; font-weight: bold; font-size: 24px; }
        h4 { display: block; font-weight: bold; font-size: 20px; }
        h5 { display: block; font-weight: bold; font-size: 18px; }
        h6 { display: block; font-weight: bold; font-size: 16px; }
        strong, b { font-weight: bold; }
        em, i { font-style: italic; }
        small { font-size: 12px; }
        mark { background-color: yellow; color: black; }
        del { text-decoration: line-through; }
        ins { text-decoration: underline; }
        sub, sup { font-size: 12px; }
        hr { border-top: 1px solid gray; margin: 8px 0; }
        a { text-decoration: underline; color: blue; }
        body { color: #222222; background-color: white; font-size: 16px; margin: 8px; font-family: sans-serif; }
    "#,
        StylesheetSource::UserAgent,
    )
}

#[cfg(feature = "dom")]
use golden_fish::{Document, Node, NodeId};

#[cfg(feature = "dom")]
#[derive(Debug, Clone, Default)]
pub struct StyleContext {
    styles: Vec<Option<ComputedStyle>>,
}

#[cfg(feature = "dom")]
impl StyleContext {
    pub fn build(document: &Document, document_stylesheets: &[Stylesheet]) -> Self {
        let mut stylesheets = vec![user_agent_stylesheet()];
        stylesheets.extend_from_slice(document_stylesheets);
        let mut context = Self {
            styles: vec![None; document.node_count()],
        };
        let mut stack = vec![document.root()];
        while let Some(node_id) = stack.pop() {
            if let Some(Node::Element { children, .. }) = document.get(node_id) {
                let parent_style = document
                    .parent(node_id)
                    .and_then(|parent| context.style_for(parent))
                    .cloned();
                let computed =
                    compute_element_style(document, node_id, parent_style.as_ref(), &stylesheets);
                context.styles[node_id] = Some(computed);
                for &child in children.iter().rev() {
                    stack.push(child);
                }
            } else {
                for &child in document.children(node_id).iter().rev() {
                    stack.push(child);
                }
            }
        }
        context
    }

    pub fn style_for(&self, node_id: NodeId) -> Option<&ComputedStyle> {
        self.styles.get(node_id).and_then(Option::as_ref)
    }

    pub fn nearest_element_style<'a>(
        &'a self,
        document: &Document,
        mut node_id: NodeId,
    ) -> Option<&'a ComputedStyle> {
        loop {
            if let Some(style) = self.style_for(node_id) {
                return Some(style);
            }
            node_id = document.parent(node_id)?;
        }
    }
}

#[cfg(feature = "dom")]
pub fn collect_embedded_stylesheets(document: &Document) -> Vec<Stylesheet> {
    let mut output = Vec::new();
    let mut stack = vec![document.root()];
    while let Some(node_id) = stack.pop() {
        let Some(node) = document.get(node_id) else {
            continue;
        };
        if let Node::Element {
            tag_name, children, ..
        } = node
        {
            if tag_name.eq_ignore_ascii_case("style") {
                let mut css = String::new();
                for &child in children {
                    if let Some(Node::Text { content }) = document.get(child) {
                        css.push_str(content);
                    }
                }
                output.push(parse_stylesheet(&css, StylesheetSource::Embedded));
            }
            for &child in children.iter().rev() {
                stack.push(child);
            }
        } else {
            for &child in document.children(node_id).iter().rev() {
                stack.push(child);
            }
        }
    }
    output
}

/// Reassembles successfully loaded style sources into document order.  Failed
/// links remain represented by `None`, so a failure cannot shift later linked
/// sheets ahead of an intervening `<style>` element.
#[cfg(feature = "dom")]
pub fn order_document_stylesheets(
    document: &Document,
    mut embedded: Vec<Stylesheet>,
    mut linked: Vec<Option<Stylesheet>>,
) -> Vec<Stylesheet> {
    let mut ordered = Vec::new();
    let mut embedded_index = 0usize;
    let mut linked_index = 0usize;
    let mut stack = vec![document.root()];
    while let Some(node_id) = stack.pop() {
        let Some(node) = document.get(node_id) else {
            continue;
        };
        if let Node::Element {
            tag_name,
            attributes,
            children,
        } = node
        {
            if tag_name.eq_ignore_ascii_case("style") {
                if let Some(sheet) = embedded.get_mut(embedded_index) {
                    ordered.push(sheet.clone());
                }
                embedded_index = embedded_index.saturating_add(1);
            } else if tag_name.eq_ignore_ascii_case("link")
                && attribute_value(attributes, "href").is_some()
                && attribute_value(attributes, "rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("stylesheet"))
                })
            {
                if let Some(Some(sheet)) = linked.get_mut(linked_index) {
                    ordered.push(sheet.clone());
                }
                linked_index = linked_index.saturating_add(1);
            }
            for &child in children.iter().rev() {
                stack.push(child);
            }
        } else {
            for &child in document.children(node_id).iter().rev() {
                stack.push(child);
            }
        }
    }
    ordered
}

#[cfg(feature = "dom")]
pub fn selector_matches(document: &Document, node_id: NodeId, selector: &Selector) -> bool {
    let Some(last) = selector.parts.last() else {
        return false;
    };
    if !simple_selector_matches(document, node_id, last) {
        return false;
    }
    let mut current = node_id;
    for part in selector.parts[..selector.parts.len().saturating_sub(1)]
        .iter()
        .rev()
    {
        let mut found = None;
        for _ in 0..MAX_DESCENDANT_DEPTH {
            let Some(parent) = document.parent(current) else {
                break;
            };
            current = parent;
            if simple_selector_matches(document, current, part) {
                found = Some(current);
                break;
            }
        }
        let Some(ancestor) = found else {
            return false;
        };
        current = ancestor;
    }
    true
}

#[cfg(feature = "dom")]
fn simple_selector_matches(
    document: &Document,
    node_id: NodeId,
    selector: &SimpleSelector,
) -> bool {
    let Some(Node::Element {
        tag_name,
        attributes,
        ..
    }) = document.get(node_id)
    else {
        return false;
    };
    if let Some(expected) = &selector.tag_name {
        if !tag_name.eq_ignore_ascii_case(expected) {
            return false;
        }
    }
    if let Some(expected) = &selector.id {
        if !attribute_value(attributes, "id").is_some_and(|value| value == expected) {
            return false;
        }
    }
    let classes = attribute_value(attributes, "class").unwrap_or("");
    selector.classes.iter().all(|expected| {
        classes
            .split_ascii_whitespace()
            .any(|actual| actual == expected)
    })
}

#[cfg(feature = "dom")]
fn compute_element_style(
    document: &Document,
    node_id: NodeId,
    parent: Option<&ComputedStyle>,
    sheets: &[Stylesheet],
) -> ComputedStyle {
    let mut computed = initial_style(parent);
    let mut winners: Vec<Option<(CascadeKey, MatchedDeclaration)>> =
        vec![None; PROPERTY_ORDER.len()];
    let mut custom_winners: Vec<(String, CascadeKey, MatchedDeclaration)> = Vec::new();
    let mut all_candidates: Vec<MatchedDeclaration> = Vec::new();
    let mut source_order = 0usize;
    for (sheet_index, sheet) in sheets.iter().enumerate() {
        for rule in &sheet.rules {
            for selector in &rule.selectors {
                if !selector_matches(document, node_id, selector) {
                    continue;
                }
                let specificity = selector_specificity(selector);
                for declaration in &rule.declarations {
                    if let Property::Custom(name) = &declaration.property {
                        let key = CascadeKey {
                            important: declaration.important,
                            origin: if matches!(sheet.source, StylesheetSource::UserAgent) {
                                0
                            } else {
                                1
                            },
                            specificity,
                            order: source_order.saturating_add(declaration.order),
                        };
                        let matched = MatchedDeclaration {
                            property: declaration.property.clone(),
                            value: declaration.value.clone(),
                            selector: selector.text.clone(),
                            original_selector: selector.original_text.clone(),
                            source: sheet.source.label(),
                            specificity,
                            inherited: false,
                            important: declaration.important,
                            source_order: source_order.saturating_add(declaration.order),
                        };
                        all_candidates.push(matched.clone());
                        if let Some(existing) =
                            custom_winners.iter_mut().find(|entry| entry.0 == *name)
                        {
                            if key >= existing.1 {
                                *existing = (name.clone(), key, matched);
                            }
                        } else {
                            custom_winners.push((name.clone(), key, matched));
                        }
                        continue;
                    }
                    let Some(property_index) = property_index(&declaration.property) else {
                        continue;
                    };
                    let key = CascadeKey {
                        important: declaration.important,
                        origin: if matches!(sheet.source, StylesheetSource::UserAgent) {
                            0
                        } else {
                            1
                        },
                        specificity,
                        order: source_order.saturating_add(declaration.order),
                    };
                    let matched = MatchedDeclaration {
                        property: declaration.property.clone(),
                        value: declaration.value.clone(),
                        selector: selector.text.clone(),
                        original_selector: selector.original_text.clone(),
                        source: sheet.source.label(),
                        specificity,
                        inherited: false,
                        important: declaration.important,
                        source_order: source_order.saturating_add(declaration.order),
                    };
                    all_candidates.push(matched.clone());
                    if winners[property_index]
                        .as_ref()
                        .is_none_or(|(old, _)| key >= *old)
                    {
                        winners[property_index] = Some((key, matched));
                    }
                }
            }
            source_order = source_order.saturating_add(1);
        }
        source_order = source_order.saturating_add(sheet_index.saturating_add(1));
    }
    if let Some(Node::Element { attributes, .. }) = document.get(node_id) {
        if let Some(style) = attribute_value(attributes, "style") {
            for declaration in parse_inline_style(style) {
                if let Property::Custom(name) = &declaration.property {
                    let key = CascadeKey {
                        important: declaration.important,
                        origin: 2,
                        specificity: Specificity {
                            ids: u16::MAX,
                            classes: 0,
                            tags: 0,
                        },
                        order: declaration.order,
                    };
                    let matched = MatchedDeclaration {
                        property: declaration.property.clone(),
                        value: declaration.value.clone(),
                        selector: String::from("style=\"\""),
                        original_selector: None,
                        source: String::from("inline style"),
                        specificity: key.specificity,
                        inherited: false,
                        important: declaration.important,
                        source_order: declaration.order,
                    };
                    all_candidates.push(matched.clone());
                    if let Some(existing) = custom_winners.iter_mut().find(|entry| entry.0 == *name)
                    {
                        if key >= existing.1 {
                            *existing = (name.clone(), key, matched);
                        }
                    } else {
                        custom_winners.push((name.clone(), key, matched));
                    }
                    continue;
                }
                let Some(property_index) = property_index(&declaration.property) else {
                    continue;
                };
                let matched = MatchedDeclaration {
                    property: declaration.property.clone(),
                    value: declaration.value.clone(),
                    selector: String::from("style=\"\""),
                    original_selector: None,
                    source: String::from("inline style"),
                    specificity: Specificity {
                        ids: u16::MAX,
                        classes: 0,
                        tags: 0,
                    },
                    inherited: false,
                    important: declaration.important,
                    source_order: declaration.order,
                };
                all_candidates.push(matched.clone());
                winners[property_index] = Some((
                    CascadeKey {
                        important: declaration.important,
                        origin: 2,
                        specificity: matched.specificity,
                        order: declaration.order,
                    },
                    matched,
                ));
            }
        }
    }
    for (index, winner) in winners.into_iter().enumerate() {
        if let Some((_, matched)) = winner {
            let raw = matched.raw_value();
            let expanded = resolve_vars(&computed.custom_properties, &raw, 0, &mut Vec::new())
                .unwrap_or_default();
            if expanded.eq_ignore_ascii_case("unset") {
                computed.properties[index].value = if matched.property.is_inherited() {
                    computed.properties[index].value.clone()
                } else {
                    initial_value(&matched.property)
                };
            } else if expanded.eq_ignore_ascii_case("initial") {
                computed.properties[index].value = initial_value(&matched.property);
            } else if expanded.eq_ignore_ascii_case("inherit") {
                computed.properties[index].value = computed.properties[index].value.clone();
            } else if !expanded.is_empty() {
                computed.properties[index].value = parse_value(&matched.property, &expanded);
            }
            computed.properties[index].matched = Some(matched);
        }
    }
    for (name, _, matched) in custom_winners {
        if let Some(existing) = computed
            .custom_properties
            .iter_mut()
            .find(|entry| entry.0 == name)
        {
            *existing = (name, matched.value.clone(), Some(matched));
        } else {
            computed
                .custom_properties
                .push((name, matched.value.clone(), Some(matched)));
        }
    }
    computed.matched_declarations = all_candidates;
    computed
}

trait MatchedRawValue {
    fn raw_value(&self) -> String;
}
impl MatchedRawValue for MatchedDeclaration {
    fn raw_value(&self) -> String {
        self.value.display()
    }
}

fn resolve_vars(
    properties: &[(String, PropertyValue, Option<MatchedDeclaration>)],
    input: &str,
    depth: usize,
    stack: &mut Vec<String>,
) -> Option<String> {
    if depth > 16 || input.len() > 8192 {
        return None;
    }
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("var(") {
        out.push_str(&rest[..start]);
        let mut level = 0usize;
        let mut end = None;
        for (i, byte) in rest.as_bytes()[start + 4..].iter().enumerate() {
            if *byte == b'(' {
                level += 1;
            }
            if *byte == b')' {
                if level == 0 {
                    end = Some(start + 4 + i);
                    break;
                }
                level -= 1;
            }
        }
        let end = end?;
        let contents = &rest[start + 4..end];
        let (name, fallback) = contents
            .split_once(',')
            .map_or((contents.trim(), None), |(n, f)| (n.trim(), Some(f.trim())));
        if !name.starts_with("--") || stack.iter().any(|item| item == name) {
            return fallback
                .and_then(|f| resolve_vars(properties, f, depth + 1, stack))
                .map(|v| {
                    out.push_str(&v);
                    rest = &rest[end + 1..];
                    v
                });
        }
        let value = properties
            .iter()
            .find(|entry| entry.0 == name)
            .map(|entry| entry.1.display());
        let replacement = match value {
            Some(value) => {
                stack.push(String::from(name));
                let result = resolve_vars(properties, &value, depth + 1, stack);
                stack.pop();
                result
            }
            None => fallback.and_then(|f| resolve_vars(properties, f, depth + 1, stack)),
        }?;
        out.push_str(&replacement);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(feature = "dom")]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CascadeKey {
    important: bool,
    origin: u8,
    specificity: Specificity,
    order: usize,
}

#[cfg(feature = "dom")]
fn selector_specificity(selector: &Selector) -> Specificity {
    let mut specificity = Specificity::default();
    for part in &selector.parts {
        specificity.ids = specificity.ids.saturating_add(part.id.is_some() as u16);
        specificity.classes = specificity
            .classes
            .saturating_add(part.classes.len() as u16);
        specificity.tags = specificity
            .tags
            .saturating_add(part.tag_name.is_some() as u16);
    }
    specificity
}

#[cfg(feature = "dom")]
fn initial_style(parent: Option<&ComputedStyle>) -> ComputedStyle {
    let mut properties = Vec::new();
    for property in PROPERTY_ORDER {
        let inherited = property.is_inherited();
        let parent_value = parent.and_then(|style| style.value(property)).cloned();
        properties.push(ComputedProperty {
            property: property.clone(),
            value: if inherited {
                parent_value
                    .clone()
                    .unwrap_or_else(|| initial_value(property))
            } else {
                initial_value(property)
            },
            matched: parent_value
                .filter(|_| inherited)
                .map(|value| MatchedDeclaration {
                    property: property.clone(),
                    value,
                    selector: String::from("inherited"),
                    original_selector: None,
                    source: String::from("parent element"),
                    specificity: Specificity::default(),
                    inherited: true,
                    important: false,
                    source_order: 0,
                }),
        });
    }
    ComputedStyle {
        properties,
        custom_properties: parent
            .map(|style| style.custom_properties.clone())
            .unwrap_or_default(),
        matched_declarations: Vec::new(),
    }
}

#[cfg(feature = "dom")]
fn initial_value(property: &Property) -> PropertyValue {
    match property {
        Property::Display => PropertyValue::Keyword(String::from("inline")),
        Property::FlexDirection => PropertyValue::Keyword(String::from("row")),
        Property::FlexWrap => PropertyValue::Keyword(String::from("nowrap")),
        Property::JustifyContent => PropertyValue::Keyword(String::from("flex-start")),
        Property::AlignItems => PropertyValue::Keyword(String::from("stretch")),
        Property::AlignContent => PropertyValue::Keyword(String::from("stretch")),
        Property::Gap | Property::RowGap | Property::ColumnGap => PropertyValue::LengthPx(0),
        Property::Color | Property::BorderColor => PropertyValue::Color(Color::Rgb(0, 0, 0)),
        Property::BackgroundColor => PropertyValue::Color(Color::Transparent),
        Property::FontSize => PropertyValue::LengthPx(16),
        Property::FontFamily => PropertyValue::Keyword(String::from("serif")),
        Property::FontWeight
        | Property::FontStyle
        | Property::TextAlign
        | Property::TextDecoration
        | Property::LineHeight
        | Property::WhiteSpace => PropertyValue::Normal,
        Property::ListStyleType => PropertyValue::Keyword(String::from("disc")),
        Property::ListStyle => PropertyValue::Keyword(String::from("disc")),
        Property::ListStylePosition => PropertyValue::Keyword(String::from("outside")),
        Property::Width | Property::Height => PropertyValue::Auto,
        Property::MarginTop
        | Property::MarginRight
        | Property::MarginBottom
        | Property::MarginLeft
        | Property::PaddingTop
        | Property::PaddingRight
        | Property::PaddingBottom
        | Property::PaddingLeft
        | Property::BorderWidth => PropertyValue::LengthPx(0),
        Property::BorderStyle => PropertyValue::Keyword(String::from("none")),
        Property::BackgroundImage => PropertyValue::Keyword(String::from("none")),
        Property::BackgroundRepeat => PropertyValue::Keyword(String::from("repeat")),
        Property::BackgroundAttachment => PropertyValue::Keyword(String::from("scroll")),
        Property::BackgroundPositionX => PropertyValue::Keyword(String::from("0%")),
        Property::BackgroundPositionY => PropertyValue::Keyword(String::from("0%")),
        Property::BoxSizing => PropertyValue::Keyword(String::from("content-box")),
        Property::MinHeight | Property::LetterSpacing => PropertyValue::LengthPx(0),
        Property::Border | Property::BorderBottom | Property::Unknown(_) | Property::Custom(_) => {
            PropertyValue::Raw(String::new())
        }
    }
}

#[cfg(feature = "dom")]
fn property_index(property: &Property) -> Option<usize> {
    PROPERTY_ORDER.iter().position(|known| known == property)
}

#[cfg(feature = "dom")]
fn attribute_value<'a>(attributes: &'a [golden_fish::Attribute], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name().eq_ignore_ascii_case(name))
        .map(golden_fish::Attribute::value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "dom")]
    use golden_fish::parse_html;

    #[test]
    fn parses_rules_declarations_selectors_and_comments() {
        let sheet = parse_stylesheet(
            "/* x */ body, .notice { color: red; padding: 8px; broken }",
            StylesheetSource::Embedded,
        );
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selectors.len(), 2);
        assert_eq!(sheet.rules[0].declarations.len(), 5);
        assert_eq!(sheet.rules[0].declarations[0].property, Property::Color);
    }
    #[test]
    fn inline_styles_are_tolerant() {
        let declarations = parse_inline_style("COLOR: #abc; malformed; margin-top: 4px");
        assert_eq!(declarations.len(), 2);
        assert_eq!(
            declarations[0].value,
            PropertyValue::Color(Color::Rgb(170, 187, 204))
        );
    }
    #[test]
    fn parses_css_values() {
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some(Color::Rgb(1, 2, 3)));
        assert_eq!(parse_color("blue"), Some(Color::Rgb(0, 0, 255)));
        assert_eq!(parse_length("12px"), Some(PropertyValue::LengthPx(12)));
    }

    #[test]
    fn preserves_custom_properties_importance_and_nested_rules() {
        let sheet = parse_stylesheet(
            ":root { --archlinux-blue: #1793d1; } #archnavbar { --logo: url(arch.svg); background: var(--logo) none repeat scroll 0 0 !important; & ul { list-style: none; } }",
            StylesheetSource::Embedded,
        );
        assert!(sheet
            .rules
            .iter()
            .any(|rule| rule.selectors[0].text == "#archnavbar"));
        assert!(sheet
            .rules
            .iter()
            .any(|rule| rule.selectors[0].text == "#archnavbar ul"));
        assert!(sheet
            .rules
            .iter()
            .flat_map(|rule| rule.declarations.iter())
            .any(|decl| decl.important));
        assert!(sheet
            .rules
            .iter()
            .flat_map(|rule| rule.declarations.iter())
            .any(|decl| matches!(decl.property, Property::Custom(_))));
    }

    #[test]
    fn padding_background_border_and_min_height_parse() {
        let sheet = parse_stylesheet("nav { padding: 10px 15px !important; background: #333 none no-repeat scroll 0 0; border-bottom: 5px solid #1793d1 !important; box-sizing: unset; min-height: 40px; }", StylesheetSource::Embedded);
        let declarations = &sheet.rules[0].declarations;
        assert!(declarations
            .iter()
            .filter(|d| matches!(
                d.property,
                Property::PaddingTop
                    | Property::PaddingRight
                    | Property::PaddingBottom
                    | Property::PaddingLeft
            ))
            .all(|d| d.important));
        assert!(declarations
            .iter()
            .any(|d| d.property == Property::BackgroundColor));
        assert!(declarations
            .iter()
            .any(|d| d.property == Property::BorderColor));
        assert!(declarations
            .iter()
            .any(|d| d.property == Property::MinHeight));
    }

    #[test]
    fn parses_flex_and_list_style_values() {
        let sheet = parse_stylesheet(
            ".menu { display: flex; flex-direction: row; flex-wrap: wrap; justify-content: space-between; align-items: center; align-content: center; gap: 8px 16px; list-style: none; }",
            StylesheetSource::Embedded,
        );
        let properties: Vec<_> = sheet.rules[0]
            .declarations
            .iter()
            .map(|d| &d.property)
            .collect();
        assert!(properties.contains(&&Property::Display));
        assert!(properties.contains(&&Property::RowGap));
        assert!(properties.contains(&&Property::ColumnGap));
        assert!(properties.contains(&&Property::ListStyleType));
    }

    #[cfg(feature = "dom")]
    fn styles(html: &str, css: &str) -> (Document, StyleContext) {
        let document = parse_html(html).unwrap();
        let context = StyleContext::build(
            &document,
            &[parse_stylesheet(css, StylesheetSource::Embedded)],
        );
        (document, context)
    }
    #[cfg(feature = "dom")]
    #[test]
    fn matches_tag_class_id_combined_and_descendant_selectors() {
        let (document, _) = styles(
            "<main><div id='banner' class='header active'><a class='download'>x</a></div></main>",
            "",
        );
        let div = document.find_first_element("div").unwrap();
        let a = document.find_first_element("a").unwrap();
        for selector in ["div", ".header", "#banner", "div.header.active"] {
            assert!(selector_matches(
                &document,
                div,
                &parse_selector(selector).unwrap()
            ));
        }
        assert!(selector_matches(
            &document,
            a,
            &parse_selector("main .download").unwrap()
        ));
        assert!(!selector_matches(
            &document,
            div,
            &parse_selector("p.notice").unwrap()
        ));
    }
    #[cfg(feature = "dom")]
    #[test]
    fn cascade_inheritance_and_non_inherited_values_work() {
        let (document, context) = styles("<body><p class='notice' id='one'>text</p></body>", "body { color: red; font-size: 18px; background-color: blue; } p { color: green; } .notice { color: blue; } #one { color: #010203; } p { background-color: red; } p { background-color: green; }");
        let body = document.find_first_element("body").unwrap();
        let p = document.find_first_element("p").unwrap();
        assert_eq!(
            context.style_for(p).unwrap().value(&Property::Color),
            Some(&PropertyValue::Color(Color::Rgb(1, 2, 3)))
        );
        assert_eq!(
            context.style_for(p).unwrap().value(&Property::FontSize),
            Some(&PropertyValue::LengthPx(18))
        );
        assert_eq!(
            context
                .style_for(p)
                .unwrap()
                .value(&Property::BackgroundColor),
            Some(&PropertyValue::Color(Color::Rgb(0, 128, 0)))
        );
        assert_eq!(
            context
                .style_for(body)
                .unwrap()
                .value(&Property::BackgroundColor),
            Some(&PropertyValue::Color(Color::Rgb(0, 0, 255)))
        );
    }
    #[cfg(feature = "dom")]
    #[test]
    fn inline_style_beats_stylesheet_and_shorthands_expand() {
        let (document, context) = styles(
            "<body><div style='color: red; padding-left: 12px'></div></body>",
            "div { color: blue; }",
        );
        let div = document.find_first_element("div").unwrap();
        let style = context.style_for(div).unwrap();
        assert_eq!(
            style.value(&Property::Color),
            Some(&PropertyValue::Color(Color::Rgb(255, 0, 0)))
        );
        assert_eq!(
            style.value(&Property::PaddingLeft),
            Some(&PropertyValue::LengthPx(12))
        );
    }
    #[cfg(feature = "dom")]
    #[test]
    fn embedded_stylesheet_integration() {
        let document = parse_html("<html><head><style>body { color: #222; } main span { font-weight: bold; }</style></head><body><main><span>x</span></main></body></html>").unwrap();
        let sheets = collect_embedded_stylesheets(&document);
        let context = StyleContext::build(&document, &sheets);
        let a = document.find_first_element("span").unwrap();
        assert_eq!(
            context.style_for(a).unwrap().value(&Property::Color),
            Some(&PropertyValue::Color(Color::Rgb(34, 34, 34)))
        );
        assert_eq!(
            context.style_for(a).unwrap().value(&Property::FontWeight),
            Some(&PropertyValue::Keyword(String::from("bold")))
        );
    }

    #[cfg(feature = "dom")]
    #[test]
    fn acceptance_fixture_computes_selected_element_styles() {
        let document = parse_html(include_str!("../tests/fixtures/css-basics.html")).unwrap();
        let context = StyleContext::build(&document, &collect_embedded_stylesheets(&document));
        let header = document.find_first_element("header").unwrap();
        let h1 = document.find_first_element("h1").unwrap();
        let notice = document.find_first_element("p").unwrap();
        let download = document.find_first_element("a").unwrap();
        assert_eq!(
            context
                .style_for(header)
                .unwrap()
                .value(&Property::PaddingLeft),
            Some(&PropertyValue::LengthPx(12))
        );
        assert_eq!(
            context.style_for(h1).unwrap().value(&Property::FontSize),
            Some(&PropertyValue::LengthPx(28))
        );
        assert_eq!(
            context.style_for(notice).unwrap().value(&Property::Color),
            Some(&PropertyValue::Color(Color::Rgb(255, 0, 0)))
        );
        assert_eq!(
            context
                .style_for(notice)
                .unwrap()
                .value(&Property::MarginTop),
            Some(&PropertyValue::LengthPx(10))
        );
        assert_eq!(
            context
                .style_for(download)
                .unwrap()
                .value(&Property::FontWeight),
            Some(&PropertyValue::Keyword(String::from("bold")))
        );
    }

    #[cfg(feature = "dom")]
    #[test]
    fn document_source_order_interleaves_embedded_and_linked_sheets() {
        let document = parse_html("<head><link rel='stylesheet' href='a.css'><style>p { color: blue; }</style></head><body><p>x</p></body>").unwrap();
        let embedded = collect_embedded_stylesheets(&document);
        let linked = vec![Some(parse_stylesheet(
            "p { color: red; }",
            StylesheetSource::External(String::from("a.css")),
        ))];
        let context = StyleContext::build(
            &document,
            &order_document_stylesheets(&document, embedded, linked),
        );
        let p = document.find_first_element("p").unwrap();
        assert_eq!(
            context.style_for(p).unwrap().value(&Property::Color),
            Some(&PropertyValue::Color(Color::Rgb(0, 0, 255)))
        );
    }
}
