//! `sun-font` — SunlightOS MiniType font rendering.
//!
//! Provides antialiased text via build-time rasterised Inter glyphs stored in
//! the custom `.mtf` binary format. The runtime is `no_std` + zero alloc: all
//! glyph data is embedded as `&'static [u8]` via `include_bytes!`.
//!
//! # Quick start
//!
//! ```ignore
//! use sun_font::{FontRole, TextStyle, draw_text, measure_text};
//! use sunlight_ui::{Canvas, Theme};
//!
//! let theme = Theme::sunlight_dark();
//! // Draw "Hello" in the regular UI font.
//! draw_text(&mut canvas, "Hello", x, y, &TextStyle::new(FontRole::UiRegular, theme.text));
//! ```
//!
//! The `y` coordinate is the **top of the em box** (where the top of capital
//! letters sits).  `baseline = y + ascent(role)`.

#![cfg_attr(not(any(test, feature = "std")), no_std)]

pub use sunlight_ui::geom::{Rect, Size};
pub use sunlight_ui::paint::Canvas;
pub use sunlight_ui::theme::Color;

// ── Embedded MTF font blobs ───────────────────────────────────────────────────

static FONT_UI_11: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_11.mtf"));
static FONT_UI_13: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_13.mtf"));
static FONT_UI_MEDIUM_13: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_medium_13.mtf"));
static FONT_UI_SEMIBOLD_13: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_semibold_13.mtf"));
static FONT_UI_16: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_16.mtf"));
static FONT_UI_TITLE_18: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_title_18.mtf"));
static FONT_MONO_REGULAR: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_mono_regular_14.mtf"));
static FONT_MONO_MEDIUM: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_mono_medium_14.mtf"));
static FONT_SERIF_REGULAR: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_serif_regular_16.mtf"));
static FONT_EMOJI: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_emoji_16.mtf"));

// ── Font roles ────────────────────────────────────────────────────────────────

/// Semantic font roles used throughout the SunlightOS UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontRole {
    /// 11 px Inter Regular — captions, hints, small status text.
    UiSmall,
    /// 13 px Inter Regular — general UI labels, file names, toolbar text.
    UiRegular,
    /// 13 px Inter Medium — selected items, buttons, slight emphasis.
    UiMedium,
    /// 13 px Inter SemiBold — headings, strong labels.
    UiBold,
    /// 16 px Inter Regular — section titles within panels.
    UiLarge,
    /// 18 px Inter Medium — window titles, major section headings.
    UiTitle,
    /// 14 px Fira Code Regular — terminal text, logs, code output.
    MonoRegular,
    /// 14 px Fira Code Medium — emphasized monospace (bold terminal output).
    MonoMedium,
    /// 16 px Noto Serif Regular — native serif document text.
    SerifRegular,
    /// 16 px OpenMoji Black — automatic monochrome emoji/symbol fallback.
    Emoji,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontId {
    SunFont,
    SunSerif,
    SunMono,
    SunEmoji,
}

impl FontId {
    pub const fn family_name(self) -> &'static str {
        match self {
            Self::SunFont => "Sun Font",
            Self::SunSerif => "Sun Serif",
            Self::SunMono => "Sun Mono",
            Self::SunEmoji => "Sun Emoji",
        }
    }

    pub const fn classification(self) -> &'static str {
        match self {
            Self::SunFont => "Sans",
            Self::SunSerif => "Serif",
            Self::SunMono => "Monospace",
            Self::SunEmoji => "Emoji / Symbol",
        }
    }
}

impl FontRole {
    pub const fn font_id(self) -> FontId {
        match self {
            Self::SerifRegular => FontId::SunSerif,
            Self::MonoRegular | Self::MonoMedium => FontId::SunMono,
            Self::Emoji => FontId::SunEmoji,
            _ => FontId::SunFont,
        }
    }
}

// ── TextStyle ─────────────────────────────────────────────────────────────────

/// Combines a font role with a fill colour for text rendering calls.
#[derive(Debug, Clone, Copy)]
pub struct TextStyle {
    pub role: FontRole,
    pub color: Color,
}

