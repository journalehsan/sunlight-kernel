//! SFS-4 deterministic hosted lifecycle and independent relocation witnesses.
use std::{cell::RefCell, collections::BTreeSet, path::PathBuf, rc::Rc};
use sunlight_sunfs::{block::*, bootstrap::Bootstrap, codec::*, engine::*, geometry::*};
use sunlight_sunfs_check::{check, check_relocation, Bytes, RelocationManifest, Report, Status};
use sunlight_sunfs_lab::simulator::{Fault, Simulator};
const SEED: u64 = 0x53465334;
#[derive(Clone)]
struct Device(Rc<RefCell<Simulator>>);
impl BlockDevice for Device {
    fn info(&self) -> DeviceInfo {
        self.0.borrow().info()
    }
    fn read(&mut self, o: u64, b: &mut [u8]) -> IoResult<()> {
        self.0.borrow_mut().read(o, b)
    }
    fn write(&mut self, o: u64, b: &[u8]) -> IoResult<WriteCompletion> {
        self.0.borrow_mut().write(o, b)
    }
    fn flush(&mut self) -> IoResult<()> {
        self.0.borrow_mut().flush()
    }
}
fn load(b: &[u8]) -> Device {
    Device(Rc::new(RefCell::new(
        Simulator::from_durable(b.to_vec(), SEED).unwrap(),
    )))
}
fn bytes(d: &Device) -> Vec<u8> {
    d.0.borrow().durable().to_vec()
}
fn next(d: &Device) -> u64 {
    d.0.borrow().trace.last().map_or(1, |e| e.id + 1)
}
fn loss(d: &Device) {
    d.0.borrow_mut().power_loss();
    d.0.borrow_mut().reset().unwrap();
}
fn fresh(bps: u32) -> Device {
    let g = Geometry::calculate(Parameters::laboratory(16 * 1024 * 1024, bps)).unwrap();
    let mut s = Simulator::new(16 * 1024 * 1024, SEED).unwrap();
    Bootstrap::plan(g, [0x34; 16])
        .unwrap()
        .publish(&mut s)
        .unwrap();
    Device(Rc::new(RefCell::new(s)))
}
fn budget() -> CleanBudget {
    CleanBudget {
        segments: 4096,
        verification_reads: 196613,
        dependency_blocks: 4096,
    }
}
fn save(name: &str, b: &[u8]) {
    if let Some(p) = std::env::var_os("SUNFS_SFS4_EVIDENCE_DIR") {
        let p = PathBuf::from(p).join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b).unwrap();
    }
}
fn page(b: &[u8], n: u64) -> Page {
    Page::decode(&b[n as usize * PAGE..(n as usize + 1) * PAGE], n).unwrap()
}
fn put(b: &mut [u8], p: &Page) {
    b[p.block as usize * PAGE..(p.block as usize + 1) * PAGE].copy_from_slice(&p.encode().unwrap());
}
fn edit_inode(image: &mut [u8], edit: impl FnOnce(&mut Inode)) {
    let report = check(&mut Bytes(image));
    let mut checkpoint = page(image, selected(&report).slot);
    let Payload::Checkpoint(cp) = &mut checkpoint.payload else {
        panic!()
    };
    let mut root = page(image, cp.node_map.block);
    let Payload::Root { pages, .. } = &mut root.payload else {
        panic!()
    };
    let mut table = page(image, pages[0].page.block);
    let Payload::Table {
        records: Records::Map(maps),
        ..
    } = &mut table.payload
    else {
        panic!()
    };
    let m = maps.iter_mut().find(|m| m.kind == 1).unwrap();
    let mut inode = page(image, m.node.block);
    let Payload::Inode(n) = &mut inode.payload else {
        panic!()
    };
    edit(n);
    put(image, &inode);
    m.node = inode.reference().unwrap();
    put(image, &table);
    pages[0].page = table.reference().unwrap();
    put(image, &root);
    cp.node_map = root.reference().unwrap();
    put(image, &checkpoint);
}
fn union(r: &Report) -> BTreeSet<u64> {
    r.generations
        .iter()
        .flat_map(|g| g.blocks.iter().copied())
        .collect()
}
fn selected(r: &Report) -> &sunlight_sunfs_check::CheckedGeneration {
    r.generations
        .iter()
        .find(|g| Some(g.generation) == r.selected)
        .unwrap()
}
fn assert_recovery(d: &Device) -> Report {
    let b = bytes(d);
    let r = check(&mut Bytes(&b));
    assert!(
        matches!(r.status, Status::Clean | Status::DegradedRecoverable),
        "{r:#?}"
    );
    let fs = Filesystem::mount(d.clone()).unwrap();
    assert_eq!(Some(fs.generation()), r.selected);
    let Payload::Superblock(sb) = page(&b, 0).payload else {
        panic!()
    };
    let u = union(&r);
    let seg: BTreeSet<_> = u
        .iter()
        .map(|b| (b - sb.segment_start) / sb.segment_blocks as u64)
        .collect();
    let a = fs.accounting();
    assert_eq!(a.protected_blocks, u.len() as u64);
    assert_eq!(
        a.available_blocks,
        (sb.segment_count - seg.len() as u64) * sb.segment_blocks as u64
    );
    assert_eq!(
        a.retained_only_blocks,
        u.len() as u64 - selected(&r).blocks.len() as u64
    );
    assert_eq!(a.pending_reserved_blocks, 0);
    for n in &selected(&r).nodes {
        if n.kind == 1 {
            // All data in these fixtures is a nonzero constant tied to NID.
            for e in &n.extents {
                assert!(
                    b[e.physical as usize * PAGE..(e.physical as usize + 1) * PAGE]
                        .iter()
                        .all(|v| *v == n.id.nid as u8)
                );
            }
        }
    }
    r
}
fn shrink_fixture(bps: u32) -> Device {
    let d = fresh(bps);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let root = fs.root();
    for i in 0..8 {
        let id = fs.create(format!("f{i}").as_bytes()).unwrap();
        fs.write(&id, 0, &vec![id.node.nid as u8; 16 * PAGE])
            .unwrap();
    }
    fs.sync_all().unwrap();
    for i in 1..8 {
        fs.unlink(&root, format!("f{i}").as_bytes()).unwrap();
    }
    fs.sync_all().unwrap();
    assert_eq!(fs.reap_orphans().unwrap(), 7);
    assert_recovery(&d);
    d
}
fn manifest(before: &[u8], after: &[u8], r: &CleanResult, pins: Vec<NodeId>) -> RelocationManifest {
    let a = check(&mut Bytes(before));
    let b = check(&mut Bytes(after));
    assert_eq!(b.selected, a.selected.map(|g| g + 1));
    RelocationManifest {
        reserved: selected(&b).blocks.clone(),
        runtime_pins: pins,
        quarantined: vec![],
        recovery_floor: r.recovery_floor,
    }
}
fn record(tag: &str, before: &[u8], after: &[u8], r: &CleanResult, b: CleanBudget) {
    let a = check(&mut Bytes(before));
    let z = check(&mut Bytes(after));
    let Payload::Superblock(sb) = page(before, 0).payload else {
        panic!()
    };
    let segs = |r: &Report| {
        union(r)
            .iter()
            .map(|v| (v - sb.segment_start) / sb.segment_blocks as u64)
            .collect::<BTreeSet<_>>()
            .len() as u64
    };
    let initial = (sb.segment_count - segs(&a)) * sb.segment_blocks as u64;
    let final_free = (sb.segment_count - segs(&z)) * sb.segment_blocks as u64;
    let row=format!("{{\"fixture\":\"{tag}\",\"capacity_blocks\":{},\"initial_protected\":{},\"candidate_segments\":{},\"reserved_blocks\":{},\"relocated_blocks\":{},\"released_blocks\":{},\"initial_reusable\":{initial},\"final_reusable\":{final_free},\"net_reusable\":{},\"bytes_written\":{},\"flushes\":{},\"reserve_used\":{},\"recovery_floor\":{},\"scan_budget\":{},\"dependency_budget\":{},\"segments_scanned\":{},\"stop\":\"{:?}\",\"selected\":{},\"checker\":\"{}\",\"status\":\"PASS\"}}\n",sb.segment_count*sb.segment_blocks as u64,union(&a).len(),r.candidates_considered,if r.checkpoints_committed>0 {selected(&z).blocks.len()} else {0},r.blocks_relocated,r.blocks_reclaimed,r.net_reusable_blocks,r.bytes_written,r.flushes,r.cleaner_reserve_used,r.recovery_floor,b.segments,b.dependency_blocks,r.segments_scanned,r.stop,z.selected.unwrap(),z.status.name());
    let row = row.trim_end().strip_suffix('}').unwrap().to_owned() + &format!(",\"data_blocks\":{},\"metadata_blocks\":{},\"verification_reads\":{},\"victim\":{},\"verification_budget\":{}}}\n", r.data_blocks_relocated, r.metadata_blocks_rewritten, r.verification_reads, r.victim.map_or("null".to_string(),|v|v.to_string()), b.verification_reads);
    save(
        &format!("fixtures/{tag}.ownership"),
        format!("before={a:#?}\nafter={z:#?}").as_bytes(),
    );
    save(&format!("fixtures/{tag}.jsonl"), row.as_bytes());
    save(
        &format!("fixtures/{tag}.trace"),
        format!("{:?}", r).as_bytes(),
    );
}

