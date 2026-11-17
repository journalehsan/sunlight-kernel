//! Local UI state and conversion to authoritative commands.
use alloc::vec::Vec;
use sunlight_empires::{BuildingKind, Command, EntityId, Tile, World};
use sunlight_game::Camera;
use sunlight_ui::{Point, Rect};
pub const TOP: i32 = 48;
pub struct Input {
    pub camera: Camera,
    pub selected: Vec<EntityId>,
    pub building: Option<EntityId>,
    pub placing: Option<BuildingKind>,
    pub rally: bool,
    pub attack_move: bool,
    pub shift: bool,
    pub drag: Option<(i32, i32)>,
    pub pointer: (i32, i32),
    pub keys: [bool; 256],
}
impl Input {
    pub fn new() -> Self {
        Self {
            camera: Camera {
                x: 0,
                y: 0,
                tile_px: 28,
            },
            selected: Vec::new(),
            building: None,
            placing: None,
            rally: false,
            attack_move: false,
            shift: false,
            drag: None,
            pointer: (0, 0),
            keys: [false; 256],
        }
    }
    pub fn tile(&self, x: i32, y: i32) -> Tile {
        let (x, y) = self.camera.screen_to_tile(x, y, TOP);
        Tile::new(x, y)
    }
    pub fn worker(&self, w: &World) -> Option<EntityId> {
        self.selected
            .iter()
            .find(|id| {
                w.units
                    .iter()
                    .any(|u| u.id == **id && u.kind == sunlight_empires::UnitKind::Worker)
            })
            .copied()
    }
    pub fn order(&self, w: &World, t: Tile) -> Option<Command> {
        if t.index().is_none() {
            return None;
        }
        if let Some(kind) = self.placing {
            return Some(Command::BuildGroup {
                units: self.selected.clone(),
                kind,
                tile: t,
            });
        }
        if self.rally {
            return Some(Command::SetRallyPoint {
                building: self.building?,
                tile: t,
            });
        }
        if let Some(target) = w
            .units
            .iter()
            .filter(|u| u.owner != 0 && u.tile() == t && w.visible(0, t))
            .map(|u| u.id)
            .chain(
                w.buildings
                    .iter()
                    .filter(|b| b.owner != 0 && b.tile == t && w.visible(0, t))
                    .map(|b| b.id),
            )
            .next()
        {
            return Some(Command::Attack {
                units: self.selected.clone(),
                target,
            });
        }
        if self.attack_move {
            return Some(Command::AttackMove {
                units: self.selected.clone(),
                target: t,
            });
        }
        if w.map.remaining[t.index()?] > 0 {
            return Some(Command::AssignWorker {
                worker: self.worker(w)?,
                node: t,
            });
        }
        Some(Command::MoveUnits {
            units: self.selected.clone(),
            target: t,
        })
    }
    pub fn select(&mut self, w: &World, x: i32, y: i32) {
        let Some((sx, sy)) = self.drag.take() else {
            return;
        };
        if !self.shift {
            self.selected.clear();
            self.building = None;
        }
        let dragged = (sx - x).abs() + (sy - y).abs() > 6;
        let r = Rect::new(
            sx.min(x),
            sy.min(y),
            (sx - x).unsigned_abs().max(1),
            (sy - y).unsigned_abs().max(1),
        );
        let tile = self.tile(x, y);
        for u in w.units.iter().filter(|u| u.owner == 0) {
            let (px, py) = self.camera.tile_to_screen(0, 0, TOP);
            let ux = px + u.x * self.camera.tile_px / sunlight_empires::SUB;
            let uy = py + u.y * self.camera.tile_px / sunlight_empires::SUB;
            if (dragged && r.contains(Point::new(ux, uy))) || (!dragged && u.tile() == tile) {
                if !self.selected.contains(&u.id) {
                    self.selected.push(u.id);
                }
            }
        }
        if !dragged && self.selected.is_empty() {
            self.building = w
                .buildings
                .iter()
                .find(|b| b.owner == 0 && b.tile == tile)
                .map(|b| b.id);
        }
        self.selected.sort_unstable();
    }
    pub fn clear_transient(&mut self) {
        self.drag = None;
        self.shift = false;
        self.keys.fill(false);
    }
}
