#![allow(dead_code)]

use crate::framebuffer::Framebuffer;

// Font constants — no division, pure bit manipulation
const GLYPH_WIDTH: u32 = 8;
const GLYPH_HEIGHT: usize = 16;
const GLYPH_BYTES: usize = 16;
const FIRST_GLYPH: u8 = 0x20;  // Space
const LAST_GLYPH: u8 = 0x7E;   // Tilde

// Special Unicode glyph pseudo-indices (used for UTF-8 symbol mapping)
const GLYPH_CHECK: u8 = 0x80;    // ✓
const GLYPH_CROSS: u8 = 0x81;    // ✗
const GLYPH_SPINNER: u8 = 0x82;  // ⟳
const GLYPH_SUN: u8 = 0x83;      // ☀

// Embedded bitmap font: 95 glyphs × 16 bytes = 1520 bytes
// ASCII 0x20–0x7E, MSB (bit 7) = leftmost pixel per row
static FONT_DATA: &[u8] = include_bytes!("font8x16.bin");

/// Hardcoded reference 'A' glyph for diagnostic testing — validates renderer pipeline
const DIAGNOSTIC_A_GLYPH: [u8; GLYPH_BYTES] = [
    0x00, 0x00, 0x00, 0x38, 0x44, 0x82, 0x82, 0x82,
    0xfe, 0x82, 0x82, 0x82, 0x82, 0x00, 0x00, 0x00,
];

/// Retrieve glyph row bytes for ASCII character [0x20..0x7E]
#[inline]
fn glyph_for_ascii(c: u8) -> Option<&'static [u8]> {
    if !(FIRST_GLYPH..=LAST_GLYPH).contains(&c) {
        return None;
    }
    let glyph_idx = (c - FIRST_GLYPH) as usize;
    let start = glyph_idx * GLYPH_BYTES;
    let end = start + GLYPH_BYTES;
    FONT_DATA.get(start..end)
}

/// Internal: Render a single glyph bitmap with optional background fill.
/// Bit rendering: MSB-first (0x80 = leftmost pixel), right-shift advancing.
/// Scaling: no division — multiply x,y by scale factor for per-pixel expansion.
/// Transparent mode: bg_color=None skips background, preserving adjacent graphics.
#[inline(always)]
fn draw_glyph(
    fb: &mut Framebuffer,
    x: u32,
    y: u32,
    glyph: &[u8],
    fg_color: u32,
    bg_color: Option<u32>,
    scale: u32,
) {
    // Fill entire character bounding box with background color (if specified)
    if let Some(bg) = bg_color {
        let glyph_w = GLYPH_WIDTH * scale;
        let glyph_h = (GLYPH_HEIGHT as u32) * scale;
        for dy in 0..glyph_h {
            for dx in 0..glyph_w {
                fb.put_pixel(x + dx, y + dy, bg);
            }
        }
    }

    // Render foreground pixels: MSB-first bit extraction with right-shift advancing
    for (row_idx, &row_byte) in glyph.iter().enumerate() {
        let base_y = y + (row_idx as u32) * scale;
        let mut bit_mask = 0x80u8;  // Start at MSB (leftmost pixel)

        for col in 0..GLYPH_WIDTH {
            if (row_byte & bit_mask) != 0 {
                // Pixel is set: fill scale×scale block starting at (x + col*scale, base_y)
                for sy in 0..scale {
                    for sx in 0..scale {
                        fb.put_pixel(x + col * scale + sx, base_y + sy, fg_color);
                    }
                }
            }
            bit_mask >>= 1;  // Shift to next bit (no division, no branching)
        }
    }
}

/// Draw a single ASCII character at pixel position (x, y).
/// Safely maps ascii [0x20..0x7E] to font glyph offsets.
/// Uses transparent mode (no background fill) — only renders foreground pixels.
pub fn draw_char(
    fb: &mut Framebuffer,
    x: u32,
    y: u32,
    ascii: u8,
    color: u32,
    scale: u32,
) {
    let glyph = match ascii {
        FIRST_GLYPH..=LAST_GLYPH => glyph_for_ascii(ascii),
        b'\n' | b'\r' => return,  // Skip newlines (handled by caller)
        _ => glyph_for_ascii(FIRST_GLYPH),  // Fallback to space for unprintable
    };

    if let Some(glyph) = glyph {
        draw_glyph(fb, x, y, glyph, color, None, scale);
    }
}

