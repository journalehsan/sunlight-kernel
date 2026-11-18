use crate::*;
fn order(w: &mut World, command: Command) {
    w.submit(Envelope {
        tick: w.tick,
        player: 0,
        sequence: w.command_log().iter().filter(|e| e.player == 0).count() as u32,
        command,
    })
    .unwrap();
}
fn run(w: &mut World, ticks: usize) {
    for _ in 0..ticks {
        w.step();
    }
}
fn replay_equal(w: &World) {
    let loaded = replay::load(&replay::save(w)).unwrap();
    assert_eq!(w.state_bytes(), loaded.state_bytes());
}
fn producer(w: &World, kind: BuildingKind) -> EntityId {
    w.buildings
        .iter()
        .find(|b| b.owner == 0 && b.kind == kind)
        .unwrap()
        .id
}
#[test]
fn sandbox_production_fifo_rally_cancel_replays() {
    let mut w = World::combat_sandbox(Civilization::Greece);
    for (building_kind, kind) in [
        (BuildingKind::TownCenter, UnitKind::Worker),
        (BuildingKind::Barracks, UnitKind::Guard),
        (BuildingKind::ArcheryRange, UnitKind::Archer),
    ] {
        let building = producer(&w, building_kind);
        order(
            &mut w,
            Command::SetRallyPoint {
                building,
                tile: Tile::new(9, 4),
            },
        );
        order(&mut w, Command::Train { building, kind });
    }
    w.step();
    assert_eq!(w.population(0), w.capacity(0));
    assert_eq!(w.units.iter().filter(|u| u.owner == 0).count(), 7);
    replay_equal(&w);
    let building = producer(&w, BuildingKind::ArcheryRange);
    order(&mut w, Command::CancelTraining { building, slot: 0 });
    w.step();
    assert_eq!(w.resources.wood, 800 - 30 - 35);
    order(
        &mut w,
        Command::Train {
            building,
            kind: UnitKind::Archer,
        },
    );
    run(&mut w, 120);
    assert_eq!(w.units.iter().filter(|u| u.owner == 0).count(), 10);
    assert!(w
        .buildings
        .iter()
        .filter(|b| b.owner == 0)
        .all(|b| b.training.is_none()));
    replay_equal(&w);
}
#[test]
fn bounded_queue_refunds_each_paid_slot_once() {
    let mut w = World::new(Civilization::Persia);
    w.resources.wood = 1000;
    let mut house = w.buildings[0].clone();
    house.kind = BuildingKind::House;
    house.id = 99;
    house.tile = Tile::new(5, 5);
    w.buildings.push(house);
    for _ in 0..6 {
        order(
            &mut w,
            Command::Train {
                building: 1,
                kind: UnitKind::Worker,
            },
        );
    }
    w.step();
    assert_eq!(w.rejected, 1);
    assert_eq!(w.resources.wood, 850);
    assert_eq!(w.population(0), 9);
    assert_eq!(w.buildings[0].queue.len(), 4);
    order(
        &mut w,
        Command::CancelTraining {
            building: 1,
            slot: 3,
        },
    );
    w.step();
    assert_eq!(w.resources.wood, 880);
    for _ in 0..5 {
        order(
            &mut w,
            Command::CancelTraining {
                building: 1,
                slot: 0,
            },
        );
    }
    w.step();
    assert_eq!(w.resources.wood, 1000);
    assert_eq!(w.rejected, 2);
    assert_eq!(w.population(0), 4);
}
#[test]
fn insufficient_resources_population_ownership_and_building_rules() {
    let mut w = World::new(Civilization::Persia);
    assert_eq!(
        w.training_blocked(0, 1, UnitKind::Guard),
        Some("This building cannot train that unit")
    );
    assert!(w.training_blocked(1, 1, UnitKind::Worker).is_some());
    w.resources.wood = 29;
    order(
        &mut w,
        Command::Train {
            building: 1,
            kind: UnitKind::Worker,
        },
    );
    w.step();
    assert_eq!(w.resources.wood, 29);
    assert_eq!(w.population(0), 4);
    w.resources.wood = 90;
    for _ in 0..2 {
        order(
            &mut w,
            Command::Train {
                building: 1,
                kind: UnitKind::Worker,
            },
        );
    }
    w.step();
    assert_eq!(w.resources.wood, 60);
    assert_eq!(w.population(0), 5);
}
#[test]
fn completed_production_waits_for_safe_exit() {
    let mut w = World::new(Civilization::Persia);
    order(
        &mut w,
        Command::Train {
            building: 1,
            kind: UnitKind::Worker,
        },
    );
    w.step();
    for t in Tile::new(8, 8).neighbors() {
        w.map.terrain[t.index().unwrap()] = Terrain::Water;
    }
    run(&mut w, 100);
    assert_eq!(w.units.len(), 4);
    assert!(w.buildings[0].training.is_some());
    w.map.terrain[Tile::new(8, 7).index().unwrap()] = Terrain::Grass;
    w.step();
    assert_eq!(w.units.len(), 5);
    assert_eq!(w.units.last().unwrap().tile(), Tile::new(8, 7));
}
#[test]
fn military_construction_group_cost_and_replay() {
    let mut w = World::new(Civilization::Persia);
    for (kind, tile) in [
        (BuildingKind::Barracks, Tile::new(10, 5)),
        (BuildingKind::ArcheryRange, Tile::new(12, 5)),
    ] {
        let wood = w.resources.wood;
        order(
            &mut w,
            Command::BuildGroup {
                units: vec![4, 3, 2, 2, 999],
                kind,
                tile,
            },
        );
        run(&mut w, 400);
        assert_eq!(w.resources.wood, wood - kind.cost());
        assert_eq!(
            w.buildings
                .iter()
                .filter(|b| b.kind == kind && b.complete)
                .count(),
            1
        );
        replay_equal(&w);
    }
}
#[test]
fn group_gather_delivery_conserves_resources_and_replays() {
    let mut w = World::new(Civilization::Persia);
    let initial = w.resources.wood
        + w.map
            .remaining
            .iter()
            .enumerate()
            .filter(|(i, _)| w.map.terrain[*i] == Terrain::Forest)
            .map(|(_, v)| *v)
            .sum::<u32>();
    order(
        &mut w,
        Command::GatherGroup {
            units: vec![4, 2, 3, 2, 5, 999],
            node: Tile::new(14, 8),
        },
    );
    run(&mut w, 1000);
    assert!(w.resources.wood > 240);
    assert!(
        w.units
            .iter()
            .filter(|u| matches!(u.task, Task::Gather(_)))
            .count()
            >= 2
    );
    let total = w.resources.wood
        + w.units
            .iter()
            .filter(|u| !u.cargo_stone)
            .map(|u| u.cargo)
            .sum::<u32>()
        + w.map
            .remaining
            .iter()
            .enumerate()
            .filter(|(i, _)| w.map.terrain[*i] == Terrain::Forest)
            .map(|(_, v)| *v)
            .sum::<u32>();
    assert_eq!(initial, total);
    replay_equal(&w);
}
#[test]
fn sandbox_group_combat_genuine_victory_replays() {
    let mut w = World::combat_sandbox(Civilization::Persia);
    let units: Vec<_> = w
        .units
        .iter()
        .filter(|u| u.owner == 0 && u.kind != UnitKind::Worker)
        .map(|u| u.id)
        .collect();
    order(
        &mut w,
        Command::AttackMove {
            units,
            target: Tile::new(22, 10),
        },
    );
    run(&mut w, 4000);
    assert_eq!(w.outcome, combat::Outcome::Victory);
    assert!(!w
        .buildings
        .iter()
        .any(|b| b.owner == 1 && b.kind == BuildingKind::TownCenter));
    replay_equal(&w);
}
#[test]
fn sandbox_is_separate_from_normal_start() {
    let normal = World::new_match(Civilization::Persia);
    let sandbox = World::combat_sandbox(Civilization::Persia);
    assert!(!normal.sandbox);
    assert!(sandbox.sandbox);
    assert_eq!(normal.resources.wood, 240);
    assert!(!normal
        .buildings
        .iter()
        .any(|b| b.owner == 0 && b.kind == BuildingKind::Barracks));
    assert_ne!(normal.checksum(), sandbox.checksum());
    replay_equal(&normal);
    replay_equal(&sandbox);
}

