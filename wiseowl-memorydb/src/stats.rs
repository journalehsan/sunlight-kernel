//! Observability counters (saturating — never wrap silently to zero).

/// Service statistics snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct DbStats {
    pub database_generation: u64,
    pub index_generation: u64,
    pub last_committed_sequence: u64,
    pub wal_bytes: u64,
    pub wal_records: u64,
    pub active_transactions: u32,
    pub record_count_active: u32,
    pub record_count_superseded: u32,
    pub record_count_tombstoned: u32,
    pub relationship_count: u64,
    pub segment_count: u32,
    pub segment_bytes_compressed: u64,
    pub segment_bytes_uncompressed: u64,
    pub primary_index_entries: u64,
    pub source_index_entries: u64,
    pub token_dictionary_entries: u64,
    pub token_posting_entries: u64,
    pub query_count: u64,
    pub query_failures: u64,
    pub query_budget_exhaustions: u64,
    pub transaction_commits: u64,
    pub transaction_aborts: u64,
    pub recovery_replayed_operations: u64,
    pub checkpoint_count: u64,
    pub compaction_count: u64,
    pub compaction_bytes_reclaimed: u64,
    pub checksum_failures: u64,
    pub quarantined_files: u32,
    pub shm_leased_bytes: u64,
    pub active_shm_leases: u32,
}

impl DbStats {
    pub fn sat_add(field: &mut u64, n: u64) {
        *field = field.saturating_add(n);
    }
}