impl TextStyle {
    #[inline]
    pub const fn new(role: FontRole, color: Color) -> Self {
        Self { role, color }
    }

    /// Quick way to change only the color (e.g., for hover state).
    #[inline]
    pub const fn with_color(self, color: Color) -> Self {
        Self { color, ..self }
    }
}

// ── MTF parsing helpers ───────────────────────────────────────────────────────

const HEADER_SIZE: usize = 8;

#[inline]
fn font_data(role: FontRole) -> &'static [u8] {
    match role {
        FontRole::UiSmall => FONT_UI_11,
        FontRole::UiRegular => FONT_UI_13,
        FontRole::UiMedium => FONT_UI_MEDIUM_13,
        FontRole::UiBold => FONT_UI_SEMIBOLD_13,
        FontRole::UiLarge => FONT_UI_16,
        FontRole::UiTitle => FONT_UI_TITLE_18,
        FontRole::MonoRegular => FONT_MONO_REGULAR,
        FontRole::MonoMedium => FONT_MONO_MEDIUM,
        FontRole::SerifRegular => FONT_SERIF_REGULAR,
        FontRole::Emoji => FONT_EMOJI,
    }
}

/// Read a u32 little-endian from `data[pos..]`.
#[inline]
fn read_u32(data: &[u8], pos: usize) -> Option<u32> {
    let b = data.get(pos..pos + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[derive(Clone, Copy, Debug)]
struct GlyphInfo {
    advance: u8,
    left: i8,
    top: i8,
    width: u8,
    height: u8,
    pixel_offset: usize, // absolute byte offset into font data
}

fn glyph_info(data: &'static [u8], ch: char) -> Option<GlyphInfo> {
    if data.get(..4) == Some(b"MTF2") {
        let glyph = mtf2_cmap_glyph(data, ch as u32)?;
        return mtf2_glyph_info(data, glyph);
    }
    if data.get(..4) != Some(b"MTF1") {
        return None;
    }
    let idx = if (ch as u32) >= 0x20 && (ch as u32) <= 0x7e {
        (ch as usize) - 0x20
    } else {
        return None;
    };

    let offset_pos = HEADER_SIZE + idx * 4;
    if offset_pos + 4 > data.len() {
        return None;
    }
    let glyph_start = read_u32(data, offset_pos)? as usize;
    glyph_info_at(data, glyph_start)
}

fn glyph_info_at(data: &'static [u8], glyph_start: usize) -> Option<GlyphInfo> {
    if glyph_start + 5 > data.len() {
        return None;
    }

    let advance = data[glyph_start];
    let left = data[glyph_start + 1] as i8;
    let top = data[glyph_start + 2] as i8;
    let width = data[glyph_start + 3];
    let height = data[glyph_start + 4];

    let pixel_offset = glyph_start + 5;
    let pixel_end = pixel_offset + width as usize * height as usize;
    if pixel_end > data.len() {
        return None;
    }

    Some(GlyphInfo {
        advance,
        left,
        top,
        width,
        height,
        pixel_offset,
    })
}

fn mtf2_cmap_glyph(data: &[u8], codepoint: u32) -> Option<u32> {
    let count = read_u32(data, 12)? as usize;
    let offset = read_u32(data, 20)? as usize;
    let mut low = 0usize;
    let mut high = count;
    while low < high {
        let middle = low + (high - low) / 2;
        let entry = offset.checked_add(middle.checked_mul(8)?)?;
        match read_u32(data, entry)?.cmp(&codepoint) {
            core::cmp::Ordering::Less => low = middle + 1,
            core::cmp::Ordering::Greater => high = middle,
            core::cmp::Ordering::Equal => return read_u32(data, entry + 4),
        }
    }
    None
}

fn mtf2_glyph_info(data: &'static [u8], glyph: u32) -> Option<GlyphInfo> {
    let count = read_u32(data, 8)?;
    if glyph >= count {
        return None;
    }
    let offsets = read_u32(data, 28)? as usize;
    let start = read_u32(data, offsets.checked_add(glyph as usize * 4)?)? as usize;
    glyph_info_at(data, start)
}

const MAX_SEQUENCE_LEN: usize = 16;
const SEQUENCE_ENTRY_SIZE: usize = 4 + MAX_SEQUENCE_LEN * 4 + 4;

fn mtf2_sequence_glyph(data: &[u8], sequence: &[u32]) -> Option<u32> {
    if sequence.len() < 2 || sequence.len() > MAX_SEQUENCE_LEN {
        return None;
    }
    let count = read_u32(data, 16)? as usize;
    let offset = read_u32(data, 24)? as usize;
    let mut low = 0usize;
    let mut high = count;
    while low < high {
        let middle = low + (high - low) / 2;
        let entry = offset.checked_add(middle.checked_mul(SEQUENCE_ENTRY_SIZE)?)?;
        let stored_len = *data.get(entry)? as usize;
        let common = stored_len.min(sequence.len());
        let mut ordering = core::cmp::Ordering::Equal;
        for (index, expected) in sequence.iter().enumerate().take(common) {
            let actual = read_u32(data, entry + 4 + index * 4)?;
            ordering = actual.cmp(expected);
            if ordering != core::cmp::Ordering::Equal {
                break;
            }
        }
        if ordering == core::cmp::Ordering::Equal {
            ordering = stored_len.cmp(&sequence.len());
        }
        match ordering {
            core::cmp::Ordering::Less => low = middle + 1,
            core::cmp::Ordering::Greater => high = middle,
            core::cmp::Ordering::Equal => return read_u32(data, entry + 4 + MAX_SEQUENCE_LEN * 4),
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedFont {
    Primary,
    SunEmoji,
    Missing,
    InvisibleControl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphPaintKind {
    MonochromeMask,
    ColorGlyph,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedCluster {
    pub byte_len: usize,
    pub scalar_len: usize,
    pub font: ResolvedFont,
    pub glyph_id: u32,
    pub advance: u8,
    pub paint_kind: GlyphPaintKind,
}

fn resolve_cluster(text: &str, role: FontRole) -> Option<(ResolvedCluster, GlyphInfo)> {
    let first = text.chars().next()?;
    if matches!(first, '\u{fe0e}' | '\u{fe0f}' | '\u{200d}') {
        return Some((
            ResolvedCluster {
                byte_len: first.len_utf8(),
                scalar_len: 1,
                font: ResolvedFont::InvisibleControl,
                glyph_id: 0,
                advance: 0,
                paint_kind: GlyphPaintKind::MonochromeMask,
            },
            GlyphInfo {
                advance: 0,
                left: 0,
                top: 0,
                width: 0,
                height: 0,
                pixel_offset: 0,
            },
        ));
    }

    let mut codes = [0u32; MAX_SEQUENCE_LEN];
    let mut ends = [0usize; MAX_SEQUENCE_LEN];
    let mut count = 0usize;
    for (offset, ch) in text.char_indices().take(MAX_SEQUENCE_LEN) {
        codes[count] = ch as u32;
        ends[count] = offset + ch.len_utf8();
        count += 1;
    }
    let primary = glyph_info(font_data(role), first);
    let mut emoji_match = None;
    for length in (2..=count).rev() {
        if let Some(glyph) = mtf2_sequence_glyph(FONT_EMOJI, &codes[..length]) {
            emoji_match = mtf2_glyph_info(FONT_EMOJI, glyph)
                .map(|info| (glyph, info, length, ends[length - 1]));
            break;
        }
    }
    let requests_emoji = emoji_match.is_some_and(|(_, _, length, _)| {
        codes[..length]
            .iter()
            .any(|code| matches!(*code, 0xfe0f | 0x200d | 0x20e3 | 0x1f3fb..=0x1f3ff))
            || (length == 2
                && codes[0] >= 0x1f1e6
                && codes[0] <= 0x1f1ff
                && codes[1] >= 0x1f1e6
                && codes[1] <= 0x1f1ff)
    });
    if (primary.is_none() || requests_emoji) && emoji_match.is_some() {
        let (glyph, info, scalar_len, byte_len) = emoji_match.unwrap();
        return Some((
            ResolvedCluster {
                byte_len,
                scalar_len,
                font: ResolvedFont::SunEmoji,
                glyph_id: glyph,
                advance: info.advance,
                paint_kind: GlyphPaintKind::MonochromeMask,
            },
            info,
        ));
    }
    if let Some(info) = primary {
        let consume_selector = text[first.len_utf8()..].starts_with('\u{fe0e}');
        return Some((
            ResolvedCluster {
                byte_len: first.len_utf8() + usize::from(consume_selector) * 3,
                scalar_len: 1 + usize::from(consume_selector),
                font: ResolvedFont::Primary,
                glyph_id: mtf2_cmap_glyph(font_data(role), first as u32).unwrap_or(first as u32),
                advance: info.advance,
                paint_kind: GlyphPaintKind::MonochromeMask,
            },
            info,
        ));
    }
    if let Some(info) = glyph_info(FONT_EMOJI, first) {
        let glyph = mtf2_cmap_glyph(FONT_EMOJI, first as u32)?;
        return Some((
            ResolvedCluster {
                byte_len: first.len_utf8(),
                scalar_len: 1,
                font: ResolvedFont::SunEmoji,
                glyph_id: glyph,
                advance: info.advance,
                paint_kind: GlyphPaintKind::MonochromeMask,
            },
            info,
        ));
    }
    let fallback = glyph_info(font_data(role), '?')?;
    Some((
        ResolvedCluster {
            byte_len: first.len_utf8(),
            scalar_len: 1,
            font: ResolvedFont::Missing,
            glyph_id: mtf2_cmap_glyph(font_data(role), '?' as u32).unwrap_or(31),
            advance: fallback.advance,
            paint_kind: GlyphPaintKind::MonochromeMask,
        },
        fallback,
    ))
}

pub fn for_each_resolved_cluster(
    text: &str,
    role: FontRole,
    mut visit: impl FnMut(usize, ResolvedCluster),
) {
    let mut offset = 0usize;
    while offset < text.len() {
        let Some((cluster, _)) = resolve_cluster(&text[offset..], role) else {
            break;
        };
        visit(offset, cluster);
        offset += cluster.byte_len.max(1);
    }
}

// ── Public metrics API ────────────────────────────────────────────────────────

/// Pixel distance from the top of the em box to the text baseline for `role`.
#[inline]
pub fn ascent(role: FontRole) -> u32 {
    let d = font_data(role);
    if d.len() >= 6 {
        d[5] as u32
    } else {
        10
    }
}

/// Recommended vertical line spacing (em box height) in pixels for `role`.
#[inline]
pub fn line_height(role: FontRole) -> u32 {
    let d = font_data(role);
    if d.len() >= 5 {
        d[4] as u32
    } else {
        13
    }
}

/// Measure the pixel width and height of `text` rendered with `role`.
///
/// Height is always `line_height(role)`.  Does not account for sub-pixel
/// advance rounding — use this for layout, not pixel-perfect clipping.
pub fn measure_text(text: &str, role: FontRole) -> Size {
    let mut w = 0u32;
    let mut offset = 0usize;
    while offset < text.len() {
        let Some((cluster, _)) = resolve_cluster(&text[offset..], role) else {
            break;
        };
        w = w.saturating_add(cluster.advance as u32);
        offset += cluster.byte_len.max(1);
    }
    Size::new(w, line_height(role))
}

// ── Rendering ─────────────────────────────────────────────────────────────────

/// Draw `text` at `(x, y)` where **y is the top of the em box**.
///
/// Alpha-blends each glyph's coverage mask over whatever is already in the
/// canvas.  Returns the x coordinate immediately after the last glyph so the
/// caller can continue rendering on the same line.
pub fn draw_text(canvas: &mut Canvas, text: &str, x: i32, y: i32, style: &TextStyle) -> i32 {
    let asc = ascent(style.role) as i32;
    let baseline_y = y + asc;
    let mut cx = x;
    let mut offset = 0usize;

    while offset < text.len() {
        let Some((cluster, g)) = resolve_cluster(&text[offset..], style.role) else {
            break;
        };
        let data = if cluster.font == ResolvedFont::SunEmoji {
            FONT_EMOJI
        } else {
            font_data(style.role)
        };

        if g.width > 0 && g.height > 0 {
            let bx = cx + g.left as i32;
            let by = baseline_y - g.top as i32;
            let pixels =
                &data[g.pixel_offset..g.pixel_offset + g.width as usize * g.height as usize];

            for row in 0..g.height as i32 {
                for col in 0..g.width as i32 {
                    let alpha = pixels[(row * g.width as i32 + col) as usize];
                    if alpha == 0 {
                        continue;
                    }
                    let fg = Color::rgba(style.color.r(), style.color.g(), style.color.b(), alpha);
                    canvas.blend_pixel(bx + col, by + row, fg);
                }
            }
        }

        cx += g.advance as i32;
        offset += cluster.byte_len.max(1);
    }
    cx
}

/// Draw `text` horizontally and vertically centred within `rect`.
pub fn draw_text_centered(canvas: &mut Canvas, rect: Rect, text: &str, style: &TextStyle) {
    let sz = measure_text(text, style.role);
    let tx = rect.x + (rect.w as i32 - sz.w as i32) / 2;
    let ty = rect.y + (rect.h as i32 - sz.h as i32) / 2;
    draw_text(canvas, text, tx, ty, style);
}

/// Draw `text` right-aligned inside `rect`, with `pad` pixels of right padding.
pub fn draw_text_right(canvas: &mut Canvas, rect: Rect, text: &str, style: &TextStyle, pad: i32) {
    let sz = measure_text(text, style.role);
    let tx = rect.right() - sz.w as i32 - pad;
    let ty = rect.y + (rect.h as i32 - sz.h as i32) / 2;
    draw_text(canvas, text, tx, ty, style);
}

/// Draw `text` at a position where it is **vertically centred** within the
/// given `height` area starting at `y`, using the explicit `x` for alignment.
///
/// Convenience wrapper for "left-aligned text that is vertically centred in a
/// row or panel of known height" — the most common file-manager pattern.
pub fn draw_text_vcenter(
    canvas: &mut Canvas,
    text: &str,
    x: i32,
    y: i32,
    height: u32,
    style: &TextStyle,
) -> i32 {
    let lh = line_height(style.role) as i32;
    let ty = y + (height as i32 - lh) / 2;
    draw_text(canvas, text, x, ty, style)
}

// ── VecFont: implements sunlight_ui::VecText ──────────────────────────────────

/// Concrete `VecText` implementation wrapping a `FontRole`.
///
/// Create a `static` instance and pass a reference to widgets:
/// ```ignore
/// static LABEL_FONT: VecFont = VecFont(FontRole::UiRegular);
/// SidebarItem::new(rect, "Home").with_font(&LABEL_FONT).draw(canvas, theme);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VecFont(pub FontRole);

impl sunlight_ui::font::VecText for VecFont {
    fn draw(&self, canvas: &mut Canvas, text: &str, x: i32, y: i32, color: Color) -> i32 {
        draw_text(canvas, text, x, y, &TextStyle::new(self.0, color))
    }

    fn draw_vcenter(
        &self,
        canvas: &mut Canvas,
        text: &str,
        x: i32,
        y: i32,
        height: u32,
        color: Color,
    ) -> i32 {
        draw_text_vcenter(canvas, text, x, y, height, &TextStyle::new(self.0, color))
    }

    fn measure_w(&self, text: &str) -> u32 {
        measure_text(text, self.0).w
    }

    fn line_height(&self) -> u32 {
        line_height(self.0)
    }
}

// ── Utility: validate that the embedded MTF blobs look sane ──────────────────

/// Panics if any embedded font blob has an invalid MTF magic header.
/// Call once at process startup to catch a corrupt build.
pub fn assert_fonts_valid() {
    for (role, data) in [
        (FontRole::UiSmall, FONT_UI_11),
        (FontRole::UiRegular, FONT_UI_13),
        (FontRole::UiMedium, FONT_UI_MEDIUM_13),
        (FontRole::UiBold, FONT_UI_SEMIBOLD_13),
        (FontRole::UiLarge, FONT_UI_16),
        (FontRole::UiTitle, FONT_UI_TITLE_18),
        (FontRole::MonoRegular, FONT_MONO_REGULAR),
        (FontRole::MonoMedium, FONT_MONO_MEDIUM),
        (FontRole::SerifRegular, FONT_SERIF_REGULAR),
        (FontRole::Emoji, FONT_EMOJI),
    ] {
        assert!(
            data.len() >= 32 && matches!(&data[0..4], b"MTF1" | b"MTF2"),
            "sun-font: embedded MTF blob for {:?} has invalid magic",
            role,
        );
    }
}

/// Convenience statics — one `VecFont` per semantic role.
///
/// Import and pass a reference to any widget that accepts `&dyn VecText`:
/// ```ignore
/// use sun_font::Typography as F;
/// Label::new(rect, "Hello").with_font(&F::UI_REGULAR).draw(canvas, theme);
/// ```
pub struct Typography;

impl Typography {
    pub const UI_SMALL: VecFont = VecFont(FontRole::UiSmall);
    pub const UI_REGULAR: VecFont = VecFont(FontRole::UiRegular);
    pub const UI_MEDIUM: VecFont = VecFont(FontRole::UiMedium);
    pub const UI_BOLD: VecFont = VecFont(FontRole::UiBold);
    pub const UI_LARGE: VecFont = VecFont(FontRole::UiLarge);
    pub const UI_TITLE: VecFont = VecFont(FontRole::UiTitle);
    pub const MONO: VecFont = VecFont(FontRole::MonoRegular);
    pub const MONO_MEDIUM: VecFont = VecFont(FontRole::MonoMedium);
    pub const SERIF: VecFont = VecFont(FontRole::SerifRegular);
    pub const EMOJI: VecFont = VecFont(FontRole::Emoji);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_have_valid_magic() {
        assert_fonts_valid();
        assert_eq!(FontRole::Emoji.font_id().family_name(), "Sun Emoji");
        assert_eq!(FontRole::Emoji.font_id().classification(), "Emoji / Symbol");
    }

    #[test]
    fn line_heights_are_sane() {
        for role in [
            FontRole::UiSmall,
            FontRole::UiRegular,
            FontRole::UiMedium,
            FontRole::UiBold,
            FontRole::UiLarge,
            FontRole::UiTitle,
            FontRole::MonoRegular,
            FontRole::MonoMedium,
            FontRole::Emoji,
        ] {
            let lh = line_height(role);
            assert!(
                lh >= 8 && lh <= 32,
                "line_height({:?}) = {} is out of range",
                role,
                lh
            );
        }
    }

    #[test]
    fn mono_and_ui_regular_have_distinct_metrics() {
        let ui_lh = line_height(FontRole::UiRegular);
        let mono_lh = line_height(FontRole::MonoRegular);
        // FiraCode 14px vs Inter 13px — line heights should differ
        assert!(
            ui_lh != mono_lh
                || measure_text("W", FontRole::MonoRegular).w
                    == measure_text("i", FontRole::MonoRegular).w,
            "MonoRegular should have fixed-width cells"
        );
    }

    #[test]
    fn serif_regular_is_valid_and_has_distinct_advances() {
        assert!(FONT_SERIF_REGULAR.len() > 32);
        assert_eq!(&FONT_SERIF_REGULAR[..4], b"MTF2");
        assert!(measure_text("The quick brown rabbit", FontRole::SerifRegular).w > 0);
        assert_ne!(
            measure_text("WWWiii", FontRole::SerifRegular).w,
            measure_text("WWWiii", FontRole::MonoRegular).w
        );
    }

    #[test]
    fn measure_empty_string_is_zero_width() {
        let sz = measure_text("", FontRole::UiRegular);
        assert_eq!(sz.w, 0);
        assert_eq!(sz.h, line_height(FontRole::UiRegular));
    }

    #[test]
    fn measure_returns_positive_width() {
        let sz = measure_text("Hello", FontRole::UiRegular);
        assert!(
            sz.w > 0,
            "measure_text returned zero width for non-empty string"
        );
    }

    #[test]
    fn draw_missing_glyph_does_not_panic() {
        let mut pixels = [0u32; 200 * 50];
        let mut canvas = Canvas::new(&mut pixels, 200, 200, 50);
        let style = TextStyle::new(FontRole::UiRegular, Color::rgb(0xFF, 0xFF, 0xFF));
        // U+2603 SNOWMAN — not in the ASCII glyph table, uses '?' fallback.
        draw_text(&mut canvas, "\u{2603} test", 0, 0, &style);
    }

    #[test]
    fn draw_empty_string_is_safe() {
        let mut pixels = [0u32; 100 * 20];
        let mut canvas = Canvas::new(&mut pixels, 100, 100, 20);
        let style = TextStyle::new(FontRole::UiSmall, Color::rgb(0xF0, 0xF0, 0xF0));
        let x = draw_text(&mut canvas, "", 10, 4, &style);
        assert_eq!(
            x, 10,
            "draw_text with empty string should not advance cursor"
        );
    }

    #[test]
    fn sun_emoji_asset_has_full_cmap_and_sequence_tables() {
        assert_eq!(&FONT_EMOJI[..4], b"MTF2");
        assert!(read_u32(FONT_EMOJI, 12).unwrap() >= 1_400);
        assert!(read_u32(FONT_EMOJI, 16).unwrap() >= 2_500);
        for ch in ['🐇', '🐧', '🦀', '😀'] {
            assert!(glyph_info(FONT_EMOJI, ch).is_some(), "missing {ch}");
        }
    }

    #[test]
    fn fallback_segments_text_and_sequences_consistently() {
        let text = "Hello 🐇 ❤️ 👍🏽 👨‍💻 🇩🇪 1️⃣";
        let mut primary = 0;
        let mut emoji = 0;
        for_each_resolved_cluster(text, FontRole::UiRegular, |_, cluster| match cluster.font {
            ResolvedFont::Primary => primary += 1,
            ResolvedFont::SunEmoji => emoji += 1,
            _ => {}
        });
        assert!(primary >= 6);
        assert_eq!(emoji, 6);
        assert!(
            measure_text(text, FontRole::UiRegular).w
                > measure_text("Hello ", FontRole::UiRegular).w
        );
    }

    #[test]
    fn text_font_wins_for_typography_and_variation_controls_are_invisible() {
        for text in ["©", "→", "“Rabbit”", "…"] {
            for_each_resolved_cluster(text, FontRole::UiRegular, |_, cluster| {
                assert_eq!(cluster.font, ResolvedFont::Primary);
            });
        }
        assert_eq!(
            measure_text("☀️", FontRole::UiRegular).w,
            measure_text("☀", FontRole::Emoji).w
        );
        assert_eq!(measure_text("\u{fe0f}", FontRole::UiRegular).w, 0);
    }

    #[test]
    fn emoji_mask_is_tinted_and_has_transparent_holes() {
        let mut pixels = [0u32; 40 * 40];
        let mut canvas = Canvas::new(&mut pixels, 40, 40, 40);
        draw_text(
            &mut canvas,
            "🐇",
            4,
            4,
            &TextStyle::new(FontRole::UiRegular, Color::rgb(240, 80, 40)),
        );
        assert!(pixels.iter().any(|pixel| (*pixel & 0x00ff_ffff) != 0));
        assert!(pixels.iter().any(|pixel| *pixel == 0));
    }
}
