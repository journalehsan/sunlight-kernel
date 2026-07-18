//! PTE-safe SWAP-1 slot identity packing.

pub const MAX_POOLS: usize = 32;
pub const SLOT_INDEX_BITS: u64 = 22;
pub const POOL_BITS: u64 = 5;
pub const GENERATION_BITS: u64 = 13;
pub const SLOT_INDEX_MASK: u64 = (1 << SLOT_INDEX_BITS) - 1;
pub const POOL_MASK: u64 = (1 << POOL_BITS) - 1;
pub const GENERATION_MASK: u64 = (1 << GENERATION_BITS) - 1;
pub const POOL_SHIFT: u64 = SLOT_INDEX_BITS;
pub const GENERATION_SHIFT: u64 = SLOT_INDEX_BITS + POOL_BITS;
pub const MAX_SLOT_GENERATION: u16 = GENERATION_MASK as u16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotId {
    raw: u64,
}

impl SlotId {
    pub fn new(pool: usize, index: usize, generation: u16) -> Option<Self> {
        if pool >= MAX_POOLS
            || index as u64 > SLOT_INDEX_MASK
            || generation == 0
            || generation > MAX_SLOT_GENERATION
        {
            return None;
        }
        Some(Self {
            raw: (u64::from(generation) << GENERATION_SHIFT)
                | ((pool as u64) << POOL_SHIFT)
                | index as u64,
        })
    }

    pub const fn raw(self) -> u64 {
        self.raw
    }

    pub fn from_raw(raw: u64) -> Option<Self> {
        if raw >> (GENERATION_SHIFT + GENERATION_BITS) != 0 {
            return None;
        }
        let generation = ((raw >> GENERATION_SHIFT) & GENERATION_MASK) as u16;
        (generation != 0).then_some(Self { raw })
    }

    pub const fn pool(self) -> usize {
        ((self.raw >> POOL_SHIFT) & POOL_MASK) as usize
    }

    pub const fn index(self) -> usize {
        (self.raw & SLOT_INDEX_MASK) as usize
    }

    pub const fn generation(self) -> u16 {
        ((self.raw >> GENERATION_SHIFT) & GENERATION_MASK) as u16
    }

    pub const fn matches_generation(self, current: u16) -> bool {
        self.generation() == current
    }
}
