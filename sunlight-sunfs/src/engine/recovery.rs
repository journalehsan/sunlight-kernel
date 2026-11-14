//! Engine-specific bounded recovery. Independently traverses the bounded namespace and orphan subset;
//! the hosted checker does not call this code and has its own wider graph walker.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountState {
    CleanWritable,
    DegradedReadOnly,
}
pub(super) struct Recovered {
    pub uuid: [u8; 16],
    pub sb: Superblock,
    pub geometry: Geometry,
    pub state: MountState,
    pub retained: Vec<Snapshot>,
}
struct Scan<'a, D> {
    device: &'a mut D,
    info: DeviceInfo,
    uuid: [u8; 16],
    sb: &'a Superblock,
    generation: u64,
    owned: BTreeMap<u64, Owner>,
    reads: u64,
}
fn raw<D: BlockDevice>(device: &mut D, info: &DeviceInfo, b: u64) -> Result<[u8; PAGE]> {
    if device.info() != *info {
        return Err(FsError::Indeterminate);
    }
    let mut bytes = [0; PAGE];
    device.read(
        b.checked_mul(PAGE as u64).ok_or(Error::Overflow)?,
        &mut bytes,
    )?;
    if device.info() != *info {
        return Err(FsError::Indeterminate);
    }
    Ok(bytes)
}
impl<D: BlockDevice> Scan<'_, D> {
    fn range(&self, b: u64) -> Result<()> {
        ensure(
            b >= self.sb.segment_start
                && b < self.sb.segment_start
                    + self.sb.segment_count * self.sb.segment_blocks as u64,
        )
    }
    fn bytes(&mut self, b: u64) -> Result<[u8; PAGE]> {
        self.range(b)?;
        self.reads += 1;
        if self.reads > 65536 {
            return Err(Error::Budget.into());
        }
        raw(self.device, &self.info, b)
    }
    fn object(
        &mut self,
        r: &BlockRef,
        kind: u16,
        node: &NodeId,
        table: u16,
        ordinal: u32,
    ) -> Result<Payload> {
        r.present()?;
        ensure(!self.owned.contains_key(&r.block))?;
        let bytes = self.bytes(r.block)?;
        let p = Page::decode(&bytes, r.block)?;
        ensure(
            p.uuid == self.uuid
                && p.epoch == r.epoch
                && p.birth <= self.generation
                && p.reference()? == *r
                && p.payload.page_type() == kind,
        )?;
        self.owned.insert(
            r.block,
            Owner {
                epoch: p.epoch,
                kind,
                node: node.clone(),
                key: ((kind as u64) << 48) | ((table as u64) << 32) | ordinal as u64,
                birth: p.birth,
                crc: r.crc,
            },
        );
        Ok(p.payload)
    }
    fn root(&mut self, r: &BlockRef, table: u16, node: &NodeId) -> Result<(u64, Vec<Descriptor>)> {
        match self.object(r, 3, node, table, 0)? {
            Payload::Root {
                kind,
                count,
                entry_bytes,
                pages,
            } => {
                let width = match table {
                    1 | 5 => 48,
                    2 => 64 + self.sb.segment_blocks / 8,
                    3 => 32,
                    4 => 0,
                    _ => return Err(FsError::Corrupt),
                };
                ensure(
                    kind == table
                        && entry_bytes == width
                        && pages.len() <= self.sb.maximum_table_pages as usize,
                )?;
                Ok((count, pages))
            }
            _ => Err(FsError::Corrupt),
        }
    }
    fn records(&mut self, r: &BlockRef, table: u16) -> Result<Records> {
        let (count, descs) = self.root(r, table, &NodeId::default())?;
        let mut maps = Vec::new();
        let mut segments = Vec::new();
        let mut orphans = Vec::new();
        for (i, d) in descs.iter().enumerate() {
            match self.object(&d.page, 4, &NodeId::default(), table, i as u32)? {
                Payload::Table {
                    first,
                    last,
                    records,
                } => {
                    ensure(
                        first == d.first
                            && last == d.last
                            && records.kind() == table
                            && records.len() == d.count as usize,
                    )?;
                    match records {
                        Records::Map(v) => maps.extend(v),
                        Records::Segments(v) => segments.extend(v),
                        Records::Orphans(v) => orphans.extend(v),
                    }
                }
                _ => return Err(FsError::Corrupt),
            }
        }
        let records = match table {
            1 => Records::Map(maps),
            2 => Records::Segments(segments),
            3 => Records::Orphans(orphans),
            _ => return Err(FsError::Corrupt),
        };
        ensure(records.len() as u64 == count)?;
        Ok(records)
    }
    fn closure(mut self, slot: u64, cp: Checkpoint) -> Result<Snapshot> {
        ensure(
            cp.state == 1
                && cp.root == NodeId::ROOT
                && cp.transaction_blocks <= self.sb.transaction_bound,
        )?;
        let maps = match self.records(&cp.node_map, 1)? {
            Records::Map(v) => v,
            _ => return Err(FsError::Corrupt),
        };
        ensure(!maps.is_empty() && maps.len() as u64 <= self.sb.maximum_nodes)?;
        if maps.len() > MAX_FILES + MAX_DIRECTORIES + 1 {
            return Err(Error::Unsupported.into());
        }
        let orphans = match self.records(&cp.orphan_table, 3)? {
            Records::Orphans(v) => v,
            _ => return Err(FsError::Corrupt),
        };
        ensure(orphans.len() as u64 <= self.sb.maximum_orphans)?;
        if orphans.len() > MAX_ORPHANS {
            return Err(Error::Unsupported.into());
        }
        let segments = match self.records(&cp.segment_table, 2)? {
            Records::Segments(v) => v,
            _ => return Err(FsError::Corrupt),
        };
        ensure(segments.len() as u64 == self.sb.segment_count)?;
        let mut nodes = BTreeMap::new();
        let mut contents = BTreeMap::new();
        let mut directories = BTreeMap::new();
        for m in &maps {
            ensure(m.id.nid < cp.next_nid)?;
            let inode = match self.object(&m.node, 5, &m.id, 0, 0)? {
                Payload::Inode(n) => n,
                _ => return Err(FsError::Corrupt),
            };
            ensure(
                inode.id == m.id
                    && inode.kind == m.kind
                    && inode.content_generation <= cp.generation,
            )?;
            if inode.kind == 2 {
                let mut bindings = Vec::new();
                let (count, ds) = self.root(&inode.directory, 4, &inode.id)?;
                if count > MAX_ENTRIES as u64 {
                    return Err(Error::Unsupported.into());
                }
                for (i, d) in ds.iter().enumerate() {
                    match self.object(&d.page, 6, &inode.id, 0, i as u32)? {
                        Payload::Directory {
                            owner,
                            ordinal,
                            entries,
                        } => {
                            ensure(
                                owner == inode.id
                                    && ordinal == i as u32
                                    && entries.len() == d.count as usize,
                            )?;
                            bindings.extend(entries);
                        }
                        _ => return Err(FsError::Corrupt),
                    }
                }
                ensure(
                    bindings.len() as u64 == count
                        && bindings.windows(2).all(|w| w[0].name < w[1].name),
                )?;
                let size: u64 = bindings
                    .iter()
                    .map(|b| (32 + b.name.len() as u64).div_ceil(8) * 8)
                    .sum();
                ensure(size == inode.size)?;
                directories.insert(inode.id.nid, bindings);
            } else {
                if inode.size > MAX_FILE_BYTES as u64 {
                    return Err(Error::Unsupported.into());
                }
                if inode.kind != 1 {
                    return Err(Error::Unsupported.into());
                }
                let (count, ds) = self.root(&inode.extents, 5, &inode.id)?;
                ensure(count <= self.sb.maximum_extents as u64)?;
                let mut extents = Vec::new();
                for (i, d) in ds.iter().enumerate() {
                    match self.object(&d.page, 7, &inode.id, 0, i as u32)? {
                        Payload::Extents { owner, entries } => {
                            ensure(
                                owner == inode.id
                                    && entries.len() == d.count as usize
                                    && entries.first().is_some_and(|e| e.logical == d.first)
                                    && entries.last().is_some_and(|e| e.logical == d.last),
                            )?;
                            extents.extend(entries);
                        }
                        _ => return Err(FsError::Corrupt),
                    }
                }
                ensure(
                    extents.len() as u64 == count
                        && extents.windows(2).all(|w| w[0].logical < w[1].logical),
                )?;
                let mut data = vec![0; inode.size as usize];
                for e in &extents {
                    ensure(
                        e.logical < inode.size.div_ceil(PAGE as u64)
                            && e.content_generation <= cp.generation
                            && !self.owned.contains_key(&e.physical),
                    )?;
                    let bytes = self.bytes(e.physical)?;
                    ensure(
                        data_crc(
                            self.uuid,
                            &inode.id,
                            e.logical,
                            e.epoch,
                            e.content_generation,
                            &bytes,
                        )? == e.data_crc,
                    )?;
                    let a = e.logical as usize * PAGE;
                    let n = PAGE.min(data.len() - a);
                    ensure(bytes[n..].iter().all(|&b| b == 0))?;
                    data[a..a + n].copy_from_slice(&bytes[..n]);
                    self.owned.insert(
                        e.physical,
                        Owner {
                            epoch: e.epoch,
                            kind: 0,
                            node: inode.id.clone(),
                            key: e.logical,
                            birth: e.content_generation,
                            crc: e.data_crc,
                        },
                    );
                }
                contents.insert(inode.id.nid, data);
            }
            ensure(nodes.insert(inode.id.nid, inode).is_none())?;
        }
        ensure(
            nodes
                .get(&1)
                .is_some_and(|n| n.id == NodeId::ROOT && n.kind == 2),
        )?;
        ensure(nodes[&1].parent == NodeId::ROOT)?;
        if nodes.values().filter(|n| n.kind == 1).count() > MAX_FILES
            || nodes.values().filter(|n| n.kind == 2).count() > MAX_DIRECTORIES + 1
        {
            return Err(Error::Unsupported.into());
        }
        let mut orphan_ids = BTreeMap::new();
        for o in &orphans {
            let n = nodes.get(&o.id.nid).ok_or(FsError::Corrupt)?;
            ensure(
                n.id == o.id
                    && n.nlink == 0
                    && o.unlink_generation <= cp.generation
                    && orphan_ids.insert(o.id.nid, o.unlink_generation).is_none(),
            )?;
            if n.kind == 2 {
                ensure(directories[&n.id.nid].is_empty())?;
            }
        }
        let mut seen = BTreeSet::new();
        let mut stack = vec![(1u64, 0usize)];
        let mut files = Vec::new();
        while let Some((nid, depth)) = stack.pop() {
            ensure(seen.insert(nid) && !orphan_ids.contains_key(&nid))?;
            if depth > MAX_DEPTH {
                return Err(Error::Unsupported.into());
            }
            let n = nodes.get(&nid).ok_or(FsError::Corrupt)?;
            if n.kind == 1 {
                ensure(n.nlink == 1)?;
                continue;
            }
            let bindings = directories.get(&nid).ok_or(FsError::Corrupt)?;
            let mut nlink = 2;
            for b in bindings {
                if b.name.len() > MAX_NAME {
                    return Err(Error::Unsupported.into());
                }
                let child = nodes.get(&b.target.nid).ok_or(FsError::Corrupt)?;
                ensure(child.id == b.target && child.kind == b.kind)?;
                if child.kind == 2 {
                    ensure(child.parent == n.id)?;
                    nlink += 1;
                }
                files.push(File {
                    name: b.name.clone(),
                    id: child.id.clone(),
                    kind: child.kind,
                    parent: n.id.clone(),
                    orphan_generation: 0,
                    data: if child.kind == 1 {
                        contents.remove(&child.id.nid).ok_or(FsError::Corrupt)?
                    } else {
                        Vec::new()
                    },
                });
                stack.push((child.id.nid, depth + 1));
            }
            ensure(n.nlink == nlink)?;
        }
        for o in &orphans {
            let n = &nodes[&o.id.nid];
            files.push(File {
                name: Vec::new(),
                id: n.id.clone(),
                kind: n.kind,
                parent: if n.kind == 2 {
                    n.parent.clone()
                } else {
                    NodeId::ROOT
                },
                orphan_generation: o.unlink_generation,
                data: if n.kind == 1 {
                    contents.remove(&n.id.nid).ok_or(FsError::Corrupt)?
                } else {
                    Vec::new()
                },
            });
        }
        ensure(seen.len() + orphans.len() == nodes.len() && contents.is_empty())?;
        // Independently reconstruct all ownership before comparing every segment
        // bit and summary entry; counters alone cannot establish free space.
        let mut summaries = BTreeMap::new();
        for (i, s) in segments.iter().enumerate() {
            ensure(s.index == i as u64 && s.bitmap.len() == self.sb.segment_blocks as usize / 8)?;
            let mut next = s.summary.clone();
            for ordinal in 0..(s.live as usize).div_ceil(82) {
                match self.object(&next, 8, &NodeId::default(), 0, ordinal as u32)? {
                    Payload::Summary {
                        segment,
                        epoch,
                        ordinal: o,
                        next: n,
                        entries,
                    } => {
                        ensure(
                            segment == s.index
                                && epoch == s.epoch
                                && o == ordinal as u32
                                && entries.len() == (s.live as usize - ordinal * 82).min(82),
                        )?;
                        for e in entries {
                            ensure(e.offset < self.sb.segment_blocks)?;
                            let b = self.sb.segment_start
                                + s.index * self.sb.segment_blocks as u64
                                + e.offset as u64;
                            ensure(summaries.insert(b, e).is_none())?;
                        }
                        next = n;
                    }
                    _ => return Err(FsError::Corrupt),
                }
            }
            ensure(next.absent())?;
        }
        for s in &segments {
            for o in 0..self.sb.segment_blocks {
                let b = self.sb.segment_start + s.index * self.sb.segment_blocks as u64 + o as u64;
                let bit = s.bitmap[o as usize / 8] & (1 << (o % 8)) != 0;
                ensure(bit == self.owned.contains_key(&b))?;
                if let Some(owner) = self.owned.get(&b) {
                    ensure(owner.epoch == s.epoch)?;
                }
            }
        }
        ensure(summaries.len() == self.owned.len())?;
        for (b, o) in &self.owned {
            let s = summaries.get(b).ok_or(FsError::Corrupt)?;
            ensure(
                s.kind == o.kind
                    && s.owner == o.node
                    && s.logical_key == o.key
                    && s.content_generation == o.birth
                    && s.expected_crc == if o.kind == 0 { o.crc } else { 0 },
            )?;
        }
        ensure(
            cp.allocated_blocks == self.owned.len() as u64
                && cp.free_blocks
                    == self.sb.segment_count * self.sb.segment_blocks as u64 - cp.allocated_blocks
                && cp.live_nodes == nodes.len() as u64
                && cp.orphan_nodes == orphans.len() as u64,
        )?;
        Ok(Snapshot {
            slot,
            cp,
            owned: self.owned,
            files,
            inodes: nodes,
        })
    }
}
pub(super) fn inspect<D: BlockDevice>(device: &mut D) -> Result<Recovered> {
    let info = device.info();
    let mut supers = Vec::new();
    let mut damaged = false;
    for b in 0..2 {
        let bytes = raw(device, &info, b)?;
        match Page::decode(&bytes, b) {
            Ok(Page {
                uuid,
                payload: Payload::Superblock(sb),
                ..
            }) => supers.push((uuid, sb)),
            Err(Error::Unsupported) => return Err(Error::Unsupported.into()),
            _ => damaged = true,
        }
    }
    if supers.is_empty() {
        return Err(FsError::Corrupt);
    }
    if supers.len() == 2 {
        let mut a = supers[0].1.clone();
        let mut b = supers[1].1.clone();
        a.copy_index = 0;
        b.copy_index = 0;
        if a != b || supers[0].0 != supers[1].0 {
            return Err(FsError::Indeterminate);
        }
    }
    let (uuid, sb) = supers.remove(0);
    let geometry = Geometry::calculate(Parameters {
        capacity: info.capacity,
        logical_block: info.logical_block,
        fs_block: 4096,
        segment_blocks: sb.segment_blocks,
        checkpoint_slots: 3,
        maximum_nodes: sb.maximum_nodes,
        maximum_orphans: sb.maximum_orphans,
        cleaner_reserve: sb.cleaner_reserve,
    })?;
    ensure(
        geometry.volume_blocks == sb.volume_blocks
            && geometry.segment_start == sb.segment_start
            && geometry.segment_count == sb.segment_count
            && sb.transaction_bound >= geometry.transaction_bound
            && sb.transaction_bound <= sb.segment_count * sb.segment_blocks as u64,
    )?;
    let mut retained = Vec::with_capacity(3);
    for slot in 2..=4 {
        let bytes = raw(device, &info, slot)?;
        if bytes.iter().all(|&b| b == 0) {
            continue;
        }
        let candidate = (|| {
            let p = Page::decode(&bytes, slot)?;
            ensure(p.uuid == uuid)?;
            let cp = match p.payload {
                Payload::Checkpoint(cp) => cp,
                _ => return Err(FsError::Corrupt),
            };
            Scan {
                device,
                info,
                uuid,
                sb: &sb,
                generation: cp.generation,
                owned: BTreeMap::new(),
                reads: 0,
            }
            .closure(slot, cp)
        })();
        match candidate {
            Ok(snapshot) => retained.push(snapshot),
            Err(e @ FsError::Io(_)) | Err(e @ FsError::Indeterminate) => return Err(e),
            Err(e @ FsError::Invalid(Error::Unsupported))
            | Err(e @ FsError::Invalid(Error::Budget)) => return Err(e),
            Err(_) => damaged = true,
        }
    }
    if retained.is_empty() {
        return Err(FsError::Corrupt);
    }
    retained.sort_by_key(|s| s.cp.generation);
    if retained
        .windows(2)
        .any(|w| w[0].cp.generation == w[1].cp.generation)
    {
        return Err(FsError::Indeterminate);
    }
    let mut union = BTreeMap::new();
    for s in &retained {
        for (&b, o) in &s.owned {
            if let Some(old) = union.insert(b, o) {
                if old != o {
                    return Err(FsError::Indeterminate);
                }
            }
        }
    }
    let state = if !damaged && retained.len() >= 2 && sb.features.readonly == 0 {
        MountState::CleanWritable
    } else {
        MountState::DegradedReadOnly
    };
    Ok(Recovered {
        uuid,
        sb,
        geometry,
        state,
        retained,
    })
}
