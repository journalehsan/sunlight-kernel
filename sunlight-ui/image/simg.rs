//! Owned (heap-backed) SIMG/TGA decoder + nearest-neighbour scaler.
//!
//! `image::tga::TgaImage` is a zero-alloc view over a `&'static [u8]`, which is
//! ideal for `include_bytes!` assets but cannot decode files loaded at runtime
//! (e.g. thumbnails cached on disk). This module provides the same TGA type-2
//! decoding logic operating on a borrowed `&[u8]`, producing an owned RGBA
//! pixel buffer. It is shared by the thumbnail daemon (`sunlight-thumbd`) and
//! any code path that must decode an image whose lifetime is not `'static`.
//!
//! ## "SIMG" format note
//! SunlightOS sample pictures and thumbnails use the `.simg` extension, but the
//! on-disk bytes are **uncompressed TGA type-2** (the only bitmap format the OS
//! can decode). So a `.simg` file and a `.tga` file are decoded identically.
//! See `image::tga` for the byte-level format description.
//!
//! The decoder is intentionally defensive: malformed or truncated inputs return
//! `Err` rather than panicking, so a broken thumbnail file can never take down
//! the File Manager or the thumbnail daemon.

use alloc::vec::Vec;

/// Errors returned by [`decode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Fewer than 18 header bytes.
    TooShort,
    /// Image type is not 2 (uncompressed true-colour).
    UnsupportedType(u8),
    /// Pixel depth is not 24 or 32 bpp.
    UnsupportedDepth(u8),
    /// Header claims dimensions whose pixel payload does not fit in `data`.
    Truncated,
}

/// Decoded image: width, height, and packed ARGB8888 pixels (top-down).
///
/// Each pixel is `(a << 24) | (r << 16) | (g << 8) | b`. Row 0 is the top row.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

impl RgbaImage {
    /// ARGB pixel at display coordinates `(x, y)` with `y = 0` at the top.
    /// Out-of-bounds coordinates are clamped.
    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> u32 {
        if self.width == 0 || self.height == 0 {
            return 0;
        }
        let x = x.min(self.width - 1);
        let y = y.min(self.height - 1);
        self.pixels[(y as usize) * (self.width as usize) + (x as usize)]
    }

    pub fn pixel_count(&self) -> usize {
        (self.width as usize) * (self.height as usize)
    }
}

