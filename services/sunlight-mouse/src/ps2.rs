//! PS/2 relative-axis decoding shared by the driver and its regression tests.

/// Decode the PS/2 packet's two relative axes into SunlightOS screen-relative
/// coordinates (`+Y` down). Overflow packets are rejected by the caller.
pub fn decode_relative_axes(flags: u8, x: u8, y: u8, invert_y: bool) -> (i16, i16) {
    let mut dx = i32::from(x);
    let mut dy = i32::from(y);
    if flags & 0x10 != 0 {
        dx |= !0xff;
    }
    if flags & 0x20 != 0 {
        dy |= !0xff;
    }
    if invert_y {
        dy = -dy;
    }
    (dx as i16, dy as i16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrected_real_ps2_vertical_direction_is_unchanged() {
        // Real PS/2 reports positive Y upward, so the real-hardware profile
        // performs the one conversion to screen-relative positive-down Y.
        assert_eq!(decode_relative_axes(0x08, 0, 1, true).1, -1);
        assert_eq!(decode_relative_axes(0x28, 0, 0xff, true).1, 1);
    }

    #[test]
    fn screen_oriented_qemu_profile_does_not_double_invert_y() {
        assert_eq!(decode_relative_axes(0x08, 0, 1, false).1, 1);
        assert_eq!(decode_relative_axes(0x28, 0, 0xff, false).1, -1);
    }
}
