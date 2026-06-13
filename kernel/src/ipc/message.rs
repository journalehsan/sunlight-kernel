use crate::arch::x86_64::syscall::SyscallRegs;
use crate::capability::CapabilityToken;

pub const IPC_REG_WORDS: usize = 4;
pub const IPC_MAX_WORDS: usize = 8;
pub const IPC_MAX_CAPS: usize = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IpcMsg {
    /// What operation is requested (server-defined meaning).
    pub label: u64,
    /// Sender identity, filled by the kernel on receive.
    pub badge: u64,
    /// How many of words[] are valid (0-8).
    pub word_count: u32,
    /// How many capability tokens are being transferred (0-2).
    pub cap_count: u32,
    /// Inline data. Large payloads pass shared-memory caps in these words.
    pub words: [u64; IPC_MAX_WORDS],
    /// Capability tokens being transferred with this message.
    pub caps: [CapabilityToken; IPC_MAX_CAPS],
}

impl IpcMsg {
    pub const fn empty() -> Self {
        Self {
            label: 0,
            badge: 0,
            word_count: 0,
            cap_count: 0,
            words: [0; IPC_MAX_WORDS],
            caps: [CapabilityToken::INVALID; IPC_MAX_CAPS],
        }
    }

    pub const fn with_label(label: u64) -> Self {
        Self {
            label,
            badge: 0,
            word_count: 0,
            cap_count: 0,
            words: [0; IPC_MAX_WORDS],
            caps: [CapabilityToken::INVALID; IPC_MAX_CAPS],
        }
    }

    pub fn word(mut self, idx: usize, val: u64) -> Self {
        if idx < IPC_MAX_WORDS {
            self.words[idx] = val;
            let count = (idx + 1) as u32;
            if self.word_count < count {
                self.word_count = count;
            }
        }
        self
    }

    pub fn with_cap(mut self, idx: usize, val: CapabilityToken) -> Self {
        if idx < IPC_MAX_CAPS {
            self.caps[idx] = val;
            let count = (idx + 1) as u32;
            if self.cap_count < count {
                self.cap_count = count;
            }
        }
        self
    }

    /// Load a compact IPC message from syscall registers.
    pub fn from_registers(regs: &SyscallRegs) -> Self {
        let counts = regs.rdx;
        let word_count = ((counts & 0xffff_ffff) as u32).min(IPC_MAX_WORDS as u32);
        let cap_count = ((counts >> 32) as u32).min(IPC_MAX_CAPS as u32);
        let mut msg = Self {
            label: regs.rdi,
            badge: 0,
            word_count,
            cap_count,
            words: [0; IPC_MAX_WORDS],
            caps: [CapabilityToken::INVALID; IPC_MAX_CAPS],
        };
        msg.words[0] = regs.r8;
        msg.words[1] = regs.r9;
        msg.words[2] = regs.r10;
        // r11 is reserved by SYSCALL/SYSRET, so word 3 uses r12 in this ABI.
        msg.words[3] = regs.r12;
        msg.caps[0] = CapabilityToken(regs.r13);
        msg.caps[1] = CapabilityToken(regs.r14);
        msg
    }

    /// Write a compact IPC message into syscall return registers.
    pub fn to_registers(&self, regs: &mut SyscallRegs) {
        regs.rdi = self.label;
        regs.rsi = self.badge;
        regs.rdx = self.word_count as u64 | ((self.cap_count as u64) << 32);
        regs.r8 = self.words[0];
        regs.r9 = self.words[1];
        regs.r10 = self.words[2];
        // r11 holds return flags for SYSRET; keep it out of the IPC ABI.
        regs.r12 = self.words[3];
        regs.r13 = self.caps[0].0;
        regs.r14 = self.caps[1].0;
    }
}
