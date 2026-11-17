use super::*;
fn send(w: &mut World, sequence: u32, command: Command) {
    w.submit(Envelope {
        tick: w.tick,
        player: 0,
        sequence,
        command,
    })
    .unwrap();
}
fn run(w: &mut World, n: usize) {
    for _ in 0..n {
        w.step()
    }
}
#[test]
fn replay_and_checksum_order() {
    let commands = [
        Envelope {
            tick: 0,
            player: 0,
            sequence: 2,
            command: Command::MoveUnits {
                units: vec![3],
                target: Tile::new(12, 14),
            },
        },
        Envelope {
            tick: 0,
            player: 0,
            sequence: 1,
            command: Command::MoveUnits {
                units: vec![2],
                target: Tile::new(12, 12),
            },
        },
    ];
    let mut a = World::new(Civilization::Persia);
    let mut b = a.clone();
    for e in commands.clone() {
        a.submit(e).unwrap()
    }
    for e in commands.into_iter().rev() {
        b.submit(e).unwrap()
    }
    for _ in 0..200 {
        a.step();
        b.step();
        assert_eq!(a.checksum(), b.checksum())
    }
    assert_eq!(a, b);
    assert_eq!(replay::save(&a), replay::save(&b));
}
#[test]
fn submission_boundaries() {
    let mut w = World::new(Civilization::Persia);
    let cmd = Command::MoveUnits {
        units: vec![2],
        target: Tile::new(2, 2),
    };
    let e = Envelope {
        tick: 0,
        player: 0,
        sequence: 1,
        command: cmd,
    };
    w.submit(e.clone()).unwrap();
    assert_eq!(w.submit(e.clone()), Err(SubmitError::Duplicate));
    w.step();
    let mut past = e.clone();
    past.sequence = 2;
    assert_eq!(w.submit(past), Err(SubmitError::Past));
    let mut other = e;
    other.tick = 1;
    other.player = 1;
    assert_eq!(w.submit(other), Err(SubmitError::Player));
}
#[test]
fn movement_boundaries_and_no_teleport() {
    let mut w = World::new(Civilization::Greece);
    let start = w.units[0].x;
    send(
        &mut w,
        0,
        Command::MoveUnits {
            units: vec![2, 3, 4],
            target: Tile::new(20, 10),
        },
    );
    w.step();
    assert!((w.units[0].x - start).abs() <= 32);
    for _ in 0..400 {
        w.step();
        for u in &w.units {
            assert!(u.tile().index().is_some());
        }
        for a in 0..w.units.len() {
            for b in a + 1..w.units.len() {
                assert_ne!(w.units[a].tile(), w.units[b].tile())
            }
        }
    }
    let before = w.units[0].tile();
    send(
        &mut w,
        1,
        Command::MoveUnits {
            units: vec![2],
            target: Tile::new(-1, 80),
        },
    );
    w.step();
    assert_eq!(w.units[0].tile(), before);
    assert!(w.rejected > 0);
}
#[test]
fn path_obstacle_and_unreachable() {
    let mut search = path::Search::new();
    let mut blocked = vec![false; CELLS];
    for y in 0..63 {
        blocked[Tile::new(3, y).index().unwrap()] = true
    }
    let p = search
        .find(Tile::new(1, 1), Tile::new(5, 1), &blocked)
        .unwrap();
    assert!(p.iter().any(|t| t.y == 63));
    assert!(p.iter().all(|t| !blocked[t.index().unwrap()]));
    blocked[Tile::new(3, 63).index().unwrap()] = true;
    assert!(search
        .find(Tile::new(1, 1), Tile::new(5, 1), &blocked)
        .is_none());
    assert!(search
        .find(Tile::new(-1, 0), Tile::new(5, 1), &blocked)
        .is_none());
}
#[test]
fn build_cost_completion_and_cancel() {
    let mut w = World::new(Civilization::Persia);
    let wood = w.resources.wood;
    send(
        &mut w,
        0,
        Command::BuildStructure {
            worker: 2,
            kind: BuildingKind::House,
            tile: Tile::new(11, 8),
        },
    );
    w.step();
    assert_eq!(w.resources.wood, wood - 40);
    assert!(!w.buildings[1].complete);
    let id = w.buildings[1].id;
    send(&mut w, 1, Command::CancelConstruction { building: id });
    w.step();
    assert_eq!(w.resources.wood, wood);
    assert_eq!(w.buildings.len(), 1);
    assert!(w.passable(Tile::new(11, 8)));
    send(&mut w, 2, Command::CancelConstruction { building: id });
    w.step();
    assert_eq!(w.resources.wood, wood);
    send(
        &mut w,
        3,
        Command::BuildStructure {
            worker: 2,
            kind: BuildingKind::House,
            tile: Tile::new(11, 8),
        },
    );
    run(&mut w, 200);
    assert!(w.buildings[1].complete);
    assert!(w.buildings[1].id > id);
    assert_eq!(w.resources.wood, wood - 40);
    let id = w.buildings[1].id;
    send(&mut w, 4, Command::CancelConstruction { building: id });
    w.step();
    assert_eq!(w.resources.wood, wood - 40);
}
#[test]
fn placement_and_insufficient_funds() {
    let mut w = World::new(Civilization::Persia);
    let before = w.resources;
    for (s, t) in [
        Tile::new(8, 8),
        Tile::new(10, 10),
        Tile::new(28, 0),
        Tile::new(14, 6),
        Tile::new(64, 64),
    ]
    .into_iter()
    .enumerate()
    {
        send(
            &mut w,
            s as u32,
            Command::BuildStructure {
                worker: 2,
                kind: BuildingKind::House,
                tile: t,
            },
        );
        w.step();
    }
    assert_eq!(w.resources, before);
    assert_eq!(w.buildings.len(), 1);
    w.resources.wood = 0;
    send(
        &mut w,
        8,
        Command::BuildStructure {
            worker: 2,
            kind: BuildingKind::House,
            tile: Tile::new(11, 8),
        },
    );
    w.step();
    assert_eq!(w.resources.wood, 0);
    assert_eq!(w.buildings.len(), 1);
}
#[test]
fn genuine_collection_and_conservation() {
    for (node, stone) in [(Tile::new(14, 8), false), (Tile::new(8, 15), true)] {
        let mut w = World::new(Civilization::Persia);
        let initial = w.resources;
        let n = node.index().unwrap();
        send(&mut w, 0, Command::AssignWorker { worker: 2, node });
        for _ in 0..1200 {
            w.step();
            let credited = if stone {
                w.resources.stone - initial.stone
            } else {
                w.resources.wood - initial.wood
            };
            let cargo: u32 = w.units.iter().map(|u| u.cargo).sum();
            assert_eq!(200 - w.map.remaining[n], credited + cargo);
        }
        assert!(if stone {
            w.resources.stone > initial.stone
        } else {
            w.resources.wood > initial.wood
        });
    }
}
#[test]
fn snapshot_roundtrip_inflight_and_future() {
    let mut w = World::new(Civilization::Greece);
    send(
        &mut w,
        0,
        Command::AssignWorker {
            worker: 2,
            node: Tile::new(14, 8),
        },
    );
    w.submit(Envelope {
        tick: 200,
        player: 0,
        sequence: 1,
        command: Command::BuildStructure {
            worker: 3,
            kind: BuildingKind::House,
            tile: Tile::new(11, 10),
        },
    })
    .unwrap();
    run(&mut w, 39);
    let save = replay::save(&w);
    let restored = replay::load(&save).unwrap();
    assert_eq!(restored.checksum(), w.checksum());
    assert_eq!(restored, w);
    assert!(replay::load(&save[..save.len() - 1]).is_err());
    let mut corrupt = save;
    corrupt[8] = 9;
    assert_eq!(
        replay::load(&corrupt).unwrap_err(),
        replay::LoadError::Version
    );
}
#[test]
fn repeated_scene_lifecycle() {
    for _ in 0..128 {
        let mut w = World::new(Civilization::Persia);
        send(
            &mut w,
            0,
            Command::AssignWorker {
                worker: 2,
                node: Tile::new(14, 8),
            },
        );
        run(&mut w, 100);
        assert_eq!(w.units.len(), 4);
        assert_eq!(w.buildings.len(), 1);
        assert!(w.pending.capacity() <= 4);
        drop(w);
    }
}
#[test]
fn civilizations_have_equal_stats() {
    let a = World::new(Civilization::Persia);
    let b = World::new(Civilization::Greece);
    assert_eq!(a.resources, b.resources);
    assert_eq!(a.units, b.units);
    assert_eq!(a.buildings, b.buildings);
}
#[test]
fn mid_movement_orders_do_not_snap() {
    let mut w = World::new(Civilization::Persia);
    send(
        &mut w,
        0,
        Command::MoveUnits {
            units: vec![2],
            target: Tile::new(20, 8),
        },
    );
    run(&mut w, 3);
    let (x, y) = (w.units[0].x, w.units[0].y);
    send(
        &mut w,
        1,
        Command::MoveUnits {
            units: vec![2],
            target: Tile::new(5, 5),
        },
    );
    w.step();
    assert!((w.units[0].x - x).abs() + (w.units[0].y - y).abs() <= 32);
    run(&mut w, 400);
    assert_eq!(w.units[0].tile(), Tile::new(5, 5));
}
#[test]
fn invalid_world_orders_survive_replay() {
    let mut w = World::new(Civilization::Persia);
    send(
        &mut w,
        0,
        Command::MoveUnits {
            units: vec![2],
            target: Tile::new(-1, i32::MAX),
        },
    );
    w.step();
    assert_eq!(w.rejected, 1);
    assert_eq!(
        replay::load(&replay::save(&w)).unwrap().checksum(),
        w.checksum()
    );
}
#[test]
fn construction_never_allows_a_route_through_its_site() {
    let mut w = World::new(Civilization::Persia);
    send(
        &mut w,
        0,
        Command::BuildStructure {
            worker: 2,
            kind: BuildingKind::House,
            tile: Tile::new(11, 8),
        },
    );
    w.step();
    for _ in 0..200 {
        w.step();
        assert!(w.units.iter().all(|u| u.tile() != Tile::new(11, 8)));
    }
    assert!(w.buildings[1].complete);
}

