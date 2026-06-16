//! Exact request/response enums for the sunlight-kv IPC protocol.
//! These are serialized/deserialized using bincode for the binary wire format.

#[cfg(feature = "host")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "sunlightos")]
use alloc::string::String;
#[cfg(feature = "sunlightos")]
use alloc::vec::Vec;

/// Client -> daemon requests.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(Serialize, Deserialize))]
#[allow(non_camel_case_types)]
pub enum Request {
    /// Store or overwrite a value under `key`.
    /// The caller identity (from transport metadata) becomes owner on first write.
    KV_PUT { key: String, value: Vec<u8> },

    /// Retrieve the value for `key`, if present and permitted.
    KV_GET { key: String },

    /// Delete the key (write a tombstone). Requires write permission.
    KV_DELETE { key: String },

    /// Return all keys that start with the given prefix (in arbitrary order).
    KV_SCAN { prefix: String },
}

/// Daemon -> client responses.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(Serialize, Deserialize))]
#[allow(non_camel_case_types)]
pub enum Response {
    /// Operation completed successfully (used for PUT and DELETE).
    OK,

    /// Successful GET; carries the stored bytes.
    VALUE(Vec<u8>),

    /// The requested key does not exist (or was deleted).
    NOT_FOUND,

    /// The caller identity lacks the required read or write permission.
    PERMISSION_DENIED,

    /// General error with a human-readable message.
    ERROR(String),

    /// Result of a SCAN: the matching keys (may be empty).
    SCAN_RESULT(Vec<String>),
}
