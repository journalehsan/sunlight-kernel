#![no_std]

extern crate alloc;

pub mod color_picker;

/// Stable host identity used by the compositor's existing KeepAlive policy.
/// Request-specific titles belong inside the dialog, not on its native window:
/// closing that window must leave the host alive to send the IPC result.
pub const WINDOW_TITLE: &str = "Sunlight Dialog";

#[cfg(test)]
mod lifecycle_tests;
