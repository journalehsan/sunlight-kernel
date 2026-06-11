pub mod address_space;
pub mod elf_loader;
pub mod layout;
pub mod spawn;
pub mod fork;
pub mod mmap;
pub mod fd_table;

use address_space::AddressSpace;
use layout::USER_STACK_TOP;
use crate::ipc::IpcMsg;
use crate::memory::pmm::PhysicalMemoryManager;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use x86_64::VirtAddr;

pub const KERNEL_STACK_SIZE: usize = 32 * 1024;

/// A schedulable process.
pub struct Process {
    pub pid: usize,
    pub ppid: usize,  // parent pid
    pub name: &'static str,
    pub state: ProcessState,
    pub address_space: AddressSpace,
    pub capabilities: Vec<Capability>,
    pub kernel_stack: alloc::boxed::Box<[u8; KERNEL_STACK_SIZE]>,
    pub kernel_stack_top: u64,
    pub user_stack_top: u64,
    pub entry_point: u64,
    pub context_rsp: u64,
    pub uid: u32,
    pub gid: u32,
    pub ipc_queue: VecDeque<IpcMsg>,
    pub ipc_endpoint: Option<u32>,
    pub ipc_reply: Option<IpcMsg>,
    pub pending_call: Option<(u64, IpcMsg)>,
    pub pending_reply_wait: Option<(u32, IpcMsg)>,
    pub fd_table: fd_table::FdTable,
    pub capability_mode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    BlockedOnIpc,
    Finished,
}

/// A capability held by a process.
#[derive(Debug, Clone, Copy)]
pub struct Capability {
    pub token: u64,
    pub endpoint_id: u32,
    pub can_send: bool,
    pub can_recv: bool,
    pub can_grant: bool,
}

impl Process {
    /// Create a new user process with its own address space.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn new(
        pid: usize,
        ppid: usize,
        name: &'static str,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Self {
        let address_space = AddressSpace::new(pmm, hhdm_offset);

        let kernel_stack = alloc::boxed::Box::new([0u8; KERNEL_STACK_SIZE]);
        let kernel_stack_top = core::ptr::addr_of!(kernel_stack[KERNEL_STACK_SIZE - 1]) as u64 + 1;
        let user_stack_top = USER_STACK_TOP;

        Self {
            pid,
            ppid,
            name,
            state: ProcessState::Ready,
            address_space,
            capabilities: Vec::new(),
            kernel_stack,
            kernel_stack_top,
            user_stack_top,
            entry_point: 0,
            context_rsp: 0,
            uid: 0,
            gid: 0,
            ipc_queue: VecDeque::new(),
            ipc_endpoint: None,
            ipc_reply: None,
            pending_call: None,
            pending_reply_wait: None,
            fd_table: fd_table::FdTable::new(),
            capability_mode: false,
        }
    }

    /// Build the initial context frame on the kernel stack for first entry.
    /// Layout matches the pop order used by `iretq_to_context` and the timer handler.
    pub fn init_context(&mut self, entry_point: u64, user_stack_top: u64) {
        self.entry_point = entry_point;
        self.user_stack_top = user_stack_top;

        // Frame layout (from context_rsp upward):
        // [+0]   r15
        // [+8]   r14
        // [+16]  r13
        // [+24]  r12
        // [+32]  rbp
        // [+40]  rbx
        // [+48]  r11
        // [+56]  r10
        // [+64]  r9
        // [+72]  r8
        // [+80]  rdi
        // [+88]  rsi
        // [+96]  rdx
        // [+104] rcx
        // [+112] rax
        // [+120] RIP
        // [+128] CS
        // [+136] RFLAGS
        // [+144] RSP
        // [+152] SS
        const FRAME_SIZE: u64 = 160;

        let frame_base = self.kernel_stack_top - FRAME_SIZE;
        self.context_rsp = frame_base;

        // SAFETY: frame_base is within the allocated kernel stack.
        unsafe {
            let base = frame_base as *mut u64;
            // 15 GPRs (all zero)
            for i in 0..15 {
                base.add(i).write_volatile(0);
            }
            // RIP
            base.add(15).write_volatile(entry_point);
            // CS (Ring 3 code)
            base.add(16).write_volatile(0x2B);
            // RFLAGS (IF set)
            base.add(17).write_volatile(0x202);
            // RSP (user stack top)
            base.add(18).write_volatile(user_stack_top);
            // SS (Ring 3 data)
            base.add(19).write_volatile(0x23);
        }
    }

    /// Set initial userspace argument registers for a freshly initialized context.
    pub fn set_initial_args(&mut self, rdi: u64, rsi: u64, rdx: u64, rcx: u64) {
        unsafe {
            let base = self.context_rsp as *mut u64;
            base.add(10).write_volatile(rdi);
            base.add(11).write_volatile(rsi);
            base.add(12).write_volatile(rdx);
            base.add(13).write_volatile(rcx);
        }
    }
}
