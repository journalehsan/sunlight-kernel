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
    Back,
    Forward,
    Up,
    Search,
    Home,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Music,
    Videos,
    RootFs,
    Volume,
    Network,
    Folder,
    MissingFolder,
    File,
    FilesApp,
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

const BACK: [u16; 9] = [
    0b000010000,
    0b000110000,
    0b001110000,
    0b011111111,
    0b001110000,
    0b000110000,
    0b000010000,
    0b000000000,
    0b000000000,
];

const FORWARD: [u16; 9] = [
    0b000010000,
    0b000011000,
    0b000001100,
    0b111111110,
    0b000001100,
    0b000011000,
    0b000010000,
    0b000000000,
    0b000000000,
];

const UP: [u16; 9] = [
    0b000010000,
    0b000111000,
    0b001111100,
    0b000111000,
    0b000111000,
    0b000111000,
    0b000111000,
    0b000000000,
    0b000000000,
];

const SEARCH: [u16; 9] = [
    0b000111000,
    0b001000100,
    0b010000010,
    0b010000010,
    0b010000010,
    0b001000100,
    0b000111010,
    0b000001100,
    0b000000000,
];

const HOME: [u16; 9] = [
    0b000010000,
    0b000111000,
    0b001010100,
    0b010010010,
    0b111111111,
    0b100000001,
    0b101111101,
    0b101000101,
    0b111111111,
];

const DESKTOP: [u16; 9] = [
    0b111111111,
    0b100000001,
    0b101111101,
    0b101000101,
    0b101000101,
    0b101111101,
    0b100000001,
    0b000111000,
    0b001000100,
];

const DOCUMENTS: [u16; 9] = [
    0b001111100,
    0b001000100,
    0b001000110,
    0b001001010,
    0b001010010,
    0b001010010,
    0b001000010,
    0b001000010,
    0b001111110,
];

const DOWNLOADS: [u16; 9] = [
    0b000010000,
    0b000010000,
    0b000010000,
    0b000010000,
    0b001111100,
    0b000111000,
    0b000010000,
    0b001111100,
    0b001111100,
];

const PICTURES: [u16; 9] = [
    0b000000000,
    0b000111000,
    0b001111100,
    0b011000110,
    0b110010011,
    0b111111111,
    0b111000111,
    0b110000011,
    0b000000000,
];

const MUSIC: [u16; 9] = [
    0b000001100,
    0b000001100,
    0b000001100,
    0b000011100,
    0b000101100,
    0b001001100,
    0b001001100,
    0b000000000,
    0b000000000,
];

const VIDEOS: [u16; 9] = [
    0b111111111,
    0b100000001,
    0b101000001,
    0b101110001,
    0b101111101,
    0b101110001,
    0b101000001,
    0b100000001,
    0b111111111,
];

const ROOT_FS: [u16; 9] = [
    0b000111000,
    0b001111100,
    0b011000110,
    0b011000110,
    0b011000110,
    0b001111100,
    0b000111000,
    0b000111000,
    0b000111000,
];

const VOLUME: [u16; 9] = [
    0b000111000,
    0b001111100,
    0b011000110,
    0b011000110,
    0b011000110,
    0b011111110,
    0b011000110,
    0b001111100,
    0b000111000,
];

const NETWORK: [u16; 9] = [
    0b000010000,
    0b000101000,
    0b001000100,
    0b010010010,
    0b100000001,
    0b010010010,
    0b001000100,
    0b000101000,
    0b000010000,
];

const FOLDER: [u16; 9] = [
    0b000000000,
    0b001111100,
    0b011111110,
    0b011000010,
    0b011111111,
    0b011111111,
    0b011111111,
    0b001111110,
    0b000000000,
];

const MISSING_FOLDER: [u16; 9] = [
    0b000000000,
    0b001111100,
    0b011111110,
    0b011000010,
    0b011011111,
    0b011110110,
    0b011111111,
    0b001111110,
    0b000000000,
];

const FILE: [u16; 9] = [
    0b001111100,
    0b001000100,
    0b001001100,
    0b001010100,
    0b001100100,
    0b001000100,
    0b001000100,
    0b001000100,
    0b001111110,
];

const FILES_APP: [u16; 9] = [
    0b000111000,
    0b001111100,
    0b011111110,
    0b011000010,
    0b011111110,
    0b011001110,
    0b011000010,
    0b001111110,
    0b000111000,
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
        UiSymbol::Back => UiGlyph::new(9, 9, 10, &BACK),
        UiSymbol::Forward => UiGlyph::new(9, 9, 10, &FORWARD),
        UiSymbol::Up => UiGlyph::new(9, 9, 10, &UP),
        UiSymbol::Search => UiGlyph::new(9, 9, 10, &SEARCH),
        UiSymbol::Home => UiGlyph::new(9, 9, 10, &HOME),
        UiSymbol::Desktop => UiGlyph::new(9, 9, 10, &DESKTOP),
        UiSymbol::Documents => UiGlyph::new(9, 9, 10, &DOCUMENTS),
        UiSymbol::Downloads => UiGlyph::new(9, 9, 10, &DOWNLOADS),
        UiSymbol::Pictures => UiGlyph::new(9, 9, 10, &PICTURES),
        UiSymbol::Music => UiGlyph::new(9, 9, 10, &MUSIC),
        UiSymbol::Videos => UiGlyph::new(9, 9, 10, &VIDEOS),
        UiSymbol::RootFs => UiGlyph::new(9, 9, 10, &ROOT_FS),
        UiSymbol::Volume => UiGlyph::new(9, 9, 10, &VOLUME),
        UiSymbol::Network => UiGlyph::new(9, 9, 10, &NETWORK),
        UiSymbol::Folder => UiGlyph::new(9, 9, 10, &FOLDER),
        UiSymbol::MissingFolder => UiGlyph::new(9, 9, 10, &MISSING_FOLDER),
        UiSymbol::File => UiGlyph::new(9, 9, 10, &FILE),
        UiSymbol::FilesApp => UiGlyph::new(9, 9, 10, &FILES_APP),
    }
}