#[test]
fn storage_delivers_only_after_completion() {
    let mut w = World::new(Civilization::Persia);
    send(
        &mut w,
        0,
        Command::BuildStructure {
            worker: 3,
            kind: BuildingKind::Storage,
            tile: Tile::new(13, 7),
        },
    );
    send(
        &mut w,
        1,
        Command::AssignWorker {
            worker: 2,
            node: Tile::new(14, 8),
        },
    );
    let mut used_completed_storage = false;
    let mut used_town_center_before_completion = false;
    for _ in 0..500 {
        let cargo = w.units[0].cargo;
        w.step();
        if cargo > 0 && w.units[0].cargo == 0 {
            let here = w.units[0].tile();
            let storage = &w.buildings[1];
            if !storage.complete {
                assert_eq!(here.distance(w.buildings[0].tile), 1);
                used_town_center_before_completion = true;
            } else if here.distance(storage.tile) == 1 && here.distance(w.buildings[0].tile) > 1 {
                used_completed_storage = true;
            }
        }
    }
    assert!(used_town_center_before_completion);
    assert!(used_completed_storage);
    assert!(w.resources.wood > 240 - 60);
}

#[test]
fn cancel_partially_built_structure_releases_worker_and_reservation() {
    let mut w = World::new(Civilization::Persia);
    send(
        &mut w,
        0,
        Command::BuildStructure {
            worker: 2,
            kind: BuildingKind::House,
            tile: Tile::new(11, 8),
        },
    );
    run(&mut w, 30);
    let site = &w.buildings[1];
    assert!(site.progress > 0 && !site.complete);
    let id = site.id;
    send(&mut w, 1, Command::CancelConstruction { building: id });
    w.step();
    assert_eq!(w.resources.wood, 240);
    assert!(w.passable(Tile::new(11, 8)));
    assert!(w.units.iter().all(|u| u.task != Task::Build(id)));
    send(&mut w, 2, Command::CancelConstruction { building: id });
    w.step();
    assert_eq!(w.resources.wood, 240);
    assert_eq!(w.buildings.len(), 1);
}
