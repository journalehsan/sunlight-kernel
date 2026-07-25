//! Pure NTP client protocol for SunlightOS timed.
//!
//! Host-testable (`cargo test -p sunlight-timed --lib`). No network I/O here —
//! packet validation, offset/delay math, and sample selection only.
//!
//! Security note: unauthenticated pool NTP improves accuracy but is not
//! cryptographically authenticated. NTS is a future hardening step.

#![cfg_attr(not(test), no_std)]

pub mod ntp;
pub mod state;

pub use ntp::*;
pub use state::*;
