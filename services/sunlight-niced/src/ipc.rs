//! IPC opcodes for the niced watchdog protocol.

#![allow(dead_code)]

/// niced's own IPC opcode namespace (0x9000 range).
pub mod NicedOp {
    /// word(0)=pid, word(1)=seq — heartbeat ping from a supervised process.
    pub const PING: u64 = 0x9001;
    /// word(0)=pid, word(1)=nice (sign-extended i8 as u64) — set nice value.
    pub const SET_NICE: u64 = 0x9010;
    /// word(0)=pid, word(1)=period_secs (0=disable) — configure watchdog.
    pub const SET_WATCHDOG: u64 = 0x9011;
    /// word(0)=pid — clear watchdog state for pid.
    pub const CLEAR_WD: u64 = 0x9012;
    /// word(0)=pid -> reply word(0)=nice, word(1)=state, word(2)=not_responding.
    pub const QUERY: u64 = 0x9013;
    /// Sent by gcd when memory pressure is detected (no payload).
    pub const TIGHTEN: u64 = 0x9014;
    pub const REPLY: u64 = 0x90FF;
    pub const ERROR: u64 = 0x90FE;
}

/// gcd's IPC opcode namespace (0x9100 range), mirrored here so niced can
/// send messages to gcd without a cross-crate dependency.
pub mod GcdOp {
    /// word(0)=pid — request reaping of a zombie process.
    pub const REAP_ZOMBIE: u64 = 0x9101;
    /// word(0)=pid — notify gcd that a process has exited.
    pub const PROCESS_EXITED: u64 = 0x9102;
    pub const REPLY: u64 = 0x91FF;
}
