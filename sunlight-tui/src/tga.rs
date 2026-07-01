//! Minimal TGA type-2 (uncompressed true-color) decoder — no_std, no heap.
//!
//! Supports 24-bit (BGR) and 32-bit (BGRA) TGA images with upper-left or
//! lower-left origin.  Provides aspect-fill rendering onto a raw framebuffer
//! with an optional dark overlay.

use crate::framebuffer::Framebuffer;

/// Decoded TGA image header, borrowed from the original byte slice.
pub struct TgaImage<'a> {
    raw: &'a [u8],
    width: u32,
    height: u32,
    bpp: u8,
    top_down: bool,
    data_off: u32,
}

impl<'a> TgaImage<'a> {
    /// Parse a TGA type-2 (uncompressed RGB/RGBA) header.
    ///
    /// Returns `None` if the data is too short, the image type is not 2, or
    /// the pixel depth is neither 24 nor 32 bpp.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 18 {
            return None;
        }
        let img_type = data[2];
        if img_type != 2 {
            return None;
        }
        let bpp = data[16];
        if bpp != 24 && bpp != 32 {
            return None;
        }
        let width = u16::from_le_bytes([data[12], data[13]]) as u32;
        let height = u16::from_le_bytes([data[14], data[15]]) as u32;
        if width == 0 || height == 0 {
            return None;
        }
        let top_down = (data[17] & 0x20) != 0;
        let id_len = data[0] as u32;
        let cm_present = data[1] != 0;
        let cm_bytes = if cm_present {
            let cm_len = u16::from_le_bytes([data[5], data[6]]) as u32;
            let cm_entry_bits = data[7] as u32;
            cm_len * ((cm_entry_bits + 7) / 8)
        } else {
            0
        };
        let data_off = 18 + id_len + cm_bytes;
        if data_off as usize >= data.len() {
            return None;
        }
        Some(Self {
            raw: data,
            width,
            height,
            bpp,
            top_down,
            data_off,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Read the pixel at image coordinates `(px, py)` as XRGB8888.
    ///
    /// `py = 0` is always the top row in display order.
    /// Out-of-bounds coordinates are clamped to the image edges.
    pub fn pixel(&self, mut px: u32, mut py: u32) -> u32 {
        px = px.min(self.width.saturating_sub(1));
        py = py.min(self.height.saturating_sub(1));
        let file_row = if self.top_down {
            py
        } else {
            self.height - 1 - py
        };
        let bpp_bytes = (self.bpp / 8) as u32;
        let idx = (self.data_off + (file_row * self.width + px) * bpp_bytes) as usize;
        if idx + 2 >= self.raw.len() {
            return 0;
        }
        let b = self.raw[idx] as u32;
        let g = self.raw[idx + 1] as u32;
        let r = self.raw[idx + 2] as u32;
        (r << 16) | (g << 8) | b
    }

    /// Read pixel + alpha at (px, py). Returns (XRGB8888, alpha).
    /// Alpha is always 0xFF for 24-bit (opaque) images.
    pub fn pixel_alpha(&self, mut px: u32, mut py: u32) -> (u32, u8) {
        px = px.min(self.width.saturating_sub(1));
        py = py.min(self.height.saturating_sub(1));
        let file_row = if self.top_down {
            py
        } else {
            self.height - 1 - py
        };
        let bpp_bytes = (self.bpp / 8) as u32;
        let idx = (self.data_off + (file_row * self.width + px) * bpp_bytes) as usize;
        if idx + 2 >= self.raw.len() {
            return (0, 0);
        }
        let b = self.raw[idx] as u32;
        let g = self.raw[idx + 1] as u32;
        let r = self.raw[idx + 2] as u32;
        let a = if self.bpp == 32 && idx + 3 < self.raw.len() {
            self.raw[idx + 3]
        } else {
            0xFF
        };
        ((r << 16) | (g << 8) | b, a)
    }
}

/// Draw a TGA icon tinted with a flat color using the icon's alpha channel as a mask.
///
/// Every opaque pixel (alpha >= 128) is painted with `tint_color` regardless of the
/// icon's original RGB values.  This makes monochrome icons work correctly in dark UIs:
/// pass muted gray for inactive state, SunlightOS orange for focused/selected state.
///
/// Falls back to a placeholder outline in `tint_color` when `img` is `None`.
pub fn draw_tga_icon_tinted(
    fb: &mut Framebuffer,
    img: Option<&TgaImage<'_>>,
    x: u32,
    y: u32,
    target_w: u32,
    target_h: u32,
    tint_color: u32,
) {
    let Some(img) = img else {
        crate::draw::rect_outline(fb, x, y, target_w, target_h, 1, tint_color);
        return;
    };
    let src_w = img.width();
    let src_h = img.height();
    if src_w == 0 || src_h == 0 || target_w == 0 || target_h == 0 {
        return;
    }
    for dy in 0..target_h {
        let sy = (dy as u64 * src_h as u64 / target_h as u64) as u32;
        for dx in 0..target_w {
            let sx = (dx as u64 * src_w as u64 / target_w as u64) as u32;
            let (_, alpha) = img.pixel_alpha(sx, sy);
            if alpha >= 128 {
                fb.put_pixel(x + dx, y + dy, tint_color);
            }
        }
    }
}

/// Draw a TGA icon scaled to `target_w × target_h` at position `(x, y)`.
///
/// Pixels with alpha < 128 are treated as transparent and skipped.
/// When `img` is `None`, draws a simple placeholder outline using `fallback_color`.
///
/// Framebuffer login screen uses this for user avatars and action buttons.
/// Icon sources: docs/icons/SunlightOS/actions/32/
pub fn draw_tga_icon(
    fb: &mut Framebuffer,
    img: Option<&TgaImage<'_>>,
    x: u32,
    y: u32,
    target_w: u32,
    target_h: u32,
    fallback_color: u32,
) {
    let Some(img) = img else {
        crate::draw::rect_outline(fb, x, y, target_w, target_h, 1, fallback_color);
        return;
    };
    let src_w = img.width();
    let src_h = img.height();
    if src_w == 0 || src_h == 0 || target_w == 0 || target_h == 0 {
        return;
    }
    for dy in 0..target_h {
        let sy = (dy as u64 * src_h as u64 / target_h as u64) as u32;
        for dx in 0..target_w {
            let sx = (dx as u64 * src_w as u64 / target_w as u64) as u32;
            let (rgb, alpha) = img.pixel_alpha(sx, sy);
            if alpha >= 128 {
                fb.put_pixel(x + dx, y + dy, rgb);
            }
        }
    }
}

/// Draw `img` onto `fb` with aspect-fill behaviour:
///   - preserve aspect ratio
///   - fill the screen (crop if necessary)
///   - centre the visible area
///
/// After drawing the image a translucent black overlay is applied
/// (`overlay_alpha` in 0–255, where 128 ≈ 50 % opaque).
pub fn draw_tga_background(fb: &mut Framebuffer, img: &TgaImage, overlay_alpha: u8) {
    let fb_w = fb.width();
    let fb_h = fb.height();
    if fb_w == 0 || fb_h == 0 {
        return;
    }
    let img_w = img.width();
    let img_h = img.height();
    if img_w == 0 || img_h == 0 {
        return;
    }

    // Overlay factor: bg_factor = 256 - overlay_alpha.
    // Each channel is multiplied by bg_factor/256.
    let bg_factor = 256u32.saturating_sub(overlay_alpha as u32);

    // Determine aspect-fill strategy with cross-multiplication (no floats).
    // img_w/fb_w > img_h/fb_h  ⇔  img_w * fb_h > fb_w * img_h
    let fit_height = (img_w as u64) * (fb_h as u64) > (fb_w as u64) * (img_h as u64);

    if fit_height {
        // Image is proportionally wider than the screen → fit by height.
        // Source Y maps linearly, source X is cropped and centred.
        //
        //   sy   = dy * img_h / fb_h
        //   sx   = off_x + dx * crop_w / fb_w
        //   where:
        //     crop_w = fb_w * img_h / fb_h   (source width that fills the screen)
        //     off_x  = (img_w - crop_w) / 2   (crop offset, centred)
        let crop_w = (fb_w as u64) * (img_h as u64) / (fb_h as u64);
        let off_x = (img_w as u64).saturating_sub(crop_w) / 2;

        for dy in 0..fb_h {
            let sy = ((dy as u64) * (img_h as u64) / (fb_h as u64)) as u32;
            for dx in 0..fb_w {
                let sx = (off_x + (dx as u64) * crop_w / (fb_w as u64)) as u32;
                let p = img.pixel(sx, sy);
                let r = ((p >> 16) & 0xFF) * bg_factor / 256;
                let g = ((p >> 8) & 0xFF) * bg_factor / 256;
                let b = (p & 0xFF) * bg_factor / 256;
                fb.put_pixel(dx, dy, (r << 16) | (g << 8) | b);
            }
        }
    } else {
        // Image is proportionally taller or matches → fit by width.
        // Source X maps linearly, source Y is cropped and centred.
        //
        //   sx   = dx * img_w / fb_w
        //   sy   = off_y + dy * crop_h / fb_h
        //   where:
        //     crop_h = fb_h * img_w / fb_w
        //     off_y  = (img_h - crop_h) / 2
        let crop_h = (fb_h as u64) * (img_w as u64) / (fb_w as u64);
        let off_y = (img_h as u64).saturating_sub(crop_h) / 2;

        for dy in 0..fb_h {
            let sy = (off_y + (dy as u64) * crop_h / (fb_h as u64)) as u32;
            for dx in 0..fb_w {
                let sx = ((dx as u64) * (img_w as u64) / (fb_w as u64)) as u32;
                let p = img.pixel(sx, sy);
                let r = ((p >> 16) & 0xFF) * bg_factor / 256;
                let g = ((p >> 8) & 0xFF) * bg_factor / 256;
                let b = (p & 0xFF) * bg_factor / 256;
                fb.put_pixel(dx, dy, (r << 16) | (g << 8) | b);
            }
        }
    }
}