/// Diagnostic: render a hardcoded reference 'A' glyph for renderer validation.
/// Use this to verify the rendering pipeline independently of font8x16.bin.
/// If this renders correctly but file-based glyphs don't, the issue is the binary file.
#[inline]
pub fn draw_char_diagnostic_a(
    fb: &mut Framebuffer,
    x: u32,
    y: u32,
    color: u32,
    scale: u32,
) {
    draw_glyph(fb, x, y, &DIAGNOSTIC_A_GLYPH, color, None, scale);
}

/// Draw a special Unicode symbol using a pseudo-glyph ID.
/// Maps GLYPH_CHECK, GLYPH_CROSS, GLYPH_SPINNER, GLYPH_SUN to ASCII fallbacks.
#[inline]
fn draw_special(
    fb: &mut Framebuffer,
    x: u32,
    y: u32,
    symbol: u8,
    color: u32,
    scale: u32,
) {
    let fallback = match symbol {
        GLYPH_CHECK => b'v',     // ✓ → v
        GLYPH_CROSS => b'x',     // ✗ → x
        GLYPH_SPINNER => b'o',   // ⟳ → o
        GLYPH_SUN => b'*',       // ☀ → *
        _ => b'?',
    };
    draw_char(fb, x, y, fallback, color, scale);
}

/// Draw a &str with full UTF-8 multi-byte symbol support.
/// Zero allocations: stateless byte-stream parser using index advancement.
/// Handles:
///   - Single-byte ASCII (0x20–0x7E, plus common control chars)
///   - 3-byte UTF-8 symbols: ✓ (E2 9C 93), ✗ (E2 9C 97), ⟳ (E2 9F B3), ☀ (E2 98 80)
/// Transparent mode: renders only foreground pixels, no background fill.
pub fn draw_str(
    fb: &mut Framebuffer,
    x: u32,
    y: u32,
    s: &str,
    color: u32,
    scale: u32,
) {
    let mut cx = x;
    let char_width = GLYPH_WIDTH * scale;
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            // UTF-8 three-byte sequences: E2 xx xx format (common Unicode range)
            0xE2 => {
                if i + 2 < bytes.len() {
                    let b1 = bytes[i + 1];
                    let b2 = bytes[i + 2];

                    match (b1, b2) {
                        (0x9C, 0x93) => { // ✓ (E2 9C 93)
                            draw_special(fb, cx, y, GLYPH_CHECK, color, scale);
                        }
                        (0x9C, 0x97) => { // ✗ (E2 9C 97)
                            draw_special(fb, cx, y, GLYPH_CROSS, color, scale);
                        }
                        (0x9F, 0xB3) => { // ⟳ (E2 9F B3)
                            draw_special(fb, cx, y, GLYPH_SPINNER, color, scale);
                        }
                        (0x98, 0x80) => { // ☀ (E2 98 80)
                            draw_special(fb, cx, y, GLYPH_SUN, color, scale);
                        }
                        _ => {
                            // Unknown UTF-8 sequence: render placeholder
                            draw_char(fb, cx, y, b'?', color, scale);
                        }
                    }
                    cx += char_width;
                    i += 3;  // Advance past 3-byte sequence
                    continue;
                }
                i += 1;
            }
            // Whitespace: newlines skipped inline; tab/space render as ASCII
            b'\n' | b'\r' => {
                i += 1;
            }
            // Standard single-byte ASCII
            _ => {
                draw_char(fb, cx, y, bytes[i], color, scale);
                cx += char_width;
                i += 1;
            }
        }
    }
}

/// Measure the width of a string in pixels (pre-render calculation).
/// Counts characters and UTF-8 sequences without rendering.
pub fn text_width(s: &str, scale: u32) -> u32 {
    let bytes = s.as_bytes();
    let mut width = 0u32;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            0xE2 => {
                if i + 2 < bytes.len() {
                    width += GLYPH_WIDTH * scale;
                    i += 3;
                    continue;
                }
                i += 1;
            }
            b'\n' | b'\r' => {
                i += 1;
            }
            _ => {
                width += GLYPH_WIDTH * scale;
                i += 1;
            }
        }
    }
    width
}

/// Single line height in pixels (constant for 8×16 glyphs).
#[inline]
pub const fn line_height(scale: u32) -> u32 {
    16 * scale
}
