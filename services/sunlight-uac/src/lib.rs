//! sunlight-uac — shared, no_std logic for SunlightOS user access control.
//!
//! This crate factors the former `uac_service` and `capabilityctl` modules out
//! of `sunlightd` into a standalone, modular package. Both the `uac_service`
//! daemon and the `capabilityctl` control client link against this library so
//! the session-cache and capability-rule models live in exactly one place.
//!
//! - [`session`]: 30-minute prompt-cache sessions and the `runas` skeleton.
//! - [`capability`]: uid/gid → path-prefix ACL translation and access checks.

#![no_std]

pub mod capability;
pub mod session;
