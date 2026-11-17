use super::*;
use combat::Outcome;
fn cmd(w: &mut World, player: u8, command: Command) {
    let sequence = 100_000 + w.command_log().len() as u32;
    w.submit(Envelope {
        tick: w.tick,
        player,
        sequence,
        command,
    })
    .unwrap();
}
fn run(w: &mut World, n: usize) {
    for _ in 0..n {
        w.step();
    }
}
fn arena(kind: UnitKind, distance: i32) -> World {
    let mut w = World::new(Civilization::Persia);
    w.units.clear();
    w.spawn_unit(0, kind, Tile::new(20, 20));
    w.spawn_unit(1, UnitKind::Worker, Tile::new(20 + distance, 20));
    w.update_visibility();
    w
}
fn attack(w: &mut World) {
    cmd(
        w,
        0,
        Command::Attack {
            units: vec![w.units[0].id],
            target: w.units[1].id,
        },
    );
}
#[test]
fn melee_damage_and_cooldown() {
    let mut w = arena(UnitKind::Guard, 1);
    attack(&mut w);
    w.step();
    assert_eq!(w.units[1].health, 38);
    run(&mut w, 19);
    assert_eq!(w.units[1].health, 38);
    w.step();
    assert_eq!(w.units[1].health, 26);
}
#[test]
fn ranged_damage_and_range() {
    let mut w = arena(UnitKind::Archer, 5);
    attack(&mut w);
    w.step();
    assert_eq!(w.units[1].health, 42);
    assert!(w.effects.iter().any(|e| e.arrow));
    let mut w = arena(UnitKind::Archer, 6);
    attack(&mut w);
    w.step();
    assert_eq!(w.units[1].health, 50);
    assert!(!w.units[0].path.is_empty());
}
#[test]
fn melee_cannot_hit_outside_range() {
    let mut w = arena(UnitKind::Guard, 2);
    attack(&mut w);
    w.step();
    assert_eq!(w.units[1].health, 50);
}
#[test]
fn death_cleanup_and_stale_target() {
    let mut w = arena(UnitKind::Guard, 1);
    w.units[1].health = 1;
    let dead = w.units[1].id;
    attack(&mut w);
    w.step();
    assert!(!w.units.iter().any(|u| u.id == dead));
    assert_eq!(w.units[0].task, Task::Idle);
    assert!(w.events.iter().any(|e| e.cue == sound::Cue::Death));
    let id = w.units[0].id;
    cmd(
        &mut w,
        0,
        Command::Attack {
            units: vec![id],
            target: dead,
        },
    );
    w.step();
    assert_eq!(w.rejected, 1);
}
#[test]
fn simultaneous_lethal_volley() {
    let mut w = arena(UnitKind::Guard, 1);
    w.units[1].kind = UnitKind::Guard;
    for u in &mut w.units {
        u.health = 1;
        u.task = Task::Hold;
    }
    w.step();
    assert!(w.units.is_empty());
    run(&mut w, 30);
    assert!(w.units.is_empty());
}
#[test]
fn group_attack_normalizes_and_rejects_wrong_ownership() {
    let mut w = arena(UnitKind::Guard, 1);
    let ours = w.units[0].id;
    let enemy = w.units[1].id;
    cmd(
        &mut w,
        0,
        Command::Attack {
            units: vec![enemy, ours, ours, u32::MAX],
            target: enemy,
        },
    );
    w.step();
    assert_eq!(w.units[1].health, 38);
    assert_eq!(w.units[1].task, Task::Idle);
    match &w.command_log()[0].command {
        Command::Attack { units, .. } => assert_eq!(units, &vec![ours, enemy, u32::MAX]),
        _ => panic!(),
    }
}
#[test]
fn multiple_melee_approach_and_kill_building() {
    let mut w = arena(UnitKind::Guard, 7);
    w.units.pop();
    for t in [
        Tile::new(20, 19),
        Tile::new(19, 20),
        Tile::new(20, 21),
        Tile::new(19, 21),
    ] {
        w.spawn_unit(0, UnitKind::Guard, t);
    }
    let id = w.id();
    w.buildings.push(Building {
        id,
        owner: 1,
        kind: BuildingKind::House,
        tile: Tile::new(25, 20),
        progress: 60,
        complete: true,
        rally: None,
        health: 400,
        training: None,
    });
    w.update_visibility();
    let units = w.units.iter().map(|u| u.id).collect();
    cmd(&mut w, 0, Command::Attack { units, target: id });
    run(&mut w, 800);
    assert!(!w.buildings.iter().any(|b| b.id == id));
    for (i, u) in w.units.iter().enumerate() {
        for other in &w.units[i + 1..] {
            assert_ne!(u.tile(), other.tile());
        }
    }
}
#[test]
fn hold_does_not_chase_and_move_overrides_attack() {
    let mut w = arena(UnitKind::Guard, 3);
    let start = w.units[0].tile();
    let id = w.units[0].id;
    cmd(&mut w, 0, Command::Hold { units: vec![id] });
    run(&mut w, 50);
    assert_eq!(w.units[0].tile(), start);
    attack(&mut w);
    w.step();
    cmd(
        &mut w,
        0,
        Command::MoveUnits {
            units: vec![id],
            target: Tile::new(19, 19),
        },
    );
    run(&mut w, 12);
    assert_eq!(w.units[0].tile(), Tile::new(19, 19));
}
#[test]
fn destroyed_target_clears_chase() {
    let mut w = arena(UnitKind::Guard, 5);
    attack(&mut w);
    w.step();
    w.units[1].health = 0;
    w.step();
    assert_eq!(w.units[0].task, Task::Idle);
    assert!(w.units[0].path.len() <= 1);
}
#[test]
fn visibility_radius_and_exploration_persist() {
    let mut w = arena(UnitKind::Worker, 7);
    assert!(w.visible(0, Tile::new(27, 20)));
    assert!(!w.visible(0, Tile::new(28, 20)));
    w.units[0].x = 4 * SUB + SUB / 2;
    w.update_visibility();
    assert!(!w.visible(0, Tile::new(27, 20)));
    assert!(w.explored(0, Tile::new(27, 20)));
}
#[test]
fn hidden_target_rejected_and_visibility_is_owner_specific() {
    let mut w = arena(UnitKind::Archer, 9);
    let enemy = w.units[1].tile();
    assert!(!w.visible(0, enemy));
    assert!(w.visible(1, enemy));
    attack(&mut w);
    w.step();
    assert_eq!(w.rejected, 1);
    assert_eq!(w.units[0].task, Task::Idle);
}
#[test]
fn target_leaving_sight_retires_order() {
    let mut w = arena(UnitKind::Archer, 6);
    attack(&mut w);
    w.step();
    w.units[1].x = 50 * SUB + SUB / 2;
    w.update_visibility();
    w.step();
    assert_eq!(w.units[0].task, Task::Idle);
}
#[test]
fn training_cost_duration_and_capacity_reservation() {
    let mut w = World::new(Civilization::Persia);
    let wood = w.resources.wood;
    cmd(
        &mut w,
        0,
        Command::Train {
            building: 1,
            kind: UnitKind::Guard,
        },
    );
    w.step();
    assert_eq!(w.resources.wood, wood - 35);
    assert_eq!(w.population(0), 5);
    cmd(
        &mut w,
        0,
        Command::Train {
            building: 1,
            kind: UnitKind::Archer,
        },
    );
    w.step();
    assert_eq!(w.rejected, 1);
    run(&mut w, 78);
    assert_eq!(w.units.len(), 5);
    assert!(w.buildings[0].training.is_none());
    cmd(
        &mut w,
        0,
        Command::Train {
            building: 1,
            kind: UnitKind::Archer,
        },
    );
    w.step();
    assert_eq!(w.rejected, 2);
}
#[test]
fn enemy_accounting_isolated_and_population_obeyed() {
    let mut w = World::new_match(Civilization::Persia);
    let player = w.resources;
    run(&mut w, 1200);
    assert_eq!(w.resources, player);
    assert!(w
        .units
        .iter()
        .any(|u| u.owner == 1 && u.kind != UnitKind::Worker));
    assert!(w.population(1) <= w.capacity(1));
    let spent: u32 = w
        .log
        .iter()
        .filter(|e| e.player == 1)
        .filter_map(|e| match e.command {
            Command::Train { kind, .. } => Some(kind.cost()),
            _ => None,
        })
        .sum();
    assert!(spent >= 35);
    let remaining: u32 = w.map.remaining.iter().sum();
    let original: u32 = World::new_match(Civilization::Persia)
        .map
        .remaining
        .iter()
        .sum();
    assert!(remaining < original);
    assert!(w.enemy_resources.wood > 0);
}
#[test]
fn ai_peaceful_period_and_attack_preparation() {
    let mut w = World::new_match(Civilization::Persia);
    run(&mut w, 3600);
    assert!(!w
        .log
        .iter()
        .any(|e| e.player == 1 && matches!(e.command, Command::AttackMove { .. })));
    assert!(
        w.units
            .iter()
            .filter(|u| u.owner == 1 && u.kind != UnitKind::Worker)
            .count()
            >= 3
    );
    run(&mut w, 101);
    assert!(w
        .log
        .iter()
        .any(|e| e.player == 1 && matches!(e.command, Command::AttackMove { .. })));
}
#[test]
fn ai_defends_only_visible_threat() {
    let mut w = World::new_match(Civilization::Persia);
    w.spawn_unit(1, UnitKind::Guard, Tile::new(35, 36));
    w.spawn_unit(0, UnitKind::Guard, Tile::new(34, 36));
    w.update_visibility();
    w.step();
    assert!(w
        .log
        .iter()
        .any(|e| e.player == 1 && matches!(e.command, Command::Attack { .. })));
}
#[test]
fn hidden_player_changes_do_not_change_ai_decisions() {
    let mut a = World::new_match(Civilization::Persia);
    let mut b = a.clone();
    b.units[0].x = 3 * SUB + SUB / 2;
    b.units[0].y = 3 * SUB + SUB / 2;
    b.update_visibility();
    run(&mut a, 500);
    run(&mut b, 500);
    let ai =
        |w: &World| -> Vec<Envelope> { w.log.iter().filter(|e| e.player == 1).cloned().collect() };
    assert_eq!(ai(&a), ai(&b));
    assert_eq!(a.enemy_resources, b.enemy_resources);
}
#[test]
fn victory_and_terminal_freeze() {
    let mut w = World::new_match(Civilization::Persia);
    w.buildings
        .iter_mut()
        .find(|b| b.owner == 1)
        .unwrap()
        .health = 0;
    w.step();
    assert_eq!(w.outcome, Outcome::Victory);
    let state = w.state_bytes();
    run(&mut w, 100);
    assert_eq!(w.state_bytes(), state);
    assert!(w.events.iter().any(|e| e.cue == sound::Cue::Victory));
}
#[test]
fn defeat_uses_town_center_not_military() {
    let mut w = World::new_match(Civilization::Persia);
    w.units.retain(|u| u.owner == 1);
    w.step();
    assert_eq!(w.outcome, Outcome::Active);
    w.buildings[0].health = 0;
    w.step();
    assert_eq!(w.outcome, Outcome::Defeat);
}
#[test]
fn restart_clears_orders_ai_and_effects() {
    let mut w = World::new_match(Civilization::Persia);
    run(&mut w, 500);
    w = World::new_match(Civilization::Persia);
    assert_eq!(w, World::new_match(Civilization::Persia));
    assert!(w.events.is_empty());
    assert!(w.effects.is_empty());
}
#[test]
fn match_replay_with_ai_and_future_commands() {
    let mut w = World::new_match(Civilization::Persia);
    cmd(
        &mut w,
        0,
        Command::Train {
            building: 1,
            kind: UnitKind::Archer,
        },
    );
    run(&mut w, 4100);
    let restored = replay::load(&replay::save(&w)).unwrap();
    assert_eq!(w.checksum(), restored.checksum());
    assert_eq!(w, restored);
}
#[test]
fn sound_events_and_throttled_original_pcm() {
    let mut w = arena(UnitKind::Guard, 1);
    attack(&mut w);
    w.step();
    assert!(w.events.iter().any(|e| e.cue == sound::Cue::Melee));
    let mut a = sound::Mixer::default();
    let mut b = sound::Mixer::default();
    a.cue(sound::Cue::Melee);
    for _ in 0..100 {
        b.cue(sound::Cue::Melee);
    }
    let mut x = [0u8; 4096];
    let mut y = x;
    a.render(&mut x);
    b.render(&mut y);
    assert_eq!(x, y);
    assert!(x.iter().any(|v| *v != 0));
    b.settings.mute = true;
    b.render(&mut y);
    assert!(y.iter().all(|v| *v == 0));
    b.clear();
    assert!(!b.active());
}
#[test]
fn volume_zero_and_voice_bound() {
    let mut mixer = sound::Mixer::default();
    mixer.settings.master = 0;
    for cue in [
        sound::Cue::Selection,
        sound::Cue::Movement,
        sound::Cue::Started,
        sound::Cue::Completed,
        sound::Cue::Trained,
        sound::Cue::Invalid,
        sound::Cue::Melee,
        sound::Cue::Arrow,
        sound::Cue::Death,
    ] {
        mixer.cue(cue);
    }
    let mut bytes = [1u8; 4096];
    mixer.render(&mut bytes);
    assert!(bytes.iter().all(|v| *v == 0));
    for _ in 0..30 {
        mixer.render(&mut bytes);
    }
    assert!(!mixer.active());
}
#[test]
fn pause_keeps_commands_until_resume() {
    use timing::{Clock, Speed};
    let mut w = World::new(Civilization::Persia);
    let mut clock = Clock::new(0);
    clock.set_speed(0, Speed::Paused);
    cmd(
        &mut w,
        0,
        Command::Train {
            building: 1,
            kind: UnitKind::Guard,
        },
    );
    let before = w.state_bytes();
    for now in [100, 500, 1000] {
        for _ in 0..clock.advance(now) {
            w.step();
        }
    }
    assert_eq!(w.state_bytes(), before);
    clock.set_speed(1000, Speed::Double);
    for _ in 0..clock.advance(1025) {
        w.step();
    }
    assert_eq!(w.tick, 1);
    assert_eq!(w.accepted, 1);
}
#[test]
fn fixed_ticks_at_all_speeds_and_no_skipped_debt() {
    use timing::{Clock, Speed};
    for (speed, expected) in [(Speed::Half, 10), (Speed::Normal, 20), (Speed::Double, 40)] {
        let mut c = Clock::new(0);
        c.set_speed(0, speed);
        let mut ticks = 0;
        for now in (0..=1000).step_by(10) {
            ticks += c.advance(now);
        }
        assert_eq!(ticks, expected);
    }
    let mut c = Clock::new(0);
    let mut total = c.advance(10000);
    assert_eq!(total, 5);
    for _ in 0..50 {
        total += c.advance(10000);
    }
    assert_eq!(total, 200);
}
#[test]
fn replay_independent_of_render_schedule() {
    fn play(frame: u64) -> World {
        let mut w = World::new_match(Civilization::Persia);
        let mut clock = timing::Clock::new(0);
        cmd(
            &mut w,
            0,
            Command::Train {
                building: 1,
                kind: UnitKind::Guard,
            },
        );
        for now in (0..=20000).step_by(frame as usize) {
            for _ in 0..clock.advance(now) {
                w.step();
            }
        }
        for _ in 0..clock.advance(20000) {
            w.step();
        }
        w
    }
    assert_eq!(play(10).checksum(), play(100).checksum());
}
#[test]
fn m11_first_unreachable_worker_does_not_reject_group() {
    let mut w = World::new(Civilization::Persia);
    w.units[0].x = 1 * SUB + SUB / 2;
    w.units[0].y = 1 * SUB + SUB / 2;
    for t in Tile::new(1, 1).neighbors() {
        w.map.terrain[t.index().unwrap()] = Terrain::Water;
    }
    cmd(
        &mut w,
        0,
        Command::BuildGroup {
            units: vec![2, 3, 4],
            kind: BuildingKind::House,
            tile: Tile::new(11, 10),
        },
    );
    run(&mut w, 400);
    assert_eq!(w.accepted, 1);
    assert!(w.buildings[1].complete);
    assert_eq!(w.resources.wood, 200);
}
#[test]
fn construction_death_never_refunds_and_clears_workers() {
    let mut w = World::new(Civilization::Persia);
    cmd(
        &mut w,
        0,
        Command::BuildGroup {
            units: vec![2, 3, 4],
            kind: BuildingKind::House,
            tile: Tile::new(11, 10),
        },
    );
    w.step();
    let id = w.buildings[1].id;
    w.buildings[1].health = 0;
    w.step();
    let balance = w.resources;
    cmd(&mut w, 0, Command::CancelConstruction { building: id });
    w.step();
    assert_eq!(w.resources, balance);
    assert!(!w.units.iter().any(|u| u.task == Task::Build(id)));
}
#[test]
fn unattended_beginner_match_reaches_defeat() {
    let mut w = World::new_match(Civilization::Persia);
    run(&mut w, 20000);
    assert_eq!(w.outcome, Outcome::Defeat);
}
#[test]
fn economic_player_can_win_through_commands() {
    let mut w = World::new_match(Civilization::Persia);
    cmd(
        &mut w,
        0,
        Command::SetRallyPoint {
            building: 1,
            tile: Tile::new(9, 5),
        },
    );
    let mut attacking = false;
    for _ in 0..20000 {
        if w.outcome != Outcome::Active {
            break;
        }
        if w.tick % 100 == 0 {
            let houses = w
                .buildings
                .iter()
                .filter(|b| b.owner == 0 && b.kind == BuildingKind::House)
                .count();
            let workers: Vec<_> = w
                .units
                .iter()
                .filter(|u| u.owner == 0 && u.kind == UnitKind::Worker && u.task == Task::Idle)
                .map(|u| u.id)
                .collect();
            for (j, id) in workers.into_iter().enumerate() {
                if houses < 3 && j == 0 && w.resources.wood >= 40 {
                    cmd(
                        &mut w,
                        0,
                        Command::BuildGroup {
                            units: vec![id],
                            kind: BuildingKind::House,
                            tile: [Tile::new(11, 8), Tile::new(11, 10), Tile::new(9, 11)][houses],
                        },
                    );
                } else {
                    let node = (0..CELLS)
                        .filter(|n| w.map.remaining[*n] > 0 && w.map.terrain[*n] == Terrain::Forest)
                        .map(|n| Tile::new(n as i32 % MAP_SIZE, n as i32 / MAP_SIZE))
                        .filter(|t| w.visible(0, *t))
                        .min_by_key(|t| (Tile::new(8, 8).distance(*t), *t));
                    if let Some(node) = node {
                        cmd(&mut w, 0, Command::AssignWorker { worker: id, node });
                    }
                }
            }
            let military: Vec<_> = w
                .units
                .iter()
                .filter(|u| u.owner == 0 && u.kind != UnitKind::Worker)
                .map(|u| u.id)
                .collect();
            if w.population(0) < w.capacity(0)
                && w.resources.wood >= 45
                && w.buildings[0].training.is_none()
            {
                cmd(
                    &mut w,
                    0,
                    Command::Train {
                        building: 1,
                        kind: UnitKind::Archer,
                    },
                );
            }
            if military.len() >= 10 || attacking {
                if !military.is_empty() {
                    cmd(
                        &mut w,
                        0,
                        Command::AttackMove {
                            units: military,
                            target: Tile::new(36, 36),
                        },
                    );
                    attacking = true;
                }
            }
        }
        w.step();
    }
    extern crate std;
    std::eprintln!(
        "M2 economic victory tick={} checksum={:016x} commands={}",
        w.tick,
        w.checksum(),
        w.log.len()
    );
    assert_eq!(
        w.outcome,
        Outcome::Victory,
        "tick {} population {} wood {}",
        w.tick,
        w.population(0),
        w.resources.wood
    );
    assert_eq!(
        w.checksum(),
        replay::load(&replay::save(&w)).unwrap().checksum()
    );
}
#[test]
fn ai_wood_conservation_includes_cargo_buildings_and_training() {
    let mut w = World::new_match(Civilization::Persia);
    let initial: u32 = w.map.remaining.iter().sum();
    run(&mut w, 1800);
    let harvested = initial - w.map.remaining.iter().sum::<u32>();
    let cargo: u32 = w
        .units
        .iter()
        .filter(|u| u.owner == 1)
        .map(|u| u.cargo)
        .sum();
    let buildings: u32 = w
        .buildings
        .iter()
        .filter(|b| b.owner == 1 && b.kind != BuildingKind::TownCenter)
        .map(|b| b.kind.cost())
        .sum();
    let trained: u32 = w
        .units
        .iter()
        .filter(|u| u.owner == 1 && u.kind != UnitKind::Worker)
        .map(|u| u.kind.cost())
        .sum();
    let reserved: u32 = w
        .buildings
        .iter()
        .filter(|b| b.owner == 1)
        .filter_map(|b| b.training.map(|(kind, _)| kind.cost()))
        .sum();
    assert_eq!(
        180 + harvested,
        w.enemy_resources.wood + cargo + buildings + trained + reserved
    );
}
#[test]
fn combat_workload_scales_with_bounded_effects_and_equal_checksums() {
    extern crate std;
    for count in [8, 32, 64, 128] {
        let mut a = World::new(Civilization::Persia);
        a.units.clear();
        a.buildings.clear();
        a.map.terrain.fill(Terrain::Grass);
        for n in 0..count {
            a.spawn_unit(
                (n % 2) as u8,
                if n % 3 == 0 {
                    UnitKind::Archer
                } else {
                    UnitKind::Guard
                },
                Tile::new(10 + n % 16, 10 + n / 16),
            );
        }
        for u in &mut a.units {
            u.task = Task::Hold;
        }
        a.update_visibility();
        let mut b = a.clone();
        let start = std::time::Instant::now();
        for _ in 0..400 {
            a.step();
            b.step();
            assert!(a.effects.len() <= 128);
            assert!(a.events.len() <= 64);
        }
        assert_eq!(a.checksum(), b.checksum());
        std::eprintln!(
            "M2 workload initial_units={} ticks=400 paired_elapsed_us={} checksum={:016x}",
            count,
            start.elapsed().as_micros(),
            a.checksum()
        );
    }
}
#[test]
fn crossing_movement_intents_resolve_by_stable_id() {
    let mut w = World::new(Civilization::Persia);
    w.units.clear();
    w.buildings.clear();
    w.map.terrain.fill(Terrain::Grass);
    w.spawn_unit(0, UnitKind::Guard, Tile::new(10, 8));
    w.spawn_unit(0, UnitKind::Guard, Tile::new(8, 10));
    let (a, b) = (w.units[0].id, w.units[1].id);
    cmd(
        &mut w,
        0,
        Command::MoveUnits {
            units: vec![a],
            target: Tile::new(10, 12),
        },
    );
    cmd(
        &mut w,
        0,
        Command::MoveUnits {
            units: vec![b],
            target: Tile::new(12, 10),
        },
    );
    for _ in 0..120 {
        w.step();
        assert_ne!(w.units[0].tile(), w.units[1].tile());
    }
    assert_eq!(w.units[0].tile(), Tile::new(10, 12));
    assert_eq!(w.units[1].tile(), Tile::new(12, 10));
}
#[test]
fn trained_worker_obeys_rally_order() {
    let mut w = World::new(Civilization::Persia);
    cmd(
        &mut w,
        0,
        Command::SetRallyPoint {
            building: 1,
            tile: Tile::new(20, 14),
        },
    );
    cmd(
        &mut w,
        0,
        Command::Train {
            building: 1,
            kind: UnitKind::Worker,
        },
    );
    run(&mut w, 600);
    assert_eq!(w.units.last().unwrap().tile(), Tile::new(20, 14));
}
#[test]
fn attack_move_group_settles_without_rally_churn() {
    let mut w = World::new(Civilization::Persia);
    let ids = w
        .units
        .iter()
        .filter(|u| u.kind != UnitKind::Worker)
        .map(|u| u.id)
        .collect();
    cmd(
        &mut w,
        0,
        Command::AttackMove {
            units: ids,
            target: Tile::new(20, 14),
        },
    );
    run(&mut w, 300);
    let position = (w.units[3].x, w.units[3].y);
    run(&mut w, 100);
    assert_eq!(position, (w.units[3].x, w.units[3].y));
}
#[test]
fn depleted_resource_opens_terrain_and_preserves_delivery_kind() {
    let mut w = World::new(Civilization::Persia);
    let node = Tile::new(8, 15);
    w.map.remaining[node.index().unwrap()] = 10;
    cmd(&mut w, 0, Command::AssignWorker { worker: 2, node });
    run(&mut w, 500);
    assert!(w.passable(node));
    assert_eq!(w.resources.stone, 110);
}
