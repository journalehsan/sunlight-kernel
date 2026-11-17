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
        Command::MoveUnits { units, target } => {
            put(out, 0);
            put(out, units.len() as u64);
            for id in units {
                put(out, *id as u64)
            }
            tile(out, *target);
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
            u.cargo as u64,
            u.cargo_stone as u64,
        ] {
            put(&mut out, v)
        }
        match u.task {
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
        ] {
            put(&mut out, v)
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
    let mut w = World::new(civ);
    for _ in 0..count {
        let tick = r.get()?;
        let player = r.get()?.try_into().map_err(|_| LoadError::Invalid)?;
        let sequence = r.u32()?;
        let cmd = match r.get()? {
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
                    _ => return Err(LoadError::Invalid),
                };
                Command::BuildStructure {
                    worker,
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
        w.step()
    }
    Ok(w)
}
