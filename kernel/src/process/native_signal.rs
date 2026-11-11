//! Bounded synchronous native-C signals. No external/asynchronous sender.
//! Return contexts stay in kernel memory; userspace supplies no register frame.
use crate::arch::x86_64::{cpu::FxState, syscall::SyscallRegs};

pub struct Saved {
    pub regs: SyscallRegs,
    pub rsp: u64,
    pub return_rsp: u64,
    pub fs_base: u64,
    pub fp: FxState,
}

#[derive(Default)]
pub struct State {
    pub actions: [[u64; 3]; 2], // handler, mask, flags
    pub trampoline: u64,
    pub saved: Option<alloc::boxed::Box<Saved>>,
    pub pending: u64,
}

impl State {
    pub fn reset_for_exec(&mut self) {
        let ignored = [self.actions[0][0] == 1, self.actions[1][0] == 1];
        *self = Self::default();
        for (index, ignore) in ignored.into_iter().enumerate() {
            if ignore {
                self.actions[index][0] = 1;
            }
        }
    }
}
