//! Versioned replay save boundary. Restoring replays deterministic commands;
//! canonical state bytes provide a full digest, not an ABI-dependent struct hash.
use crate::*;
use alloc::{vec, vec::Vec};
fn put(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn tile(out: &mut Vec<u8>, t: Tile) {
    put(out, t.x as u64);
    put(out, t.y as u64);
}
fn command(out: &mut Vec<u8>, e: &Envelope) {
    put(out, e.tick);
    put(out, e.player as u64);
    put(out, e.sequence as u64);
    match &e.command {
        Command::CancelTraining { building, slot } => {
            put(out, 11);
            put(out, *building as u64);
            put(out, *slot as u64);
        }
        Command::GatherGroup { units, node } => {
            put(out, 12);
            put(out, units.len() as u64);
            for id in units {
                put(out, *id as u64);
            }
            tile(out, *node);
        }
        Command::Attack { units, target } => {
            put(out, 6);
            put(out, units.len() as u64);
            for id in units {
                put(out, *id as u64);
            }
            put(out, *target as u64);
        }
        Command::AttackMove { units, target } => {
            put(out, 7);
            put(out, units.len() as u64);
            for id in units {
                put(out, *id as u64);
            }
            tile(out, *target);
        }
        Command::Stop { units } | Command::Hold { units } => {
            put(
                out,
                if matches!(e.command, Command::Stop { .. }) {
                    8
                } else {
                    9
                },
            );
            put(out, units.len() as u64);
            for id in units {
                put(out, *id as u64);
            }
        }
        Command::Train { building, kind } => {
            put(out, 10);
            put(out, *building as u64);
            put(out, *kind as u64);
        }
        Command::MoveUnits { units, target } => {
            put(out, 0);
            put(out, units.len() as u64);
            for id in units {
                put(out, *id as u64)
            }
            tile(out, *target);
        }
        Command::BuildGroup {
            units,
            kind,
            tile: t,
        } => {
            put(out, 5);
            put(out, units.len() as u64);
            for id in units {
                put(out, *id as u64);
            }
            put(out, *kind as u64);
            tile(out, *t);
        }
        Command::BuildStructure {
            worker,
            kind,
            tile: t,
        } => {
            put(out, 1);
            put(out, *worker as u64);
            put(out, *kind as u64);
            tile(out, *t);
        }
        Command::AssignWorker { worker, node } => {
            put(out, 2);
            put(out, *worker as u64);
            tile(out, *node);
        }
        Command::CancelConstruction { building } => {
            put(out, 3);
            put(out, *building as u64);
        }
        Command::SetRallyPoint { building, tile: t } => {
            put(out, 4);
            put(out, *building as u64);
            tile(out, *t);
        }
    }
}
pub fn state_bytes(w: &World) -> Vec<u8> {
    let mut out = Vec::with_capacity(40000);
    for v in [
        RULES_VERSION as u64,
        content::CONTENT_VERSION as u64,
        w.tick,
        w.match_enabled as u64,
        w.sandbox as u64,
        w.outcome as u64,
        w.ai_sequence as u64,
        w.ai_next_attack,
        w.ai_pacing.peaceful_ticks,
        w.ai_pacing.attack_interval,
        w.ai_pacing.decision_interval,
        w.enemy_resources.food as u64,
        w.enemy_resources.wood as u64,
        w.enemy_resources.stone as u64,
        w.enemy_resources.gold as u64,
        w.enemy_resources.knowledge as u64,
        w.civilization as u64,
        w.next_id as u64,
        w.completed as u64,
        w.accepted as u64,
        w.rejected as u64,
        w.resources.food as u64,
        w.resources.wood as u64,
        w.resources.stone as u64,
        w.resources.gold as u64,
        w.resources.knowledge as u64,
    ] {
        put(&mut out, v)
    }
    for v in w.visibility.iter().chain(w.explored.iter()) {
        out.extend_from_slice(v);
    }
    for (t, r) in w.map.terrain.iter().zip(&w.map.remaining) {
        out.push(*t as u8);
        out.extend_from_slice(&r.to_le_bytes());
    }
    put(&mut out, w.units.len() as u64);
    for u in &w.units {
        for v in [
            u.id as u64,
            u.owner as u64,
            u.kind as u64,
            u.x as u64,
            u.y as u64,
            u.work as u64,
            u.health as u64,
            u.cooldown as u64,
            u.cargo as u64,
            u.cargo_stone as u64,
        ] {
            put(&mut out, v)
        }
        match u.task {
            Task::Attack(id) => {
                put(&mut out, 4);
                put(&mut out, id as u64);
            }
            Task::AttackMove(t) => {
                put(&mut out, 5);
                tile(&mut out, t);
            }
            Task::Hold => put(&mut out, 6),
            Task::Idle => put(&mut out, 0),
            Task::Move => put(&mut out, 1),
            Task::Gather(t) => {
                put(&mut out, 2);
                tile(&mut out, t)
            }
            Task::Build(id) => {
                put(&mut out, 3);
                put(&mut out, id as u64)
            }
        }
        put(&mut out, u.path.len() as u64);
        for t in &u.path {
            tile(&mut out, *t)
        }
    }
    put(&mut out, w.buildings.len() as u64);
    for b in &w.buildings {
        for v in [
            b.id as u64,
            b.owner as u64,
            b.kind as u64,
            b.progress as u64,
            b.complete as u64,
            b.health as u64,
        ] {
            put(&mut out, v)
        }
        put(&mut out, b.training.is_some() as u64);
        if let Some((kind, ticks)) = b.training {
            put(&mut out, kind as u64);
            put(&mut out, ticks as u64);
        }
        put(&mut out, b.queue.len() as u64);
        for kind in &b.queue {
            put(&mut out, *kind as u64);
        }
        tile(&mut out, b.tile);
        put(&mut out, b.rally.is_some() as u64);
        if let Some(t) = b.rally {
            tile(&mut out, t)
        }
    }
    put(&mut out, w.pending.len() as u64);
    for e in &w.pending {
        command(&mut out, e)
    }
    out
}
/// Save format is a bounded command log plus target tick. No disk I/O here.
pub fn save(w: &World) -> Vec<u8> {
    let mut out = vec![];
    out.extend_from_slice(b"SEMPM1\0\0");
    for v in [
        RULES_VERSION as u64,
        content::CONTENT_VERSION as u64,
        w.civilization as u64,
        w.tick,
        w.log.len() as u64,
        w.match_enabled as u64,
        w.sandbox as u64,
        w.ai_pacing.peaceful_ticks,
        w.ai_pacing.attack_interval,
        w.ai_pacing.decision_interval,
    ] {
        put(&mut out, v)
    }
    let mut ordered = w.log.clone();
    ordered.sort_by_key(Envelope::key);
    for e in &ordered {
        command(&mut out, e)
    }
    out
}
#[derive(Debug, PartialEq, Eq)]
pub enum LoadError {
    Invalid,
    Version,
    Limit,
}
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn get(&mut self) -> Result<u64, LoadError> {
        let s = self
            .bytes
            .get(self.at..self.at + 8)
            .ok_or(LoadError::Invalid)?;
        self.at += 8;
        Ok(u64::from_le_bytes(s.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, LoadError> {
        self.get()?.try_into().map_err(|_| LoadError::Invalid)
    }
    fn tile(&mut self) -> Result<Tile, LoadError> {
        let x = i32::try_from(self.get()? as i64).map_err(|_| LoadError::Invalid)?;
        let y = i32::try_from(self.get()? as i64).map_err(|_| LoadError::Invalid)?;
        Ok(Tile::new(x, y))
    }
}
/// Bounded replay recovery, intended for M1-scale offline sessions. A later
/// checkpoint format can avoid re-simulating long matches under a new version.
pub fn load(bytes: &[u8]) -> Result<World, LoadError> {
    if bytes.len() > 1024 * 1024 || bytes.get(..8) != Some(b"SEMPM1\0\0") {
        return Err(LoadError::Invalid);
    }
    let mut r = Reader { bytes, at: 8 };
    if r.get()? != RULES_VERSION as u64 || r.get()? != content::CONTENT_VERSION as u64 {
        return Err(LoadError::Version);
    }
    let civ = match r.get()? {
        0 => Civilization::Persia,
        1 => Civilization::Greece,
        _ => return Err(LoadError::Invalid),
    };
    let tick = r.get()?;
    let count = r.get()?;
    if tick > 72000 || count > MAX_COMMANDS as u64 {
        return Err(LoadError::Limit);
    }
    let match_enabled = r.get()?;
    if match_enabled > 1 {
        return Err(LoadError::Invalid);
    }
    let sandbox = r.get()?;
    if sandbox > 1 || (sandbox == 1 && match_enabled == 0) {
        return Err(LoadError::Invalid);
    }
    let mut w = if sandbox == 1 {
        World::combat_sandbox(civ)
    } else if match_enabled == 1 {
        World::new_match(civ)
    } else {
        World::new(civ)
    };
    w.ai_pacing = content::AiPacing {
        peaceful_ticks: r.get()?,
        attack_interval: r.get()?,
        decision_interval: r.get()?,
    };
    for _ in 0..count {
        let tick = r.get()?;
        let player = r.get()?.try_into().map_err(|_| LoadError::Invalid)?;
        let sequence = r.u32()?;
        let tag = r.get()?;
        let cmd = match tag {
            11 => Command::CancelTraining {
                building: r.u32()?,
                slot: r.u32()?,
            },
            12 => {
                let n = r.get()?;
                if n > 64 {
                    return Err(LoadError::Limit);
                }
                let mut units = Vec::new();
                for _ in 0..n {
                    units.push(r.u32()?);
                }
                Command::GatherGroup {
                    units,
                    node: r.tile()?,
                }
            }
            6..=9 => {
                let n = r.get()?;
                if n > 64 {
                    return Err(LoadError::Limit);
                }
                let mut units = Vec::new();
                for _ in 0..n {
                    units.push(r.u32()?);
                }
                match tag {
                    6 => Command::Attack {
                        units,
                        target: r.u32()?,
                    },
                    7 => Command::AttackMove {
                        units,
                        target: r.tile()?,
                    },
                    8 => Command::Stop { units },
                    _ => Command::Hold { units },
                }
            }
            10 => {
                let building = r.u32()?;
                let kind = match r.get()? {
                    0 => UnitKind::Worker,
                    1 => UnitKind::Guard,
                    2 => UnitKind::Archer,
                    _ => return Err(LoadError::Invalid),
                };
                Command::Train { building, kind }
            }
            0 => {
                let n = r.get()?;
                if n > 64 {
                    return Err(LoadError::Limit);
                }
                let mut units = Vec::new();
                for _ in 0..n {
                    units.push(r.u32()?)
                }
                Command::MoveUnits {
                    units,
                    target: r.tile()?,
                }
            }
            1 => {
                let worker = r.u32()?;
                let kind = match r.get()? {
                    0 => BuildingKind::TownCenter,
                    1 => BuildingKind::House,
                    2 => BuildingKind::Storage,
                    3 => BuildingKind::University,
                    4 => BuildingKind::Barracks,
                    5 => BuildingKind::ArcheryRange,
                    _ => return Err(LoadError::Invalid),
                };
                Command::BuildStructure {
                    worker,
                    kind,
                    tile: r.tile()?,
                }
            }
            5 => {
                let count = r.get()?;
                if count > 64 {
                    return Err(LoadError::Invalid);
                }
                let mut units = Vec::new();
                for _ in 0..count {
                    units.push(r.u32()?);
                }
                let kind = match r.get()? {
                    0 => BuildingKind::TownCenter,
                    1 => BuildingKind::House,
                    2 => BuildingKind::Storage,
                    3 => BuildingKind::University,
                    4 => BuildingKind::Barracks,
                    5 => BuildingKind::ArcheryRange,
                    _ => return Err(LoadError::Invalid),
                };
                Command::BuildGroup {
                    units,
                    kind,
                    tile: r.tile()?,
                }
            }
            2 => Command::AssignWorker {
                worker: r.u32()?,
                node: r.tile()?,
            },
            3 => Command::CancelConstruction { building: r.u32()? },
            4 => Command::SetRallyPoint {
                building: r.u32()?,
                tile: r.tile()?,
            },
            _ => return Err(LoadError::Invalid),
        };
        w.submit(Envelope {
            tick,
            player,
            sequence,
            command: cmd,
        })
        .map_err(|_| LoadError::Invalid)?;
    }
    if r.at != bytes.len() {
        return Err(LoadError::Invalid);
    }
    while w.tick < tick {
        if w.outcome != combat::Outcome::Active {
            return Err(LoadError::Invalid);
        }
        w.step()
    }
    Ok(w)
}
