//! In-memory index rebuilt from the append-only log at startup.

use std::collections::HashMap;

/// Entry in the live in-memory index.
/// Points at the latest (or only) record for a key in the log file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    /// Absolute byte offset of the RecordHeader in the file.
    pub offset: u64,
    /// Total on-disk size of this record (header + key + value + acl).
    pub total_len: u32,
    /// Record flags (FLAG_PUT or FLAG_DELETE). DELETE entries are never kept in the index.
    pub flags: u16,
}

/// The live index: key -> most recent record location.
pub type Index = HashMap<String, IndexEntry>;
