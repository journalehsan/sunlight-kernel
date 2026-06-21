//! IPC opcodes for gcd.

#![allow(dead_code)]

/// gcd's IPC opcode namespace (0x9100 range).
pub mod GcdOp {
    /// word(0)=pid — request reaping of a zombie process (sent by niced).
    pub const REAP_ZOMBIE: u64 = 0x9101;
    /// word(0)=pid — notify gcd that a process has exited.
    pub const PROCESS_EXITED: u64 = 0x9102;
    /// Memory-pressure notify (no payload).
    pub const MEM_PRESSURE: u64 = 0x9103;
    pub const REPLY: u64 = 0x91FF;
}

/// proc manager IPC opcodes served by gcd for now.
pub mod ProcOp {
    /// word(0)=session root pid, word(1)=signal.
    pub const TERMINATE_SESSION: u64 = 0x9201;
    /// reply.words[0] = number of tasks targeted.
    pub const REPLY: u64 = 0x92FF;
}

/// niced's IPC opcode namespace (0x9000 range), mirrored here so gcd can
/// notify niced without a cross-crate dependency. Only the opcode used by
/// gcd is listed.
pub mod NicedOp {
    /// Sent by gcd to niced under memory pressure (no payload).
    pub const TIGHTEN: u64 = 0x9014;
    pub const REPLY: u64 = 0x90FF;
}
