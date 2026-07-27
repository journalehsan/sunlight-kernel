//! Saturating indexer telemetry counters (never wrap; never log payloads).

/// Indexer statistics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct IndexStats {
    pub configured_roots: u64,
    pub available_roots: u64,
    pub active_scans: u64,
    pub directories_visited: u64,
    pub files_discovered: u64,
    pub files_prefilter_skipped: u64,
    pub metadata_fast_skips: u64,
    pub files_hashed: u64,
    pub hash_bytes: u64,
    pub strong_hash_files: u64,
    pub strong_hash_bytes: u64,
    pub strong_hash_unchanged: u64,
    pub files_unchanged: u64,
    pub files_new: u64,
    pub files_changed: u64,
    pub files_renamed: u64,
    pub files_missing: u64,
    pub files_failed: u64,
    pub files_excluded: u64,
    pub files_indexed: u64,
    pub files_deleted: u64,
    /// Sources that completed UTF-8/binary validation (accepted or newly rejected).
    pub files_validated: u64,
    /// Newly classified permanent rejections (first time or after digest/policy change).
    pub files_rejected_new: u64,
    /// Unchanged rejected sources reusing durable rejection identity (no parse/tokenize).
    pub files_rejected_cached: u64,
    /// Actual parser execution only (not hash / rejection-cache lookup).
    pub files_reparsed: u64,
    /// Actual tokenizer execution only.
    pub files_retokenized: u64,
    pub database_generations_created: u64,
    /// Previous active document generations superseded by a newer import.
    pub database_generations_superseded: u64,
    /// Source delete requests issued (not merely missing confirmation).
    pub source_delete_requests: u64,
    /// Source delete commits completed.
    pub source_delete_commits: u64,
    /// Sources that reached the missing-confirmation threshold (root available path).
    pub files_missing_confirmed: u64,
    pub bytes_read: u64,
    pub bytes_parsed: u64,
    pub blocks_parsed: u64,
    pub chunks_created: u64,
    pub chunks_reused: u64,
    pub tokens_emitted: u64,
    pub unique_tokens: u64,
    pub token_positions_stored: u64,
    pub token_positions_truncated: u64,
    pub token_collisions_detected: u64,
    pub database_transactions_started: u64,
    pub database_transactions_committed: u64,
    pub database_transactions_aborted: u64,
    pub database_retries: u64,
    pub source_delete_batches: u64,
    pub scan_budget_exhaustions: u64,
    pub retry_queue_length: u64,
    pub shm_leased_bytes: u64,
    pub active_shm_leases: u64,
    pub sources_tracked: u64,
    // Phase 3.5 transport / reconciliation
    pub memorydb_connection_attempts: u64,
    pub memorydb_connection_successes: u64,
    pub memorydb_disconnects: u64,
    pub memorydb_reconnects: u64,
    pub memorydb_protocol_failures: u64,
    pub memorydb_unavailable_operations: u64,
    pub pending_imports: u64,
    pub uncertain_imports: u64,
    pub imports_reconciled_committed: u64,
    pub imports_reconciled_retried: u64,
    pub imports_reconciled_conflict: u64,
    pub native_transactions_started: u64,
    pub native_transactions_committed: u64,
    pub native_transactions_aborted: u64,
    pub native_transaction_timeouts: u64,
    pub native_shm_bytes_sent: u64,
    pub native_shm_bytes_received: u64,
    pub native_shm_leases: u64,
    pub native_shm_lease_failures: u64,
    pub manifest_migrations_started: u64,
    pub manifest_migrations_completed: u64,
    pub manifest_migrations_failed: u64,
    pub native_lexical_queries: u64,
    pub native_lexical_hits: u64,
}

impl IndexStats {
    pub fn sat_add(field: &mut u64, n: u64) {
        *field = field.saturating_add(n);
    }
}
