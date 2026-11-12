//! Network IPC opcode/status definitions.
//!
//! The canonical constants live in `sunlight-ipc` so that native libc clients
//! and the userspace network service share one definition. This module keeps
//! the historical `sunlight_net::netop::*` paths working.
pub use sunlight_ipc::netop::*;
