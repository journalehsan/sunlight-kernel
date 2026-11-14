//! Independent cross-image relocation qualification. Only byte codecs and the
//! checker's own closure walker supply authority; no engine/cleaner imports.
use super::*;

#[derive(Clone, Debug, Default)]
pub struct RelocationManifest {
    pub reserved: Vec<u64>,
    pub runtime_pins: Vec<NodeId>,
    pub quarantined: Vec<u64>,
    pub recovery_floor: u64,
}
#[derive(Clone, Debug)]
pub struct RelocationAudit {
    pub initial_reusable: u64,
    pub final_reusable: u64,
    pub relocated_blocks: u64,
    pub released_blocks: u64,
    pub protected_blocks: u64,
    pub reserved_segment_blocks: u64,
    pub reserve_used: u64,
    pub recovery_floor: u64,
}
fn recovery_floor(s: &Superblock) -> Result<u64> {
    // Independent format arithmetic, not the engine geometry implementation.
    let bps = s.segment_blocks as u64;
    let pages = s.segment_count.div_ceil(3976 / (64 + bps / 8));
    let base = s
        .maximum_nodes
        .checked_mul(2)
        .and_then(|n| n.checked_add(3))
        .and_then(|n| n.checked_add(s.maximum_nodes.div_ceil(82)))
        .and_then(|n| n.checked_add(pages))
        .and_then(|n| n.checked_add(s.maximum_orphans.div_ceil(124)))
        .ok_or(Failure::Invalid("reserve overflow"))?;
    let mut summaries = 0;
    for _ in 0..128 {
        let total = base
            .checked_add(summaries)
            .ok_or(Failure::Invalid("reserve overflow"))?;
        let next = (total / bps)
            .checked_mul(bps.div_ceil(82))
            .and_then(|n| n.checked_add((total % bps).div_ceil(82)))
            .ok_or(Failure::Invalid("reserve overflow"))?;
        if next == summaries {
            return total
                .checked_mul(2)
                .and_then(|n| n.checked_add(3))
                .ok_or(Failure::Invalid("reserve overflow"));
        }
        summaries = next;
    }
    Err(Failure::Budget)
}
fn sb(image: &[u8]) -> Result<Superblock> {
    match Page::decode(&Bytes(image).read_page(0)?, 0)?.payload {
        Payload::Superblock(s) => Ok(s),
        _ => Err(Failure::Invalid("superblock")),
    }
}
fn available(s: &Superblock, blocks: &BTreeSet<u64>) -> Result<u64> {
    let mut segments = BTreeSet::new();
    for b in blocks {
        let rel = b
            .checked_sub(s.segment_start)
            .ok_or(Failure::Invalid("reserved ownership"))?;
        let segment = rel / (s.segment_blocks as u64);
        require(segment < s.segment_count, "ownership outside segments")?;
        segments.insert(segment);
    }
    s.segment_count
        .checked_sub(segments.len() as u64)
        .and_then(|n| n.checked_mul(s.segment_blocks as u64))
        .ok_or(Failure::Invalid("capacity overflow"))
}
fn logical(image: &[u8], n: &CheckedNode) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    for e in &n.extents {
        let start = usize::try_from(e.physical)
            .ok()
            .and_then(|b| b.checked_mul(PAGE))
            .ok_or(Failure::Read)?;
        let bytes = image
            .get(start..start.checked_add(PAGE).ok_or(Failure::Read)?)
            .ok_or(Failure::Read)?;
        data.extend_from_slice(bytes);
    }
    if n.kind == 1 {
        require(data.len() >= n.size as usize, "missing relocation data")?;
        data.truncate(n.size as usize);
    }
    Ok(data)
}
/// Certifies an acknowledged single-generation relocation. Runtime pins and
/// quarantine are supplied by the fixture manifest, never inferred from cold media.
pub fn check_relocation(
    before: &[u8],
    after: &[u8],
    m: &RelocationManifest,
) -> Result<RelocationAudit> {
    let a = check(&mut Bytes(before));
    let b = check(&mut Bytes(after));
    require(
        a.status == Status::Clean && b.status == Status::Clean,
        "complete clean closures required",
    )?;
    let old = a
        .generations
        .iter()
        .find(|g| Some(g.generation) == a.selected)
        .ok_or(Failure::Invalid("old selection"))?;
    let new = b
        .generations
        .iter()
        .find(|g| Some(g.generation) == b.selected)
        .ok_or(Failure::Invalid("new selection"))?;
    require(
        old.generation.checked_add(1) == Some(new.generation),
        "relocation generation transition",
    )?;
    let s = sb(before)?;
    require(s == sb(after)?, "geometry changed")?;
    let floor = recovery_floor(&s)?;
    require(
        m.recovery_floor == floor,
        "incorrect recovery floor manifest",
    )?;
    require(
        old.nodes.len() == new.nodes.len() && old.orphans == new.orphans,
        "relocation node/orphan loss",
    )?;
    for n in &old.nodes {
        let next = new
            .nodes
            .iter()
            .find(|v| v.id == n.id)
            .ok_or(Failure::Invalid("relocated identity lost"))?;
        let mut logical_before = n.inode.clone();
        let mut logical_after = next.inode.clone();
        logical_before.extents = BlockRef::default();
        logical_after.extents = BlockRef::default();
        logical_before.directory = BlockRef::default();
        logical_after.directory = BlockRef::default();
        require(logical_before == logical_after, "inode attributes changed")?;
        require(
            n.kind == next.kind
                && n.size == next.size
                && n.parent == next.parent
                && n.nlink == next.nlink
                && n.entries == next.entries
                && n.content_generation == next.content_generation,
            "logical metadata changed",
        )?;
        require(
            logical(before, n)? == logical(after, next)?,
            "relocated bytes changed",
        )?;
        require(
            n.extents.len() == next.extents.len(),
            "relocated extent count",
        )?;
        for (x, y) in n.extents.iter().zip(&next.extents) {
            require(
                x.logical == y.logical
                    && x.blocks == y.blocks
                    && x.content_generation == y.content_generation
                    && x.physical != y.physical,
                "extent identity or mapping changed incorrectly",
            )?;
        }
    }
    for pin in &m.runtime_pins {
        require(
            old.nodes.iter().any(|n| n.id == *pin) && new.nodes.iter().any(|n| n.id == *pin),
            "runtime pin lost",
        )?;
    }
    let initial: BTreeSet<_> = a
        .generations
        .iter()
        .flat_map(|g| g.blocks.iter().copied())
        .collect();
    let final_blocks: BTreeSet<_> = b
        .generations
        .iter()
        .flat_map(|g| g.blocks.iter().copied())
        .collect();
    let targets: BTreeSet<_> = m.reserved.iter().copied().collect();
    require(
        targets.len() == m.reserved.len(),
        "duplicate relocation reservation",
    )?;
    require(
        targets == new.blocks.iter().copied().collect(),
        "unreserved relocation mapping",
    )?;
    require(
        targets.is_disjoint(&initial),
        "premature protected source reuse",
    )?;
    let initial_segments: BTreeSet<_> = initial
        .iter()
        .map(|b| (b - s.segment_start) / s.segment_blocks as u64)
        .collect();
    let target_segments: BTreeSet<_> = targets
        .iter()
        .map(|b| (b - s.segment_start) / s.segment_blocks as u64)
        .collect();
    require(
        target_segments.is_disjoint(&initial_segments),
        "protected segment tail reused",
    )?;
    let quarantine: BTreeSet<_> = m.quarantined.iter().copied().collect();
    require(
        targets.is_disjoint(&quarantine),
        "quarantined relocation target",
    )?;
    for &block in &initial {
        require(
            Bytes(before).read_page(block)? == Bytes(after).read_page(block)?,
            "protected source overwritten",
        )?;
    }
    // A newer root may remove only generations older than the previous newest.
    // Every surviving historical generation must retain exactly its closure.
    require(
        b.generations.len() == 2,
        "acknowledged retained generation count",
    )?;
    for g in &b.generations {
        if g.generation == new.generation {
            continue;
        }
        let prior = a
            .generations
            .iter()
            .find(|p| p.generation == g.generation)
            .ok_or(Failure::Invalid("invented fallback"))?;
        require(
            g.generation == old.generation && g.blocks == prior.blocks,
            "fallback changed or released early",
        )?;
    }
    let occupied: BTreeSet<_> = initial.union(&targets).copied().chain(quarantine).collect();
    let need = (target_segments.len() as u64)
        .checked_mul(s.segment_blocks as u64)
        .ok_or(Failure::Invalid("target capacity overflow"))?;
    require(
        need <= s
            .cleaner_reserve
            .checked_sub(floor)
            .ok_or(Failure::Invalid("reserve floor"))?,
        "cleaner tranche exceeded",
    )?;
    require(
        available(&s, &occupied)? >= m.recovery_floor,
        "recovery floor consumed at admission",
    )?;
    Ok(RelocationAudit {
        initial_reusable: available(&s, &initial)?,
        final_reusable: available(&s, &final_blocks)?,
        relocated_blocks: targets.len() as u64,
        released_blocks: initial.difference(&final_blocks).count() as u64,
        protected_blocks: final_blocks.len() as u64,
        reserved_segment_blocks: need,
        reserve_used: need
            .saturating_sub(available(&s, &initial)?.saturating_sub(s.cleaner_reserve)),
        recovery_floor: floor,
    })
}
