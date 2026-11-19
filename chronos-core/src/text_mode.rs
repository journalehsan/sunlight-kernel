use crate::GuestMemory;

/// Logical DOS text-mode width.
pub const TEXT_COLUMNS: usize = 80;
/// Logical DOS text-mode height.
pub const TEXT_ROWS: usize = 25;
pub const VIDEO_SEGMENT: u16 = 0xb800;
pub const VIDEO_PHYSICAL: usize = 0xb8000;
pub const VIDEO_BYTES: usize = TEXT_COLUMNS * TEXT_ROWS * 2;
pub const DEFAULT_ATTRIBUTE: u8 = 0x07;

/// Interpretation of text attribute bit 7, selected by the virtual VGA
/// Attribute Controller mode-control bit through `INT 10h / AX=1003h`.
///
/// The virtual adapter models the conventional VGA default after a mode set:
/// blinking is enabled, so bit 7 marks a blinking character and background
/// color uses only bits 4-6.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAttributeMode {
    #[default]
    Blink,
    BackgroundIntensity,
}

/// Decoded 8-bit VGA text attribute. The raw byte stays authoritative in guest
/// memory; this view is derived on demand from the current mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextAttribute {
    pub foreground: u8,
    pub background: u8,
    pub blink: bool,
}

impl TextAttribute {
    /// Bits 0-3 are always foreground color. In blink mode bits 4-6 are the
    /// background color and bit 7 marks blinking; in background-intensity mode
    /// bit 7 is the fourth background bit and no character blinks.
    pub const fn decode(attribute: u8, mode: TextAttributeMode) -> Self {
        match mode {
            TextAttributeMode::Blink => Self {
                foreground: attribute & 0x0f,
                background: (attribute >> 4) & 0x07,
                blink: attribute & 0x80 != 0,
            },
            TextAttributeMode::BackgroundIntensity => Self {
                foreground: attribute & 0x0f,
                background: (attribute >> 4) & 0x0f,
                blink: false,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextCell {
    pub character: u8,
    pub attribute: u8,
}

/// Stateless view of the authoritative guest color-text memory.
///
/// The surface deliberately owns no cells: all rendering and BIOS output use
/// the `0xB8000` bytes in [`GuestMemory`].
#[derive(Clone, Copy, Debug, Default)]
pub struct TextModeSurface;

impl TextModeSurface {
    pub const fn new() -> Self {
        Self
    }

    pub fn cell(memory: &GuestMemory, column: usize, row: usize) -> TextCell {
        let offset = ((row * TEXT_COLUMNS + column) * 2) as u16;
        TextCell {
            character: memory.read_display_byte(VIDEO_SEGMENT, offset),
            attribute: memory.read_display_byte(VIDEO_SEGMENT, offset.wrapping_add(1)),
        }
    }

    pub fn clear(memory: &mut GuestMemory, attribute: u8) {
        for index in 0..TEXT_COLUMNS * TEXT_ROWS {
            let offset = (index * 2) as u16;
            memory.write_u8(VIDEO_SEGMENT, offset, b' ');
            memory.write_u8(VIDEO_SEGMENT, offset.wrapping_add(1), attribute);
        }
    }
}

/// CP437 conversion boundary. ASCII is exact and the box-drawing block covers
/// the characters used by ordinary DOS text interfaces.
pub fn display_char(byte: u8) -> char {
    match byte {
        0x20..=0x7e => byte as char,
        0xb0 => '░',
        0xb1 => '▒',
        0xb2 => '▓',
        0xb3 => '│',
        0xc4 => '─',
        0xda => '┌',
        0xbf => '┐',
        0xc0 => '└',
        0xd9 => '┘',
        0xc3 => '├',
        0xb4 => '┤',
        0xc2 => '┬',
        0xc1 => '┴',
        0xc5 => '┼',
        _ => '·',
    }
}

#[cfg(test)]
mod tests {
    use super::{display_char, TextAttribute, TextAttributeMode};

    #[test]
    fn foreground_color_uses_the_low_nibble_in_both_modes() {
        for mode in [
            TextAttributeMode::Blink,
            TextAttributeMode::BackgroundIntensity,
        ] {
            for foreground in 0..16u8 {
                let attribute = TextAttribute::decode(foreground, mode);
                assert_eq!(attribute.foreground, foreground);
            }
        }
    }

    #[test]
    fn blink_mode_limits_background_to_three_bits_and_marks_blinking() {
        let plain = TextAttribute::decode(0x4e, TextAttributeMode::Blink);
        assert_eq!(plain.foreground, 0x0e);
        assert_eq!(plain.background, 0x04);
        assert!(!plain.blink);

        let blinking = TextAttribute::decode(0x8f, TextAttributeMode::Blink);
        assert_eq!(blinking.foreground, 0x0f);
        assert_eq!(blinking.background, 0x00);
        assert!(blinking.blink);
    }

    #[test]
    fn intensity_mode_uses_bit_seven_as_the_fourth_background_bit() {
        let dim = TextAttribute::decode(0x4e, TextAttributeMode::BackgroundIntensity);
        assert_eq!(dim.foreground, 0x0e);
        assert_eq!(dim.background, 0x04);
        assert!(!dim.blink);

        let bright = TextAttribute::decode(0x8f, TextAttributeMode::BackgroundIntensity);
        assert_eq!(bright.foreground, 0x0f);
        assert_eq!(bright.background, 0x08);
        assert!(!bright.blink);

        let highest = TextAttribute::decode(0xf0, TextAttributeMode::BackgroundIntensity);
        assert_eq!(highest.background, 0x0f);
        assert!(!highest.blink);
    }

    #[test]
    fn cp437_line_drawing_mapping_is_unchanged() {
        for (byte, glyph) in [
            (0xb3, '│'),
            (0xc4, '─'),
            (0xda, '┌'),
            (0xbf, '┐'),
            (0xc0, '└'),
            (0xd9, '┘'),
            (0xc3, '├'),
            (0xb4, '┤'),
            (0xc2, '┬'),
            (0xc1, '┴'),
            (0xc5, '┼'),
        ] {
            assert_eq!(display_char(byte), glyph);
        }
        assert_eq!(display_char(b'A'), 'A');
    }
}
