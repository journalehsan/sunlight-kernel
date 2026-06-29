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
pub use sunlight_ui::theme::Color;
pub use sunlight_ui::paint::Canvas;

// ── Embedded MTF font blobs ───────────────────────────────────────────────────

static FONT_UI_11:         &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_11.mtf"));
static FONT_UI_13:         &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_13.mtf"));
static FONT_UI_MEDIUM_13:  &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_medium_13.mtf"));
static FONT_UI_SEMIBOLD_13:&[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_semibold_13.mtf"));
static FONT_UI_16:         &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_16.mtf"));
static FONT_UI_TITLE_18:   &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_ui_title_18.mtf"));
static FONT_MONO_REGULAR:  &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_mono_regular_14.mtf"));
static FONT_MONO_MEDIUM:   &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight_mono_medium_14.mtf"));

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
const OFFSET_TABLE_SIZE: usize = 95 * 4;
const GLYPH_DATA_START: usize = HEADER_SIZE + OFFSET_TABLE_SIZE; // 388

#[inline]
fn font_data(role: FontRole) -> &'static [u8] {
    match role {
        FontRole::UiSmall     => FONT_UI_11,
        FontRole::UiRegular   => FONT_UI_13,
        FontRole::UiMedium    => FONT_UI_MEDIUM_13,
        FontRole::UiBold      => FONT_UI_SEMIBOLD_13,
        FontRole::UiLarge     => FONT_UI_16,
        FontRole::UiTitle     => FONT_UI_TITLE_18,
        FontRole::MonoRegular => FONT_MONO_REGULAR,
        FontRole::MonoMedium  => FONT_MONO_MEDIUM,
    }
}

/// Read a u32 little-endian from `data[pos..]`.
#[inline]
fn read_u32(data: &[u8], pos: usize) -> u32 {
    let b = &data[pos..pos + 4];
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

struct GlyphInfo {
    advance: u8,
    left: i8,
    top: i8,
    width: u8,
    height: u8,
    pixel_offset: usize, // absolute byte offset into font data
}

/// Look up glyph info for `ch`.  Falls back to '?' for out-of-range characters.
fn glyph_info(data: &'static [u8], ch: char) -> Option<GlyphInfo> {
    let idx = if (ch as u32) >= 0x20 && (ch as u32) <= 0x7E {
        (ch as usize) - 0x20
    } else if ch == '?' || (b'?' as usize) >= 0x20 {
        b'?' as usize - 0x20 // fallback glyph
    } else {
        return None;
    };

    let offset_pos = HEADER_SIZE + idx * 4;
    if offset_pos + 4 > data.len() {
        return None;
    }
    let glyph_start = read_u32(data, offset_pos) as usize;
    if glyph_start + 5 > data.len() {
        return None;
    }

    let advance = data[glyph_start];
    let left    = data[glyph_start + 1] as i8;
    let top     = data[glyph_start + 2] as i8;
    let width   = data[glyph_start + 3];
    let height  = data[glyph_start + 4];

    let pixel_offset = glyph_start + 5;
    let pixel_end = pixel_offset + width as usize * height as usize;
    if pixel_end > data.len() {
        return None;
    }

    Some(GlyphInfo { advance, left, top, width, height, pixel_offset })
}

// ── Public metrics API ────────────────────────────────────────────────────────

/// Pixel distance from the top of the em box to the text baseline for `role`.
#[inline]
pub fn ascent(role: FontRole) -> u32 {
    let d = font_data(role);
    if d.len() >= 6 { d[5] as u32 } else { 10 }
}

/// Recommended vertical line spacing (em box height) in pixels for `role`.
#[inline]
pub fn line_height(role: FontRole) -> u32 {
    let d = font_data(role);
    if d.len() >= 5 { d[4] as u32 } else { 13 }
}

/// Measure the pixel width and height of `text` rendered with `role`.
///
/// Height is always `line_height(role)`.  Does not account for sub-pixel
/// advance rounding — use this for layout, not pixel-perfect clipping.
pub fn measure_text(text: &str, role: FontRole) -> Size {
    let data = font_data(role);
    let mut w = 0u32;
    for ch in text.chars() {
        if let Some(g) = glyph_info(data, ch) {
            w += g.advance as u32;
        }
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
    let data = font_data(style.role);
    let asc  = ascent(style.role) as i32;
    let baseline_y = y + asc;
    let mut cx = x;

    for ch in text.chars() {
        let g = match glyph_info(data, ch) {
            Some(g) => g,
            None    => { cx += 4; continue; } // unknown char — skip with small gap
        };

        if g.width > 0 && g.height > 0 {
            let bx = cx + g.left as i32;
            let by = baseline_y - g.top as i32;
            let pixels = &data[g.pixel_offset..g.pixel_offset + g.width as usize * g.height as usize];

            for row in 0..g.height as i32 {
                for col in 0..g.width as i32 {
                    let alpha = pixels[(row * g.width as i32 + col) as usize];
                    if alpha == 0 {
                        continue;
                    }
                    let fg = Color::rgba(
                        style.color.r(),
                        style.color.g(),
                        style.color.b(),
                        alpha,
                    );
                    canvas.blend_pixel(bx + col, by + row, fg);
                }
            }
        }

        cx += g.advance as i32;
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
pub fn draw_text_right(
    canvas: &mut Canvas,
    rect: Rect,
    text: &str,
    style: &TextStyle,
    pad: i32,
) {
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
        (FontRole::UiSmall,     FONT_UI_11),
        (FontRole::UiRegular,   FONT_UI_13),
        (FontRole::UiMedium,    FONT_UI_MEDIUM_13),
        (FontRole::UiBold,      FONT_UI_SEMIBOLD_13),
        (FontRole::UiLarge,     FONT_UI_16),
        (FontRole::UiTitle,     FONT_UI_TITLE_18),
        (FontRole::MonoRegular, FONT_MONO_REGULAR),
        (FontRole::MonoMedium,  FONT_MONO_MEDIUM),
    ] {
        assert!(
            data.len() >= GLYPH_DATA_START && &data[0..4] == b"MTF1",
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
    pub const UI_SMALL:   VecFont = VecFont(FontRole::UiSmall);
    pub const UI_REGULAR: VecFont = VecFont(FontRole::UiRegular);
    pub const UI_MEDIUM:  VecFont = VecFont(FontRole::UiMedium);
    pub const UI_BOLD:    VecFont = VecFont(FontRole::UiBold);
    pub const UI_LARGE:   VecFont = VecFont(FontRole::UiLarge);
    pub const UI_TITLE:   VecFont = VecFont(FontRole::UiTitle);
    pub const MONO:       VecFont = VecFont(FontRole::MonoRegular);
    pub const MONO_MEDIUM:VecFont = VecFont(FontRole::MonoMedium);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_have_valid_magic() {
        assert_fonts_valid();
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
        ] {
            let lh = line_height(role);
            assert!(lh >= 8 && lh <= 32, "line_height({:?}) = {} is out of range", role, lh);
        }
    }

    #[test]
    fn mono_and_ui_regular_have_distinct_metrics() {
        let ui_lh = line_height(FontRole::UiRegular);
        let mono_lh = line_height(FontRole::MonoRegular);
        // FiraCode 14px vs Inter 13px — line heights should differ
        assert!(ui_lh != mono_lh || measure_text("W", FontRole::MonoRegular).w == measure_text("i", FontRole::MonoRegular).w,
            "MonoRegular should have fixed-width cells");
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
        assert!(sz.w > 0, "measure_text returned zero width for non-empty string");
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
        assert_eq!(x, 10, "draw_text with empty string should not advance cursor");
    }
}