#[test]
fn f1_mixed_relocation_positive_capacity_and_eventual_source_reuse() {
    for bps in [64, 256, 512] {
        let d = shrink_fixture(bps);
        let before = bytes(&d);
        let mut fs = Filesystem::mount(d.clone()).unwrap();
        let states = fs.segment_liveness(4096).unwrap();
        assert!(states
            .iter()
            .any(|s| !s.current.is_empty() && s.current.len() < bps as usize));
        let source: BTreeSet<_> = states
            .iter()
            .flat_map(|s| s.current.iter().copied())
            .collect();
        let start = next(&d);
        let r = fs.clean_segments(budget(), CleanPolicy::Reclaim).unwrap();
        if bps == 64 {
            assert_eq!(r.stop, CleanStop::Reclaimed);
            assert!(r.net_reusable_blocks > 0);
        } else {
            // Large segments cannot recover capacity from this 8-file fixture;
            // this is an explicit negative geometry result, not positive progress.
            assert_eq!(r.stop, CleanStop::FallbackProtected);
            assert_eq!(next(&d), start);
            record(
                &format!("F1-geometry-{bps}"),
                &before,
                &bytes(&d),
                &r,
                budget(),
            );
            continue;
        }
        let after = bytes(&d);
        let m = manifest(&before, &after, &r, vec![]);
        let audit = check_relocation(&before, &after, &m).unwrap();
        assert_eq!(
            audit.final_reusable - audit.initial_reusable,
            r.net_reusable_blocks as u64
        );
        assert_eq!(audit.released_blocks, r.blocks_reclaimed);
        assert_eq!(audit.relocated_blocks, r.blocks_relocated);
        assert_eq!(audit.reserve_used, r.cleaner_reserve_used);
        let ops: Vec<_> =
            d.0.borrow()
                .trace
                .iter()
                .filter(|e| e.id >= start)
                .cloned()
                .collect();
        assert_eq!(
            ops.iter().filter(|e| e.operation == "flush").count() as u64,
            r.flushes
        );
        assert_eq!(
            ops.iter()
                .filter(|e| e.operation == "write")
                .map(|e| e.length as u64)
                .sum::<u64>(),
            r.bytes_written
        );
        assert!(union(&assert_recovery(&d)).is_superset(&source));
        record("F1-mixed-64", &before, &after, &r, budget());
        save("mixed-before.img", &before);
        save("mixed-after.img", &after);
        let relocated_source: BTreeSet<_> = selected(&check(&mut Bytes(&after)))
            .blocks
            .iter()
            .copied()
            .collect();
        let victim_segment = (*relocated_source.first().unwrap() - bps as u64) / bps as u64;
        let segment_start = bps as u64 + victim_segment * bps as u64;
        // Reuse leaves real obsolete nonzero data outside the current closure.
        assert!(
            (segment_start..segment_start + bps as u64).any(|b| !relocated_source.contains(&b)
                && after[b as usize * PAGE..(b as usize + 1) * PAGE]
                    .iter()
                    .any(|v| *v != 0))
        );
        let again = fs.clean_segments(budget(), CleanPolicy::Relocate).unwrap();
        assert_eq!(again.stop, CleanStop::DeferredRetirement);
        let second = bytes(&d);
        check_relocation(&after, &second, &manifest(&after, &second, &again, vec![])).unwrap();
        record(
            "F1-obsolete-tail-relocation",
            &after,
            &second,
            &again,
            budget(),
        );
        assert!(union(&assert_recovery(&d)).is_disjoint(&source));
        // Ordinary admission may now safely place a new snapshot in the source.
        let id = fs.lookup(b"f0").unwrap();
        fs.write(&id, 0, &[id.node.nid as u8]).unwrap();
        let plan = fs.admission().unwrap();
        assert!(plan.blocks.iter().any(|b| source.contains(b)));
        fs.sync_all().unwrap();
        let final_report = assert_recovery(&d);
        assert!(union(&final_report).is_disjoint(&relocated_source));
    }
}
#[test]
fn f2_dead_and_f3_fully_protected_segments_are_not_rewritten() {
    let d = fresh(64);
    let before = bytes(&d);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let states = fs.segment_liveness(4096).unwrap();
    assert!(states.iter().any(|s| s.state == SegmentState::Free));
    assert!(states
        .iter()
        .any(|s| s.current == s.fallback && !s.current.is_empty()));
    let start = next(&d);
    let r = fs.clean_segments(budget(), CleanPolicy::Reclaim).unwrap();
    assert_eq!(r.stop, CleanStop::FallbackProtected);
    assert_eq!(next(&d), start);
    assert_eq!(before, bytes(&d));
    record(
        "F2-dead-already-reusable",
        &before,
        &bytes(&d),
        &r,
        budget(),
    );
    record("F3-protected", &before, &bytes(&d), &r, budget());
    save(
        "liveness-bootstrap.trace",
        format!("{states:#?}").as_bytes(),
    );
}
#[test]
fn f4_fallback_releases_only_after_publication_and_retirement() {
    let d = shrink_fixture(64);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let before = bytes(&d);
    let initial = assert_recovery(&d);
    let r = fs.clean_segments(budget(), CleanPolicy::Reclaim).unwrap();
    assert_eq!(r.stop, CleanStop::Reclaimed);
    let after = bytes(&d);
    check_relocation(&before, &after, &manifest(&before, &after, &r, vec![])).unwrap();
    for b in union(&initial) {
        assert_eq!(
            &before[b as usize * PAGE..(b as usize + 1) * PAGE],
            &after[b as usize * PAGE..(b as usize + 1) * PAGE]
        );
    }
    record("F4-fallback", &before, &after, &r, budget());
}
#[test]
fn f5_runtime_orphan_pins_offsets_close_and_distinct_reaping() {
    let d = shrink_fixture(64);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let id = fs.lookup(b"f0").unwrap();
    let h = fs.open(&id, AccessMode::ReadWrite).unwrap();
    let dup = fs.dup(&h).unwrap();
    fs.seek(&h, 7).unwrap();
    fs.unlink(&fs.root(), b"f0").unwrap();
    fs.sync_all().unwrap();
    let before = bytes(&d);
    let start = next(&d);
    let refuse = fs.clean_segments(budget(), CleanPolicy::Reclaim).unwrap();
    assert_eq!(refuse.stop, CleanStop::RuntimePinned);
    assert_eq!(next(&d), start);
    let r = fs.clean_segments(budget(), CleanPolicy::Relocate).unwrap();
    assert_eq!(r.stop, CleanStop::DeferredRetirement);
    assert_eq!(fs.position(&dup).unwrap(), 7);
    let after = bytes(&d);
    check_relocation(
        &before,
        &after,
        &manifest(&before, &after, &r, vec![id.node.clone()]),
    )
    .unwrap();
    let mut out = [0; 17];
    assert_eq!(fs.read_handle(&h, &mut out).unwrap(), 17);
    assert_eq!(out, [id.node.nid as u8; 17]);
    assert_eq!(fs.reap_orphans().unwrap(), 0);
    let start = next(&d);
    fs.close(&h).unwrap();
    fs.close(&dup).unwrap();
    assert_eq!(next(&d), start);
    assert_eq!(fs.runtime_accounting().reap_candidates, 1);
    let moved = fs.clean_segments(budget(), CleanPolicy::Relocate).unwrap();
    assert_eq!(moved.checkpoints_committed, 1);
    assert_eq!(fs.runtime_accounting().orphan_nodes, 1);
    assert_eq!(fs.reap_orphans().unwrap(), 1);
    assert_eq!(fs.reap_orphans().unwrap(), 0);
    let check = assert_recovery(&d);
    assert!(selected(&check).orphans.is_empty());
    assert!(check.generations.iter().any(|g| !g.orphans.is_empty()));
    record("F5-pinned-orphan", &before, &after, &r, budget());
}
#[test]
fn f6_near_full_uses_cleaner_reserve_but_keeps_recovery_floor() {
    let d = shrink_fixture(64);
    let mut before = bytes(&d);
    let initial = assert_recovery(&d);
    let Payload::Superblock(sb) = page(&before, 0).payload else {
        panic!()
    };
    let segments: BTreeSet<_> = union(&initial)
        .iter()
        .map(|b| (b - sb.segment_start) / 64)
        .collect();
    for n in [0, 1] {
        let mut p = page(&before, n);
        let Payload::Superblock(s) = &mut p.payload else {
            panic!()
        };
        s.cleaner_reserve = (s.segment_count - segments.len() as u64) * 64;
        put(&mut before, &p);
    }
    let d = load(&before);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    assert!(matches!(fs.admission(), Err(FsError::NoSpace)));
    let r = fs.clean_segments(budget(), CleanPolicy::Reclaim).unwrap();
    assert_eq!(r.stop, CleanStop::Reclaimed);
    assert!(r.cleaner_reserve_used > 0);
    assert!(r.remaining_free_segments * 64 >= r.recovery_floor);
    let after = bytes(&d);
    check_relocation(&before, &after, &manifest(&before, &after, &r, vec![])).unwrap();
    record("F6-reserve", &before, &after, &r, budget());
    // Cleaning restores ordinary admission and permits bounded real overwrites.
    let id = fs.lookup(b"f0").unwrap();
    for _ in 0..24 {
        fs.write(&id, 0, &[id.node.nid as u8]).unwrap();
        fs.sync_all().unwrap();
    }
    assert_recovery(&d);
    save("F8-restored-admission.img", &bytes(&d));
}
#[test]
fn f7_no_progress_budget_dirty_and_repeated_refusal_are_bounded() {
    let d = shrink_fixture(256);
    let before = bytes(&d);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    for _ in 0..8 {
        let start = next(&d);
        let r = fs.clean_segments(budget(), CleanPolicy::Reclaim).unwrap();
        assert_eq!(r.stop, CleanStop::FallbackProtected);
        assert_eq!(next(&d), start);
        assert!(r.segments_scanned <= budget().segments);
        record("F7-no-progress", &before, &bytes(&d), &r, budget());
    }
    for b in [
        CleanBudget {
            segments: 0,
            ..budget()
        },
        CleanBudget {
            dependency_blocks: 0,
            ..budget()
        },
        CleanBudget {
            verification_reads: 0,
            ..budget()
        },
    ] {
        let r = fs.clean_segments(b, CleanPolicy::Relocate).unwrap();
        assert_eq!(r.stop, CleanStop::Budget);
    }
    let id = fs.lookup(b"f0").unwrap();
    fs.write(&id, 0, &[2]).unwrap();
    let r = fs.clean_segments(budget(), CleanPolicy::Relocate).unwrap();
    assert_eq!(r.stop, CleanStop::Dirty);
    assert_eq!(before, bytes(&d));
}
#[test]
fn f8_overwrite_and_f9_namespace_churn_reuse_without_ownership_loss() {
    let d = shrink_fixture(64);
    let initial = bytes(&d);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let r = fs.clean_segments(budget(), CleanPolicy::Reclaim).unwrap();
    assert!(r.net_reusable_blocks > 0);
    let after = bytes(&d);
    record("F8-overwrite-start", &initial, &after, &r, budget());
    for i in 0..24 {
        let id = fs.lookup(b"f0").unwrap();
        fs.write(&id, 0, &vec![2; 16 * PAGE]).unwrap();
        fs.sync_all().unwrap();
        let tmp = fs.create(b"tmp").unwrap();
        fs.write(&tmp, 0, &[tmp.node.nid as u8; PAGE]).unwrap();
        fs.rename(&fs.root(), b"tmp", &fs.root(), b"renamed")
            .unwrap();
        fs.sync_all().unwrap();
        fs.unlink(&fs.root(), b"renamed").unwrap();
        fs.sync_all().unwrap();
        assert_eq!(fs.reap_orphans().unwrap(), 1);
        if i % 4 == 0 {
            assert_recovery(&d);
        }
        let r = fs.clean_segments(budget(), CleanPolicy::Reclaim).unwrap();
        assert_eq!(r.stop, CleanStop::FallbackProtected);
        assert!(fs.accounting().available_blocks >= fs.accounting().recovery_reserve_blocks);
    }
    assert_recovery(&d);
    save("F9-churn-final.img", &bytes(&d));
}
#[test]
fn f12_reserve_and_plan_exhaustion_reject_before_io() {
    // Round complete plan cost up to segments; the minimum tranche need not fit
    // the largest supported populated full snapshot. Rejection is explicit.
    let original = fresh(64);
    let mut image = bytes(&original);
    for n in [0, 1] {
        let mut p = page(&image, n);
        let Payload::Superblock(sb) = &mut p.payload else {
            panic!()
        };
        sb.transaction_bound += 64;
        put(&mut image, &p);
    }
    let d = load(&image);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let directory = fs.mkdir(&fs.root(), b"d").unwrap();
    for i in 1..16 {
        fs.mkdir(&fs.root(), format!("d{i}").as_bytes()).unwrap();
    }
    for i in 0..16 {
        let id = fs
            .create_at(&directory, format!("f{i}").as_bytes())
            .unwrap();
        fs.write(&id, 0, &vec![id.node.nid as u8; 16 * PAGE])
            .unwrap();
    }
    fs.sync_all().unwrap();
    let before = bytes(&d);
    let start = next(&d);
    let r = fs.clean_segments(budget(), CleanPolicy::Relocate).unwrap();
    assert_eq!(r.stop, CleanStop::InsufficientReserve);
    assert_eq!(next(&d), start);
    assert_eq!(r.checkpoints_committed, 0);
    assert_eq!(before, bytes(&d));
    assert_recovery(&d);
    record("F12-large-plan", &before, &bytes(&d), &r, budget());
}

