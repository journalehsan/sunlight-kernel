//! Native syscall 177's temporary TCP readiness observation wire format.
//!
//! Kernel registrations remain authoritative. Snapshot (r9=0) copies at most
//! 64 records to rsi (rdx must be 64). Collect (r9=1) reads rdx observations
//! from rsi, merges local readiness, and writes up to r8 native epoll events
//! to r10. Ctl (r9 low byte=2) uses ordinary epoll_ctl arguments, r8 is the
//! net_server identity (u64::MAX for local), and r9 >> 8 is the endpoint
//! generation captured before reading libc's attachment map. Generation query
//! (r9=3) takes fd in rdi, returning zero for local descriptors. Errors are
//! negative errno. Snapshot/collect never sleep. Reserved bits must be zero.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ExternalObservation {
    pub fd: i32,
    pub events: u32,
    pub token: u64,
    pub identity: u64,
    pub ready: u32,
    pub reserved: u32,
}
const _: () = assert!(core::mem::size_of::<ExternalObservation>() == 32);
const _: () = assert!(core::mem::align_of::<ExternalObservation>() == 8);
