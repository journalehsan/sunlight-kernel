//! Hard resource limits for wiseowl-index.
//!
//! All client-controlled sizes are checked before allocation. Defaults keep
//! scan and parse work bounded for low-memory hardware.

/// Configurable hard limits for the document indexer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct IndexQuotaConfig {
    pub max_roots_per_user: u16,
    pub max_path_bytes: u16,
    pub max_relative_path_bytes: u16,
    pub max_traversal_depth: u16,
    pub max_directories_per_scan: u32,
    pub max_files_inspected_per_scan: u32,
    pub max_files_hashed_per_scan: u32,
    pub max_files_parsed_per_scan: u32,
    pub max_hash_bytes_per_scan: u64,
    pub max_parse_bytes_per_scan: u64,
    pub max_file_size_bytes: u64,
    pub max_parser_nesting: u16,
    pub max_blocks_per_file: u32,
    pub max_chunks_per_file: u32,
    pub max_bytes_per_chunk: u32,
    pub max_tokens_per_chunk: u32,
    pub max_unique_tokens_per_chunk: u32,
    pub max_positions_per_token: u32,
    pub max_token_length: u16,
    pub max_heading_depth: u16,
    pub max_heading_path_bytes: u16,
    pub max_failures_retained: u32,
    pub max_retry_queue: u32,
    pub max_concurrent_file_reads: u16,
    pub max_concurrent_db_transactions: u16,
    pub max_source_list_results: u32,
    pub max_shm_result_bytes: u32,
    pub max_records_inserted_per_scan: u32,
    pub max_deletions_per_scan: u32,
    pub max_retry_ops_per_scan: u32,
    pub max_line_bytes: u32,
    pub max_csv_columns: u16,
    pub max_csv_rows_per_block: u16,
    pub max_json_entries: u32,
    pub max_scalar_bytes: u16,
    pub max_ignore_rules: u16,
    pub max_token_dictionary_entries: u32,
    pub deletion_grace_confirmations: u16,
    pub max_scan_queue: u8,
    pub read_buffer_bytes: u32,
    /// Phase 3: documents must fit one memorydb transaction (no partial visibility).
    pub max_ingest_ops_per_tx: u32,
}

impl Default for IndexQuotaConfig {
    fn default() -> Self {
        Self {
            max_roots_per_user: 8,
            max_path_bytes: 512,
            max_relative_path_bytes: 384,
            max_traversal_depth: 16,
            max_directories_per_scan: 512,
            max_files_inspected_per_scan: 1024,
            max_files_hashed_per_scan: 256,
            max_files_parsed_per_scan: 64,
            max_hash_bytes_per_scan: 8 * 1024 * 1024,
            max_parse_bytes_per_scan: 4 * 1024 * 1024,
            // Keep under memorydb max_payload and one-transaction chunk budget.
            max_file_size_bytes: 48 * 1024,
            max_parser_nesting: 8,
            max_blocks_per_file: 256,
            max_chunks_per_file: 14,
            max_bytes_per_chunk: 4096,
            max_tokens_per_chunk: 256,
            max_unique_tokens_per_chunk: 256,
            max_positions_per_token: 32,
            max_token_length: 64,
            max_heading_depth: 8,
            max_heading_path_bytes: 256,
            max_failures_retained: 128,
            max_retry_queue: 64,
            max_concurrent_file_reads: 2,
            max_concurrent_db_transactions: 1,
            max_source_list_results: 64,
            max_shm_result_bytes: 64 * 1024,
            max_records_inserted_per_scan: 512,
            max_deletions_per_scan: 32,
            max_retry_ops_per_scan: 16,
            max_line_bytes: 8192,
            max_csv_columns: 64,
            max_csv_rows_per_block: 32,
            max_json_entries: 512,
            max_scalar_bytes: 512,
            max_ignore_rules: 128,
            max_token_dictionary_entries: 8192,
            deletion_grace_confirmations: 2,
            max_scan_queue: 2,
            read_buffer_bytes: 4096,
            max_ingest_ops_per_tx: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_bounded() {
        let q = IndexQuotaConfig::default();
        assert!(q.max_file_size_bytes <= 64 * 1024);
        assert!(q.max_chunks_per_file <= 16);
        assert!(q.max_traversal_depth > 0);
        assert!(q.deletion_grace_confirmations >= 1);
    }
}
