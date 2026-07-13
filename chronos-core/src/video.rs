use crate::GuestMemory;

pub const VGA_WIDTH: usize = 320;
pub const VGA_HEIGHT: usize = 200;
pub const VGA_FRAMEBUFFER_SEGMENT: u16 = 0xa000;
pub const VGA_FRAMEBUFFER_PHYSICAL: usize = 0xa0000;
pub const VGA_FRAMEBUFFER_BYTES: usize = VGA_WIDTH * VGA_HEIGHT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestVideoMode {
    Text80x25Color,
    Vga320x200x256,
}

impl GuestVideoMode {
    pub const fn bios_mode(self) -> u8 {
        match self {
            Self::Text80x25Color => 0x03,
            Self::Vga320x200x256 => 0x13,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VgaDacEntry {
    pub red_6bit: u8,
    pub green_6bit: u8,
    pub blue_6bit: u8,
}

impl VgaDacEntry {
    pub const fn new(red_6bit: u8, green_6bit: u8, blue_6bit: u8) -> Self {
        Self {
            red_6bit,
            green_6bit,
            blue_6bit,
        }
    }

    pub const fn to_rgb8(self) -> Rgb8 {
        Rgb8::new(
            dac6_to_host8(self.red_6bit),
            dac6_to_host8(self.green_6bit),
            dac6_to_host8(self.blue_6bit),
        )
    }
}

/// Round a VGA DAC component to the nearest host component. This preserves
/// both endpoints, unlike the common `component * 4` approximation.
pub const fn dac6_to_host8(component: u8) -> u8 {
    let component = if component > 63 { 63 } else { component } as u16;
    ((component * 255 + 31) / 63) as u8
}

const fn host8_to_dac6(component: u8) -> u8 {
    ((component as u16 * 63 + 127) / 255) as u8
}

pub const fn default_vga_dac_entries() -> [VgaDacEntry; 256] {
    let rgb = legacy_default_vga_palette();
    let mut entries = [VgaDacEntry::new(0, 0, 0); 256];
    let mut index = 0;
    while index < entries.len() {
        entries[index] = VgaDacEntry::new(
            host8_to_dac6(rgb[index].r),
            host8_to_dac6(rgb[index].g),
            host8_to_dac6(rgb[index].b),
        );
        index += 1;
    }
    entries
}

impl Rgb8 {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Prompt 5A's deterministic default palette recipe. Prompt 5A.1 quantizes it
/// once into real six-bit DAC entries before deriving the active host palette.
/// Entries 0..15 remain exact; a few cube/ramp components move by at most two
/// host levels because arbitrary eight-bit values are not all DAC-representable.
const fn legacy_default_vga_palette() -> [Rgb8; 256] {
    let mut palette = [Rgb8::new(0, 0, 0); 256];
    palette[0] = Rgb8::new(0, 0, 0);
    palette[1] = Rgb8::new(0, 0, 170);
    palette[2] = Rgb8::new(0, 170, 0);
    palette[3] = Rgb8::new(0, 170, 170);
    palette[4] = Rgb8::new(170, 0, 0);
    palette[5] = Rgb8::new(170, 0, 170);
    palette[6] = Rgb8::new(170, 85, 0);
    palette[7] = Rgb8::new(170, 170, 170);
    palette[8] = Rgb8::new(85, 85, 85);
    palette[9] = Rgb8::new(85, 85, 255);
    palette[10] = Rgb8::new(85, 255, 85);
    palette[11] = Rgb8::new(85, 255, 255);
    palette[12] = Rgb8::new(255, 85, 85);
    palette[13] = Rgb8::new(255, 85, 255);
    palette[14] = Rgb8::new(255, 255, 85);
    palette[15] = Rgb8::new(255, 255, 255);

    let levels = [0, 51, 102, 153, 204, 255];
    let mut index = 16;
    let mut red = 0;
    while red < 6 {
        let mut green = 0;
        while green < 6 {
            let mut blue = 0;
            while blue < 6 {
                palette[index] = Rgb8::new(levels[red], levels[green], levels[blue]);
                index += 1;
                blue += 1;
            }
            green += 1;
        }
        red += 1;
    }
    let mut gray = 0;
    while gray < 24 {
        let value = 8 + gray as u8 * 10;
        palette[232 + gray] = Rgb8::new(value, value, value);
        gray += 1;
    }
    palette
}

pub const fn default_vga_palette() -> [Rgb8; 256] {
    let dac = default_vga_dac_entries();
    let mut palette = [Rgb8::new(0, 0, 0); 256];
    let mut index = 0;
    while index < palette.len() {
        palette[index] = dac[index].to_rgb8();
        index += 1;
    }
    palette
}

pub const DEFAULT_VGA_PALETTE: [Rgb8; 256] = default_vga_palette();

/// Convert selected dirty rows from authoritative indexed guest memory into a
/// caller-owned presentation cache. A short destination is rejected without
/// reading or writing out of bounds.
pub fn convert_indexed_rows(
    memory: &GuestMemory,
    palette: &[Rgb8; 256],
    dirty_rows: &[bool; VGA_HEIGHT],
    destination: &mut [Rgb8],
) -> bool {
    if destination.len() < VGA_FRAMEBUFFER_BYTES {
        return false;
    }
    for (row, dirty) in dirty_rows.iter().copied().enumerate() {
        if !dirty {
            continue;
        }
        let row_start = row * VGA_WIDTH;
        for column in 0..VGA_WIDTH {
            let index = memory.read_u8(VGA_FRAMEBUFFER_SEGMENT, (row_start + column) as u16);
            destination[row_start + column] = palette[index as usize];
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::{
        convert_indexed_rows, dac6_to_host8, GuestMemory, Rgb8, DEFAULT_VGA_PALETTE,
        VGA_FRAMEBUFFER_BYTES, VGA_FRAMEBUFFER_SEGMENT, VGA_HEIGHT,
    };

    #[test]
    fn palette_has_stable_classic_vga_colors() {
        assert_eq!(DEFAULT_VGA_PALETTE[0], Rgb8::new(0, 0, 0));
        assert_eq!(DEFAULT_VGA_PALETTE[4], Rgb8::new(170, 0, 0));
        assert_eq!(DEFAULT_VGA_PALETTE[10], Rgb8::new(85, 255, 85));
        assert_eq!(DEFAULT_VGA_PALETTE[15], Rgb8::new(255, 255, 255));
        assert_eq!(DEFAULT_VGA_PALETTE, super::default_vga_palette());
    }

    #[test]
    fn six_bit_dac_conversion_rounds_stably_and_preserves_endpoints() {
        assert_eq!(dac6_to_host8(0), 0);
        assert_eq!(dac6_to_host8(1), 4);
        assert_eq!(dac6_to_host8(31), 125);
        assert_eq!(dac6_to_host8(32), 130);
        assert_eq!(dac6_to_host8(62), 251);
        assert_eq!(dac6_to_host8(63), 255);
        assert_eq!(dac6_to_host8(255), 255);
    }

    #[test]
    fn indexed_conversion_is_row_selective_and_bounds_checked() {
        let mut memory = GuestMemory::new();
        memory.write_u8(VGA_FRAMEBUFFER_SEGMENT, 0, 4);
        memory.write_u8(VGA_FRAMEBUFFER_SEGMENT, 320, 10);
        let mut destination = vec![Rgb8::new(1, 2, 3); VGA_FRAMEBUFFER_BYTES];
        let mut dirty = [false; VGA_HEIGHT];
        dirty[0] = true;
        assert!(convert_indexed_rows(
            &memory,
            &DEFAULT_VGA_PALETTE,
            &dirty,
            &mut destination
        ));
        assert_eq!(destination[0], Rgb8::new(170, 0, 0));
        assert_eq!(destination[320], Rgb8::new(1, 2, 3));

        let mut short = vec![Rgb8::default(); VGA_FRAMEBUFFER_BYTES - 1];
        assert!(!convert_indexed_rows(
            &memory,
            &DEFAULT_VGA_PALETTE,
            &dirty,
            &mut short
        ));
    }
}
