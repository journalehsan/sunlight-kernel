//! wiseowl-memoryd — short-term memory daemon (host UDS or native SunlightOS IPC).

#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", no_main)]
#![cfg_attr(feature = "sunlightos", allow(static_mut_refs))]

#[cfg(feature = "sunlightos")]
extern crate alloc;

#[cfg(all(feature = "host", not(feature = "sunlightos")))]
include!("../bin_parts/wiseowl-memoryd-host-body.rs");

#[cfg(feature = "sunlightos")]
include!("../bin_parts/wiseowl-memoryd-native-body.rs");
