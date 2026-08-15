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
pub const MAX_NESTING_DEPTH: usize = 8;
pub const MAX_IMPORT_DEPTH: usize = 8;
pub const MAX_IMPORTED_STYLESHEETS: usize = 32;
pub const MAX_TOTAL_STYLESHEET_BYTES: usize = 512 * 1024;
pub const MAX_TOTAL_CSS_RULES: usize = 8_192;
/// Desktop-first default used when a caller does not supply a viewport.
/// `@media (max-width: 848px)` rules from sites like kernel.org stay inactive.
pub const DEFAULT_VIEWPORT_WIDTH: u32 = 1024;
pub const MAX_GRADIENT_STOPS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StylesheetImport {
    pub raw_url: String,
    pub media: String,
    pub location: SourceLocation,
}

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
    /// Combined `@media` condition wrapping this rule, if any.
    pub media: Option<String>,
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
    /// Combinator between each adjacent pair of parts.  A missing entry is
    /// never allowed; the vector is always parts.len() - 1 long.
    pub combinators: Vec<Combinator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SimpleSelector {
    pub tag_name: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub universal: bool,
    pub root: bool,
    pub hover: bool,
    pub first_child: bool,
    pub last_child: bool,
    /// CSS `an+b` formula for `:nth-child`. `None` means the pseudo is absent.
    pub nth_child: Option<(i32, i32)>,
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
    BackgroundSize,
    FontSize,
    FontFamily,
    FontWeight,
    FontStyle,
    TextAlign,
    TextDecoration,
    TextShadow,
    WhiteSpace,
    ListStyleType,
    ListStyle,
    ListStylePosition,
    LineHeight,
    Width,
    Height,
    MinWidth,
    MaxWidth,
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
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    BorderTopStyle,
    BorderRightStyle,
    BorderBottomStyle,
    BorderLeftStyle,
    BorderTopColor,
    BorderRightColor,
    BorderBottomColor,
    BorderLeftColor,
    BorderRadius,
    BorderTopLeftRadius,
    BorderTopRightRadius,
    BorderBottomRightRadius,
    BorderBottomLeftRadius,
    BoxShadow,
    Opacity,
    Float,
    Clear,
    BorderCollapse,
    BorderSpacing,
    BorderBottom,
    Custom(String),
    Unknown(String),
}

