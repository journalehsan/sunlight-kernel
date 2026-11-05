//! Document rendering service for SunlightOS userspace.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
#[cfg(feature = "hayro-native")]
extern crate sunlight_libc;

#[cfg(all(feature = "hayro-native", not(target_os = "sunlightos")))]
compile_error!("hayro-native requires the native SunlightOS target and std overlay");

#[cfg(all(
    feature = "hayro-host",
    not(all(target_os = "linux", target_env = "gnu"))
))]
compile_error!("hayro-host requires Linux GNU. For native SunlightOS use --no-default-features --features hayro-native and tools/build_native_std.sh.");
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
