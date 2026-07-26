//! wiseowl-memoryctl — diagnostic CLI (host UDS or native IPC).

#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", no_main)]

#[cfg(feature = "sunlightos")]
extern crate alloc;

#[cfg(all(feature = "host", not(feature = "sunlightos")))]
include!("../bin_parts/wiseowl-memoryctl-host-body.rs");

#[cfg(feature = "sunlightos")]
include!("../bin_parts/wiseowl-memoryctl-native-body.rs");
