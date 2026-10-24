//! Boot-scoped nonsecret references. Possession is not authorization: every
//! operation also authenticates the kernel service role and incarnation.
use crate::{raw_syscall, CapabilityToken, SunlightSyscall};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionRef {
    pub id: u64,
    pub epoch: u64,
}

pub const REGISTER: u64 = 0;
pub const ACTIVATE: u64 = 1;
pub const SUSPEND: u64 = 2;
pub const REVOKE: u64 = 3;

/// Registration consumes a delivered login proof belonging to the authenticated
/// IPC caller. It starts suspended; sessiond activates only a ready session.
pub fn register(proof: CapabilityToken, caller: u64) -> Option<(SessionRef, u32, u32)> {
    let (ret, msg) = unsafe {
        raw_syscall(
            SunlightSyscall::AccountSession,
            REGISTER,
            proof.0,
            caller,
            u64::MAX,
            0,
            0,
            0,
        )
    };
    (ret != u64::MAX).then_some((
        SessionRef {
            id: msg.words[0],
            epoch: msg.words[1],
        },
        ret as u32,
        msg.words[2] as u32,
    ))
}

/// Synchronous kernel transition: success means committed. No asynchronous IPC
/// acknowledgment is involved. A missing response must never imply success.
pub fn transition(session: SessionRef, operation: u64) -> bool {
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::AccountSession,
            operation,
            session.id,
            session.epoch,
            0,
            0,
            0,
            0,
        )
    };
    ret == 0
}
