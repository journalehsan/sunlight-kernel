//! Document viewer state, usable with alloc-only native clients.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
#[cfg(feature = "std")]
pub mod ui;
pub mod viewer;
