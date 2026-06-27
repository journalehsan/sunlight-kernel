#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiSymbol {
    Minimize,
    Maximize,
    Restore,
    Close,
    Help,
    Pin,
    Divide,
    Multiply,
    Minus,
    SquareRoot,
    PlusMinus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiGlyph {
    pub width: u8,
    pub height: u8,
    pub advance: u8,
    rows: &'static [u16],
}

impl UiGlyph {
    pub const fn new(width: u8, height: u8, advance: u8, rows: &'static [u16]) -> Self {
        Self {
            width,
            height,
            advance,
            rows,
        }
    }

    pub const fn rows(self) -> &'static [u16] {
        self.rows
    }
}

const MINIMIZE: [u16; 9] = [
    0b000000000,
    0b000000000,
    0b000000000,
    0b000000000,
    0b000000000,
    0b000000000,
    0b011111110,
    0b011111110,
    0b000000000,
];

const MAXIMIZE: [u16; 9] = [
    0b011111110,
    0b010000010,
    0b010000010,
    0b010000010,
    0b010000010,
    0b010000010,
    0b010000010,
    0b011111110,
    0b000000000,
];

const RESTORE: [u16; 9] = [
    0b0011111100,
    0b0010000110,
    0b0010000010,
    0b0111111010,
    0b0100000010,
    0b0100000010,
    0b0100000010,
    0b0111111110,
    0b0000000000,
];

const CLOSE: [u16; 9] = [
    0b100000001,
    0b010000010,
    0b001000100,
    0b000101000,
    0b000010000,
    0b000101000,
    0b001000100,
    0b010000010,
    0b100000001,
];

const HELP: [u16; 11] = [
    0b001111100,
    0b010000010,
    0b000000010,
    0b000001100,
    0b000011000,
    0b000010000,
    0b000000000,
    0b000011000,
    0b000011000,
    0b010000010,
    0b001111100,
];

const PIN: [u16; 11] = [
    0b000111000,
    0b001111100,
    0b001111100,
    0b000111000,
    0b000111000,
    0b011111110,
    0b000111000,
    0b000111000,
    0b000101000,
    0b000101000,
    0b001010100,
];

const DIVIDE: [u16; 9] = [
    0b000011000,
    0b000011000,
    0b000000000,
    0b011111110,
    0b011111110,
    0b000000000,
    0b000011000,
    0b000011000,
    0b000000000,
];

const MULTIPLY: [u16; 9] = [
    0b100000001,
    0b010000010,
    0b001000100,
    0b000101000,
    0b000010000,
    0b000101000,
    0b001000100,
    0b010000010,
    0b100000001,
];

const MINUS: [u16; 9] = [
    0b000000000,
    0b000000000,
    0b000000000,
    0b011111110,
    0b011111110,
    0b000000000,
    0b000000000,
    0b000000000,
    0b000000000,
];

const SQUARE_ROOT: [u16; 9] = [
    0b000000001,
    0b000000011,
    0b000000110,
    0b000001100,
    0b100011000,
    0b110110000,
    0b011100000,
    0b001000000,
    0b000111111,
];

const PLUS_MINUS: [u16; 11] = [
    0b000011000,
    0b000011000,
    0b001111100,
    0b000011000,
    0b000011000,
    0b000000000,
    0b000000000,
    0b001111100,
    0b001111100,
    0b000000000,
    0b000000000,
];

pub const fn glyph(symbol: UiSymbol) -> UiGlyph {
    match symbol {
        UiSymbol::Minimize => UiGlyph::new(9, 9, 10, &MINIMIZE),
        UiSymbol::Maximize => UiGlyph::new(9, 9, 10, &MAXIMIZE),
        UiSymbol::Restore => UiGlyph::new(10, 9, 11, &RESTORE),
        UiSymbol::Close => UiGlyph::new(9, 9, 10, &CLOSE),
        UiSymbol::Help => UiGlyph::new(9, 11, 10, &HELP),
        UiSymbol::Pin => UiGlyph::new(9, 11, 10, &PIN),
        UiSymbol::Divide => UiGlyph::new(9, 9, 10, &DIVIDE),
        UiSymbol::Multiply => UiGlyph::new(9, 9, 10, &MULTIPLY),
        UiSymbol::Minus => UiGlyph::new(9, 9, 10, &MINUS),
        UiSymbol::SquareRoot => UiGlyph::new(9, 9, 10, &SQUARE_ROOT),
        UiSymbol::PlusMinus => UiGlyph::new(9, 11, 10, &PLUS_MINUS),
    }
}