#[test]
fn fifo_cancel_waiting_preserves_active_work_and_refunds() {
    let mut w = World::combat_sandbox(Civilization::Persia);
    for _ in 0..3 {
        order(
            &mut w,
            Command::Train {
                building: 1,
                kind: UnitKind::Worker,
            },
        );
    }
    run(&mut w, 20);
    assert_eq!(w.buildings[0].training, Some((UnitKind::Worker, 40)));
    order(
        &mut w,
        Command::CancelTraining {
            building: 1,
            slot: 1,
        },
    );
    w.step();
    assert_eq!(w.buildings[0].training, Some((UnitKind::Worker, 39)));
    assert_eq!(w.resources.wood, 740);
    run(&mut w, 39);
    assert_eq!(w.buildings[0].training, Some((UnitKind::Worker, 60)));
    assert_eq!(w.buildings[0].queue.len(), 0);
    replay_equal(&w);
}
#[test]
fn production_destruction_releases_reservations_without_refund() {
    let mut w = World::combat_sandbox(Civilization::Persia);
    let building = producer(&w, BuildingKind::Barracks);
    for _ in 0..3 {
        order(
            &mut w,
            Command::Train {
                building,
                kind: UnitKind::Guard,
            },
        );
    }
    w.step();
    assert_eq!(w.population(0), 10);
    let wood = w.resources.wood;
    w.buildings
        .iter_mut()
        .find(|b| b.id == building)
        .unwrap()
        .health = 0;
    w.step();
    assert_eq!(w.population(0), 7);
    assert_eq!(w.resources.wood, wood);
    order(&mut w, Command::CancelTraining { building, slot: 0 });
    w.step();
    assert_eq!(w.resources.wood, wood);
}
#[test]
fn gathering_keeps_cargo_when_dropoff_is_destroyed() {
    let mut w = World::new(Civilization::Persia);
    order(
        &mut w,
        Command::AssignWorker {
            worker: 2,
            node: Tile::new(14, 8),
        },
    );
    for _ in 0..400 {
        w.step();
        if w.units.iter().any(|u| u.id == 2 && u.cargo > 0) {
            break;
        }
    }
    let worker = w.units.iter().find(|u| u.id == 2).unwrap();
    assert_eq!(worker.cargo, 10);
    let wood = w.resources.wood;
    w.buildings.clear();
    run(&mut w, 100);
    let worker = w.units.iter().find(|u| u.id == 2).unwrap();
    assert_eq!(worker.cargo, 10);
    assert_eq!(worker.task, Task::Idle);
    assert_eq!(w.resources.wood, wood);
}
#[test]
fn losing_capacity_blocks_paid_completion_until_house_rebuilt() {
    let mut w = World::combat_sandbox(Civilization::Persia);
    order(
        &mut w,
        Command::Train {
            building: 1,
            kind: UnitKind::Worker,
        },
    );
    w.step();
    let house = w
        .buildings
        .iter()
        .find(|b| b.owner == 0 && b.kind == BuildingKind::House)
        .unwrap()
        .clone();
    w.buildings.retain(|b| b.id != house.id);
    run(&mut w, 100);
    assert!(w.buildings[0].training.is_some());
    assert_eq!(w.units.iter().filter(|u| u.owner == 0).count(), 7);
    w.buildings.push(house);
    w.step();
    assert!(w.buildings[0].training.is_none());
    assert_eq!(w.units.iter().filter(|u| u.owner == 0).count(), 8);
}
