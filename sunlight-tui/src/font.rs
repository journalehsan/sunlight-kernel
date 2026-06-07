//! 8×16 bitmap font — no heap, complete ASCII + special glyphs
//!
//! font8x16.bin: raw glyph data for ASCII 0x20–0x7E (95 glyphs × 16 bytes = 1520 bytes).
//! Sourced from cp866-8x16 PSF2 console font (standard VGA 8×16, public domain).
//! Bit order: MSB (bit 7) = leftmost pixel of each row.
//! Offset formula: glyph_start = (char_code − 0x20) × 16.

#![allow(dead_code)]

use crate::framebuffer::Framebuffer;

/// Raw glyph data: 95 chars × 16 bytes, ASCII 0x20–0x7E, MSB = leftmost pixel.
static FONT_DATA: &[u8] = include_bytes!("font8x16.bin");

const FIRST_GLYPH: u8 = 0x20;
const LAST_GLYPH: u8 = 0x7E;
const GLYPH_WIDTH: u32 = 8;
const GLYPH_HEIGHT: usize = 16;
const GLYPH_BYTES: usize = 16;

const DIAGNOSTIC_A_GLYPH: [u8; GLYPH_BYTES] = [
    0x00, 0x00, 0x10, 0x38, 0x6C, 0xC6, 0xC6, 0xFE,
    0xC6, 0xC6, 0xC6, 0xC6, 0x00, 0x00, 0x00, 0x00,
];

/// Special Unicode glyphs mapped to indices beyond ASCII
const GLYPH_CHECK: u8 = 0x80;    // ✓
const GLYPH_CROSS: u8 = 0x81;    // ✗
const GLYPH_SPINNER: u8 = 0x82;  // ⟳
const GLYPH_SUN: u8 = 0x83;      // ☀

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

fn draw_glyph(fb: &mut Framebuffer, x: u32, y: u32, glyph: &[u8], color: u32, scale: u32) {
    for (row, &byte) in glyph.iter().enumerate() {
        for col in 0..GLYPH_WIDTH {
            // MSB-first: bit 7 is column 0, the leftmost pixel.
            if (byte & (0x80u8 >> col)) != 0 {
                for sy in 0..scale {
                    for sx in 0..scale {
                        fb.put_pixel(
                            x + col * scale + sx,
                            y + (row as u32) * scale + sy,
                            color,
                        );
                    }
                }
            }
        }
    }
}

/// Draw a single character at pixel position (x, y).
///
/// Bit steering: `byte & (0x80 >> col)` — bit 7 (MSB) drives col 0 (leftmost pixel).
/// This is correct for the MSB-first encoding used in font8x16.bin.
pub fn draw_char(fb: &mut Framebuffer, x: u32, y: u32, c: u8, color: u32, scale: u32) {
    let glyph = match c {
        FIRST_GLYPH..=LAST_GLYPH => glyph_for_ascii(c),
        b'\n' | b'\r' => return,
        _ => glyph_for_ascii(FIRST_GLYPH),  // fallback to space
    };

    if let Some(glyph) = glyph {
        draw_glyph(fb, x, y, glyph, color, scale);
    }
}

/// Diagnostic: render a hardcoded reference 'A' glyph (cp866/VGA standard).
/// Use this to verify the renderer independently of font8x16.bin.
/// If this renders correctly but file-based characters don't, the issue is
/// the font binary's offset or content, not the rendering loop.
#[allow(dead_code)]
pub fn draw_char_diagnostic_a(fb: &mut Framebuffer, x: u32, y: u32, color: u32, scale: u32) {
    draw_glyph(fb, x, y, &DIAGNOSTIC_A_GLYPH, color, scale);
}

/// Draw special Unicode glyph
pub fn draw_special(fb: &mut Framebuffer, x: u32, y: u32, glyph: u8, color: u32, scale: u32) {
    // For now, draw ASCII approximations until we embed actual glyphs
    let fallback = match glyph {
        GLYPH_CHECK => b'v',     // ✓ → v
        GLYPH_CROSS => b'x',     // ✗ → x
        GLYPH_SPINNER => b'o',   // ⟳ → o
        GLYPH_SUN => b'*',       // ☀ → *
        _ => b'?',
    };
    draw_char(fb, x, y, fallback, color, scale);
}

/// Draw a &str — no allocation, no iterators over String
pub fn draw_str(fb: &mut Framebuffer, x: u32, y: u32, s: &str, color: u32, scale: u32) {
    let mut cx = x;
    let char_width = 8 * scale;
    
    for byte in s.as_bytes() {
        match *byte {
            // Special Unicode chars (UTF-8 encoded)
            0xE2 => {
                // Peek next bytes for UTF-8 sequences
                // For now, skip complex UTF-8, draw fallback
                cx += char_width;
            }
            b'\n' => {
                // Not handled in single-line draw
            }
            _ => {
                draw_char(fb, cx, y, *byte, color, scale);
                cx += char_width;
            }
        }
    }
}

/// Measure text width in pixels
pub fn text_width(s: &str, scale: u32) -> u32 {
    let char_count = s.as_bytes().iter().filter(|&&b| b >= 0x20 && b <= 0x7E).count();
    (char_count as u32) * 8 * scale
}

/// Single line height in pixels
pub const fn line_height(scale: u32) -> u32 {
    GLYPH_HEIGHT as u32 * scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_binary_has_printable_ascii_layout() {
        assert_eq!(FONT_DATA.len(), 95 * GLYPH_BYTES);
        assert_eq!(glyph_for_ascii(b' ').unwrap(), &[0; GLYPH_BYTES]);
        assert_eq!(glyph_for_ascii(b'A').unwrap(), &DIAGNOSTIC_A_GLYPH);
    }

    #[test]
    fn draw_char_uses_msb_as_leftmost_pixel() {
        const COLOR: u32 = 0x00EE_EEEE;
        let mut pixels = [0u32; GLYPH_HEIGHT * GLYPH_WIDTH as usize];
        let mut fb = unsafe {
            Framebuffer::from_limine(
                pixels.as_mut_ptr(),
                GLYPH_WIDTH,
                GLYPH_HEIGHT as u32,
                GLYPH_WIDTH * 4,
            )
        };

        draw_char(&mut fb, 0, 0, b'A', COLOR, 1);

        for row in 0..GLYPH_HEIGHT {
            let byte = DIAGNOSTIC_A_GLYPH[row];
            for col in 0..GLYPH_WIDTH as usize {
                let expected = if (byte & (0x80u8 >> col)) != 0 { COLOR } else { 0 };
                assert_eq!(pixels[row * GLYPH_WIDTH as usize + col], expected);
            }
        }
    }
}
