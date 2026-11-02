//! Private debug-image authority protocol. No mappings, payload or Doorbell.
//! A handle never authorizes a caller; the kernel authenticates every operation.
use crate::account_session::SessionRef;
pub const REQUEST: u64 = 0xF1C100;
pub const REPLY: u64 = 0xF1C101;
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    pub token: u64,
    pub generation: u64,
}
impl core::fmt::Debug for Handle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Handle")
            .field("token", &"REDACTED")
            .field("generation", &self.generation)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Error {
    Denied = 1,
    Stale = 2,
    Duplicate = 3,
    Terminal = 4,
    Capacity = 5,
    Exhausted = 6,
    Incomplete = 7,
    Disabled = 8,
    Cancelled = 9,
    Changed = 10,
    Pending = 11,
}
pub fn call(op: u64, h: Handle, arg: u64, session: SessionRef) -> Result<(u64, u64), Error> {
    let mut result = 0xf1c1u64;
    let mut value = h.token;
    let mut generation = h.generation;
    unsafe {
        core::arch::asm!("syscall", inout("rax") result,
        in("rdi") op, inout("rsi") value, inout("rdx") generation,
        in("r10") arg, in("r8") session.id, in("r9") session.epoch,
        lateout("rcx") _, lateout("r11") _);
    }
    match result {
        0 => Ok((value, generation)),
        1 => Err(Error::Denied),
        2 => Err(Error::Stale),
        3 => Err(Error::Duplicate),
        4 => Err(Error::Terminal),
        5 => Err(Error::Capacity),
        6 => Err(Error::Exhausted),
        7 => Err(Error::Incomplete),
        9 => Err(Error::Cancelled),
        10 => Err(Error::Changed),
        11 => Err(Error::Pending),
        _ => Err(Error::Disabled),
    }
}
pub const EMPTY: Handle = Handle {
    token: 0,
    generation: 0,
};
pub const NO_SESSION: SessionRef = SessionRef { id: 0, epoch: 0 };
pub fn operate(op: u64, h: Handle, arg: u64) -> Result<u64, Error> {
    call(op, h, arg, NO_SESSION).map(|r| r.0)
}

/// Debug-only M4 park. Zero means the registration completed; collect with op
/// 13. Changed means no registration was needed; observe again with op 11.
pub fn doorbell_wait(h: Handle, observed: u64) -> Result<(), Error> {
    let code: u64;
    unsafe {
        core::arch::asm!("syscall", inlateout("rax") 0xf1c2u64 => code,
            in("rdi") h.token, in("rsi") h.generation, in("rdx") observed,
            lateout("rcx") _, lateout("r11") _);
    }
    match code {
        0 => Ok(()),
        1 => Err(Error::Denied),
        2 => Err(Error::Stale),
        3 => Err(Error::Duplicate),
        4 => Err(Error::Terminal),
        5 => Err(Error::Capacity),
        6 => Err(Error::Exhausted),
        7 => Err(Error::Incomplete),
        10 => Err(Error::Changed),
        _ => Err(Error::Disabled),
    }
}

/// M5 experimental backend. A fixed page-relative word index, never a pointer.
/// Ordinary/M1–M4 images deny these operations. Each access reauthenticates the
/// exact role and mapping lifetime. Release store / Acquire load on x86-64.
/// The kernel admits one publication and one acknowledgement per fresh channel.
pub fn transfer_word(h: Handle, store: bool, index: u64, value: u64) -> Result<u64, Error> {
    call(if store { 19 } else { 18 }, h, index, SessionRef { id: value, epoch: 0 })
        .map(|r| r.0)
}
