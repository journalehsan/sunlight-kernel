//! sunlight-kv library
//! Core storage engine and protocol types for the SunlightOS key-value daemon.

// When building the SunlightOS variant we are a no_std crate with alloc.
#![cfg_attr(feature = "sunlightos", no_std)]

#[cfg(feature = "sunlightos")]
extern crate alloc;

pub mod acl;
pub mod ipc;
/// Versioned binary stats snapshot layout (host + SunlightOS; pure core).
pub mod stats_wire;

// Host-only rich modules (direct fs + UDS daemon).
#[cfg(feature = "host")]
pub mod daemon;
#[cfg(feature = "host")]
pub mod storage;

pub use acl::Acl;
pub use ipc::{Request, Response};
pub use stats_wire::{StatsSnapshotV1, STATS_BYTES, STATS_MAGIC, STATS_VERSION};

#[cfg(feature = "host")]
pub use storage::StorageEngine;
