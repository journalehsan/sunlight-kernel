#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", allow(static_mut_refs))]

#[cfg(feature = "sunlightos")]
extern crate alloc;

pub mod error;
pub mod caps;
pub mod protocol;
pub mod native_ipc;
pub mod context;
pub mod memory_layers;
pub mod diagnostics;
pub mod greeting;
pub mod pipeline;
pub mod provider;

pub use pipeline::CognitivePipeline;
pub use provider::{BrainProvider, LocalBoundedProvider, FutureOnlineProvider, ProviderRegistry};
pub use diagnostics::BrainDiagnostics;
