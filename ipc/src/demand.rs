//! D1 activation protocol. Discovery returns metadata, never a service capability.
//! ACTIVATE requires kernel-checked ServiceLifecycle authority. Invocation still
//! requires the caller's own nameserver lookup and the service's normal checks.
use crate::{CapabilityToken, IpcMsg, ServiceCapability, SunlightSyscall};

pub const DISCOVER: u64 = 0xD100;
pub const ACTIVATE: u64 = 0xD101;
pub const HELLO: u64 = 0xD102;
pub const READY: u64 = 0xD103;
pub const METRICS: u64 = 0xD104;
pub const OK: u64 = 0xD180;
pub const ERROR: u64 = 0xD1FF;
pub const TIMEOUT_MS: u64 = 5_000;
pub const RETRY_MS: u64 = 1_000;
pub const NOT_FOUND: u64 = 1;
pub const BUSY: u64 = 2;
pub const START_FAILED: u64 = 3;
pub const TIMED_OUT: u64 = 4;
pub const DIED: u64 = 5;
pub const STALE: u64 = 6;
pub const DEPENDENCY: u64 = 7;
pub const STOPPED: u64 = 8;
pub const EXHAUSTED: u64 = 9;
/// Explicit mode in the signal argument; legacy two-argument kill callers
/// need not initialize otherwise-unused registers.
pub const KILL_GENERATION_FLAG: u64 = 1 << 63;

pub fn activation_authorized(restrictions: Option<u64>) -> bool {
    restrictions.is_none_or(|mask| mask & ServiceCapability::ServiceLifecycle.bit() != 0)
}

pub fn error(code: u64) -> IpcMsg {
    IpcMsg::with_label(ERROR).word(0, code)
}

/// Trusted SunlightD only. Checks a SEND capability against the expected live
/// process incarnation and returns its actual endpoint ID. Uses the existing
/// nameserver validation syscall's manager mode, not userspace identity claims.
pub fn endpoint_identity(cap: CapabilityToken, pid: u32, generation: u64) -> Option<u32> {
    if pid == 0 || generation == 0 {
        return None;
    }
    let (ret, _) = unsafe {
        crate::raw_syscall(
            SunlightSyscall::NameserverEndpointValidate,
            cap.0,
            pid as u64,
            generation,
            0,
            0,
            0,
            0,
        )
    };
    ret.checked_sub(1).and_then(|id| u32::try_from(id).ok())
}

/// Name occupies four register words. Reject truncation, embedded NUL and
/// non-ASCII rather than silently selecting a different unit.
pub fn request(op: u64, name: &str) -> Option<IpcMsg> {
    if name.is_empty() || name.len() > 31 || !name.is_ascii() || name.as_bytes().contains(&0) {
        return None;
    }
    let mut msg = IpcMsg::with_label(op);
    for (i, b) in name.bytes().enumerate() {
        msg.words[i / 8] |= (b as u64) << ((i % 8) * 8);
    }
    msg.word_count = 4;
    Some(msg)
}

/// Manager-only generation-qualified use of existing kill syscall.
pub fn kill_instance(pid: u32, generation: u64, signal: u32) -> bool {
    if generation == 0 {
        return false;
    }
    let (ret, _) = unsafe {
        crate::raw_syscall(
            SunlightSyscall::Kill,
            pid as u64,
            signal as u64 | KILL_GENERATION_FLAG,
            generation,
            0,
            0,
            0,
            0,
        )
    };
    ret == 0
}
