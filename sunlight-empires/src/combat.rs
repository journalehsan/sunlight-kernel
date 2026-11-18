//! Fixed-tick combat, visibility, training and beginner opponent. Commands share
//! admission, ownership checks and stable ordering with economy commands.
use crate::*;
use alloc::vec::Vec;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Active,
    Victory,
    Defeat,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    pub from: Tile,
    pub to: Tile,
    pub until: u64,
    pub arrow: bool,
    pub damage: u32,
}
impl World {
    pub fn combat_sandbox(civ: Civilization) -> Self {
        let mut w = Self::new_match(civ);
        w.sandbox = true;
        w.resources.wood = 800;
        w.buildings.retain(|b| b.owner == 0);
        w.units.retain(|u| u.owner == 0);
        for (owner, kind, tile) in [
            (0, BuildingKind::House, Tile::new(7, 7)),
            (0, BuildingKind::Barracks, Tile::new(10, 6)),
            (0, BuildingKind::ArcheryRange, Tile::new(12, 6)),
            (1, BuildingKind::TownCenter, Tile::new(22, 10)),
        ] {
            let id = w.id();
            w.buildings.push(Building {
                id,
                owner,
                kind,
                tile,
                progress: kind.duration(),
                complete: true,
                rally: None,
                health: content::BUILDING_HEALTH,
                training: None,
                queue: Vec::new(),
            });
        }
        for (owner, kind, tile) in [
            (0, UnitKind::Guard, Tile::new(12, 11)),
            (0, UnitKind::Archer, Tile::new(12, 12)),
            (0, UnitKind::Archer, Tile::new(11, 12)),
            (1, UnitKind::Guard, Tile::new(20, 10)),
            (1, UnitKind::Archer, Tile::new(21, 11)),
        ] {
            w.spawn_unit(owner, kind, tile);
        }
        w.update_visibility();
        w
    }
    pub fn new_match(civ: Civilization) -> Self {
        let mut w = Self::new(civ);
        w.match_enabled = true;
        let id = w.id();
        w.buildings.push(Building {
            id,
            owner: 1,
            kind: BuildingKind::TownCenter,
            tile: Tile::new(36, 36),
            progress: 120,
            complete: true,
            rally: None,
            health: content::BUILDING_HEALTH,
            training: None,
            queue: Vec::new(),
        });
        for t in [Tile::new(38, 36), Tile::new(38, 38), Tile::new(36, 39)] {
            w.spawn_unit(1, UnitKind::Worker, t);
        }
        for y in 35..=40 {
            for x in 41..=44 {
                let n = Tile::new(x, y).index().unwrap();
                w.map.terrain[n] = Terrain::Forest;
                w.map.remaining[n] = 200;
            }
        }
        w.update_visibility();
        w
    }
    pub fn balance(&self, owner: PlayerId) -> &Resources {
        if owner == 0 {
            &self.resources
        } else {
            &self.enemy_resources
        }
    }
    pub(crate) fn balance_mut(&mut self, owner: PlayerId) -> &mut Resources {
        if owner == 0 {
            &mut self.resources
        } else {
            &mut self.enemy_resources
        }
    }
    pub fn population(&self, owner: PlayerId) -> usize {
        self.units.iter().filter(|u| u.owner == owner).count()
            + self
                .buildings
                .iter()
                .filter(|b| b.owner == owner)
                .map(|b| b.queue.len() + usize::from(b.training.is_some()))
                .sum::<usize>()
    }
    pub fn capacity(&self, owner: PlayerId) -> usize {
        self.buildings
            .iter()
            .filter(|b| {
                b.owner == owner
                    && b.complete
                    && matches!(b.kind, BuildingKind::TownCenter | BuildingKind::House)
            })
            .count()
            * 5
    }
    pub(crate) fn spawn_unit(&mut self, owner: PlayerId, kind: UnitKind, t: Tile) {
        let id = self.id();
        self.units.push(Unit {
            id,
            owner,
            kind,
            x: t.x * SUB + SUB / 2,
            y: t.y * SUB + SUB / 2,
            task: Task::Idle,
            path: Vec::new(),
            work: 0,
            cargo: 0,
            cargo_stone: false,
            health: kind.combat().health,
            cooldown: 0,
        });
    }
    pub fn visible(&self, owner: PlayerId, t: Tile) -> bool {
        owner < 2
            && t.index()
                .is_some_and(|n| self.visibility[owner as usize][n] != 0)
    }
    pub fn explored(&self, owner: PlayerId, t: Tile) -> bool {
        owner < 2
            && t.index()
                .is_some_and(|n| self.explored[owner as usize][n] != 0)
    }
    pub(crate) fn update_visibility(&mut self) {
        for v in &mut self.visibility {
            v.fill(0);
        }
        for (owner, t, radius) in self
            .units
            .iter()
            .filter(|u| u.health > 0)
            .map(|u| (u.owner, u.tile(), content::UNIT_SIGHT))
            .chain(
                self.buildings
                    .iter()
                    .filter(|b| b.health > 0)
                    .map(|b| (b.owner, b.tile, content::BUILDING_SIGHT)),
            )
        {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx.abs() + dy.abs() > radius {
                        continue;
                    }
                    if let Some(n) = Tile::new(t.x + dx, t.y + dy).index() {
                        self.visibility[owner as usize][n] = 1;
                        self.explored[owner as usize][n] = 1;
                    }
                }
            }
        }
    }
    pub fn target(&self, owner: PlayerId, id: EntityId) -> Option<Tile> {
        let t = self
            .units
            .iter()
            .find(|u| u.id == id && u.owner != owner && u.health > 0)
            .map(Unit::tile)
            .or_else(|| {
                self.buildings
                    .iter()
                    .find(|b| b.id == id && b.owner != owner && b.health > 0)
                    .map(|b| b.tile)
            })?;
        self.visible(owner, t).then_some(t)
    }
    pub(crate) fn clear_route(&mut self, i: usize) {
        let next = if self.units[i].centered() {
            None
        } else {
            self.units[i].path.last().copied()
        };
        self.units[i].path.clear();
        if let Some(t) = next {
            self.units[i].path.push(t);
        }
    }
    pub(crate) fn order_attack(
        &mut self,
        owner: PlayerId,
        ids: Vec<EntityId>,
        target: EntityId,
    ) -> bool {
        if self.target(owner, target).is_none() {
            return false;
        }
        self.order_stance(owner, ids, Task::Attack(target))
    }
    pub(crate) fn order_stance(&mut self, owner: PlayerId, ids: Vec<EntityId>, task: Task) -> bool {
        if let Task::AttackMove(t) = task {
            if t.index().is_none() {
                return false;
            }
        }
        let mut any = false;
        for id in ids {
            if let Some(u) = self
                .units
                .iter_mut()
                .find(|u| u.id == id && u.owner == owner && u.health > 0)
            {
                if matches!(task, Task::Attack(_) | Task::AttackMove(_))
                    && u.kind == UnitKind::Worker
                {
                    continue;
                }
                // Preserve the current reserved segment to avoid snapping on override.
                let next = if u.centered() {
                    None
                } else {
                    u.path.last().copied()
                };
                u.path.clear();
                if let Some(t) = next {
                    u.path.push(t);
                }
                u.task = task;
                u.work = 0;
                any = true;
            }
        }
        any
    }
    pub fn training_blocked(
        &self,
        owner: PlayerId,
        id: EntityId,
        kind: UnitKind,
    ) -> Option<&'static str> {
        let Some(b) = self
            .buildings
            .iter()
            .find(|b| b.id == id && b.owner == owner)
        else {
            return Some("Select your production building");
        };
        if !b.complete {
            return Some("Construction incomplete");
        }
        if b.kind.produces() != Some(kind) {
            return Some("This building cannot train that unit");
        }
        if b.queue.len() + usize::from(b.training.is_some()) >= TRAIN_QUEUE_LIMIT {
            return Some("Training queue full (5)");
        }
        if self.balance(owner).wood < kind.cost() {
            return Some("Insufficient wood");
        }
        if self.population(owner) >= self.capacity(owner) {
            return Some("Population full: build a house");
        }
        if self.units.len()
            + self
                .buildings
                .iter()
                .map(|b| b.queue.len() + usize::from(b.training.is_some()))
                .sum::<usize>()
            >= 256
        {
            return Some("World unit limit reached");
        }
        None
    }
    pub(crate) fn order_train(&mut self, owner: PlayerId, id: EntityId, kind: UnitKind) -> bool {
        if self.training_blocked(owner, id, kind).is_some() {
            return false;
        }
        let Some(b) = self.buildings.iter_mut().find(|b| {
            b.id == id
                && b.owner == owner
                && b.complete
                && b.kind.produces() == Some(kind)
                && b.queue.len() + usize::from(b.training.is_some()) < TRAIN_QUEUE_LIMIT
        }) else {
            return false;
        };
        if b.training.is_none() {
            b.training = Some((kind, kind.train_ticks()));
        } else {
            b.queue.push(kind);
        }
        self.balance_mut(owner).wood -= kind.cost();
        true
    }
    pub(crate) fn training_step(&mut self) {
        for i in 0..self.buildings.len() {
            let Some((kind, remaining)) = self.buildings[i].training else {
                continue;
            };
            if remaining > 1 {
                self.buildings[i].training = Some((kind, remaining - 1));
                continue;
            }
            // Completed units wait inside if exits are blocked. Population remains reserved.
            let owner = self.buildings[i].owner;
            if self.units.iter().filter(|u| u.owner == owner).count() >= self.capacity(owner) {
                continue;
            }
            let t = self.buildings[i].tile;
            if let Some(exit) = t
                .neighbors()
                .into_iter()
                .find(|t| self.passable(*t) && !self.occupied(*t, usize::MAX))
            {
                let owner = self.buildings[i].owner;
                self.spawn_unit(owner, kind, exit);
                self.buildings[i].training = None;
                if !self.buildings[i].queue.is_empty() {
                    let next = self.buildings[i].queue.remove(0);
                    self.buildings[i].training = Some((next, next.train_ticks()));
                }
                if let Some(rally) = self.buildings[i].rally {
                    let i = self.units.len() - 1;
                    if kind == UnitKind::Worker {
                        self.route(i, rally);
                        self.units[i].task = Task::Move;
                    } else {
                        self.units[i].task = Task::AttackMove(rally);
                    }
                }
                if owner == 0 {
                    self.emit(sound::Cue::Trained);
                }
            }
        }
    }
    fn nearest_enemy(&self, i: usize, radius: i32) -> Option<EntityId> {
        let u = &self.units[i];
        let here = u.tile();
        self.units
            .iter()
            .filter(|v| v.owner != u.owner && v.health > 0)
            .map(|v| (v.id, v.tile()))
            .chain(
                self.buildings
                    .iter()
                    .filter(|b| b.owner != u.owner && b.health > 0)
                    .map(|b| (b.id, b.tile)),
            )
            .filter(|(_, t)| self.visible(u.owner, *t) && here.distance(*t) <= radius)
            .min_by_key(|(id, t)| (here.distance(*t), *id))
            .map(|(id, _)| id)
    }
    fn approach_range(&mut self, i: usize, t: Tile, range: i32) {
        // Search a bounded ring, nearest range first. Destination reservations
        // spread attackers; exhausted melee slots wait and retry after casualties.
        for r in 1..=range {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() + dy.abs() != r {
                        continue;
                    }
                    let p = Tile::new(t.x + dx, t.y + dy);
                    if self.passable(p)
                        && !self.occupied(p, i)
                        && !self
                            .units
                            .iter()
                            .enumerate()
                            .any(|(j, u)| j != i && u.destination() == Some(p))
                        && self.route(i, p)
                    {
                        return;
                    }
                }
            }
        }
    }
    pub(crate) fn combat_step(&mut self) {
        let mut hits = Vec::with_capacity(self.units.len());
        for i in 0..self.units.len() {
            if self.units[i].health == 0 {
                continue;
            }
            self.units[i].cooldown = self.units[i].cooldown.saturating_sub(1);
            let stats = self.units[i].kind.combat();
            if stats.damage == 0 {
                continue;
            }
            let task = self.units[i].task;
            let target = match task {
                Task::Attack(id) => {
                    if self.target(self.units[i].owner, id).is_none() {
                        self.units[i].task = Task::Idle;
                        self.clear_route(i);
                        None
                    } else {
                        Some(id)
                    }
                }
                Task::Hold => self.nearest_enemy(i, stats.range),
                Task::Idle | Task::AttackMove(_) => self.nearest_enemy(i, content::UNIT_SIGHT),
                _ => None,
            };
            if let Some(id) = target {
                if matches!(task, Task::AttackMove(_)) {
                    self.units[i].work = 0;
                }
                let t = self.target(self.units[i].owner, id).unwrap();
                let here = self.units[i].tile();
                // Range uses authoritative sub-tile centers, never tile truncation.
                let (tx, ty) = self
                    .units
                    .iter()
                    .find(|u| u.id == id)
                    .map(|u| (u.x, u.y))
                    .unwrap_or((t.x * SUB + SUB / 2, t.y * SUB + SUB / 2));
                let distance = (self.units[i].x - tx).abs() + (self.units[i].y - ty).abs();
                if distance <= stats.range * SUB {
                    if self.units[i].centered() {
                        self.units[i].path.clear();
                    }
                    if self.units[i].cooldown == 0 {
                        self.units[i].cooldown = stats.cooldown;
                        hits.push((id, stats.damage));
                        let arrow = self.units[i].kind == UnitKind::Archer;
                        if self.effects.len() < 128 {
                            self.effects.push(Effect {
                                from: here,
                                to: t,
                                until: self.tick + 6,
                                arrow,
                                damage: stats.damage,
                            });
                        }
                        if self.visible(0, t) || self.visible(0, here) {
                            self.emit(if arrow {
                                sound::Cue::Arrow
                            } else {
                                sound::Cue::Melee
                            });
                        }
                    }
                } else if task != Task::Hold
                    && (self.units[i].path.is_empty() || self.tick % 20 == 0)
                {
                    self.approach_range(i, t, stats.range);
                }
            } else if let Task::AttackMove(t) = task {
                if self.units[i].path.is_empty() && self.units[i].work == 0 && self.tick % 20 == 0 {
                    // Reuse normalized group movement for destination spreading.
                    let id = self.units[i].id;
                    let owner = self.units[i].owner;
                    let routed = self.apply(Envelope {
                        tick: self.tick,
                        player: owner,
                        sequence: 0,
                        command: Command::MoveUnits {
                            units: vec![id],
                            target: t,
                        },
                    });
                    self.units[i].task = Task::AttackMove(t);
                    if routed {
                        self.units[i].work = 1;
                    }
                }
            }
        }
        // Simultaneous volleys: all units alive at this tick's combat boundary
        // may attack. Damage then resolves in stable attacker-ID order.
        for (id, damage) in hits {
            if let Some(u) = self.units.iter_mut().find(|u| u.id == id) {
                u.health = u.health.saturating_sub(damage);
            } else if let Some(b) = self.buildings.iter_mut().find(|b| b.id == id) {
                b.health = b.health.saturating_sub(damage);
            }
        }
        let dead = self
            .units
            .iter()
            .filter(|u| u.health == 0 && self.visible(0, u.tile()))
            .count()
            + self
                .buildings
                .iter()
                .filter(|b| b.health == 0 && self.visible(0, b.tile))
                .count();
        self.units.retain(|u| u.health > 0);
        self.buildings.retain(|b| b.health > 0);
        if dead > 0 {
            self.emit(sound::Cue::Death);
        }
        for i in 0..self.units.len() {
            let stale = match self.units[i].task {
                Task::Attack(id) => self.target(self.units[i].owner, id).is_none(),
                Task::Build(id) => !self.buildings.iter().any(|b| b.id == id && !b.complete),
                _ => false,
            };
            if stale {
                self.units[i].task = Task::Idle;
                self.clear_route(i);
            }
        }
    }
    pub(crate) fn check_outcome(&mut self) {
        if !self.match_enabled {
            return;
        }
        let tc = |owner| {
            self.buildings
                .iter()
                .any(|b| b.owner == owner && b.kind == BuildingKind::TownCenter)
        };
        // A simultaneous loss is defeat; unfinished Town Centers also count.
        self.outcome = if !tc(0) {
            Outcome::Defeat
        } else if !tc(1) {
            Outcome::Victory
        } else {
            Outcome::Active
        };
        match self.outcome {
            Outcome::Victory => self.emit(sound::Cue::Victory),
            Outcome::Defeat => self.emit(sound::Cue::Defeat),
            _ => {}
        }
    }
    fn ai_command(&mut self, command: Command) {
        let e = Envelope {
            tick: self.tick,
            player: 1,
            sequence: self.ai_sequence,
            command,
        };
        self.ai_sequence += 1;
        let _ = self.submit(e);
    }
    pub(crate) fn ai_decide(&mut self) {
        if self.tick % self.ai_pacing.decision_interval.max(1) != 0 {
            return;
        }
        let Some(base) = self
            .buildings
            .iter()
            .find(|b| b.owner == 1 && b.kind == BuildingKind::TownCenter)
            .cloned()
        else {
            return;
        };
        if base.rally.is_none() {
            self.ai_command(Command::SetRallyPoint {
                building: base.id,
                tile: Tile::new(base.tile.x, base.tile.y - 4),
            });
        }
        let worker = self
            .units
            .iter()
            .find(|u| {
                u.owner == 1
                    && u.kind == UnitKind::Worker
                    && (matches!(u.task, Task::Idle)
                        || (u.cargo == 0
                            && matches!(u.task, Task::Gather(_))
                            && [BuildingKind::Barracks, BuildingKind::ArcheryRange]
                                .iter()
                                .any(|k| {
                                    self.balance(1).wood >= k.cost()
                                        && !self
                                            .buildings
                                            .iter()
                                            .any(|b| b.owner == 1 && b.kind == *k)
                                })))
            })
            .map(|u| u.id);
        if let Some(worker) = worker {
            if !self
                .buildings
                .iter()
                .any(|b| b.owner == 1 && b.kind == BuildingKind::House)
            {
                self.ai_command(Command::BuildGroup {
                    units: vec![worker],
                    kind: BuildingKind::House,
                    tile: Tile::new(base.tile.x - 2, base.tile.y),
                });
            } else if let Some(kind) = [BuildingKind::Barracks, BuildingKind::ArcheryRange]
                .into_iter()
                .find(|kind| {
                    self.balance(1).wood >= kind.cost()
                        && !self
                            .buildings
                            .iter()
                            .any(|b| b.owner == 1 && b.kind == *kind)
                })
            {
                self.ai_command(Command::BuildGroup {
                    units: vec![worker],
                    kind,
                    tile: Tile::new(
                        base.tile.x - 3,
                        base.tile.y + if kind == BuildingKind::Barracks { 2 } else { 4 },
                    ),
                });
            } else if !self
                .buildings
                .iter()
                .any(|b| b.owner == 1 && b.kind == BuildingKind::Storage)
                && self.balance(1).wood >= 120
            {
                self.ai_command(Command::BuildGroup {
                    units: vec![worker],
                    kind: BuildingKind::Storage,
                    tile: Tile::new(base.tile.x + 3, base.tile.y - 2),
                });
            } else {
                // Inspect only currently visible resource nodes.
                let node = (0..CELLS)
                    .filter(|n| {
                        self.map.remaining[*n] > 0 && self.map.terrain[*n] == Terrain::Forest
                    })
                    .map(|n| Tile::new(n as i32 % MAP_SIZE, n as i32 / MAP_SIZE))
                    .filter(|t| self.visible(1, *t))
                    .min_by_key(|t| (base.tile.distance(*t), *t));
                if let Some(node) = node {
                    self.ai_command(Command::GatherGroup {
                        units: vec![worker],
                        node,
                    });
                }
            }
        }
        let army = self
            .units
            .iter()
            .filter(|u| u.owner == 1 && u.kind != UnitKind::Worker)
            .count();
        let army_limit = content::AI_INITIAL_ARMY
            + (self.tick.saturating_sub(self.ai_pacing.peaceful_ticks) / content::AI_PRESSURE_TICKS)
                .min((content::AI_MAX_ARMY - content::AI_INITIAL_ARMY) as u64)
                as usize;
        if base.training.is_none()
            && army < army_limit
            && self.population(1) < self.capacity(1)
            && self.balance(1).wood >= 80
        {
            let kind = if self
                .units
                .iter()
                .filter(|u| u.owner == 1 && u.kind != UnitKind::Worker)
                .count()
                % 3
                == 2
            {
                UnitKind::Archer
            } else {
                UnitKind::Guard
            };
            if let Some(b) = self
                .buildings
                .iter()
                .find(|b| {
                    b.owner == 1
                        && b.complete
                        && b.kind.produces() == Some(kind)
                        && b.training.is_none()
                })
                .map(|b| b.id)
            {
                if self
                    .buildings
                    .iter()
                    .any(|building| building.id == b && building.rally.is_none())
                {
                    self.ai_command(Command::SetRallyPoint {
                        building: b,
                        tile: Tile::new(base.tile.x, base.tile.y - 4),
                    });
                }
                self.ai_command(Command::Train { building: b, kind });
            }
        }
        let soldiers: Vec<_> = self
            .units
            .iter()
            .filter(|u| u.owner == 1 && u.kind != UnitKind::Worker)
            .map(|u| u.id)
            .collect();
        // Defense is limited to observed enemies near home. No hidden IDs/positions.
        let threat = self
            .units
            .iter()
            .filter(|u| {
                u.owner == 0 && self.visible(1, u.tile()) && u.tile().distance(base.tile) <= 12
            })
            .min_by_key(|u| u.id)
            .map(|u| u.id);
        if let Some(target) = threat {
            self.ai_command(Command::Attack {
                units: soldiers,
                target,
            });
        } else if self.tick >= self.ai_pacing.peaceful_ticks
            && self.tick >= self.ai_next_attack
            && soldiers.len() >= 3
        {
            let units = soldiers
                .into_iter()
                .take(
                    content::AI_ATTACK_GROUP
                        + ((self.tick - self.ai_pacing.peaceful_ticks) / content::AI_PRESSURE_TICKS)
                            .min(3) as usize,
                )
                .collect();
            // Public map patrol waypoint, never a lookup of hidden player entities.
            self.ai_command(Command::AttackMove {
                units,
                target: Tile::new(8, 8),
            });
            self.ai_next_attack = self.tick + self.ai_pacing.attack_interval.max(1);
        }
    }
}
