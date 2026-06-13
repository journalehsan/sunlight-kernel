use alloc::vec::Vec;

pub const ZRAM_BLOCK_SIZE: usize = 4096;
pub const ZRAM_CAPACITY_MB: usize = 256;
pub const ZRAM_BLOCK_COUNT: usize = (ZRAM_CAPACITY_MB * 1024 * 1024) / ZRAM_BLOCK_SIZE;

pub type BlockId = usize;

#[derive(Debug, Clone, Copy)]
pub enum ZramError {
    OutOfSpace,
    InvalidBlock,
    InvalidData,
    NoData,
}

#[derive(Default)]
struct ZramState {
    slots: Vec<(BlockId, Option<Vec<u8>>)>,
    next_free: usize,
}

impl ZramState {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            next_free: 0,
        }
    }

    fn free_slots(&self) -> usize {
        ZRAM_BLOCK_COUNT.saturating_sub(self.slots.len())
    }

    fn allocate_slot(&mut self) -> Option<BlockId> {
        if self.slots.len() >= ZRAM_BLOCK_COUNT {
            return None;
        }

        let start = self.next_free;
        for _ in 0..ZRAM_BLOCK_COUNT {
            let idx = self.next_free;
            self.next_free = (self.next_free + 1) % ZRAM_BLOCK_COUNT;

            if self.find_slot(idx).is_none() {
                self.slots.push((idx, None));
                return Some(idx);
            }
        }

        self.next_free = start;
        None
    }

    fn find_slot(&self, id: BlockId) -> Option<usize> {
        self.slots.iter().position(|(slot_id, _)| *slot_id == id)
    }
}

static ZRAM: spin::Lazy<spin::Mutex<ZramState>> =
    spin::Lazy::new(|| spin::Mutex::new(ZramState::new()));

fn compress_page(src: &[u8; ZRAM_BLOCK_SIZE]) -> Result<Vec<u8>, ()> {
    let mut out = Vec::with_capacity(ZRAM_BLOCK_SIZE);
    out.extend_from_slice(src);
    Ok(out)
}

fn decompress_page(src: &[u8], dst: &mut [u8; ZRAM_BLOCK_SIZE]) -> Result<(), ()> {
    if src.len() > ZRAM_BLOCK_SIZE {
        return Err(());
    }

    let len = src.len().min(ZRAM_BLOCK_SIZE);
    dst[..len].copy_from_slice(&src[..len]);
    dst[len..].fill(0);
    Ok(())
}

pub fn init() {
    let _ = &*ZRAM;
}

pub fn alloc_block() -> Result<BlockId, ZramError> {
    let mut state = ZRAM.lock();
    state.allocate_slot().ok_or(ZramError::OutOfSpace)
}

pub fn free_slots() -> usize {
    let state = ZRAM.lock();
    state.free_slots()
}

pub fn write_block(id: BlockId, data: &[u8; ZRAM_BLOCK_SIZE]) -> Result<(), ZramError> {
    if id >= ZRAM_BLOCK_COUNT {
        return Err(ZramError::InvalidBlock);
    }

    let mut state = ZRAM.lock();
    let page = compress_page(data).map_err(|_| ZramError::InvalidData)?;

    if let Some(slot_idx) = state.find_slot(id) {
        state.slots[slot_idx].1 = Some(page);
    } else {
        state.slots.push((id, Some(page)));
    }
    Ok(())
}

pub fn write_page(data: &[u8; ZRAM_BLOCK_SIZE]) -> Result<BlockId, ZramError> {
    let id = alloc_block()?;
    let mut state = ZRAM.lock();
    let page = compress_page(data).map_err(|_| ZramError::InvalidData)?;
    let slot_idx = state.find_slot(id).ok_or(ZramError::InvalidBlock)?;
    state.slots[slot_idx].1 = Some(page);
    // alloc_block() increments used_slots only when it returns an empty slot.
    Ok(id)
}

pub fn read_block(id: BlockId, out: &mut [u8; ZRAM_BLOCK_SIZE]) -> Result<(), ZramError> {
    let state = ZRAM.lock();
    match state
        .find_slot(id)
        .and_then(|slot_idx| state.slots[slot_idx].1.as_ref())
    {
        Some(page) => decompress_page(page, out).map_err(|_| ZramError::InvalidData),
        None => Err(ZramError::NoData),
    }
}

pub fn discard_block(id: BlockId) -> Result<(), ZramError> {
    let mut state = ZRAM.lock();
    if id >= ZRAM_BLOCK_COUNT {
        return Err(ZramError::InvalidBlock);
    }
    let slot_idx = state.find_slot(id).ok_or(ZramError::NoData)?;
    state.slots.swap_remove(slot_idx);
    Ok(())
}

pub fn stats() -> (usize, usize, usize) {
    let state = ZRAM.lock();
    let used = state.slots.len();
    let total = ZRAM_BLOCK_COUNT;
    let used_bytes = state
        .slots
        .iter()
        .filter_map(|(_, page)| page.as_ref())
        .map(Vec::len)
        .sum::<usize>();
    (total, used, used_bytes)
}