impl Property {
    pub fn parse(name: &str) -> Self {
        let trimmed = name.trim();
        if trimmed.starts_with("--") {
            return Self::Custom(String::from(trimmed));
        }
        match trimmed.to_ascii_lowercase().as_str() {
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
            "background-size" => Self::BackgroundSize,
            "font-size" => Self::FontSize,
            "font-family" => Self::FontFamily,
            "font-weight" => Self::FontWeight,
            "font-style" => Self::FontStyle,
            "text-align" => Self::TextAlign,
            "text-decoration" => Self::TextDecoration,
            "text-shadow" => Self::TextShadow,
            "white-space" => Self::WhiteSpace,
            "list-style-type" => Self::ListStyleType,
            "list-style" => Self::ListStyle,
            "list-style-position" => Self::ListStylePosition,
            "line-height" => Self::LineHeight,
            "width" => Self::Width,
            "height" => Self::Height,
            "min-width" => Self::MinWidth,
            "max-width" => Self::MaxWidth,
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
            "border-top-width" => Self::BorderTopWidth,
            "border-right-width" => Self::BorderRightWidth,
            "border-bottom-width" => Self::BorderBottomWidth,
            "border-left-width" => Self::BorderLeftWidth,
            "border-top-style" => Self::BorderTopStyle,
            "border-right-style" => Self::BorderRightStyle,
            "border-bottom-style" => Self::BorderBottomStyle,
            "border-left-style" => Self::BorderLeftStyle,
            "border-top-color" => Self::BorderTopColor,
            "border-right-color" => Self::BorderRightColor,
            "border-bottom-color" => Self::BorderBottomColor,
            "border-left-color" => Self::BorderLeftColor,
            "border-radius" => Self::BorderRadius,
            "border-top-left-radius" => Self::BorderTopLeftRadius,
            "border-top-right-radius" => Self::BorderTopRightRadius,
            "border-bottom-right-radius" => Self::BorderBottomRightRadius,
            "border-bottom-left-radius" => Self::BorderBottomLeftRadius,
            "box-shadow" => Self::BoxShadow,
            "opacity" => Self::Opacity,
            "float" => Self::Float,
            "clear" => Self::Clear,
            "border-collapse" => Self::BorderCollapse,
            "border-spacing" => Self::BorderSpacing,
            "border-bottom" => Self::BorderBottom,
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
            Self::BackgroundSize => "background-size",
            Self::FontSize => "font-size",
            Self::FontFamily => "font-family",
            Self::FontWeight => "font-weight",
            Self::FontStyle => "font-style",
            Self::TextAlign => "text-align",
            Self::TextDecoration => "text-decoration",
            Self::TextShadow => "text-shadow",
            Self::WhiteSpace => "white-space",
            Self::ListStyleType => "list-style-type",
            Self::ListStyle => "list-style",
            Self::ListStylePosition => "list-style-position",
            Self::LineHeight => "line-height",
            Self::Width => "width",
            Self::Height => "height",
            Self::MinWidth => "min-width",
            Self::MaxWidth => "max-width",
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
            Self::BorderTopWidth => "border-top-width",
            Self::BorderRightWidth => "border-right-width",
            Self::BorderBottomWidth => "border-bottom-width",
            Self::BorderLeftWidth => "border-left-width",
            Self::BorderTopStyle => "border-top-style",
            Self::BorderRightStyle => "border-right-style",
            Self::BorderBottomStyle => "border-bottom-style",
            Self::BorderLeftStyle => "border-left-style",
            Self::BorderTopColor => "border-top-color",
            Self::BorderRightColor => "border-right-color",
            Self::BorderBottomColor => "border-bottom-color",
            Self::BorderLeftColor => "border-left-color",
            Self::BorderRadius => "border-radius",
            Self::BorderTopLeftRadius => "border-top-left-radius",
            Self::BorderTopRightRadius => "border-top-right-radius",
            Self::BorderBottomRightRadius => "border-bottom-right-radius",
            Self::BorderBottomLeftRadius => "border-bottom-left-radius",
            Self::BoxShadow => "box-shadow",
            Self::Opacity => "opacity",
            Self::Float => "float",
            Self::Clear => "clear",
            Self::BorderCollapse => "border-collapse",
            Self::BorderSpacing => "border-spacing",
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
                | Self::TextShadow
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
    CurrentColor,
    LengthPx(i32),
    /// Fixed-point em value in thousandths, retained until the element font is known.
    LengthEm(i32),
    /// Fixed-point percentage in hundredths of one percent.
    Percentage(i32),
    Auto,
    Normal,
    Keyword(String),
    Raw(String),
}

impl PropertyValue {
    pub fn display(&self) -> String {
        match self {
            Self::Color(color) => color.display(),
            Self::CurrentColor => String::from("currentColor"),
            Self::LengthPx(value) => format!("{value}px"),
            Self::LengthEm(value) => format!("{}em", *value as f32 / 1000.0),
            Self::Percentage(value) => format!("{}%", *value as f32 / 100.0),
            Self::Auto => String::from("auto"),
            Self::Normal => String::from("normal"),
            Self::Keyword(value) | Self::Raw(value) => value.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Rgb(u8, u8, u8),
    Rgba(u8, u8, u8, u8),
    Transparent,
}

impl Color {
    pub fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::Rgb(red, green, blue)
    }

    pub fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        if alpha == 0 {
            Self::Transparent
        } else if alpha == 255 {
            Self::Rgb(red, green, blue)
        } else {
            Self::Rgba(red, green, blue, alpha)
        }
    }

    pub fn channels(self) -> (u8, u8, u8, u8) {
        match self {
            Self::Rgb(red, green, blue) => (red, green, blue, 255),
            Self::Rgba(red, green, blue, alpha) => (red, green, blue, alpha),
            Self::Transparent => (0, 0, 0, 0),
        }
    }

    pub fn display(self) -> String {
        match self {
            Self::Rgb(red, green, blue) => format!("#{red:02x}{green:02x}{blue:02x}"),
            Self::Rgba(red, green, blue, alpha) => {
                format!("rgba({red}, {green}, {blue}, {})", alpha as f32 / 255.0)
            }
            Self::Transparent => String::from("transparent"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientKind {
    Linear,
    Radial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GradientStop {
    pub color: Color,
    /// Fixed-point percentage in hundredths of one percent (`0..=10_000`).
    pub position: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssGradient {
    pub kind: GradientKind,
    /// CSS degrees, where `0` is upward and `180` is the default top-to-bottom.
    pub angle_deg: i32,
    pub stops: Vec<GradientStop>,
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
    pub location: Option<SourceLocation>,
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
    Property::BackgroundSize,
    Property::FontSize,
    Property::FontFamily,
    Property::FontWeight,
    Property::FontStyle,
    Property::TextAlign,
    Property::TextDecoration,
    Property::TextShadow,
    Property::WhiteSpace,
    Property::ListStyleType,
    Property::ListStyle,
    Property::ListStylePosition,
    Property::LineHeight,
    Property::Width,
    Property::Height,
    Property::MinWidth,
    Property::MaxWidth,
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
    Property::BorderTopWidth,
    Property::BorderRightWidth,
    Property::BorderBottomWidth,
    Property::BorderLeftWidth,
    Property::BorderTopStyle,
    Property::BorderRightStyle,
    Property::BorderBottomStyle,
    Property::BorderLeftStyle,
    Property::BorderTopColor,
    Property::BorderRightColor,
    Property::BorderBottomColor,
    Property::BorderLeftColor,
    Property::BorderRadius,
    Property::BorderTopLeftRadius,
    Property::BorderTopRightRadius,
    Property::BorderBottomRightRadius,
    Property::BorderBottomLeftRadius,
    Property::BoxShadow,
    Property::Opacity,
    Property::Float,
    Property::Clear,
    Property::BorderCollapse,
    Property::BorderSpacing,
    Property::BorderBottom,
];

/// Parses an ordinary stylesheet without panicking on malformed website CSS.
pub fn parse_stylesheet(css: &str, source: StylesheetSource) -> Stylesheet {
    let css = bounded_css(css);
    let clean = strip_comments(css);
    let mut rules = Vec::new();
    parse_rule_block(&clean, None, &mut rules, &css, None);
    Stylesheet { source, rules }
}

/// Extract valid, leading statement-form `@import` rules without treating
/// strings, comments, or parentheses as statement boundaries. Imports after
/// the first qualified or block rule are deliberately ignored.
pub fn parse_leading_imports(css: &str) -> Vec<StylesheetImport> {
    let css = bounded_css(css);
    let bytes = css.as_bytes();
    let mut imports = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        cursor = skip_css_whitespace_and_comments(css, cursor);
        if cursor >= bytes.len() || bytes[cursor] != b'@' {
            break;
        }
        let name_start = cursor + 1;
        let mut name_end = name_start;
        while name_end < bytes.len()
            && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'-')
        {
            name_end += 1;
        }
        let name = &css[name_start..name_end];
        let Some(statement_end) = css_statement_end(css, name_end) else {
            break;
        };
        if statement_end.1 {
            break;
        }
        if name.eq_ignore_ascii_case("import") {
            if let Some((raw_url, media)) = parse_import_prelude(&css[name_end..statement_end.0]) {
                imports.push(StylesheetImport {
                    raw_url,
                    media,
                    location: location_from_offset(css, cursor),
                });
            }
        } else if !name.eq_ignore_ascii_case("charset") && !name.eq_ignore_ascii_case("layer") {
            break;
        }
        cursor = statement_end.0.saturating_add(1);
    }
    imports
}

/// Evaluates the intentionally small media subset needed by stylesheet
/// imports. Unknown conditions remain active so usable CSS is not silently
/// discarded; the returned reason makes that approximation observable.
pub fn import_media_active(media: &str, viewport_width: u32) -> (bool, Option<String>) {
    let media = media.trim();
    if media.is_empty() || media.eq_ignore_ascii_case("all") || media.eq_ignore_ascii_case("screen")
    {
        return (true, None);
    }
    let lower = media.to_ascii_lowercase();
    if lower == "print" {
        return (false, None);
    }
    for (feature, is_min) in [("min-width", true), ("max-width", false)] {
        if let Some(start) = lower.find(feature) {
            let tail = &lower[start + feature.len()..];
            let Some(colon) = tail.find(':') else {
                continue;
            };
            let value = tail[colon + 1..]
                .trim_start()
                .trim_end_matches(|ch: char| ch == ')' || ch.is_ascii_whitespace());
            if let Some(px) = value
                .strip_suffix("px")
                .and_then(|v| v.trim().parse::<u32>().ok())
            {
                return (
                    if is_min {
                        viewport_width >= px
                    } else {
                        viewport_width <= px
                    },
                    None,
                );
            }
        }
    }
    (
        true,
        Some(format!(
            "unsupported media condition treated as active: {media}"
        )),
    )
}

fn merge_media(parent: Option<&str>, nested: &str) -> String {
    match parent {
        Some(parent) if !parent.is_empty() && !nested.is_empty() => {
            format!("{parent} and {nested}")
        }
        Some(parent) if !parent.is_empty() => String::from(parent),
        _ => String::from(nested),
    }
}

fn skip_css_whitespace_and_comments(css: &str, mut cursor: usize) -> usize {
    let bytes = css.as_bytes();
    loop {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor + 1 < bytes.len() && bytes[cursor] == b'/' && bytes[cursor + 1] == b'*' {
            cursor += 2;
            while cursor + 1 < bytes.len() && !(bytes[cursor] == b'*' && bytes[cursor + 1] == b'/')
            {
                cursor += 1;
            }
            cursor = cursor.saturating_add(2).min(bytes.len());
            continue;
        }
        return cursor;
    }
}

/// Returns `(terminator_offset, encountered_block)`.
fn css_statement_end(css: &str, mut cursor: usize) -> Option<(usize, bool)> {
    let bytes = css.as_bytes();
    let mut quote = None;
    let mut escaped = false;
    let mut parens = 0usize;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                quote = None;
            }
            cursor += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' => parens = parens.saturating_add(1),
            b')' => parens = parens.saturating_sub(1),
            b';' if parens == 0 => return Some((cursor, false)),
            b'{' if parens == 0 => return Some((cursor, true)),
            b'/' if cursor + 1 < bytes.len() && bytes[cursor + 1] == b'*' => {
                cursor = skip_css_whitespace_and_comments(css, cursor);
                continue;
            }
            _ => {}
        }
        cursor += 1;
    }
    None
}

fn parse_import_prelude(prelude: &str) -> Option<(String, String)> {
    let value = prelude.trim();
    let (raw_url, rest) = if value.starts_with(['\'', '"']) {
        let delimiter = value.as_bytes()[0];
        let mut end = 1usize;
        let mut escaped = false;
        while end < value.len() {
            let byte = value.as_bytes()[end];
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                break;
            }
            end += 1;
        }
        if end >= value.len() {
            return None;
        }
        (String::from(&value[1..end]), &value[end + 1..])
    } else if value
        .get(..4)
        .is_some_and(|head| head.eq_ignore_ascii_case("url("))
    {
        let close = value[4..].find(')')? + 4;
        let token = value[4..close].trim();
        let token = token
            .strip_prefix(['\'', '"'])
            .and_then(|inner| inner.strip_suffix(['\'', '"']))
            .unwrap_or(token);
        (String::from(token), &value[close + 1..])
    } else {
        return None;
    };
    if raw_url.is_empty() {
        None
    } else {
        Some((raw_url, String::from(rest.trim())))
    }
}

fn parse_rule_block(
    input: &str,
    parent: Option<&str>,
    rules: &mut Vec<Rule>,
    original_for_lines: &str,
    media: Option<&str>,
) {
    let mut cursor = 0usize;
    while cursor < input.len() && rules.len() < MAX_RULES {
        let Some(open_rel) = input[cursor..].find('{') else {
            break;
        };
        let open = cursor + open_rel;

        // A stylesheet may start with statement at-rules, most commonly an
        // @import, before its first qualified rule.  Do not let that
        // statement become part of the next selector prelude (which would
        // make the complete qualified rule look like an unsupported at-rule).
        // Keep this deliberately narrow: only a semicolon-terminated at-rule
        // is skipped here; block at-rules are handled below.
        if let Some(statement_rel) = input[cursor..open].find(';') {
            let statement_end = cursor + statement_rel;
            if input[cursor..statement_end].trim_start().starts_with('@') {
                cursor = statement_end + 1;
                continue;
            }
        }
        let selector_prelude = &input[cursor..open];
        let selector_text_raw = selector_prelude.trim();
        let selector_offset = cursor
            + selector_prelude
                .len()
                .saturating_sub(selector_prelude.trim_start().len());
        let loc = location_from_offset(original_for_lines, selector_offset);
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
        if !selector_text.is_empty() && selector_text.starts_with('@') {
            // Preserve the useful qualified rules inside container at-rules.
            // `@media` conditions are attached to the nested rules so the
            // cascade can drop inactive sheets (kernel.org mobile CSS).
            // Other at-rules keep the inherited media query, if any.
            let nested_media = if selector_text_raw
                .get(..6)
                .is_some_and(|head| head.eq_ignore_ascii_case("@media"))
            {
                Some(merge_media(media, selector_text_raw[6..].trim()))
            } else {
                media.map(String::from)
            };
            parse_rule_block(
                body,
                parent,
                rules,
                original_for_lines,
                nested_media.as_deref(),
            );
        } else if !selector_text.is_empty() {
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
                    media: media.map(String::from),
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
                                media: media.map(String::from),
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
                        media,
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
            for ch in rest[start..].chars() {
                if ch == '\n' || ch == '\r' {
                    result.push(ch);
                } else {
                    for _ in 0..ch.len_utf8() {
                        result.push(' ');
                    }
                }
            }
            rest = "";
            break;
        };
        // Preserve byte offsets and line breaks so rule provenance remains
        // exact. The replacement spaces also keep comments as selector
        // whitespace (`#nav/**/ul` is a descendant selector).
        for ch in rest[start..start + 2 + end + 2].chars() {
            if ch == '\n' || ch == '\r' {
                result.push(ch);
            } else {
                for _ in 0..ch.len_utf8() {
                    result.push(' ');
                }
            }
        }
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
    let mut combinators = Vec::new();
    let mut token = String::new();
    let mut pending = None;
    let mut flush = |token: &mut String,
                     pending: &mut Option<Combinator>,
                     parts: &mut Vec<SimpleSelector>,
                     combinators: &mut Vec<Combinator>|
     -> Option<()> {
        if token.is_empty() {
            return Some(());
        }
        if !parts.is_empty() {
            combinators.push(pending.take().unwrap_or(Combinator::Descendant));
        } else {
            pending.take();
        }
        parts.push(parse_simple_selector(token)?);
        token.clear();
        Some(())
    };
    let mut saw_space = false;
    for byte in text.as_bytes().iter().copied() {
        match byte {
            b' ' | b'\t' | b'\r' | b'\n' => saw_space = true,
            b'>' => {
                flush(&mut token, &mut pending, &mut parts, &mut combinators)?;
                pending = Some(Combinator::Child);
                saw_space = false;
            }
            _ => {
                if saw_space && !token.is_empty() && pending.is_none() {
                    flush(&mut token, &mut pending, &mut parts, &mut combinators)?;
                }
                token.push(byte as char);
                saw_space = false;
            }
        }
    }
    flush(&mut token, &mut pending, &mut parts, &mut combinators)?;
    if combinators.len() + 1 != parts.len() {
        return None;
    }
    if parts.len() > MAX_DESCENDANT_DEPTH {
        return None;
    }
    (!parts.is_empty()).then(|| Selector {
        parts,
        text: String::from(text),
        original_text: original,
        combinators,
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
        if marker == b':' {
            if bytes.get(index + 1) == Some(&b':') {
                return None;
            }
            index += 1;
            let name_start = index;
            while bytes
                .get(index)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
            {
                index += 1;
            }
            let name = input[name_start..index].to_ascii_lowercase();
            let argument = if bytes.get(index) == Some(&b'(') {
                let close = matching_paren(input, index)?;
                let inner = input[index + 1..close].trim();
                index = close + 1;
                Some(inner)
            } else {
                None
            };
            match (name.as_str(), argument) {
                ("root", None) => out.root = true,
                ("hover", None) => out.hover = true,
                ("first-child", None) => out.first_child = true,
                ("last-child", None) => out.last_child = true,
                ("nth-child", Some(formula)) => out.nth_child = Some(parse_nth_formula(formula)?),
                _ => return None,
            }
            continue;
        }
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
    (out.universal
        || out.root
        || out.hover
        || out.first_child
        || out.last_child
        || out.nth_child.is_some()
        || out.tag_name.is_some()
        || out.id.is_some()
        || !out.classes.is_empty())
    .then_some(out)
}

fn matching_paren(input: &str, open: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0usize;
    for (offset, byte) in bytes.iter().enumerate().skip(open) {
        match *byte {
            b'(' => depth = depth.saturating_add(1),
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_nth_formula(input: &str) -> Option<(i32, i32)> {
    let value = input.trim().to_ascii_lowercase();
    match value.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }
    if let Some(n) = value.find('n') {
        let a = match value[..n].trim() {
            "" | "+" => 1,
            "-" => -1,
            other => other.parse().ok()?,
        };
        let b = match value[n + 1..].trim() {
            "" => 0,
            other => other.parse().ok()?,
        };
        Some((a, b))
    } else {
        Some((0, value.parse().ok()?))
    }
}

fn nth_matches(index: i32, a: i32, b: i32) -> bool {
    if a == 0 {
        return index == b;
    }
    let delta = index - b;
    if delta % a != 0 {
        return false;
    }
    let n = delta / a;
    n >= 0
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
        let name = name.trim();
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
    let tokens = css_value_tokens(raw);
    if !tokens.iter().any(|token| is_background_image_token(token)) {
        out.push(Declaration {
            property: Property::BackgroundImage,
            value: PropertyValue::Raw(String::from("none")),
            raw_value: String::from("none"),
            order,
            important,
        });
    }
    for token in tokens {
        let property = if looks_like_color_token(token) {
            Property::BackgroundColor
        } else if is_background_image_token(token) {
            Property::BackgroundImage
        } else if matches!(
            token.to_ascii_lowercase().as_str(),
            "repeat" | "no-repeat" | "repeat-x" | "repeat-y"
        ) {
            Property::BackgroundRepeat
        } else if matches!(
            token.to_ascii_lowercase().as_str(),
            "scroll" | "fixed" | "local"
        ) {
            Property::BackgroundAttachment
        } else if matches!(
            token.to_ascii_lowercase().as_str(),
            "contain" | "cover" | "auto"
        ) {
            Property::BackgroundSize
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

fn looks_like_color_token(token: &str) -> bool {
    parse_color(token).is_some()
        || token.eq_ignore_ascii_case("currentcolor")
        || token.starts_with("var(")
}

fn is_background_image_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    lower == "none"
        || lower.starts_with("url(")
        || lower.contains("gradient(")
        || lower.starts_with("var(") && lower.contains("url(")
}

/// Split a CSS value on top-level whitespace, keeping `url(...)` and
/// `linear-gradient(...)` as single tokens.
fn css_value_tokens(input: &str) -> Vec<&str> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut quote = None;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if let Some(delimiter) = quote {
            if byte == delimiter {
                quote = None;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' if depth > 0 || start.is_some() => quote = Some(byte),
            b'(' => {
                depth = depth.saturating_add(1);
                if start.is_none() {
                    start = Some(index);
                }
            }
            b')' => depth = depth.saturating_sub(1),
            b if b.is_ascii_whitespace() && depth == 0 => {
                if let Some(from) = start.take() {
                    tokens.push(&input[from..index]);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(index);
                }
            }
        }
    }
    if let Some(from) = start {
        tokens.push(&input[from..]);
    }
    tokens
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
                    output.push(make(Property::BorderColor, part, order));
                    for property in [
                        Property::BorderTopColor,
                        Property::BorderRightColor,
                        Property::BorderBottomColor,
                        Property::BorderLeftColor,
                    ] {
                        output.push(make(property, part, order));
                    }
                } else if let Some(length) = parse_length(part) {
                    let _ = length;
                    output.push(make(Property::BorderWidth, part, order));
                    for property in [
                        Property::BorderTopWidth,
                        Property::BorderRightWidth,
                        Property::BorderBottomWidth,
                        Property::BorderLeftWidth,
                    ] {
                        output.push(make(property, part, order));
                    }
                } else if is_border_style(part) {
                    output.push(make(Property::BorderStyle, part, order));
                    for property in [
                        Property::BorderTopStyle,
                        Property::BorderRightStyle,
                        Property::BorderBottomStyle,
                        Property::BorderLeftStyle,
                    ] {
                        output.push(make(property, part, order));
                    }
                }
            }
            if !raw
                .split_ascii_whitespace()
                .any(|part| parse_color(part).is_some())
            {
                for property in [
                    Property::BorderTopColor,
                    Property::BorderRightColor,
                    Property::BorderBottomColor,
                    Property::BorderLeftColor,
                ] {
                    output.push(make(property, "currentColor", order));
                }
            }
            output
        }
        Property::BorderColor => {
            let values = raw.split_ascii_whitespace().collect::<Vec<_>>();
            if values.is_empty() || values.len() > 4 {
                return Vec::new();
            }
            let (top, right, bottom, left) = match values.len() {
                1 => (values[0], values[0], values[0], values[0]),
                2 => (values[0], values[1], values[0], values[1]),
                3 => (values[0], values[1], values[2], values[1]),
                _ => (values[0], values[1], values[2], values[3]),
            };
            vec![
                make(Property::BorderColor, raw, order),
                make(Property::BorderTopColor, top, order),
                make(Property::BorderRightColor, right, order),
                make(Property::BorderBottomColor, bottom, order),
                make(Property::BorderLeftColor, left, order),
            ]
        }
        Property::BorderRadius => {
            let values = raw.split_ascii_whitespace().collect::<Vec<_>>();
            if values.is_empty() || values.len() > 4 || raw.contains('/') {
                return Vec::new();
            }
            let (tl, tr, br, bl) = match values.len() {
                1 => (values[0], values[0], values[0], values[0]),
                2 => (values[0], values[1], values[0], values[1]),
                3 => (values[0], values[1], values[2], values[1]),
                _ => (values[0], values[1], values[2], values[3]),
            };
            vec![
                make(Property::BorderRadius, raw, order),
                make(Property::BorderTopLeftRadius, tl, order),
                make(Property::BorderTopRightRadius, tr, order),
                make(Property::BorderBottomRightRadius, br, order),
                make(Property::BorderBottomLeftRadius, bl, order),
            ]
        }
        Property::BorderBottom => {
            let mut output = vec![make(Property::BorderBottom, raw, order)];
            // A var() may contain the complete shorthand, so derived longhand
            // candidates must be produced after substitution, not from the
            // unspecialized token stream.
            if raw.trim_start().starts_with("var(") {
                return output;
            }
            for part in raw.split_ascii_whitespace() {
                if parse_color(part).is_some()
                    || part.eq_ignore_ascii_case("currentcolor")
                    || part.starts_with("var(")
                {
                    output.push(make(Property::BorderColor, part, order));
                    output.push(make(Property::BorderBottomColor, part, order));
                } else if parse_length(part).is_some() {
                    output.push(make(Property::BorderWidth, part, order));
                    output.push(make(Property::BorderBottomWidth, part, order));
                } else if is_border_style(part) {
                    output.push(make(Property::BorderStyle, part, order));
                    output.push(make(Property::BorderBottomStyle, part, order));
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
        | Property::BackgroundSize
        | Property::TextShadow
        | Property::Opacity
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
        Property::BackgroundImage | Property::TextShadow | Property::BackgroundSize
    ) {
        return PropertyValue::Raw(String::from(value));
    }
    if property == &Property::Opacity {
        return parse_opacity(value).unwrap_or_else(|| PropertyValue::Raw(String::from(value)));
    }
    if matches!(
        property,
        Property::Color
            | Property::BackgroundColor
            | Property::BorderColor
            | Property::BorderTopColor
            | Property::BorderRightColor
            | Property::BorderBottomColor
            | Property::BorderLeftColor
    ) {
        if value.eq_ignore_ascii_case("currentcolor") && property != &Property::Color {
            return PropertyValue::CurrentColor;
        }
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
            | Property::MinWidth
            | Property::MaxWidth
            | Property::MinHeight
            | Property::FontSize
            | Property::LineHeight
            | Property::BorderWidth
            | Property::BorderTopWidth
            | Property::BorderRightWidth
            | Property::BorderBottomWidth
            | Property::BorderLeftWidth
            | Property::BorderRadius
            | Property::BorderTopLeftRadius
            | Property::BorderTopRightRadius
            | Property::BorderBottomRightRadius
            | Property::BorderBottomLeftRadius
            | Property::BorderSpacing
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
    for (suffix, scale, ctor) in [
        ("px", 1, 0u8),
        ("rem", 16, 0u8),
        ("em", 1000, 1u8),
        ("%", 100, 2u8),
    ] {
        if let Some(number) = normalized.strip_suffix(suffix).map(str::trim) {
            let scaled = parse_fixed(number, scale)?;
            return Some(match ctor {
                0 => PropertyValue::LengthPx(scaled),
                1 => PropertyValue::LengthEm(scaled),
                _ => PropertyValue::Percentage(scaled),
            });
        }
    }
    match normalized.as_str() {
        "thin" => Some(PropertyValue::LengthPx(1)),
        "medium" => Some(PropertyValue::LengthPx(3)),
        "thick" => Some(PropertyValue::LengthPx(5)),
        _ => None,
    }
}

fn parse_opacity(value: &str) -> Option<PropertyValue> {
    let trimmed = value.trim();
    if let Some(percent) = trimmed.strip_suffix('%') {
        let scaled = parse_fixed(percent.trim(), 100)?;
        return Some(PropertyValue::Percentage(scaled.clamp(0, 10_000)));
    }
    let scaled = parse_fixed(trimmed, 100)?;
    Some(PropertyValue::Percentage((scaled * 100).clamp(0, 10_000)))
}

fn parse_fixed(value: &str, scale: i32) -> Option<i32> {
    let negative = value.starts_with('-');
    let unsigned = value.trim_start_matches(['+', '-']);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let whole = if whole.is_empty() {
        0
    } else {
        whole.parse::<i32>().ok()?
    };
    let fraction_scale = 10i32.checked_pow(fraction.len().min(6) as u32)?;
    let fraction_value = if fraction.is_empty() {
        0
    } else {
        fraction[..fraction.len().min(6)].parse::<i32>().ok()?
    };
    let result = whole.checked_mul(scale)?.checked_add(
        fraction_value
            .checked_mul(scale)?
            .checked_div(fraction_scale)?,
    )?;
    Some(if negative { -result } else { result })
}

pub fn parse_color(value: &str) -> Option<Color> {
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
        "silver" => Some((192, 192, 192)),
        "purple" => Some((128, 0, 128)),
        "orange" => Some((255, 165, 0)),
        "navy" => Some((0, 0, 128)),
        "teal" => Some((0, 128, 128)),
        "maroon" => Some((128, 0, 0)),
        "olive" => Some((128, 128, 0)),
        "lime" => Some((0, 255, 0)),
        "aqua" | "cyan" => Some((0, 255, 255)),
        "fuchsia" | "magenta" => Some((255, 0, 255)),
        "gold" => Some((255, 215, 0)),
        "coral" => Some((255, 127, 80)),
        "brown" => Some((165, 42, 42)),
        "pink" => Some((255, 192, 203)),
        "indigo" => Some((75, 0, 130)),
        _ => None,
    };
    if let Some((r, g, b)) = named {
        return Some(Color::Rgb(r, g, b));
    }
    if let Some(hex) = lower.strip_prefix('#') {
        let bytes = hex.as_bytes();
        return match hex.len() {
            3 => Some(Color::Rgb(
                hex_digit(bytes[0])? * 17,
                hex_digit(bytes[1])? * 17,
                hex_digit(bytes[2])? * 17,
            )),
            4 => Some(Color::rgba(
                hex_digit(bytes[0])? * 17,
                hex_digit(bytes[1])? * 17,
                hex_digit(bytes[2])? * 17,
                hex_digit(bytes[3])? * 17,
            )),
            6 => Some(Color::Rgb(
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            )),
            8 => Some(Color::rgba(
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
                u8::from_str_radix(&hex[6..8], 16).ok()?,
            )),
            _ => None,
        };
    }
    if let Some(inner) = function_args(&lower, "rgb").or_else(|| function_args(&lower, "rgba")) {
        return parse_rgb_args(&inner);
    }
    if let Some(inner) = function_args(&lower, "hsl").or_else(|| function_args(&lower, "hsla")) {
        return parse_hsl_args(&inner);
    }
    None
}

fn function_args<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    value
        .strip_prefix(name)?
        .strip_prefix('(')?
        .strip_suffix(')')
}

fn parse_rgb_args(inner: &str) -> Option<Color> {
    let parts = split_color_args(inner);
    if parts.len() < 3 {
        return None;
    }
    let red = parse_rgb_channel(&parts[0])?;
    let green = parse_rgb_channel(&parts[1])?;
    let blue = parse_rgb_channel(&parts[2])?;
    let alpha = if let Some(alpha) = parts.get(3) {
        parse_alpha_component(alpha)?
    } else {
        255
    };
    Some(Color::rgba(red, green, blue, alpha))
}

fn parse_hsl_args(inner: &str) -> Option<Color> {
    let parts = split_color_args(inner);
    if parts.len() < 3 {
        return None;
    }
    let hue = parse_fixed(parts[0].trim_end_matches("deg"), 1)?;
    let sat = parse_percent_component(&parts[1])?;
    let light = parse_percent_component(&parts[2])?;
    let alpha = if let Some(alpha) = parts.get(3) {
        parse_alpha_component(alpha)?
    } else {
        255
    };
    let (red, green, blue) = hsl_to_rgb(hue.rem_euclid(360), sat, light);
    Some(Color::rgba(red, green, blue, alpha))
}

fn split_color_args(inner: &str) -> Vec<String> {
    let normalized = inner.replace('/', " ");
    if normalized.contains(',') {
        normalized
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(String::from)
            .collect()
    } else {
        normalized
            .split_ascii_whitespace()
            .filter(|part| !part.is_empty())
            .map(String::from)
            .collect()
    }
}

fn parse_rgb_channel(value: &str) -> Option<u8> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        let scaled = parse_fixed(percent.trim(), 100)?.clamp(0, 10_000);
        return Some(((scaled * 255) / 10_000) as u8);
    }
    value.parse::<i32>().ok().map(|v| v.clamp(0, 255) as u8)
}

fn parse_percent_component(value: &str) -> Option<i32> {
    let value = value.trim().strip_suffix('%').unwrap_or(value.trim());
    Some(parse_fixed(value, 100)?.clamp(0, 10_000))
}

fn parse_alpha_component(value: &str) -> Option<u8> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        let scaled = parse_fixed(percent.trim(), 100)?.clamp(0, 10_000);
        return Some(((scaled * 255) / 10_000) as u8);
    }
    let scaled = parse_fixed(value, 1000)?.clamp(0, 1000);
    Some(((scaled * 255) / 1000) as u8)
}

fn hsl_to_rgb(hue: i32, sat: i32, light: i32) -> (u8, u8, u8) {
    let s = sat;
    let l = light;
    let c = (10_000 - (2 * l - 10_000).unsigned_abs() as i32) * s / 10_000;
    let h_sector = (hue * 10) % 3600;
    let x = c * (600 - ((h_sector % 1200) - 600).unsigned_abs() as i32) / 600;
    let m = l - c / 2;
    let (r1, g1, b1) = match hue / 60 {
        0 => (c, x, 0),
        1 => (x, c, 0),
        2 => (0, c, x),
        3 => (0, x, c),
        4 => (x, 0, c),
        _ => (c, 0, x),
    };
    let channel = |value: i32| ((value + m) * 255 / 10_000).clamp(0, 255) as u8;
    (channel(r1), channel(g1), channel(b1))
}

/// Parses `linear-gradient(...)` / `radial-gradient(...)` from a background-image value.
pub fn parse_gradient(value: &str) -> Option<CssGradient> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let (kind, rest) = if let Some(rest) = strip_gradient_prefix(&lower, "linear-gradient") {
        (GradientKind::Linear, rest)
    } else if let Some(rest) = strip_gradient_prefix(&lower, "repeating-linear-gradient") {
        (GradientKind::Linear, rest)
    } else if let Some(rest) = strip_gradient_prefix(&lower, "radial-gradient") {
        (GradientKind::Radial, rest)
    } else if let Some(rest) = strip_gradient_prefix(&lower, "repeating-radial-gradient") {
        (GradientKind::Radial, rest)
    } else {
        return None;
    };
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?.trim();
    if inner.is_empty() {
        return None;
    }
    let args = split_css_comma_list(inner);
    if args.is_empty() {
        return None;
    }
    let mut angle_deg = 180i32;
    let mut start = 0usize;
    if kind == GradientKind::Linear {
        if let Some(angle) = parse_gradient_angle(args[0]) {
            angle_deg = angle;
            start = 1;
        }
    } else if looks_like_radial_size(args[0]) {
        start = 1;
    }
    let mut stops = Vec::new();
    for arg in args.iter().skip(start) {
        if stops.len() >= MAX_GRADIENT_STOPS {
            break;
        }
        if let Some(stop) = parse_gradient_stop(arg) {
            stops.push(stop);
        }
    }
    if stops.len() < 2 {
        return None;
    }
    normalize_gradient_stops(&mut stops);
    Some(CssGradient {
        kind,
        angle_deg,
        stops,
    })
}

fn strip_gradient_prefix<'a>(lower: &'a str, name: &str) -> Option<&'a str> {
    let rest = if let Some(rest) = lower.strip_prefix("-webkit-") {
        rest
    } else if let Some(rest) = lower.strip_prefix("-moz-") {
        rest
    } else {
        lower
    };
    rest.strip_prefix(name)
}

