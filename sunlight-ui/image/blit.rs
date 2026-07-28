//! Image pixel math: source-over blend, premultiplied bilinear sampling,
//! and analytic antialiased rounded-rect coverage.
//!
//! Conventions (stable for callers and tests):
//! - Packed pixels are ARGB8888: `(a << 24) | (r << 16) | (g << 8) | b`.
//! - Stored image alpha is **straight** (not premultiplied).
//! - Compositing uses Porter–Duff source-over on straight alpha.
//! - Bilinear sampling converts to premultiplied RGB before interpolating,
//!   then un-premultiplies so the blend stage always sees straight alpha.
//! - Fixed-point paths use 8 fractional bits (1/256) and round half-up via `+127`.
//! - Coverage is an 8-bit weight (0 = outside, 255 = fully inside).
//!
//! No heap allocation. All helpers are pure and suitable for unit tests.

/// Round `(value * factor) / 255` with half-up.
#[inline]
pub const fn mul_div_255(value: u32, factor: u32) -> u32 {
    (value * factor + 127) / 255
}

/// Porter–Duff source-over for straight-alpha ARGB8888 pixels.
///
/// Cheap endpoints: `src_a == 0` → `dst`, `src_a == 255` → `src`.
/// Result alpha and RGB stay in straight form (same convention as
/// [`crate::theme::Color::blend_over`]).
#[inline]
pub fn blend_source_over(src: u32, dst: u32) -> u32 {
    let src_a = (src >> 24) as u64;
    if src_a == 255 {
        return src;
    }
    if src_a == 0 {
        return dst;
    }
    let dst_a = (dst >> 24) as u64;
    let inv_src_a = 255 - src_a;
    let out_a_numerator = src_a * 255 + dst_a * inv_src_a;
    let out_a = (out_a_numerator + 127) / 255;

    let blend_ch = |src_c: u32, dst_c: u32| -> u32 {
        let numerator = src_c as u64 * src_a * 255 + dst_c as u64 * dst_a * inv_src_a;
        ((numerator + out_a_numerator / 2) / out_a_numerator) as u32
    };

    let r = blend_ch((src >> 16) & 0xFF, (dst >> 16) & 0xFF);
    let g = blend_ch((src >> 8) & 0xFF, (dst >> 8) & 0xFF);
    let b = blend_ch(src & 0xFF, dst & 0xFF);
    ((out_a as u32) << 24) | (r << 16) | (g << 8) | b
}

/// Premultiply straight ARGB → (pr, pg, pb, a) as u32 channels 0..=255.
#[inline]
pub fn premultiply(argb: u32) -> (u32, u32, u32, u32) {
    let a = argb >> 24;
    if a == 0 {
        return (0, 0, 0, 0);
    }
    if a == 255 {
        return ((argb >> 16) & 0xFF, (argb >> 8) & 0xFF, argb & 0xFF, 255);
    }
    (
        mul_div_255((argb >> 16) & 0xFF, a),
        mul_div_255((argb >> 8) & 0xFF, a),
        mul_div_255(argb & 0xFF, a),
        a,
    )
}