/// Decode an uncompressed TGA type-2 image (24 or 32 bpp) from `data`.
///
/// This is the reusable SIMG/TGA decoder. Both `.simg` and `.tga` extensions
/// map to this path. Handles the image-origin bit so the returned buffer is
/// always top-down regardless of how the file stored its rows.
pub fn decode(data: &[u8]) -> Result<RgbaImage, DecodeError> {
    if data.len() < 18 {
        return Err(DecodeError::TooShort);
    }
    let id_len = data[0] as u32;
    let img_type = data[2];
    if img_type != 2 {
        return Err(DecodeError::UnsupportedType(img_type));
    }
    let bpp = data[16];
    if bpp != 24 && bpp != 32 {
        return Err(DecodeError::UnsupportedDepth(bpp));
    }
    let width = u16::from_le_bytes([data[12], data[13]]) as u32;
    let height = u16::from_le_bytes([data[14], data[15]]) as u32;
    // bit 5 = 1 -> upper-left origin (top-down); 0 -> lower-left (bottom-up).
    let top_down = (data[17] & 0x20) != 0;

    let cm_len = u16::from_le_bytes([data[5], data[6]]) as u32;
    let cm_entry_bits = data[7] as u32;
    let cm_bytes = if data[1] != 0 {
        cm_len * ((cm_entry_bits + 7) / 8)
    } else {
        0
    };
    let data_offset = (18 + id_len + cm_bytes) as usize;

    if width == 0 || height == 0 {
        return Err(DecodeError::Truncated);
    }
    let bpp_bytes = (bpp / 8) as u32;
    let pixel_count = (width as usize) * (height as usize);
    let needed = data_offset + pixel_count * (bpp_bytes as usize);
    if data.len() < needed {
        return Err(DecodeError::Truncated);
    }

    let mut pixels = Vec::with_capacity(pixel_count);
    for dy in 0..height {
        let file_row = if top_down { dy } else { height - 1 - dy };
        let row_base = data_offset + (file_row as usize) * (width as usize) * (bpp_bytes as usize);
        for dx in 0..width {
            let idx = row_base + (dx as usize) * (bpp_bytes as usize);
            let b = data[idx] as u32;
            let g = data[idx + 1] as u32;
            let r = data[idx + 2] as u32;
            let a = if bpp == 32 {
                data[idx + 3] as u32
            } else {
                0xFF
            };
            pixels.push((a << 24) | (r << 16) | (g << 8) | b);
        }
    }

    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

/// Scale `src` so it fits within `(max_w, max_h)` while preserving aspect
/// ratio. Uses nearest-neighbour sampling (v0 scaler).
///
/// Returns a new owned image; the source is unchanged.
pub fn scale_fit(src: &RgbaImage, max_w: u32, max_h: u32) -> RgbaImage {
    if max_w == 0 || max_h == 0 || src.width == 0 || src.height == 0 {
        return RgbaImage {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };
    }
    let (tw, th) = fit_dimensions(src.width, src.height, max_w, max_h);
    let mut out = Vec::with_capacity((tw as usize) * (th as usize));
    for dy in 0..th {
        let sy = (dy as u64 * src.height as u64 / th as u64) as u32;
        for dx in 0..tw {
            let sx = (dx as u64 * src.width as u64 / tw as u64) as u32;
            out.push(src.pixel(sx, sy));
        }
    }
    RgbaImage {
        width: tw,
        height: th,
        pixels: out,
    }
}

/// Compute `(w, h)` that fits inside `(max_w, max_h)` preserving aspect ratio.
pub fn fit_dimensions(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 || max_w == 0 || max_h == 0 {
        return (0, 0);
    }
    // Pick the smaller of the two axis scales via cross-multiplication.
    let scale_w = max_w as u64 * h as u64;
    let scale_h = max_h as u64 * w as u64;
    let (tw, th) = if scale_w <= scale_h {
        (max_w, (max_w as u64 * h as u64 / w as u64) as u32)
    } else {
        ((max_h as u64 * w as u64 / h as u64) as u32, max_h)
    };
    (tw.max(1), th.max(1))
}

/// Encode an [`RgbaImage`] as an uncompressed TGA type-2 (24 bpp BGR) byte
/// vector — the canonical SIMG/thumbnail container the File Manager decodes.
/// Top-down origin (descriptor `0x20`) is used so blitters need no Y-flip.
pub fn encode_tga_type2_bgr24(img: &RgbaImage) -> Vec<u8> {
    let pixel_count = img.pixel_count();
    let mut out = Vec::with_capacity(18 + pixel_count * 3);
    out.extend_from_slice(&[
        0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    out.extend_from_slice(&(img.width as u16).to_le_bytes());
    out.extend_from_slice(&(img.height as u16).to_le_bytes());
    out.push(24);
    out.push(0x20);
    for px in &img.pixels {
        out.push((*px) as u8);
        out.push((*px >> 8) as u8);
        out.push((*px >> 16) as u8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_tga() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&[
            0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        v.extend_from_slice(&3u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.push(24);
        v.push(0x20);
        let px: [[u8; 3]; 6] = [
            [0, 0, 255],
            [0, 255, 0],
            [255, 0, 0],
            [255, 255, 255],
            [0, 0, 0],
            [128, 128, 128],
        ];
        for p in px {
            v.extend_from_slice(&p);
        }
        v
    }

    #[test]
    fn decode_tiny_tga() {
        let img = decode(&tiny_tga()).expect("decode");
        assert_eq!(img.width, 3);
        assert_eq!(img.height, 2);
        assert_eq!(img.pixel(0, 0), 0xFFFF0000);
        assert_eq!(img.pixel(2, 0), 0xFF0000FF);
        assert_eq!(img.pixel(0, 1), 0xFFFFFFFF);
    }

    #[test]
    fn malformed_rejects_safely() {
        assert_eq!(decode(&[0u8; 5]), Err(DecodeError::TooShort));
        let mut bad = tiny_tga();
        bad[2] = 10;
        assert_eq!(decode(&bad), Err(DecodeError::UnsupportedType(10)));
        let mut bad2 = tiny_tga();
        bad2[16] = 16;
        assert_eq!(decode(&bad2), Err(DecodeError::UnsupportedDepth(16)));
        let mut trunc = tiny_tga();
        trunc.truncate(18);
        assert_eq!(decode(&trunc), Err(DecodeError::Truncated));
    }

    #[test]
    fn origin_bit_flips_rows() {
        let mut bottom_up = tiny_tga();
        bottom_up[17] = 0x00;
        let img = decode(&bottom_up).expect("decode");
        assert_eq!(img.pixel(0, 1), 0xFFFF0000);
        assert_eq!(img.pixel(0, 0), 0xFFFFFFFF);
    }

    #[test]
    fn scale_preserves_aspect_ratio() {
        let img = decode(&tiny_tga()).unwrap();
        let s = scale_fit(&img, 100, 100);
        assert_eq!(s.width, 100);
        assert!((s.height as i32 - 66).abs() <= 1, "h={}", s.height);
    }

    #[test]
    fn encode_roundtrip() {
        let img = decode(&tiny_tga()).unwrap();
        let bytes = encode_tga_type2_bgr24(&img);
        let img2 = decode(&bytes).expect("roundtrip decode");
        assert_eq!(img2.width, img.width);
        assert_eq!(img2.height, img.height);
        assert_eq!(img2.pixel(0, 0), img.pixel(0, 0));
    }
}
