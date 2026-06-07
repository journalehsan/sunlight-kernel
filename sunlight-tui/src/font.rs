//! 8×16 bitmap font — no heap, complete ASCII + special glyphs

#![allow(dead_code)]

use crate::framebuffer::Framebuffer;

/// 8×16 bitmap font data for ASCII 0x20–0x7E
/// Each character is 16 bytes (one byte per row, MSB = leftmost pixel)
static FONT_DATA: &[u8] = include_bytes!("font8x16.bin");

/// Special Unicode glyphs mapped to indices beyond ASCII
const GLYPH_CHECK: u8 = 0x80;    // ✓
const GLYPH_CROSS: u8 = 0x81;    // ✗
const GLYPH_SPINNER: u8 = 0x82;  // ⟳
const GLYPH_SUN: u8 = 0x83;      // ☀

/// Draw a single character at pixel position (x, y)
pub fn draw_char(fb: &mut Framebuffer, x: u32, y: u32, c: u8, color: u32, scale: u32) {
    let glyph_idx = match c {
        0x20..=0x7E => (c - 0x20) as usize,
        b'\n' | b'\r' => return,  // ignore control chars
        _ => 0,  // fallback to space
    };
    
    if glyph_idx * 16 + 16 > FONT_DATA.len() {
        return;
    }
    
    let glyph = &FONT_DATA[glyph_idx * 16..(glyph_idx * 16 + 16)];
    
    for (row, &byte) in glyph.iter().enumerate() {
        for col in 0..8 {
            if (byte & (0x80 >> col)) != 0 {
                // Draw scaled pixel
                for sy in 0..scale {
                    for sx in 0..scale {
                        fb.put_pixel(
                            x + col * scale + sx,
                            y + (row as u32) * scale + sy,
                            color
                        );
                    }
                }
            }
        }
    }
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
    16 * scale
}