/// Un-premultiply to straight ARGB. Fully transparent → 0 (RGB cleared).
#[inline]
pub fn unpremultiply(pr: u32, pg: u32, pb: u32, a: u32) -> u32 {
    if a == 0 {
        return 0;
    }
    if a == 255 {
        return 0xFF00_0000 | (pr << 16) | (pg << 8) | pb;
    }
    let r = ((pr * 255 + a / 2) / a).min(255);
    let g = ((pg * 255 + a / 2) / a).min(255);
    let b = ((pb * 255 + a / 2) / a).min(255);
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Multiply source straight alpha by an 8-bit coverage weight.
#[inline]
pub fn apply_coverage(argb: u32, coverage: u8) -> u32 {
    if coverage == 0 {
        return 0;
    }
    if coverage == 255 {
        return argb;
    }
    let a = mul_div_255(argb >> 24, coverage as u32);
    if a == 0 {
        return 0;
    }
    (a << 24) | (argb & 0x00FF_FFFF)
}

/// Integer square root (floor) for coverage distance.
#[inline]
pub fn isqrt_u32(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Clamp a corner radius so it never exceeds half the rect on either axis.
#[inline]
pub fn clamp_corner_radius(w: u32, h: u32, radius: u32) -> u32 {
    radius.min(w / 2).min(h / 2)
}

/// Analytic coverage (0..=255) for pixel `(px, py)` inside a rounded rectangle
/// defined by origin `(ox, oy)` and size `(w, h)`.
///
/// - Radius `0` → always full coverage inside the axis-aligned rect.
/// - Interior and non-corner edge strips → full coverage (fast path).
/// - Corner regions use a ~1 px transition: `clamp(0.5 + (R − d), 0, 1)`.
/// - Outside the rect → 0.
///
/// Coordinates are framebuffer / canvas pixels (integer pixel indices).
#[inline]
pub fn rounded_rect_coverage(
    ox: i32,
    oy: i32,
    w: u32,
    h: u32,
    radius: u32,
    px: i32,
    py: i32,
) -> u8 {
    if w == 0 || h == 0 {
        return 0;
    }
    if px < ox || py < oy || px >= ox + w as i32 || py >= oy + h as i32 {
        return 0;
    }
    let r = clamp_corner_radius(w, h, radius) as i32;
    if r <= 0 {
        return 255;
    }

    let lx = px - ox;
    let ly = py - oy;
    let wi = w as i32;
    let hi = h as i32;

    // Distance into the corner region (0 if not in a corner band).
    let dx = if lx < r {
        r - 1 - lx
    } else if lx >= wi - r {
        lx - (wi - r)
    } else {
        0
    };
    let dy = if ly < r {
        r - 1 - ly
    } else if ly >= hi - r {
        ly - (hi - r)
    } else {
        0
    };

    // Centre / edge strips: fully inside.
    if dx <= 0 || dy <= 0 {
        return 255;
    }

    // Distance from the quarter-circle centre to the pixel centre.
    // Pixel centre is at (lx+0.5, ly+0.5); corner centre matches the binary
    // clip convention at (r-0.5, r-0.5) relative to the corner, so the
    // integer offsets (dx, dy) already measure from that centre when using
    // the same dx/dy construction as the hard clip.
    let dist2 = (dx * dx + dy * dy) as u32;
    let dist = isqrt_u32(dist2);
    // coverage ≈ clamp(0.5 + (r - dist), 0, 1) with 1-pixel falloff
    // edge = 2*(r - dist) + 1  in half-pixel units
    let edge = 2 * r + 1 - 2 * dist as i32;
    if edge <= 0 {
        0
    } else if edge >= 2 {
        255
    } else {
        // edge == 1 → half coverage
        128
    }
}

/// Map destination pixel index `d` (`0..dst_len`) to a source coordinate in
/// 24.8 fixed-point (8 fractional bits).
///
/// Uses pixel-centre mapping: `s = (d + 0.5) * src / dst − 0.5`.
/// Safe for 1-wide / 1-high dimensions (no division by zero).
#[inline]
pub fn map_src_fp(d: i32, dst_len: u32, src_len: u32) -> i32 {
    if src_len == 0 || dst_len == 0 {
        return 0;
    }
    if src_len == 1 || dst_len == 1 {
        // Single source column/row: always sample centre (index 0).
        // Single dest column/row: also sample the centre of the source span.
        if src_len == 1 {
            return 0;
        }
        // dst_len == 1: map to centre of source.
        return ((src_len - 1) as i32) << 7; // (src_len-1)/2 in 8.8
    }
    // ((d*2+1) * src * 128) / dst - 128
    let num = (d as i64 * 2 + 1) * src_len as i64 * 128;
    (num / dst_len as i64 - 128) as i32
}

/// Bilinear sample of a straight-alpha ARGB image in premultiplied space.
///
/// `sample(x, y)` must return a clamped in-bounds ARGB pixel.
/// `x_fp` / `y_fp` are 24.8 fixed-point source coordinates.
/// Edge samples clamp to the image border (no wrap, no OOB read).
#[inline]
pub fn sample_bilinear_premul(
    sample: &dyn Fn(u32, u32) -> u32,
    src_w: u32,
    src_h: u32,
    x_fp: i32,
    y_fp: i32,
) -> u32 {
    if src_w == 0 || src_h == 0 {
        return 0;
    }
    if src_w == 1 && src_h == 1 {
        return sample(0, 0);
    }

    let max_x = src_w.saturating_sub(1) as i32;
    let max_y = src_h.saturating_sub(1) as i32;

    // Clamp continuous coords into [0, max].
    let x_fp = x_fp.clamp(0, max_x << 8);
    let y_fp = y_fp.clamp(0, max_y << 8);

    let x0 = (x_fp >> 8).clamp(0, max_x);
    let y0 = (y_fp >> 8).clamp(0, max_y);
    let x1 = (x0 + 1).min(max_x);
    let y1 = (y0 + 1).min(max_y);
    let fx = (x_fp & 0xFF) as u32; // 0..255 weight toward x1
    let fy = (y_fp & 0xFF) as u32;
    let ifx = 256 - fx;
    let ify = 256 - fy;

    let (r00, g00, b00, a00) = premultiply(sample(x0 as u32, y0 as u32));
    let (r10, g10, b10, a10) = premultiply(sample(x1 as u32, y0 as u32));
    let (r01, g01, b01, a01) = premultiply(sample(x0 as u32, y1 as u32));
    let (r11, g11, b11, a11) = premultiply(sample(x1 as u32, y1 as u32));

    // Bilinear weights sum to 256*256; divide by 65536 with half-up.
    let lerp = |c00: u32, c10: u32, c01: u32, c11: u32| -> u32 {
        let top = c00 * ifx + c10 * fx;
        let bot = c01 * ifx + c11 * fx;
        // top/bot are *256 scale; combine with fy then / 65536
        let v = top * ify + bot * fy;
        ((v + 32768) >> 16).min(255)
    };

    let pr = lerp(r00, r10, r01, r11);
    let pg = lerp(g00, g10, g01, g11);
    let pb = lerp(b00, b10, b01, b11);
    let pa = lerp(a00, a10, a01, a11);
    unpremultiply(pr, pg, pb, pa)
}

/// Nearest-neighbour sample (for the 1:1 fast path and reference tests).
#[inline]
pub fn sample_nearest(
    sample: &dyn Fn(u32, u32) -> u32,
    src_w: u32,
    src_h: u32,
    x: u32,
    y: u32,
) -> u32 {
    if src_w == 0 || src_h == 0 {
        return 0;
    }
    let x = x.min(src_w - 1);
    let y = y.min(src_h - 1);
    sample(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_0_leaves_destination_unchanged() {
        let dst = 0xFF12_3456;
        assert_eq!(blend_source_over(0x00FF_0000, dst), dst);
        assert_eq!(blend_source_over(0x0000_0000, dst), dst);
    }

    #[test]
    fn alpha_255_produces_source_color() {
        let src = 0xFFAB_CDEF;
        let dst = 0xFF00_1122;
        assert_eq!(blend_source_over(src, dst), src);
    }

    #[test]
    fn fifty_percent_source_over_matches_known_rounding() {
        // Half red over opaque blue → 0xFF80_007F (matches Color::blend_over).
        let src = 0x80FF_0000;
        let dst = 0xFF00_00FF;
        assert_eq!(blend_source_over(src, dst), 0xFF80_007F);
    }

    #[test]
    fn bilinear_does_not_import_transparent_rgb_fringe() {
        // Opaque white next to fully transparent red (hostile RGB).
        // Midpoint in premul space must stay neutral white, not pink.
        let sample = |x: u32, _y: u32| -> u32 {
            if x == 0 {
                0xFFFF_FFFF
            } else {
                0x00FF_0000 // a=0, rgb=red
            }
        };
        // Exactly halfway between x=0 and x=1 → x_fp = 0.5 = 128/256
        let mid = sample_bilinear_premul(&sample, 2, 1, 128, 0);
        let a = mid >> 24;
        let r = (mid >> 16) & 0xFF;
        let g = (mid >> 8) & 0xFF;
        let b = mid & 0xFF;
        assert!(a > 100 && a < 160, "alpha={a}");
        // Channels should be nearly equal (white), not red-biased.
        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        assert!(
            max_c - min_c <= 2,
            "fringe detected: r={r} g={g} b={b} (expected near-white)"
        );
        assert!(r > 200, "expected bright white-ish, r={r}");
    }

    #[test]
    fn scale_1x1_preserves_color() {
        let sample = |_x: u32, _y: u32| 0x80AA_BBCC;
        let p = sample_bilinear_premul(&sample, 1, 1, 0, 0);
        assert_eq!(p, 0x80AA_BBCC);
        // Mapping a larger dest onto 1×1 still yields the same pixel.
        let fp = map_src_fp(3, 8, 1);
        let p2 = sample_bilinear_premul(&sample, 1, 1, fp, fp);
        assert_eq!(p2, 0x80AA_BBCC);
    }

    #[test]
    fn scale_1_wide_and_1_high_are_safe() {
        // 1×4 strip
        let sample_col = |x: u32, y: u32| {
            assert_eq!(x, 0);
            0xFF00_0000 | (y * 40)
        };
        let _ = sample_bilinear_premul(&sample_col, 1, 4, 0, map_src_fp(1, 8, 4));
        // 4×1 strip
        let sample_row = |x: u32, y: u32| {
            assert_eq!(y, 0);
            0xFF00_0000 | (x * 40)
        };
        let _ = sample_bilinear_premul(&sample_row, 4, 1, map_src_fp(1, 8, 4), 0);
    }

    #[test]
    fn rounded_coverage_inside_outside_and_boundary() {
        // 8×8 rect, radius 4 (circle-ish).
        let ox = 0;
        let oy = 0;
        let w = 8u32;
        let h = 8u32;
        let r = 4u32;
        // Centre fully inside.
        assert_eq!(rounded_rect_coverage(ox, oy, w, h, r, 3, 3), 255);
        assert_eq!(rounded_rect_coverage(ox, oy, w, h, r, 4, 4), 255);
        // Far outside the rect.
        assert_eq!(rounded_rect_coverage(ox, oy, w, h, r, -1, 3), 0);
        assert_eq!(rounded_rect_coverage(ox, oy, w, h, r, 8, 3), 0);
        // Outer corner pixel should be outside or partial, not full.
        let corner = rounded_rect_coverage(ox, oy, w, h, r, 0, 0);
        assert!(
            corner < 255,
            "outer corner should not be fully covered: {corner}"
        );
        // Pixel clearly inside the top-left quarter-circle.
        let inner_corner = rounded_rect_coverage(ox, oy, w, h, r, 2, 2);
        assert_eq!(inner_corner, 255);
    }

    #[test]
    fn radius_zero_is_full_rect() {
        assert_eq!(rounded_rect_coverage(0, 0, 4, 4, 0, 0, 0), 255);
        assert_eq!(rounded_rect_coverage(0, 0, 4, 4, 0, 3, 3), 255);
        assert_eq!(rounded_rect_coverage(0, 0, 4, 4, 0, 4, 0), 0);
    }

    #[test]
    fn excessive_radius_is_clamped() {
        assert_eq!(clamp_corner_radius(10, 8, 100), 4);
        assert_eq!(clamp_corner_radius(10, 8, 0), 0);
        // Coverage with huge radius still only paints the rect.
        assert_eq!(rounded_rect_coverage(0, 0, 6, 6, 1000, 2, 2), 255);
        assert_eq!(rounded_rect_coverage(0, 0, 6, 6, 1000, -1, 0), 0);
    }

    #[test]
    fn channel_order_argb_preserved_in_blend() {
        // Red over black
        assert_eq!(blend_source_over(0xFFFF_0000, 0xFF00_0000), 0xFFFF_0000);
        // Green over black
        assert_eq!(blend_source_over(0xFF00_FF00, 0xFF00_0000), 0xFF00_FF00);
        // Blue over black
        assert_eq!(blend_source_over(0xFF00_00FF, 0xFF00_0000), 0xFF00_00FF);
        // Premul of pure red with a=128 keeps R, zeros G/B.
        let (pr, pg, pb, a) = premultiply(0x80FF_0000);
        assert_eq!((pr, pg, pb, a), (128, 0, 0, 128));
        assert_eq!(unpremultiply(pr, pg, pb, a) & 0x00FF_FFFF, 0x00FF_0000);
    }

    #[test]
    fn coverage_scales_alpha_only() {
        let src = 0xFF10_2030;
        assert_eq!(apply_coverage(src, 0), 0);
        assert_eq!(apply_coverage(src, 255), src);
        let half = apply_coverage(src, 128);
        assert_eq!(half >> 24, 128);
        assert_eq!(half & 0x00FF_FFFF, 0x0010_2030);
    }

    #[test]
    fn matches_color_blend_over() {
        use crate::theme::Color;
        let cases = [
            (0x80FF_0000, 0xFF00_00FF),
            (0x40AA_BBCC, 0x0000_0000),
            (0xFF12_3456, 0xFF65_4321),
            (0x01FF_FF00, 0x80FF_00FF),
        ];
        for (s, d) in cases {
            assert_eq!(
                blend_source_over(s, d),
                Color(s).blend_over(Color(d)).0,
                "mismatch for {s:#010x} over {d:#010x}"
            );
        }
    }
}
