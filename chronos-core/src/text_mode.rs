/// Logical DOS text-mode width.
pub const TEXT_COLUMNS: usize = 80;
/// Logical DOS text-mode height.
pub const TEXT_ROWS: usize = 25;
const DEFAULT_ATTRIBUTE: u8 = 0x07;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextCell {
    pub character: u8,
    pub attribute: u8,
}

impl TextCell {
    const BLANK: Self = Self {
        character: b' ',
        attribute: DEFAULT_ATTRIBUTE,
    };
}

/// DOS text output model independent from the native window renderer.
pub struct TextModeSurface {
    cells: [TextCell; TEXT_COLUMNS * TEXT_ROWS],
    cursor_column: usize,
    cursor_row: usize,
    dirty: bool,
}

impl TextModeSurface {
    pub const fn new() -> Self {
        Self {
            cells: [TextCell::BLANK; TEXT_COLUMNS * TEXT_ROWS],
            cursor_column: 0,
            cursor_row: 0,
            dirty: true,
        }
    }

    pub fn cell(&self, column: usize, row: usize) -> TextCell {
        self.cells[row * TEXT_COLUMNS + column]
    }

    pub fn cells(&self) -> &[TextCell] {
        &self.cells
    }

    pub const fn cursor_column(&self) -> usize {
        self.cursor_column
    }

    pub const fn cursor_row(&self) -> usize {
        self.cursor_row
    }

    pub fn take_dirty(&mut self) -> bool {
        let dirty = self.dirty;
        self.dirty = false;
        dirty
    }

    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\r' => {
                self.cursor_column = 0;
                self.dirty = true;
            }
            b'\n' => {
                self.cursor_row += 1;
                self.normalize_cursor();
                self.dirty = true;
            }
            0x08 => {
                if self.cursor_column > 0 {
                    self.cursor_column -= 1;
                    let index = self.cursor_row * TEXT_COLUMNS + self.cursor_column;
                    self.cells[index] = TextCell::BLANK;
                    self.dirty = true;
                }
            }
            _ => {
                let index = self.cursor_row * TEXT_COLUMNS + self.cursor_column;
                self.cells[index] = TextCell {
                    character: byte,
                    attribute: DEFAULT_ATTRIBUTE,
                };
                self.cursor_column += 1;
                self.normalize_cursor();
                self.dirty = true;
            }
        }
    }

    fn normalize_cursor(&mut self) {
        if self.cursor_column >= TEXT_COLUMNS {
            self.cursor_column = 0;
            self.cursor_row += 1;
        }
        if self.cursor_row >= TEXT_ROWS {
            self.cells.copy_within(TEXT_COLUMNS.., 0);
            for cell in &mut self.cells[(TEXT_ROWS - 1) * TEXT_COLUMNS..] {
                *cell = TextCell::BLANK;
            }
            self.cursor_row = TEXT_ROWS - 1;
        }
    }
}

impl Default for TextModeSurface {
    fn default() -> Self {
        Self::new()
    }
}

/// Initial, explicit code-page conversion boundary. ASCII is exact; later
/// milestones can replace the fallback with a full configurable code page.
pub fn display_char(byte: u8) -> char {
    match byte {
        0x20..=0x7e => byte as char,
        _ => '·',
    }
}

#[cfg(test)]
mod tests {
    use super::{TextModeSurface, TEXT_ROWS};

    #[test]
    fn controls_wrap_and_scroll_the_text_surface() {
        let mut surface = TextModeSurface::new();
        surface.write_byte(b'A');
        surface.write_byte(b'\r');
        surface.write_byte(b'B');
        assert_eq!(surface.cell(0, 0).character, b'B');

        for _ in 0..TEXT_ROWS {
            surface.write_byte(b'\n');
        }
        assert_eq!(surface.cursor_row(), TEXT_ROWS - 1);
    }
}
