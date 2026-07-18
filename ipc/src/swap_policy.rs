//! Fixed SWAP-1 boot policy shared by the privileged policy service and kernel.

pub const POLICY_VERSION: u32 = 1;
pub const PAGE_SIZE: u64 = 4096;
pub const GIB: u64 = 1024 * 1024 * 1024;
pub const MIN_POOL_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_POOLS: usize = 32;

/// The kernel heap is currently a fixed 8 MiB arena. SWAP-1 deliberately gives
/// compressed payloads at most one quarter of it, leaving bounded headroom for
/// page tables, process metadata, IPC, and slot metadata.
pub const MAX_PHYSICAL_BUDGET_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PoolPolicy {
    pub logical_pages: u64,
    pub physical_budget_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapPolicy {
    pub version: u32,
    pub detected_ram_bytes: u64,
    pub detected_online_cpus: u32,
    pub total_logical_bytes: u64,
    pub total_logical_pages: u64,
    pub total_physical_budget_bytes: u64,
    pub pool_count: usize,
    pub pools: [PoolPolicy; MAX_POOLS],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AggregateDiagnostics {
    pub active_pool_count: u64,
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
    pub candidate_scans: u64,
    pub pages_reclaimed: u64,
    pub watermark_activations: u64,
    pub service_configured: u64,
    pub admin_owner_alive: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PoolDiagnostics {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyError {
    NoUsableMemory,
    ArithmeticOverflow,
    CapacityExceeded,
}

/// Calculate the immutable SWAP-1 policy from PMM-tracked usable RAM and the
/// scheduler's online CPU count. RAM is rounded down to whole 4 KiB pages.
pub fn calculate(usable_ram_bytes: u64, online_cpus: u32) -> Result<SwapPolicy, PolicyError> {
    let ram_pages = usable_ram_bytes / PAGE_SIZE;
    if ram_pages == 0 {
        return Err(PolicyError::NoUsableMemory);
    }
    let ram_bytes = ram_pages
        .checked_mul(PAGE_SIZE)
        .ok_or(PolicyError::ArithmeticOverflow)?;
    let target_unaligned = if ram_bytes <= 2 * GIB {
        ram_bytes
    } else if ram_bytes <= 8 * GIB {
        ram_bytes / 2
    } else {
        ram_bytes / 3
    };
    let target_pages = target_unaligned / PAGE_SIZE;
    if target_pages == 0 {
        return Err(PolicyError::NoUsableMemory);
    }
    let target_bytes = target_pages
        .checked_mul(PAGE_SIZE)
        .ok_or(PolicyError::ArithmeticOverflow)?;

    // Slot ids reserve 22 bits for a pool-local index. No configured pool may
    // exceed what can be represented in a non-present x86_64 leaf PTE.
    const MAX_POOL_PAGES: u64 = 1 << 22;
    let cpus = online_cpus.max(1);
    let cpu_pool_limit = usize::try_from(cpus / 2).unwrap_or(usize::MAX).max(1);
    let size_pool_limit = usize::try_from(target_bytes / MIN_POOL_BYTES)
        .unwrap_or(usize::MAX)
        .max(1);
    let pool_count = cpu_pool_limit.min(size_pool_limit).min(MAX_POOLS);
    if target_pages.div_ceil(pool_count as u64) > MAX_POOL_PAGES {
        return Err(PolicyError::CapacityExceeded);
    }

    // Never promise a compression ratio. This is merely a hard allocation
    // ceiling: at most RAM/4, at most half the logical target, and at most the
    // fixed-heap safety cap above.
    let total_physical_budget_bytes = (ram_bytes / 4)
        .min(target_bytes / 2)
        .min(MAX_PHYSICAL_BUDGET_BYTES)
        / PAGE_SIZE
        * PAGE_SIZE;
    if total_physical_budget_bytes == 0 {
        return Err(PolicyError::NoUsableMemory);
    }

    let mut pools = [PoolPolicy::default(); MAX_POOLS];
    let logical_base = target_pages / pool_count as u64;
    let logical_remainder = target_pages % pool_count as u64;
    let physical_base = total_physical_budget_bytes / pool_count as u64;
    let physical_remainder = total_physical_budget_bytes % pool_count as u64;
    for (index, pool) in pools.iter_mut().take(pool_count).enumerate() {
        pool.logical_pages = logical_base + u64::from((index as u64) < logical_remainder);
        pool.physical_budget_bytes = physical_base + u64::from((index as u64) < physical_remainder);
        if pool.logical_pages == 0 {
            return Err(PolicyError::NoUsableMemory);
        }
    }

    Ok(SwapPolicy {
        version: POLICY_VERSION,
        detected_ram_bytes: ram_bytes,
        detected_online_cpus: cpus,
        total_logical_bytes: target_bytes,
        total_logical_pages: target_pages,
        total_physical_budget_bytes,
        pool_count,
        pools,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gib(value: u64) -> u64 {
        value * GIB
    }

    #[test]
    fn ram_boundaries_follow_fixed_policy() {
        let cases = [
            (gib(1), gib(1)),
            (gib(2), gib(2)),
            (
                gib(2) + PAGE_SIZE,
                (gib(2) + PAGE_SIZE) / 2 / PAGE_SIZE * PAGE_SIZE,
            ),
            (gib(4), gib(2)),
            (gib(8), gib(4)),
            (
                gib(8) + PAGE_SIZE,
                (gib(8) + PAGE_SIZE) / 3 / PAGE_SIZE * PAGE_SIZE,
            ),
            (gib(12), gib(4)),
        ];
        for (ram, expected) in cases {
            assert_eq!(calculate(ram, 4).unwrap().total_logical_bytes, expected);
        }
    }

    #[test]
    fn cpu_counts_and_minimum_pool_size_are_bounded() {
        for (cpus, expected) in [(1, 1), (2, 1), (3, 1), (4, 2), (12, 6)] {
            assert_eq!(calculate(gib(8), cpus).unwrap().pool_count, expected);
        }
        assert_eq!(calculate(128 * 1024 * 1024, 64).unwrap().pool_count, 1);
    }

    #[test]
    fn pool_sums_are_exact_nonzero_and_remainder_is_first() {
        let policy = calculate(gib(8) + 7 * PAGE_SIZE, 12).unwrap();
        let active = &policy.pools[..policy.pool_count];
        assert!(active.iter().all(|pool| pool.logical_pages != 0));
        assert_eq!(
            active.iter().map(|pool| pool.logical_pages).sum::<u64>(),
            policy.total_logical_pages
        );
        assert_eq!(
            active
                .iter()
                .map(|pool| pool.physical_budget_bytes)
                .sum::<u64>(),
            policy.total_physical_budget_bytes
        );
        for pair in active.windows(2) {
            assert!(pair[0].logical_pages >= pair[1].logical_pages);
            assert!(pair[0].logical_pages - pair[1].logical_pages <= 1);
        }
    }

    #[test]
    fn checked_error_paths_are_deterministic() {
        assert_eq!(calculate(0, 0), Err(PolicyError::NoUsableMemory));
        assert_eq!(calculate(u64::MAX, 1), Err(PolicyError::CapacityExceeded));
        assert_eq!(calculate(gib(4), 0).unwrap().detected_online_cpus, 1);
    }

    #[test]
    fn examples_have_two_equal_pools_on_four_cores() {
        for (ram, total) in [
            (gib(1), gib(1)),
            (gib(2), gib(2)),
            (gib(4), gib(2)),
            (gib(8), gib(4)),
            (gib(12), gib(4)),
        ] {
            let policy = calculate(ram, 4).unwrap();
            assert_eq!(policy.total_logical_bytes, total);
            assert_eq!(policy.pool_count, 2);
            assert_eq!(policy.pools[0].logical_pages, policy.pools[1].logical_pages);
        }
    }
}
