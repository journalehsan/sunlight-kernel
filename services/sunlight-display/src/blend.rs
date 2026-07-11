/// Maximum sane dimension for client surface allocation.
pub const MAX_SURFACE_DIM: u32 = 8192;
/// Maximum surface allocation in bytes (8192 × 8192 × 4 = 268 MiB).
pub const MAX_SURFACE_BYTES: usize = MAX_SURFACE_DIM as usize * MAX_SURFACE_DIM as usize * 4;

/// Source-over coverage blend. `src` and `dst` are XRGB pixels (A byte ignored).
/// `src_alpha` is the coverage/opacity of the source pixel (0=transparent, 255=opaque).
///
/// This blends the source RGB over the destination RGB with the given opacity,
/// ignoring the source pixel's alpha channel. Useful for rounded-corner masking
/// and window chrome where per-pixel alpha is supplied externally.
#[inline(always)]
pub fn blend_pixel(src: u32, dst: u32, src_alpha: u8) -> u32 {
    match src_alpha {
        255 => src,
        0 => dst,
        a => {
            let a = a as u32;
            let ia = 255 - a;
            let r = ((src >> 16 & 0xFF) * a + (dst >> 16 & 0xFF) * ia + 127) / 255;
            let g = ((src >> 8 & 0xFF) * a + (dst >> 8 & 0xFF) * ia + 127) / 255;
            let b = ((src & 0xFF) * a + (dst & 0xFF) * ia + 127) / 255;
            (r << 16) | (g << 8) | b
        }
    }
}

/// Premultiplied-alpha source-over blend.
///
/// `src` and `dst` are ARGB pixels where the colour channels are already
/// multiplied by the alpha channel: `src.r <= src.a`, `src.g <= src.a`, etc.
/// Output is also premultiplied ARGB.
///
/// Formula (Porter-Duff source-over, premultiplied):
///   out.rgb = src.rgb + dst.rgb * (1 - src.a)
///   out.a   = src.a   + dst.a   * (1 - src.a)
///
/// All arithmetic is integer with correct rounding (+127 before /255).
/// An opaque source (a=255) passes through unmodified.
/// A transparent source (a=0) returns the destination unmodified.
#[inline(always)]
pub fn blend_src_over_premul(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    match sa {
        0 => dst,
        255 => src,
        a => {
            let a = a as u32;
            let inv_a = 255 - a;
            let sr = (src >> 16) & 0xFF;
            let sg = (src >> 8) & 0xFF;
            let sb = src & 0xFF;
            let dr = (dst >> 16) & 0xFF;
            let dg = (dst >> 8) & 0xFF;
            let db = dst & 0xFF;
            let da = (dst >> 24) & 0xFF;
            let out_r = sr + (dr * inv_a + 127) / 255;
            let out_g = sg + (dg * inv_a + 127) / 255;
            let out_b = sb + (db * inv_a + 127) / 255;
            let out_a = a + (da * inv_a + 127) / 255;
            (out_r.min(255) << 16) | (out_g.min(255) << 8) | out_b.min(255) | (out_a.min(255) << 24)
        }
    }
}

/// Straight-alpha (non-premultiplied) source-over blend.
///
/// The source colour channels are assumed to be stored in non-premultiplied
/// form (i.e. `src.r` may be > `src.a`). The operation conceptually:
///   1. Premultiplies src RGB by src alpha
///   2. Blends over dst using source-over
///   3. Returns straight-alpha ARGB
///
/// All arithmetic is integer with correct rounding (+127 before /255).
/// An opaque source (a=255) replaces the destination (fast path).
/// A transparent source (a=0) returns the destination unmodified (fast path).
/// An opaque destination (dst.a=255) skips the output alpha computation.
#[inline(always)]
pub fn blend_src_over_straight(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    match sa {
        0 => dst,
        255 => src | 0xFF00_0000,
        a => {
            let a = a as u32;
            let inv_a = 255 - a;
            let sr = (src >> 16) & 0xFF;
            let sg = (src >> 8) & 0xFF;
            let sb = src & 0xFF;
            let dr = (dst >> 16) & 0xFF;
            let dg = (dst >> 8) & 0xFF;
            let db = dst & 0xFF;
            let da = (dst >> 24) & 0xFF;
            let out_r = (sr * a + dr * inv_a + 127) / 255;
            let out_g = (sg * a + dg * inv_a + 127) / 255;
            let out_b = (sb * a + db * inv_a + 127) / 255;
            let out_a = if da == 255 {
                255
            } else {
                (a * 255 + da * inv_a + 127) / 255
            };
            (out_r.min(255) << 16) | (out_g.min(255) << 8) | out_b.min(255) | (out_a.min(255) << 24)
        }
    }
}

/// Validate a client-requested surface width and height.
///
/// Returns `Some((w, h))` if both dimensions are non-zero and the total
/// allocation fits within `MAX_SURFACE_BYTES`. Returns `None` on overflow
/// or out-of-range dimensions.
#[inline]
pub fn validate_surface_dims(w: u32, h: u32) -> Option<(u32, u32)> {
    if w == 0 || h == 0 || w > MAX_SURFACE_DIM || h > MAX_SURFACE_DIM {
        return None;
    }
    let bytes = (w as usize).checked_mul(h as usize)?;
    if bytes.checked_mul(4).is_none() || bytes * 4 > MAX_SURFACE_BYTES {
        return None;
    }
    Some((w, h))
}
