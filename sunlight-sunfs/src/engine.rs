//! Bounded, exclusive-owner root filesystem. All mutations are volatile until fsync.
//! No native adapter, locks, host APIs, implicit sync on drop, or automatic repair.
use crate::{
    block::*,
    codec::*,
    geometry::{Geometry, Parameters},
    Error,
};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec,
    vec::Vec,
};
use core::sync::atomic::{AtomicU64, Ordering};
// Runtime handles must never acquire authority in a different mount session.
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
mod recovery;
pub use recovery::MountState;
pub const MAX_FILES: usize = 16;
pub const MAX_NAME: usize = 64;
pub const MAX_FILE_BYTES: usize = 16 * PAGE;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsError {
    Invalid(Error),
    Io(IoFailure),
    Corrupt,
    Indeterminate,
    ReadOnly,
    NotFound,
    Exists,
    Stale,
    Limit,
    NoSpace,
}
impl From<Error> for FsError {
    fn from(e: Error) -> Self {
        Self::Invalid(e)
    }
}
impl From<IoFailure> for FsError {
    fn from(e: IoFailure) -> Self {
        Self::Io(e)
    }
}
pub type Result<T> = core::result::Result<T, FsError>;
fn ensure(v: bool) -> Result<()> {
    if v {
        Ok(())
    } else {
        Err(FsError::Corrupt)
    }
}
/// Mount-scoped handle. The public UUID/node tuple is the persistent identity;
/// reacquire a handle through lookup after remount, even for the same identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileId {
    pub volume: [u8; 16],
    pub node: NodeId,
    session: u64,
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: Vec<u8>,
    pub id: FileId,
    pub size: u64,
}
#[derive(Clone)]
struct File {
    name: Vec<u8>,
    id: NodeId,
    data: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Owner {
    epoch: u64,
    kind: u16,
    node: NodeId,
    key: u64,
    birth: u64,
    crc: u64,
}
#[derive(Clone)]
struct Snapshot {
    slot: u64,
    cp: Checkpoint,
    owned: BTreeMap<u64, Owner>,
    files: Vec<File>,
}
#[derive(Clone, Debug, Default)]
pub struct Accounting {
    pub protected_blocks: u64,
    pub protected_segments: u64,
    pub placement_overhead_blocks: u64,
    pub available_blocks: u64,
    pub recovery_reserve_blocks: u64,
}
#[derive(Clone, Debug, Default)]
pub struct Admission {
    pub generation: u64,
    pub dependency_blocks: u64,
    pub data_blocks: u64,
    pub metadata_blocks: u64,
    pub reserved_segment_blocks: u64,
    pub placement_overhead_blocks: u64,
    pub encoded_bytes: usize,
    pub blocks: Vec<u64>,
}
struct Plan {
    admission: Admission,
    dependencies: Vec<(u64, [u8; PAGE])>,
    snapshot: Snapshot,
    checkpoint: [u8; PAGE],
}
/// Owns the device exclusively. Errors during persistent I/O latch read-only until
/// drop/remount; reservations and uncertain ownership remain held by this owner.
pub struct Filesystem<D: BlockDevice> {
    device: D,
    session: u64,
    info: DeviceInfo,
    uuid: [u8; 16],
    sb: Superblock,
    geometry: Geometry,
    state: MountState,
    retained: Vec<Snapshot>,
    files: Vec<File>,
    next_nid: u64,
    dirty: bool,
    frozen: bool,
    quarantine: Option<Plan>,
}
impl<D: BlockDevice> Filesystem<D> {
    pub fn mount(mut device: D) -> Result<Self> {
        let info = device.info();
        info.validate()?;
        if info.max_transfer < PAGE as u32 {
            return Err(Error::Unsupported.into());
        }
        let recovered = recovery::inspect(&mut device)?;
        let newest = recovered.retained.last().ok_or(FsError::Corrupt)?;
        let session = NEXT_SESSION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::Overflow)?;
        let files = newest.files.clone();
        let next_nid = newest.cp.next_nid;
        let state = if info.read_only || !info.flush {
            MountState::DegradedReadOnly
        } else {
            recovered.state
        };
        Ok(Self {
            device,
            session,
            info,
            uuid: recovered.uuid,
            sb: recovered.sb,
            geometry: recovered.geometry,
            state,
            retained: recovered.retained,
            files,
            next_nid,
            dirty: false,
            frozen: false,
            quarantine: None,
        })
    }
    pub fn state(&self) -> MountState {
        self.state
    }
    pub fn generation(&self) -> u64 {
        self.retained.last().map_or(0, |s| s.cp.generation)
    }
    pub fn device(&self) -> &D {
        &self.device
    }
    /// Consumes the mount without flushing. Caller must fence uncertain I/O before
    /// a new writable session; synchronous hosted adapters already fence on reset.
    pub fn into_device(self) -> D {
        self.device
    }
    fn writable(&self) -> Result<()> {
        if self.frozen || self.state != MountState::CleanWritable {
            return Err(FsError::ReadOnly);
        }
        if self.device.info() != self.info {
            return Err(IoFailure::uncertain(IoKind::Offline).into());
        }
        Ok(())
    }
    pub fn entries(&self) -> Vec<Entry> {
        self.files
            .iter()
            .map(|f| Entry {
                name: f.name.clone(),
                id: FileId {
                    volume: self.uuid,
                    node: f.id.clone(),
                    session: self.session,
                },
                size: f.data.len() as u64,
            })
            .collect()
    }
    pub fn lookup(&self, name: &[u8]) -> Result<FileId> {
        self.files
            .iter()
            .find(|f| f.name == name)
            .map(|f| FileId {
                volume: self.uuid,
                node: f.id.clone(),
                session: self.session,
            })
            .ok_or(FsError::NotFound)
    }
    fn index(&self, id: &FileId) -> Result<usize> {
        if id.volume != self.uuid || id.session != self.session {
            return Err(FsError::Stale);
        }
        self.files
            .iter()
            .position(|f| f.id == id.node)
            .ok_or(FsError::Stale)
    }
    pub fn create(&mut self, name: &[u8]) -> Result<FileId> {
        self.writable()?;
        DirEntry {
            name: name.to_vec(),
            target: NodeId::ROOT,
            kind: 1,
        }
        .validate()?;
        if name.len() > MAX_NAME
            || self.files.len() >= MAX_FILES
            || self.files.len() as u64 + 1 >= self.sb.maximum_nodes
        {
            return Err(FsError::Limit);
        }
        if self.files.iter().any(|f| f.name == name) {
            return Err(FsError::Exists);
        }
        let next = self.next_nid.checked_add(1).ok_or(Error::Overflow)?;
        let id = NodeId {
            nid: self.next_nid,
            incarnation: 1,
        };
        self.files.push(File {
            name: name.to_vec(),
            id: id.clone(),
            data: Vec::new(),
        });
        self.files.sort_by(|a, b| a.name.cmp(&b.name));
        self.next_nid = next;
        self.dirty = true;
        Ok(FileId {
            volume: self.uuid,
            node: id,
            session: self.session,
        })
    }
    pub fn write(&mut self, id: &FileId, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.writable()?;
        let i = self.index(id)?;
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or(Error::Overflow)?;
        if offset > MAX_FILE_BYTES as u64
            || end > MAX_FILE_BYTES as u64
            || (!bytes.is_empty() && end.div_ceil(PAGE as u64) > self.sb.maximum_extents as u64)
        {
            return Err(FsError::Limit);
        }
        if bytes.is_empty() {
            return Ok(0);
        }
        let data = &mut self.files[i].data;
        if data.len() < end as usize {
            data.resize(end as usize, 0);
        }
        data[offset as usize..end as usize].copy_from_slice(bytes);
        self.dirty = true;
        Ok(bytes.len())
    }
    pub fn read(&self, id: &FileId, offset: u64, out: &mut [u8]) -> Result<usize> {
        let file = &self.files[self.index(id)?];
        if offset >= file.data.len() as u64 {
            return Ok(0);
        }
        let count = out.len().min(file.data.len() - offset as usize);
        out[..count].copy_from_slice(&file.data[offset as usize..offset as usize + count]);
        Ok(count)
    }
    pub fn accounting(&self) -> Accounting {
        let mut blocks = BTreeSet::new();
        let mut segments = BTreeSet::new();
        for s in &self.retained {
            for &b in s.owned.keys() {
                blocks.insert(b);
                segments.insert((b - self.sb.segment_start) / self.sb.segment_blocks as u64);
            }
        }
        if let Some(p) = &self.quarantine {
            for &b in &p.admission.blocks {
                blocks.insert(b);
                segments.insert((b - self.sb.segment_start) / self.sb.segment_blocks as u64);
            }
        }
        let capacity = segments.len() as u64 * self.sb.segment_blocks as u64;
        Accounting {
            protected_blocks: blocks.len() as u64,
            protected_segments: segments.len() as u64,
            placement_overhead_blocks: capacity - blocks.len() as u64,
            available_blocks: self.sb.segment_count * self.sb.segment_blocks as u64 - capacity,
            recovery_reserve_blocks: self.sb.cleaner_reserve,
        }
    }
    /// Exact read-only plan. This is repeated by fsync under the exclusive owner;
    /// no mutation or persistent write can intervene in admission/publication.
    pub fn admission(&self) -> Result<Admission> {
        self.writable()?;
        Ok(self.plan()?.admission)
    }
    pub fn fsync(&mut self, id: &FileId) -> Result<()> {
        self.writable()?;
        self.index(id)?;
        if !self.dirty {
            return Ok(());
        }
        let plan = self.plan()?; // all allocation, encoding and overflow checks before I/O
        self.quarantine = Some(plan);
        let result = self.publish();
        if result.is_err() {
            self.frozen = true;
            self.state = MountState::DegradedReadOnly;
        }
        result
    }
    fn checked_session(&self) -> Result<()> {
        if self.device.info() != self.info {
            return Err(IoFailure::uncertain(IoKind::Offline).into());
        }
        Ok(())
    }
    fn put(&mut self, b: u64, bytes: &[u8; PAGE]) -> Result<()> {
        self.checked_session()?;
        self.device
            .write(b.checked_mul(PAGE as u64).ok_or(Error::Overflow)?, bytes)?;
        self.checked_session()
    }
    fn barrier(&mut self) -> Result<()> {
        self.checked_session()?;
        self.device.flush()?;
        self.checked_session()
    }
    fn retire(&mut self) -> Result<()> {
        let slot = self.retained[0].slot;
        self.put(slot, &[0; PAGE])?;
        self.barrier()?;
        self.retained.remove(0);
        Ok(())
    }
    fn publish(&mut self) -> Result<()> {
        if self.retained.len() == 3 {
            self.retire()?;
        } // R0; plan excludes all three closures
          // Move one pre-encoded block at a time, without allocating after admission.
        let count = self
            .quarantine
            .as_ref()
            .ok_or(FsError::Corrupt)?
            .dependencies
            .len();
        for i in 0..count {
            let (b, bytes) = self
                .quarantine
                .as_ref()
                .ok_or(FsError::Corrupt)?
                .dependencies[i];
            self.put(b, &bytes)?;
        }
        self.barrier()?; // D
        let p = self.quarantine.as_ref().ok_or(FsError::Corrupt)?;
        let slot = p.snapshot.slot;
        let cp = p.checkpoint;
        self.put(slot, &cp)?;
        self.barrier()?; // P
                         // R happens while old closure still owned. Do not allocate a retained slot
                         // after admission; mount pre-reserves the Vec for all three generations.
        if self.retained.len() == 2 {
            self.retire()?;
        }
        let p = self.quarantine.take().ok_or(FsError::Corrupt)?;
        self.retained.push(p.snapshot);
        self.dirty = false;
        Ok(())
    }
    fn plan(&self) -> Result<Plan> {
        let generation = self.generation().checked_add(1).ok_or(Error::Overflow)?;
        let mut used = BTreeSet::new();
        for s in &self.retained {
            for &b in s.owned.keys() {
                used.insert((b - self.sb.segment_start) / self.sb.segment_blocks as u64);
            }
        }
        let data_count: usize = self.files.iter().map(|f| f.data.len().div_ceil(PAGE)).sum();
        // root inode/root, optional directory page, map page/root, orphan root,
        // segment root/pages; file inode/extent root and optional extent page.
        let base = 6
            + self.geometry.segment_table_pages as usize
            + usize::from(!self.files.is_empty())
            + self.files.len() * 2
            + self.files.iter().filter(|f| !f.data.is_empty()).count()
            + data_count;
        let summaries =
            crate::geometry::summary_fixed_point(base as u64, self.sb.segment_blocks as u64)?.0
                as usize;
        let count = base.checked_add(summaries).ok_or(Error::Overflow)?;
        if count as u64 > self.sb.transaction_bound {
            return Err(FsError::NoSpace);
        }
        let segment_need = (count as u64).div_ceil(self.sb.segment_blocks as u64);
        let free = self.sb.segment_count - used.len() as u64;
        if free * (self.sb.segment_blocks as u64)
            < segment_need * self.sb.segment_blocks as u64 + self.sb.cleaner_reserve
        {
            return Err(FsError::NoSpace);
        }
        let segments: Vec<u64> = (0..self.sb.segment_count)
            .filter(|s| !used.contains(s))
            .take(segment_need as usize)
            .collect();
        let blocks: Vec<u64> = segments
            .iter()
            .flat_map(|s| {
                (0..self.sb.segment_blocks as u64)
                    .map(move |o| self.sb.segment_start + s * self.sb.segment_blocks as u64 + o)
            })
            .take(count)
            .collect();
        ensure(blocks.len() == count)?;
        let slot = if self.retained.len() == 3 {
            self.retained[0].slot
        } else {
            (2..=4)
                .find(|b| !self.retained.iter().any(|s| s.slot == *b))
                .ok_or(FsError::Corrupt)?
        };
        let mut builder = Builder {
            uuid: self.uuid,
            generation,
            blocks: blocks.clone(),
            cursor: 0,
            dependencies: Vec::with_capacity(count),
            owned: BTreeMap::new(),
        };
        let mut maps = Vec::new();
        let mut bindings = Vec::new();
        // Data first; every snapshot has private dense blocks and zero EOF tails.
        for f in &self.files {
            let mut extents = Vec::new();
            for (logical, chunk) in f.data.chunks(PAGE).enumerate() {
                let b = builder.allocate()?;
                let mut bytes = [0; PAGE];
                bytes[..chunk.len()].copy_from_slice(chunk);
                let crc = data_crc(
                    self.uuid,
                    &f.id,
                    logical as u64,
                    generation,
                    generation,
                    &bytes,
                )?;
                builder.owned.insert(
                    b,
                    Owner {
                        epoch: generation,
                        kind: 0,
                        node: f.id.clone(),
                        key: logical as u64,
                        birth: generation,
                        crc,
                    },
                );
                builder.dependencies.push((b, bytes));
                extents.push(Extent {
                    logical: logical as u64,
                    physical: b,
                    epoch: generation,
                    content_generation: generation,
                    blocks: 1,
                    data_crc: crc,
                    ..Default::default()
                });
            }
            let ep = if extents.is_empty() {
                Vec::new()
            } else {
                let n = extents.len();
                let last = n as u64 - 1;
                let r = builder.page(
                    Payload::Extents {
                        owner: f.id.clone(),
                        entries: extents,
                    },
                    f.id.clone(),
                    0,
                    0,
                )?;
                vec![Descriptor {
                    first: 0,
                    last,
                    page: r,
                    count: n as u32,
                    ..Default::default()
                }]
            };
            let ext = builder.page(
                Payload::Root {
                    kind: 5,
                    count: f.data.len().div_ceil(PAGE) as u64,
                    entry_bytes: 48,
                    pages: ep,
                },
                f.id.clone(),
                5,
                0,
            )?;
            let node = builder.page(
                Payload::Inode(Inode {
                    id: f.id.clone(),
                    kind: 1,
                    mode: 0o644,
                    nlink: 1,
                    size: f.data.len() as u64,
                    content_generation: generation,
                    extents: ext,
                    ..Default::default()
                }),
                f.id.clone(),
                0,
                0,
            )?;
            maps.push(MapEntry {
                id: f.id.clone(),
                node,
                kind: 1,
                ..Default::default()
            });
            bindings.push(DirEntry {
                name: f.name.clone(),
                target: f.id.clone(),
                kind: 1,
            });
        }
        let directory_size = bindings
            .iter()
            .map(|e| (32 + e.name.len() as u64).div_ceil(8) * 8)
            .sum();
        let pages = if bindings.is_empty() {
            Vec::new()
        } else {
            let count = bindings.len() as u32;
            let page = builder.page(
                Payload::Directory {
                    owner: NodeId::ROOT,
                    ordinal: 0,
                    entries: bindings,
                },
                NodeId::ROOT,
                0,
                0,
            )?;
            vec![Descriptor {
                first: 0,
                last: 0,
                page,
                count,
                ..Default::default()
            }]
        };
        let directory = builder.page(
            Payload::Root {
                kind: 4,
                count: self.files.len() as u64,
                entry_bytes: 0,
                pages,
            },
            NodeId::ROOT,
            4,
            0,
        )?;
        let root = builder.page(
            Payload::Inode(Inode {
                id: NodeId::ROOT,
                kind: 2,
                mode: 0o755,
                nlink: 2,
                size: directory_size,
                content_generation: generation,
                parent: NodeId::ROOT,
                directory,
                ..Default::default()
            }),
            NodeId::ROOT,
            0,
            0,
        )?;
        maps.push(MapEntry {
            id: NodeId::ROOT,
            node: root,
            kind: 2,
            ..Default::default()
        });
        maps.sort_by_key(|m| m.id.nid);
        let last = maps.last().ok_or(FsError::Corrupt)?.id.nid;
        let nodes = maps.len() as u64;
        let map_page = builder.page(
            Payload::Table {
                first: 1,
                last,
                records: Records::Map(maps),
            },
            NodeId::default(),
            1,
            0,
        )?;
        let node_map = builder.page(
            Payload::Root {
                kind: 1,
                count: nodes,
                entry_bytes: 48,
                pages: vec![Descriptor {
                    first: 1,
                    last,
                    page: map_page,
                    count: nodes as u32,
                    ..Default::default()
                }],
            },
            NodeId::default(),
            1,
            0,
        )?;
        let orphan_table = builder.page(empty_root(3, 32), NodeId::default(), 3, 0)?;
        // Reserve segment root/pages and summary locations, then fill circular
        // ownership dependencies. Metadata summary CRCs are zero by v0.2 design.
        let seg_root = builder.allocate()?;
        let seg_pages: Vec<u64> = (0..self.geometry.segment_table_pages)
            .map(|_| builder.allocate())
            .collect::<Result<_>>()?;
        let summary_blocks = builder.blocks[builder.cursor..].to_vec();
        ensure(summary_blocks.len() == summaries)?;
        builder.identify(seg_root, 3, NodeId::default(), 2, 0, 0);
        for (i, &b) in seg_pages.iter().enumerate() {
            builder.identify(b, 4, NodeId::default(), 2, i as u32, 0);
        }
        let mut heads = BTreeMap::new();
        let mut pos = 0;
        for &seg in &segments {
            let live = blocks
                .iter()
                .filter(|&&b| (b - self.sb.segment_start) / self.sb.segment_blocks as u64 == seg)
                .count();
            let n = live.div_ceil(82);
            let mut refs = Vec::new();
            for ordinal in 0..n {
                let b = summary_blocks[pos];
                pos += 1;
                builder.identify(b, 8, NodeId::default(), 0, ordinal as u32, 0);
                refs.push(b);
            }
            heads.insert(seg, refs);
        }
        let mut head_refs = BTreeMap::new();
        for (&seg, locations) in &heads {
            let owned: Vec<_> = builder
                .owned
                .iter()
                .filter(|(b, _)| {
                    (**b - self.sb.segment_start) / self.sb.segment_blocks as u64 == seg
                })
                .map(|(&b, o)| SummaryEntry {
                    offset: ((b - self.sb.segment_start) % self.sb.segment_blocks as u64) as u32,
                    kind: o.kind,
                    owner: o.node.clone(),
                    logical_key: o.key,
                    content_generation: o.birth,
                    expected_crc: if o.kind == 0 { o.crc } else { 0 },
                    ..Default::default()
                })
                .collect();
            let mut next = BlockRef::default();
            for ordinal in (0..locations.len()).rev() {
                next = builder.at(
                    locations[ordinal],
                    Payload::Summary {
                        segment: seg,
                        epoch: generation,
                        ordinal: ordinal as u32,
                        next,
                        entries: owned[ordinal * 82..owned.len().min((ordinal + 1) * 82)].to_vec(),
                    },
                )?;
            }
            head_refs.insert(seg, next);
        }
        let mut records = Vec::new();
        for seg in 0..self.sb.segment_count {
            let mut bitmap = vec![0u8; self.sb.segment_blocks as usize / 8];
            let mut live = 0;
            for &b in &blocks {
                let rel = b - self.sb.segment_start;
                if rel / self.sb.segment_blocks as u64 == seg {
                    let o = (rel % self.sb.segment_blocks as u64) as usize;
                    bitmap[o / 8] |= 1 << (o % 8);
                    live += 1;
                }
            }
            records.push(Segment {
                index: seg,
                epoch: if live > 0 { generation } else { 0 },
                state: if live > 0 { 1 } else { 0 },
                live,
                next_hint: live,
                summary: head_refs.remove(&seg).unwrap_or_default(),
                bitmap,
            });
        }
        let mut descriptors = Vec::new();
        for (i, chunk) in records
            .chunks(3976 / self.geometry.segment_record_bytes as usize)
            .enumerate()
        {
            let first = chunk[0].index;
            let last = chunk[chunk.len() - 1].index;
            let page = builder.at(
                seg_pages[i],
                Payload::Table {
                    first,
                    last,
                    records: Records::Segments(chunk.to_vec()),
                },
            )?;
            descriptors.push(Descriptor {
                first,
                last,
                page,
                count: chunk.len() as u32,
                ..Default::default()
            });
        }
        let segment_table = builder.at(
            seg_root,
            Payload::Root {
                kind: 2,
                count: self.sb.segment_count,
                entry_bytes: self.geometry.segment_record_bytes as u32,
                pages: descriptors,
            },
        )?;
        ensure(builder.owned.len() == count && builder.dependencies.len() == count)?;
        let cp = Checkpoint {
            generation,
            predecessor: self.generation(),
            root: NodeId::ROOT,
            node_map,
            segment_table,
            orphan_table,
            next_nid: self.next_nid,
            allocated_blocks: count as u64,
            free_blocks: self.sb.segment_count * self.sb.segment_blocks as u64 - count as u64,
            live_nodes: nodes,
            orphan_nodes: 0,
            transaction_blocks: count as u64,
            state: 1,
            ..Default::default()
        };
        let checkpoint = Page {
            uuid: self.uuid,
            block: slot,
            epoch: 0,
            birth: generation,
            payload: Payload::Checkpoint(cp.clone()),
        }
        .encode()?;
        let reserved_segment_blocks = segment_need * self.sb.segment_blocks as u64;
        Ok(Plan {
            admission: Admission {
                generation,
                dependency_blocks: count as u64,
                data_blocks: data_count as u64,
                metadata_blocks: (count - data_count) as u64,
                reserved_segment_blocks,
                placement_overhead_blocks: reserved_segment_blocks - count as u64,
                encoded_bytes: (count + 1) * PAGE,
                blocks,
            },
            dependencies: builder.dependencies,
            snapshot: Snapshot {
                slot,
                cp,
                owned: builder.owned,
                files: self.files.clone(),
            },
            checkpoint,
        })
    }
}
struct Builder {
    uuid: [u8; 16],
    generation: u64,
    blocks: Vec<u64>,
    cursor: usize,
    dependencies: Vec<(u64, [u8; PAGE])>,
    owned: BTreeMap<u64, Owner>,
}
impl Builder {
    fn allocate(&mut self) -> Result<u64> {
        let b = *self.blocks.get(self.cursor).ok_or(FsError::Corrupt)?;
        self.cursor += 1;
        Ok(b)
    }
    fn identify(&mut self, b: u64, kind: u16, node: NodeId, table: u16, ordinal: u32, crc: u64) {
        self.owned.insert(
            b,
            Owner {
                epoch: self.generation,
                kind,
                node,
                key: ((kind as u64) << 48) | ((table as u64) << 32) | ordinal as u64,
                birth: self.generation,
                crc,
            },
        );
    }
    fn at(&mut self, b: u64, payload: Payload) -> Result<BlockRef> {
        let p = Page {
            uuid: self.uuid,
            block: b,
            epoch: self.generation,
            birth: self.generation,
            payload,
        };
        let bytes = p.encode()?;
        let r = p.reference()?;
        if let Some(o) = self.owned.get_mut(&b) {
            o.crc = r.crc;
        }
        self.dependencies.push((b, bytes));
        Ok(r)
    }
    fn page(
        &mut self,
        payload: Payload,
        node: NodeId,
        table: u16,
        ordinal: u32,
    ) -> Result<BlockRef> {
        let b = self.allocate()?;
        self.identify(b, payload.page_type(), node, table, ordinal, 0);
        self.at(b, payload)
    }
}
