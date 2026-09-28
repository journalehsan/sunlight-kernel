//! Kernel-facing re-export of the host-tested Linux eventfd counter and pool.
pub use sunlight_compat_linux::eventfd::{
    can_write, create, read, readiness, release, retain, set_status, status, write, EventError,
    EFD_CLOEXEC, EFD_NONBLOCK,
};
