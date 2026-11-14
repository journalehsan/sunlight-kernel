//! Full-table geometry and bounded fixed-point bootstrap reservation.
use crate::{Error, Result};
pub const BLOCK: u64 = 4096;
pub const MAX_PAGES: u64 = 82;
pub const MAX_NODES: u64 = 6724;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameters {
    pub capacity: u64,
    pub logical_block: u32,
    pub fs_block: u32,
    pub segment_blocks: u32,
    pub checkpoint_slots: u32,
    pub maximum_nodes: u64,
    pub maximum_orphans: u64,
    /// Zero selects the calculated minimum. Nonzero must meet that minimum.
    pub cleaner_reserve: u64,
}
impl Parameters {
    pub fn laboratory(capacity: u64, segment_blocks: u32) -> Self {
        Self {
            capacity,
            logical_block: 512,
            fs_block: 4096,
            segment_blocks,
            checkpoint_slots: 3,
            maximum_nodes: 128,
            maximum_orphans: 32,
            cleaner_reserve: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub volume_blocks: u64,
    pub segment_start: u64,
    pub segment_count: u64,
    pub segment_blocks: u32,
    pub maximum_nodes: u64,
    pub maximum_orphans: u64,
    pub segment_record_bytes: u64,
    pub segment_table_pages: u64,
    pub bootstrap_blocks: u64,
    pub summary_pages: u64,
    pub metadata_bound: u64,
    pub cleaner_reserve: u64,
    pub transaction_bound: u64,
    pub minimum_blocks: u64,
    pub tail_bytes: u64,
    pub fixed_point_iterations: u32,
}
pub fn ceil(n: u64, d: u64) -> Result<u64> {
    n.checked_add(d.checked_sub(1).ok_or(Error::Geometry)?)
        .ok_or(Error::Overflow)
        .map(|v| v / d)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(Error::Overflow)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(Error::Overflow)
}
/// Dense placement from offset zero. Summary pages own themselves. Monotone and
/// bounded: adding a page can cross a segment or summary capacity boundary.
pub fn summary_fixed_point(base: u64, segment_blocks: u64) -> Result<(u64, u32)> {
    if ![64, 256, 512].contains(&segment_blocks) {
        return Err(Error::Geometry);
    }
    let mut summaries = 0;
    for iteration in 1..=128 {
        let total = add(base, summaries)?;
        let full = total / segment_blocks;
        let tail = total % segment_blocks;
        let next = add(mul(full, ceil(segment_blocks, 82)?)?, ceil(tail, 82)?)?;
        if next == summaries {
            return Ok((summaries, iteration));
        }
        summaries = next;
    }
    Err(Error::Budget)
}
impl Geometry {
    pub fn calculate(p: Parameters) -> Result<Self> {
        if p.fs_block != 4096
            || ![64, 256, 512].contains(&p.segment_blocks)
            || !p.logical_block.is_power_of_two()
            || !(512..=4096).contains(&p.logical_block)
            || !p.capacity.is_multiple_of(p.logical_block as u64)
            || p.checkpoint_slots != 3
            || p.maximum_nodes == 0
            || p.maximum_nodes > MAX_NODES
            || p.maximum_orphans == 0
            || p.maximum_orphans > p.maximum_nodes
        {
            return Err(Error::Geometry);
        }
        let bps = p.segment_blocks as u64;
        let volume_blocks = p.capacity / BLOCK;
        let segment_start = bps;
        let segment_count = volume_blocks
            .checked_sub(segment_start)
            .ok_or(Error::Capacity)?
            / bps;
        let segment_record_bytes = 64 + bps / 8;
        let per_page = 3976 / segment_record_bytes;
        if segment_count == 0 {
            return Err(Error::Capacity);
        }
        if segment_count > MAX_PAGES * per_page {
            return Err(Error::Budget);
        }
        let segment_table_pages = ceil(segment_count, per_page)?;
        // node, directory root, node-map page/root, orphan root, segment root/pages.
        let base = add(6, segment_table_pages)?;
        let (summary_pages, fixed_point_iterations) = summary_fixed_point(base, bps)?;
        let bootstrap_blocks = add(base, summary_pages)?;
        let maximum_base = add(
            add(
                add(mul(p.maximum_nodes, 2)?, 3)?,
                ceil(p.maximum_nodes, 82)?,
            )?,
            add(segment_table_pages, ceil(p.maximum_orphans, 124)?)?,
        )?;
        let metadata_bound = add(maximum_base, summary_fixed_point(maximum_base, bps)?.0)?;
        // Explicit admission budget; not a future arbitrary mutation/cleaner proof.
        let transaction_bound = add(metadata_bound, bps)?;
        let minimum_reserve = add(add(bps, mul(metadata_bound, 3)?)?, 3)?;
        let cleaner_reserve = if p.cleaner_reserve == 0 {
            minimum_reserve
        } else {
            p.cleaner_reserve
        };
        if cleaner_reserve < minimum_reserve {
            return Err(Error::Reserve);
        }
        let minimum_blocks = add(
            segment_start,
            mul(ceil(add(bootstrap_blocks, cleaner_reserve)?, bps)?, bps)?,
        )?;
        if volume_blocks < minimum_blocks {
            return Err(Error::Capacity);
        }
        let used_bytes = mul(add(segment_start, mul(segment_count, bps)?)?, BLOCK)?;
        Ok(Self {
            volume_blocks,
            segment_start,
            segment_count,
            segment_blocks: p.segment_blocks,
            maximum_nodes: p.maximum_nodes,
            maximum_orphans: p.maximum_orphans,
            segment_record_bytes,
            segment_table_pages,
            bootstrap_blocks,
            summary_pages,
            metadata_bound,
            cleaner_reserve,
            transaction_bound,
            minimum_blocks,
            tail_bytes: p.capacity - used_bytes,
            fixed_point_iterations,
        })
    }
    pub fn segment_end(&self) -> u64 {
        self.segment_start + self.segment_count * self.segment_blocks as u64
    }
    pub fn contains(&self, block: u64) -> bool {
        block >= self.segment_start && block < self.segment_end()
    }
    pub fn segment(&self, block: u64) -> Result<(u64, u32)> {
        if !self.contains(block) {
            return Err(Error::Reference);
        }
        let relative = block - self.segment_start;
        Ok((
            relative / self.segment_blocks as u64,
            (relative % self.segment_blocks as u64) as u32,
        ))
    }
}
