//! Storage engine modules.
//! - record: on-disk binary format and CRC
//! - index: in-memory HashMap index
//! - file: StorageEngine implementation (open/recover/put/get/delete/scan)

pub mod file;
pub mod index;
pub mod record;

pub use file::{StorageEngine, StorageError};
pub use index::{Index, IndexEntry};
pub use record::{
    compute_crc, read_record, write_record, FLAG_DELETE, FLAG_PUT, RECORD_MAGIC, RECORD_VERSION,
    RecordHeader,
};