fn fault_list(op: &str) -> Vec<(String, Option<Fault>, bool)> {
    let mut v = vec![
        ("power-before".into(), Some(Fault::PowerLoss), false),
        ("power-after".into(), None, true),
    ];
    match op {
        "read" => v.push(("read-fail".into(), Some(Fault::ReadFailure), false)),
        "write" => {
            v.push(("write-fail".into(), Some(Fault::WriteFailure), false));
            for n in [0, 1, 511, 512, 2048, 4095, 4096] {
                v.push((format!("partial-{n}"), Some(Fault::Partial(n)), false));
            }
            for (start, len) in [(1, 511), (512, 512), (2048, 2048)] {
                v.push((
                    format!("torn-{start}-{len}"),
                    Some(Fault::Torn { start, len }),
                    false,
                ));
            }
            for persist in [false, true] {
                v.push((
                    format!("uncertain-{persist}"),
                    Some(Fault::UncertainWrite { persist }),
                    false,
                ));
            }
        }
        "flush" => {
            for persist_writes in [0, 1, usize::MAX] {
                for uncertain in [false, true] {
                    v.push((
                        format!("flush-{persist_writes}-{uncertain}"),
                        Some(Fault::FlushFailure {
                            persist_writes,
                            uncertain,
                        }),
                        false,
                    ));
                }
            }
        }
        _ => panic!("unknown operation"),
    }
    v
}
fn campaign(tag: &str, base: &[u8]) {
    let dry = load(base);
    let mut fs = Filesystem::mount(dry.clone()).unwrap();
    let old = fs.generation();
    let initial = assert_recovery(&dry);
    let start = next(&dry);
    let completed = fs.clean_segments(budget(), CleanPolicy::Relocate).unwrap();
    assert_eq!(completed.checkpoints_committed, 1);
    let dry_after = bytes(&dry);
    check_relocation(
        base,
        &dry_after,
        &manifest(base, &dry_after, &completed, vec![]),
    )
    .unwrap();
    let events: Vec<_> = dry
        .0
        .borrow()
        .trace
        .iter()
        .filter(|e| e.id >= start)
        .cloned()
        .collect();
    let publication = events
        .iter()
        .position(|e| {
            e.operation == "write"
                && (2 * PAGE as u64..=4 * PAGE as u64).contains(&e.offset)
                && e.digest != crc64(&[0; PAGE])
        })
        .unwrap();
    let targets: Vec<_> = events
        .iter()
        .filter(|e| e.operation == "write" && e.offset >= 64 * PAGE as u64)
        .map(|e| e.offset / PAGE as u64)
        .collect();
    assert_eq!(
        targets.iter().copied().collect::<BTreeSet<_>>().len(),
        targets.len()
    );
    assert!(targets.iter().all(|b| !union(&initial).contains(b)));
    save(&format!("{tag}-before.img"), base);
    save(&format!("{tag}-committed.img"), &dry_after);
    let mut rows = String::new();
    for (i, event) in events.iter().enumerate() {
        for (label, fault, after) in fault_list(event.operation) {
            let case = format!("{tag}-seed-{SEED}-event-{i}-{label}");
            if std::env::var("SUNFS_SFS4_CASE").is_ok_and(|s| s != case) {
                continue;
            }
            let d = load(base);
            let mut fs = Filesystem::mount(d.clone()).unwrap();
            let request_start = next(&d);
            let target = request_start + i as u64;
            if let Some(f) = fault.clone() {
                d.0.borrow_mut().faults.insert(target, f);
            }
            if after {
                d.0.borrow_mut().crash_after = Some(target);
            }
            let result = fs.clean_segments(budget(), CleanPolicy::Relocate).unwrap();
            let io: Vec<_> =
                d.0.borrow()
                    .trace
                    .iter()
                    .filter(|e| e.id >= request_start)
                    .cloned()
                    .collect();
            let submitted = io
                .iter()
                .filter(|e| e.operation == "write")
                .map(|e| e.length as u64)
                .sum::<u64>();
            let confirmed = io
                .iter()
                .filter(|e| e.operation == "write")
                .map(|e| {
                    if e.outcome.starts_with("Ok") {
                        e.length as u64
                    } else {
                        e.outcome
                            .split("completed: Some(")
                            .nth(1)
                            .map_or(0, |s| s.split(')').next().unwrap().parse::<u64>().unwrap())
                    }
                })
                .sum::<u64>();
            assert_eq!(result.bytes_submitted, submitted, "{case}");
            assert_eq!(result.bytes_written, confirmed, "{case}");
            assert_eq!(
                result.flushes_attempted,
                io.iter().filter(|e| e.operation == "flush").count() as u64,
                "{case}"
            );
            assert_eq!(
                result.flushes,
                io.iter()
                    .filter(|e| e.operation == "flush" && e.outcome.starts_with("Ok"))
                    .count() as u64,
                "{case}"
            );
            let acknowledged = result.checkpoints_committed == 1;
            if result.error.is_some() {
                assert_eq!(fs.sync_all(), Err(FsError::ReadOnly));
                let states = fs.segment_liveness(4096).unwrap();
                assert!(states.iter().all(|s| s.state == SegmentState::Quarantined));
                let a = fs.accounting();
                assert_eq!(
                    a.pending_reserved_blocks,
                    if result.verification_reads != 0 {
                        targets.len() as u64
                    } else {
                        0
                    }
                );
                assert_eq!(result.blocks_relocated, 0);
            }
            let expected = if i < publication {
                old
            } else if i == publication {
                if matches!(fault, Some(Fault::UncertainWrite { persist: true })) {
                    old + 1
                } else {
                    old
                }
            } else if i == publication + 1 {
                if after
                    || matches!(
                        fault,
                        Some(Fault::FlushFailure {
                            persist_writes: 1..,
                            ..
                        })
                    )
                {
                    old + 1
                } else {
                    old
                }
            } else {
                old + 1
            };
            if acknowledged {
                assert_eq!(expected, old + 1);
            }
            drop(fs);
            loss(&d);
            let image = bytes(&d);
            let verified = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let r = assert_recovery(&d);
                assert_eq!(r.selected, Some(expected), "{case}");
                let source = if expected == old { base } else { &dry_after };
                let sr = check(&mut Bytes(source));
                let want = selected(&sr);
                let got = selected(&r);
                assert_eq!(got.nodes.len(), want.nodes.len());
                assert_eq!(got.orphans, want.orphans);
                for n in &want.nodes {
                    let z = got.nodes.iter().find(|z| z.id == n.id).unwrap();
                    assert_eq!(
                        (
                            n.kind,
                            n.size,
                            n.nlink,
                            n.parent.clone(),
                            n.entries.clone(),
                            n.content_generation
                        ),
                        (
                            z.kind,
                            z.size,
                            z.nlink,
                            z.parent.clone(),
                            z.entries.clone(),
                            z.content_generation
                        )
                    );
                    assert_eq!(n.extents, z.extents);
                }
                for b in union(&initial) {
                    assert_eq!(
                        &base[b as usize * PAGE..(b as usize + 1) * PAGE],
                        &image[b as usize * PAGE..(b as usize + 1) * PAGE],
                        "{case}"
                    );
                }
                for g in &r.generations {
                    assert_eq!(
                        g.blocks.len(),
                        g.blocks.iter().copied().collect::<BTreeSet<_>>().len()
                    );
                }
                let mut retry = Filesystem::mount(d.clone()).unwrap();
                if retry.state() == MountState::CleanWritable {
                    let attempt = retry
                        .clean_segments(budget(), CleanPolicy::Relocate)
                        .unwrap();
                    assert_eq!(attempt.checkpoints_committed, 1);
                    drop(retry);
                    loss(&d);
                    assert_recovery(&d);
                }
            }));
            save(
                &format!("traces/{case}.trace"),
                format!("{:?}", d.0.borrow().trace).as_bytes(),
            );
            if let Err(p) = verified {
                save(&format!("FAILED-{case}.img"), &image);
                std::panic::resume_unwind(p);
            }
            let r = check(&mut Bytes(&image));
            rows.push_str(&format!("{{\"case\":\"{case}\",\"geometry_bps\":64,\"initial_generation\":{old},\"event\":{target},\"fault\":\"{fault:?}\",\"operation\":\"{}\",\"expected\":[{expected}],\"selected\":{},\"acknowledged\":{acknowledged},\"initial_protected\":{},\"reserved_blocks\":{},\"recovery_floor\":{},\"checker\":\"{}\",\"bytes_namespace_ownership\":\"PASS\",\"retry\":\"PASS_OR_READONLY\",\"status\":\"PASS\"}}\n",event.operation,r.selected.unwrap(),union(&initial).len(),targets.len(),completed.recovery_floor,r.status.name()));
        }
    }
    save(&format!("crash-{tag}.jsonl"), rows.as_bytes());
    println!(
        "SFS4_CRASH {tag} cases={} events={}",
        rows.lines().count(),
        events.len()
    );
}
#[test]
fn f10_interrupted_relocation_and_f11_retry_crash_matrix() {
    let d = shrink_fixture(64);
    let base = bytes(&d);
    campaign("mixed", &base);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let start = next(&d);
    fs.clean_segments(budget(), CleanPolicy::Relocate).unwrap();
    let events: Vec<_> =
        d.0.borrow()
            .trace
            .iter()
            .filter(|e| e.id >= start)
            .cloned()
            .collect();
    let cp = events
        .iter()
        .position(|e| {
            e.operation == "write" && (2 * PAGE as u64..=4 * PAGE as u64).contains(&e.offset)
        })
        .unwrap();
    let triple = load(&base);
    let mut fs = Filesystem::mount(triple.clone()).unwrap();
    let cut = next(&triple) + (cp + 1) as u64;
    triple.0.borrow_mut().crash_after = Some(cut);
    let result = fs.clean_segments(budget(), CleanPolicy::Relocate).unwrap();
    assert_eq!(result.checkpoints_committed, 0);
    drop(fs);
    loss(&triple);
    assert_eq!(assert_recovery(&triple).generations.len(), 3);
    campaign("three", &bytes(&triple));
    let d = load(&base);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let h = fs
        .open(&fs.lookup(b"f0").unwrap(), AccessMode::Read)
        .unwrap();
    fs.unlink(&fs.root(), b"f0").unwrap();
    fs.sync_all().unwrap();
    fs.close(&h).unwrap();
    campaign("orphan", &bytes(&d));
}

