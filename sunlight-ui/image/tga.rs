//! TGA type-2 (uncompressed true-colour) reader — no_std, zero-alloc.
//!
//! Detected format for the SunlightOS wallpaper:
//!   TGA type 2, 1672x941, 24 bpp (BGR), upper-left origin (image descriptor = 0x20,
//!   bit 5 = 1 means row 0 is the topmost row — no Y-flip needed).
//!   Header: 18 bytes. Color map: absent. Pixel data starts at offset 18.
//!   File size: 4,720,074 bytes (18 header + 1672 x 941 x 3 pixel bytes).

/// Errors returned by [`TgaImage::parse`].
#[derive(Debug, Clone, Copy)]
pub enum TgaError {
    TooShort,
    /// Image type is not 2 (uncompressed RGB/RGBA).
    UnsupportedType(u8),
    /// Pixel depth is not 24 or 32 bpp.
    UnsupportedDepth(u8),
}

/// Zero-alloc view over a `&'static [u8]` slice containing a TGA type-2 image.
///
/// Pixels are decoded on-demand via [`pixel_xrgb`]; no decoded buffer is stored.
/// The Y axis direction is derived from bit 5 of the image descriptor byte:
/// - bit 5 = 1 (upper-left origin): row 0 in the file = top of screen, no flip.
/// - bit 5 = 0 (lower-left origin): row 0 in the file = bottom of screen, flip applied.
#[derive(Clone, Copy)]
pub struct TgaImage {
    pub width: u32,
    pub height: u32,
    bpp: u8,
    /// True when bit 5 of image descriptor is set (upper-left / top-down order).
    top_down: bool,
    raw: &'static [u8],
    data_offset: u32,
}

impl TgaImage {
    /// Parse a TGA type-2 header from a `'static` byte slice.
    ///
    /// Only type 2 (uncompressed RGB / RGBA) at 24 or 32 bpp is accepted.
    /// On success the returned `TgaImage` borrows the same static slice.
    pub fn parse(data: &'static [u8]) -> Result<Self, TgaError> {
        if data.len() < 18 {
            return Err(TgaError::TooShort);
        }
        let id_len = data[0] as u32;
        let img_type = data[2];
        if img_type != 2 {
            return Err(TgaError::UnsupportedType(img_type));
        }
        let bpp = data[16];
        if bpp != 24 && bpp != 32 {
            return Err(TgaError::UnsupportedDepth(bpp));
        }
        let width = u16::from_le_bytes([data[12], data[13]]) as u32;
        let height = u16::from_le_bytes([data[14], data[15]]) as u32;
        // Image descriptor byte: bit 5 = screen origin (0=lower-left, 1=upper-left).
        let top_down = (data[17] & 0x20) != 0;
        // Color map: bytes 3-7 (first_entry_idx 2B, length 2B, entry_size 1B).
        let cm_len = u16::from_le_bytes([data[5], data[6]]) as u32;
        let cm_entry_bits = data[7] as u32;
        let cm_bytes = if data[1] != 0 {
            cm_len * ((cm_entry_bits + 7) / 8)
        } else {
            0
        };
        let data_offset = 18 + id_len + cm_bytes;
        Ok(Self { width, height, bpp, top_down, raw: data, data_offset })
    }

    /// Returns the XRGB8888 pixel at image coordinates `(x, y)`.
    ///
    /// `y = 0` is always the **top** row in display order regardless of the
    /// stored origin.  Out-of-bounds coordinates are clamped to the image edges.
    #[inline]
    pub fn pixel_xrgb(&self, x: u32, y: u32) -> u32 {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        // Remap y to the file row according to origin flag.
        let file_row = if self.top_down {
            y
        } else {
            self.height - 1 - y
        };
        let bpp_bytes = (self.bpp / 8) as u32;
        let idx = (self.data_offset + (file_row * self.width + x) * bpp_bytes) as usize;
        if idx + 2 >= self.raw.len() {
            return 0;
        }
        let b = self.raw[idx] as u32;
        let g = self.raw[idx + 1] as u32;
        let r = self.raw[idx + 2] as u32;
        (r << 16) | (g << 8) | b
    }
}
