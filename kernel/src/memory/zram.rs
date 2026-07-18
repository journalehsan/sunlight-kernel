//! Bounded, immutable-after-activation, multi-pool in-memory swap storage.

use super::pmm::PhysicalMemoryManager;
pub use super::swap_slot::SlotId;
use super::swap_slot::{MAX_SLOT_GENERATION, SLOT_INDEX_MASK};
use super::zram_codec::{self, CodecError, MAX_COMPRESSED_SIZE, PAGE_SIZE};
use ::sunlight_ipc::swap_policy::{SwapPolicy, MAX_POOLS, POLICY_VERSION};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;
use x86_64::{PhysAddr, VirtAddr};

pub const ZRAM_BLOCK_SIZE: usize = PAGE_SIZE;
pub const MAX_GLOBAL_SLOTS: usize = 16 * 1024;
const STORAGE_CHUNK_BYTES: usize = 64;
const STORAGE_CHUNKS_PER_PAGE: usize = PAGE_SIZE / STORAGE_CHUNK_BYTES;

pub type BlockId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZramError {
    NotConfigured,
    AlreadyConfigured,
    InvalidPolicy,
    OutOfSpace,
    PhysicalBudgetExceeded,
    AllocationFailure,
    MetadataExhausted,
    Incompressible,
    InvalidBlock,
    InvalidData,
    NoData,
    StaleSlot,
    ChecksumMismatch,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PoolStats {
    pub logical_capacity_pages: u64,
    pub physical_budget_bytes: u64,
    pub used_logical_pages: u64,
    pub used_compressed_bytes: u64,
    pub allocator_consumed_bytes: u64,
    pub compression_successes: u64,
    pub compression_failures: u64,
    pub raw_pages: u64,
    pub incompressible_rejected: u64,
    pub swap_out_attempts: u64,
    pub swap_out_successes: u64,
    pub swap_out_failures: u64,
    pub swap_in_attempts: u64,
    pub swap_in_successes: u64,
    pub swap_in_failures: u64,
    pub checksum_failures: u64,
    pub decompression_failures: u64,
    pub full_events: u64,
    pub budget_full_events: u64,
    pub slot_releases: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AggregateStats {
    pub active_pool_count: usize,
    pub configured_logical_pages: u64,
    pub configured_physical_budget_bytes: u64,
    pub stored_pages: u64,
    pub compressed_bytes: u64,
    pub allocator_consumed_bytes: u64,
    pub pages_stored_raw: u64,
    pub incompressible_rejected: u64,
    pub swap_out_attempts: u64,
    pub swap_out_successes: u64,
    pub swap_out_failures: u64,
    pub swap_in_attempts: u64,
    pub swap_in_successes: u64,
    pub swap_in_failures: u64,
    pub checksum_failures: u64,
    pub decompression_failures: u64,
    pub full_pool_events: u64,
    pub budget_full_events: u64,
    pub fallback_to_next_pool: u64,
    pub service_configured: bool,
    pub admin_owner_alive: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct PoolConfig {
    identity: u8,
    logical_capacity_pages: u64,
    physical_budget_bytes: u64,
    slot_limit: usize,
}

struct Slot {
    generation: u16,
    checksum: u32,
    pte_flags: u64,
    payload: Option<StoredPayload>,
}

impl Slot {
    const fn vacant(generation: u16) -> Self {
        Self {
            generation,
            checksum: 0,
            pte_flags: 0,
            payload: None,
        }
    }
}

#[derive(Clone, Copy)]
struct StoredPayload {
    storage_page_index: u32,
    first_chunk: u8,
    chunk_count: u8,
    compressed_len: u16,
}

#[derive(Clone, Copy)]
struct StoragePage {
    frame: Option<PhysAddr>,
    allocated_chunks: u64,
}

impl StoragePage {
    const fn vacant() -> Self {
        Self {
            frame: None,
            allocated_chunks: 0,
        }
    }
}

struct PoolState {
    config: PoolConfig,
    slots: Vec<Slot>,
    free: Vec<u32>,
    storage_pages: Vec<StoragePage>,
    stats: PoolStats,
}

impl PoolState {
    const fn new() -> Self {
        Self {
            config: PoolConfig {
                identity: 0,
                logical_capacity_pages: 0,
                physical_budget_bytes: 0,
                slot_limit: 0,
            },
            slots: Vec::new(),
            free: Vec::new(),
            storage_pages: Vec::new(),
            stats: PoolStats {
                logical_capacity_pages: 0,
                physical_budget_bytes: 0,
                used_logical_pages: 0,
                used_compressed_bytes: 0,
                allocator_consumed_bytes: 0,
                compression_successes: 0,
                compression_failures: 0,
                raw_pages: 0,
                incompressible_rejected: 0,
                swap_out_attempts: 0,
                swap_out_successes: 0,
                swap_out_failures: 0,
                swap_in_attempts: 0,
                swap_in_successes: 0,
                swap_in_failures: 0,
                checksum_failures: 0,
                decompression_failures: 0,
                full_events: 0,
                budget_full_events: 0,
                slot_releases: 0,
            },
        }
    }

    fn configure(&mut self, config: PoolConfig) {
        debug_assert!(self.slots.is_empty());
        self.config = config;
        self.stats.logical_capacity_pages = config.logical_capacity_pages;
        self.stats.physical_budget_bytes = config.physical_budget_bytes;
    }

    fn allocate_index(&mut self) -> Result<(usize, u16), ZramError> {
        while let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            if slot.generation < MAX_SLOT_GENERATION {
                slot.generation += 1;
                return Ok((index as usize, slot.generation));
            }
        }
        if self.slots.len() >= self.config.slot_limit
            || self.slots.len() as u64 >= self.config.logical_capacity_pages
            || self.slots.len() as u64 > SLOT_INDEX_MASK
        {
            return Err(ZramError::MetadataExhausted);
        }
        self.slots
            .try_reserve(1)
            .map_err(|_| ZramError::AllocationFailure)?;
        // Reserve the future discard push before publishing this index. Once a
        // slot exists, release can therefore never require a heap allocation.
        self.free
            .try_reserve(1)
            .map_err(|_| ZramError::AllocationFailure)?;
        let index = self.slots.len();
        self.slots.push(Slot::vacant(1));
        Ok((index, 1))
    }

    fn allocate_payload(
        &mut self,
        payload: &[u8],
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<StoredPayload, ZramError> {
        let chunk_count = payload.len().div_ceil(STORAGE_CHUNK_BYTES);
        if chunk_count == 0 || chunk_count > STORAGE_CHUNKS_PER_PAGE {
            return Err(ZramError::InvalidData);
        }

        for (page_index, storage_page) in self.storage_pages.iter_mut().enumerate() {
            let Some(frame) = storage_page.frame else {
                continue;
            };
            if let Some(first_chunk) =
                find_free_chunk_run(storage_page.allocated_chunks, chunk_count)
            {
                let mask = chunk_mask(first_chunk, chunk_count);
                storage_page.allocated_chunks |= mask;
                write_stored_payload(frame, first_chunk, payload, hhdm_offset);
                return Ok(StoredPayload {
                    storage_page_index: page_index as u32,
                    first_chunk: first_chunk as u8,
                    chunk_count: chunk_count as u8,
                    compressed_len: payload.len() as u16,
                });
            }
        }

        let new_physical_bytes = self
            .stats
            .allocator_consumed_bytes
            .checked_add(PAGE_SIZE as u64)
            .ok_or(ZramError::PhysicalBudgetExceeded)?;
        if new_physical_bytes > self.config.physical_budget_bytes {
            self.stats.full_events += 1;
            self.stats.budget_full_events += 1;
            return Err(ZramError::PhysicalBudgetExceeded);
        }

        let reusable_index = self
            .storage_pages
            .iter()
            .position(|storage_page| storage_page.frame.is_none());
        if reusable_index.is_none() {
            self.storage_pages
                .try_reserve(1)
                .map_err(|_| ZramError::AllocationFailure)?;
        }
        let frame = pmm.alloc_frame().ok_or(ZramError::AllocationFailure)?;
        let page_index = reusable_index.unwrap_or(self.storage_pages.len());
        let mask = chunk_mask(0, chunk_count);
        if page_index == self.storage_pages.len() {
            self.storage_pages.push(StoragePage {
                frame: Some(frame),
                allocated_chunks: mask,
            });
        } else {
            self.storage_pages[page_index] = StoragePage {
                frame: Some(frame),
                allocated_chunks: mask,
            };
        }
        self.stats.allocator_consumed_bytes = new_physical_bytes;
        write_stored_payload(frame, 0, payload, hhdm_offset);
        Ok(StoredPayload {
            storage_page_index: page_index as u32,
            first_chunk: 0,
            chunk_count: chunk_count as u8,
            compressed_len: payload.len() as u16,
        })
    }

    fn release_payload(
        &mut self,
        payload: StoredPayload,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<(), ZramError> {
        let storage_page = self
            .storage_pages
            .get_mut(payload.storage_page_index as usize)
            .ok_or(ZramError::InvalidBlock)?;
        let frame = storage_page.frame.ok_or(ZramError::NoData)?;
        let mask = chunk_mask(payload.first_chunk as usize, payload.chunk_count as usize);
        if storage_page.allocated_chunks & mask != mask {
            return Err(ZramError::NoData);
        }
        let start = payload.first_chunk as usize * STORAGE_CHUNK_BYTES;
        let length = payload.chunk_count as usize * STORAGE_CHUNK_BYTES;
        unsafe {
            core::ptr::write_bytes(
                (hhdm_offset + frame.as_u64()).as_mut_ptr::<u8>().add(start),
                0,
                length,
            );
        }
        storage_page.allocated_chunks &= !mask;
        if storage_page.allocated_chunks == 0 {
            storage_page.frame = None;
            pmm.free_frame(frame);
            self.stats.allocator_consumed_bytes = self
                .stats
                .allocator_consumed_bytes
                .checked_sub(PAGE_SIZE as u64)
                .expect("live ZRAM storage frame missing physical accounting");
        }
        Ok(())
    }

    fn store(
        &mut self,
        payload: Vec<u8>,
        checksum: u32,
        pte_flags: u64,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<SlotId, (ZramError, Vec<u8>)> {
        self.stats.swap_out_attempts += 1;
        if self.stats.used_logical_pages >= self.config.logical_capacity_pages {
            self.stats.full_events += 1;
            self.stats.swap_out_failures += 1;
            return Err((ZramError::OutOfSpace, payload));
        }
        let (index, generation) = match self.allocate_index() {
            Ok(value) => value,
            Err(error) => {
                self.stats.full_events += 1;
                self.stats.swap_out_failures += 1;
                return Err((error, payload));
            }
        };
        let id = match SlotId::new(self.config.identity as usize, index, generation) {
            Some(id) => id,
            None => return Err((ZramError::InvalidBlock, payload)),
        };
        let stored_payload = match self.allocate_payload(&payload, pmm, hhdm_offset) {
            Ok(stored_payload) => stored_payload,
            Err(error) => {
                if generation < MAX_SLOT_GENERATION {
                    self.free.push(index as u32);
                }
                self.stats.swap_out_failures += 1;
                return Err((error, payload));
            }
        };
        let slot = &mut self.slots[index];
        debug_assert!(slot.payload.is_none());
        slot.checksum = checksum;
        slot.pte_flags = pte_flags;
        slot.payload = Some(stored_payload);
        self.stats.used_logical_pages += 1;
        self.stats.used_compressed_bytes += payload.len() as u64;
        self.stats.compression_successes += 1;
        self.stats.swap_out_successes += 1;
        Ok(id)
    }

    fn read(
        &mut self,
        id: SlotId,
        output: &mut [u8; PAGE_SIZE],
        hhdm_offset: VirtAddr,
    ) -> Result<u64, ZramError> {
        self.stats.swap_in_attempts += 1;
        let Some(slot) = self.slots.get(id.index()) else {
            self.stats.swap_in_failures += 1;
            return Err(ZramError::InvalidBlock);
        };
        if slot.generation != id.generation() {
            self.stats.swap_in_failures += 1;
            return Err(ZramError::StaleSlot);
        }
        let Some(payload) = slot.payload else {
            self.stats.swap_in_failures += 1;
            return Err(ZramError::NoData);
        };
        let Some(storage_page) = self.storage_pages.get(payload.storage_page_index as usize) else {
            self.stats.swap_in_failures += 1;
            return Err(ZramError::InvalidBlock);
        };
        let Some(frame) = storage_page.frame else {
            self.stats.swap_in_failures += 1;
            return Err(ZramError::NoData);
        };
        let mask = chunk_mask(payload.first_chunk as usize, payload.chunk_count as usize);
        if storage_page.allocated_chunks & mask != mask {
            self.stats.swap_in_failures += 1;
            return Err(ZramError::NoData);
        }
        let start = payload.first_chunk as usize * STORAGE_CHUNK_BYTES;
        let compressed = unsafe {
            core::slice::from_raw_parts(
                (hhdm_offset + frame.as_u64()).as_ptr::<u8>().add(start),
                payload.compressed_len as usize,
            )
        };
        let result = zram_codec::decompress_page(compressed, slot.checksum, output);
        match result {
            Ok(()) => {
                self.stats.swap_in_successes += 1;
                Ok(slot.pte_flags)
            }
            Err(CodecError::ChecksumMismatch) => {
                self.stats.checksum_failures += 1;
                self.stats.swap_in_failures += 1;
                Err(ZramError::ChecksumMismatch)
            }
            Err(_) => {
                self.stats.decompression_failures += 1;
                self.stats.swap_in_failures += 1;
                Err(ZramError::InvalidData)
            }
        }
    }

    fn discard(
        &mut self,
        id: SlotId,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<(), ZramError> {
        let Some(slot) = self.slots.get(id.index()) else {
            return Err(ZramError::InvalidBlock);
        };
        if slot.generation != id.generation() {
            return Err(ZramError::StaleSlot);
        }
        let payload = slot.payload.ok_or(ZramError::NoData)?;
        let compressed_len = payload.compressed_len as u64;
        self.release_payload(payload, pmm, hhdm_offset)?;
        let slot = self
            .slots
            .get_mut(id.index())
            .expect("validated ZRAM slot disappeared under pool lock");
        slot.payload = None;
        self.stats.used_logical_pages = self
            .stats
            .used_logical_pages
            .checked_sub(1)
            .expect("live ZRAM slot missing logical-page accounting");
        self.stats.used_compressed_bytes = self
            .stats
            .used_compressed_bytes
            .checked_sub(compressed_len)
            .expect("live ZRAM slot missing payload accounting");
        self.stats.slot_releases += 1;
        slot.checksum = 0;
        slot.pte_flags = 0;
        if slot.generation < MAX_SLOT_GENERATION {
            // Capacity was reserved when the slot table entry was created.
            self.free.push(id.index() as u32);
        }
        Ok(())
    }
}

fn chunk_mask(first_chunk: usize, chunk_count: usize) -> u64 {
    if chunk_count == STORAGE_CHUNKS_PER_PAGE {
        u64::MAX
    } else {
        ((1u64 << chunk_count) - 1) << first_chunk
    }
}

fn find_free_chunk_run(allocated_chunks: u64, chunk_count: usize) -> Option<usize> {
    (0..=STORAGE_CHUNKS_PER_PAGE - chunk_count)
        .find(|first_chunk| allocated_chunks & chunk_mask(*first_chunk, chunk_count) == 0)
}

fn write_stored_payload(
    frame: PhysAddr,
    first_chunk: usize,
    payload: &[u8],
    hhdm_offset: VirtAddr,
) {
    let start = first_chunk * STORAGE_CHUNK_BYTES;
    unsafe {
        core::ptr::copy_nonoverlapping(
            payload.as_ptr(),
            (hhdm_offset + frame.as_u64()).as_mut_ptr::<u8>().add(start),
            payload.len(),
        );
    }
}

static POOLS: [Mutex<PoolState>; MAX_POOLS] = [const { Mutex::new(PoolState::new()) }; MAX_POOLS];
static ACTIVE_POOLS: AtomicUsize = AtomicUsize::new(0);
static FALLBACKS: AtomicU64 = AtomicU64::new(0);
static GLOBAL_OUT_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static GLOBAL_OUT_FAILURES: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
struct Configuration {
    policy: Option<SwapPolicy>,
    owner_pid: usize,
    owner_generation: u64,
    owner_alive: bool,
}

impl Configuration {
    const fn new() -> Self {
        Self {
            policy: None,
            owner_pid: 0,
            owner_generation: 0,
            owner_alive: false,
        }
    }
}

static CONFIGURATION: Mutex<Configuration> = Mutex::new(Configuration::new());

pub fn init() {}

pub fn configure(
    policy: SwapPolicy,
    owner_pid: usize,
    owner_generation: u64,
) -> Result<(), ZramError> {
    if policy.version != POLICY_VERSION
        || policy.pool_count == 0
        || policy.pool_count > MAX_POOLS
        || policy.total_logical_pages == 0
        || policy.total_physical_budget_bytes == 0
    {
        return Err(ZramError::InvalidPolicy);
    }
    let logical_bytes = policy
        .total_logical_pages
        .checked_mul(PAGE_SIZE as u64)
        .ok_or(ZramError::InvalidPolicy)?;
    if logical_bytes != policy.total_logical_bytes {
        return Err(ZramError::InvalidPolicy);
    }
    let mut config = CONFIGURATION.lock();
    if config.policy.is_some() {
        return Err(ZramError::AlreadyConfigured);
    }
    let logical_sum = policy.pools[..policy.pool_count]
        .iter()
        .try_fold(0u64, |sum, pool| {
            sum.checked_add(pool.logical_capacity_pages)
        })
        .ok_or(ZramError::InvalidPolicy)?;
    let physical_sum = policy.pools[..policy.pool_count]
        .iter()
        .try_fold(0u64, |sum, pool| {
            sum.checked_add(pool.physical_budget_bytes)
        })
        .ok_or(ZramError::InvalidPolicy)?;
    if logical_sum != policy.total_logical_pages
        || physical_sum != policy.total_physical_budget_bytes
        || policy.pools[..policy.pool_count]
            .iter()
            .any(|pool| pool.logical_capacity_pages == 0 || pool.physical_budget_bytes == 0)
    {
        return Err(ZramError::InvalidPolicy);
    }

    let slot_base = MAX_GLOBAL_SLOTS / policy.pool_count;
    let slot_remainder = MAX_GLOBAL_SLOTS % policy.pool_count;
    for (index, pool_policy) in policy.pools[..policy.pool_count].iter().enumerate() {
        let logical_capacity_pages = usize::try_from(pool_policy.logical_capacity_pages)
            .map_err(|_| ZramError::InvalidPolicy)?;
        let slot_limit =
            (slot_base + usize::from(index < slot_remainder)).min(logical_capacity_pages);
        POOLS[index].lock().configure(PoolConfig {
            identity: index as u8,
            logical_capacity_pages: pool_policy.logical_capacity_pages,
            physical_budget_bytes: pool_policy.physical_budget_bytes,
            slot_limit,
        });
    }
    config.policy = Some(policy);
    config.owner_pid = owner_pid;
    config.owner_generation = owner_generation;
    config.owner_alive = true;
    ACTIVE_POOLS.store(policy.pool_count, Ordering::Release);
    Ok(())
}

pub fn revoke_admin(owner_pid: usize, owner_generation: u64) {
    let mut config = CONFIGURATION.lock();
    if config.owner_pid == owner_pid && config.owner_generation == owner_generation {
        config.owner_alive = false;
    }
}

pub fn policy() -> Option<SwapPolicy> {
    CONFIGURATION.lock().policy
}

pub fn write_page(
    data: &[u8; PAGE_SIZE],
    pte_flags: u64,
    selection_key: u64,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Result<SlotId, ZramError> {
    let pool_count = ACTIVE_POOLS.load(Ordering::Acquire);
    if pool_count == 0 {
        return Err(ZramError::NotConfigured);
    }
    GLOBAL_OUT_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    let mut scratch = [0u8; MAX_COMPRESSED_SIZE];
    let (compressed_len, checksum) = match zram_codec::compress_page(data, &mut scratch) {
        Ok(value) => value,
        Err(CodecError::Incompressible) => {
            let preferred = selection_key as usize % pool_count;
            let mut pool = POOLS[preferred].lock();
            pool.stats.incompressible_rejected += 1;
            pool.stats.compression_failures += 1;
            pool.stats.swap_out_attempts += 1;
            pool.stats.swap_out_failures += 1;
            GLOBAL_OUT_FAILURES.fetch_add(1, Ordering::Relaxed);
            return Err(ZramError::Incompressible);
        }
        Err(_) => {
            GLOBAL_OUT_FAILURES.fetch_add(1, Ordering::Relaxed);
            return Err(ZramError::InvalidData);
        }
    };
    let mut payload = Vec::new();
    if payload.try_reserve_exact(compressed_len).is_err() {
        GLOBAL_OUT_FAILURES.fetch_add(1, Ordering::Relaxed);
        return Err(ZramError::AllocationFailure);
    }
    payload.extend_from_slice(&scratch[..compressed_len]);
    let stored_representation_bytes = zram_codec::allocator_consumed_bytes(payload.capacity())
        .ok_or(ZramError::AllocationFailure)?;
    if stored_representation_bytes >= PAGE_SIZE {
        let preferred = selection_key as usize % pool_count;
        let mut pool = POOLS[preferred].lock();
        pool.stats.incompressible_rejected += 1;
        pool.stats.compression_failures += 1;
        pool.stats.swap_out_attempts += 1;
        pool.stats.swap_out_failures += 1;
        GLOBAL_OUT_FAILURES.fetch_add(1, Ordering::Relaxed);
        return Err(ZramError::Incompressible);
    }
    scratch.fill(0);

    let start = selection_key as usize % pool_count;
    let mut last_error = ZramError::OutOfSpace;
    for offset in 0..pool_count {
        if offset != 0 {
            FALLBACKS.fetch_add(1, Ordering::Relaxed);
        }
        let pool_index = (start + offset) % pool_count;
        match POOLS[pool_index]
            .lock()
            .store(payload, checksum, pte_flags, pmm, hhdm_offset)
        {
            Ok(id) => return Ok(id),
            Err((error, returned_payload)) => {
                last_error = error;
                payload = returned_payload;
            }
        }
    }
    GLOBAL_OUT_FAILURES.fetch_add(1, Ordering::Relaxed);
    Err(last_error)
}

pub fn read_page(
    id: SlotId,
    output: &mut [u8; PAGE_SIZE],
    hhdm_offset: VirtAddr,
) -> Result<u64, ZramError> {
    let pool_count = ACTIVE_POOLS.load(Ordering::Acquire);
    if id.pool() >= pool_count {
        return Err(ZramError::InvalidBlock);
    }
    POOLS[id.pool()].lock().read(id, output, hhdm_offset)
}

pub fn discard(
    id: SlotId,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Result<(), ZramError> {
    let pool_count = ACTIVE_POOLS.load(Ordering::Acquire);
    if id.pool() >= pool_count {
        return Err(ZramError::InvalidBlock);
    }
    POOLS[id.pool()].lock().discard(id, pmm, hhdm_offset)
}

pub fn discard_block(
    raw: BlockId,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Result<(), ZramError> {
    discard(
        SlotId::from_raw(raw).ok_or(ZramError::InvalidBlock)?,
        pmm,
        hhdm_offset,
    )
}

pub fn block_exists(raw: BlockId) -> bool {
    let Some(id) = SlotId::from_raw(raw) else {
        return false;
    };
    let pool_count = ACTIVE_POOLS.load(Ordering::Acquire);
    if id.pool() >= pool_count {
        return false;
    }
    let pool = POOLS[id.pool()].lock();
    pool.slots
        .get(id.index())
        .is_some_and(|slot| slot.generation == id.generation() && slot.payload.is_some())
}

pub fn pool_stats(index: usize) -> Option<PoolStats> {
    (index < ACTIVE_POOLS.load(Ordering::Acquire)).then(|| POOLS[index].lock().stats)
}

pub fn aggregate_stats() -> AggregateStats {
    let pool_count = ACTIVE_POOLS.load(Ordering::Acquire);
    let config = CONFIGURATION.lock();
    let mut result = AggregateStats {
        active_pool_count: pool_count,
        fallback_to_next_pool: FALLBACKS.load(Ordering::Relaxed),
        service_configured: config.policy.is_some(),
        admin_owner_alive: config.owner_alive,
        ..AggregateStats::default()
    };
    drop(config);
    for pool in POOLS.iter().take(pool_count) {
        let stats = pool.lock().stats;
        result.configured_logical_pages += stats.logical_capacity_pages;
        result.configured_physical_budget_bytes += stats.physical_budget_bytes;
        result.stored_pages += stats.used_logical_pages;
        result.compressed_bytes += stats.used_compressed_bytes;
        result.allocator_consumed_bytes += stats.allocator_consumed_bytes;
        result.pages_stored_raw += stats.raw_pages;
        result.incompressible_rejected += stats.incompressible_rejected;
        result.swap_out_successes += stats.swap_out_successes;
        result.swap_in_attempts += stats.swap_in_attempts;
        result.swap_in_successes += stats.swap_in_successes;
        result.swap_in_failures += stats.swap_in_failures;
        result.checksum_failures += stats.checksum_failures;
        result.decompression_failures += stats.decompression_failures;
        result.full_pool_events += stats.full_events;
        result.budget_full_events += stats.budget_full_events;
    }
    result.swap_out_attempts = GLOBAL_OUT_ATTEMPTS.load(Ordering::Relaxed);
    result.swap_out_failures = GLOBAL_OUT_FAILURES.load(Ordering::Relaxed);
    result
}

/// Compatibility telemetry tuple: logical pages, stored pages, compressed bytes.
pub fn stats() -> (usize, usize, usize) {
    let stats = aggregate_stats();
    (
        stats.configured_logical_pages as usize,
        stats.stored_pages as usize,
        stats.compressed_bytes as usize,
    )
}

#[cfg(feature = "swap1_test")]
pub fn corrupt_payload_for_test(id: SlotId, hhdm_offset: VirtAddr) -> Result<(), ZramError> {
    let pool_count = ACTIVE_POOLS.load(Ordering::Acquire);
    if id.pool() >= pool_count {
        return Err(ZramError::InvalidBlock);
    }
    let mut pool = POOLS[id.pool()].lock();
    let slot = pool
        .slots
        .get_mut(id.index())
        .ok_or(ZramError::InvalidBlock)?;
    if slot.generation != id.generation() {
        return Err(ZramError::StaleSlot);
    }
    let payload = slot.payload.ok_or(ZramError::NoData)?;
    let storage_page = pool
        .storage_pages
        .get(payload.storage_page_index as usize)
        .ok_or(ZramError::InvalidBlock)?;
    let frame = storage_page.frame.ok_or(ZramError::NoData)?;
    let start = payload.first_chunk as usize * STORAGE_CHUNK_BYTES;
    unsafe {
        let byte = (hhdm_offset + frame.as_u64()).as_mut_ptr::<u8>().add(start);
        *byte ^= 0x5a;
    }
    Ok(())
}
