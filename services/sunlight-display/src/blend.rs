/// Multiply two 8-bit values and divide by 255 with one defined rounding step.
#[inline(always)]
pub fn mul_u8_div_255_round(value: u8, factor: u8) -> u8 {
    (((value as u16 * factor as u16) + 127) / 255) as u8
}

#[inline(always)]
fn xrgb(rgb: u32) -> u32 {
    0xFF00_0000 | (rgb & 0x00FF_FFFF)
}

/// Blend an XRGB source over an XRGB destination using external coverage.
/// The source high byte is not interpreted as alpha.
#[inline(always)]
pub fn blend_xrgb_with_coverage(src: u32, dst: u32, coverage: u8) -> u32 {
    match coverage {
        0 => xrgb(dst),
        255 => xrgb(src),
        a => {
            let inv = 255 - a as u16;
            let a = a as u16;
            let r = (((src >> 16 & 0xFF) as u16 * a + (dst >> 16 & 0xFF) as u16 * inv + 127) / 255)
                as u32;
            let g = (((src >> 8 & 0xFF) as u16 * a + (dst >> 8 & 0xFF) as u16 * inv + 127) / 255)
                as u32;
            let b = (((src & 0xFF) as u16 * a + (dst & 0xFF) as u16 * inv + 127) / 255) as u32;
            0xFF00_0000 | (r << 16) | (g << 8) | b
        }
    }
}

/// Blend a straight-alpha ARGB image pixel over the XRGB destination.
/// The destination remains XRGB; no alpha is propagated to it.
#[inline(always)]
pub fn blend_straight_alpha_over_xrgb(src: u32, dst: u32) -> u32 {
    let alpha = (src >> 24) as u8;
    match alpha {
        0 => xrgb(dst),
        255 => xrgb(src),
        a => {
            let inv = 255 - a as u16;
            let a = a as u16;
            let r = (((src >> 16 & 0xFF) as u16 * a + (dst >> 16 & 0xFF) as u16 * inv + 127) / 255)
                as u32;
            let g = (((src >> 8 & 0xFF) as u16 * a + (dst >> 8 & 0xFF) as u16 * inv + 127) / 255)
                as u32;
            let b = (((src & 0xFF) as u16 * a + (dst & 0xFF) as u16 * inv + 127) / 255) as u32;
            0xFF00_0000 | (r << 16) | (g << 8) | b
        }
    }
}

/// Blend a premultiplied-alpha ARGB source over an XRGB destination.
///
/// Decoration gradients are stored as a color plus a coverage value, but their
/// final blend is performed in premultiplied form.  Keeping that rule here
/// avoids dark fringes around low-alpha glow and shadow pixels.
#[inline(always)]
pub fn blend_premultiplied_alpha_over_xrgb(src: u32, dst: u32) -> u32 {
    let alpha = (src >> 24) as u8;
    match alpha {
        0 => xrgb(dst),
        255 => xrgb(src),
        a => {
            let inv = 255 - a as u16;
            let r = ((src >> 16 & 0xFF) as u16 + (((dst >> 16 & 0xFF) as u16 * inv + 127) / 255))
                .min(255) as u32;
            let g = ((src >> 8 & 0xFF) as u16 + (((dst >> 8 & 0xFF) as u16 * inv + 127) / 255))
                .min(255) as u32;
            let b =
                ((src & 0xFF) as u16 + (((dst & 0xFF) as u16 * inv + 127) / 255)).min(255) as u32;
            0xFF00_0000 | (r << 16) | (g << 8) | b
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiply_has_exact_endpoints_and_rounding() {
        assert_eq!(mul_u8_div_255_round(0, 255), 0);
        assert_eq!(mul_u8_div_255_round(255, 0), 0);
        assert_eq!(mul_u8_div_255_round(255, 255), 255);
        assert_eq!(mul_u8_div_255_round(127, 128), 64);
    }

    #[test]
    fn coverage_and_straight_alpha_match_for_all_key_values() {
        let src = 0x0012_34AB;
        let dst = 0xFFCD_EF10;
        for a in [0, 1, 127, 128, 254, 255] {
            assert_eq!(
                blend_xrgb_with_coverage(src, dst, a),
                blend_straight_alpha_over_xrgb((a as u32) << 24 | src, dst)
            );
        }
    }

    #[test]
    fn endpoints_preserve_rgb_and_xrgb_high_byte() {
        let src = 0x7F_FF_FF_FF;
        let dst = 0x12_01_02_03;
        assert_eq!(blend_xrgb_with_coverage(src, dst, 255), 0xFFFF_FFFF);
        assert_eq!(blend_xrgb_with_coverage(src, dst, 0), 0xFF01_0203);
        assert_eq!(
            blend_straight_alpha_over_xrgb(0x00FF_FFFF, dst),
            0xFF01_0203
        );
        assert_eq!(blend_straight_alpha_over_xrgb(0xFF00_1020, dst), 0xFF001020);
    }

    #[test]
    fn channel_order_and_full_coverage_are_stable() {
        let red = 0x00FF_0000;
        let green = 0x0000_FF00;
        let blue = 0x0000_00FF;
        assert_eq!(blend_xrgb_with_coverage(red, green, 255), 0xFFFF_0000);
        assert_eq!(blend_xrgb_with_coverage(green, blue, 255), 0xFF00_FF00);
        assert_eq!(blend_xrgb_with_coverage(blue, red, 255), 0xFF00_00FF);
        let mut out = 0xFF12_3456;
        for _ in 0..8 {
            out = blend_xrgb_with_coverage(0x00FF_FFFF, out, 255);
        }
        assert_eq!(out, 0xFFFF_FFFF);
    }

    #[test]
    fn premultiplied_alpha_blend_preserves_xrgb_and_uses_premultiplied_rgb() {
        // 50% orange is already premultiplied: (128, 61, 0), alpha 128.
        let src = 0x8080_3D00;
        let dst = 0xFF20_4060;
        let out = blend_premultiplied_alpha_over_xrgb(src, dst);
        assert_eq!(out, 0xFF90_5D30);
    }
}
