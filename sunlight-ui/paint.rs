//! Low-level framebuffer paint helpers.
//!
//! All drawing is done into a `Canvas` — a mutable view over a pixel slice
//! with a known stride. Widgets call these helpers; nothing here knows about
//! themes or widget state.

use crate::geom::Rect;
use crate::theme::Color;
use crate::font::ui_symbols;
use crate::font::UiSymbol;

/// A mutable view over a region of a framebuffer.
/// Pixels are 32-bit ARGB, row-major, stride in *pixels* (not bytes).
pub struct Canvas<'fb> {
    pub pixels: &'fb mut [u32],
    pub stride: u32, // pixels per row in the full framebuffer
    pub width: u32,
    pub height: u32,
}

impl<'fb> Canvas<'fb> {
    pub fn new(pixels: &'fb mut [u32], stride: u32, width: u32, height: u32) -> Self {
        Self {
            pixels,
            stride,
            width,
            height,
        }
    }

    /// Write a single pixel, bounds-checked.
    #[inline]
    pub fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let idx = y as usize * self.stride as usize + x as usize;
        if idx < self.pixels.len() {
            self.pixels[idx] = color.0;
        }
    }

    /// Fill a rectangle with a solid color.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let x0 = rect.x.max(0) as u32;
        let y0 = rect.y.max(0) as u32;
        let x1 = (rect.right()).min(self.width as i32).max(0) as u32;
        let y1 = (rect.bottom()).min(self.height as i32).max(0) as u32;

        for y in y0..y1 {
            let row_start = y as usize * self.stride as usize;
            for x in x0..x1 {
                self.pixels[row_start + x as usize] = color.0;
            }
        }
    }

    /// Draw a 1-pixel border around `rect`.
    pub fn draw_rect(&mut self, rect: Rect, color: Color) {
        // top / bottom
        for x in rect.x..rect.right() {
            self.put_pixel(x, rect.y, color);
            self.put_pixel(x, rect.bottom() - 1, color);
        }
        // left / right
        for y in rect.y..rect.bottom() {
            self.put_pixel(rect.x, y, color);
            self.put_pixel(rect.right() - 1, y, color);
        }
    }

    /// Draw a horizontal line.
    pub fn hline(&mut self, x: i32, y: i32, len: u32, color: Color) {
        for i in 0..len as i32 {
            self.put_pixel(x + i, y, color);
        }
    }

    /// Draw a vertical line.
    pub fn vline(&mut self, x: i32, y: i32, len: u32, color: Color) {
        for i in 0..len as i32 {
            self.put_pixel(x, y + i, color);
        }
    }

    /// Draw a thick horizontal bar (filled rect of height `thickness`).
    pub fn hbar(&mut self, x: i32, y: i32, len: u32, thickness: u32, color: Color) {
        self.fill_rect(Rect::new(x, y, len, thickness), color);
    }

    /// Blend a color over the existing pixel (alpha composite).
    pub fn blend_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let idx = y as usize * self.stride as usize + x as usize;
        if idx < self.pixels.len() {
            let dst = Color(self.pixels[idx]);
            self.pixels[idx] = color.blend_over(dst).0;
        }
    }

    /// Draw a simple bitmap glyph. `bitmap` is a packed bit array, row-major,
    /// MSB-first, `pitch` bits wide.
    pub fn draw_glyph(
        &mut self,
        x: i32,
        y: i32,
        bitmap: &[u8],
        pitch: u32,
        rows: u32,
        color: Color,
    ) {
        for row in 0..rows {
            for col in 0..pitch {
                let bit_idx = row * pitch + col;
                let byte = bitmap[(bit_idx / 8) as usize];
                let bit = (byte >> (7 - (bit_idx % 8))) & 1;
                if bit != 0 {
                    self.put_pixel(x + col as i32, y + row as i32, color);
                }
            }
        }
    }

    /// Draw a glyph stored as one bitmask row per `u16`.
    pub fn draw_glyph_rows(
        &mut self,
        x: i32,
        y: i32,
        rows: &[u16],
        width: u32,
        color: Color,
    ) {
        for (row_idx, &row_bits) in rows.iter().enumerate() {
            for col in 0..width as usize {
                let bit = (row_bits >> (width as usize - 1 - col)) & 1;
                if bit != 0 {
                    self.put_pixel(x + col as i32, y + row_idx as i32, color);
                }
            }
        }
    }

    /// Render ASCII text using the embedded 6×10 bitmap font.
    /// Returns the x position after the last character.
    pub fn draw_text(&mut self, x: i32, y: i32, text: &str, color: Color) -> i32 {
        let mut cx = x;
        for ch in text.chars() {
            cx = self.draw_char(cx, y, ch, color);
        }
        cx
    }

    /// Render a single character; returns x after the glyph.
    pub fn draw_char(&mut self, x: i32, y: i32, ch: char, color: Color) -> i32 {
        let glyph = font::glyph(ch);
        // Column-major format: each byte is one column, LSB = top row.
        for (col, &col_bits) in glyph.iter().enumerate() {
            for row in 0..font::GLYPH_H as usize {
                if (col_bits >> row) & 1 != 0 {
                    self.put_pixel(x + col as i32, y + row as i32, color);
                }
            }
        }
        x + font::GLYPH_W as i32 + 1
    }

    /// Draw a built-in UI symbol glyph; returns x after the glyph.
    pub fn draw_ui_symbol(&mut self, x: i32, y: i32, symbol: UiSymbol, color: Color) -> i32 {
        let glyph = ui_symbols::glyph(symbol);
        self.draw_glyph_rows(x, y, glyph.rows(), glyph.width as u32, color);
        x + glyph.advance as i32
    }

    pub fn measure_ui_symbol(symbol: UiSymbol) -> u32 {
        ui_symbols::glyph(symbol).advance as u32
    }

    pub fn draw_ui_symbol_centered(&mut self, rect: Rect, symbol: UiSymbol, color: Color) {
        let glyph = ui_symbols::glyph(symbol);
        let tx = rect.x + (rect.w as i32 - glyph.width as i32) / 2;
        let ty = rect.y + (rect.h as i32 - glyph.height as i32) / 2;
        self.draw_ui_symbol(tx, ty, symbol, color);
    }

    /// Measure pixel width of `text`.
    pub fn measure_text(text: &str) -> u32 {
        text.chars().count() as u32 * (font::GLYPH_W + 1)
    }

    /// Draw text centered in `rect`.
    pub fn draw_text_centered(&mut self, rect: Rect, text: &str, color: Color) {
        let tw = Self::measure_text(text);
        let th = font::GLYPH_H;
        let tx = rect.x + (rect.w as i32 - tw as i32) / 2;
        let ty = rect.y + (rect.h as i32 - th as i32) / 2;
        self.draw_text(tx, ty, text, color);
    }

    /// Draw text right-aligned inside `rect`, with optional right padding.
    pub fn draw_text_right(&mut self, rect: Rect, text: &str, color: Color, pad: i32) {
        let tw = Self::measure_text(text);
        let tx = rect.right() - tw as i32 - pad;
        let ty = rect.y + (rect.h as i32 - font::GLYPH_H as i32) / 2;
        self.draw_text(tx, ty, text, color);
    }

    /// Fill the canvas by scaling `img` to cover it (nearest-neighbour, no alloc).
    ///
    /// Both axes are scaled independently so the image fills the canvas exactly.
    /// For same-aspect-ratio images (e.g., a 16:9 wallpaper on a 16:9 screen)
    /// this is identical to cover/center-crop with no cropping required.
    pub fn draw_image_cover(&mut self, img: &crate::image::TgaImage) {
        let fw = self.width as usize;
        let fh = self.height as usize;
        let iw = img.width as usize;
        let ih = img.height as usize;
        if iw == 0 || ih == 0 || fw == 0 || fh == 0 {
            return;
        }
        for y in 0..fh {
            let src_y = (y * ih / fh) as u32;
            let row_off = y * self.stride as usize;
            for x in 0..fw {
                let src_x = (x * iw / fw) as u32;
                let idx = row_off + x;
                if idx < self.pixels.len() {
                    self.pixels[idx] = img.pixel_xrgb(src_x, src_y);
                }
            }
        }
    }

    /// Create a sub-canvas clipped to `rect`.
    /// NOTE: This is a zero-copy view — the sub-canvas writes into the same
    /// pixel buffer, using the original stride, just starting at a different offset.
    pub fn sub_canvas(&mut self, rect: Rect) -> Canvas<'_> {
        let x = rect.x.max(0) as u32;
        let y = rect.y.max(0) as u32;
        let w = rect.w.min(self.width.saturating_sub(x));
        let h = rect.h.min(self.height.saturating_sub(y));
        let offset = y as usize * self.stride as usize + x as usize;
        Canvas {
            pixels: &mut self.pixels[offset..],
            stride: self.stride,
            width: w,
            height: h,
        }
    }
}

