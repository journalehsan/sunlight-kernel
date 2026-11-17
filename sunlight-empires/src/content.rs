//! Identity is independent of progression; both civilizations use the same rules.
pub const CONTENT_VERSION: u32 = 2;
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
    Guard, // Infantry (retain the M1 public name).
    Archer,
}
impl UnitKind {
    pub fn speed(self) -> i32 {
        match self {
            Self::Worker => 32,
            Self::Guard => 48,
            Self::Archer => 40,
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

/// Integer balance data; range and sight use Manhattan tiles.
#[derive(Clone, Copy, Debug)]
pub struct CombatStats {
    pub health: u32,
    pub damage: u32,
    pub range: i32,
    pub cooldown: u32,
}
impl UnitKind {
    pub fn combat(self) -> CombatStats {
        match self {
            Self::Worker => CombatStats {
                health: 50,
                damage: 0,
                range: 1,
                cooldown: 20,
            },
            Self::Guard => CombatStats {
                health: 100,
                damage: 12,
                range: 1,
                cooldown: 20,
            },
            Self::Archer => CombatStats {
                health: 65,
                damage: 8,
                range: 5,
                cooldown: 25,
            },
        }
    }
    /// Wood-only militia economy: no passive or infinite food income.
    pub fn cost(self) -> u32 {
        match self {
            Self::Worker => 30,
            Self::Guard => 35,
            Self::Archer => 45,
        }
    }
    pub fn train_ticks(self) -> u32 {
        match self {
            Self::Worker => 60,
            Self::Guard => 80,
            Self::Archer => 100,
        }
    }
}
pub const UNIT_SIGHT: i32 = 7;
pub const BUILDING_SIGHT: i32 = 9;
pub const BUILDING_HEALTH: u32 = 400;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiPacing {
    pub peaceful_ticks: u64,
    pub attack_interval: u64,
    pub decision_interval: u64,
}
impl Default for AiPacing {
    fn default() -> Self {
        Self {
            peaceful_ticks: 3600,
            attack_interval: 1200,
            decision_interval: 100,
        }
    }
}
pub const AI_INITIAL_ARMY: usize = 4;
pub const AI_MAX_ARMY: usize = 7;
pub const AI_PRESSURE_TICKS: u64 = 2400;
pub const AI_ATTACK_GROUP: usize = 3;
