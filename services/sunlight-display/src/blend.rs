/// Source-over alpha blend. `src` and `dst` are XRGB pixels (A byte ignored).
/// `src_alpha` is the coverage/opacity of the source pixel (0=transparent, 255=opaque).
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