#[test]
fn cleaner_checker_rejects_crc_resealed_semantic_and_manifest_defects() {
    let d = shrink_fixture(64);
    let before = bytes(&d);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let r = fs.clean_segments(budget(), CleanPolicy::Reclaim).unwrap();
    let after = bytes(&d);
    let m = manifest(&before, &after, &r, vec![]);
    check_relocation(&before, &after, &m).unwrap();
    let mut rows = String::new();
    for tag in [
        "duplicate-target",
        "unreserved-target",
        "quarantine",
        "runtime-pin",
        "floor",
        "orphan-loss",
        "generation",
        "source-overwrite",
        "node-size",
        "content-generation",
        "attributes",
        "nlink-semantics",
    ] {
        let mut image = after.clone();
        let mut manifest = m.clone();
        match tag {
            "duplicate-target" => manifest.reserved.push(manifest.reserved[0]),
            "unreserved-target" => {
                manifest.reserved.pop();
            }
            "quarantine" => manifest.quarantined.push(manifest.reserved[0]),
            "runtime-pin" => manifest.runtime_pins.push(NodeId {
                nid: 900,
                incarnation: 1,
            }),
            "floor" => manifest.recovery_floor = u64::MAX,
            "source-overwrite" => {
                let b = *union(&check(&mut Bytes(&before))).first().unwrap();
                image[b as usize * PAGE] ^= 1;
            }
            "orphan-loss" | "generation" => {
                let rr = check(&mut Bytes(&image));
                let mut p = page(&image, selected(&rr).slot);
                let Payload::Checkpoint(c) = &mut p.payload else {
                    panic!()
                };
                if tag == "orphan-loss" {
                    c.orphan_nodes += 1;
                } else {
                    c.generation += 1;
                    p.birth = c.generation;
                }
                put(&mut image, &p);
            }
            "node-size" | "content-generation" | "attributes" | "nlink-semantics" => {
                edit_inode(&mut image, |n| match tag {
                    "node-size" => n.size -= 1,
                    "content-generation" => n.content_generation += 1,
                    "attributes" => n.uid += 1,
                    _ => n.nlink = 0,
                });
            }
            _ => unreachable!(),
        }
        assert!(
            check_relocation(&before, &image, &manifest).is_err(),
            "{tag}"
        );
        // All individual changed metadata pages are CRC-resealed; a closure with
        // broken ancestor references must still be rejected independently.
        save(&format!("corrupt/{tag}.img"), &image);
        save(
            &format!("corrupt/{tag}.manifest"),
            format!("{manifest:#?}").as_bytes(),
        );
        rows.push_str(&format!(
            "{{\"case\":\"{tag}\",\"checker_rejected\":true,\"status\":\"PASS\"}}\n"
        ));
    }
    save("corruption.jsonl", rows.as_bytes());
}