// ── Embedded bitmap font (5 × 7 px, printable ASCII 0x20–0x7F) ───────────────
// Same font as the window-manager chrome (FONT_5X7 in sunlight-display).
// Format: column-major. Each glyph is 5 bytes, one per column (left→right).
// Within each byte, LSB = top pixel, MSB = bottom pixel.
pub mod font {
    pub const GLYPH_W: u32 = 5;
    pub const GLYPH_H: u32 = 7;

    /// Return the 5-byte column-major glyph for a character.
    pub fn glyph(ch: char) -> &'static [u8] {
        let idx = (ch as usize).saturating_sub(0x20).min(95);
        &FONT_DATA[idx]
    }

    // 96 glyphs (0x20 space … 0x7F DEL), 5 bytes each, column-major.
    // Identical to FONT_5X7 in services/sunlight-display/src/main.rs.
    static FONT_DATA: [[u8; 5]; 96] = [
        [0x00, 0x00, 0x00, 0x00, 0x00], // ' '
        [0x00, 0x00, 0x5F, 0x00, 0x00], // '!'
        [0x00, 0x07, 0x00, 0x07, 0x00], // '"'
        [0x14, 0x7F, 0x14, 0x7F, 0x14], // '#'
        [0x24, 0x2A, 0x7F, 0x2A, 0x12], // '$'
        [0x23, 0x13, 0x08, 0x64, 0x62], // '%'
        [0x36, 0x49, 0x55, 0x22, 0x50], // '&'
        [0x00, 0x05, 0x03, 0x00, 0x00], // '\''
        [0x00, 0x1C, 0x22, 0x41, 0x00], // '('
        [0x00, 0x41, 0x22, 0x1C, 0x00], // ')'
        [0x14, 0x08, 0x3E, 0x08, 0x14], // '*'
        [0x08, 0x08, 0x3E, 0x08, 0x08], // '+'
        [0x00, 0x50, 0x30, 0x00, 0x00], // ','
        [0x08, 0x08, 0x08, 0x08, 0x08], // '-'
        [0x00, 0x60, 0x60, 0x00, 0x00], // '.'
        [0x20, 0x10, 0x08, 0x04, 0x02], // '/'
        [0x3E, 0x51, 0x49, 0x45, 0x3E], // '0'
        [0x00, 0x42, 0x7F, 0x40, 0x00], // '1'
        [0x42, 0x61, 0x51, 0x49, 0x46], // '2'
        [0x21, 0x41, 0x45, 0x4B, 0x31], // '3'
        [0x18, 0x14, 0x12, 0x7F, 0x10], // '4'
        [0x27, 0x45, 0x45, 0x45, 0x39], // '5'
        [0x3C, 0x4A, 0x49, 0x49, 0x30], // '6'
        [0x01, 0x71, 0x09, 0x05, 0x03], // '7'
        [0x36, 0x49, 0x49, 0x49, 0x36], // '8'
        [0x06, 0x49, 0x49, 0x29, 0x1E], // '9'
        [0x00, 0x36, 0x36, 0x00, 0x00], // ':'
        [0x00, 0x56, 0x36, 0x00, 0x00], // ';'
        [0x08, 0x14, 0x22, 0x41, 0x00], // '<'
        [0x14, 0x14, 0x14, 0x14, 0x14], // '='
        [0x00, 0x41, 0x22, 0x14, 0x08], // '>'
        [0x02, 0x01, 0x51, 0x09, 0x06], // '?'
        [0x32, 0x49, 0x79, 0x41, 0x3E], // '@'
        [0x7E, 0x11, 0x11, 0x11, 0x7E], // 'A'
        [0x7F, 0x49, 0x49, 0x49, 0x36], // 'B'
        [0x3E, 0x41, 0x41, 0x41, 0x22], // 'C'
        [0x7F, 0x41, 0x41, 0x22, 0x1C], // 'D'
        [0x7F, 0x49, 0x49, 0x49, 0x41], // 'E'
        [0x7F, 0x09, 0x09, 0x09, 0x01], // 'F'
        [0x3E, 0x41, 0x49, 0x49, 0x7A], // 'G'
        [0x7F, 0x08, 0x08, 0x08, 0x7F], // 'H'
        [0x00, 0x41, 0x7F, 0x41, 0x00], // 'I'
        [0x20, 0x40, 0x41, 0x3F, 0x01], // 'J'
        [0x7F, 0x08, 0x14, 0x22, 0x41], // 'K'
        [0x7F, 0x40, 0x40, 0x40, 0x40], // 'L'
        [0x7F, 0x02, 0x0C, 0x02, 0x7F], // 'M'
        [0x7F, 0x04, 0x08, 0x10, 0x7F], // 'N'
        [0x3E, 0x41, 0x41, 0x41, 0x3E], // 'O'
        [0x7F, 0x09, 0x09, 0x09, 0x06], // 'P'
        [0x3E, 0x41, 0x51, 0x21, 0x5E], // 'Q'
        [0x7F, 0x09, 0x19, 0x29, 0x46], // 'R'
        [0x46, 0x49, 0x49, 0x49, 0x31], // 'S'
        [0x01, 0x01, 0x7F, 0x01, 0x01], // 'T'
        [0x3F, 0x40, 0x40, 0x40, 0x3F], // 'U'
        [0x1F, 0x20, 0x40, 0x20, 0x1F], // 'V'
        [0x3F, 0x40, 0x38, 0x40, 0x3F], // 'W'
        [0x63, 0x14, 0x08, 0x14, 0x63], // 'X'
        [0x07, 0x08, 0x70, 0x08, 0x07], // 'Y'
        [0x61, 0x51, 0x49, 0x45, 0x43], // 'Z'
        [0x00, 0x7F, 0x41, 0x41, 0x00], // '['
        [0x02, 0x04, 0x08, 0x10, 0x20], // '\\'
        [0x00, 0x41, 0x41, 0x7F, 0x00], // ']'
        [0x04, 0x02, 0x01, 0x02, 0x04], // '^'
        [0x40, 0x40, 0x40, 0x40, 0x40], // '_'
        [0x00, 0x01, 0x02, 0x04, 0x00], // '`'
        [0x20, 0x54, 0x54, 0x54, 0x78], // 'a'
        [0x7F, 0x48, 0x44, 0x44, 0x38], // 'b'
        [0x38, 0x44, 0x44, 0x44, 0x20], // 'c'
        [0x38, 0x44, 0x44, 0x48, 0x7F], // 'd'
        [0x38, 0x54, 0x54, 0x54, 0x18], // 'e'
        [0x08, 0x7E, 0x09, 0x01, 0x02], // 'f'
        [0x0C, 0x52, 0x52, 0x52, 0x3E], // 'g'
        [0x7F, 0x08, 0x04, 0x04, 0x78], // 'h'
        [0x00, 0x44, 0x7D, 0x40, 0x00], // 'i'
        [0x20, 0x40, 0x44, 0x3D, 0x00], // 'j'
        [0x7F, 0x10, 0x28, 0x44, 0x00], // 'k'
        [0x00, 0x41, 0x7F, 0x40, 0x00], // 'l'
        [0x7C, 0x04, 0x18, 0x04, 0x78], // 'm'
        [0x7C, 0x08, 0x04, 0x04, 0x78], // 'n'
        [0x38, 0x44, 0x44, 0x44, 0x38], // 'o'
        [0x7C, 0x14, 0x14, 0x14, 0x08], // 'p'
        [0x08, 0x14, 0x14, 0x18, 0x7C], // 'q'
        [0x7C, 0x08, 0x04, 0x04, 0x08], // 'r'
        [0x48, 0x54, 0x54, 0x54, 0x20], // 's'
        [0x04, 0x3F, 0x44, 0x40, 0x20], // 't'
        [0x3C, 0x40, 0x40, 0x20, 0x7C], // 'u'
        [0x1C, 0x20, 0x40, 0x20, 0x1C], // 'v'
        [0x3C, 0x40, 0x30, 0x40, 0x3C], // 'w'
        [0x44, 0x28, 0x10, 0x28, 0x44], // 'x'
        [0x0C, 0x50, 0x50, 0x50, 0x3C], // 'y'
        [0x44, 0x64, 0x54, 0x4C, 0x44], // 'z'
        [0x00, 0x08, 0x36, 0x41, 0x00], // '{'
        [0x00, 0x00, 0x7F, 0x00, 0x00], // '|'
        [0x00, 0x41, 0x36, 0x08, 0x00], // '}'
        [0x10, 0x08, 0x08, 0x10, 0x08], // '~'
        [0x00, 0x00, 0x00, 0x00, 0x00], // DEL
    ];
}
