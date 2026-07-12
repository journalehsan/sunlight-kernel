use crate::GuestMemory;

/// Logical DOS text-mode width.
pub const TEXT_COLUMNS: usize = 80;
/// Logical DOS text-mode height.
pub const TEXT_ROWS: usize = 25;
pub const VIDEO_SEGMENT: u16 = 0xb800;
pub const VIDEO_PHYSICAL: usize = 0xb8000;
pub const VIDEO_BYTES: usize = TEXT_COLUMNS * TEXT_ROWS * 2;
pub const DEFAULT_ATTRIBUTE: u8 = 0x07;

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
            character: memory.read_u8(VIDEO_SEGMENT, offset),
            attribute: memory.read_u8(VIDEO_SEGMENT, offset.wrapping_add(1)),
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
