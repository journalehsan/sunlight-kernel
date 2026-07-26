//! Hard resource limits for wiseowl-memorydb.
//!
//! All client-controlled sizes are checked before allocation. Defaults target
//! low-memory hardware while remaining useful for host integration tests.

/// Configurable hard limits.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct DbQuotaConfig {
    /// Maximum total database size on disk (logical compressed segments + WAL + indexes).
    pub max_database_bytes: u64,
    /// Maximum WAL size before checkpoint pressure is applied.
    pub max_wal_bytes: u64,
    /// Maximum single sealed segment size (compressed).
    pub max_segment_bytes: u32,
    /// Maximum uncompressed segment size after decompression.
    pub max_segment_uncompressed: u32,
    /// Maximum single record payload.
    pub max_payload_bytes: u32,
    /// Maximum live + historical records tracked in indexes.
    pub max_records: u32,
    /// Maximum concurrent open transactions.
    pub max_active_transactions: u32,
    /// Maximum operations in one transaction.
    pub max_ops_per_transaction: u32,
    /// Maximum payload bytes staged in one transaction.
    pub max_bytes_per_transaction: u32,
    /// Maximum relationships in one transaction.
    pub max_relationships_per_transaction: u32,
    /// Maximum open transaction age (nanoseconds; 0 = no time limit on host tests).
    pub max_transaction_age_ns: u64,
    /// Maximum tokens per record.
    pub max_tokens_per_record: u32,
    /// Maximum positions per token.
    pub max_positions_per_token: u32,
    /// Maximum relationships per record (outgoing).
    pub max_relationships_per_record: u32,
    /// Maximum attributes per record.
    pub max_attributes_per_record: u32,
    /// Maximum attribute key length.
    pub max_attribute_key_bytes: u32,
    /// Maximum attribute text value length.
    pub max_attribute_text_bytes: u32,
    /// Maximum provenance parents.
    pub max_provenance_parents: u32,
    /// Maximum query results per page (service hard cap).
    pub max_query_results: u32,
    /// Maximum posting list page size.
    pub max_posting_page: u32,
    /// Maximum relationship graph traversal depth.
    pub max_graph_depth: u32,
    /// Maximum SHM result bytes leased at once.
    pub max_shm_result_bytes: u32,
    /// Maximum quarantine directory bytes.
    pub max_quarantine_bytes: u64,
    /// Maximum quarantine file count.
    pub max_quarantine_files: u32,
    /// Maximum recovery work (bytes inspected) per startup.
    pub max_recovery_bytes: u64,
    /// Maximum segments processed per compaction iteration.
    pub max_compaction_segments: u32,
    /// Maximum records rewritten per compaction iteration.
    pub max_compaction_records: u32,
    /// Maximum bytes read per compaction iteration.
    pub max_compaction_bytes_read: u64,
    /// Maximum bytes written per compaction iteration.
    pub max_compaction_bytes_write: u64,
    /// Maximum service RAM for indexes + staging (approximate).
    pub max_service_ram_bytes: u64,
}

impl Default for DbQuotaConfig {
    fn default() -> Self {
        Self {
            max_database_bytes: 32 * 1024 * 1024,
            max_wal_bytes: 2 * 1024 * 1024,
            max_segment_bytes: 512 * 1024,
            max_segment_uncompressed: 1024 * 1024,
            max_payload_bytes: 64 * 1024,
            max_records: 4096,
            max_active_transactions: 4,
            max_ops_per_transaction: 32,
            max_bytes_per_transaction: 256 * 1024,
            max_relationships_per_transaction: 16,
            max_transaction_age_ns: 30_000_000_000, // 30s
            max_tokens_per_record: 256,
            max_positions_per_token: 32,
            max_relationships_per_record: 32,
            max_attributes_per_record: 16,
            max_attribute_key_bytes: 32,
            max_attribute_text_bytes: 128,
            max_provenance_parents: 8,
            max_query_results: 64,
            max_posting_page: 64,
            max_graph_depth: 3,
            max_shm_result_bytes: 64 * 1024,
            max_quarantine_bytes: 1024 * 1024,
            max_quarantine_files: 16,
            max_recovery_bytes: 8 * 1024 * 1024,
            max_compaction_segments: 4,
            max_compaction_records: 256,
            max_compaction_bytes_read: 2 * 1024 * 1024,
            max_compaction_bytes_write: 2 * 1024 * 1024,
            max_service_ram_bytes: 4 * 1024 * 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_bounded() {
        let c = DbQuotaConfig::default();
        assert!(c.max_payload_bytes <= c.max_segment_uncompressed);
        assert!(c.max_query_results > 0);
        assert!(c.max_ops_per_transaction > 0);
        assert!(c.max_wal_bytes < c.max_database_bytes);
    }
}
