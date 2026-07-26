//! wiseowl-memorydb — durable long-term memory database daemon.

#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", no_main)]
#![cfg_attr(feature = "sunlightos", allow(static_mut_refs))]

#[cfg(feature = "sunlightos")]
extern crate alloc;

#[cfg(all(feature = "host", not(feature = "sunlightos")))]
include!("../bin_parts/wiseowl-memorydb-host-body.rs");

#[cfg(feature = "sunlightos")]
include!("../bin_parts/wiseowl-memorydb-native-body.rs");