fn split_css_comma_list(input: &str) -> Vec<&str> {
    let bytes = input.as_bytes();
    let mut items = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for (index, byte) in bytes.iter().copied().enumerate() {
        match byte {
            b'(' => depth = depth.saturating_add(1),
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                let part = input[start..index].trim();
                if !part.is_empty() {
                    items.push(part);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let part = input[start..].trim();
    if !part.is_empty() {
        items.push(part);
    }
    items
}

fn parse_gradient_angle(token: &str) -> Option<i32> {
    let token = token.trim().to_ascii_lowercase();
    if let Some(number) = token.strip_suffix("deg") {
        return parse_fixed(number.trim(), 1);
    }
    if let Some(number) = token.strip_suffix("turn") {
        let thousandths = parse_fixed(number.trim(), 1000)?;
        return Some(((thousandths * 360) / 1000) % 360);
    }
    if let Some(rest) = token.strip_prefix("to ") {
        return Some(match rest.trim() {
            "top" => 0,
            "right" => 90,
            "bottom" => 180,
            "left" => 270,
            "top right" | "right top" => 45,
            "bottom right" | "right bottom" => 135,
            "bottom left" | "left bottom" => 225,
            "top left" | "left top" => 315,
            _ => return None,
        });
    }
    None
}

fn looks_like_radial_size(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    lower.contains("circle")
        || lower.contains("ellipse")
        || lower.contains("closest-")
        || lower.contains("farthest-")
        || lower.starts_with("at ")
}

fn parse_gradient_stop(token: &str) -> Option<GradientStop> {
    let token = token.trim();
    let (color_part, position) = if let Some((color, rest)) = split_color_and_position(token) {
        (color, parse_stop_position(rest))
    } else {
        (token, None)
    };
    let color = if color_part.eq_ignore_ascii_case("currentcolor") {
        Color::Rgb(0, 0, 0)
    } else {
        parse_color(color_part)?
    };
    Some(GradientStop {
        color,
        position: position.unwrap_or(-1),
    })
}

fn split_color_and_position(token: &str) -> Option<(&str, &str)> {
    if token.starts_with('#') {
        let split = token.find(char::is_whitespace)?;
        return Some((token[..split].trim(), token[split..].trim()));
    }
    if let Some(close) = token.find(')') {
        let rest = token[close + 1..].trim();
        if rest.is_empty() {
            return None;
        }
        return Some((token[..=close].trim(), rest));
    }
    token.split_once(char::is_whitespace)
}

fn parse_stop_position(value: &str) -> Option<i32> {
    let first = value.split_ascii_whitespace().next()?;
    match parse_length(first)? {
        PropertyValue::Percentage(value) => Some(value.clamp(0, 10_000)),
        PropertyValue::LengthPx(value) => Some((value * 100).clamp(0, 10_000)),
        _ => None,
    }
}

fn normalize_gradient_stops(stops: &mut [GradientStop]) {
    if stops.is_empty() {
        return;
    }
    if stops[0].position < 0 {
        stops[0].position = 0;
    }
    let last = stops.len() - 1;
    if stops[last].position < 0 {
        stops[last].position = 10_000;
    }
    let mut index = 0usize;
    while index < stops.len() {
        if stops[index].position >= 0 {
            index += 1;
            continue;
        }
        let start = index;
        while index < stops.len() && stops[index].position < 0 {
            index += 1;
        }
        let before = stops[start - 1].position;
        let after = stops.get(index).map(|stop| stop.position).unwrap_or(10_000);
        let span = (index - start + 1) as i32;
        for (offset, stop) in stops[start..index].iter_mut().enumerate() {
            stop.position = before + (after - before) * (offset as i32 + 1) / span;
        }
    }
    let mut previous = 0;
    for stop in stops.iter_mut() {
        if stop.position < previous {
            stop.position = previous;
        }
        previous = stop.position;
    }
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
        html, body, div, header, main, section, article, aside, nav, footer, address, p, pre, dl, dt, dd, blockquote, hr { display: block; }
        span, a, strong, b, em, i, code, small, mark, del, ins, sub, sup, br { display: inline; }
        img { display: inline-block; }
        table { display: table; border-collapse: separate; border-spacing: 2px; }
        thead { display: table-header-group; }
        tbody { display: table-row-group; }
        tfoot { display: table-footer-group; }
        tr { display: table-row; }
        th, td { display: table-cell; padding: 1px; }
        th { font-weight: bold; text-align: center; }
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
        Self::build_with_viewport(document, document_stylesheets, DEFAULT_VIEWPORT_WIDTH)
    }

    pub fn build_with_viewport(
        document: &Document,
        document_stylesheets: &[Stylesheet],
        viewport_width: u32,
    ) -> Self {
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
                let computed = compute_element_style(
                    document,
                    node_id,
                    parent_style.as_ref(),
                    &stylesheets,
                    viewport_width,
                );
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
    mut linked: Vec<Option<Vec<Stylesheet>>>,
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
                if let Some(Some(sheets)) = linked.get_mut(linked_index) {
                    ordered.extend(sheets.iter().cloned());
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
    for index in (0..selector.parts.len().saturating_sub(1)).rev() {
        let part = &selector.parts[index];
        let combinator = selector
            .combinators
            .get(index)
            .copied()
            .unwrap_or(Combinator::Descendant);
        match combinator {
            Combinator::Child => {
                let Some(parent) = document.parent(current) else {
                    return false;
                };
                if !simple_selector_matches(document, parent, part) {
                    return false;
                }
                current = parent;
            }
            Combinator::Descendant => {
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
                if found.is_none() {
                    return false;
                }
            }
        }
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
    if selector.hover {
        // Static style computation has no pointer state.  Keep the rule in
        // the stylesheet for DevTools and let the interactive state layer
        // opt into it when it has a hovered node.
        return false;
    }
    if selector.root
        && !document
            .parent(node_id)
            .is_some_and(|parent| matches!(document.get(parent), Some(Node::Document { .. })))
    {
        return false;
    }
    if selector.first_child || selector.last_child || selector.nth_child.is_some() {
        let Some((index, count)) = document.element_sibling_index(node_id) else {
            return false;
        };
        if selector.first_child && index != 1 {
            return false;
        }
        if selector.last_child && index != count {
            return false;
        }
        if let Some((a, b)) = selector.nth_child {
            if !nth_matches(index as i32, a, b) {
                return false;
            }
        }
    }
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
    viewport_width: u32,
) -> ComputedStyle {
    let mut computed = initial_style(parent);
    let mut winners: Vec<Option<(CascadeKey, MatchedDeclaration)>> =
        vec![None; PROPERTY_ORDER.len()];
    let mut custom_winners: Vec<(String, CascadeKey, MatchedDeclaration)> = Vec::new();
    let mut all_candidates: Vec<MatchedDeclaration> = Vec::new();
    let mut source_order = 0usize;
    for (sheet_index, sheet) in sheets.iter().enumerate() {
        for rule in &sheet.rules {
            if let Some(media) = &rule.media {
                if !import_media_active(media, viewport_width).0 {
                    continue;
                }
            }
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
                            location: rule.location,
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
                        location: rule.location,
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
                        location: None,
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
                    location: None,
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
    // Install declarations from this element before resolving ordinary
    // properties.  Custom properties are inherited, but declarations on the
    // consuming element are also visible to var() in that same element.
    for (name, _, matched) in &custom_winners {
        if let Some(existing) = computed
            .custom_properties
            .iter_mut()
            .find(|entry| entry.0 == *name)
        {
            *existing = (name.clone(), matched.value.clone(), Some(matched.clone()));
        } else {
            computed.custom_properties.push((
                name.clone(),
                matched.value.clone(),
                Some(matched.clone()),
            ));
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
            if matched.property == Property::BorderBottom && !expanded.is_empty() {
                // A custom property can contain the complete border value;
                // expand it only after var() substitution so `5px solid
                // var(--color)` follows the normal border grammar.
                for token in expanded.split_ascii_whitespace() {
                    let (property, value) = if parse_color(token).is_some() {
                        (
                            Property::BorderColor,
                            parse_value(&Property::BorderColor, token),
                        )
                    } else if parse_length(token).is_some() {
                        (
                            Property::BorderWidth,
                            parse_value(&Property::BorderWidth, token),
                        )
                    } else if is_border_style(token) {
                        (
                            Property::BorderStyle,
                            parse_value(&Property::BorderStyle, token),
                        )
                    } else {
                        continue;
                    };
                    if let Some(derived) = computed
                        .properties
                        .iter_mut()
                        .find(|entry| entry.property == property)
                    {
                        if derived.matched.is_none()
                            || derived.matched.as_ref().is_some_and(|m| m.inherited)
                        {
                            derived.value = value;
                            derived.matched = Some(matched.clone());
                        }
                    }
                }
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
    resolve_element_relative_values(&mut computed, parent);
    computed.matched_declarations = all_candidates;
    computed
}

#[cfg(feature = "dom")]
fn resolve_element_relative_values(style: &mut ComputedStyle, parent: Option<&ComputedStyle>) {
    let parent_font = parent
        .and_then(|parent| parent.value(&Property::FontSize))
        .and_then(|value| match value {
            PropertyValue::LengthPx(px) => Some(*px),
            _ => None,
        })
        .unwrap_or(16)
        .max(1);
    let font_index = property_index(&Property::FontSize).expect("font-size property");
    let font_size = match style.properties[font_index].value {
        PropertyValue::LengthEm(em) => parent_font.saturating_mul(em) / 1000,
        PropertyValue::Percentage(percent) => parent_font.saturating_mul(percent) / 10_000,
        PropertyValue::LengthPx(px) => px,
        _ => parent_font,
    }
    .clamp(0, 4096);
    style.properties[font_index].value = PropertyValue::LengthPx(font_size);

    let current_color = style
        .value(&Property::Color)
        .cloned()
        .unwrap_or(PropertyValue::Color(Color::Rgb(0, 0, 0)));
    for entry in &mut style.properties {
        if entry.property != Property::FontSize {
            if let PropertyValue::LengthEm(em) = entry.value {
                entry.value = PropertyValue::LengthPx(font_size.saturating_mul(em) / 1000);
            } else if entry.property == Property::LineHeight {
                if let PropertyValue::Percentage(percent) = entry.value {
                    entry.value =
                        PropertyValue::LengthPx(font_size.saturating_mul(percent) / 10_000);
                }
            }
        }
        if matches!(entry.value, PropertyValue::CurrentColor) {
            entry.value = current_color.clone();
        }
    }
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
            .saturating_add(part.classes.len() as u16)
            .saturating_add(
                (part.root
                    || part.hover
                    || part.first_child
                    || part.last_child
                    || part.nth_child.is_some()) as u16,
            );
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
                    location: None,
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
        Property::Color => PropertyValue::Color(Color::Rgb(0, 0, 0)),
        Property::BorderColor
        | Property::BorderTopColor
        | Property::BorderRightColor
        | Property::BorderBottomColor
        | Property::BorderLeftColor => PropertyValue::CurrentColor,
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
        Property::MaxWidth => PropertyValue::Keyword(String::from("none")),
        Property::MarginTop
        | Property::MarginRight
        | Property::MarginBottom
        | Property::MarginLeft
        | Property::PaddingTop
        | Property::PaddingRight
        | Property::PaddingBottom
        | Property::PaddingLeft
        | Property::MinWidth
        | Property::BorderWidth
        | Property::BorderTopWidth
        | Property::BorderRightWidth
        | Property::BorderBottomWidth
        | Property::BorderLeftWidth
        | Property::BorderRadius
        | Property::BorderTopLeftRadius
        | Property::BorderTopRightRadius
        | Property::BorderBottomRightRadius
        | Property::BorderBottomLeftRadius
        | Property::BorderSpacing => PropertyValue::LengthPx(0),
        Property::BorderStyle
        | Property::BorderTopStyle
        | Property::BorderRightStyle
        | Property::BorderBottomStyle
        | Property::BorderLeftStyle => PropertyValue::Keyword(String::from("none")),
        Property::BackgroundImage => PropertyValue::Keyword(String::from("none")),
        Property::BackgroundRepeat => PropertyValue::Keyword(String::from("repeat")),
        Property::BackgroundAttachment => PropertyValue::Keyword(String::from("scroll")),
        Property::BackgroundPositionX => PropertyValue::Keyword(String::from("0%")),
        Property::BackgroundPositionY => PropertyValue::Keyword(String::from("0%")),
        Property::BackgroundSize => PropertyValue::Keyword(String::from("auto")),
        Property::TextShadow => PropertyValue::Keyword(String::from("none")),
        Property::Opacity => PropertyValue::Percentage(10_000),
        Property::BoxSizing => PropertyValue::Keyword(String::from("content-box")),
        Property::BoxShadow => PropertyValue::Keyword(String::from("none")),
        Property::Float | Property::Clear => PropertyValue::Keyword(String::from("none")),
        Property::BorderCollapse => PropertyValue::Keyword(String::from("separate")),
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
        assert_eq!(
            parse_color("rgba(0, 0, 0, 0.2)"),
            Some(Color::Rgba(0, 0, 0, 51))
        );
        assert_eq!(parse_color("#1234"), Some(Color::Rgba(17, 34, 51, 68)));
        assert_eq!(parse_length("12px"), Some(PropertyValue::LengthPx(12)));
        assert_eq!(parse_length("2rem"), Some(PropertyValue::LengthPx(32)));
        let gradient = parse_gradient("linear-gradient(to right, #333, #eee 75%)").unwrap();
        assert_eq!(gradient.kind, GradientKind::Linear);
        assert_eq!(gradient.angle_deg, 90);
        assert_eq!(gradient.stops.len(), 2);
        assert_eq!(gradient.stops[0].color, Color::Rgb(0x33, 0x33, 0x33));
        assert_eq!(gradient.stops[1].position, 7500);
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
        let linked = vec![Some(vec![parse_stylesheet(
            "p { color: red; }",
            StylesheetSource::External(String::from("a.css")),
        )])];
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

    #[cfg(feature = "dom")]
    #[test]
    fn arch_nested_fixture_reaches_computed_style() {
        let document = parse_html(include_str!("../tests/fixtures/arch-navbar.html")).unwrap();
        let context = StyleContext::build(&document, &collect_embedded_stylesheets(&document));
        let by_id = |id: &str| {
            (0..document.node_count()).find(|node_id| {
                document
                    .get(*node_id)
                    .and_then(|node| match node {
                        golden_fish::Node::Element { attributes, .. } => {
                            attribute_value(attributes, "id")
                        }
                        _ => None,
                    })
                    .is_some_and(|value| value == id)
            })
        };
        let navbar = by_id("archnavbar").unwrap();
        let list = by_id("archnavbarlist").unwrap();
        let first_li = (0..document.node_count())
            .find(|node_id| document.tag_name(*node_id) == Some("li"))
            .unwrap();
        let navbar_style = context.style_for(navbar).unwrap();
        let list_style = context.style_for(list).unwrap();
        assert_eq!(
            navbar_style.value(&Property::BackgroundColor),
            Some(&PropertyValue::Color(Color::Rgb(51, 51, 51)))
        );
        assert_eq!(
            navbar_style.value(&Property::BorderWidth),
            Some(&PropertyValue::LengthPx(5))
        );
        assert_eq!(
            navbar_style.value(&Property::BorderStyle),
            Some(&PropertyValue::Keyword(String::from("solid")))
        );
        assert_eq!(
            navbar_style.value(&Property::BorderColor),
            Some(&PropertyValue::Color(Color::Rgb(23, 147, 209)))
        );
        assert_eq!(
            navbar_style.value(&Property::MinHeight),
            Some(&PropertyValue::LengthPx(40))
        );
        assert_eq!(
            list_style.value(&Property::Display),
            Some(&PropertyValue::Keyword(String::from("block")))
        );
        assert_eq!(
            list_style.value(&Property::ListStyleType),
            Some(&PropertyValue::Keyword(String::from("none")))
        );
        assert_eq!(
            list_style.value(&Property::TextAlign),
            Some(&PropertyValue::Keyword(String::from("right")))
        );
        assert_eq!(
            list_style.value(&Property::FontSize),
            Some(&PropertyValue::LengthPx(0))
        );
        assert_eq!(
            context
                .style_for(first_li)
                .unwrap()
                .value(&Property::Display),
            Some(&PropertyValue::Keyword(String::from("inline-block")))
        );
        assert_eq!(
            context
                .style_for(first_li)
                .unwrap()
                .value(&Property::FontSize),
            Some(&PropertyValue::LengthPx(14))
        );
    }

    #[cfg(feature = "dom")]
    #[test]
    fn real_arch_navbar_stylesheet_survives_import_and_nested_rules() {
        let sheet = parse_stylesheet(
            include_str!("../tests/fixtures/arch-navbar-live.css"),
            StylesheetSource::External(String::from(
                "https://archlinux.org/static/archlinux_common_style/navbar.css",
            )),
        );
        assert!(sheet.rules.iter().any(|rule| {
            rule.selectors
                .iter()
                .any(|selector| selector.text == "#archnavbar")
        }));
        assert!(sheet.rules.iter().any(|rule| {
            rule.selectors
                .iter()
                .any(|selector| selector.text == "#archnavbar ul")
        }));
        assert!(sheet.rules.iter().any(|rule| {
            rule.selectors
                .iter()
                .any(|selector| selector.text == "#archnavbar ul li")
        }));
        assert!(sheet.rules.iter().any(|rule| {
            rule.selectors
                .iter()
                .any(|selector| selector.text == "#archnavbar #logo a")
        }));
        assert!(sheet
            .rules
            .iter()
            .flat_map(|rule| rule.selectors.iter())
            .all(|selector| !selector.text.starts_with('@')));

        let document = parse_html(include_str!("../tests/fixtures/arch-navbar.html")).unwrap();
        let context = StyleContext::build(&document, &[sheet]);
        let list = document.find_first_element("ul").unwrap();
        let item = document.find_first_element("li").unwrap();
        let anchor = document.find_first_element("a").unwrap();
        assert_eq!(
            context
                .style_for(list)
                .unwrap()
                .value(&Property::ListStyleType),
            Some(&PropertyValue::Keyword(String::from("none")))
        );
        assert_eq!(
            context.style_for(list).unwrap().value(&Property::TextAlign),
            Some(&PropertyValue::Keyword(String::from("right")))
        );
        assert_eq!(
            context.style_for(item).unwrap().value(&Property::Display),
            Some(&PropertyValue::Keyword(String::from("inline-block")))
        );
        assert_eq!(
            context.style_for(anchor).unwrap().value(&Property::Display),
            Some(&PropertyValue::Keyword(String::from("block")))
        );
    }

    #[cfg(feature = "dom")]
    #[test]
    fn selector_combinators_and_root_are_not_collapsed_to_descendants() {
        let (document, _) = styles(
            "<html><body><div id='outer'><section><ul><li>x</li></ul></section></div></body></html>",
            "",
        );
        let ul = document.find_first_element("ul").unwrap();
        assert!(selector_matches(
            &document,
            ul,
            &parse_selector("#outer ul").unwrap()
        ));
        assert!(!selector_matches(
            &document,
            ul,
            &parse_selector("#outer > ul").unwrap()
        ));
        assert!(selector_matches(
            &document,
            ul,
            &parse_selector("section > ul").unwrap()
        ));
        let html = document.find_first_element("html").unwrap();
        assert!(selector_matches(
            &document,
            html,
            &parse_selector(":root").unwrap()
        ));
    }

    #[cfg(feature = "dom")]
    #[test]
    fn selector_harness_covers_real_navbar_ancestry_and_exact_classes() {
        let document = parse_html(
            "<div id='archnavbar'><div id='archnavbarmenu'><ul id='archnavbarlist'><li id='anb-home' class='nav active'><a id='home-link'>Home</a></li></ul></div></div>",
        )
        .unwrap();
        let node = |id: &str| {
            (0..document.node_count())
                .find(|node_id| {
                    document
                        .get(*node_id)
                        .and_then(|node| match node {
                            Node::Element { attributes, .. } => attribute_value(attributes, "id"),
                            _ => None,
                        })
                        .is_some_and(|value| value == id)
                })
                .unwrap()
        };
        let assert_matches = |selector: &str, id: &str| {
            let cleaned = strip_comments(selector);
            let parsed = parse_selector(&cleaned)
                .unwrap_or_else(|| panic!("selector parser rejected {selector}"));
            assert!(
                selector_matches(&document, node(id), &parsed),
                "selector {selector} did not match {id}"
            );
        };
        let assert_not_matches = |selector: &str, id: &str| {
            let cleaned = strip_comments(selector);
            let parsed = parse_selector(&cleaned)
                .unwrap_or_else(|| panic!("selector parser rejected {selector}"));
            assert!(
                !selector_matches(&document, node(id), &parsed),
                "selector {selector} unexpectedly matched {id}"
            );
        };

        for selector in [
            "#archnavbar",
            "#archnavbarmenu",
            "#archnavbarlist",
            "#anb-home",
        ] {
            assert_matches(selector, selector.trim_start_matches('#'));
        }
        for selector in [
            "#archnavbar ul",
            "#archnavbar #archnavbarmenu ul",
            "#archnavbar div ul",
            "#archnavbar ul li",
            "#archnavbar ul li a",
            "#archnavbarmenu > ul",
            "ul#archnavbarlist",
            "li#anb-home",
            "#archnavbarlist > li#anb-home",
        ] {
            let target = if selector.ends_with(" a") {
                "home-link"
            } else if selector.starts_with("ul")
                || selector.ends_with(" ul")
                || selector.ends_with("> ul")
            {
                "archnavbarlist"
            } else {
                "anb-home"
            };
            assert_matches(selector, target);
        }
        assert_not_matches("#archnavbar > ul", "archnavbarlist");
        assert_matches(".nav", "anb-home");
        assert_matches(".active", "anb-home");
        assert_not_matches(".act", "anb-home");
        assert_matches("#archnavbar/**/ul", "archnavbarlist");
        assert_matches("#archnavbar > div", "archnavbarmenu");
    }

    #[test]
    fn parser_keeps_declarations_around_nested_rules_and_container_at_rules() {
        let sheet = parse_stylesheet(
            "@media (min-width: 600px) { #archnavbar { color: red; & ul { list-style: none; } padding: 4px; } }",
            StylesheetSource::Embedded,
        );
        assert!(sheet.rules.iter().any(|rule| {
            rule.selectors
                .iter()
                .any(|selector| selector.text == "#archnavbar")
                && rule
                    .declarations
                    .iter()
                    .any(|decl| decl.property == Property::Color)
        }));
        assert!(sheet.rules.iter().any(|rule| {
            rule.selectors
                .iter()
                .any(|selector| selector.text == "#archnavbar ul")
                && rule
                    .declarations
                    .iter()
                    .any(|decl| decl.property == Property::ListStyleType)
        }));
        assert!(sheet.rules.iter().any(|rule| {
            rule.selectors
                .iter()
                .any(|selector| selector.text == "#archnavbar")
                && rule
                    .declarations
                    .iter()
                    .any(|decl| decl.property == Property::PaddingTop)
        }));
    }

    #[test]
    fn classic_lengths_borders_radii_and_shadow_compute() {
        let document = parse_html("<style>body{font-size:20px;color:#123456} #x{font-size:.5em;width:50em;margin:1em auto;padding:.4em;border:thin solid;border-color:#dddadf #ccc8b8 #bbb59f;border-bottom-left-radius:.5em;box-shadow:-.1em -.3em .5em currentColor inset}</style><body><div id=x>x</div></body>").unwrap();
        let context = StyleContext::build(&document, &collect_embedded_stylesheets(&document));
        let node = document.find_first_element("div").unwrap();
        let style = context.style_for(node).unwrap();
        assert_eq!(
            style.value(&Property::FontSize),
            Some(&PropertyValue::LengthPx(10))
        );
        assert_eq!(
            style.value(&Property::Width),
            Some(&PropertyValue::LengthPx(500))
        );
        assert_eq!(
            style.value(&Property::MarginTop),
            Some(&PropertyValue::LengthPx(10))
        );
        assert_eq!(
            style.value(&Property::MarginRight),
            Some(&PropertyValue::Auto)
        );
        assert_eq!(
            style.value(&Property::PaddingLeft),
            Some(&PropertyValue::LengthPx(4))
        );
        assert_eq!(
            style.value(&Property::BorderTopWidth),
            Some(&PropertyValue::LengthPx(1))
        );
        assert_eq!(
            style.value(&Property::BorderTopColor),
            Some(&PropertyValue::Color(Color::Rgb(0xdd, 0xda, 0xdf)))
        );
        assert_eq!(
            style.value(&Property::BorderRightColor),
            Some(&PropertyValue::Color(Color::Rgb(0xcc, 0xc8, 0xb8)))
        );
        assert_eq!(
            style.value(&Property::BorderBottomColor),
            Some(&PropertyValue::Color(Color::Rgb(0xbb, 0xb5, 0x9f)))
        );
        assert_eq!(
            style.value(&Property::BorderBottomLeftRadius),
            Some(&PropertyValue::LengthPx(5))
        );
        assert!(
            matches!(style.value(&Property::BoxShadow), Some(PropertyValue::Keyword(value)) if value.contains("inset"))
        );
    }

    #[test]
    fn border_side_longhand_obeys_source_order_around_shorthand() {
        let document = parse_html("<style>#a{border-top-color:red;border-color:#111 #222 #333} #b{border-color:#111 #222 #333;border-top-color:red}</style><div id=a></div><div id=b></div>").unwrap();
        let context = StyleContext::build(&document, &collect_embedded_stylesheets(&document));
        let divs = (0..document.node_count())
            .filter(|id| document.tag_name(*id) == Some("div"))
            .collect::<Vec<_>>();
        assert_eq!(
            context
                .style_for(divs[0])
                .unwrap()
                .value(&Property::BorderTopColor),
            Some(&PropertyValue::Color(Color::Rgb(0x11, 0x11, 0x11)))
        );
        assert_eq!(
            context
                .style_for(divs[1])
                .unwrap()
                .value(&Property::BorderTopColor),
            Some(&PropertyValue::Color(Color::Rgb(255, 0, 0)))
        );
    }

    #[test]
    fn parses_leading_import_forms_and_stops_after_qualified_rule() {
        let imports = parse_leading_imports(
            r#"@charset "utf-8";
               @layer reset;
               @import "base.css";
               @import url("cards.css") screen;
               @import url(layout/narrow.css) (max-width: 600px);
               #banner { color: black; }
               @import "too-late.css";"#,
        );
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].raw_url, "base.css");
        assert_eq!(imports[1].raw_url, "cards.css");
        assert_eq!(imports[1].media, "screen");
        assert_eq!(imports[2].raw_url, "layout/narrow.css");
        assert_eq!(imports[2].media, "(max-width: 600px)");
    }

    #[test]
    fn import_media_uses_viewport_and_keeps_unknown_conditions_observable() {
        assert_eq!(import_media_active("", 800), (true, None));
        assert_eq!(import_media_active("screen", 800), (true, None));
        assert_eq!(import_media_active("(min-width: 600px)", 800), (true, None));
        assert_eq!(
            import_media_active("(min-width: 600px)", 500),
            (false, None)
        );
        assert_eq!(import_media_active("(max-width: 600px)", 500), (true, None));
        assert!(!import_media_active("speech", 800).1.unwrap().is_empty());
    }

    #[cfg(feature = "dom")]
    #[test]
    fn kernel_main_top_level_rules_survive_import_and_font_face() {
        let css = r#"/** live stylesheet structure */
            @import "normalize.css";
            @font-face { font-family: oxygen; src: url('../fonts/oxygen.woff2'); }
            #banner {
                text-align: center; background: #fff; margin: 0em auto;
                padding: 1em; width: 50em; border: thin solid;
                border-color: #dddadf #ccc8b8 #bbb59f;
                box-shadow: 0 0.1em 0.3em #ccc8b8;
                border-bottom-right-radius: 0.5em;
                border-bottom-left-radius: 0.5em;
            }
            #latest { float:right; background:#ffd133; color:#4c3d00; }
            #releases { clear:both; width:100%; margin-bottom:.25em; }"#;
        let sheet = parse_stylesheet(
            css,
            StylesheetSource::External(String::from("https://www.kernel.org/theme/css/main.css")),
        );
        for expected in ["#banner", "#latest", "#releases"] {
            assert!(sheet.rules.iter().any(|rule| rule
                .selectors
                .iter()
                .any(|selector| selector.text == expected)));
        }
        let document = parse_html(
            "<header id=banner><h1>x</h1><nav></nav></header><aside id=featured><table id=latest></table><table id=releases></table></aside>",
        )
        .unwrap();
        let context = StyleContext::build(&document, &[sheet]);
        let banner = (0..document.node_count())
            .find(|id| document.tag_name(*id) == Some("header"))
            .unwrap();
        let latest = (0..document.node_count())
            .find(|id| {
                document.tag_name(*id) == Some("table")
                    && document.get(*id).and_then(|node| match node {
                        Node::Element { attributes, .. } => attribute_value(attributes, "id"),
                        _ => None,
                    }) == Some("latest")
            })
            .unwrap();
        assert_eq!(
            context
                .style_for(banner)
                .unwrap()
                .value(&Property::BackgroundColor),
            Some(&PropertyValue::Color(Color::Rgb(255, 255, 255)))
        );
        assert_eq!(
            context.style_for(latest).unwrap().value(&Property::Float),
            Some(&PropertyValue::Keyword(String::from("right")))
        );
        let banner_rule = context
            .style_for(banner)
            .unwrap()
            .matched_declarations
            .iter()
            .find(|matched| matched.selector == "#banner")
            .unwrap();
        assert_eq!(banner_rule.location.unwrap().line, 4);
        assert!(banner_rule.source.contains("kernel.org/theme/css/main.css"));
    }

    #[cfg(feature = "dom")]
    #[test]
    fn imported_bundle_precedes_parent_override_and_preserves_source() {
        let document =
            parse_html("<link rel=stylesheet href=main.css><header id=banner></header>").unwrap();
        let imported = parse_stylesheet(
            "#banner { background: white; color: blue; }",
            StylesheetSource::External(String::from("https://example.test/card.css")),
        );
        let parent = parse_stylesheet(
            "@import 'card.css'; #banner { color: black; }",
            StylesheetSource::External(String::from("https://example.test/main.css")),
        );
        let ordered =
            order_document_stylesheets(&document, Vec::new(), vec![Some(vec![imported, parent])]);
        let context = StyleContext::build(&document, &ordered);
        let banner = document.find_first_element("header").unwrap();
        let style = context.style_for(banner).unwrap();
        assert_eq!(
            style.value(&Property::BackgroundColor),
            Some(&PropertyValue::Color(Color::Rgb(255, 255, 255)))
        );
        assert_eq!(
            style.value(&Property::Color),
            Some(&PropertyValue::Color(Color::Rgb(0, 0, 0)))
        );
        assert!(style.matched_declarations.iter().any(|matched| {
            matched.selector == "#banner" && matched.source.contains("card.css")
        }));
    }

    #[cfg(feature = "dom")]
    #[test]
    fn nth_child_and_media_queries_follow_viewport() {
        let document = parse_html(
            "<style>
                #releases tr:nth-child(2n+1) { background-color: #f7f6f1; }
                #releases tr:first-child { color: navy; }
                @media screen and (max-width: 848px) {
                    #banner { display: none; }
                }
                #hero { background: linear-gradient(to bottom, #ffd133, #b39223); }
            </style>
            <header id=banner>x</header>
            <div id=hero>y</div>
            <table id=releases><tr><td>a</td></tr><tr><td>b</td></tr><tr><td>c</td></tr></table>",
        )
        .unwrap();
        let sheets = collect_embedded_stylesheets(&document);
        assert!(sheets[0].rules.iter().any(|rule| rule
            .media
            .as_deref()
            .is_some_and(|media| media.contains("max-width"))));
        let wide = StyleContext::build_with_viewport(&document, &sheets, 1024);
        let narrow = StyleContext::build_with_viewport(&document, &sheets, 640);
        let banner = (0..document.node_count())
            .find(|id| {
                document.get(*id).and_then(|node| match node {
                    Node::Element { attributes, .. } => attribute_value(attributes, "id"),
                    _ => None,
                }) == Some("banner")
            })
            .unwrap();
        let hero = (0..document.node_count())
            .find(|id| {
                document.get(*id).and_then(|node| match node {
                    Node::Element { attributes, .. } => attribute_value(attributes, "id"),
                    _ => None,
                }) == Some("hero")
            })
            .unwrap();
        let rows = (0..document.node_count())
            .filter(|id| document.tag_name(*id) == Some("tr"))
            .collect::<Vec<_>>();
        assert_eq!(
            wide.style_for(banner).unwrap().value(&Property::Display),
            Some(&PropertyValue::Keyword(String::from("block")))
        );
        assert_eq!(
            narrow.style_for(banner).unwrap().value(&Property::Display),
            Some(&PropertyValue::Keyword(String::from("none")))
        );
        assert_eq!(
            wide.style_for(rows[0])
                .unwrap()
                .value(&Property::BackgroundColor),
            Some(&PropertyValue::Color(Color::Rgb(0xf7, 0xf6, 0xf1)))
        );
        assert_eq!(
            wide.style_for(rows[1])
                .unwrap()
                .value(&Property::BackgroundColor),
            Some(&PropertyValue::Color(Color::Transparent))
        );
        assert_eq!(
            wide.style_for(rows[0]).unwrap().value(&Property::Color),
            Some(&PropertyValue::Color(Color::Rgb(0, 0, 128)))
        );
        let image = wide
            .style_for(hero)
            .unwrap()
            .value(&Property::BackgroundImage)
            .unwrap()
            .display();
        assert!(image.contains("linear-gradient"));
        assert!(parse_gradient(&image).is_some());
    }
}
