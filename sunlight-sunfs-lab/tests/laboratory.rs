use sunlight_sunfs::{block::*, bootstrap::Bootstrap, codec::*, geometry::*};
use sunlight_sunfs_check::{check, Bytes, Failure, ReadImage, Status};
use sunlight_sunfs_lab::{
    image::{parse_size, Image},
    simulator::{Fault, Simulator},
};
fn plan() -> Bootstrap {
    Bootstrap::plan(
        Geometry::calculate(Parameters::laboratory(8 * 1024 * 1024, 256)).unwrap(),
        [0x53; 16],
    )
    .unwrap()
}
fn formatted() -> (Bootstrap, Simulator) {
    let p = plan();
    let mut s = Simulator::new(8 * 1024 * 1024, 0x534653).unwrap();
    p.publish(&mut s).unwrap();
    (p, s)
}
fn status(bytes: &[u8]) -> Status {
    check(&mut Bytes(bytes)).status
}
fn stamp(b: &mut [u8]) {
    b[64..72].fill(0);
    let crc = crc64(b);
    b[64..72].copy_from_slice(&crc.to_le_bytes());
}
fn encoded_mutate(bytes: &mut [u8], block: u64, f: impl FnOnce(&mut Page)) {
    let a = block as usize * PAGE;
    let mut p = Page::decode(&bytes[a..a + PAGE], block).unwrap();
    f(&mut p);
    bytes[a..a + PAGE].copy_from_slice(&p.encode().unwrap());
}
#[test]
fn formatter_checker_clean_geometries() {
    for bps in [64, 256, 512] {
        let p = Bootstrap::plan(
            Geometry::calculate(Parameters::laboratory(16 * 1024 * 1024, bps)).unwrap(),
            [9; 16],
        )
        .unwrap();
        let mut s = Simulator::new(16 * 1024 * 1024, 0).unwrap();
        p.publish(&mut s).unwrap();
        s.power_loss();
        s.reset().unwrap();
        let before = s.durable().to_vec();
        let r = check(&mut Bytes(s.durable()));
        assert_eq!(r.status, Status::Clean, "{r:#?}");
        assert_eq!(r.selected, Some(2));
        assert_eq!(r.union_blocks, p.dependencies.len());
        assert_eq!(before, s.durable());
    }
    // Maximum-capacity geometries use a sparse independent reader, keeping the
    // test bounded without allocating multi-GiB simulator images. The 64-block
    // case forces bootstrap metadata and self-accounting summaries across segments.
    struct Sparse {
        capacity: u64,
        pages: std::collections::BTreeMap<u64, [u8; PAGE]>,
    }
    impl ReadImage for Sparse {
        fn capacity(&self) -> u64 {
            self.capacity
        }
        fn read_page(&mut self, block: u64) -> Result<[u8; PAGE], Failure> {
            if block >= self.capacity / PAGE as u64 {
                return Err(Failure::Read);
            }
            Ok(self.pages.get(&block).copied().unwrap_or([0; PAGE]))
        }
    }
    for bps in [64u32, 256, 512] {
        let segments = 82 * (3976 / (64 + bps as u64 / 8));
        let capacity = (segments + 1) * bps as u64 * 4096;
        let g = Geometry::calculate(Parameters::laboratory(capacity, bps)).unwrap();
        let p = Bootstrap::plan(g, [9; 16]).unwrap();
        if bps == 64 {
            assert!(p.dependencies.len() > 64);
            assert_eq!(g.summary_pages, 2);
        }
        let pages = p
            .superblocks
            .iter()
            .chain(&p.dependencies)
            .chain(&p.checkpoints)
            .map(|p| (p.block, p.encode().unwrap()))
            .collect();
        let r = check(&mut Sparse { capacity, pages });
        assert_eq!(r.status, Status::Clean, "maximum geometry {bps}: {r:#?}");
    }
}
#[test]
fn persistence_ack_flush_loss_and_reset() {
    let mut s = Simulator::new(4096, 1).unwrap();
    assert_eq!(s.write(0, &[1; 512]).unwrap(), WriteCompletion::Completed);
    assert_eq!(&s.durable()[..512], &[0; 512]);
    let mut read = [0; 512];
    s.read(0, &mut read).unwrap();
    assert_eq!(read, [1; 512]);
    s.power_loss();
    assert_eq!(&s.durable()[..512], &[0; 512]);
    assert!(s.read(0, &mut read).is_err());
    s.reset().unwrap();
    assert_eq!(s.info().incarnation, 2);
    s.write(0, &[2; 512]).unwrap();
    s.flush().unwrap();
    s.power_loss();
    assert_eq!(&s.durable()[..512], &[2; 512]);
}
#[test]
fn persistence_failures_uncertainty_and_dishonesty() {
    for persist in [0, 1] {
        for uncertain in [false, true] {
            let mut s = Simulator::new(4096, 2).unwrap();
            s.write(0, &[3; 512]).unwrap();
            s.faults.insert(
                2,
                Fault::FlushFailure {
                    persist_writes: persist,
                    uncertain,
                },
            );
            let e = s.flush().unwrap_err();
            assert_eq!(
                e.certainty,
                if uncertain {
                    Certainty::Uncertain
                } else {
                    Certainty::Known
                }
            );
            s.power_loss();
            assert_eq!(s.durable()[0], if persist == 0 { 0 } else { 3 });
        }
    }
    for persist in [false, true] {
        let mut s = Simulator::new(4096, 2).unwrap();
        s.faults.insert(1, Fault::UncertainWrite { persist });
        let e = s.write(0, &[4; 512]).unwrap_err();
        assert_eq!(e.certainty, Certainty::Uncertain);
        s.power_loss();
        assert_eq!(s.durable()[0], if persist { 4 } else { 0 });
    }
    let mut s = Simulator::new(4096, 3).unwrap();
    s.dishonest_flush = true;
    s.write(0, &[5; 512]).unwrap();
    s.flush().unwrap();
    s.power_loss();
    assert_eq!(s.durable()[0], 0); // explicitly excluded model
}
#[test]
fn reordered_persistence_and_repeated_blocks() {
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for order in permutations {
        let mut s = Simulator::new(4096, 4).unwrap();
        for i in 0..3 {
            s.write(i * 512, &[i as u8 + 1; 512]).unwrap();
        }
        for i in order {
            s.persist(i, 0, 512).unwrap();
        }
        s.power_loss();
        for i in 0..3 {
            assert_eq!(s.durable()[i * 512], i as u8 + 1);
        }
    }
    let mut s = Simulator::new(4096, 4).unwrap();
    s.write(0, &[1; 512]).unwrap();
    s.write(0, &[2; 512]).unwrap();
    assert!(s.persist(1, 0, 512).is_err());
    s.persist(0, 0, 512).unwrap();
    s.persist(1, 0, 512).unwrap();
    s.flush().unwrap();
    s.power_loss();
    assert_eq!(s.durable()[0], 2);
}
#[test]
fn geometry_io_and_sector_adapter() {
    let mut s = Simulator::new(4096, 0).unwrap();
    let data = [1; 512];
    assert!(s.write(u64::MAX - 511, &data).is_err());
    assert!(s.write(1, &data).is_err());
    assert!(s.write(4096, &data).is_err());
    assert!(s.write(0, &[]).is_err());
    s.set_flush_support(false);
    assert_eq!(s.flush().unwrap_err().kind, IoKind::UnsupportedDurability);
    assert_eq!(
        s.write_fua(0, &data).unwrap_err().kind,
        IoKind::UnsupportedDurability
    );
    s.set_read_only(true);
    assert_eq!(s.write(0, &data).unwrap_err().kind, IoKind::ReadOnly);
    let mut adapter = Sector512Adapter::new(Simulator::new(4096, 5).unwrap()).unwrap();
    adapter.write(0, &[7; 4096]).unwrap();
    adapter.flush().unwrap();
    assert_eq!(
        adapter
            .inner
            .trace
            .iter()
            .filter(|e| e.operation == "write")
            .count(),
        8
    );
    adapter.inner.faults.insert(12, Fault::ReadFailure);
    let mut output = [99; 4096];
    assert!(adapter.read(0, &mut output).is_err());
    assert_eq!(output, [99; 4096]);
    let mut adapter = Sector512Adapter::new(Simulator::new(4096, 5).unwrap()).unwrap();
    adapter.inner.faults.insert(3, Fault::Partial(17));
    let e = adapter.write(0, &[1; 4096]).unwrap_err();
    assert_eq!(e.completed, Some(1041));
}
#[test]
fn formatter_crashes_each_operation_boundary() {
    let (p, s) = formatted();
    let events = s.trace.len();
    if let Some(dir) = std::env::var_os("SUNFS_EVIDENCE_DIR") {
        std::fs::write(
            std::path::PathBuf::from(dir).join("bootstrap.img"),
            s.durable(),
        )
        .unwrap();
    }
    let mut counts = [0usize; 3];
    for seed in [0, 1, 2, 0x534653] {
        for boundary in 1..=events as u64 {
            for after in [false, true] {
                let mut sim = Simulator::new(8 * 1024 * 1024, seed).unwrap();
                if after {
                    sim.crash_after = Some(boundary);
                } else {
                    sim.faults.insert(boundary, Fault::PowerLoss);
                }
                let result = p.publish(&mut sim);
                sim.power_loss();
                let r = check(&mut Bytes(sim.durable()));
                match r.status {
                    Status::Clean => counts[0] += 1,
                    Status::DegradedRecoverable => counts[1] += 1,
                    Status::Corrupt => counts[2] += 1,
                    _ => panic!("unexpected {r:#?}"),
                }
                if result.is_ok() {
                    assert_eq!(r.status, Status::Clean);
                }
                if r.status == Status::Clean {
                    assert_eq!(r.selected, Some(2));
                }
                // Repeat the identical schedule from a new device; compare full durable bytes and trace.
                let mut replay = Simulator::new(8 * 1024 * 1024, seed).unwrap();
                if after {
                    replay.crash_after = Some(boundary);
                } else {
                    replay.faults.insert(boundary, Fault::PowerLoss);
                }
                assert_eq!(result, p.publish(&mut replay));
                replay.power_loss();
                assert_eq!(sim.durable(), replay.durable());
                assert_eq!(sim.trace, replay.trace);
                if let Some(dir) = std::env::var_os("SUNFS_EVIDENCE_DIR") {
                    let out = std::path::PathBuf::from(dir).join("crash-traces");
                    std::fs::create_dir_all(&out).unwrap();
                    std::fs::write(out.join(format!("seed-{seed}-event-{boundary}-after-{after}.trace")),
                        format!("seed={seed} event={boundary} after={after}\nresult={result:?}\nstatus={}\ndurable_crc64={:016x}\ntrace={:#?}\nchecker={r:#?}",r.status.name(),crc64(sim.durable()),sim.trace)).unwrap();
                }
            }
        }
    }
    println!(
        "SFS1_CRASH_BOUNDARIES cases={} events={} clean={} degraded={} invalid={}",
        events * 2 * 4,
        events,
        counts[0],
        counts[1],
        counts[2]
    );
}
#[test]
fn formatter_torn_partial_failed_and_uncertain_operations() {
    let (p, s) = formatted();
    let writes: Vec<u64> = s
        .trace
        .iter()
        .filter(|e| e.operation == "write")
        .map(|e| e.id)
        .collect();
    let flushes: Vec<u64> = s
        .trace
        .iter()
        .filter(|e| e.operation == "flush")
        .map(|e| e.id)
        .collect();
    let cuts = [
        0, 1, 511, 512, 1024, 1536, 2048, 2560, 3072, 3584, 4095, 4096,
    ];
    let mut cases = 0;
    let first_flush = flushes[0];
    let last_flush = *flushes.last().unwrap();
    use std::io::Write;
    let mut corpus = std::env::var_os("SUNFS_EVIDENCE_DIR").map(|dir| {
        std::io::BufWriter::new(
            std::fs::File::create(std::path::PathBuf::from(dir).join("fault-corpus.jsonl"))
                .unwrap(),
        )
    });
    for id in writes {
        for cut in cuts {
            for fault in [
                Fault::Torn { start: 0, len: cut },
                Fault::Torn {
                    start: 4096 - cut,
                    len: cut,
                },
                Fault::Partial(cut),
            ] {
                let mut sim = Simulator::new(8 * 1024 * 1024, 0x534653).unwrap();
                // A prefix tear can contain every nonzero CP byte: the rest
                // of a fresh slot is already zero. Compare complete page bytes,
                // rather than assuming only a 4096-byte transfer can recover.
                let complete_cp = if let Fault::Torn { start, len } = &fault {
                    if let Some(cp) = p
                        .checkpoints
                        .iter()
                        .find(|cp| cp.block * 4096 == s.trace[(id - 1) as usize].offset)
                    {
                        let encoded = cp.encode().unwrap();
                        let mut persisted = [0u8; PAGE];
                        persisted[*start..*start + *len]
                            .copy_from_slice(&encoded[*start..*start + *len]);
                        persisted == encoded
                    } else {
                        false
                    }
                } else {
                    false
                };
                let expected = if id > first_flush && complete_cp {
                    Status::DegradedRecoverable
                } else {
                    Status::Corrupt
                };

                let fault_name = format!("{fault:?}");
                sim.faults.insert(id, fault);
                assert!(p.publish(&mut sim).is_err());
                sim.power_loss();
                let observed = status(sim.durable());
                assert_eq!(observed, expected, "event={id} fault={fault_name}");
                if let Some(out) = corpus.as_mut() {
                    writeln!(out,"{{\"seed\":5457491,\"event\":{id},\"fault\":\"{fault_name}\",\"classification\":\"{}\"}}",observed.name()).unwrap();
                }
                assert!(!sim
                    .trace
                    .iter()
                    .any(|e| e.id > id && e.operation == "write"));
                cases += 1;
            }
        }
        for fault in [
            Fault::WriteFailure,
            Fault::UncertainWrite { persist: false },
            Fault::UncertainWrite { persist: true },
        ] {
            let mut sim = Simulator::new(8 * 1024 * 1024, 1).unwrap();
            let expected = if fault == (Fault::UncertainWrite { persist: true }) && id > first_flush
            {
                if id == last_flush - 1 {
                    Status::Clean
                } else {
                    Status::DegradedRecoverable
                }
            } else {
                Status::Corrupt
            };
            let fault_name = format!("{fault:?}");
            sim.faults.insert(id, fault);
            assert!(p.publish(&mut sim).is_err());
            sim.power_loss();
            let observed = status(sim.durable());
            assert_eq!(observed, expected, "event={id} fault={fault_name}");
            if let Some(out) = corpus.as_mut() {
                writeln!(out,"{{\"seed\":1,\"event\":{id},\"fault\":\"{fault_name}\",\"classification\":\"{}\"}}",observed.name()).unwrap();
            }
            cases += 1;
        }
    }
    for id in flushes {
        for count in [0, 1, usize::MAX] {
            for uncertain in [false, true] {
                let mut sim = Simulator::new(8 * 1024 * 1024, 2).unwrap();
                sim.faults.insert(
                    id,
                    Fault::FlushFailure {
                        persist_writes: count,
                        uncertain,
                    },
                );
                assert!(p.publish(&mut sim).is_err());
                sim.power_loss();
                let expected = if id == last_flush && count > 0 {
                    if count == 1 {
                        Status::DegradedRecoverable
                    } else {
                        Status::Clean
                    }
                } else {
                    Status::Corrupt
                };
                let observed = status(sim.durable());
                assert_eq!(
                    observed, expected,
                    "flush_event={id} count={count} uncertain={uncertain}"
                );
                if let Some(out) = corpus.as_mut() {
                    writeln!(out,"{{\"seed\":2,\"event\":{id},\"fault\":\"FlushFailure count={count} uncertain={uncertain}\",\"classification\":\"{}\"}}",observed.name()).unwrap();
                }
                cases += 1;
            }
        }
    }
    println!("SFS1_FAULT_MATRIX cases={cases} tear_offsets={cuts:?}");
}
#[test]
fn corruption_classification_and_generation_selection() {
    let (p, s) = formatted();
    let original = s.durable();
    let mut cases = 0;
    for block in [
        0,
        1,
        2,
        3,
        p.geometry.segment_start,
        p.geometry.segment_start + 2,
        p.geometry.segment_start + 5,
        p.geometry.segment_start + 6,
    ] {
        let mut bytes = original.to_vec();
        bytes[block as usize * PAGE + 200] ^= 1;
        let r = check(&mut Bytes(&bytes));
        assert_eq!(
            r.status,
            if block <= 3 {
                Status::DegradedRecoverable
            } else {
                Status::Corrupt
            },
            "{r:#?}"
        );
        cases += 1;
    }
    let mut bytes = original.to_vec();
    encoded_mutate(&mut bytes, 3, |p| {
        if let Payload::Checkpoint(cp) = &mut p.payload {
            cp.generation = 3;
            cp.predecessor = 2;
            p.birth = 3;
            cp.node_map.crc ^= 1;
        }
    });
    let r = check(&mut Bytes(&bytes));
    assert_eq!(r.status, Status::DegradedRecoverable);
    assert_eq!(r.selected, Some(1));
    assert_eq!(r.slots[1].class, "UNRECOVERABLE");
    cases += 1;
    let mut bytes = original.to_vec();
    encoded_mutate(&mut bytes, 3, |p| {
        if let Payload::Checkpoint(cp) = &mut p.payload {
            cp.generation = 5;
            cp.predecessor = 1;
            p.birth = 5;
        }
    });
    assert_eq!(check(&mut Bytes(&bytes)).selected, Some(5));
    cases += 1;
    let mut bytes = original.to_vec();
    encoded_mutate(&mut bytes, 3, |p| {
        if let Payload::Checkpoint(cp) = &mut p.payload {
            cp.generation = 1;
            cp.predecessor = 0;
            p.birth = 1;
        }
    });
    assert_eq!(status(&bytes), Status::Indeterminate);
    cases += 1;
    let mut bytes = original.to_vec();
    encoded_mutate(&mut bytes, 0, |p| p.uuid = [7; 16]);
    assert_eq!(status(&bytes), Status::Indeterminate);
    cases += 1;
    let mut bytes = original.to_vec();
    for block in 0..2 {
        let a = block * PAGE;
        bytes[a + 120..a + 128].copy_from_slice(&7u64.to_le_bytes());
        stamp(&mut bytes[a..a + PAGE]);
    }
    assert_eq!(status(&bytes), Status::Unsupported);
    cases += 1;
    let mut bytes = original.to_vec();
    for block in 0..2 {
        encoded_mutate(&mut bytes, block, |p| {
            if let Payload::Superblock(sb) = &mut p.payload {
                sb.segment_start = 0;
            }
        });
    }
    assert_eq!(status(&bytes), Status::Corrupt);
    cases += 1;
    let mut bytes = original.to_vec();
    encoded_mutate(&mut bytes, 3, |p| {
        if let Payload::Checkpoint(cp) = &mut p.payload {
            cp.state = 0;
        }
    });
    assert_eq!(status(&bytes), Status::DegradedRecoverable);
    cases += 1;
    assert_ne!(status(&original[..original.len() - 1]), Status::Clean);
    cases += 1;
    println!("SFS1_CORRUPTION cases={cases}");
}
// Recompute all metadata reference CRCs after a deliberate semantic edit. The
// checker does not import this test helper or the formatter's planner.
fn relink(bytes: &mut [u8], p: &Bootstrap) {
    for _ in 0..4 {
        for original in p.dependencies.iter().chain(&p.checkpoints) {
            let a = original.block as usize * PAGE;
            let mut page = Page::decode(&bytes[a..a + PAGE], original.block).unwrap();
            let update = |r: &mut BlockRef| {
                if !r.absent() {
                    let x = r.block as usize * PAGE;
                    r.crc = u64::from_le_bytes(bytes[x + 64..x + 72].try_into().unwrap());
                }
            };
            match &mut page.payload {
                Payload::Inode(i) => {
                    update(&mut i.directory);
                    update(&mut i.extents);
                }
                Payload::Root { pages, .. } => {
                    for d in pages {
                        update(&mut d.page);
                    }
                }
                Payload::Table { records, .. } => match records {
                    Records::Map(v) => {
                        for m in v {
                            update(&mut m.node);
                        }
                    }
                    Records::Segments(v) => {
                        for s in v {
                            update(&mut s.summary);
                        }
                    }
                    _ => {}
                },
                Payload::Summary { next, .. } => update(next),
                Payload::Checkpoint(c) => {
                    update(&mut c.node_map);
                    update(&mut c.segment_table);
                    update(&mut c.orphan_table);
                }
                _ => {}
            }
            bytes[a..a + PAGE].copy_from_slice(&page.encode().unwrap());
        }
    }
}
#[test]
fn semantic_corruption_with_valid_checksums() {
    let (p, s) = formatted();
    let start = p.geometry.segment_start;
    for mutation in 0..7 {
        let mut bytes = s.durable().to_vec();
        match mutation {
            0 => encoded_mutate(&mut bytes, start, |p| {
                if let Payload::Inode(i) = &mut p.payload {
                    i.nlink = 3;
                }
            }),
            1 => encoded_mutate(&mut bytes, start + 6, |p| {
                if let Payload::Table {
                    records: Records::Segments(v),
                    ..
                } = &mut p.payload
                {
                    v[0].bitmap[0] ^= 1;
                    v[0].bitmap[1] ^= 1;
                }
            }),
            2 => encoded_mutate(&mut bytes, start + 2, |p| {
                if let Payload::Table {
                    records: Records::Map(v),
                    ..
                } = &mut p.payload
                {
                    v[0].id.incarnation = 2;
                }
            }),
            3 => encoded_mutate(&mut bytes, start, |p| {
                if let Payload::Inode(i) = &mut p.payload {
                    i.directory.block = start + 3;
                }
            }),
            4 => encoded_mutate(&mut bytes, start + 6, |p| {
                if let Payload::Table {
                    records: Records::Segments(v),
                    ..
                } = &mut p.payload
                {
                    v[0].epoch = 2;
                }
            }),
            5 => encoded_mutate(&mut bytes, start + 6, |p| {
                if let Payload::Table {
                    records: Records::Segments(v),
                    ..
                } = &mut p.payload
                {
                    v[0].summary.block = start;
                }
            }),
            _ => {
                encoded_mutate(&mut bytes, 2, |p| {
                    if let Payload::Checkpoint(c) = &mut p.payload {
                        c.allocated_blocks += 1;
                    }
                });
                encoded_mutate(&mut bytes, 3, |p| {
                    if let Payload::Checkpoint(c) = &mut p.payload {
                        c.allocated_blocks += 1;
                    }
                });
            }
        }
        relink(&mut bytes, &p);
        let r = check(&mut Bytes(&bytes));
        assert_eq!(r.status, Status::Corrupt, "mutation {mutation}: {r:#?}");
    }
}
#[test]
fn checker_read_failure_is_indeterminate() {
    struct Broken;
    impl ReadImage for Broken {
        fn capacity(&self) -> u64 {
            8 * 1024 * 1024
        }
        fn read_page(&mut self, _: u64) -> Result<[u8; PAGE], Failure> {
            Err(Failure::Read)
        }
    }
    assert_eq!(check(&mut Broken).status, Status::Indeterminate);
}
#[test]
fn disposable_image_safety_and_readonly_check() {
    let root = std::env::temp_dir().join(format!("sunfs-lab-test-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("new.img");
    let p = plan();
    assert!(Image::create(&path, 8 * 1024 * 1024, false).is_err());
    assert!(!path.exists());
    let mut image = Image::create(&path, 8 * 1024 * 1024, true).unwrap();
    p.publish(&mut image).unwrap();
    drop(image);
    let before = std::fs::read(&path).unwrap();
    assert!(Image::create(&path, 8 * 1024 * 1024, true).is_err());
    let mut file = sunlight_sunfs_check::ReadFile::open(&path).unwrap();
    assert_eq!(check(&mut file).status, Status::Clean);
    assert_eq!(before, std::fs::read(&path).unwrap());
    std::os::unix::fs::symlink(&path, root.join("alias.img")).unwrap();
    assert!(Image::create(&root.join("alias.img"), 8 * 1024 * 1024, true).is_err());
    std::os::unix::fs::symlink(&root, root.join("alias-dir")).unwrap();
    assert!(Image::create(&root.join("alias-dir/other.img"), 8 * 1024 * 1024, true).is_err());
    assert!(Image::create(&root.join("../other.img"), 8 * 1024 * 1024, true).is_err());
    assert!(Image::create(std::path::Path::new("/dev/null"), 8 * 1024 * 1024, true).is_err());
    for text in [
        "0",
        "-1",
        "18446744073709551616",
        "18446744073709551615GiB",
        "1MiBjunk",
        "1",
    ] {
        assert!(parse_size(text).is_err());
    }
    assert_eq!(parse_size("8MiB").unwrap(), 8 * 1024 * 1024);
    // Test-created files only; no user path cleanup.
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn bounded_codec_and_checker_fuzz() {
    let (p, s) = formatted();
    let original = s.durable();
    let mut cases = 0;
    use std::io::Write;
    let mut corpus = std::env::var_os("SUNFS_EVIDENCE_DIR").map(|dir| {
        std::io::BufWriter::new(
            std::fs::File::create(std::path::PathBuf::from(dir).join("fuzz-corpus.jsonl")).unwrap(),
        )
    });
    for seed in [0u64, 1, 2, 0x534653] {
        let mut random = seed.wrapping_add(0x9e3779b97f4a7c15);
        for index in 0..512 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let mut bytes = original.to_vec();
            let block = if index % 2 == 0 {
                (random % 5) as usize
            } else {
                p.dependencies[(random as usize) % p.dependencies.len()].block as usize
            };
            let offset = ((random >> 17) as usize) % PAGE;
            bytes[block * PAGE + offset] ^= ((random >> 32) as u8) | 1;
            if index % 3 == 0 {
                stamp(&mut bytes[block * PAGE..(block + 1) * PAGE]);
            }
            let result = std::panic::catch_unwind(|| {
                let decode = Page::decode(&bytes[block * PAGE..(block + 1) * PAGE], block as u64);
                let report = check(&mut Bytes(&bytes));
                (decode, report)
            });
            match result {
                Ok((_decoded, r)) => {
                    if r.status == Status::Clean {
                        assert_eq!(
                            bytes, original,
                            "mutation accepted as clean seed={seed} input={index}"
                        );
                    }
                    if let Some(out) = corpus.as_mut() {
                        writeln!(out,"{{\"seed\":{seed},\"index\":{index},\"kind\":\"image-mutation\",\"block\":{block},\"offset\":{offset},\"mutated_page_crc64\":\"{:016x}\",\"classification\":\"{}\",\"panic\":false}}",crc64(&bytes[block*PAGE..(block+1)*PAGE]),r.status.name()).unwrap();
                    }
                    assert!(matches!(
                        r.status,
                        Status::Clean
                            | Status::DegradedRecoverable
                            | Status::Corrupt
                            | Status::Unsupported
                            | Status::Indeterminate
                    ));
                }
                Err(e) => {
                    let name = format!("/tmp/sunfs-fuzz-seed-{seed}-input-{index}.img");
                    std::fs::write(&name, &bytes).unwrap();
                    panic!("seed={seed} input={index} block={block} offset={offset} reproducer={name} panic={e:?}");
                }
            }
            cases += 1;
        }
        // Fully arbitrary pages plus all eight typed payload decoders.
        for index in 0..512 {
            let mut bytes = [0; PAGE];
            for b in &mut bytes {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                *b = random as u8;
            }
            let result = std::panic::catch_unwind(|| {
                let classification = format!("{:?}", Page::decode(&bytes, 0).map(|_| ()));
                for kind in 1..=8 {
                    let _ = Payload::decode(kind, &bytes[..(random as usize) % 4001]);
                }
                classification
            });
            if let Some(out) = corpus.as_mut() {
                writeln!(out,"{{\"seed\":{seed},\"index\":{index},\"kind\":\"arbitrary-page\",\"input_crc64\":\"{:016x}\",\"classification\":\"{}\",\"panic\":{}}}",crc64(&bytes),result.as_ref().map(|s|s.as_str()).unwrap_or("PANIC"),result.is_err()).unwrap();
            }
            assert!(result.is_ok(), "seed={seed} arbitrary_input={index}");
            cases += 1;
        }
    }
    println!(
        "SFS1_FUZZ cases={cases} seeds=0,1,2,5457491 checker_images=2048 arbitrary_pages=2048"
    );
}

/// Independently assembled nonempty fixture, not an engine create/write claim.
/// Exercises file data, a directory binding and a committed unlinked orphan.
fn nonempty_fixture() -> (Bootstrap, Vec<u8>) {
    let (mut p, s) = formatted();
    let start = p.geometry.segment_start;
    assert_eq!(p.dependencies.len(), 8);
    let reference = |index: u64| BlockRef {
        block: start + index,
        epoch: 1,
        crc: 0,
    };
    let file = NodeId {
        nid: 2,
        incarnation: 1,
    };
    let orphan = NodeId {
        nid: 3,
        incarnation: 1,
    };
    let mut data = [0; PAGE];
    data[..3].copy_from_slice(b"abc");
    let extent = Extent {
        logical: 0,
        physical: start + 15,
        epoch: 1,
        content_generation: 1,
        blocks: 1,
        flags: 0,
        data_crc: data_crc([0x53; 16], &file, 0, 1, 1, &data).unwrap(),
    };
    let desc = |index: u64, first: u64, last: u64, count: u32| Descriptor {
        first,
        last,
        page: reference(index),
        count,
        ..Default::default()
    };
    let mut add = |index: u64, payload: Payload| {
        p.dependencies.push(Page {
            uuid: [0x53; 16],
            block: start + index,
            epoch: 1,
            birth: 1,
            payload,
        })
    };
    add(
        8,
        Payload::Inode(Inode {
            id: file.clone(),
            kind: 1,
            mode: 0o600,
            nlink: 1,
            size: 3,
            content_generation: 1,
            extents: reference(9),
            ..Default::default()
        }),
    );
    add(
        9,
        Payload::Root {
            kind: 5,
            count: 1,
            entry_bytes: 48,
            pages: vec![desc(10, 0, 0, 1)],
        },
    );
    add(
        10,
        Payload::Extents {
            owner: file.clone(),
            entries: vec![extent],
        },
    );
    add(
        11,
        Payload::Directory {
            owner: NodeId::ROOT,
            ordinal: 0,
            entries: vec![DirEntry {
                target: file.clone(),
                kind: 1,
                name: b"file".to_vec(),
            }],
        },
    );
    add(
        12,
        Payload::Inode(Inode {
            id: orphan.clone(),
            kind: 1,
            mode: 0o600,
            nlink: 0,
            content_generation: 1,
            extents: reference(13),
            ..Default::default()
        }),
    );
    add(13, empty_root(5, 48));
    add(
        14,
        Payload::Table {
            first: 3,
            last: 3,
            records: Records::Orphans(vec![Orphan {
                id: orphan.clone(),
                unlink_generation: 1,
                reason: 1,
                ..Default::default()
            }]),
        },
    );
    if let Payload::Inode(i) = &mut p.dependencies[0].payload {
        i.size = 40;
    }
    p.dependencies[1].payload = Payload::Root {
        kind: 4,
        count: 1,
        entry_bytes: 0,
        pages: vec![desc(11, 0, 0, 1)],
    };
    if let Payload::Table {
        last,
        records: Records::Map(v),
        ..
    } = &mut p.dependencies[2].payload
    {
        *last = 3;
        v.push(MapEntry {
            id: file.clone(),
            node: reference(8),
            kind: 1,
            ..Default::default()
        });
        v.push(MapEntry {
            id: orphan.clone(),
            node: reference(12),
            kind: 1,
            ..Default::default()
        });
    }
    p.dependencies[3].payload = Payload::Root {
        kind: 1,
        count: 3,
        entry_bytes: 48,
        pages: vec![desc(2, 1, 3, 3)],
    };
    p.dependencies[4].payload = Payload::Root {
        kind: 3,
        count: 1,
        entry_bytes: 32,
        pages: vec![desc(14, 3, 3, 1)],
    };
    if let Payload::Table {
        records: Records::Segments(v),
        ..
    } = &mut p.dependencies[6].payload
    {
        v[0].live = 16;
        v[0].next_hint = 16;
        v[0].bitmap[0] = 255;
        v[0].bitmap[1] = 255;
    }
    let mut summaries = Vec::new();
    for page in &p.dependencies {
        let (owner, table) = match &page.payload {
            Payload::Inode(i) => (i.id.clone(), 0),
            Payload::Directory { owner, .. } | Payload::Extents { owner, .. } => (owner.clone(), 0),
            Payload::Root { kind, .. } => (
                if *kind >= 4 {
                    if page.block == start + 1 {
                        NodeId::ROOT
                    } else if page.block == start + 9 {
                        file.clone()
                    } else {
                        orphan.clone()
                    }
                } else {
                    NodeId::default()
                },
                *kind,
            ),
            Payload::Table { records, .. } => (NodeId::default(), records.kind()),
            _ => (NodeId::default(), 0),
        };
        let kind = page.payload.page_type();
        summaries.push(SummaryEntry {
            offset: (page.block - start) as u32,
            kind,
            owner,
            logical_key: ((kind as u64) << 48) | ((table as u64) << 32),
            content_generation: 1,
            expected_crc: 0,
            ..Default::default()
        });
    }
    summaries.push(SummaryEntry {
        offset: 15,
        kind: 0,
        owner: file,
        logical_key: 0,
        content_generation: 1,
        expected_crc: data_crc(
            [0x53; 16],
            &NodeId {
                nid: 2,
                incarnation: 1,
            },
            0,
            1,
            1,
            &data,
        )
        .unwrap(),
        ..Default::default()
    });
    if let Payload::Summary { entries, .. } = &mut p.dependencies[7].payload {
        *entries = summaries;
    }
    for cp in &mut p.checkpoints {
        if let Payload::Checkpoint(c) = &mut cp.payload {
            c.next_nid = 4;
            c.live_nodes = 3;
            c.orphan_nodes = 1;
            c.allocated_blocks = 16;
            c.free_blocks = p.geometry.segment_count * 256 - 16;
            c.transaction_blocks = 16;
        }
    }
    let mut bytes = s.durable().to_vec();
    for page in p.dependencies.iter().chain(&p.checkpoints) {
        let a = page.block as usize * PAGE;
        bytes[a..a + PAGE].copy_from_slice(&page.encode().unwrap());
    }
    let a = (start + 15) as usize * PAGE;
    bytes[a..a + PAGE].copy_from_slice(&data);
    relink(&mut bytes, &p);
    (p, bytes)
}
#[test]
fn checker_nonempty_graph_and_corruptions() {
    let (p, original) = nonempty_fixture();
    let r = check(&mut Bytes(&original));
    assert_eq!(r.status, Status::Clean, "{r:#?}");
    let start = p.geometry.segment_start;
    if let Some(dir) = std::env::var_os("SUNFS_EVIDENCE_DIR") {
        std::fs::write(
            std::path::PathBuf::from(dir).join("nonempty.img"),
            &original,
        )
        .unwrap();
    }
    for mutation in 0..9 {
        let mut bytes = original.clone();
        match mutation {
            0 => encoded_mutate(&mut bytes, start + 11, |p| {
                if let Payload::Directory { entries, .. } = &mut p.payload {
                    entries[0].target.incarnation = 2;
                }
            }),
            1 => encoded_mutate(&mut bytes, start + 11, |p| {
                if let Payload::Directory { entries, .. } = &mut p.payload {
                    entries[0].target = NodeId::ROOT;
                    entries[0].kind = 2;
                }
            }),
            2 => encoded_mutate(&mut bytes, start + 10, |p| {
                if let Payload::Extents { entries, .. } = &mut p.payload {
                    entries[0].physical = 3;
                }
            }),
            3 => encoded_mutate(&mut bytes, start + 10, |p| {
                if let Payload::Extents { entries, .. } = &mut p.payload {
                    entries[0].physical = start + 8;
                }
            }),
            4 => encoded_mutate(&mut bytes, start + 14, |p| {
                if let Payload::Table {
                    records: Records::Orphans(v),
                    ..
                } = &mut p.payload
                {
                    v[0].id.incarnation = 2;
                }
            }),
            5 => encoded_mutate(&mut bytes, start + 14, |p| {
                if let Payload::Table {
                    records: Records::Orphans(v),
                    ..
                } = &mut p.payload
                {
                    v[0].unlink_generation = 3;
                }
            }),
            6 => encoded_mutate(&mut bytes, start + 12, |p| {
                if let Payload::Inode(i) = &mut p.payload {
                    i.nlink = 1;
                }
            }),
            7 => bytes[(start + 15) as usize * PAGE] ^= 1,
            _ => encoded_mutate(&mut bytes, start + 10, |p| {
                if let Payload::Extents { entries, .. } = &mut p.payload {
                    entries[0].epoch = 2;
                }
            }),
        }
        relink(&mut bytes, &p);
        let r = check(&mut Bytes(&bytes));
        assert_eq!(
            r.status,
            Status::Corrupt,
            "nonempty mutation {mutation}: {r:#?}"
        );
    }
}
