use sunlight_sunfs::{bootstrap::Bootstrap, codec::*, geometry::*, Error};
fn plan() -> Bootstrap {
    Bootstrap::plan(
        Geometry::calculate(Parameters::laboratory(16 * 1024 * 1024, 256)).unwrap(),
        [7; 16],
    )
    .unwrap()
}
fn page(payload: Payload) -> Page {
    Page {
        uuid: [1; 16],
        block: 256,
        epoch: 1,
        birth: 1,
        payload,
    }
}
fn rec<T: Field + std::fmt::Debug + PartialEq>(value: T) -> Vec<u8> {
    let mut e = Encoder::new();
    e.field(&value).unwrap();
    let b = e.finish();
    let mut d = Decoder::new(&b).unwrap();
    assert_eq!(T::read(&mut d).unwrap(), value);
    d.end().unwrap();
    b
}
fn reseal(b: &mut [u8; PAGE]) {
    b[64..72].fill(0);
    let crc = crc64(b);
    b[64..72].copy_from_slice(&crc.to_le_bytes());
}
#[test]
fn crc_known_answers() {
    assert_eq!(crc64(b"123456789"), 0x6c40df5f0b497347);
    assert_eq!(crc64(b""), 0);
    assert_eq!(crc64(&[0; 4096]), 0);
}
#[test]
fn bootstrap_all_page_roundtrips() {
    let p = plan();
    for v in p
        .superblocks
        .iter()
        .chain(&p.dependencies)
        .chain(&p.checkpoints)
    {
        let b = v.encode().unwrap();
        assert_eq!(Page::decode(&b, v.block).unwrap(), *v);
    }
}
#[test]
fn record_roundtrips_and_endian() {
    let b = rec(NodeId {
        nid: 0x0102030405060708,
        incarnation: u64::MAX,
    });
    assert_eq!(&b[..8], &[8, 7, 6, 5, 4, 3, 2, 1]);
    assert_ne!(&b[..8], &0x0102030405060708u64.to_be_bytes());
    rec(BlockRef {
        block: u64::MAX,
        epoch: 1,
        crc: u64::MAX,
    });
    rec(DiskTime {
        seconds: i64::MIN,
        nanos: 999999999,
        ..Default::default()
    });
    rec(Features {
        compatible: u64::MAX,
        readonly: u64::MAX,
        incompatible: 3,
    });
    rec(Extent {
        logical: (1 << 28) - 1,
        physical: u64::MAX,
        epoch: u64::MAX,
        content_generation: u64::MAX,
        blocks: 1,
        flags: 0,
        data_crc: u64::MAX,
    });
    rec(DirEntry {
        target: NodeId::ROOT,
        kind: 1,
        name: vec![b'x'; 255],
    });
    rec(Orphan {
        id: NodeId::ROOT,
        unlink_generation: u64::MAX,
        reason: 1,
        ..Default::default()
    });
    rec(SummaryEntry {
        offset: 511,
        kind: 0,
        owner: NodeId::ROOT,
        content_generation: u64::MAX,
        expected_crc: u64::MAX,
        ..Default::default()
    });
}
#[test]
fn unused_structure_roundtrips() {
    let id = NodeId::ROOT;
    let reference = BlockRef {
        block: 256,
        epoch: 1,
        crc: 1,
    };
    let ext = Extent {
        logical: 0,
        physical: 300,
        epoch: 1,
        content_generation: 1,
        blocks: 1,
        ..Default::default()
    };
    let orphan = Orphan {
        id: id.clone(),
        unlink_generation: 1,
        reason: 1,
        ..Default::default()
    };
    let payloads = vec![
        Payload::Directory {
            owner: id.clone(),
            ordinal: 0,
            entries: vec![DirEntry {
                target: id.clone(),
                kind: 1,
                name: b"alpha".to_vec(),
            }],
        },
        Payload::Extents {
            owner: id.clone(),
            entries: vec![ext],
        },
        Payload::Table {
            first: 1,
            last: 1,
            records: Records::Orphans(vec![orphan]),
        },
        Payload::Inode(Inode {
            id: id.clone(),
            kind: 1,
            mode: 0o600,
            nlink: 0,
            size: 1,
            content_generation: 1,
            extents: reference,
            ..Default::default()
        }),
    ];
    for payload in payloads {
        let p = page(payload);
        assert_eq!(Page::decode(&p.encode().unwrap(), 256).unwrap(), p);
    }
}
#[test]
fn independent_python_golden_pages() {
    for (bytes, address, kind, len) in [
        (&include_bytes!("fixtures/superblock.bin")[..], 0, 1, 136),
        (&include_bytes!("fixtures/inode.bin")[..], 256, 5, 200),
        (&include_bytes!("fixtures/directory.bin")[..], 258, 6, 72),
        (
            &include_bytes!("fixtures/empty-orphan-root.bin")[..],
            259,
            3,
            24,
        ),
    ] {
        let p = Page::decode(bytes, address).unwrap();
        assert_eq!(p.payload.page_type(), kind);
        assert_eq!(p.payload.encode().unwrap().len(), len);
        assert_eq!(p.encode().unwrap().as_slice(), bytes);
    }
}
#[test]
fn header_negatives() {
    let original = plan().superblocks[0].encode().unwrap();
    for (offset, value, expected) in [
        (0, 0, Error::Magic),
        (10, 3, Error::Unsupported),
        (12, 0, Error::Structure),
        (14, 1, Error::Reserved),
        (60, 1, Error::Reserved),
        (72, 1, Error::Reserved),
        (4000, 1, Error::Reserved),
    ] {
        let mut b = original;
        b[offset] = value;
        reseal(&mut b);
        assert_eq!(Page::decode(&b, 0), Err(expected));
    }
    let mut b = original;
    b[64] ^= 1;
    assert_eq!(Page::decode(&b, 0), Err(Error::Checksum));
    let mut b = original;
    b[56..60].copy_from_slice(&u32::MAX.to_le_bytes());
    reseal(&mut b);
    assert_eq!(Page::decode(&b, 0), Err(Error::Bounds));
    assert_eq!(Page::decode(&original, 1), Err(Error::Reference));
    for len in [0, 1, 95, 96, 4095] {
        assert_eq!(Page::decode(&original[..len], 0), Err(Error::Bounds));
    }
}
#[test]
fn payload_negatives() {
    // RETIRED has no current owners; non-FREE segments have been allocated.
    assert!(Segment {
        index: 0,
        epoch: 1,
        state: 3,
        live: 1,
        next_hint: 1,
        summary: BlockRef {
            block: 256,
            epoch: 1,
            crc: 1
        },
        bitmap: vec![1, 0, 0, 0, 0, 0, 0, 0]
    }
    .validate()
    .is_err());
    assert!(Segment {
        index: 0,
        epoch: 0,
        state: 1,
        live: 0,
        next_hint: 0,
        summary: BlockRef::default(),
        bitmap: vec![0; 8]
    }
    .validate()
    .is_err());

    let original = plan().superblocks[0].encode().unwrap();
    for offset in [96, 98, 100, 102, 120] {
        let mut b = original;
        b[offset] = 0xff;
        reseal(&mut b);
        assert_eq!(Page::decode(&b, 0), Err(Error::Unsupported));
    }
    let mut b = original;
    b[228] = 1;
    reseal(&mut b);
    assert_eq!(Page::decode(&b, 0), Err(Error::Reserved));
    for name in [
        b"".to_vec(),
        b".".to_vec(),
        b"..".to_vec(),
        b"a/b".to_vec(),
        vec![0],
        vec![255],
        vec![b'x'; 256],
    ] {
        assert!(DirEntry {
            target: NodeId::ROOT,
            kind: 1,
            name
        }
        .validate()
        .is_err());
    }
    assert!(NodeId {
        nid: 0,
        incarnation: 1
    }
    .validate()
    .is_err());
    assert!(BlockRef {
        block: 0,
        epoch: 1,
        crc: 0
    }
    .validate()
    .is_err());
    assert!(DiskTime {
        seconds: 0,
        nanos: 1_000_000_000,
        ..Default::default()
    }
    .validate()
    .is_err());
    let mut b = page(empty_root(1, 48)).encode().unwrap();
    b[100..104].copy_from_slice(&u32::MAX.to_le_bytes());
    reseal(&mut b);
    assert_eq!(Page::decode(&b, 256), Err(Error::Bounds));
}
#[test]
fn malformed_record_lengths_and_references() {
    let dir = DirEntry {
        target: NodeId::ROOT,
        kind: 1,
        name: b"x".to_vec(),
    };
    let mut bytes = rec(dir);
    for length in [0, 1, 39, 41, u16::MAX] {
        bytes[20..22].copy_from_slice(&length.to_le_bytes());
        assert!(DirEntry::read(&mut Decoder::new(&bytes).unwrap()).is_err());
    }
    assert!(Extent {
        blocks: 2,
        ..Default::default()
    }
    .validate()
    .is_err());
    let mut inode = Inode {
        id: NodeId::ROOT,
        kind: 2,
        mode: 0o755,
        nlink: 2,
        content_generation: 1,
        parent: NodeId::ROOT,
        directory: BlockRef {
            block: 256,
            epoch: 1,
            crc: 0,
        },
        ..Default::default()
    };
    inode.atime.seconds = 1;
    assert!(inode.validate().is_err());
    inode.time_valid = 1;
    assert!(inode.validate().is_ok());
}
#[test]
fn geometry_boundaries() {
    for bps in [64, 256, 512] {
        let g = Geometry::calculate(Parameters::laboratory(16 * 1024 * 1024, bps)).unwrap();
        assert!(!g.contains(4));
        assert!(!g.contains(g.segment_start - 1));
        assert!(g.contains(g.segment_start));
        assert!(!g.contains(g.segment_end()));
        let mut p = Parameters::laboratory(g.minimum_blocks * 4096, bps);
        assert!(Geometry::calculate(p).is_ok());
        p.capacity = (g.minimum_blocks - 1) * 4096;
        assert!(Geometry::calculate(p).is_err());
        p = Parameters::laboratory(16 * 1024 * 1024, bps);
        p.cleaner_reserve = 1;
        assert_eq!(Geometry::calculate(p), Err(Error::Reserve));
    }
    for capacity in [0, 512, 4096, 65536, u64::MAX] {
        assert!(Geometry::calculate(Parameters::laboratory(capacity, 256)).is_err());
    }
    let mut p = Parameters::laboratory(16 * 1024 * 1024, 256);
    p.maximum_nodes = 6725;
    assert_eq!(Geometry::calculate(p), Err(Error::Geometry));
    p = Parameters::laboratory(16 * 1024 * 1024, 256);
    p.checkpoint_slots = 2;
    assert_eq!(Geometry::calculate(p), Err(Error::Geometry));
}
#[test]
fn geometry_properties_and_fixed_point() {
    let mut accepted = 0;
    for bps in [64, 256, 512] {
        for segments in 1..=500 {
            if let Ok(g) = Geometry::calculate(Parameters::laboratory(
                (segments + 1) * bps as u64 * 4096,
                bps,
            )) {
                accepted += 1;
                assert!(g.bootstrap_blocks + g.cleaner_reserve <= g.segment_count * bps as u64);
                assert_eq!(g.segment_start, bps as u64);
                assert_eq!(
                    g.segment_table_pages,
                    g.segment_count.div_ceil(3976 / g.segment_record_bytes)
                );
                assert_eq!(
                    Bootstrap::plan(g, [1; 16]).unwrap().dependencies.len() as u64,
                    g.bootstrap_blocks
                );
            }
        }
    }
    assert!(accepted > 1400);
    for base in [1, 63, 64, 65, 81, 82, 83, 255, 256, 511, 512, 513, 4096] {
        for bps in [64, 256, 512] {
            let (s, _) = summary_fixed_point(base, bps).unwrap();
            let total = base + s;
            assert_eq!(
                s,
                (total / bps) * bps.div_ceil(82) + (total % bps).div_ceil(82)
            );
        }
    }
}

#[test]
fn geometry_limits_and_devices() {
    for bps in [64, 256, 512] {
        let maximum = 82 * (3976 / (64 + bps as u64 / 8));
        let g = Geometry::calculate(Parameters::laboratory(
            (maximum + 1) * bps as u64 * 4096,
            bps,
        ))
        .unwrap();
        assert_eq!(g.segment_count, maximum);
        assert_eq!(g.segment_table_pages, 82);
        assert_eq!(
            Geometry::calculate(Parameters::laboratory(
                (maximum + 2) * bps as u64 * 4096,
                bps
            )),
            Err(Error::Budget)
        );
        let mut p = Parameters::laboratory(16 * 1024 * 1024, bps);
        p.logical_block = 4096;
        assert!(Geometry::calculate(p).is_ok());
        p.logical_block = 1000;
        assert_eq!(Geometry::calculate(p), Err(Error::Geometry));
    }
    assert_eq!(summary_fixed_point(u64::MAX, 64), Err(Error::Overflow));
    assert_eq!(ceil(u64::MAX, 4096), Err(Error::Overflow));
    assert_eq!(ceil(1, 0), Err(Error::Geometry));
}