#[test]
fn inode_attributes_survive_and_changed_source_freezes_before_writes() {
    let d = shrink_fixture(64);
    let mut before = bytes(&d);
    edit_inode(&mut before, |n| {
        n.uid = 42;
        n.gid = 43;
        n.mode = 0o600;
    });
    let d = load(&before);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let r = fs.clean_segments(budget(), CleanPolicy::Reclaim).unwrap();
    assert_eq!(r.stop, CleanStop::Reclaimed);
    let after = bytes(&d);
    check_relocation(&before, &after, &manifest(&before, &after, &r, vec![])).unwrap();
    let report = assert_recovery(&d);
    assert!(selected(&report)
        .nodes
        .iter()
        .any(|n| n.inode.uid == 42 && n.inode.gid == 43 && n.inode.mode == 0o600));
    // Model media damage after mount without changing the advertised incarnation.
    let d = load(&before);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let mut corrupt = before.clone();
    edit_inode(&mut corrupt, |n| n.nlink = 0);
    *d.0.borrow_mut() = Simulator::from_durable(corrupt.clone(), SEED).unwrap();
    let r = fs.clean_segments(budget(), CleanPolicy::Relocate).unwrap();
    assert_eq!(r.stop, CleanStop::Uncertain);
    assert_eq!(r.checkpoints_committed, 0);
    assert!(!d
        .0
        .borrow()
        .trace
        .iter()
        .any(|e| e.operation == "write" || e.operation == "flush"));
    assert_eq!(fs.sync_all(), Err(FsError::ReadOnly));
    assert!(fs
        .segment_liveness(4096)
        .unwrap()
        .iter()
        .all(|s| s.state == SegmentState::Quarantined));
    save("changed-source.img", &corrupt);
    save(
        "changed-source.trace",
        format!("{:?}", d.0.borrow().trace).as_bytes(),
    );
    let d = fresh(64);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let file = fs.create(b"small").unwrap();
    fs.write(&file, 0, &[2; PAGE]).unwrap();
    fs.sync_all().unwrap();
    let read_budget = fs.accounting().protected_blocks + 5;
    let larger = bytes(&shrink_fixture(64));
    *d.0.borrow_mut() = Simulator::from_durable(larger, SEED).unwrap();
    let r = fs
        .clean_segments(
            CleanBudget {
                verification_reads: read_budget,
                ..budget()
            },
            CleanPolicy::Relocate,
        )
        .unwrap();
    assert_eq!(r.stop, CleanStop::Budget);
    assert_eq!(r.checkpoints_committed, 0);
    assert_eq!(
        d.0.borrow()
            .trace
            .iter()
            .filter(|e| e.operation == "read")
            .count() as u64,
        read_budget
    );
    assert!(!d
        .0
        .borrow()
        .trace
        .iter()
        .any(|e| e.operation == "write" || e.operation == "flush"));
    assert_eq!(fs.sync_all(), Err(FsError::ReadOnly));
    save(
        "changed-source-budget.trace",
        format!("{:?}", d.0.borrow().trace).as_bytes(),
    );
}
