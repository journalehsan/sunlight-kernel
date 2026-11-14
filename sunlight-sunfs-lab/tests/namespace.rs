//! SFS-3 hosted lifetime and crash oracles. Cold-media traversal is independent.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    rc::Rc,
    time::Instant,
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
    let g = Geometry::calculate(Parameters::laboratory((mib * 1024 * 1024) as u64, bps)).unwrap();
    let mut sim = Simulator::new(mib * 1024 * 1024, 0x53465333).unwrap();
    Bootstrap::plan(g, [0x33; 16])
        .unwrap()
        .publish(&mut sim)
        .unwrap();
    Device(Rc::new(RefCell::new(sim)))
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
fn save(name: &str, bytes: &[u8]) {
    if let Some(root) = std::env::var_os("SUNFS_SFS3_EVIDENCE_DIR") {
        let p = PathBuf::from(root).join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Object {
    nid: u64,
    kind: u8,
    bytes: Vec<u8>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    paths: BTreeMap<Vec<Vec<u8>>, Object>,
    orphans: BTreeMap<u64, (Object, u64)>,
}
fn path(s: &str) -> Vec<Vec<u8>> {
    s.split('/').map(|c| c.as_bytes().to_vec()).collect()
}
fn object(nid: u64, kind: u8, bytes: &[u8]) -> Object {
    Object {
        nid,
        kind,
        bytes: bytes.to_vec(),
    }
}
fn data(bytes: &[u8], n: &sunlight_sunfs_check::CheckedNode) -> Vec<u8> {
    let mut out = vec![0; n.size as usize];
    if n.kind == 2 {
        return vec![];
    }
    for e in &n.extents {
        let a = e.logical as usize * PAGE;
        let len = PAGE.min(out.len() - a);
        let p = e.physical as usize * PAGE;
        out[a..a + len].copy_from_slice(&bytes[p..p + len]);
    }
    out
}
fn observed(bytes: &[u8], r: &Report, generation: u64) -> State {
    let g = r
        .generations
        .iter()
        .find(|g| g.generation == generation)
        .unwrap();
    let mut result = State::default();
    let mut stack = vec![(1, vec![])];
    let mut seen = BTreeSet::new();
    while let Some((nid, prefix)) = stack.pop() {
        assert!(seen.insert(nid));
        let n = g.nodes.iter().find(|n| n.id.nid == nid).unwrap();
        assert_eq!(n.id.incarnation, 1);
        for e in &n.entries {
            let mut p = prefix.clone();
            p.push(e.name.clone());
            let child = g.nodes.iter().find(|n| n.id == e.target).unwrap();
            assert!(result
                .paths
                .insert(
                    p.clone(),
                    object(child.id.nid, child.kind, &data(bytes, child))
                )
                .is_none());
            if e.kind == 2 {
                stack.push((e.target.nid, p));
            }
        }
    }
    for o in &g.orphans {
        let n = g.nodes.iter().find(|n| n.id == o.id).unwrap();
        assert_eq!(n.nlink, 0);
        assert!(result
            .orphans
            .insert(
                o.id.nid,
                (
                    object(n.id.nid, n.kind, &data(bytes, n)),
                    o.unlink_generation
                )
            )
            .is_none());
    }
    assert_eq!(result.paths.len() + result.orphans.len() + 1, g.nodes.len());
    result
}
fn checked(bytes: &[u8], expected: &State) -> Report {
    let r = check(&mut Bytes(bytes));
    assert!(
        matches!(r.status, Status::Clean | Status::DegradedRecoverable),
        "{r:#?}"
    );
    assert_eq!(&observed(bytes, &r, r.selected.unwrap()), expected);
    r
}
fn agreement(d: &Device, expected: &State) -> Filesystem<Device> {
    let bytes = durable(d);
    let r = checked(&bytes, expected);
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
    let union: BTreeSet<_> = r
        .generations
        .iter()
        .flat_map(|g| g.blocks.iter().copied())
        .collect();
    let Payload::Superblock(sb) = page(&bytes, 0).payload else {
        panic!()
    };
    let segments: BTreeSet<_> = union
        .iter()
        .map(|b| (b - sb.segment_start) / sb.segment_blocks as u64)
        .collect();
    let a = fs.accounting();
    assert_eq!(a.protected_blocks, union.len() as u64);
    assert_eq!(a.protected_segments, segments.len() as u64);
    let placed = segments.len() as u64 * sb.segment_blocks as u64;
    assert_eq!(a.placement_overhead_blocks, placed - union.len() as u64);
    assert_eq!(
        a.available_blocks,
        sb.segment_count * sb.segment_blocks as u64 - placed
    );
    assert_eq!(a.recovery_reserve_blocks, sb.cleaner_reserve);
    let current_blocks = r.generations.last().unwrap().blocks.len() as u64;
    assert_eq!(a.current_checkpoint_blocks, current_blocks);
    assert_eq!(a.retained_only_blocks, union.len() as u64 - current_blocks);
    assert_eq!(a.pending_reserved_blocks, 0);
    assert_eq!(
        a.reclaimable_blocks,
        a.available_blocks.saturating_sub(sb.cleaner_reserve)
    );
    for (components, obj) in &expected.paths {
        let c: Vec<_> = components.iter().map(Vec::as_slice).collect();
        let id = fs.lookup_components(&c).unwrap();
        assert_eq!(id.node.nid, obj.nid);
        if obj.kind == 1 {
            let mut out = vec![0; obj.bytes.len() + 1];
            assert_eq!(fs.read(&id, 0, &mut out).unwrap(), obj.bytes.len());
            assert_eq!(&out[..obj.bytes.len()], &obj.bytes);
        }
    }
    assert_eq!(fs.runtime_accounting().orphan_nodes, expected.orphans.len());
    assert_eq!(
        fs.runtime_accounting().reap_candidates,
        expected.orphans.len()
    );
    assert_eq!(durable(d), bytes);
    fs
}
fn page(bytes: &[u8], b: u64) -> Page {
    Page::decode(&bytes[b as usize * PAGE..(b as usize + 1) * PAGE], b).unwrap()
}
fn base_state() -> State {
    let mut s = State::default();
    for (p, o) in [
        ("a", object(2, 2, b"")),
        ("b", object(3, 2, b"")),
        ("a/source", object(4, 1, b"source bytes")),
        ("b/dest", object(5, 1, b"destination bytes")),
        ("empty", object(6, 2, b"")),
    ] {
        s.paths.insert(path(p), o);
    }
    s
}
fn fixture(bps: u32) -> Vec<u8> {
    let d = fresh(bps, 16);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let root = fs.root();
    let a = fs.mkdir(&root, b"a").unwrap();
    let b = fs.mkdir(&root, b"b").unwrap();
    let source = fs.create_at(&a, b"source").unwrap();
    fs.write(&source, 0, b"source bytes").unwrap();
    let dest = fs.create_at(&b, b"dest").unwrap();
    fs.write(&dest, 0, b"destination bytes").unwrap();
    fs.mkdir(&root, b"empty").unwrap();
    fs.sync_all().unwrap();
    drop(fs);
    checked(&durable(&d), &base_state());
    durable(&d)
}
#[test]
fn hierarchy_dot_type_depth_and_entry_bounds() {
    for bps in [64, 256, 512] {
        let d = load(&fixture(bps), 0);
        let mut fs = agreement(&d, &base_state());
        let root = fs.root();
        let a = fs.lookup(b"a").unwrap();
        let source = fs.lookup_at(&a, b"source").unwrap();
        assert_eq!(
            fs.lookup_components(&[b"a", b".", b"..", b"b", b"..", b".."]),
            Ok(root.clone())
        );
        assert_eq!(
            fs.lookup_components(&[b"missing", b".."]),
            Err(FsError::NotFound)
        );
        assert_eq!(fs.lookup_at(&source, b".."), Err(FsError::NotDirectory));
        assert_eq!(fs.create_at(&source, b"x"), Err(FsError::NotDirectory));
        for name in [b".".as_slice(), b"..", b"a/b", b"", b"a\0b", b"\xff"] {
            assert!(fs.create_at(&a, name).is_err());
        }
        assert!(fs.create_at(&a, &[b'x'; 65]).is_err());
        assert!(fs.mkdir(&a, b"source").is_err());
        assert_eq!(fs.rmdir(&root, b"a"), Err(FsError::NotEmpty));
        assert_eq!(fs.unlink(&root, b"a"), Err(FsError::IsDirectory));
        assert_eq!(fs.rmdir(&a, b"source"), Err(FsError::NotDirectory));
        assert_eq!(
            fs.rmdir(&root, b"."),
            Err(FsError::Invalid(sunlight_sunfs::Error::Structure))
        );
        let mut dir = a;
        for depth in 2..=MAX_DEPTH {
            dir = fs.mkdir(&dir, format!("d{depth}").as_bytes()).unwrap();
        }
        assert_eq!(fs.create_at(&dir, b"too-deep"), Err(FsError::Limit));
        assert_eq!(
            fs.lookup_components(&vec![b".".as_slice(); MAX_COMPONENTS + 1]),
            Err(FsError::Limit)
        );
        assert_eq!(
            fs.rename(&root, b"b", &dir, b"too-deep"),
            Err(FsError::Limit)
        );
        let b = fs.lookup(b"b").unwrap();
        for i in 0..15 {
            fs.mkdir(&b, format!("e{i}").as_bytes())
                .unwrap_or_else(|e| {
                    assert_eq!(e, FsError::Limit);
                    root.clone()
                });
        }
        fs.sync_all().unwrap();
        drop(fs);
        let bytes = durable(&d);
        let r = check(&mut Bytes(&bytes));
        assert_eq!(r.status, Status::Clean);
        Filesystem::mount(d).unwrap();
    }
    let d = fresh(256, 16);
    let mut fs = Filesystem::mount(d).unwrap();
    let root = fs.root();
    for i in 0..MAX_ENTRIES {
        fs.create(format!("f{i}").as_bytes()).unwrap();
    }
    assert_eq!(fs.mkdir(&root, b"entry-limit"), Err(FsError::Limit));
}
#[test]
fn handles_dup_access_offsets_and_stale_sessions() {
    let d = load(&fixture(256), 0);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let id = fs.lookup_components(&[b"a", b"source"]).unwrap();
    let h = fs.open(&id, AccessMode::ReadWrite).unwrap();
    let dup = fs.dup(&h).unwrap();
    let independent = fs.open(&id, AccessMode::Read).unwrap();
    assert_eq!(fs.read_handle(&h, &mut [0; 2]).unwrap(), 2);
    assert_eq!(fs.position(&dup), Ok(2));
    assert_eq!(fs.position(&independent), Ok(0));
    assert_eq!(fs.write_handle(&independent, b"x"), Err(FsError::Access));
    fs.close(&h).unwrap();
    assert_eq!(fs.close(&h), Err(FsError::Stale));
    assert_eq!(fs.position(&h), Err(FsError::Stale));
    fs.seek(&dup, 0).unwrap();
    fs.write_handle(&dup, b"SOURCE").unwrap();
    fs.fsync_handle(&dup).unwrap();
    let w = fs.open(&id, AccessMode::Write).unwrap();
    assert_eq!(fs.read_handle(&w, &mut [0; 1]), Err(FsError::Access));
    fs.close(&w).unwrap();
    fs.close(&dup).unwrap();
    fs.close(&independent).unwrap();
    assert_eq!(fs.runtime_accounting().descriptions, 0);
    let new = fs.open(&id, AccessMode::Read).unwrap();
    assert_eq!(fs.position(&h), Err(FsError::Stale));
    drop(fs);
    loss(&d);
    let mut fs = Filesystem::mount(d).unwrap();
    assert_eq!(fs.read_handle(&new, &mut [0; 1]), Err(FsError::Stale));
    let id = fs.lookup_components(&[b"a", b"source"]).unwrap();
    let h = fs.open(&id, AccessMode::Read).unwrap();
    let mut all = vec![h.clone()];
    for _ in 1..MAX_HANDLES {
        all.push(fs.dup(&h).unwrap());
    }
    assert_eq!(fs.dup(&h), Err(FsError::Limit));
    for h in all {
        fs.close(&h).unwrap();
    }
    assert_eq!(fs.runtime_accounting().handles, 0);
    let mut independent = vec![];
    for _ in 0..MAX_DESCRIPTIONS {
        independent.push(fs.open(&id, AccessMode::Read).unwrap());
    }
    assert_eq!(fs.open(&id, AccessMode::Read), Err(FsError::Limit));
}
#[test]
fn open_unlink_name_reuse_last_close_and_retained_closures() {
    for bps in [64, 256, 512] {
        let d = fresh(bps, 16);
        let mut fs = Filesystem::mount(d.clone()).unwrap();
        let root = fs.root();
        let a = fs.create(b"A").unwrap();
        fs.write(&a, 0, b"original").unwrap();
        fs.sync_all().unwrap();
        let h = fs.open(&a, AccessMode::ReadWrite).unwrap();
        let dup = fs.dup(&h).unwrap();
        let other = fs.open(&a, AccessMode::Read).unwrap();
        fs.unlink(&root, b"A").unwrap();
        assert_eq!(fs.lookup(b"A"), Err(FsError::NotFound));
        assert_eq!(fs.read(&a, 0, &mut [0; 8]), Err(FsError::Stale));
        let b = fs.create(b"A").unwrap();
        assert_ne!(a.node, b.node);
        fs.write(&b, 0, b"replacement").unwrap();
        let mut out = [0; 8];
        assert_eq!(fs.read_handle(&h, &mut out), Ok(8));
        assert_eq!(&out, b"original");
        fs.seek(&dup, 0).unwrap();
        fs.write_handle(&dup, b"ORIGINAL").unwrap();
        fs.fsync_handle(&dup).unwrap();
        assert_eq!(fs.runtime_accounting().pinned_orphans, 1);
        assert_eq!(fs.reap_orphans(), Ok(0));
        let before = fs.accounting().protected_blocks;
        let old_bytes = durable(&d);
        let old_report = check(&mut Bytes(&old_bytes));
        fs.close(&h).unwrap();
        fs.close(&dup).unwrap();
        assert_eq!(fs.reap_orphans(), Ok(0));
        fs.close(&other).unwrap();
        assert_eq!(fs.close(&other), Err(FsError::Stale));
        assert_eq!(fs.accounting().protected_blocks, before);
        assert_eq!(fs.runtime_accounting().reap_candidates, 1);
        assert_eq!(fs.reap_orphans(), Ok(1));
        assert_eq!(fs.reap_orphans(), Ok(0));
        let bytes = durable(&d);
        let r = check(&mut Bytes(&bytes));
        assert_eq!(r.status, Status::Clean);
        assert!(r.generations.last().unwrap().orphans.is_empty());
        assert_eq!(r.generations[0].orphans.len(), 1);
        for g in &old_report.generations {
            if r.generations.iter().any(|n| n.generation == g.generation) {
                for block in &g.blocks {
                    let a = *block as usize * PAGE;
                    assert_eq!(&old_bytes[a..a + PAGE], &bytes[a..a + PAGE]);
                }
            }
        }
        drop(fs);
        loss(&d);
        let mut expected = State::default();
        expected
            .paths
            .insert(path("A"), object(b.node.nid, 1, b"replacement"));
        let mut fs = agreement(&d, &expected);
        assert_eq!(fs.read_handle(&h, &mut [0; 1]), Err(FsError::Stale));
        let c = fs.create(b"next").unwrap();
        assert!(c.node.nid > b.node.nid);
    }
}
#[test]
fn rename_open_replacement_directory_cycles_and_types() {
    let d = load(&fixture(256), 0);
    let mut fs = agreement(&d, &base_state());
    let root = fs.root();
    let a = fs.lookup(b"a").unwrap();
    let b = fs.lookup(b"b").unwrap();
    let source = fs.lookup_at(&a, b"source").unwrap();
    let dest = fs.lookup_at(&b, b"dest").unwrap();
    let hs = fs.open(&source, AccessMode::Read).unwrap();
    let hd = fs.open(&dest, AccessMode::Read).unwrap();
    fs.rename(&a, b"source", &b, b"dest").unwrap();
    assert_eq!(fs.lookup_at(&a, b"source"), Err(FsError::NotFound));
    assert_eq!(fs.lookup_at(&b, b"dest"), Ok(source.clone()));
    assert_eq!(fs.handle_identity(&hd).unwrap().1, dest.node);
    let mut out = [0; 17];
    assert_eq!(fs.read_handle(&hd, &mut out), Ok(17));
    assert_eq!(&out, b"destination bytes");
    assert_eq!(fs.handle_identity(&hs).unwrap().1, source.node);
    fs.sync_all().unwrap();
    let mut expected = base_state();
    expected.paths.remove(&path("a/source"));
    expected
        .paths
        .insert(path("b/dest"), object(4, 1, b"source bytes"));
    expected
        .orphans
        .insert(5, (object(5, 1, b"destination bytes"), 4));
    checked(&durable(&d), &expected);
    let child = fs.mkdir(&a, b"child").unwrap();
    assert_eq!(
        fs.rename(&root, b"a", &child, b"cycle"),
        Err(FsError::Cycle)
    );
    assert_eq!(fs.rename(&root, b"a", &root, b"b"), Err(FsError::NotEmpty));
    assert_eq!(
        fs.rename(&b, b"dest", &root, b"empty"),
        Err(FsError::IsDirectory)
    );
    assert_eq!(
        fs.rename(&root, b"empty", &b, b"dest"),
        Err(FsError::NotDirectory)
    );
    fs.rename(&root, b"a", &root, b"empty").unwrap();
    assert_eq!(fs.lookup_at(&child, b".."), Ok(a.clone()));
    assert_eq!(fs.lookup(b"empty"), Ok(a.clone()));
    fs.rename(&root, b"empty", &b, b"moved").unwrap();
    assert_eq!(fs.lookup_at(&a, b".."), Ok(b));
    fs.sync_all().unwrap();
    let bytes = durable(&d);
    assert_eq!(check(&mut Bytes(&bytes)).status, Status::Clean);
    Filesystem::mount(load(&bytes, 0)).unwrap();
}
#[test]
fn recovery_orphans_are_eligible_readonly_never_reaps_and_exhaustion_is_safe() {
    let d = load(&fixture(256), 0);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let a = fs.lookup(b"a").unwrap();
    let id = fs.lookup_at(&a, b"source").unwrap();
    let h = fs.open(&id, AccessMode::Read).unwrap();
    fs.unlink(&a, b"source").unwrap();
    fs.sync_all().unwrap();
    assert_eq!(fs.runtime_accounting().pinned_orphans, 1);
    drop(fs);
    loss(&d);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    assert_eq!(fs.runtime_accounting().reap_candidates, 1);
    assert_eq!(fs.position(&h), Err(FsError::Stale));
    assert_eq!(fs.reap_orphans(), Ok(1));
    drop(fs);
    loss(&d);
    assert_eq!(
        Filesystem::mount(d.clone())
            .unwrap()
            .runtime_accounting()
            .orphan_nodes,
        0
    );
    let bytes = fixture(256);
    let d = load(&bytes, 0);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let root = fs.root();
    fs.rmdir(&root, b"empty").unwrap();
    fs.sync_all().unwrap();
    drop(fs);
    d.0.borrow_mut().set_read_only(true);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let before = durable(&d);
    assert_eq!(fs.reap_orphans(), Err(FsError::ReadOnly));
    assert_eq!(durable(&d), before);
    // Lower persistent cap is enforced before unlink mutates either binding.
    let mut bytes = fixture(256);
    for block in [0, 1] {
        let mut p = page(&bytes, block);
        let Payload::Superblock(sb) = &mut p.payload else {
            panic!()
        };
        sb.maximum_orphans = 1;
        put(&mut bytes, &p);
    }
    let d = load(&bytes, 0);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let root = fs.root();
    fs.rmdir(&root, b"empty").unwrap();
    let a = fs.lookup(b"a").unwrap();
    assert_eq!(fs.unlink(&a, b"source"), Err(FsError::Limit));
    assert!(fs.lookup_at(&a, b"source").is_ok());
    assert_eq!(
        fs.rename(&a, b"source", &fs.lookup(b"b").unwrap(), b"dest"),
        Err(FsError::Limit)
    );
    assert!(fs.lookup_at(&a, b"source").is_ok());
}
fn put(bytes: &mut [u8], p: &Page) -> BlockRef {
    let a = p.block as usize * PAGE;
    bytes[a..a + PAGE].copy_from_slice(&p.encode().unwrap());
    p.reference().unwrap()
}
fn orphan_source(mut state: State, generation: u64) -> State {
    let obj = state.paths.remove(&path("a/source")).unwrap();
    state.orphans.insert(4, (obj, generation));
    state
}
fn prepare(
    d: &Device,
    tag: &str,
    previous: &State,
) -> (Filesystem<Device>, State, bool, Vec<Handle>) {
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let mut expected = previous.clone();
    let mut handles = vec![];
    let root = fs.root();
    let a = fs.lookup(b"a").unwrap();
    let b = fs.lookup(b"b").unwrap();
    let g = fs.generation() + 1;
    let reap = tag.ends_with("reap");
    match tag {
        "mkdir" => {
            let n = fs.mkdir(&a, b"new").unwrap();
            assert_eq!(n.node.nid, 7);
            expected.paths.insert(path("a/new"), object(7, 2, b""));
        }
        "create" => {
            let n = fs.create_at(&a, b"new").unwrap();
            assert_eq!(n.node.nid, 7);
            fs.write(&n, 0, b"created").unwrap();
            expected
                .paths
                .insert(path("a/new"), object(7, 1, b"created"));
        }
        "write" => {
            let n = fs.lookup_at(&a, b"source").unwrap();
            fs.write(&n, 0, b"SOURCE bytes").unwrap();
            expected.paths.get_mut(&path("a/source")).unwrap().bytes = b"SOURCE bytes".to_vec();
        }
        "rename-same" | "rename-cross" => {
            let to = if tag == "rename-same" { &a } else { &b };
            fs.rename(&a, b"source", to, b"renamed").unwrap();
            let n = expected.paths.remove(&path("a/source")).unwrap();
            expected.paths.insert(
                path(if tag == "rename-same" {
                    "a/renamed"
                } else {
                    "b/renamed"
                }),
                n,
            );
        }
        "replace" | "three-replace" => {
            let dest = fs.lookup_at(&b, b"dest").unwrap();
            let h = fs.open(&dest, AccessMode::Read).unwrap();
            let dup = fs.dup(&h).unwrap();
            fs.close(&h).unwrap();
            let source = fs.lookup_at(&a, b"source").unwrap();
            let reader = fs.open(&source, AccessMode::Read).unwrap();
            assert_eq!(fs.read_handle(&reader, &mut [0; 1]), Ok(1));
            fs.rename(&a, b"source", &b, b"dest").unwrap();
            let mut out = [0; 17];
            assert_eq!(fs.read_handle(&dup, &mut out), Ok(17));
            assert_eq!(&out, b"destination bytes");
            assert_eq!(fs.read_handle(&reader, &mut [0; 1]), Ok(1));
            handles.extend([dup, reader]);
            let replacement = expected.paths.remove(&path("a/source")).unwrap();
            let old = expected.paths.insert(path("b/dest"), replacement).unwrap();
            expected.orphans.insert(5, (old, g));
        }
        "directory-move" => {
            fs.rename(&root, b"a", &b, b"moved").unwrap();
            let dir = expected.paths.remove(&path("a")).unwrap();
            let file = expected.paths.remove(&path("a/source")).unwrap();
            expected.paths.insert(path("b/moved"), dir);
            expected.paths.insert(path("b/moved/source"), file);
        }
        "directory-replace" => {
            fs.rename(&root, b"a", &root, b"empty").unwrap();
            let old = expected.paths.remove(&path("empty")).unwrap();
            expected.orphans.insert(6, (old, g));
            let dir = expected.paths.remove(&path("a")).unwrap();
            let file = expected.paths.remove(&path("a/source")).unwrap();
            expected.paths.insert(path("empty"), dir);
            expected.paths.insert(path("empty/source"), file);
        }
        "unlink" | "orphan-write" | "last-close-reap" => {
            let n = fs.lookup_at(&a, b"source").unwrap();
            let h = fs.open(&n, AccessMode::ReadWrite).unwrap();
            let dup = fs.dup(&h).unwrap();
            assert_eq!(fs.read_handle(&h, &mut [0; 1]), Ok(1));
            fs.unlink(&a, b"source").unwrap();
            assert_eq!(fs.lookup_at(&a, b"source"), Err(FsError::NotFound));
            fs.seek(&dup, 0).unwrap();
            let mut out = [0; 12];
            assert_eq!(fs.read_handle(&dup, &mut out), Ok(12));
            assert_eq!(&out, b"source bytes");
            expected = orphan_source(expected, g);
            if tag == "orphan-write" {
                fs.seek(&h, 0).unwrap();
                fs.write_handle(&h, b"ORPHAN bytes").unwrap();
                expected.orphans.get_mut(&4).unwrap().0.bytes = b"ORPHAN bytes".to_vec();
            }
            if tag == "last-close-reap" {
                fs.sync_all().unwrap();
                fs.close(&h).unwrap();
                assert_eq!(fs.reap_orphans(), Ok(0));
                fs.close(&dup).unwrap();
                expected.orphans.clear();
            } else {
                handles.extend([h, dup]);
            }
        }
        "rmdir" => {
            fs.rmdir(&root, b"empty").unwrap();
            let old = expected.paths.remove(&path("empty")).unwrap();
            expected.orphans.insert(6, (old, g));
        }
        "recovery-reap" | "three-reap" => {
            expected.orphans.clear();
        }
        _ => panic!("unknown scenario {tag}"),
    }
    (fs, expected, reap, handles)
}
fn perform(fs: &mut Filesystem<Device>, reap: bool) -> Result<()> {
    if reap {
        fs.reap_orphans().map(|_| ())
    } else {
        fs.sync_all()
    }
}
fn protection(before: &[u8], after: &[u8], initial: &Report, plan: &Admission) {
    let protected: BTreeSet<_> = initial
        .generations
        .iter()
        .flat_map(|g| g.blocks.iter().copied())
        .collect();
    assert_eq!(
        plan.blocks.iter().copied().collect::<BTreeSet<_>>().len(),
        plan.blocks.len()
    );
    for b in &plan.blocks {
        assert!(!protected.contains(b));
    }
    for b in protected {
        let a = b as usize * PAGE;
        assert_eq!(&before[a..a + PAGE], &after[a..a + PAGE], "early reuse {b}");
    }
}
#[test]
fn namespace_d_p_r_r0_crash_fault_matrix() {
    let linked = fixture(64);
    let d = load(&linked, 0);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let a = fs.lookup(b"a").unwrap();
    let n = fs.lookup_at(&a, b"source").unwrap();
    let h = fs.open(&n, AccessMode::Read).unwrap();
    fs.unlink(&a, b"source").unwrap();
    fs.sync_all().unwrap();
    assert_eq!(fs.position(&h), Ok(0));
    drop(fs);
    let orphan = durable(&d);
    let orphan_state = orphan_source(base_state(), 4);
    // Preserve an interrupted retirement with three independently valid closures.
    fn three(bytes: &[u8]) -> Vec<u8> {
        let d = load(bytes, 0);
        let mut fs = Filesystem::mount(d.clone()).unwrap();
        let id = fs.lookup_components(&[b"b", b"dest"]).unwrap();
        fs.write(&id, 0, b"destination bytes").unwrap();
        let first = next(&d);
        let p = fs.admission().unwrap();
        d.0.borrow_mut()
            .faults
            .insert(first + p.dependency_blocks + 3, Fault::WriteFailure);
        assert!(fs.sync_all().is_err());
        drop(fs);
        let bytes = durable(&d);
        assert_eq!(check(&mut Bytes(&bytes)).generations.len(), 3);
        bytes
    }
    let three_linked = three(&linked);
    let three_orphan = three(&orphan);
    for tag in [
        "mkdir",
        "create",
        "write",
        "rename-same",
        "rename-cross",
        "replace",
        "directory-move",
        "directory-replace",
        "unlink",
        "orphan-write",
        "rmdir",
        "last-close-reap",
        "recovery-reap",
        "three-replace",
        "three-reap",
    ] {
        let base = match tag {
            "recovery-reap" => &orphan,
            "three-replace" => &three_linked,
            "three-reap" => &three_orphan,
            _ => &linked,
        };
        let previous = if tag == "recovery-reap" || tag == "three-reap" {
            &orphan_state
        } else {
            &base_state()
        };
        campaign(base, previous, tag);
    }
}
fn campaign(base: &[u8], previous: &State, tag: &str) {
    let dry = load(base, 0x53465333);
    let (mut fs, new, reap, _) = prepare(&dry, tag, previous);
    let before = durable(&dry);
    let old = if tag == "last-close-reap" {
        orphan_source(previous.clone(), 4)
    } else {
        previous.clone()
    };
    let initial = checked(&before, &old);
    let old_generation = initial.selected.unwrap();
    let new_generation = old_generation + 1;
    // Reap plans omit candidates, so inspect the actual successful emitted set
    // below rather than using the unreaped admission preview as the sole oracle.
    let start = next(&dry);
    perform(&mut fs, reap).unwrap();
    let events: Vec<_> = dry
        .0
        .borrow()
        .trace
        .iter()
        .filter(|e| e.id >= start)
        .cloned()
        .collect();
    let blocks: Vec<_> = events
        .iter()
        .filter(|e| e.operation == "write" && e.offset >= 5 * PAGE as u64)
        .map(|e| e.offset / PAGE as u64)
        .collect();
    let plan = Admission {
        blocks,
        ..Default::default()
    };
    let cp_index = events
        .iter()
        .position(|e| {
            e.operation == "write"
                && (2 * PAGE as u64..=4 * PAGE as u64).contains(&e.offset)
                && e.digest != 0
        })
        .unwrap();
    let p_flush = cp_index + 1;
    assert_eq!(events[p_flush].operation, "flush");
    checked(&durable(&dry), &new);
    save(&format!("{tag}-before.img"), &before);
    save(&format!("{tag}-committed.img"), &durable(&dry));
    let mut rows = String::new();
    let mut count = 0;
    for admission in [false, true] {
        let d = load(base, 0x53465333);
        let (fs, _, _, _) = prepare(&d, tag, previous);
        if admission {
            fs.admission().unwrap();
        }
        drop(fs);
        loss(&d);
        agreement(&d, &old);
        rows.push_str(&format!("{{\"case\":\"{tag}-admission-{admission}\",\"expected\":[{old_generation}],\"selected\":{old_generation},\"D\":true,\"P\":true,\"R\":true,\"R0\":true,\"status\":\"PASS\"}}\n"));
        count += 1;
    }
    for (i, event) in events.iter().enumerate() {
        let mut faults = vec![
            ("power-before".to_string(), Some(Fault::PowerLoss), false),
            ("power-after".into(), None, true),
        ];
        if event.operation == "write" {
            faults.push(("write-fail".into(), Some(Fault::WriteFailure), false));
            for n in [0, 1, 511, 512, 2048, 4095, 4096] {
                faults.push((format!("partial-{n}"), Some(Fault::Partial(n)), false));
            }
            for (start, len) in [(1, 511), (512, 512), (2048, 2048)] {
                faults.push((
                    format!("torn-{start}-{len}"),
                    Some(Fault::Torn { start, len }),
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
        } else {
            assert_eq!(event.operation, "flush");
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
            let case = format!("{tag}-seed-1397117747-event-{i}-{label}");
            if std::env::var("SUNFS_SFS3_CASE").is_ok_and(|wanted| wanted != case) {
                continue;
            }
            let d = load(base, 0x53465333);
            let (mut fs, _, reap, handles) = prepare(&d, tag, previous);
            let target = next(&d) + i as u64;
            if let Some(f) = fault.clone() {
                d.0.borrow_mut().faults.insert(target, f);
            }
            if after {
                d.0.borrow_mut().crash_after = Some(target);
            }
            let result = perform(&mut fs, reap);
            let acknowledged = result.is_ok();
            if result.is_err() {
                assert_eq!(fs.sync_all(), Err(FsError::ReadOnly));
            }
            let expected = if i < cp_index {
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
            if acknowledged {
                assert_eq!(expected, new_generation);
            }
            drop(fs);
            loss(&d);
            let bytes = durable(&d);
            let verify = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let state = if expected == new_generation {
                    &new
                } else {
                    &old
                };
                let r = checked(&bytes, state);
                assert_eq!(r.selected, Some(expected), "{case}");
                protection(&before, &bytes, &initial, &plan);
                let mut mounted = agreement(&d, state);
                for h in &handles {
                    assert_eq!(mounted.position(h), Err(FsError::Stale));
                }
                for g in &r.generations {
                    assert_eq!(
                        g.blocks.iter().copied().collect::<BTreeSet<_>>().len(),
                        g.blocks.len()
                    );
                }
                if mounted.state() == MountState::CleanWritable && !state.orphans.is_empty() {
                    assert_eq!(mounted.reap_orphans().unwrap(), state.orphans.len());
                    drop(mounted);
                    loss(&d);
                    let mut reaped = state.clone();
                    reaped.orphans.clear();
                    agreement(&d, &reaped);
                }
            }));
            save(
                &format!("traces/{case}.trace"),
                format!("{:?}", d.0.borrow().trace).as_bytes(),
            );
            if let Err(panic) = verify {
                save(&format!("FAILED-{case}.img"), &bytes);
                std::panic::resume_unwind(panic);
            }
            let r = check(&mut Bytes(&bytes));
            rows.push_str(&format!("{{\"case\":\"{case}\",\"seed\":1397117747,\"event\":{target},\"operation\":\"{}\",\"fault\":\"{fault:?}\",\"acknowledged\":{acknowledged},\"expected\":[{expected}],\"selected\":{},\"classification\":\"{}\",\"D\":true,\"P\":true,\"R\":true,\"R0\":true,\"status\":\"PASS\"}}\n",event.operation,r.selected.unwrap(),r.status.name()));
            count += 1;
        }
    }
    save(&format!("crash-{tag}.jsonl"), rows.as_bytes());
    println!(
        "SFS3_CRASH scenario={tag} cases={count} events={}",
        events.len()
    );
}
#[test]
fn metadata_costs_and_repeated_namespace_churn() {
    let d = load(&fixture(256), 0);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let root = fs.root();
    let a = fs.lookup(b"a").unwrap();
    let b = fs.lookup(b"b").unwrap();
    let id = fs.lookup_at(&a, b"source").unwrap();
    let h = fs.open(&id, AccessMode::Read).unwrap();
    let mut rows = String::new();
    let mut peak = fs.accounting().protected_blocks;
    let begin = Instant::now();
    for _ in 0..10_000 {
        assert_eq!(fs.lookup_components(&[b"a", b"source"]).unwrap(), id);
    }
    let traversal = begin.elapsed().as_nanos();
    for operation in ["rename", "unlink", "retirement"] {
        match operation {
            "rename" => fs.rename(&a, b"source", &b, b"new").unwrap(),
            "unlink" => fs.unlink(&b, b"new").unwrap(),
            _ => fs.close(&h).unwrap(),
        }
        let before_bytes = durable(&d);
        let before_report = check(&mut Bytes(&before_bytes));
        let before_union: BTreeSet<_> = before_report
            .generations
            .iter()
            .flat_map(|g| g.blocks.iter().copied())
            .collect();
        let before = next(&d);
        let started = Instant::now();
        if operation == "retirement" {
            assert_eq!(fs.reap_orphans(), Ok(1));
        } else {
            fs.fsync(&root).unwrap();
        }
        let elapsed = started.elapsed().as_nanos();
        let bytes = durable(&d);
        let report = check(&mut Bytes(&bytes));
        let current = report.generations.last().unwrap();
        let data: BTreeSet<_> = current
            .nodes
            .iter()
            .flat_map(|n| n.extents.iter().map(|e| e.physical * PAGE as u64))
            .collect();
        let sim = d.0.borrow();
        let events: Vec<_> = sim.trace.iter().filter(|e| e.id >= before).collect();
        let written: usize = events
            .iter()
            .filter(|e| e.operation == "write")
            .map(|e| e.length)
            .sum();
        let data_bytes: usize = events
            .iter()
            .filter(|e| e.operation == "write" && data.contains(&e.offset))
            .map(|e| e.length)
            .sum();
        let flushes = events.iter().filter(|e| e.operation == "flush").count();
        // The new private closure coexisted with every pre-commit closure at
        // flush P, before the oldest slot was durably retired. Reconstruct that
        // transient peak from independent physical block lists, not final counters.
        assert!(current.blocks.iter().all(|b| !before_union.contains(b)));
        let transient_union = before_union.len() + current.blocks.len();
        peak = peak.max(transient_union as u64);
        rows.push_str(&format!("{{\"operation\":\"{operation}\",\"written_bytes\":{written},\"metadata_bytes_including_cp_retirement\":{},\"data_bytes\":{data_bytes},\"flushes\":{flushes},\"elapsed_ns\":{elapsed},\"protected_blocks\":{},\"orphan_records\":{},\"orphan_record_bytes\":32,\"orphan_root_bytes\":4096,\"orphan_page_bytes_when_nonempty\":4096,\"lookup_components\":2,\"lookup_iterations\":10000,\"lookup_elapsed_ns\":{traversal},\"peak_retained_blocks\":{peak},\"transient_retained_union_blocks\":{transient_union}}}\n",written-data_bytes,fs.accounting().protected_blocks,fs.runtime_accounting().orphan_nodes));
    }
    save("performance.jsonl", rows.as_bytes());
    // Many more than three slot rotations, followed by cold remount each round.
    drop(fs);
    let mut expected = base_state();
    expected.paths.remove(&path("a/source"));
    for i in 0..24 {
        loss(&d);
        let mut fs = agreement(&d, &expected);
        let a = fs.lookup(b"a").unwrap();
        let id = fs.create_at(&a, b"churn").unwrap();
        fs.write(&id, 0, &[i]).unwrap();
        let h = fs.open(&id, AccessMode::Read).unwrap();
        fs.rename(&a, b"churn", &a, b"renamed").unwrap();
        fs.unlink(&a, b"renamed").unwrap();
        fs.sync_all().unwrap();
        fs.close(&h).unwrap();
        fs.reap_orphans().unwrap();
        assert!(id.node.nid >= 7 + i as u64);
    }
    loss(&d);
    agreement(&d, &expected);
}
fn corrupt_namespace(bytes: &mut [u8], kind: &str) {
    let report = check(&mut Bytes(bytes));
    let latest = report.generations.last().unwrap();
    let mut checkpoint = page(bytes, latest.slot);
    let Payload::Checkpoint(cp) = &mut checkpoint.payload else {
        panic!()
    };
    if kind.starts_with("orphan-") {
        let mut root = page(bytes, cp.orphan_table.block);
        let Payload::Root { pages, .. } = &mut root.payload else {
            panic!()
        };
        let mut table = page(bytes, pages[0].page.block);
        let Payload::Table {
            records: Records::Orphans(records),
            ..
        } = &mut table.payload
        else {
            panic!()
        };
        match kind {
            "orphan-future" => records[0].unlink_generation = cp.generation + 1,
            "orphan-incarnation" => records[0].id.incarnation += 1,
            _ => records[0].id.nid = 6,
        }
        // Rewriting first/last keys makes the table locally valid for unmapped id.
        if kind == "orphan-wrong-node" {
            if let Payload::Table { first, last, .. } = &mut table.payload {
                *first = 6;
                *last = 6;
            }
            pages[0].first = 6;
            pages[0].last = 6;
        }
        pages[0].page = put(bytes, &table);
        cp.orphan_table = put(bytes, &root);
    } else {
        let mut root = page(bytes, cp.node_map.block);
        let Payload::Root { pages, .. } = &mut root.payload else {
            panic!()
        };
        let mut table = page(bytes, pages[0].page.block);
        let Payload::Table {
            records: Records::Map(maps),
            ..
        } = &mut table.payload
        else {
            panic!()
        };
        let nid = if kind == "duplicate-name" || kind == "cycle" {
            1
        } else {
            2
        };
        let m = maps.iter_mut().find(|m| m.id.nid == nid).unwrap();
        let mut node = page(bytes, m.node.block);
        let Payload::Inode(n) = &mut node.payload else {
            panic!()
        };
        match kind {
            "parent" => n.parent = n.id.clone(),
            "nlink" => n.nlink += 1,
            "cycle" | "duplicate-name" => {
                let mut dirroot = page(bytes, n.directory.block);
                let Payload::Root { pages, .. } = &mut dirroot.payload else {
                    panic!()
                };
                let mut dir = page(bytes, pages[0].page.block);
                if kind == "cycle" {
                    let Payload::Directory { entries, .. } = &mut dir.payload else {
                        panic!()
                    };
                    entries[0].target = NodeId::ROOT;
                    pages[0].page = put(bytes, &dir);
                } else {
                    // a and b have equal encoded width. Change b to a directly,
                    // resealing CRC without running the encoder's sorted check.
                    let mut raw = dir.encode().unwrap();
                    let first_name = HEADER + 32 + 32;
                    let second_name = first_name + 40;
                    assert_eq!(raw[second_name], b'b');
                    raw[second_name] = b'a';
                    raw[64..72].fill(0);
                    let crc = crc64(&raw);
                    raw[64..72].copy_from_slice(&crc.to_le_bytes());
                    let at = dir.block as usize * PAGE;
                    bytes[at..at + PAGE].copy_from_slice(&raw);
                    pages[0].page = BlockRef {
                        block: dir.block,
                        epoch: dir.epoch,
                        crc,
                    };
                }
                n.directory = put(bytes, &dirroot);
            }
            _ => panic!(),
        }
        m.node = put(bytes, &node);
        pages[0].page = put(bytes, &table);
        cp.node_map = put(bytes, &root);
    }
    put(bytes, &checkpoint);
}
#[test]
fn crc_valid_directory_and_orphan_corruptions_fallback_without_writes() {
    let bytes = fixture(256);
    let d = load(&bytes, 0);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let a = fs.lookup(b"a").unwrap();
    let id = fs.lookup_at(&a, b"source").unwrap();
    let _h = fs.open(&id, AccessMode::Read).unwrap();
    fs.unlink(&a, b"source").unwrap();
    fs.sync_all().unwrap();
    drop(fs);
    let bytes = durable(&d);
    let mut rows = String::new();
    for kind in [
        "parent",
        "nlink",
        "cycle",
        "duplicate-name",
        "orphan-future",
        "orphan-incarnation",
        "orphan-wrong-node",
    ] {
        let mut bad = bytes.clone();
        corrupt_namespace(&mut bad, kind);
        let d = load(&bad, 0);
        let mut fs = agreement(&d, &base_state());
        assert_eq!(fs.state(), MountState::DegradedReadOnly);
        assert_eq!(fs.reap_orphans(), Err(FsError::ReadOnly));
        assert_eq!(durable(&d), bad);
        save(&format!("corrupt-{kind}.img"), &bad);
        rows.push_str(&format!(
            "{{\"case\":\"{kind}\",\"crc_resealed\":true,\"selected\":3,\"status\":\"PASS\"}}\n"
        ));
    }
    save("corruption.jsonl", rows.as_bytes());
}
#[test]
fn low_space_reap_failure_preserves_working_orphans_and_reservations() {
    let d = fresh(256, 16);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let root = fs.root();
    let a = fs.create(b"a").unwrap();
    fs.write(&a, 0, b"old").unwrap();
    fs.sync_all().unwrap();
    fs.unlink(&root, b"a").unwrap();
    fs.sync_all().unwrap();
    drop(fs);
    let mut bytes = durable(&d);
    let report = check(&mut Bytes(&bytes));
    let union: BTreeSet<_> = report
        .generations
        .iter()
        .flat_map(|g| g.blocks.iter().map(|b| (*b - 256) / 256))
        .collect();
    // Raise the configured reserve to all but one available block. No user
    // transaction may dip into it, including a metadata-only orphan retirement.
    for block in [0, 1] {
        let mut p = page(&bytes, block);
        let Payload::Superblock(sb) = &mut p.payload else {
            panic!()
        };
        sb.cleaner_reserve = (sb.segment_count - union.len() as u64) * 256 - 1;
        put(&mut bytes, &p);
    }
    let d = load(&bytes, 0);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let before = next(&d);
    let accounting = fs.accounting();
    assert_eq!(fs.reap_orphans(), Err(FsError::NoSpace));
    assert_eq!(next(&d), before);
    assert_eq!(fs.runtime_accounting().orphan_nodes, 1);
    assert_eq!(fs.runtime_accounting().reap_candidates, 1);
    assert_eq!(
        fs.accounting().protected_blocks,
        accounting.protected_blocks
    );
    assert_eq!(durable(&d), bytes);
}
#[test]
fn reordered_namespace_dependencies_preserve_previous_complete_state() {
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
            let result = self.inner.write(o, b)?;
            self.writes += 1;
            Ok(result)
        }
        fn flush(&mut self) -> IoResult<()> {
            for index in (0..self.writes).rev().take(self.cut) {
                self.inner.0.borrow_mut().persist(index, 512, 512)?;
                self.inner.0.borrow_mut().persist(index, 0, PAGE)?;
            }
            self.inner.0.borrow_mut().power_loss();
            Err(IoFailure::uncertain(IoKind::Flush))
        }
    }
    let linked = fixture(64);
    let d = load(&linked, 0);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let a = fs.lookup(b"a").unwrap();
    fs.unlink(&a, b"source").unwrap();
    fs.sync_all().unwrap();
    drop(fs);
    let orphan = durable(&d);
    let mut rows = String::new();
    let mut total = 0;
    for tag in ["rename-cross", "unlink", "recovery-reap"] {
        let (base, expected) = if tag == "recovery-reap" {
            (&orphan, orphan_source(base_state(), 4))
        } else {
            (&linked, base_state())
        };
        let initial = checked(base, &expected);
        let dry = load(base, 0);
        let (mut fs, _, reap, _) = prepare(&dry, tag, &expected);
        let first = next(&dry);
        perform(&mut fs, reap).unwrap();
        let blocks: Vec<_> = dry
            .0
            .borrow()
            .trace
            .iter()
            .filter(|e| e.id >= first && e.operation == "write" && e.offset >= 5 * PAGE as u64)
            .map(|e| e.offset / PAGE as u64)
            .collect();
        let plan = Admission {
            blocks,
            ..Default::default()
        };
        for cut in 0..=plan.blocks.len() {
            let d = load(base, cut as u64);
            let mut fs = Filesystem::mount(Reorder {
                inner: d.clone(),
                writes: 0,
                cut,
            })
            .unwrap();
            let a = fs.lookup(b"a").unwrap();
            let result = match tag {
                "rename-cross" => {
                    let b = fs.lookup(b"b").unwrap();
                    fs.rename(&a, b"source", &b, b"new").unwrap();
                    fs.sync_all()
                }
                "unlink" => {
                    let id = fs.lookup_at(&a, b"source").unwrap();
                    let _h = fs.open(&id, AccessMode::Read).unwrap();
                    fs.unlink(&a, b"source").unwrap();
                    fs.sync_all()
                }
                _ => fs.reap_orphans().map(|_| ()),
            };
            assert!(result.is_err());
            drop(fs);
            loss(&d);
            agreement(&d, &expected);
            protection(base, &durable(&d), &initial, &plan);
            save(
                &format!("traces/reorder-{tag}-{cut}.trace"),
                format!("{:?}", d.0.borrow().trace).as_bytes(),
            );
            let generation = initial.selected.unwrap();
            rows.push_str(&format!("{{\"scenario\":\"{tag}\",\"reverse_prefix\":{cut},\"expected\":[{generation}],\"selected\":{generation},\"D\":true,\"P\":true,\"R\":true,\"R0\":true,\"status\":\"PASS\"}}\n"));
            total += 1;
        }
    }
    save("reordering.jsonl", rows.as_bytes());
    println!("SFS3_REORDER cases={total}");
}
#[test]
fn maximum_bounded_namespace_and_full_orphan_table() {
    let d = fresh(256, 16);
    let mut fs = Filesystem::mount(d.clone()).unwrap();
    let root = fs.root();
    let mut dirs = vec![];
    for i in 0..MAX_DIRECTORIES {
        dirs.push(fs.mkdir(&root, format!("d{i:02}").as_bytes()).unwrap());
    }
    assert_eq!(fs.mkdir(&root, b"over"), Err(FsError::Limit));
    let mut ids = vec![];
    let mut names = vec![];
    for i in 0..MAX_FILES {
        let name = format!("{i:064}").into_bytes();
        let id = fs.create_at(&dirs[0], &name).unwrap();
        fs.write(&id, 0, &[i as u8; PAGE]).unwrap();
        ids.push(id);
        names.push(name);
    }
    let h = fs.open(&ids[0], AccessMode::Read).unwrap();
    fs.sync_all().unwrap();
    let root_entries = fs.entries_at(&root).unwrap();
    assert_eq!(root_entries.len(), MAX_DIRECTORIES);
    assert!(root_entries.iter().all(|e| e.kind == 2));
    assert!(root_entries.windows(2).all(|e| e[0].name < e[1].name));
    let file_entries = fs.entries_at(&dirs[0]).unwrap();
    assert_eq!(file_entries.len(), MAX_FILES);
    for (entry, id) in file_entries.iter().zip(&ids) {
        assert_eq!(entry.id, *id);
        assert_eq!(entry.kind, 1);
        assert_eq!(entry.size, PAGE as u64);
    }
    assert_eq!(fs.create_at(&dirs[1], b"file-limit"), Err(FsError::Limit));
    for name in &names {
        fs.unlink(&dirs[0], name).unwrap();
    }
    fs.sync_all().unwrap();
    let bytes = durable(&d);
    let r = check(&mut Bytes(&bytes));
    assert_eq!(r.status, Status::Clean);
    let g = r.generations.last().unwrap();
    assert_eq!(g.orphans.len(), 16);
    assert_eq!(g.nodes.len(), 33);
    assert_eq!(fs.rmdir(&root, b"d00"), Err(FsError::Limit));
    assert_eq!(fs.reap_orphans(), Ok(15));
    assert_eq!(fs.runtime_accounting().pinned_orphans, 1);
    fs.seek(&h, 0).unwrap();
    assert_eq!(fs.read_handle(&h, &mut [0; PAGE]), Ok(PAGE));
    fs.close(&h).unwrap();
    assert_eq!(fs.reap_orphans(), Ok(1));
    drop(fs);
    loss(&d);
    let fs = Filesystem::mount(d.clone()).unwrap();
    assert_eq!(fs.runtime_accounting().namespace_nodes, 17);
    assert_eq!(fs.runtime_accounting().orphan_nodes, 0);
    let bytes = durable(&d);
    assert_eq!(check(&mut Bytes(&bytes)).status, Status::Clean);
}
