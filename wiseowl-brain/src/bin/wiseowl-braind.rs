//! wiseowl-braind — Wise Owl service interface.

#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(sunlight_native, allow(static_mut_refs))]

#[cfg(sunlight_native)]
extern crate alloc;

#[cfg(all(feature = "host", not(sunlight_native)))]
include!("../bin_parts/wiseowl-braind-host-body.rs");

#[cfg(sunlight_native)]
include!("../bin_parts/wiseowl-braind-native-body.rs");

#[cfg(not(any(feature = "host", sunlight_native)))]
compile_error!("enable host for host tools, or sunlightos for the native SunlightOS target");
