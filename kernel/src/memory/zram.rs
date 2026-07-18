//! Bounded, immutable-after-activation, multi-pool in-memory swap storage.

pub use super::swap_slot::SlotId;
use super::swap_slot::{MAX_SLOT_GENERATION, SLOT_INDEX_MASK};
use super::zram_codec::{self, CodecError, MAX_COMPRESSED_SIZE, PAGE_SIZE};
use ::sunlight_ipc::swap_policy::{SwapPolicy, MAX_POOLS, POLICY_VERSION};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;

pub const ZRAM_BLOCK_SIZE: usize = PAGE_SIZE;
pub const MAX_GLOBAL_SLOTS: usize = 16 * 1024;

pub type BlockId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZramError {
    NotConfigured,
    AlreadyConfigured,
    InvalidPolicy,
    OutOfSpace,
    PhysicalBudgetExceeded,
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
    pub slot_releases: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AggregateStats {
    pub active_pool_count: usize,
    pub configured_logical_pages: u64,
    pub configured_physical_budget_bytes: u64,
    pub stored_pages: u64,
    pub compressed_bytes: u64,
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
    pub fallback_to_next_pool: u64,
    pub service_configured: bool,
    pub admin_owner_alive: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct PoolConfig {
    identity: u8,
    logical_pages: u64,
    physical_budget_bytes: u64,
    slot_limit: usize,
}

struct Slot {
    generation: u16,
    checksum: u32,
    pte_flags: u64,
    payload: Option<Vec<u8>>,
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

struct PoolState {
    config: PoolConfig,
    slots: Vec<Slot>,
    free: Vec<u32>,
    stats: PoolStats,
}

impl PoolState {
    const fn new() -> Self {
        Self {
            config: PoolConfig {
                identity: 0,
                logical_pages: 0,
                physical_budget_bytes: 0,
                slot_limit: 0,
            },
            slots: Vec::new(),
            free: Vec::new(),
            stats: PoolStats {
                logical_capacity_pages: 0,
                physical_budget_bytes: 0,
                used_logical_pages: 0,
                used_compressed_bytes: 0,
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
                slot_releases: 0,
            },
        }
    }

    fn configure(&mut self, config: PoolConfig) {
        debug_assert!(self.slots.is_empty());
        self.config = config;
        self.stats.logical_capacity_pages = config.logical_pages;
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
            || self.slots.len() as u64 >= self.config.logical_pages
            || self.slots.len() as u64 > SLOT_INDEX_MASK
        {
            return Err(ZramError::MetadataExhausted);
        }
        self.slots
            .try_reserve(1)
            .map_err(|_| ZramError::MetadataExhausted)?;
        // Reserve the future discard push before publishing this index. Once a
        // slot exists, release can therefore never require a heap allocation.
        self.free
            .try_reserve(1)
            .map_err(|_| ZramError::MetadataExhausted)?;
        let index = self.slots.len();
        self.slots.push(Slot::vacant(1));
        Ok((index, 1))
    }

    fn store(
        &mut self,
        payload: Vec<u8>,
        checksum: u32,
        pte_flags: u64,
    ) -> Result<SlotId, (ZramError, Vec<u8>)> {
        self.stats.swap_out_attempts += 1;
        if self.stats.used_logical_pages >= self.config.logical_pages {
            self.stats.full_events += 1;
            self.stats.swap_out_failures += 1;
            return Err((ZramError::OutOfSpace, payload));
        }
        let new_bytes = match self
            .stats
            .used_compressed_bytes
            .checked_add(payload.len() as u64)
        {
            Some(value) if value <= self.config.physical_budget_bytes => value,
            _ => {
                self.stats.full_events += 1;
                self.stats.swap_out_failures += 1;
                return Err((ZramError::PhysicalBudgetExceeded, payload));
            }
        };
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
        let slot = &mut self.slots[index];
        debug_assert!(slot.payload.is_none());
        slot.checksum = checksum;
        slot.pte_flags = pte_flags;
        slot.payload = Some(payload);
        self.stats.used_logical_pages += 1;
        self.stats.used_compressed_bytes = new_bytes;
        self.stats.compression_successes += 1;
        self.stats.swap_out_successes += 1;
        Ok(id)
    }

    fn read(&mut self, id: SlotId, output: &mut [u8; PAGE_SIZE]) -> Result<u64, ZramError> {
        self.stats.swap_in_attempts += 1;
        let Some(slot) = self.slots.get(id.index()) else {
            self.stats.swap_in_failures += 1;
            return Err(ZramError::InvalidBlock);
        };
        if slot.generation != id.generation() {
            self.stats.swap_in_failures += 1;
            return Err(ZramError::StaleSlot);
        }
        let Some(payload) = slot.payload.as_ref() else {
            self.stats.swap_in_failures += 1;
            return Err(ZramError::NoData);
        };
        let result = zram_codec::decompress_page(payload, slot.checksum, output);
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

    fn discard(&mut self, id: SlotId) -> Result<(), ZramError> {
        let Some(slot) = self.slots.get_mut(id.index()) else {
            return Err(ZramError::InvalidBlock);
        };
        if slot.generation != id.generation() {
            return Err(ZramError::StaleSlot);
        }
        let payload = slot.payload.take().ok_or(ZramError::NoData)?;
        self.stats.used_logical_pages -= 1;
        self.stats.used_compressed_bytes -= payload.len() as u64;
        self.stats.slot_releases += 1;
        slot.checksum = 0;
        slot.pte_flags = 0;
        drop(payload);
        if slot.generation < MAX_SLOT_GENERATION {
            // Capacity was reserved when the slot table entry was created.
            self.free.push(id.index() as u32);
        }
        Ok(())
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
    let mut config = CONFIGURATION.lock();
    if config.policy.is_some() {
        return Err(ZramError::AlreadyConfigured);
    }
    let logical_sum = policy.pools[..policy.pool_count]
        .iter()
        .try_fold(0u64, |sum, pool| sum.checked_add(pool.logical_pages))
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
            .any(|pool| pool.logical_pages == 0 || pool.physical_budget_bytes == 0)
    {
        return Err(ZramError::InvalidPolicy);
    }

    let slot_base = MAX_GLOBAL_SLOTS / policy.pool_count;
    let slot_remainder = MAX_GLOBAL_SLOTS % policy.pool_count;
    for (index, pool_policy) in policy.pools[..policy.pool_count].iter().enumerate() {
        let slot_limit = (slot_base + usize::from(index < slot_remainder))
            .min(pool_policy.logical_pages as usize);
        POOLS[index].lock().configure(PoolConfig {
            identity: index as u8,
            logical_pages: pool_policy.logical_pages,
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
    payload
        .try_reserve_exact(compressed_len)
        .map_err(|_| ZramError::MetadataExhausted)?;
    payload.extend_from_slice(&scratch[..compressed_len]);
    scratch.fill(0);

    let start = selection_key as usize % pool_count;
    let mut last_error = ZramError::OutOfSpace;
    for offset in 0..pool_count {
        if offset != 0 {
            FALLBACKS.fetch_add(1, Ordering::Relaxed);
        }
        let pool_index = (start + offset) % pool_count;
        match POOLS[pool_index].lock().store(payload, checksum, pte_flags) {
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

pub fn read_page(id: SlotId, output: &mut [u8; PAGE_SIZE]) -> Result<u64, ZramError> {
    let pool_count = ACTIVE_POOLS.load(Ordering::Acquire);
    if id.pool() >= pool_count {
        return Err(ZramError::InvalidBlock);
    }
    POOLS[id.pool()].lock().read(id, output)
}

pub fn discard(id: SlotId) -> Result<(), ZramError> {
    let pool_count = ACTIVE_POOLS.load(Ordering::Acquire);
    if id.pool() >= pool_count {
        return Err(ZramError::InvalidBlock);
    }
    POOLS[id.pool()].lock().discard(id)
}

pub fn discard_block(raw: BlockId) -> Result<(), ZramError> {
    discard(SlotId::from_raw(raw).ok_or(ZramError::InvalidBlock)?)
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
        result.pages_stored_raw += stats.raw_pages;
        result.incompressible_rejected += stats.incompressible_rejected;
        result.swap_out_successes += stats.swap_out_successes;
        result.swap_in_attempts += stats.swap_in_attempts;
        result.swap_in_successes += stats.swap_in_successes;
        result.swap_in_failures += stats.swap_in_failures;
        result.checksum_failures += stats.checksum_failures;
        result.decompression_failures += stats.decompression_failures;
        result.full_pool_events += stats.full_events;
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

/// Block ids written by the most recent `freezram_fill`, pending verification.
static FREEZRAM_BLOCKS: Mutex<Vec<BlockId>> = Mutex::new(Vec::new());

pub fn freezram_fill(n: usize) -> usize {
    let mut blocks = FREEZRAM_BLOCKS.lock();
    for id in blocks.drain(..) {
        let _ = discard_block(id);
    }
    if blocks.try_reserve(n.min(MAX_GLOBAL_SLOTS)).is_err() {
        return 0;
    }
    let mut written = 0;
    for index in 0..n.min(MAX_GLOBAL_SLOTS) {
        let mut page = [0u8; PAGE_SIZE];
        page.fill((index & 0xff) as u8);
        match write_page(&page, 0, index as u64) {
            Ok(id) => {
                blocks.push(id.raw());
                written += 1;
            }
            Err(_) => break,
        }
    }
    written
}

pub fn freezram_verify() -> Option<usize> {
    let mut blocks = FREEZRAM_BLOCKS.lock();
    let mut verified = 0;
    for (index, raw) in blocks.drain(..).enumerate() {
        let id = SlotId::from_raw(raw)?;
        let mut output = [0u8; PAGE_SIZE];
        read_page(id, &mut output).ok()?;
        if output.iter().all(|byte| *byte == (index & 0xff) as u8) {
            verified += 1;
        }
        discard(id).ok()?;
    }
    Some(verified)
}
