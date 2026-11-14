//! SFS-2 real engine transactions. Durable bytes and independent traversal are the
//! oracles; engine recovery is never used as its own sole correctness oracle.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    rc::Rc,
};
use sunlight_sunfs::{block::*, bootstrap::Bootstrap, codec::*, engine::*, geometry::*};
use sunlight_sunfs_check::{check, Bytes, Report, Status};
use sunlight_sunfs_lab::simulator::{Fault, Simulator};
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
fn fresh(bps: u32, mib: usize) -> Device {
    let capacity = mib * 1024 * 1024;
    let g = Geometry::calculate(Parameters::laboratory(capacity as u64, bps)).unwrap();
    let mut s = Simulator::new(capacity, 0x53465332).unwrap();
    Bootstrap::plan(g, [0x32; 16])
        .unwrap()
        .publish(&mut s)
        .unwrap();
    Device(Rc::new(RefCell::new(s)))
}
fn load(bytes: &[u8], seed: u64) -> Device {
    Device(Rc::new(RefCell::new(
        Simulator::from_durable(bytes.to_vec(), seed).unwrap(),
    )))
}
fn durable(d: &Device) -> Vec<u8> {
    d.0.borrow().durable().to_vec()
}
fn next(d: &Device) -> u64 {
    d.0.borrow().trace.last().map_or(1, |e| e.id + 1)
}
fn loss(d: &Device) {
    d.0.borrow_mut().power_loss();
    d.0.borrow_mut().reset().unwrap();
}
fn evidence() -> Option<PathBuf> {
    std::env::var_os("SUNFS_SFS2_EVIDENCE_DIR").map(PathBuf::from)
}
fn save(name: &str, bytes: &[u8]) {
    if let Some(p) = evidence() {
        std::fs::create_dir_all(&p).unwrap();
        let path = p.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}
fn inspected(bytes: &[u8], expected: &BTreeMap<Vec<u8>, Vec<u8>>) -> Report {
    let r = check(&mut Bytes(bytes));
    assert!(
        matches!(r.status, Status::Clean | Status::DegradedRecoverable),
        "{r:#?}"
    );
    let g = r.generations.last().unwrap();
    assert_eq!(Some(g.generation), r.selected);
    assert_eq!(g.root_entries.len(), expected.len());
    for binding in &g.root_entries {
        let data = expected
            .get(&binding.name)
            .expect("unexpected independent binding");
        let node = g
            .nodes
            .iter()
            .find(|n| n.id == binding.target)
            .expect("missing independent node");
        assert_eq!(node.size, data.len() as u64);
        let mut actual = vec![0; data.len()];
        for e in &node.extents {
            let a = e.logical as usize * PAGE;
            let n = PAGE.min(actual.len() - a);
            let p = e.physical as usize * PAGE;
            actual[a..a + n].copy_from_slice(&bytes[p..p + n]);
        }
        assert_eq!(&actual, data, "checker-derived data differs");
    }
    r
}
fn state(rows: &[(&[u8], &[u8])]) -> BTreeMap<Vec<u8>, Vec<u8>> {
    rows.iter().map(|(n, d)| (n.to_vec(), d.to_vec())).collect()
}
fn agreement(d: &Device, expected: &BTreeMap<Vec<u8>, Vec<u8>>) -> Filesystem<Device> {
    let bytes = durable(d);
    let r = inspected(&bytes, expected);
    let fs = Filesystem::mount(d.clone()).unwrap();
    assert_eq!(fs.generation(), r.selected.unwrap());
    assert_eq!(
        fs.state(),
        if r.status == Status::Clean {
            MountState::CleanWritable
        } else {
            MountState::DegradedReadOnly
        }
    );
    assert_eq!(fs.accounting().protected_blocks, r.union_blocks as u64);
    let union: BTreeSet<_> = r
        .generations
        .iter()
        .flat_map(|g| g.blocks.iter().copied())
        .collect();
    assert_eq!(union.len(), r.union_blocks);
    let sb = (0..2)
        .find_map(|b| {
            let a = b as usize * PAGE;
            match Page::decode(&bytes[a..a + PAGE], b) {
                Ok(Page {
                    payload: Payload::Superblock(sb),
                    ..
                }) => Some(sb),
                _ => None,
            }
        })
        .unwrap();
    let segments: BTreeSet<_> = union
        .iter()
        .map(|b| (b - sb.segment_start) / sb.segment_blocks as u64)
        .collect();
    let a = fs.accounting();
    let placed = segments.len() as u64 * sb.segment_blocks as u64;
    assert_eq!(a.protected_segments, segments.len() as u64);
    assert_eq!(a.placement_overhead_blocks, placed - union.len() as u64);
    assert_eq!(
        a.available_blocks,
        sb.segment_count * sb.segment_blocks as u64 - placed
    );
    assert_eq!(a.recovery_reserve_blocks, sb.cleaner_reserve);
    for (name, data) in expected {
        let id = fs.lookup(name).unwrap();
        let mut out = vec![0; data.len() + 16];
        assert_eq!(fs.read(&id, 0, &mut out).unwrap(), data.len());
        assert_eq!(&out[..data.len()], data);
    }
    assert_eq!(durable(d), bytes, "mount/checker wrote media");
    fs
}
#[test]
fn hello_power_loss_and_volatile_semantics() {
    for bps in [64, 256, 512] {
        let d = fresh(bps, 16);
        let mut fs = Filesystem::mount(d.clone()).unwrap();
        let id = fs.create(b"hello.txt").unwrap();
        fs.write(&id, 0, b"Hello SunlightOS\n").unwrap();
        let plan = fs.admission().unwrap();
        let start = next(&d);
        fs.fsync(&id).unwrap();
        let after = next(&d);
        assert!(after > start);
        assert_eq!(fs.generation(), 3);
        assert_eq!(
            fs.admission().unwrap().dependency_blocks,
            plan.dependency_blocks
        );
        let repeated = next(&d);
        fs.fsync(&id).unwrap();
        assert_eq!(next(&d), repeated);
        drop(fs);
        loss(&d);
        let expected = state(&[(b"hello.txt", b"Hello SunlightOS\n")]);
        let mut fs = agreement(&d, &expected);
        let reopened = fs.lookup(b"hello.txt").unwrap();
        assert_eq!((&reopened.volume, &reopened.node), (&id.volume, &id.node));
        assert_eq!(fs.write(&id, 0, b"stale"), Err(FsError::Stale));
        let id = reopened;
        fs.write(&id, 1, b"VOLATILE").unwrap();
        fs.create(b"lost.txt").unwrap();
        drop(fs);
        loss(&d);
        agreement(&d, &expected);
        if bps == 256 {
            save("hello.img", &durable(&d));
        }
    }
}
#[test]
fn writes_limits_identity_and_multiple_commits() {
    let d = fresh(256, 16);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let mut id = fs.create(b"a").unwrap();
    assert_eq!(fs.write(&id, 65536, b"").unwrap(), 0);
    assert!(fs.write(&id, u64::MAX, b"x").is_err());
    assert!(fs.write(&id, 65536, b"x").is_err());
    assert!(fs.create(&[b'x'; 65]).is_err());
    assert!(fs.create(b"../bad").is_err());
    let mut wrong = id.clone();
    wrong.node.incarnation += 1;
    assert_eq!(fs.write(&wrong, 0, b"x"), Err(FsError::Stale));
    wrong = id.clone();
    wrong.volume[0] ^= 1;
    assert_eq!(fs.read(&wrong, 0, &mut [0; 1]), Err(FsError::Stale));
    let mut expected = BTreeMap::new();
    let mut bytes = vec![0; PAGE + 9];
    bytes[PAGE - 3..PAGE + 9].copy_from_slice(b"across-block");
    fs.write(&id, (PAGE - 3) as u64, b"across-block").unwrap();
    let empty = fs.create(b"empty").unwrap();
    fs.fsync(&empty).unwrap();
    expected.insert(b"a".to_vec(), bytes.clone());
    expected.insert(b"empty".to_vec(), vec![]);
    drop(fs);
    loss(&d);
    let mut fs = agreement(&d, &expected);
    let stable = (id.volume, id.node.clone());
    id = fs.lookup(b"a").unwrap();
    for round in 0..8 {
        let offset = round * 97;
        let update = vec![round as u8 + 1; 4000];
        fs.write(&id, offset as u64, &update).unwrap();
        bytes.resize(bytes.len().max(offset + 4000), 0);
        bytes[offset..offset + 4000].copy_from_slice(&update);
        fs.fsync(&id).unwrap();
        expected.insert(b"a".to_vec(), bytes.clone());
        drop(fs);
        loss(&d);
        fs = agreement(&d, &expected);
        id = fs.lookup(b"a").unwrap();
        assert_eq!((id.volume, id.node.clone()), stable);
    }
    for i in 2..MAX_FILES {
        let name = format!("f{i:02}");
        let id = fs.create(name.as_bytes()).unwrap();
        fs.write(&id, 0, &vec![i as u8; MAX_FILE_BYTES]).unwrap();
    }
    assert_eq!(fs.create(b"over-limit"), Err(FsError::Limit));
    let p = fs.admission().unwrap();
    assert!(p.dependency_blocks > 200);
    fs.fsync(&id).unwrap();
    drop(fs);
    loss(&d);
    assert_eq!(check(&mut Bytes(&durable(&d))).status, Status::Clean);
}
#[test]
fn admission_low_space_is_readonly_and_exact() {
    let d = fresh(64, 5);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let id = fs.create(b"kept").unwrap();
    fs.write(&id, 0, b"committed").unwrap();
    fs.fsync(&id).unwrap();
    let old = durable(&d);
    let before = next(&d);
    for i in 1..MAX_FILES {
        let f = fs.create(format!("f{i:02}").as_bytes()).unwrap();
        fs.write(&f, 0, &vec![i as u8; MAX_FILE_BYTES]).unwrap();
    }
    assert_eq!(fs.admission().unwrap_err(), FsError::NoSpace);
    assert_eq!(fs.fsync(&id), Err(FsError::NoSpace));
    assert_eq!(next(&d), before);
    assert_eq!(durable(&d), old);
    drop(fs);
    loss(&d);
    agreement(&d, &state(&[(b"kept", b"committed")]));
    let d = fresh(256, 16);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let id = fs.create(b"one").unwrap();
    let p = fs.admission().unwrap();
    let before = next(&d);
    fs.fsync(&id).unwrap();
    let s = d.0.borrow();
    let writes: Vec<_> = s
        .trace
        .iter()
        .filter(|e| e.id >= before && e.operation == "write" && e.offset >= 256 * PAGE as u64)
        .map(|e| e.offset / PAGE as u64)
        .collect();
    assert_eq!(writes.len(), p.dependency_blocks as usize);
    assert_eq!(
        writes.iter().copied().collect::<BTreeSet<_>>(),
        p.blocks.iter().copied().collect()
    );
    assert_eq!(p.data_blocks + p.metadata_blocks, p.dependency_blocks);
    assert_eq!(
        p.reserved_segment_blocks,
        p.dependency_blocks + p.placement_overhead_blocks
    );
}
fn committed_fixture() -> (Vec<u8>, BTreeMap<Vec<u8>, Vec<u8>>) {
    let d = fresh(256, 16);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let id = fs.create(b"kept").unwrap();
    fs.write(&id, 0, b"previous state").unwrap();
    fs.fsync(&id).unwrap();
    let other = fs.create(b"unrelated").unwrap();
    fs.write(&other, 0, b"unrelated bytes").unwrap();
    fs.fsync(&other).unwrap();
    drop(fs);
    (
        durable(&d),
        state(&[
            (b"kept", b"previous state"),
            (b"unrelated", b"unrelated bytes"),
        ]),
    )
}
fn prepare(d: &Device) -> (Filesystem<Device>, FileId, BTreeMap<Vec<u8>, Vec<u8>>) {
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let id = fs.lookup(b"kept").unwrap();
    fs.write(&id, 0, b"replacement across snapshots").unwrap();
    let new_name: &[u8] = if fs.lookup(b"new.txt").is_ok() {
        b"new2.txt"
    } else {
        b"new.txt"
    };
    let created = fs.create(new_name).unwrap();
    fs.write(&created, (PAGE - 2) as u64, b"cross").unwrap();
    let mut cross = vec![0; PAGE + 3];
    cross[PAGE - 2..].copy_from_slice(b"cross");
    let mut expected: BTreeMap<Vec<u8>, Vec<u8>> = [
        (b"kept".to_vec(), b"replacement across snapshots".to_vec()),
        (b"unrelated".to_vec(), b"unrelated bytes".to_vec()),
        (new_name.to_vec(), cross.clone()),
    ]
    .into_iter()
    .collect();
    if new_name == b"new2.txt" {
        expected.insert(b"new.txt".to_vec(), cross);
    }
    (fs, id, expected)
}
fn resource_oracle(old: &[u8], new: &[u8], plan: &Admission, start_slots: &Report) {
    let protected: BTreeSet<_> = start_slots
        .generations
        .iter()
        .flat_map(|g| g.blocks.iter().copied())
        .collect();
    assert_eq!(
        plan.blocks.iter().copied().collect::<BTreeSet<_>>().len(),
        plan.blocks.len()
    );
    for b in &plan.blocks {
        assert!(!protected.contains(b), "planned protected block {b}");
    }
    for b in protected {
        let a = b as usize * PAGE;
        assert_eq!(
            &old[a..a + PAGE],
            &new[a..a + PAGE],
            "protected block overwritten {b}"
        );
    }
}
#[test]
fn deterministic_d_p_r_r0_crash_fault_matrix() {
    let (base, previous) = committed_fixture();
    campaign(&base, &previous, "two");
    let d = load(&base, 0);
    let (mut fs, id, new) = prepare(&d);
    let first = next(&d);
    let p = fs.admission().unwrap();
    // dependency writes, D, CP, P, then retirement write.
    d.0.borrow_mut()
        .faults
        .insert(first + p.dependency_blocks + 3, Fault::WriteFailure);
    assert!(fs.fsync(&id).is_err());
    drop(fs);
    let three = durable(&d);
    assert_eq!(inspected(&three, &new).generations.len(), 3);
    save("r0-input.img", &three);
    campaign(&three, &new, "three");
}
fn campaign(base: &[u8], previous: &BTreeMap<Vec<u8>, Vec<u8>>, tag: &str) {
    let initial = inspected(base, previous);
    let old_generation = initial.selected.unwrap();
    let new_generation = old_generation + 1;
    let dry = load(base, 0);
    let (mut fs, id, new) = prepare(&dry);
    let plan = fs.admission().unwrap();
    let start = next(&dry);
    fs.fsync(&id).unwrap();
    let events: Vec<_> = dry
        .0
        .borrow()
        .trace
        .iter()
        .filter(|e| e.id >= start)
        .cloned()
        .collect();
    drop(fs);
    let cp_index = events
        .iter()
        .position(|e| {
            e.operation == "write"
                && e.digest != 0
                && (2 * PAGE as u64..=4 * PAGE as u64).contains(&e.offset)
        })
        .unwrap();
    let p_flush = cp_index + 1;
    assert_eq!(events[p_flush].operation, "flush");
    let mut rows = String::new();
    let mut count = 0;
    // Both no-I/O boundaries (before admission and after exact admission).
    for admit in [false, true] {
        let d = load(base, 0);
        let (fs, _, _) = prepare(&d);
        if admit {
            fs.admission().unwrap();
        }
        drop(fs);
        loss(&d);
        agreement(&d, previous);
        count += 1;
        rows.push_str(&format!("{{\"case\":\"{tag}-admission-{admit}\",\"expected\":[{old_generation}],\"selected\":{old_generation},\"status\":\"PASS\"}}\n"));
    }
    for seed in [0, 1, 2, 0x53465332] {
        for (i, event) in events.iter().enumerate() {
            let mut faults = vec![
                ("power-before".to_string(), Some(Fault::PowerLoss), false),
                ("power-after".to_string(), None, true),
            ];
            if event.operation == "write" {
                faults.push(("write-fail".into(), Some(Fault::WriteFailure), false));
                for n in [0, 1, 511, 512, 2048, 4095, 4096] {
                    faults.push((format!("partial-{n}"), Some(Fault::Partial(n)), false));
                }
                for (a, n) in [(1, 511), (512, 512), (2048, 2048)] {
                    faults.push((
                        format!("torn-{a}-{n}"),
                        Some(Fault::Torn { start: a, len: n }),
                        false,
                    ));
                }
                for persist in [false, true] {
                    faults.push((
                        format!("uncertain-{persist}"),
                        Some(Fault::UncertainWrite { persist }),
                        false,
                    ));
                }
            } else if event.operation == "flush" {
                for persist_writes in [0, 1, usize::MAX] {
                    for uncertain in [false, true] {
                        faults.push((
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
            for (label, fault, after) in faults {
                let case_name = format!("{tag}-seed-{seed}-event-{i}-{label}");
                if std::env::var("SUNFS_SFS2_CASE").is_ok_and(|wanted| wanted != case_name) {
                    continue;
                }
                let d = load(base, seed);
                let (mut fs, id, _) = prepare(&d);
                let first = next(&d);
                let target = first + i as u64;
                if let Some(f) = fault.clone() {
                    d.0.borrow_mut().faults.insert(target, f);
                }
                if after {
                    d.0.borrow_mut().crash_after = Some(target);
                }
                let result = fs.fsync(&id);
                let acknowledged = result.is_ok();
                if result.is_err() {
                    assert_eq!(fs.write(&id, 0, b"forbidden"), Err(FsError::ReadOnly));
                    assert_eq!(fs.fsync(&id), Err(FsError::ReadOnly));
                }
                drop(fs);
                loss(&d);
                let bytes = durable(&d);
                let report = check(&mut Bytes(&bytes));
                // The fixture controls persistence exactly. Derive the expected
                // durable generation from that schedule, independently of the
                // engine reply and the checker's observed selection.
                let expected_generation = if i < cp_index {
                    old_generation
                } else if i == cp_index {
                    if matches!(fault, Some(Fault::UncertainWrite { persist: true })) {
                        new_generation
                    } else {
                        old_generation
                    }
                } else if i == p_flush {
                    if after
                        || matches!(
                            fault,
                            Some(Fault::FlushFailure {
                                persist_writes: 1..,
                                ..
                            })
                        )
                    {
                        new_generation
                    } else {
                        old_generation
                    }
                } else {
                    new_generation
                };
                let permitted = vec![expected_generation];
                if acknowledged {
                    assert_eq!(expected_generation, new_generation);
                }
                let name = format!("{tag}-seed-{seed}-event-{i}-{label}");
                let verification = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    assert!(
                        permitted.contains(&report.selected.unwrap()),
                        "seed {seed} event {i} {label}: {report:#?}"
                    );
                    let selected = report.selected.unwrap();
                    let expected = if selected == new_generation {
                        &new
                    } else {
                        previous
                    };
                    resource_oracle(base, &bytes, &plan, &initial);
                    agreement(&d, expected);
                    // Every surviving older generation is independently checked too.
                    for g in &report.generations {
                        assert_eq!(
                            g.blocks.iter().copied().collect::<BTreeSet<_>>().len(),
                            g.blocks.len()
                        );
                    }

                    save(
                        &format!("traces/{name}.trace"),
                        format!("{:#?}", d.0.borrow().trace).as_bytes(),
                    );
                }));
                if let Err(panic) = verification {
                    save(&format!("unexpected-{name}.img"), &bytes);
                    save(
                        &format!("unexpected-{name}.trace"),
                        format!("{:#?}", d.0.borrow().trace).as_bytes(),
                    );
                    std::panic::resume_unwind(panic);
                }
                let selected = report.selected.unwrap();
                rows.push_str(&format!("{{\"case\":\"{name}\",\"event\":{target},\"operation\":\"{}\",\"fault\":\"{fault:?}\",\"acknowledged\":{acknowledged},\"expected\":{permitted:?},\"selected\":{selected},\"classification\":\"{}\",\"D\":true,\"P\":true,\"R\":true,\"R0\":true,\"status\":\"PASS\"}}\n",event.operation,report.status.name()));
                count += 1;
                if seed == 0 && label == "partial-511" && i == cp_index {
                    save(&format!("{tag}-torn-publication.img"), &bytes);
                }
                if seed == 0 && label == "power-after" && i == p_flush {
                    save(&format!("{tag}-three-retained.img"), &bytes);
                }
            }
        }
    }
    save(&format!("crash-schedules-{tag}.jsonl"), rows.as_bytes());
    println!(
        "SFS2_CRASH campaign={tag} cases={count} events={} seeds=4",
        events.len()
    );
}
fn page(bytes: &[u8], b: u64) -> Page {
    let a = b as usize * PAGE;
    Page::decode(&bytes[a..a + PAGE], b).unwrap()
}
fn replace(bytes: &mut [u8], p: &Page) -> BlockRef {
    let a = p.block as usize * PAGE;
    bytes[a..a + PAGE].copy_from_slice(&p.encode().unwrap());
    p.reference().unwrap()
}
/// Reseal each ancestor after an intentional semantic defect. This prevents CRC
/// rejection from substituting for the semantic checks under qualification.
fn corrupt_graph(bytes: &mut [u8], slot: u64, kind: &str) {
    let mut checkpoint = page(bytes, slot);
    let Payload::Checkpoint(cp) = &mut checkpoint.payload else {
        panic!()
    };
    let mut map_root = page(bytes, cp.node_map.block);
    let Payload::Root { pages, .. } = &mut map_root.payload else {
        panic!()
    };
    let mut map_page = page(bytes, pages[0].page.block);
    let Payload::Table {
        records: Records::Map(maps),
        ..
    } = &mut map_page.payload
    else {
        panic!()
    };
    if kind == "allocation" {
        let mut root = page(bytes, cp.segment_table.block);
        let Payload::Root { pages, .. } = &mut root.payload else {
            panic!()
        };
        let mut table = page(bytes, pages[0].page.block);
        let Payload::Table {
            records: Records::Segments(segments),
            ..
        } = &mut table.payload
        else {
            panic!()
        };
        let s = segments.iter_mut().find(|s| s.live > 1).unwrap();
        // Swap one live/free bit to preserve the codec's popcount invariants.
        let live = (0..256usize)
            .find(|o| s.bitmap[o / 8] & (1 << (o % 8)) != 0)
            .unwrap();
        let free = (0..256usize)
            .find(|o| s.bitmap[o / 8] & (1 << (o % 8)) == 0)
            .unwrap();
        s.bitmap[live / 8] ^= 1 << (live % 8);
        s.bitmap[free / 8] ^= 1 << (free % 8);
        pages[0].page = replace(bytes, &table);
        cp.segment_table = replace(bytes, &root);
    } else {
        let m = maps
            .iter_mut()
            .find(|m| {
                if kind == "binding" {
                    m.kind == 2
                } else {
                    m.kind == 1
                }
            })
            .unwrap();
        let mut inode_page = page(bytes, m.node.block);
        let Payload::Inode(inode) = &mut inode_page.payload else {
            panic!()
        };
        match kind {
            "node" => inode.id.incarnation += 1,
            "links" => inode.nlink = 0,
            "extent" | "duplicate" | "generation" => {
                let mut root = page(bytes, inode.extents.block);
                let Payload::Root { pages, .. } = &mut root.payload else {
                    panic!()
                };
                let mut ep = page(bytes, pages[0].page.block);
                let Payload::Extents { entries, .. } = &mut ep.payload else {
                    panic!()
                };
                match kind {
                    "extent" => entries[0].physical = u64::MAX,
                    "duplicate" => entries[0].physical = m.node.block,
                    _ => entries[0].content_generation = cp.generation + 1,
                }
                pages[0].page = replace(bytes, &ep);
                inode.extents = replace(bytes, &root);
            }
            "binding" => {
                let mut root = page(bytes, inode.directory.block);
                let Payload::Root { pages, .. } = &mut root.payload else {
                    panic!()
                };
                let mut dp = page(bytes, pages[0].page.block);
                let Payload::Directory { entries, .. } = &mut dp.payload else {
                    panic!()
                };
                entries[0].target.incarnation += 1;
                pages[0].page = replace(bytes, &dp);
                inode.directory = replace(bytes, &root);
            }
            _ => panic!(),
        }
        m.node = replace(bytes, &inode_page);
        pages[0].page = replace(bytes, &map_page);
        cp.node_map = replace(bytes, &map_root);
    }
    replace(bytes, &checkpoint);
}
#[test]
fn recovery_semantic_corruption_fallback_and_read_errors() {
    let (base, current) = committed_fixture();
    let r = inspected(&base, &current);
    let newest = r.generations.last().unwrap();
    // Generation 3 has only kept; generation 4 adds unrelated.
    let old = state(&[(b"kept", b"previous state")]);
    let mut rows = String::new();
    for kind in [
        "checkpoint",
        "data",
        "node",
        "links",
        "extent",
        "duplicate",
        "generation",
        "binding",
        "allocation",
    ] {
        let mut bytes = base.clone();
        if kind == "checkpoint" {
            bytes[newest.slot as usize * PAGE + 64] ^= 1;
        } else if kind == "data" {
            let n = newest.nodes.iter().find(|n| !n.extents.is_empty()).unwrap();
            bytes[n.extents[0].physical as usize * PAGE] ^= 1;
        } else {
            corrupt_graph(&mut bytes, newest.slot, kind);
        }
        let d = load(&bytes, 0);
        let fs = agreement(&d, &old);
        assert_eq!(fs.state(), MountState::DegradedReadOnly);
        assert_eq!(fs.generation(), 3);
        drop(fs);
        let mut fs = Filesystem::mount(d.clone()).unwrap();
        assert_eq!(fs.create(b"forbidden"), Err(FsError::ReadOnly));
        assert_eq!(durable(&d), bytes);
        save(&format!("corrupt-{kind}.img"), &bytes);
        rows.push_str(&format!(
            "{{\"kind\":\"{kind}\",\"expected\":3,\"read_only\":true,\"status\":\"PASS\"}}\n"
        ));
    }
    // A missing/corrupt fallback closure also prevents writable admission, even
    // though the selected newest state is complete.
    let mut bytes = base.clone();
    let older = &r.generations[0];
    bytes[older.blocks[0] as usize * PAGE + 64] ^= 1;
    let d = load(&bytes, 0);
    let fs = agreement(&d, &current);
    assert_eq!(fs.state(), MountState::DegradedReadOnly);
    // Read failure in SB, CP and data paths yields explicit I/O, never guessed data.
    let dry = load(&base, 0);
    Filesystem::mount(dry.clone()).unwrap();
    let reads = dry.0.borrow().trace.clone();
    for e in reads {
        let d = load(&base, 0);
        d.0.borrow_mut().faults.insert(e.id, Fault::ReadFailure);
        assert!(
            matches!(Filesystem::mount(d.clone()), Err(FsError::Io(_))),
            "read {}",
            e.id
        );
        assert_eq!(durable(&d), base);
    }
    let d = load(&base, 0);
    d.0.borrow_mut().set_flush_support(false);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let id = fs.lookup(b"kept").unwrap();
    assert_eq!(fs.fsync(&id), Err(FsError::ReadOnly));
    let mut bytes = base.clone();
    for b in [2u64, 3, 4] {
        bytes[b as usize * PAGE..(b as usize + 1) * PAGE].fill(0);
    }
    let d = load(&bytes, 0);
    assert!(matches!(Filesystem::mount(d), Err(FsError::Corrupt)));
    let mut bytes = base.clone();
    for b in [0, 1] {
        let mut p = page(&bytes, b);
        let Payload::Superblock(sb) = &mut p.payload else {
            panic!()
        };
        sb.features.readonly = 1;
        replace(&mut bytes, &p);
    }
    let d = load(&bytes, 0);
    assert_eq!(
        Filesystem::mount(d).unwrap().state(),
        MountState::DegradedReadOnly
    );
    let mut bytes = base;
    for b in [0usize, 1] {
        let a = b * PAGE;
        bytes[a + HEADER + 2..a + HEADER + 4].copy_from_slice(&3u16.to_le_bytes());
        bytes[a + 64..a + 72].fill(0);
        let c = crc64(&bytes[a..a + PAGE]);
        bytes[a + 64..a + 72].copy_from_slice(&c.to_le_bytes());
    }
    let d = load(&bytes, 0);
    assert!(matches!(
        Filesystem::mount(d),
        Err(FsError::Invalid(sunlight_sunfs::Error::Unsupported))
    ));
    save("corruption-cases.jsonl", rows.as_bytes());
    println!("SFS2_CORRUPTION cases=9 resealed_semantic=7");
}
/// Inject explicit reverse persistence of independent dependency blocks before D,
/// including partial fragments. Stop at every prefix of the persistence schedule.
#[test]
fn reordered_dependency_persistence_never_publishes_without_checkpoint() {
    struct Reorder {
        inner: Device,
        writes: usize,
        cut: usize,
    }
    impl BlockDevice for Reorder {
        fn info(&self) -> DeviceInfo {
            self.inner.info()
        }
        fn read(&mut self, o: u64, b: &mut [u8]) -> IoResult<()> {
            self.inner.read(o, b)
        }
        fn write(&mut self, o: u64, b: &[u8]) -> IoResult<WriteCompletion> {
            let r = self.inner.write(o, b)?;
            self.writes += 1;
            Ok(r)
        }
        fn flush(&mut self) -> IoResult<()> {
            if self.writes > 2 {
                for index in (0..self.writes).rev().take(self.cut) {
                    self.inner.0.borrow_mut().persist(index, 512, 512)?;
                    self.inner.0.borrow_mut().persist(index, 0, PAGE)?;
                }
                self.inner.0.borrow_mut().power_loss();
                return Err(IoFailure::uncertain(IoKind::Flush));
            }
            self.writes = 0;
            self.inner.flush()
        }
    }
    let (base, old) = committed_fixture();
    let d = load(&base, 0);
    let (fs, _, _) = prepare(&d);
    let n = fs.admission().unwrap().dependency_blocks as usize;
    drop(fs);
    let mut rows = String::new();
    for cut in 0..=n {
        let d = load(&base, cut as u64);
        let mut fs = Filesystem::mount(Reorder {
            inner: d.clone(),
            writes: 0,
            cut,
        })
        .unwrap();
        let id = fs.lookup(b"kept").unwrap();
        fs.write(&id, 0, b"replacement across snapshots").unwrap();
        let f = fs.create(b"new.txt").unwrap();
        fs.write(&f, (PAGE - 2) as u64, b"cross").unwrap();
        assert!(fs.fsync(&id).is_err());
        drop(fs);
        loss(&d);
        agreement(&d, &old);
        rows.push_str(&format!(
            "{{\"reverse_prefix\":{cut},\"expected\":[4],\"selected\":4,\"status\":\"PASS\"}}\n"
        ));
        save(
            &format!("traces/reverse-prefix-{cut}.trace"),
            format!("{:#?}", d.0.borrow().trace).as_bytes(),
        );
    }
    save("reordering.jsonl", rows.as_bytes());
    println!("SFS2_REORDER cases={}", n + 1);
}
#[test]
fn configured_extent_cap_and_counter_exhaustion_reject_before_io() {
    let d = fresh(256, 16);
    let mut bytes = durable(&d);
    for b in [0, 1] {
        let mut p = page(&bytes, b);
        let Payload::Superblock(sb) = &mut p.payload else {
            panic!()
        };
        sb.maximum_extents = 1;
        replace(&mut bytes, &p);
    }
    let d = load(&bytes, 0);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let id = fs.create(b"cap").unwrap();
    assert_eq!(fs.write(&id, PAGE as u64, b"x"), Err(FsError::Limit));
    fs.write(&id, 0, b"x").unwrap();
    fs.fsync(&id).unwrap();
    drop(fs);
    loss(&d);
    agreement(&d, &state(&[(b"cap", b"x")]));
    let (bytes, current) = committed_fixture();
    let r = inspected(&bytes, &current);
    for field in ["nid", "generation"] {
        let mut bytes = bytes.clone();
        let mut p = page(&bytes, r.generations.last().unwrap().slot);
        let Payload::Checkpoint(cp) = &mut p.payload else {
            panic!()
        };
        if field == "nid" {
            cp.next_nid = u64::MAX;
        } else {
            cp.generation = u64::MAX;
            p.birth = u64::MAX;
        }
        replace(&mut bytes, &p);
        let d = load(&bytes, 0);
        let mut fs = Filesystem::mount(d.clone()).unwrap();
        let first = next(&d);
        if field == "nid" {
            assert!(matches!(
                fs.create(b"overflow"),
                Err(FsError::Invalid(sunlight_sunfs::Error::Overflow))
            ));
        } else {
            let id = fs.lookup(b"kept").unwrap();
            fs.write(&id, 0, b"new").unwrap();
            assert!(matches!(
                fs.fsync(&id),
                Err(FsError::Invalid(sunlight_sunfs::Error::Overflow))
            ));
        }
        assert_eq!(next(&d), first);
        assert_eq!(durable(&d), bytes);
    }
}
#[test]
fn handles_from_an_abandoned_mount_never_alias_new_nodes() {
    let d = fresh(256, 16);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let abandoned = fs.create(b"lost").unwrap();
    fs.write(&abandoned, 0, b"volatile").unwrap();
    drop(fs);
    loss(&d);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let replacement = fs.create(b"replacement").unwrap();
    // The uncommitted high-water mark vanished with its epoch. Its old runtime
    // handle must still fail, even if the new working identity has the same tuple.
    assert_eq!(abandoned.node, replacement.node);
    assert_eq!(fs.write(&abandoned, 0, b"bad"), Err(FsError::Stale));
    fs.write(&replacement, 0, b"correct").unwrap();
    fs.fsync(&replacement).unwrap();
    let stable = (replacement.volume, replacement.node.clone());
    drop(fs);
    loss(&d);
    let mut fs = Filesystem::mount(d).unwrap();
    let reopened = fs.lookup(b"replacement").unwrap();
    assert_eq!((reopened.volume, reopened.node.clone()), stable);
    assert_eq!(fs.read(&replacement, 0, &mut [0; 8]), Err(FsError::Stale));
    let next = fs.create(b"next").unwrap();
    assert!(next.node.nid > reopened.node.nid);
}
