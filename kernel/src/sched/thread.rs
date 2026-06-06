use alloc::boxed::Box;
use crate::serial_println;

pub const STACK_SIZE: usize = 32 * 1024; // 32 KiB

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Ready,
    Running,
    Finished,
}

// Saved CPU state for context switch
#[repr(C)]
pub struct CpuContext {
    pub rsp: u64,
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbx: u64,
    pub rbp: u64,
}

impl CpuContext {
    #[allow(dead_code)]
    pub const fn empty() -> Self {
        Self {
            rsp: 0,
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbx: 0,
            rbp: 0,
        }
    }
}

#[allow(dead_code)]
pub struct Thread {
    pub id: usize,
    pub name: &'static str,
    pub state: ThreadState,
    pub stack: Box<[u8; STACK_SIZE]>,
    pub context: CpuContext,
}

impl Thread {
    pub fn new(id: usize, name: &'static str, entry: fn() -> !) -> Self {
        let stack = Box::new([0u8; STACK_SIZE]);
        let stack_top = core::ptr::addr_of!(stack[STACK_SIZE - 1]) as u64 + 1;

        serial_println!("[SCHED] Thread '{}' stack: {:#x}..{:#x}, entry: {:#x}", name, stack_top - STACK_SIZE as u64, stack_top, entry as u64);

        // Set up initial stack so that `switch_context` doing `ret` will jump to entry.
        // Stack layout (grows downward):
        //   top: [entry fn ptr]  <- initial RSP will point here
        //        [rbp=0]
        //        [rbx=0]
        //        [r12=0]
        //        [r13=0]
        //        [r14=0]
        //        [r15=0]
        let rsp = stack_top;
        let rsp = push_u64(rsp, entry as u64); // return address = entry function
        let rsp = push_u64(rsp, 0); // rbp
        let rsp = push_u64(rsp, 0); // rbx
        let rsp = push_u64(rsp, 0); // r12
        let rsp = push_u64(rsp, 0); // r13
        let rsp = push_u64(rsp, 0); // r14
        let rsp = push_u64(rsp, 0); // r15

        Self {
            id,
            name,
            state: ThreadState::Ready,
            stack,
            context: CpuContext {
                rsp,
                r15: 0,
                r14: 0,
                r13: 0,
                r12: 0,
                rbx: 0,
                rbp: 0,
            },
        }
    }
}

fn push_u64(rsp: u64, val: u64) -> u64 {
    let rsp = rsp - 8;
    // SAFETY: rsp is within the allocated stack.
    unsafe { core::ptr::write(rsp as *mut u64, val); }
    rsp
}
