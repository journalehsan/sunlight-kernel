//! Empty-image dependency plan and fail-stop D0/P0 publication only.
use crate::{
    block::*,
    codec::*,
    geometry::{Geometry, BLOCK},
    Error, Result,
};
use alloc::{vec, vec::Vec};
#[derive(Clone, Debug)]
pub struct Bootstrap {
    pub geometry: Geometry,
    pub superblocks: Vec<Page>,
    pub dependencies: Vec<Page>,
    pub checkpoints: Vec<Page>,
}
#[derive(Clone)]
struct Identity {
    kind: u16,
    owner: NodeId,
    key: u64,
}
fn global(kind: u16, table: u16, ordinal: u32) -> Identity {
    Identity {
        kind,
        owner: NodeId::default(),
        key: ((kind as u64) << 48) | ((table as u64) << 32) | ordinal as u64,
    }
}
fn make(uuid: [u8; 16], block: u64, payload: Payload) -> Page {
    Page {
        uuid,
        block,
        epoch: 1,
        birth: 1,
        payload,
    }
}
fn descriptor(page: &Page, first: u64, last: u64, count: u32) -> Result<Descriptor> {
    Ok(Descriptor {
        first,
        last,
        page: page.reference()?,
        count,
        ..Default::default()
    })
}
impl Bootstrap {
    pub fn plan(g: Geometry, uuid: [u8; 16]) -> Result<Self> {
        if uuid == [0; 16] {
            return Err(Error::Structure);
        }
        let capacity = g
            .volume_blocks
            .checked_mul(BLOCK)
            .and_then(|v| v.checked_add(g.tail_bytes % BLOCK))
            .ok_or(Error::Overflow)?;
        let validated = Geometry::calculate(crate::geometry::Parameters {
            capacity,
            logical_block: 512,
            fs_block: 4096,
            segment_blocks: g.segment_blocks,
            checkpoint_slots: 3,
            maximum_nodes: g.maximum_nodes,
            maximum_orphans: g.maximum_orphans,
            cleaner_reserve: g.cleaner_reserve,
        })?;
        if validated != g {
            return Err(Error::Geometry);
        }
        let n = usize::try_from(g.bootstrap_blocks).map_err(|_| Error::Overflow)?;
        let base = 6 + g.segment_table_pages as usize;
        let start = g.segment_start;
        let mut identities = vec![global(0, 0, 0); n];
        identities[0] = Identity {
            kind: 5,
            owner: NodeId::ROOT,
            key: 5 << 48,
        };
        identities[1] = Identity {
            kind: 3,
            owner: NodeId::ROOT,
            key: (3 << 48) | (4 << 32),
        };
        identities[2] = global(4, 1, 0);
        identities[3] = global(3, 1, 0);
        identities[4] = global(3, 3, 0);
        identities[5] = global(3, 2, 0);
        for i in 0..g.segment_table_pages as usize {
            identities[6 + i] = global(4, 2, i as u32);
        }
        // Assign all summary locations before encoding; summarize self ownership too.
        let live_segments = (n as u64).div_ceil(g.segment_blocks as u64);
        let mut summaries = Vec::new();
        let mut cursor = base;
        for segment in 0..live_segments {
            let first = segment * g.segment_blocks as u64;
            let count = (n as u64 - first).min(g.segment_blocks as u64);
            for ordinal in 0..count.div_ceil(82) {
                if cursor >= n {
                    return Err(Error::Budget);
                }
                identities[cursor] = global(8, 0, ordinal as u32);
                summaries.push((segment, ordinal as u32, cursor));
                cursor += 1;
            }
        }
        if cursor != n {
            return Err(Error::Budget);
        }
        let mut pages: Vec<Option<Page>> = vec![None; n];
        let dir = make(uuid, start + 1, empty_root(4, 0));
        let inode = Inode {
            id: NodeId::ROOT,
            kind: 2,
            mode: 0o755,
            nlink: 2,
            content_generation: 1,
            parent: NodeId::ROOT,
            directory: dir.reference()?,
            ..Default::default()
        };
        let node = make(uuid, start, Payload::Inode(inode));
        let map_page = make(
            uuid,
            start + 2,
            Payload::Table {
                first: 1,
                last: 1,
                records: Records::Map(vec![MapEntry {
                    id: NodeId::ROOT,
                    node: node.reference()?,
                    kind: 2,
                    ..Default::default()
                }]),
            },
        );
        let map_root = make(
            uuid,
            start + 3,
            Payload::Root {
                kind: 1,
                count: 1,
                entry_bytes: 48,
                pages: vec![descriptor(&map_page, 1, 1, 1)?],
            },
        );
        let orphan = make(uuid, start + 4, empty_root(3, 32));
        pages[0] = Some(node);
        pages[1] = Some(dir);
        pages[2] = Some(map_page);
        pages[3] = Some(map_root);
        pages[4] = Some(orphan);
        let mut summary_heads = vec![BlockRef::default(); live_segments as usize];
        for &(segment, ordinal, index) in summaries.iter().rev() {
            let first = segment * g.segment_blocks as u64 + ordinal as u64 * 82;
            let end = (first + 82).min(((segment + 1) * g.segment_blocks as u64).min(n as u64));
            let mut entries = Vec::new();
            for i in first..end {
                let id = &identities[i as usize];
                entries.push(SummaryEntry {
                    offset: (i % g.segment_blocks as u64) as u32,
                    kind: id.kind,
                    owner: id.owner.clone(),
                    logical_key: id.key,
                    content_generation: 1,
                    expected_crc: 0,
                    ..Default::default()
                });
            }
            let p = make(
                uuid,
                start + index as u64,
                Payload::Summary {
                    segment,
                    epoch: 1,
                    ordinal,
                    next: summary_heads[segment as usize].clone(),
                    entries,
                },
            );
            summary_heads[segment as usize] = p.reference()?;
            pages[index] = Some(p);
        }
        let mut records = Vec::new();
        for segment in 0..g.segment_count {
            let count = (n as u64)
                .saturating_sub(segment * g.segment_blocks as u64)
                .min(g.segment_blocks as u64) as u32;
            let mut bitmap = vec![0u8; g.segment_blocks as usize / 8];
            for bit in 0..count {
                bitmap[bit as usize / 8] |= 1 << (bit % 8);
            }
            records.push(Segment {
                index: segment,
                epoch: if count > 0 { 1 } else { 0 },
                state: if count > 0 { 1 } else { 0 },
                live: count,
                next_hint: count,
                summary: summary_heads
                    .get(segment as usize)
                    .cloned()
                    .unwrap_or_default(),
                bitmap,
            });
        }
        let per_page = 3976 / g.segment_record_bytes as usize;
        let mut descriptors = Vec::new();
        for (i, chunk) in records.chunks(per_page).enumerate() {
            let first = chunk.first().ok_or(Error::Structure)?.index;
            let last = chunk.last().ok_or(Error::Structure)?.index;
            let p = make(
                uuid,
                start + 6 + i as u64,
                Payload::Table {
                    first,
                    last,
                    records: Records::Segments(chunk.to_vec()),
                },
            );
            descriptors.push(descriptor(&p, first, last, chunk.len() as u32)?);
            pages[6 + i] = Some(p);
        }
        pages[5] = Some(make(
            uuid,
            start + 5,
            Payload::Root {
                kind: 2,
                count: g.segment_count,
                entry_bytes: g.segment_record_bytes as u32,
                pages: descriptors,
            },
        ));
        let dependencies: Vec<Page> = pages
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(Error::Structure)?;
        let mut superblocks = Vec::new();
        for copy in 0..2 {
            superblocks.push(Page {
                uuid,
                block: copy,
                epoch: 0,
                birth: 0,
                payload: Payload::Superblock(Superblock {
                    major: 0,
                    minor: 2,
                    reader_major: 0,
                    reader_minor: 2,
                    features: Features {
                        incompatible: 3,
                        ..Default::default()
                    },
                    block_bytes: 4096,
                    segment_blocks: g.segment_blocks,
                    volume_blocks: g.volume_blocks,
                    segment_start: g.segment_start,
                    segment_count: g.segment_count,
                    cp0: 2,
                    cp1: 3,
                    cp2: 4,
                    maximum_nodes: g.maximum_nodes,
                    maximum_orphans: g.maximum_orphans,
                    maximum_table_pages: 82,
                    maximum_extents: 6724,
                    cleaner_reserve: g.cleaner_reserve,
                    transaction_bound: g.transaction_bound,
                    copy_index: copy as u32,
                    ..Default::default()
                }),
            });
        }
        let mut checkpoints = Vec::new();
        for generation in 1..=2 {
            checkpoints.push(Page {
                uuid,
                block: 1 + generation,
                epoch: 0,
                birth: generation,
                payload: Payload::Checkpoint(Checkpoint {
                    generation,
                    predecessor: generation - 1,
                    root: NodeId::ROOT,
                    node_map: dependencies[3].reference()?,
                    segment_table: dependencies[5].reference()?,
                    orphan_table: dependencies[4].reference()?,
                    next_nid: 2,
                    allocated_blocks: n as u64,
                    free_blocks: g.segment_count * g.segment_blocks as u64 - n as u64,
                    live_nodes: 1,
                    orphan_nodes: 0,
                    transaction_blocks: n as u64,
                    state: 1,
                    ..Default::default()
                }),
            });
        }
        Ok(Self {
            geometry: g,
            superblocks,
            dependencies,
            checkpoints,
        })
    }
    /// Requires fresh zero-filled, exclusively owned disposable storage. On failure
    /// return immediately: no CP after a failed D0, no retry or state advancement.
    pub fn publish<D: BlockDevice>(&self, device: &mut D) -> IoResult<()> {
        let info = device.info();
        info.validate()?;
        if info.read_only {
            return Err(IoFailure::known(IoKind::ReadOnly, 0));
        }
        if !info.flush {
            return Err(IoFailure::known(IoKind::UnsupportedDurability, 0));
        }
        if info.capacity / BLOCK != self.geometry.volume_blocks {
            return Err(Error::Geometry.into());
        }
        let incarnation = info.incarnation;
        for page in self.superblocks.iter().chain(&self.dependencies) {
            if device.info().incarnation != incarnation {
                return Err(IoFailure::uncertain(IoKind::Offline));
            }
            device.write(
                page.block.checked_mul(BLOCK).ok_or(Error::Overflow)?,
                &page.encode()?,
            )?;
        }
        device.flush()?;
        for page in &self.checkpoints {
            if device.info().incarnation != incarnation {
                return Err(IoFailure::uncertain(IoKind::Offline));
            }
            device.write(
                page.block.checked_mul(BLOCK).ok_or(Error::Overflow)?,
                &page.encode()?,
            )?;
        }
        device.flush()?;
        if device.info().incarnation != incarnation {
            return Err(IoFailure::uncertain(IoKind::Offline));
        }
        Ok(())
    }
}
