//! Low-level framebuffer paint helpers.
//!
//! All drawing is done into a `Canvas` — a mutable view over a pixel slice
//! with a known stride. Widgets call these helpers; nothing here knows about
//! themes or widget state.

use crate::font::ui_symbols;
use crate::font::UiSymbol;
use crate::geom::Rect;
use crate::material::Material;
use crate::theme::Color;

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

    /// Replace a region with transparent pixels.
    ///
    /// This is intended for explicitly alpha-capable native surfaces. Legacy
    /// XRGB windows must continue painting an opaque root.
    pub fn clear_transparent(&mut self, rect: Rect) {
        self.fill_rect(rect, Color::TRANSPARENT);
    }

    /// Alpha-composite a rectangle over the existing pixels.
    pub fn blend_rect(&mut self, rect: Rect, color: Color) {
        let x0 = rect.x.max(0) as u32;
        let y0 = rect.y.max(0) as u32;
        let x1 = (rect.right()).min(self.width as i32).max(0) as u32;
        let y1 = (rect.bottom()).min(self.height as i32).max(0) as u32;

        for y in y0..y1 {
            let row_start = y as usize * self.stride as usize;
            for x in x0..x1 {
                let idx = row_start + x as usize;
                self.pixels[idx] = color.blend_over(Color(self.pixels[idx])).0;
            }
        }
    }

    /// Fill `rect` with a reusable [`Material`] (solid / tinted / glass).
    ///
    /// Glass uses the static noise tile; no per-frame allocation. Fully opaque
    /// solids take the fast `fill_rect` path. Noise is skipped when strength is 0.
    pub fn fill_material(&mut self, rect: Rect, material: Material) {
        let m = material.clamp();
        let radius = m.radius;
        match m.kind {
            crate::material::MaterialKind::Solid if radius == 0 => {
                self.fill_rect(rect, m.tint);
            }
            crate::material::MaterialKind::Solid => {
                self.fill_rounded_rect(rect, radius, m.tint);
            }
            crate::material::MaterialKind::Tinted if m.opacity == 255 && m.noise_strength == 0 => {
                if radius == 0 {
                    self.fill_rect(rect, m.tint);
                } else {
                    self.fill_rounded_rect(rect, radius, m.tint);
                }
            }
            _ => {
                // Per-pixel sample for glass / partial opacity / noise.
                let x0 = rect.x.max(0);
                let y0 = rect.y.max(0);
                let x1 = rect.right().min(self.width as i32).max(0);
                let y1 = rect.bottom().min(self.height as i32).max(0);
                if x0 >= x1 || y0 >= y1 {
                    // still may draw border below
                } else if radius == 0 {
                    for y in y0..y1 {
                        let row_start = y as usize * self.stride as usize;
                        for x in x0..x1 {
                            let idx = row_start + x as usize;
                            if idx >= self.pixels.len() {
                                continue;
                            }
                            let src = m.sample_color(x, y);
                            self.pixels[idx] = src.blend_over(Color(self.pixels[idx])).0;
                        }
                    }
                } else {
                    // Rounded: sample only inside the rounded shape via coverage-ish
                    // fill by reusing rounded fill with a solid then noise is hard;
                    // approximate with blend_rounded for the base tint, then light
                    // noise pass inset by 1.
                    let base = Color::rgba(m.tint.r(), m.tint.g(), m.tint.b(), m.opacity);
                    self.blend_rounded_rect(rect, radius, base);
                    if matches!(m.kind, crate::material::MaterialKind::Glass)
                        && m.noise_strength > 0
                    {
                        let inset = rect.inset(1);
                        let ix0 = inset.x.max(0);
                        let iy0 = inset.y.max(0);
                        let ix1 = inset.right().min(self.width as i32).max(0);
                        let iy1 = inset.bottom().min(self.height as i32).max(0);
                        for y in iy0..iy1 {
                            let row_start = y as usize * self.stride as usize;
                            for x in ix0..ix1 {
                                let idx = row_start + x as usize;
                                if idx >= self.pixels.len() {
                                    continue;
                                }
                                // Apply only the noise delta as a faint overlay.
                                let n = crate::material::noise_sample(x, y);
                                let delta = ((n as i16 - 128) * m.noise_strength as i16) / 255;
                                if delta == 0 {
                                    continue;
                                }
                                let dst = Color(self.pixels[idx]);
                                let r = (dst.r() as i16 + delta).clamp(0, 255) as u8;
                                let g = (dst.g() as i16 + delta).clamp(0, 255) as u8;
                                let b = (dst.b() as i16 + delta).clamp(0, 255) as u8;
                                // Noise changes only straight RGB. Preserve the
                                // material's alpha so a rounded glass surface
                                // never becomes accidentally opaque.
                                self.pixels[idx] = Color::rgba(r, g, b, dst.a()).0;
                            }
                        }
                    }
                }
            }
        }
        if let Some(border) = m.border {
            if radius == 0 {
                self.draw_rect(rect, border);
            } else {
                self.stroke_rounded_rect(rect, radius, 1, border);
            }
        }
    }

    /// Replace `rect` with a material instead of compositing over the previous
    /// frame. This avoids alpha accumulation when an app repaints the root of
    /// an opt-in translucent surface.
    pub fn paint_material(&mut self, rect: Rect, material: Material) {
        self.clear_transparent(rect);
        self.fill_material(rect, material);
    }

    /// Draw text with an optional 1-px ambient shadow (glass readability only).
    pub fn draw_text_on_material(
        &mut self,
        x: i32,
        y: i32,
        text: &str,
        color: Color,
        material: Material,
    ) -> i32 {
        if material.wants_text_ambient_shadow() {
            self.draw_text(x + 1, y + 1, text, Color::rgba(0, 0, 0, 140));
        }
        self.draw_text(x, y, text, color)
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
    pub fn draw_glyph_rows(&mut self, x: i32, y: i32, rows: &[u16], width: u32, color: Color) {
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
    /// Blit a TGA icon scaled to `dst`, with alpha compositing.
    ///
    /// The source image is nearest-neighbour scaled from its native resolution to
    /// `dst.w × dst.h`.  Fully-transparent pixels are skipped; partially-
    /// transparent pixels are blended over whatever is already in the canvas.
    /// Clipping against canvas bounds is applied automatically.
    pub fn draw_tga_icon(&mut self, img: &crate::image::TgaImage, dst: Rect) {
        if img.width == 0 || img.height == 0 {
            return;
        }
        let cx0 = dst.x.max(0) as u32;
        let cy0 = dst.y.max(0) as u32;
        let cx1 = (dst.right() as u32).min(self.width);
        let cy1 = (dst.bottom() as u32).min(self.height);
        if cx0 >= cx1 || cy0 >= cy1 {
            return;
        }
        let dw = (dst.right() - dst.x.max(0)).max(1) as u32;
        let dh = (dst.bottom() - dst.y.max(0)).max(1) as u32;

        for dy in cy0..cy1 {
            let src_y = (dy - cy0) * img.height / dh;
            let row_off = dy as usize * self.stride as usize;
            for dx in cx0..cx1 {
                let src_x = (dx - cx0) * img.width / dw;
                let argb = img.pixel_argb(src_x, src_y);
                let a = (argb >> 24) as u8;
                if a == 0 {
                    continue;
                }
                let idx = row_off + dx as usize;
                if idx >= self.pixels.len() {
                    continue;
                }
                if a == 255 {
                    self.pixels[idx] = argb;
                } else {
                    self.pixels[idx] = Color(argb).blend_over(Color(self.pixels[idx])).0;
                }
            }
        }
    }

    /// Draw a TGA icon (typically white+alpha from Material Icons raster) tinted
    /// to a solid `tint` color using the source alpha mask. This produces clean
    /// monochrome icons that match theme.icon_foreground / accent etc. without
    /// baking color into the asset. Reduces reliance on colored bitmaps → lower RAM.
    pub fn draw_tga_icon_tinted(&mut self, img: &crate::image::TgaImage, dst: Rect, tint: Color) {
        if img.width == 0 || img.height == 0 {
            return;
        }
        let cx0 = dst.x.max(0) as u32;
        let cy0 = dst.y.max(0) as u32;
        let cx1 = (dst.right() as u32).min(self.width);
        let cy1 = (dst.bottom() as u32).min(self.height);
        if cx0 >= cx1 || cy0 >= cy1 {
            return;
        }
        let dw = (dst.right() - dst.x.max(0)).max(1) as u32;
        let dh = (dst.bottom() - dst.y.max(0)).max(1) as u32;

        for dy in cy0..cy1 {
            let src_y = (dy - cy0) * img.height / dh;
            let row_off = dy as usize * self.stride as usize;
            for dx in cx0..cx1 {
                let src_x = (dx - cx0) * img.width / dw;
                let argb = img.pixel_argb(src_x, src_y);
                let a = (argb >> 24) as u8;
                if a == 0 {
                    continue;
                }
                let idx = row_off + dx as usize;
                if idx >= self.pixels.len() {
                    continue;
                }
                let source_alpha = ((a as u32 * tint.a() as u32 + 127) / 255) as u8;
                let source = Color::rgba(tint.r(), tint.g(), tint.b(), source_alpha);
                self.pixels[idx] = source.blend_over(Color(self.pixels[idx])).0;
            }
        }
    }

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
                    // Cover images are opaque content. Preserve that fact in
                    // ARGB-capable native surfaces instead of relying on the
                    // historical compositor behavior of ignoring the high byte.
                    self.pixels[idx] = 0xFF00_0000 | img.pixel_xrgb(src_x, src_y);
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

#[cfg(test)]
mod material_tests {
    use super::*;
    use crate::image::TgaImage;
    use crate::material::Material;

    static HALF_RED_TGA: [u8; 22] = [
        0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 0x20, 0, 0, 255, 128,
    ];
    static OPAQUE_RED_TGA: [u8; 22] = [
        0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 0x20, 0, 0, 255, 255,
    ];
    static OPAQUE_BLUE_24_TGA: [u8; 21] = [
        0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 24, 0x20, 255, 0, 0,
    ];

    #[test]
    fn paint_material_replaces_previous_frame_alpha() {
        let mut pixels = [0xFFFF_FFFF; 16];
        let mut canvas = Canvas::new(&mut pixels, 4, 4, 4);
        let glass = Material::glass(Color::rgb(20, 30, 40), 200).with_noise(0);
        canvas.paint_material(Rect::new(0, 0, 4, 4), glass);
        assert!(pixels.iter().all(|pixel| (pixel >> 24) == 200));
    }

    #[test]
    fn rounded_glass_noise_never_forces_pixels_opaque() {
        let mut pixels = [0u32; 64];
        let mut canvas = Canvas::new(&mut pixels, 8, 8, 8);
        let glass = Material::glass(Color::rgb(20, 30, 40), 200)
            .with_noise(5)
            .with_radius(3);
        canvas.fill_material(Rect::new(0, 0, 8, 8), glass);
        assert_eq!(pixels[4 * 8 + 4] >> 24, 200);
        assert!(pixels.iter().all(|pixel| (pixel >> 24) <= 200));
    }

    #[test]
    fn tga_icon_keeps_straight_alpha_on_transparent_canvas() {
        let image = TgaImage::parse(&HALF_RED_TGA).unwrap();
        let mut pixels = [0u32; 1];
        let mut canvas = Canvas::new(&mut pixels, 1, 1, 1);
        canvas.draw_tga_icon(&image, Rect::new(0, 0, 1, 1));
        assert_eq!(pixels[0], 0x80FF_0000);
    }

    #[test]
    fn tga_icon_over_opaque_content_remains_opaque() {
        let image = TgaImage::parse(&HALF_RED_TGA).unwrap();
        let mut pixels = [0xFF00_00FFu32; 1];
        let mut canvas = Canvas::new(&mut pixels, 1, 1, 1);
        canvas.draw_tga_icon(&image, Rect::new(0, 0, 1, 1));
        assert_eq!(pixels[0], 0xFF80_007F);
    }

    #[test]
    fn opaque_and_tinted_tga_paths_never_drop_alpha() {
        let opaque = TgaImage::parse(&OPAQUE_RED_TGA).unwrap();
        let masked = TgaImage::parse(&HALF_RED_TGA).unwrap();
        let mut pixels = [0u32; 2];
        let mut canvas = Canvas::new(&mut pixels, 2, 2, 1);
        canvas.draw_tga_icon(&opaque, Rect::new(0, 0, 1, 1));
        canvas.draw_tga_icon_tinted(
            &masked,
            Rect::new(1, 0, 1, 1),
            Color::rgb(20, 40, 60),
        );
        assert_eq!(pixels[0], 0xFFFF_0000);
        assert_eq!(pixels[1], 0x8014_283C);
    }

    #[test]
    fn cover_image_is_explicitly_opaque() {
        let image = TgaImage::parse(&OPAQUE_BLUE_24_TGA).unwrap();
        let mut pixels = [0u32; 1];
        let mut canvas = Canvas::new(&mut pixels, 1, 1, 1);
        canvas.draw_image_cover(&image);
        assert_eq!(pixels[0], 0xFF00_00FF);
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
