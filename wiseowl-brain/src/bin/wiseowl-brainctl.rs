//! wiseowl-brainctl — diagnostic and query CLI for wiseowl-braind.

#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", no_main)]
#![cfg_attr(feature = "sunlightos", allow(static_mut_refs))]

#[cfg(feature = "sunlightos")]
extern crate alloc;

#[cfg(all(feature = "host", not(feature = "sunlightos")))]
include!("../bin_parts/wiseowl-brainctl-host-body.rs");

#[cfg(feature = "sunlightos")]
include!("../bin_parts/wiseowl-brainctl-native-body.rs");
