//! Shared IPC contract for process/session teardown requests.

#![allow(dead_code)]

pub mod ProcOp {
    /// word(0)=session root pid (usually the tab's shell pid), word(1)=signal.
    pub const TERMINATE_SESSION: u64 = 0x9201;
    pub const REPLY: u64 = 0x92FF;
}

pub const SIGKILL: u64 = 9;
