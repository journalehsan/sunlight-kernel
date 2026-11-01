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
