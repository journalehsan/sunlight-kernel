//! Document rendering service for SunlightOS userspace.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

#[cfg(all(
    feature = "hayro-host",
    not(all(target_os = "linux", target_env = "gnu"))
))]
compile_error!("Hayro is host-only (Linux GNU); native SunlightOS PDF rendering is not available. Disable default features; use mupdf-legacy only for explicit rollback.");
#[cfg(all(feature = "hayro-host", feature = "native-smoke"))]
compile_error!("native smoke tests require --no-default-features --features native-smoke; Hayro must not run through Helios.");
#[cfg(feature = "std")]
pub mod backend;
#[cfg(feature = "renderer")]
pub mod backends;
pub mod client;
pub mod ipc;
#[cfg(feature = "renderer")]
pub mod service;
pub mod thumbnail;
