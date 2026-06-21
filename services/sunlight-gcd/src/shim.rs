//! Kernel operation shims (gcd side).
//!
//! `set_nice`, `send_signal`, and `force_terminate` are real wrappers over the
//! current syscall surface. `reap` remains a stub because the scheduler reaps
//! finished tasks internally on deschedule.

pub trait KernelOps {
    /// REAL: wraps syscall 83 (SetNice).
    fn set_nice(&self, pid: usize, nice: i8) -> bool;

    /// REAL: wraps syscall 72 (Kill).
    fn send_signal(&self, pid: usize, sig: u8) -> bool;

    /// REAL: forceful termination is `kill(pid, SIGKILL)`.
    fn force_terminate(&self, pid: usize) -> bool;

    /// SHIM — TODO: kernel syscall — reap: release a Finished (zombie)
    /// process's PCB slot back to the scheduler. No such syscall exists.
    fn reap(&self, pid: usize) -> bool;
}

pub struct RealKernelOps;

impl KernelOps for RealKernelOps {
    fn set_nice(&self, pid: usize, nice: i8) -> bool {
        sunlight_ipc::set_nice(pid as u64, nice)
    }

    fn send_signal(&self, pid: usize, sig: u8) -> bool {
        sunlight_ipc::kill(pid as u64, sig as u32)
    }

    fn force_terminate(&self, pid: usize) -> bool {
        sunlight_ipc::kill(pid as u64, 9)
    }

    fn reap(&self, pid: usize) -> bool {
        serial_println!("[GCD] TODO: kernel syscall — reap: pid={}", pid);
        false
    }
}
