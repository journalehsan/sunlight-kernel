//! Observability counters for wiseowl-memoryd.
//!
//! Counters use saturating arithmetic and never wrap silently.

/// Service-wide counters and gauges.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct ServiceStats {
    // Gauges
    pub working_bytes: u64,
    pub hot_bytes: u64,
    pub cold_compressed_bytes: u64,
    pub cold_uncompressed_logical_bytes: u64,
    pub entry_count: u64,
    pub segment_count: u64,
    pub active_sessions: u64,

    // Counters (saturating)
    pub creates: u64,
    pub reads: u64,
    pub seals: u64,
    pub expirations: u64,
    pub evictions: u64,
    pub rejected_allocations: u64,
    pub compression_successes: u64,
    pub compression_failures: u64,
    pub decompression_successes: u64,
    pub decompression_failures: u64,
    pub checksum_failures: u64,
    pub kv_promotion_successes: u64,
    pub kv_promotion_failures: u64,
    pub shm_validation_failures: u64,
    pub malformed_ipc_requests: u64,
    pub maintenance_runs: u64,
    pub client_disconnects: u64,
    pub quarantined_spill_records: u64,
}

impl ServiceStats {
    pub fn inc(field: &mut u64) {
        *field = field.saturating_add(1);
    }

    pub fn add(field: &mut u64, n: u64) {
        *field = field.saturating_add(n);
    }
}
