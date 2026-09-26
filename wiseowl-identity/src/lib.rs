//! Wise Owl persistent identity contracts.
//!
//! Phase A deliberately contains only immutable identity, genesis lineage,
//! and committed-head formats. It contains no migration, recovery, activation,
//! device-key, or public mutation protocol.

#![cfg_attr(not(feature = "host"), no_std)]

pub mod codec;
pub mod id;
pub mod lineage;
pub mod root;
pub mod validation;

#[cfg(feature = "host")]
pub use id::fill_host_entropy;
pub use id::{EntropyError, IdentityId};
pub use lineage::{
    ContinuityGeneration, GenesisEventKind, LineageEventId, LineageHead, LineageRecord,
    LineageSequence,
};
pub use root::IdentityRoot;
pub use validation::{validate_identity_set, IdentityError, ValidatedIdentity};

/// All Phase A durable identity formats use version 1.
pub const IDENTITY_FORMAT_VERSION: u16 = 1;
