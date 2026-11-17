//! Identity is independent of progression; both civilizations use the same rules.
pub const CONTENT_VERSION: u32 = 1;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Civilization {
    Persia,
    Greece,
}
impl Civilization {
    pub fn name(self) -> &'static str {
        match self {
            Self::Persia => "Persia",
            Self::Greece => "Greece",
        }
    }
    pub fn color(self) -> (u8, u8, u8) {
        match self {
            Self::Persia => (220, 154, 75),
            Self::Greece => (95, 174, 232),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Worker,
    Guard,
}
impl UnitKind {
    pub fn speed(self) -> i32 {
        match self {
            Self::Worker => 32,
            Self::Guard => 48,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildingKind {
    TownCenter,
    House,
    Storage,
    University,
}
impl BuildingKind {
    pub fn cost(self) -> u32 {
        match self {
            Self::TownCenter => 300,
            Self::House => 40,
            Self::Storage => 60,
            Self::University => 100,
        }
    }
    pub fn duration(self) -> u32 {
        match self {
            Self::House => 60,
            Self::Storage => 80,
            _ => 120,
        }
    }
    pub fn delivery(self) -> bool {
        matches!(self, Self::TownCenter | Self::Storage)
    }
}
