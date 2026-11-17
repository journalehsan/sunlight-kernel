//! Deterministic offline RTS rules. No windows, clock, graphics, input or I/O.
#![no_std]
extern crate alloc;
pub mod content;
pub mod path;
pub mod replay;
use alloc::{vec, vec::Vec};
pub use content::{BuildingKind, Civilization, UnitKind};

pub const RULES_VERSION: u32 = 1;
pub const TICKS_PER_SECOND: u32 = 20;
pub const MAP_SIZE: i32 = 64;
pub const CELLS: usize = (MAP_SIZE * MAP_SIZE) as usize;
pub const SUB: i32 = 256;
pub const MAX_COMMANDS: usize = 4096;
pub type EntityId = u32;
pub type PlayerId = u8;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
}
impl Tile {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
    pub fn index(self) -> Option<usize> {
        if self.x >= 0 && self.y >= 0 && self.x < MAP_SIZE && self.y < MAP_SIZE {
            Some((self.y * MAP_SIZE + self.x) as usize)
        } else {
            None
        }
    }
    pub fn distance(self, other: Self) -> i32 {
        (self.x - other.x).abs() + (self.y - other.y).abs()
    }
    pub fn neighbors(self) -> [Self; 4] {
        [
            Self::new(self.x, self.y - 1),
            Self::new(self.x - 1, self.y),
            Self::new(self.x + 1, self.y),
            Self::new(self.x, self.y + 1),
        ]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terrain {
    Grass,
    Forest,
    Water,
    Stone,
    Ground,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Map {
    pub terrain: Vec<Terrain>,
    pub remaining: Vec<u32>,
}
impl Default for Map {
    fn default() -> Self {
        let mut m = Self {
            terrain: vec![Terrain::Grass; CELLS],
            remaining: vec![0; CELLS],
        };
        for y in 0..MAP_SIZE {
            for x in 0..MAP_SIZE {
                let t = Tile::new(x, y);
                let i = t.index().unwrap();
                m.terrain[i] = if x == 28 && !(18..=21).contains(&y) {
                    Terrain::Water
                } else if (14..=18).contains(&x) && (6..=12).contains(&y) {
                    Terrain::Forest
                } else if (6..=9).contains(&x) && (15..=18).contains(&y) {
                    Terrain::Stone
                } else if x % 11 == 0 {
                    Terrain::Ground
                } else {
                    Terrain::Grass
                };
                if matches!(m.terrain[i], Terrain::Forest | Terrain::Stone) {
                    m.remaining[i] = 200;
                }
            }
        }
        m
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resources {
    pub food: u32,
    pub wood: u32,
    pub stone: u32,
    pub gold: u32,
    pub knowledge: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Idle,
    Move,
    Gather(Tile),
    Build(EntityId),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub id: EntityId,
    pub owner: PlayerId,
    pub kind: UnitKind,
    pub x: i32,
    pub y: i32,
    pub task: Task,
    pub path: Vec<Tile>,
    pub work: u32,
    pub cargo: u32,
    pub cargo_stone: bool,
}
impl Unit {
    pub fn tile(&self) -> Tile {
        Tile::new(self.x / SUB, self.y / SUB)
    }
    pub fn destination(&self) -> Option<Tile> {
        self.path.first().copied()
    }
    fn centered(&self) -> bool {
        self.x % SUB == SUB / 2 && self.y % SUB == SUB / 2
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Building {
    pub id: EntityId,
    pub owner: PlayerId,
    pub kind: BuildingKind,
    pub tile: Tile,
    pub progress: u32,
    pub complete: bool,
    pub rally: Option<Tile>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    MoveUnits {
        units: Vec<EntityId>,
        target: Tile,
    },
    BuildStructure {
        worker: EntityId,
        kind: BuildingKind,
        tile: Tile,
    },
    AssignWorker {
        worker: EntityId,
        node: Tile,
    },
    CancelConstruction {
        building: EntityId,
    },
    SetRallyPoint {
        building: EntityId,
        tile: Tile,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub tick: u64,
    pub player: PlayerId,
    pub sequence: u32,
    pub command: Command,
}
impl Envelope {
    pub fn key(&self) -> (u64, PlayerId, u32) {
        (self.tick, self.player, self.sequence)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    Past,
    Player,
    Duplicate,
    Full,
    TooManyUnits,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct World {
    pub tick: u64,
    pub civilization: Civilization,
    pub map: Map,
    pub resources: Resources,
    pub units: Vec<Unit>,
    pub buildings: Vec<Building>,
    pub completed: u32,
    pub accepted: u32,
    pub rejected: u32,
    pub(crate) next_id: EntityId,
    pub(crate) pending: Vec<Envelope>,
    pub(crate) log: Vec<Envelope>,
    scratch: path::Search,
}
impl World {
    pub fn new(civilization: Civilization) -> Self {
        let mut w = Self {
            tick: 0,
            civilization,
            map: Map::default(),
            resources: Resources {
                food: 200,
                wood: 240,
                stone: 100,
                gold: 100,
                knowledge: 0,
            },
            units: Vec::new(),
            buildings: Vec::new(),
            completed: 0,
            accepted: 0,
            rejected: 0,
            next_id: 1,
            pending: Vec::new(),
            log: Vec::new(),
            scratch: path::Search::new(),
        };
        let id = w.id();
        w.buildings.push(Building {
            id,
            owner: 0,
            kind: BuildingKind::TownCenter,
            tile: Tile::new(8, 8),
            progress: 120,
            complete: true,
            rally: None,
        });
        for (kind, t) in [
            (UnitKind::Worker, Tile::new(10, 8)),
            (UnitKind::Worker, Tile::new(10, 10)),
            (UnitKind::Worker, Tile::new(8, 11)),
            (UnitKind::Guard, Tile::new(12, 10)),
        ] {
            let id = w.id();
            w.units.push(Unit {
                id,
                owner: 0,
                kind,
                x: t.x * SUB + SUB / 2,
                y: t.y * SUB + SUB / 2,
                task: Task::Idle,
                path: Vec::new(),
                work: 0,
                cargo: 0,
                cargo_stone: false,
            });
        }
        w
    }
    fn id(&mut self) -> EntityId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn submit(&mut self, mut e: Envelope) -> Result<(), SubmitError> {
        if e.tick < self.tick {
            return Err(SubmitError::Past);
        }
        if e.player != 0 {
            return Err(SubmitError::Player);
        }
        if self.log.len() >= MAX_COMMANDS {
            return Err(SubmitError::Full);
        }
        if self
            .log
            .iter()
            .any(|p| p.player == e.player && p.sequence == e.sequence)
        {
            return Err(SubmitError::Duplicate);
        }
        if let Command::MoveUnits { units, .. } = &mut e.command {
            if units.len() > 64 {
                return Err(SubmitError::TooManyUnits);
            }
            units.sort_unstable();
            units.dedup();
        }
        self.log.push(e.clone());
        self.log.sort_by_key(Envelope::key);
        self.pending.push(e);
        self.pending.sort_by_key(Envelope::key);
        Ok(())
    }
    pub fn command_log(&self) -> &[Envelope] {
        &self.log
    }
    pub fn passable(&self, t: Tile) -> bool {
        t.index()
            .map(|i| matches!(self.map.terrain[i], Terrain::Grass | Terrain::Ground))
            .unwrap_or(false)
            && !self.buildings.iter().any(|b| b.tile == t)
    }
    fn occupied(&self, t: Tile, except: usize) -> bool {
        self.units
            .iter()
            .enumerate()
            .any(|(i, u)| i != except && (u.tile() == t || u.path.last() == Some(&t)))
    }
    fn blocked(&self, except: usize) -> Vec<bool> {
        let mut blocked = vec![true; CELLS];
        for y in 0..MAP_SIZE {
            for x in 0..MAP_SIZE {
                let t = Tile::new(x, y);
                blocked[t.index().unwrap()] = !self.passable(t) || self.occupied(t, except);
            }
        }
        blocked
    }
    fn route_avoiding(&mut self, i: usize, target: Tile, avoid: Option<Tile>) -> bool {
        let mut blocked = self.blocked(i);
        if let Some(n) = avoid.and_then(Tile::index) {
            blocked[n] = true;
        }
        // Finish the already reserved segment before following a replacement
        // order. This keeps input responsive without snapping or diagonal cuts.
        let next = if self.units[i].centered() {
            None
        } else {
            self.units[i].path.last().copied()
        };
        let start = next.unwrap_or(self.units[i].tile());
        if let Some(mut path) = self.scratch.find(start, target, &blocked) {
            if let Some(t) = next {
                path.push(t);
            }
            self.units[i].path = path;
            true
        } else {
            false
        }
    }
    fn route(&mut self, i: usize, target: Tile) -> bool {
        self.route_avoiding(i, target, None)
    }
    fn approach(&mut self, i: usize, target: Tile) -> bool {
        if self.units[i].tile().distance(target) == 1 && self.units[i].centered() {
            self.units[i].path.clear();
            return true;
        }
        for t in target.neighbors() {
            if self.passable(t) && !self.occupied(t, i) && self.route_avoiding(i, t, Some(target)) {
                return true;
            }
        }
        false
    }
    fn worker(&self, id: EntityId, player: PlayerId) -> Option<usize> {
        self.units
            .iter()
            .position(|u| u.id == id && u.owner == player && u.kind == UnitKind::Worker)
    }
    fn apply(&mut self, e: Envelope) -> bool {
        match e.command {
            Command::MoveUnits { units, target } => {
                if target.index().is_none() {
                    return false;
                }
                let mut destinations = Vec::new();
                let mut any = false;
                for id in units {
                    let Some(i) = self
                        .units
                        .iter()
                        .position(|u| u.id == id && u.owner == e.player)
                    else {
                        continue;
                    };
                    'search: for radius in 0i32..=5 {
                        for dy in -radius..=radius {
                            for dx in -radius..=radius {
                                if dx.abs().max(dy.abs()) != radius {
                                    continue;
                                }
                                let t = Tile::new(target.x + dx, target.y + dy);
                                if !destinations.contains(&t)
                                    && self.passable(t)
                                    && !self.occupied(t, i)
                                    && self.route(i, t)
                                {
                                    self.units[i].task = Task::Move;
                                    self.units[i].work = 0;
                                    destinations.push(t);
                                    any = true;
                                    break 'search;
                                }
                            }
                        }
                    }
                }
                any
            }
            Command::AssignWorker { worker, node } => {
                let Some(n) = node.index() else { return false };
                if self.map.remaining[n] == 0 {
                    return false;
                }
                let Some(i) = self.worker(worker, e.player) else {
                    return false;
                };
                if !self.approach(i, node) {
                    return false;
                }
                self.units[i].task = Task::Gather(node);
                self.units[i].work = 0;
                true
            }
            Command::BuildStructure { worker, kind, tile } => {
                let Some(i) = self.worker(worker, e.player) else {
                    return false;
                };
                if self.buildings.len() >= 128
                    || !self.passable(tile)
                    || self.occupied(tile, usize::MAX)
                    || self.resources.wood < kind.cost()
                {
                    return false;
                }
                // Prove a worker can reach an edge before charging or reserving.
                if !self.approach(i, tile) {
                    return false;
                }
                let id = self.id();
                self.resources.wood -= kind.cost();
                self.buildings.push(Building {
                    id,
                    owner: e.player,
                    kind,
                    tile,
                    progress: 0,
                    complete: false,
                    rally: None,
                });
                self.units[i].task = Task::Build(id);
                self.units[i].work = 0;
                true
            }
            Command::CancelConstruction { building } => {
                let Some(i) = self
                    .buildings
                    .iter()
                    .position(|b| b.id == building && b.owner == e.player && !b.complete)
                else {
                    return false;
                };
                let b = self.buildings.remove(i);
                self.resources.wood += b.kind.cost();
                for u in &mut self.units {
                    if u.task == Task::Build(building) {
                        u.task = Task::Idle;
                        if u.centered() {
                            u.path.clear();
                        } else if let Some(t) = u.path.last().copied() {
                            u.path.clear();
                            u.path.push(t);
                        }
                    }
                }
                true
            }
            Command::SetRallyPoint { building, tile } => {
                if !self.passable(tile) {
                    return false;
                }
                let Some(b) = self
                    .buildings
                    .iter_mut()
                    .find(|b| b.id == building && b.owner == e.player && b.complete)
                else {
                    return false;
                };
                b.rally = Some(tile);
                true
            }
        }
    }
    pub fn step(&mut self) {
        while self
            .pending
            .first()
            .map(|e| e.tick == self.tick)
            .unwrap_or(false)
        {
            let e = self.pending.remove(0);
            if self.apply(e) {
                self.accepted += 1;
            } else {
                self.rejected += 1;
            }
        }
        for i in 0..self.units.len() {
            if let Some(t) = self.units[i].path.last().copied() {
                // Reserve both current and next cells; no swaps or overlap.
                if self.occupied(t, i) || !self.passable(t) {
                    if self.units[i].centered() && self.tick % 20 == 0 {
                        let destination = self.units[i].destination().unwrap();
                        self.route(i, destination);
                    }
                    continue;
                }
                let u = &mut self.units[i];
                let tx = t.x * SUB + SUB / 2;
                let ty = t.y * SUB + SUB / 2;
                let speed = u.kind.speed();
                u.x += (tx - u.x).clamp(-speed, speed);
                u.y += (ty - u.y).clamp(-speed, speed);
                if u.x == tx && u.y == ty {
                    u.path.pop();
                }
                continue;
            }
            match self.units[i].task {
                Task::Idle => {}
                Task::Move => self.units[i].task = Task::Idle,
                Task::Build(id) => {
                    let Some(b) = self
                        .buildings
                        .iter_mut()
                        .find(|b| b.id == id && !b.complete)
                    else {
                        self.units[i].task = Task::Idle;
                        continue;
                    };
                    if self.units[i].tile().distance(b.tile) == 1 {
                        b.progress += 1;
                        if b.progress >= b.kind.duration() {
                            b.complete = true;
                            self.completed += 1;
                            self.units[i].task = Task::Idle;
                        }
                    }
                }
                Task::Gather(node) => self.gather(i, node),
            }
        }
        self.tick += 1;
    }
    fn gather(&mut self, i: usize, node: Tile) {
        if self.units[i].cargo > 0 {
            let here = self.units[i].tile();
            if self.buildings.iter().any(|b| {
                b.complete
                    && b.owner == self.units[i].owner
                    && b.kind.delivery()
                    && here.distance(b.tile) == 1
            }) {
                let u = &mut self.units[i];
                if u.cargo_stone {
                    self.resources.stone += u.cargo;
                } else {
                    self.resources.wood += u.cargo;
                }
                u.cargo = 0;
                u.work = 0;
                self.approach(i, node);
            } else if self.tick % 20 == 0 {
                let mut deliveries: Vec<(i32, EntityId, Tile)> = self
                    .buildings
                    .iter()
                    .filter(|b| b.complete && b.kind.delivery() && b.owner == self.units[i].owner)
                    .map(|b| (here.distance(b.tile), b.id, b.tile))
                    .collect();
                deliveries.sort_unstable();
                for (_, _, t) in deliveries {
                    if self.approach(i, t) {
                        break;
                    }
                }
            }
            return;
        }
        let n = node.index().unwrap();
        if self.map.remaining[n] == 0 {
            self.units[i].task = Task::Idle;
            return;
        }
        if self.units[i].tile().distance(node) != 1 {
            if self.tick % 20 == 0 {
                self.approach(i, node);
            }
            return;
        }
        self.units[i].work += 1;
        if self.units[i].work >= 20 {
            let amount = self.map.remaining[n].min(10);
            self.map.remaining[n] -= amount;
            self.units[i].cargo = amount;
            self.units[i].cargo_stone = self.map.terrain[n] == Terrain::Stone;
            self.units[i].work = 0;
        }
    }
    /// Canonical state bytes, explicit little endian fields, including pending
    /// orders and paths. Search scratch and arrival order of replay logs excluded.
    pub fn state_bytes(&self) -> Vec<u8> {
        replay::state_bytes(self)
    }
    pub fn checksum(&self) -> u64 {
        self.state_bytes()
            .iter()
            .fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
            })
    }
}
#[cfg(test)]
mod tests;
