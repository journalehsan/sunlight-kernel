//! Independent read-only traversal. Shares byte codecs, not bootstrap planning,
//! geometry calculation, allocator, publication, or recovery selection code.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
use sunlight_sunfs::codec::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Clean,
    DegradedRecoverable,
    Corrupt,
    Unsupported,
    Indeterminate,
}
impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Self::Clean => "CLEAN",
            Self::DegradedRecoverable => "DEGRADED_RECOVERABLE",
            Self::Corrupt => "CORRUPT",
            Self::Unsupported => "UNSUPPORTED",
            Self::Indeterminate => "INDETERMINATE",
        }
    }
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Clean => 0,
            Self::DegradedRecoverable => 1,
            Self::Corrupt => 2,
            Self::Unsupported => 3,
            Self::Indeterminate => 4,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Invalid(&'static str),
    Codec(sunlight_sunfs::Error),
    Read,
    Budget,
}
impl From<sunlight_sunfs::Error> for Failure {
    fn from(e: sunlight_sunfs::Error) -> Self {
        Self::Codec(e)
    }
}
type Result<T> = std::result::Result<T, Failure>;
fn require(value: bool, message: &'static str) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(Failure::Invalid(message))
    }
}
pub trait ReadImage {
    fn capacity(&self) -> u64;
    fn read_page(&mut self, block: u64) -> Result<[u8; PAGE]>;
}
pub struct Bytes<'a>(pub &'a [u8]);
impl ReadImage for Bytes<'_> {
    fn capacity(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_page(&mut self, block: u64) -> Result<[u8; PAGE]> {
        let start = usize::try_from(block)
            .ok()
            .and_then(|b| b.checked_mul(PAGE))
            .ok_or(Failure::Read)?;
        let end = start.checked_add(PAGE).ok_or(Failure::Read)?;
        self.0
            .get(start..end)
            .ok_or(Failure::Read)?
            .try_into()
            .map_err(|_| Failure::Read)
    }
}
pub struct ReadFile {
    file: File,
    size: u64,
}
impl ReadFile {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(std::io::Error::other("regular image required"));
        }
        Ok(Self {
            file,
            size: meta.len(),
        })
    }
}
impl ReadImage for ReadFile {
    fn capacity(&self) -> u64 {
        self.size
    }
    fn read_page(&mut self, block: u64) -> Result<[u8; PAGE]> {
        let offset = block.checked_mul(PAGE as u64).ok_or(Failure::Read)?;
        if offset
            .checked_add(PAGE as u64)
            .is_none_or(|end| end > self.size)
        {
            return Err(Failure::Read);
        }
        let mut b = [0; PAGE];
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.read_exact(&mut b))
            .map_err(|_| Failure::Read)?;
        Ok(b)
    }
}
#[derive(Clone, Debug)]
pub struct Slot {
    pub block: u64,
    pub generation: Option<u64>,
    pub class: String,
    pub error: Option<Failure>,
}
#[derive(Clone, Debug)]
pub struct CheckedNode {
    pub id: NodeId,
    pub kind: u8,
    pub size: u64,
    pub extents: Vec<Extent>,
}
#[derive(Clone, Debug)]
pub struct CheckedGeneration {
    pub generation: u64,
    pub slot: u64,
    pub blocks: Vec<u64>,
    pub nodes: Vec<CheckedNode>,
    pub root_entries: Vec<DirEntry>,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub status: Status,
    pub selected: Option<u64>,
    pub superblocks: Vec<String>,
    pub slots: Vec<Slot>,
    pub issues: Vec<String>,
    pub union_blocks: usize,
    pub pages_read: u64,
    pub generations: Vec<CheckedGeneration>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Ownership {
    epoch: u64,
    crc: u64,
    kind: u16,
    owner: NodeId,
    key: u64,
    generation: u64,
}
struct Closure {
    inspection: CheckedGeneration,
    cp: Checkpoint,
    owned: BTreeMap<u64, Ownership>,
}
struct Walker<'a, R: ReadImage> {
    reader: &'a mut R,
    sb: &'a Superblock,
    uuid: [u8; 16],
    generation: u64,
    owned: BTreeMap<u64, Ownership>,
    reads: u64,
}
impl<R: ReadImage> Walker<'_, R> {
    fn range(&self, block: u64) -> Result<(u64, u32)> {
        let relative = block
            .checked_sub(self.sb.segment_start)
            .ok_or(Failure::Invalid("reserved reference"))?;
        let seg = relative / self.sb.segment_blocks as u64;
        require(
            seg < self.sb.segment_count,
            "reference outside segment area",
        )?;
        Ok((seg, (relative % self.sb.segment_blocks as u64) as u32))
    }
    fn raw(&mut self, block: u64) -> Result<[u8; PAGE]> {
        self.reads += 1;
        if self.reads > 65536 {
            return Err(Failure::Budget);
        }
        self.reader.read_page(block)
    }
    fn fetch(
        &mut self,
        r: &BlockRef,
        kind: u16,
        owner: NodeId,
        table: u16,
        ordinal: u32,
    ) -> Result<Payload> {
        r.present()?;
        self.range(r.block)?;
        require(
            !self.owned.contains_key(&r.block),
            "duplicate live metadata block",
        )?;
        let b = self.raw(r.block)?;
        let p = Page::decode(&b, r.block)?;
        let crc = u64::from_le_bytes(b[64..72].try_into().map_err(|_| Failure::Read)?);
        require(
            p.uuid == self.uuid
                && p.epoch == r.epoch
                && crc == r.crc
                && p.birth <= self.generation
                && p.payload.page_type() == kind,
            "page identity or parent checksum",
        )?;
        let key = ((kind as u64) << 48) | ((table as u64) << 32) | ordinal as u64;
        self.owned.insert(
            r.block,
            Ownership {
                epoch: p.epoch,
                crc,
                kind,
                owner,
                key,
                generation: p.birth,
            },
        );
        Ok(p.payload)
    }
    fn root(&mut self, r: &BlockRef, kind: u16, owner: NodeId) -> Result<(u64, Vec<Descriptor>)> {
        match self.fetch(r, 3, owner, kind, 0)? {
            Payload::Root {
                kind: k,
                count,
                entry_bytes,
                pages,
            } => {
                require(
                    k == kind && pages.len() <= self.sb.maximum_table_pages as usize,
                    "table root kind or page cap",
                )?;
                let expected = match kind {
                    1 | 5 => 48,
                    2 => 64 + self.sb.segment_blocks / 8,
                    3 => 32,
                    4 => 0,
                    _ => return Err(Failure::Invalid("table kind")),
                };
                require(entry_bytes == expected, "table record width")?;
                Ok((count, pages))
            }
            _ => Err(Failure::Invalid("expected table root")),
        }
    }
    fn table(&mut self, r: &BlockRef, kind: u16) -> Result<Records> {
        let (count, pages) = self.root(r, kind, NodeId::default())?;
        let mut maps = Vec::new();
        let mut segments = Vec::new();
        let mut orphans = Vec::new();
        for (ordinal, desc) in pages.iter().enumerate() {
            match self.fetch(&desc.page, 4, NodeId::default(), kind, ordinal as u32)? {
                Payload::Table {
                    first,
                    last,
                    records,
                } => {
                    require(
                        records.kind() == kind
                            && first == desc.first
                            && last == desc.last
                            && records.len() == desc.count as usize,
                        "table descriptor mismatch",
                    )?;
                    match records {
                        Records::Map(v) => maps.extend(v),
                        Records::Segments(v) => segments.extend(v),
                        Records::Orphans(v) => orphans.extend(v),
                    }
                }
                _ => return Err(Failure::Invalid("expected table page")),
            }
        }
        let records = match kind {
            1 => Records::Map(maps),
            2 => Records::Segments(segments),
            3 => Records::Orphans(orphans),
            _ => return Err(Failure::Invalid("table kind")),
        };
        require(records.len() as u64 == count, "table count")?;
        Ok(records)
    }
    fn check(mut self, cp: Checkpoint) -> Result<(Closure, u64)> {
        require(
            cp.state == 1
                && cp.root == NodeId::ROOT
                && cp.transaction_blocks <= self.sb.transaction_bound,
            "incomplete checkpoint or transaction bound",
        )?;
        let maps = match self.table(&cp.node_map, 1)? {
            Records::Map(v) => v,
            _ => return Err(Failure::Invalid("node map")),
        };
        require(
            !maps.is_empty() && maps.len() as u64 <= self.sb.maximum_nodes,
            "node limit",
        )?;
        let orphans = match self.table(&cp.orphan_table, 3)? {
            Records::Orphans(v) => v,
            _ => return Err(Failure::Invalid("orphan table")),
        };
        require(
            orphans.len() as u64 <= self.sb.maximum_orphans,
            "orphan limit",
        )?;
        let segments = match self.table(&cp.segment_table, 2)? {
            Records::Segments(v) => v,
            _ => return Err(Failure::Invalid("segment table")),
        };
        require(
            segments.len() as u64 == self.sb.segment_count,
            "segment count",
        )?;
        let mut checked_nodes = Vec::new();
        let mut nodes = BTreeMap::new();
        let mut bindings: BTreeMap<u64, Vec<DirEntry>> = BTreeMap::new();
        let mut total_bindings = 0u64;
        for map in &maps {
            require(map.id.nid < cp.next_nid, "next_nid high water")?;
            let inode = match self.fetch(&map.node, 5, map.id.clone(), 0, 0)? {
                Payload::Inode(v) => v,
                _ => return Err(Failure::Invalid("node record")),
            };
            require(
                inode.id == map.id
                    && inode.kind == map.kind
                    && inode.content_generation <= cp.generation,
                "node identity",
            )?;
            let mut checked_extents = Vec::new();
            if inode.kind == 2 {
                let (count, pages) = self.root(&inode.directory, 4, inode.id.clone())?;
                total_bindings = total_bindings.checked_add(count).ok_or(Failure::Budget)?;
                require(
                    total_bindings <= self.sb.maximum_nodes,
                    "namespace traversal budget",
                )?;
                let mut entries = Vec::new();
                let mut size = 0u64;
                for (ordinal, desc) in pages.iter().enumerate() {
                    match self.fetch(&desc.page, 6, inode.id.clone(), 0, ordinal as u32)? {
                        Payload::Directory {
                            owner,
                            ordinal: o,
                            entries: v,
                        } => {
                            require(
                                owner == inode.id
                                    && o as usize == ordinal
                                    && desc.count as usize == v.len(),
                                "directory descriptor",
                            )?;
                            for entry in &v {
                                size += (32 + entry.name.len() as u64).div_ceil(8) * 8;
                            }
                            entries.extend(v);
                        }
                        _ => return Err(Failure::Invalid("directory page")),
                    }
                }
                require(
                    entries.len() as u64 == count
                        && size == inode.size
                        && entries.windows(2).all(|v| v[0].name < v[1].name),
                    "directory count, size or ordering",
                )?;
                bindings.insert(inode.id.nid, entries);
            } else {
                let (count, pages) = self.root(&inode.extents, 5, inode.id.clone())?;
                require(count <= self.sb.maximum_extents as u64, "extent cap")?;
                let mut total = 0u64;
                let mut previous = None;
                for (ordinal, desc) in pages.iter().enumerate() {
                    match self.fetch(&desc.page, 7, inode.id.clone(), 0, ordinal as u32)? {
                        Payload::Extents { owner, entries } => {
                            require(
                                owner == inode.id
                                    && entries.len() == desc.count as usize
                                    && entries.first().is_some_and(|e| e.logical == desc.first)
                                    && entries.last().is_some_and(|e| e.logical == desc.last),
                                "extent descriptor",
                            )?;
                            for e in entries {
                                checked_extents.push(e.clone());
                                require(
                                    previous.is_none_or(|v| v < e.logical)
                                        && e.content_generation <= cp.generation
                                        && e.logical < inode.size.div_ceil(PAGE as u64),
                                    "extent order or EOF",
                                )?;
                                previous = Some(e.logical);
                                self.range(e.physical)?;
                                require(
                                    !self.owned.contains_key(&e.physical),
                                    "duplicate live data block",
                                )?;
                                let data = self.raw(e.physical)?;
                                require(
                                    data_crc(
                                        self.uuid,
                                        &inode.id,
                                        e.logical,
                                        e.epoch,
                                        e.content_generation,
                                        &data,
                                    )? == e.data_crc,
                                    "data checksum",
                                )?;
                                if (e.logical + 1) * PAGE as u64 > inode.size {
                                    let used = (inode.size % PAGE as u64) as usize;
                                    require(
                                        data[used..].iter().all(|&v| v == 0),
                                        "nonzero EOF tail",
                                    )?;
                                }
                                self.owned.insert(
                                    e.physical,
                                    Ownership {
                                        epoch: e.epoch,
                                        crc: e.data_crc,
                                        kind: 0,
                                        owner: inode.id.clone(),
                                        key: e.logical,
                                        generation: e.content_generation,
                                    },
                                );
                                total += 1;
                            }
                        }
                        _ => return Err(Failure::Invalid("extent page")),
                    }
                }
                require(total == count, "extent root count")?;
            }
            checked_nodes.push(CheckedNode {
                id: inode.id.clone(),
                kind: inode.kind,
                size: inode.size,
                extents: checked_extents,
            });
            require(
                nodes.insert(inode.id.nid, inode).is_none(),
                "duplicate node id",
            )?;
        }
        let root = nodes.get(&1).ok_or(Failure::Invalid("missing root"))?;
        require(
            root.id == NodeId::ROOT && root.kind == 2 && root.parent == NodeId::ROOT,
            "root metadata",
        )?;
        let mut orphan_ids = BTreeSet::new();
        for o in &orphans {
            let node = nodes
                .get(&o.id.nid)
                .ok_or(Failure::Invalid("unmapped orphan"))?;
            require(
                o.id == node.id
                    && node.nlink == 0
                    && o.unlink_generation <= cp.generation
                    && orphan_ids.insert(o.id.nid),
                "orphan identity or generation",
            )?;
            if node.kind == 2 {
                require(
                    bindings.get(&o.id.nid).is_some_and(|v| v.is_empty()),
                    "nonempty orphan directory",
                )?;
            }
        }
        let mut reachable = BTreeSet::new();
        let mut stack = vec![1u64];
        let mut links: BTreeMap<u64, u32> = BTreeMap::new();
        while let Some(nid) = stack.pop() {
            require(reachable.insert(nid), "directory cycle or hardlink")?;
            require(!orphan_ids.contains(&nid), "reachable orphan")?;
            let node = nodes.get(&nid).ok_or(Failure::Invalid("unmapped child"))?;
            if node.kind == 2 {
                let mut nlink = 2;
                for entry in bindings
                    .get(&nid)
                    .ok_or(Failure::Invalid("directory absent"))?
                {
                    let child = nodes
                        .get(&entry.target.nid)
                        .ok_or(Failure::Invalid("unmapped directory target"))?;
                    require(
                        child.id == entry.target && child.kind == entry.kind,
                        "stale directory target",
                    )?;
                    if child.kind == 2 {
                        require(child.parent == node.id, "directory parent mismatch")?;
                        nlink += 1;
                    }
                    stack.push(child.id.nid);
                }
                links.insert(nid, nlink);
            } else {
                links.insert(nid, 1);
            }
            if stack.len() > 6724 {
                return Err(Failure::Budget);
            }
        }
        for (nid, node) in &nodes {
            require(
                reachable.contains(nid) || orphan_ids.contains(nid),
                "unreachable non-orphan node",
            )?;
            let expected = if orphan_ids.contains(nid) {
                0
            } else {
                *links.get(nid).ok_or(Failure::Invalid("missing links"))?
            };
            require(node.nlink == expected, "namespace nlink mismatch")?;
        }
        let mut summaries: BTreeMap<u64, SummaryEntry> = BTreeMap::new();
        for (index, segment) in segments.iter().enumerate() {
            require(
                segment.index == index as u64
                    && segment.bitmap.len() == self.sb.segment_blocks as usize / 8,
                "segment record geometry/order",
            )?;
            let mut next = segment.summary.clone();
            let pages = (segment.live as usize).div_ceil(82);
            for ordinal in 0..pages {
                match self.fetch(&next, 8, NodeId::default(), 0, ordinal as u32)? {
                    Payload::Summary {
                        segment: s,
                        epoch,
                        ordinal: o,
                        next: n,
                        entries,
                    } => {
                        require(
                            s == index as u64 && epoch == segment.epoch && o == ordinal as u32,
                            "summary identity",
                        )?;
                        require(
                            entries.len() == (segment.live as usize - ordinal * 82).min(82),
                            "summary entry count",
                        )?;
                        for e in entries {
                            require(e.offset < self.sb.segment_blocks, "summary offset")?;
                            let block = self.sb.segment_start
                                + index as u64 * self.sb.segment_blocks as u64
                                + e.offset as u64;
                            require(
                                summaries.insert(block, e).is_none(),
                                "duplicate summary ownership",
                            )?;
                        }
                        next = n;
                    }
                    _ => return Err(Failure::Invalid("summary page")),
                }
            }
            require(next.absent(), "summary chain length")?;
        }
        for (index, segment) in segments.iter().enumerate() {
            for offset in 0..self.sb.segment_blocks {
                let block = self.sb.segment_start
                    + index as u64 * self.sb.segment_blocks as u64
                    + offset as u64;
                let bit = segment.bitmap[offset as usize / 8] & (1 << (offset % 8)) != 0;
                require(
                    bit == self.owned.contains_key(&block),
                    "segment bitmap differs from closure",
                )?;
                if let Some(owner) = self.owned.get(&block) {
                    require(owner.epoch == segment.epoch, "segment epoch")?;
                }
            }
        }
        require(
            summaries.len() == self.owned.len(),
            "summary and closure size",
        )?;
        for (block, owner) in &self.owned {
            let e = summaries
                .get(block)
                .ok_or(Failure::Invalid("missing summary entry"))?;
            require(
                e.kind == owner.kind
                    && e.owner == owner.owner
                    && e.logical_key == owner.key
                    && e.content_generation == owner.generation
                    && e.expected_crc == if owner.kind == 0 { owner.crc } else { 0 },
                "summary owner differs from graph",
            )?;
        }
        require(
            cp.allocated_blocks == self.owned.len() as u64
                && cp.free_blocks
                    == self.sb.segment_count * self.sb.segment_blocks as u64 - cp.allocated_blocks
                && cp.live_nodes == nodes.len() as u64
                && cp.orphan_nodes == orphans.len() as u64,
            "checkpoint accounting",
        )?;
        Ok((
            Closure {
                inspection: CheckedGeneration {
                    generation: cp.generation,
                    slot: 0,
                    blocks: self.owned.keys().copied().collect(),
                    nodes: checked_nodes,
                    root_entries: bindings.remove(&1).unwrap_or_default(),
                },
                cp,
                owned: self.owned,
            },
            self.reads,
        ))
    }
}
fn validate_superblock<R: ReadImage>(r: &R, sb: &Superblock) -> Result<()> {
    let bps = sb.segment_blocks as u64;
    require(
        sb.volume_blocks == r.capacity() / PAGE as u64 && r.capacity().is_multiple_of(512),
        "volume size mismatch",
    )?;
    require(
        sb.segment_start == bps
            && sb.segment_count
                == sb
                    .volume_blocks
                    .checked_sub(bps)
                    .ok_or(Failure::Invalid("short volume"))?
                    / bps,
        "segment geometry",
    )?;
    let pages = sb.segment_count.div_ceil(3976 / (64 + bps / 8));
    require(
        sb.segment_count > 0 && pages <= sb.maximum_table_pages as u64,
        "segment table capacity",
    )?;
    let base = sb.maximum_nodes * 2
        + 3
        + sb.maximum_nodes.div_ceil(82)
        + pages
        + sb.maximum_orphans.div_ceil(124);
    let mut summaries = 0;
    let mut converged = false;
    for _ in 0..128 {
        let total = base + summaries;
        let next = (total / bps) * bps.div_ceil(82) + (total % bps).div_ceil(82);
        if next == summaries {
            converged = true;
            break;
        }
        summaries = next;
    }
    require(converged, "reserve fixed point")?;
    let metadata = base + summaries;
    let minimum = bps + metadata * 3 + 3;
    require(
        sb.cleaner_reserve >= minimum
            && sb.cleaner_reserve < sb.segment_count * bps
            && sb.transaction_bound >= metadata + bps
            && sb.transaction_bound <= sb.segment_count * bps,
        "reserve and transaction bounds",
    )?;
    Ok(())
}
pub fn check<R: ReadImage>(reader: &mut R) -> Report {
    let mut report = Report {
        status: Status::Corrupt,
        selected: None,
        superblocks: Vec::new(),
        slots: Vec::new(),
        issues: Vec::new(),
        union_blocks: 0,
        pages_read: 0,
        generations: Vec::new(),
    };
    let mut supers = Vec::new();
    let mut unsupported = false;
    let mut unreadable = false;
    for block in 0..2 {
        report.pages_read += 1;
        let result = reader
            .read_page(block)
            .and_then(|b| Page::decode(&b, block).map_err(Failure::from))
            .and_then(|p| match p.payload {
                Payload::Superblock(sb) => {
                    validate_superblock(reader, &sb)?;
                    Ok((p.uuid, sb))
                }
                _ => Err(Failure::Invalid("superblock type")),
            });
        match result {
            Ok(v) => {
                report.superblocks.push("VALID".into());
                supers.push(v);
            }
            Err(e) => {
                if e == Failure::Codec(sunlight_sunfs::Error::Unsupported) {
                    unsupported = true;
                }
                if e == Failure::Read || e == Failure::Budget {
                    unreadable = true;
                }
                report.superblocks.push(format!("{e:?}"));
            }
        }
    }
    if unsupported {
        report.status = Status::Unsupported;
        return report;
    }
    if supers.is_empty() {
        if unreadable {
            report.status = Status::Indeterminate;
        }
        return report;
    }
    if supers.len() == 2 {
        let mut a = supers[0].1.clone();
        let mut b = supers[1].1.clone();
        a.copy_index = 0;
        b.copy_index = 0;
        if supers[0].0 != supers[1].0 || a != b {
            report.status = Status::Indeterminate;
            report.issues.push("conflicting valid superblocks".into());
            return report;
        }
    }
    let (uuid, sb) = supers.remove(0);
    let mut valid = Vec::new();
    let mut damaged = false;
    for block in 2..=4 {
        report.pages_read += 1;
        let mut generation = None;
        let result = reader.read_page(block).and_then(|b| {
            if b.iter().all(|&v| v == 0) {
                return Ok(None);
            }
            let p = Page::decode(&b, block)?;
            require(p.uuid == uuid, "checkpoint UUID")?;
            let cp = match p.payload {
                Payload::Checkpoint(v) => v,
                _ => return Err(Failure::Invalid("checkpoint type")),
            };
            generation = Some(cp.generation);
            if cp.state == 0 {
                return Err(Failure::Invalid("INCOMPLETE"));
            }
            let walker = Walker {
                reader,
                sb: &sb,
                uuid,
                generation: cp.generation,
                owned: BTreeMap::new(),
                reads: 0,
            };
            let (mut closure, reads) = walker.check(cp)?;
            closure.inspection.slot = block;
            report.pages_read += reads;
            Ok(Some(closure))
        });
        match result {
            Ok(None) => report.slots.push(Slot {
                block,
                generation,
                class: "EMPTY".into(),
                error: None,
            }),
            Ok(Some(c)) => {
                report.slots.push(Slot {
                    block,
                    generation,
                    class: "VALID".into(),
                    error: None,
                });
                valid.push(c);
            }
            Err(e) => {
                damaged = true;
                if e == Failure::Read || e == Failure::Budget {
                    unreadable = true;
                }
                if e == Failure::Codec(sunlight_sunfs::Error::Unsupported) {
                    unsupported = true;
                }
                let class = if e == Failure::Invalid("INCOMPLETE") {
                    "INCOMPLETE"
                } else if generation.is_some() {
                    "UNRECOVERABLE"
                } else {
                    "CORRUPT_OR_INTERRUPTED"
                };
                report.slots.push(Slot {
                    block,
                    generation,
                    class: class.into(),
                    error: Some(e),
                });
            }
        }
    }
    if unsupported {
        report.status = Status::Unsupported;
        return report;
    }
    if unreadable {
        report.status = Status::Indeterminate;
        return report;
    }
    if valid.is_empty() {
        return report;
    }
    valid.sort_by_key(|c| c.cp.generation);
    if valid
        .windows(2)
        .any(|c| c[0].cp.generation == c[1].cp.generation)
    {
        report.status = Status::Indeterminate;
        report
            .issues
            .push("duplicate or conflicting generation".into());
        return report;
    }
    let mut union: BTreeMap<u64, Ownership> = BTreeMap::new();
    for closure in &valid {
        for (block, owner) in &closure.owned {
            if let Some(old) = union.insert(*block, owner.clone()) {
                if old != *owner {
                    report.status = Status::Indeterminate;
                    report
                        .issues
                        .push("immutable identity changed across checkpoints".into());
                    return report;
                }
            }
        }
    }
    report.generations = valid.iter().map(|c| c.inspection.clone()).collect();
    report.union_blocks = union.len();
    report.selected = valid.last().map(|c| c.cp.generation);
    report.status = if report.superblocks.iter().filter(|s| *s == "VALID").count() == 2
        && valid.len() >= 2
        && !damaged
        && sb.features.readonly == 0
    {
        Status::Clean
    } else {
        Status::DegradedRecoverable
    };
    report
}
