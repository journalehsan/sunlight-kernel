//! Shared lightweight command layout and hit testing for the native Canvas.
use crate::input::Input;
use sunlight_empires::{BuildingKind, UnitKind, World};
pub const HEIGHT: i32 = 176;
#[derive(Clone, Copy)]
pub enum Action {
    Build(BuildingKind),
    Train(UnitKind),
    Move,
    Gather,
    Attack,
    AttackMove,
    Stop,
    Hold,
    Rally,
    Cancel,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Build(BuildingKind::House) => "House 40W [H]",
            Self::Build(BuildingKind::Storage) => "Storage 60W [S]",
            Self::Build(BuildingKind::University) => "University 100W [U]",
            Self::Build(BuildingKind::Barracks) => "Barracks 80W",
            Self::Build(_) => "Archery 100W",
            Self::Train(UnitKind::Worker) => "Worker 30W [W]",
            Self::Train(UnitKind::Guard) => "Infantry 35W [I]",
            Self::Train(UnitKind::Archer) => "Archer 45W [B]",
            Self::Move => "Move",
            Self::Gather => "Gather",
            Self::Attack => "Attack",
            Self::AttackMove => "Attack Move [A]",
            Self::Stop => "Stop [O]",
            Self::Hold => "Hold [D]",
            Self::Rally => "Rally [R]",
            Self::Cancel => "Cancel site [X]",
        }
    }
    pub fn cost(self) -> u32 {
        match self {
            Self::Build(k) => k.cost(),
            Self::Train(k) => k.cost(),
            _ => 0,
        }
    }
}
pub fn actions(w: &World, input: &Input) -> &'static [Action] {
    use Action::*;
    if let Some(b) = input
        .building
        .and_then(|id| w.buildings.iter().find(|b| b.id == id && b.owner == 0))
    {
        if !b.complete {
            return &[Cancel];
        }
        return match b.kind {
            BuildingKind::TownCenter => &[Train(UnitKind::Worker), Rally],
            BuildingKind::Barracks => &[Train(UnitKind::Guard), Rally],
            BuildingKind::ArcheryRange => &[Train(UnitKind::Archer), Rally],
            _ => &[],
        };
    }
    if input.worker(w).is_some()
        && input.selected.iter().any(|id| {
            w.units
                .iter()
                .any(|u| u.id == *id && u.kind != UnitKind::Worker)
        })
    {
        &[
            Build(BuildingKind::House),
            Build(BuildingKind::Storage),
            Build(BuildingKind::Barracks),
            Build(BuildingKind::ArcheryRange),
            Build(BuildingKind::University),
            Move,
            Stop,
            Gather,
            Attack,
            AttackMove,
            Hold,
        ]
    } else if input.worker(w).is_some() {
        &[
            Build(BuildingKind::House),
            Build(BuildingKind::Storage),
            Build(BuildingKind::Barracks),
            Build(BuildingKind::ArcheryRange),
            Build(BuildingKind::University),
            Move,
            Stop,
            Gather,
        ]
    } else if input
        .selected
        .iter()
        .any(|id| w.units.iter().any(|u| u.id == *id))
    {
        &[Move, Attack, AttackMove, Hold, Stop]
    } else {
        &[]
    }
}
pub fn button(width: u32, bottom: i32, i: usize, count: usize) -> sunlight_ui::Rect {
    let left = width as i32 * 28 / 100;
    let columns = if count > 8 { 3 } else { 2 };
    let bw = (width as i32 * 48 / 100) / columns as i32;
    sunlight_ui::Rect::new(
        left + (i % columns) as i32 * bw,
        bottom + 32 + (i / columns) as i32 * 30,
        (bw - 5).max(1) as u32,
        26,
    )
}
pub fn queue_button(width: u32, bottom: i32, i: usize) -> sunlight_ui::Rect {
    sunlight_ui::Rect::new(
        width as i32 * 77 / 100,
        bottom + 32 + i as i32 * 24,
        (width * 23 / 100).saturating_sub(8),
        22,
    )
}
